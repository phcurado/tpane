use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use mlua::{Function, Lua, RegistryKey, Table, UserData, UserDataFields, UserDataMethods, Value};

use crate::process::ProcessInfo;
use crate::protocol::PaneSnapshot;
use crate::tmux::{self, PaneInfo};

pub struct LuaRuntime {
    lua: Lua,
    kinds: Rc<RefCell<Vec<Kind>>>,
    commands: Rc<RefCell<HashMap<String, RegistryKey>>>,
    events: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    panes: Rc<RefCell<Vec<PaneSnapshot>>>,
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
    window: String,
    session: String,
    active: bool,
    zoomed: bool,
    kind: String,
    label: String,
}

#[derive(Debug, Clone)]
struct LuaProcTree(Vec<ProcessInfo>);

impl LuaRuntime {
    pub fn new(panes: Rc<RefCell<Vec<PaneSnapshot>>>) -> Result<Self> {
        let lua = Lua::new();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let commands = Rc::new(RefCell::new(HashMap::new()));
        let events = Rc::new(RefCell::new(HashMap::new()));
        let runtime = Self {
            lua,
            kinds,
            commands,
            events,
            panes,
        };
        runtime.install_api()?;
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
            window: pane.window.clone(),
            session: pane.session.clone(),
            active: pane.active,
            zoomed: pane.zoomed,
            kind: String::new(),
            label: String::new(),
        };
        let userdata = self.lua.create_userdata(lua_pane).map_err(lua_err)?;

        for kind in self.kinds.borrow().iter() {
            let Ok(detect) = self.lua.registry_value::<Function>(&kind.detect) else {
                continue;
            };
            let Ok(matched) = detect.call::<bool>(userdata.clone()) else {
                continue;
            };
            if matched {
                let Ok(label_fn) = self.lua.registry_value::<Function>(&kind.label) else {
                    continue;
                };
                let Ok(label) = label_fn.call::<String>(userdata.clone()) else {
                    continue;
                };
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

        let commands = Rc::clone(&self.commands);
        let register_command = self
            .lua
            .create_function(move |lua, table: Table| {
                let name: String = table.get("name")?;
                let handler: Function = table.get("handler")?;
                let handler = lua.create_registry_value(handler)?;
                commands.borrow_mut().insert(name, handler);
                Ok(())
            })
            .map_err(lua_err)?;
        castr
            .set("register_command", register_command)
            .map_err(lua_err)?;

        let events = Rc::clone(&self.events);
        let on = self
            .lua
            .create_function(move |lua, (event, handler): (String, Function)| {
                let handler = lua.create_registry_value(handler)?;
                events.borrow_mut().entry(event).or_default().push(handler);
                Ok(())
            })
            .map_err(lua_err)?;
        castr.set("on", on).map_err(lua_err)?;

        let panes = Rc::clone(&self.panes);
        let panes_fn = self
            .lua
            .create_function(move |lua, ()| snapshots_table(lua, &panes.borrow()))
            .map_err(lua_err)?;
        castr.set("panes", panes_fn).map_err(lua_err)?;

        castr.set("tmux", tmux_api(&self.lua)?).map_err(lua_err)?;
        castr
            .set("with_pane", with_pane_fn(&self.lua)?)
            .map_err(lua_err)?;
        self.lua.globals().set("castr", castr).map_err(lua_err)?;
        Ok(())
    }

    pub fn load_source(&self, name: &str, source: &str) -> Result<()> {
        self.lua
            .load(source)
            .set_name(name)
            .exec()
            .map_err(|error| anyhow!("{error}"))
    }

    pub fn load_builtins(&self) -> Result<()> {
        self.load_source("builtin-kinds.lua", BUILTIN_KINDS)
            .map_err(|error| anyhow!("failed to load built-in Lua kinds: {error}"))
    }

    pub fn kind_count(&self) -> usize {
        self.kinds.borrow().len()
    }

    pub fn run_command(&self, name: &str, args: &[String]) -> Result<Option<String>> {
        let handler: Function = {
            let commands = self.commands.borrow();
            let Some(handler_key) = commands.get(name) else {
                anyhow::bail!("unknown command: {name}");
            };
            self.lua.registry_value(handler_key).map_err(lua_err)?
        };
        let arg_table = self.lua.create_table().map_err(lua_err)?;
        for (idx, arg) in args.iter().enumerate() {
            arg_table.set(idx + 1, arg.as_str()).map_err(lua_err)?;
        }
        let value = handler.call::<Value>(arg_table).map_err(lua_err)?;
        Ok(match value {
            Value::Nil => None,
            Value::String(value) => Some(value.to_string_lossy()),
            other => Some(format!("{other:?}")),
        })
    }

    pub fn fire_event(&self, event: &str, pane: Option<&PaneSnapshot>) -> Vec<String> {
        let mut errors = Vec::new();
        let handlers: Vec<Function> = {
            let events = self.events.borrow();
            let Some(handler_keys) = events.get(event) else {
                return Vec::new();
            };
            handler_keys
                .iter()
                .filter_map(
                    |handler_key| match self.lua.registry_value::<Function>(handler_key) {
                        Ok(handler) => Some(handler),
                        Err(error) => {
                            errors.push(format!("event {event}: {error}"));
                            None
                        }
                    },
                )
                .collect()
        };

        let pane_value = pane
            .map(|pane| snapshot_table(&self.lua, pane).map(Value::Table))
            .unwrap_or(Ok(Value::Nil));
        let pane_value = match pane_value {
            Ok(value) => value,
            Err(error) => return vec![format!("event {event}: {error}")],
        };

        for handler in handlers {
            if let Err(error) = handler.call::<()>(pane_value.clone()) {
                errors.push(format!("event {event}: {error}"));
            }
        }
        errors
    }
}

pub fn user_plugin_files() -> Vec<PathBuf> {
    let config = config_dir();
    let mut files = Vec::new();
    for dir in [
        config.clone(),
        config.join("kinds"),
        config.join("panels"),
        config.join("commands"),
    ] {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        files.extend(
            entries
                .filter_map(std::result::Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("lua")),
        );
    }
    files.sort();
    files
}

pub(crate) fn config_dir() -> PathBuf {
    env::var_os("CASTR_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_CONFIG_HOME").map(|home| PathBuf::from(home).join("castr")))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/castr")))
        .unwrap_or_else(|| PathBuf::from(".config/castr"))
}

