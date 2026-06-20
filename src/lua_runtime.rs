use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use mlua::{Function, Lua, RegistryKey, Table, UserData, UserDataFields, UserDataMethods};

use crate::process::ProcessInfo;
use crate::tmux::PaneInfo;

pub struct LuaRuntime {
    lua: Lua,
    kinds: Rc<RefCell<Vec<Kind>>>,
}

struct Kind {
    name: String,
    detect: RegistryKey,
    label: RegistryKey,
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub kind: String,
    pub label: String,
}

#[derive(Debug, Clone)]
struct LuaPane {
    id: String,
    pid: i32,
    cwd: String,
    cwd_basename: String,
    proc_tree: Vec<ProcessInfo>,
}

#[derive(Debug, Clone)]
struct LuaProcTree(Vec<ProcessInfo>);

impl LuaRuntime {
    pub fn new() -> Result<Self> {
        let lua = Lua::new();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let runtime = Self { lua, kinds };
        runtime.install_api()?;
        runtime.load_user_kinds()?;
        runtime.load_builtin_kinds()?;
        Ok(runtime)
    }

    pub fn detect(
        &self,
        pane: &PaneInfo,
        proc_tree: Vec<ProcessInfo>,
    ) -> Result<Option<Detection>> {
        let lua_pane = LuaPane {
            id: pane.id.clone(),
            pid: pane.pid,
            cwd: pane.cwd.clone(),
            cwd_basename: basename(&pane.cwd),
            proc_tree,
        };
        let userdata = self.lua.create_userdata(lua_pane).map_err(lua_err)?;

        for kind in self.kinds.borrow().iter() {
            let detect: Function = self.lua.registry_value(&kind.detect).map_err(lua_err)?;
            let matched = detect
                .call::<bool>(userdata.clone())
                .map_err(|error| anyhow!("kind {} detect failed: {error}", kind.name))?;
            if matched {
                let label_fn: Function = self.lua.registry_value(&kind.label).map_err(lua_err)?;
                let label = label_fn
                    .call::<String>(userdata.clone())
                    .map_err(|error| anyhow!("kind {} label failed: {error}", kind.name))?;
                return Ok(Some(Detection {
                    kind: kind.name.clone(),
                    label,
                }));
            }
        }

        Ok(None)
    }

    fn install_api(&self) -> Result<()> {
        let castr = self.lua.create_table().map_err(lua_err)?;
        let kinds = Rc::clone(&self.kinds);
        let register_kind = self
            .lua
            .create_function(move |lua, table: Table| {
                let name: String = table.get("name")?;
                let detect: Function = table.get("detect")?;
                let label: Function = table.get("label")?;
                let detect = lua.create_registry_value(detect)?;
                let label = lua.create_registry_value(label)?;
                kinds.borrow_mut().push(Kind {
                    name,
                    detect,
                    label,
                });
                Ok(())
            })
            .map_err(lua_err)?;
        castr.set("register_kind", register_kind).map_err(lua_err)?;
        self.lua.globals().set("castr", castr).map_err(lua_err)?;
        Ok(())
    }

    fn load_user_kinds(&self) -> Result<()> {
        let dir = config_dir().join("kinds");
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(());
        };

        let mut files = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("lua"))
            .collect::<Vec<_>>();
        files.sort();

        for path in files {
            if let Err(error) = self.load_user_kind(&path) {
                eprintln!("castr: failed to load {}: {error}", path.display());
            }
        }

        Ok(())
    }

    fn load_user_kind(&self, path: &Path) -> Result<()> {
        let source = fs::read_to_string(path)?;
        self.lua
            .load(&source)
            .set_name(path.display().to_string())
            .exec()
            .map_err(|error| anyhow!("{error}"))
    }

    fn load_builtin_kinds(&self) -> Result<()> {
        self.lua
            .load(BUILTIN_KINDS)
            .set_name("builtin-kinds.lua")
            .exec()
            .map_err(|error| anyhow!("failed to load built-in Lua kinds: {error}"))
    }
}

fn config_dir() -> PathBuf {
    env::var_os("CASTR_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_CONFIG_HOME").map(|home| PathBuf::from(home).join("castr")))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/castr")))
        .unwrap_or_else(|| PathBuf::from(".config/castr"))
}

impl UserData for LuaPane {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.clone()));
        fields.add_field_method_get("pid", |_, this| Ok(this.pid));
        fields.add_field_method_get("cwd", |_, this| Ok(this.cwd.clone()));
        fields.add_field_method_get("cwd_basename", |_, this| Ok(this.cwd_basename.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("proc_tree", |_, this, ()| {
            Ok(LuaProcTree(this.proc_tree.clone()))
        });
    }
}

impl UserData for LuaProcTree {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("any", |lua, this, predicate: Function| {
            for process in &this.0 {
                let table = process_table(lua, process)?;
                if predicate.call::<bool>(table)? {
                    return Ok(true);
                }
            }
            Ok(false)
        });

        methods.add_method("list", |lua, this, ()| {
            let table = lua.create_table()?;
            for (idx, process) in this.0.iter().enumerate() {
                table.set(idx + 1, process_table(lua, process)?)?;
            }
            Ok(table)
        });
    }
}

fn lua_err(error: mlua::Error) -> anyhow::Error {
    anyhow!(error.to_string())
}

fn process_table(lua: &Lua, process: &ProcessInfo) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("pid", process.pid)?;
    table.set("ppid", process.ppid)?;
    table.set("argv", process.argv.clone())?;
    Ok(table)
}

fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

const BUILTIN_KINDS: &str = r#"
local function argv_has(p, pattern)
  return p:proc_tree():any(function(x)
    return x.argv:match(pattern) ~= nil
  end)
end

castr.register_kind {
  name = "pi",
  detect = function(p)
    return argv_has(p, "pi%-coding%-agent")
        or argv_has(p, "@earendil%-works/pi")
        or argv_has(p, "^pi$")
        or argv_has(p, "^pi%s")
        or argv_has(p, "/pi$")
        or argv_has(p, "/pi%s")
  end,
  label = function(p)
    return "pi · " .. p.cwd_basename
  end,
}

castr.register_kind {
  name = "nvim",
  detect = function(p)
    return argv_has(p, "nvim") or argv_has(p, "vim")
  end,
  label = function(p)
    return "nvim · " .. p.cwd_basename
  end,
}

castr.register_kind {
  name = "claude",
  detect = function(p)
    return argv_has(p, "claude")
  end,
  label = function(p)
    return "claude · " .. p.cwd_basename
  end,
}

castr.register_kind {
  name = "copilot",
  detect = function(p)
    return argv_has(p, "copilot")
  end,
  label = function(p)
    return "copilot · " .. p.cwd_basename
  end,
}

castr.register_kind {
  name = "term",
  detect = function(_p)
    return true
  end,
  label = function(p)
    return "term · " .. p.cwd_basename
  end,
}
"#;