fn snapshots_table(lua: &Lua, panes: &[PaneSnapshot]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    for (idx, pane) in panes.iter().enumerate() {
        table.set(idx + 1, snapshot_table(lua, pane)?)?;
    }
    Ok(table)
}

fn snapshot_table(lua: &Lua, pane: &PaneSnapshot) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", pane.id.clone())?;
    table.set("pid", pane.pid)?;
    table.set("kind", pane.kind.clone())?;
    table.set("label", pane.label.clone())?;
    table.set("cwd", pane.cwd.clone())?;
    table.set("session", pane.session.clone())?;
    table.set("window", pane.window.clone())?;
    table.set("active", pane.active)?;
    table.set("zoomed", pane.zoomed)?;
    Ok(table)
}

fn tmux_api(lua: &Lua) -> Result<Table> {
    let table = lua.create_table().map_err(lua_err)?;
    table
        .set(
            "select",
            lua.create_function(|_, pane_id: String| {
                tmux::select_pane(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "zoom",
            lua.create_function(|_, pane_id: String| tmux::zoom(&pane_id).map_err(mlua_external))
                .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "capture",
            lua.create_function(|_, pane_id: String| {
                tmux::capture(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "split",
            lua.create_function(|_, opts: Table| {
                let target: String = opts.get("target")?;
                let direction = match opts.get::<Option<String>>("direction")?.as_deref() {
                    Some("v") | Some("vertical") => tmux::SplitDirection::Vertical,
                    _ => tmux::SplitDirection::Horizontal,
                };
                tmux::split(
                    &target,
                    tmux::SplitOptions {
                        direction,
                        size: opts.get("size")?,
                        cwd: opts.get("cwd")?,
                        command: opts.get("command")?,
                        detached: opts.get::<Option<bool>>("detached")?.unwrap_or(false),
                    },
                )
                .map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "join",
            lua.create_function(|_, opts: Table| {
                let src: String = opts.get("src")?;
                let target: String = opts.get("target")?;
                tmux::join(
                    &src,
                    &target,
                    tmux::JoinOptions {
                        horizontal: opts.get::<Option<bool>>("horizontal")?.unwrap_or(true),
                        size: opts.get("size")?,
                    },
                )
                .map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "break_pane",
            lua.create_function(|_, opts: Table| {
                let pane: String = opts.get("pane")?;
                let session: String = opts.get("session")?;
                let name: String = opts.get("name").unwrap_or_else(|_| "castr".to_string());
                tmux::break_pane(&pane, &session, &name).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "break",
            table.get::<Function>("break_pane").map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "stash",
            table.get::<Function>("break_pane").map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set("unstash", table.get::<Function>("join").map_err(lua_err)?)
        .map_err(lua_err)?;
    Ok(table)
}

fn with_pane_fn(lua: &Lua) -> Result<Function> {
    lua.create_function(|_, (pane, opts, body): (Value, Table, Function)| {
        let pane_id = pane_id_from_value(pane)?;
        let _guard = PaneGuard::stage(&pane_id, &opts).map_err(mlua_external)?;
        body.call::<Value>(())
    })
    .map_err(lua_err)
}

struct PaneGuard {
    pane_id: String,
    active_before: String,
    zoomed_before: bool,
}

impl PaneGuard {
    fn stage(pane_id: &str, opts: &Table) -> Result<Self> {
        let active_before = tmux::active_pane(pane_id)?;
        let zoomed_before = tmux::is_zoomed(pane_id)?;
        tmux::select_pane(pane_id)?;
        if opts
            .get::<Option<bool>>("zoom")
            .map_err(lua_err)?
            .unwrap_or(false)
            && !tmux::is_zoomed(pane_id)?
        {
            tmux::zoom(pane_id)?;
        }
        if let Some(state) = opts.get::<Option<String>>("state").map_err(lua_err)? {
            tmux::set_pane_var(pane_id, "@castr_state", &state)?;
        }
        Ok(Self {
            pane_id: pane_id.to_string(),
            active_before,
            zoomed_before,
        })
    }
}

impl Drop for PaneGuard {
    fn drop(&mut self) {
        let _ = tmux::select_pane(&self.pane_id);
        if let Ok(current_zoomed) = tmux::is_zoomed(&self.pane_id) {
            if current_zoomed != self.zoomed_before {
                let _ = tmux::zoom(&self.pane_id);
            }
        }
        let _ = tmux::select_pane(&self.active_before);
    }
}

fn pane_id_from_value(value: Value) -> mlua::Result<String> {
    match value {
        Value::String(value) => Ok(value.to_string_lossy()),
        Value::Table(table) => table.get("id"),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected pane id string or pane table, got {other:?}"
        ))),
    }
}

fn mlua_external(error: anyhow::Error) -> mlua::Error {
    mlua::Error::RuntimeError(error.to_string())
}

impl UserData for LuaPane {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("id", |_, this| Ok(this.id.clone()));
        fields.add_field_method_get("pid", |_, this| Ok(this.pid));
        fields.add_field_method_get("cwd", |_, this| Ok(this.cwd.clone()));
        fields.add_field_method_get("cwd_basename", |_, this| Ok(this.cwd_basename.clone()));
        fields.add_field_method_get("window", |_, this| Ok(this.window.clone()));
        fields.add_field_method_get("session", |_, this| Ok(this.session.clone()));
        fields.add_field_method_get("active", |_, this| Ok(this.active));
        fields.add_field_method_get("zoomed", |_, this| Ok(this.zoomed));
        fields.add_field_method_get("kind", |_, this| Ok(this.kind.clone()));
        fields.add_field_method_get("label", |_, this| Ok(this.label.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("proc_tree", |_, this, ()| {
            Ok(LuaProcTree(this.proc_tree.clone()))
        });
        methods.add_method("var", |_, this, name: String| {
            tmux::get_pane_var(&this.id, &name).map_err(mlua_external)
        });
        methods.add_method("capture", |_, this, ()| {
            tmux::capture(&this.id).map_err(mlua_external)
        });
        methods.add_method("set", |_, this, table: Table| {
            for name in ["kind", "label", "state", "role"] {
                if let Some(value) = table.get::<Option<String>>(name)? {
                    tmux::set_pane_var(&this.id, &format!("@castr_{name}"), &value)
                        .map_err(mlua_external)?;
                }
            }
            Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> PaneSnapshot {
        PaneSnapshot {
            id: id.to_string(),
            pid: 123,
            kind: "term".to_string(),
            label: "term · castr".to_string(),
            cwd: "/tmp/castr".to_string(),
            session: "s".to_string(),
            window: "1:w".to_string(),
            active: true,
            zoomed: false,
        }
    }

    fn runtime() -> (LuaRuntime, Rc<RefCell<Vec<PaneSnapshot>>>) {
        let panes = Rc::new(RefCell::new(Vec::new()));
        (LuaRuntime::new(Rc::clone(&panes)).unwrap(), panes)
    }

    #[test]
    fn registered_command_returns_string_and_receives_args() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "hello",
                  handler = function(args) return "hi " .. args[1] end,
                }
                "#,
            )
            .unwrap();

        let out = runtime
            .run_command("hello", &["there".to_string()])
            .unwrap();
        assert_eq!(out.as_deref(), Some("hi there"));
    }

    #[test]
    fn unknown_and_throwing_commands_return_errors() {
        let (runtime, _) = runtime();
        assert!(runtime.run_command("missing", &[]).is_err());

        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "boom",
                  handler = function() error("nope") end,
                }
                "#,
            )
            .unwrap();

        let error = runtime.run_command("boom", &[]).unwrap_err().to_string();
        assert!(error.contains("nope"));
    }

    #[test]
    fn command_can_register_command_without_refcell_panic() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "outer",
                  handler = function()
                    castr.register_command{
                      name = "inner",
                      handler = function() return "inner ok" end,
                    }
                    return "outer ok"
                  end,
                }
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.run_command("outer", &[]).unwrap().as_deref(),
            Some("outer ok")
        );
        assert_eq!(
            runtime.run_command("inner", &[]).unwrap().as_deref(),
            Some("inner ok")
        );
    }

    #[test]
    fn event_can_register_event_without_refcell_panic() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.on("tick", function()
                  castr.on("tick", function() end)
                end)
                "#,
            )
            .unwrap();

        assert!(runtime.fire_event("tick", None).is_empty());
    }

    #[test]
    fn panes_reads_shared_live_state() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "pane_id",
                  handler = function()
                    local panes = castr.panes()
                    return panes[1].id .. ":" .. panes[1].kind .. ":" .. panes[1].pid
                  end,
                }
                "#,
            )
            .unwrap();

        let out = runtime.run_command("pane_id", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("%1:term:123"));
    }

    #[test]
    fn events_call_handlers_and_collect_errors() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                seen = ""
                castr.on("pane:new", function(p) seen = p.id end)
                castr.on("pane:new", function() error("bad event") end)
                castr.register_command{
                  name = "seen",
                  handler = function() return seen end,
                }
                "#,
            )
            .unwrap();

        let errors = runtime.fire_event("pane:new", Some(&pane("%9")));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad event"));
        let out = runtime.run_command("seen", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("%9"));
    }

    #[test]
    fn detect_skips_throwing_kind_and_uses_next_match() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_kind{
                  name = "broken",
                  detect = function() error("bad detect") end,
                  label = function() return "broken" end,
                }
                castr.register_kind{
                  name = "ok",
                  detect = function() return true end,
                  label = function(p) return "ok · " .. p.cwd_basename end,
                }
                "#,
            )
            .unwrap();

        let detected = runtime
            .detect(
                &PaneInfo {
                    id: "%1".to_string(),
                    pid: 1,
                    cwd: "/tmp/work".to_string(),
                    session: "s".to_string(),
                    window: "1:w".to_string(),
                    active: true,
                    zoomed: false,
                },
                Vec::new(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(detected.kind, "ok");
        assert_eq!(detected.label, "ok · work");
    }
}
