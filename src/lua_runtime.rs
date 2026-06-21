use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use anyhow::{Result, anyhow};
use mlua::{
    Function, Lua, ObjectLike, RegistryKey, Table, UserData, UserDataFields, UserDataMethods, Value,
};

use crate::process::ProcessInfo;
use crate::protocol::{PaneSnapshot, PanelCard, PanelView};
use crate::tmux::{self, PaneInfo};

pub struct LuaRuntime {
    lua: Lua,
    kinds: Rc<RefCell<Vec<Kind>>>,
    commands: Rc<RefCell<HashMap<String, RegistryKey>>>,
    events: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    keybinds: Rc<RefCell<Vec<Keybind>>>,
    panels: Rc<RefCell<Vec<Panel>>>,
    panes: Rc<RefCell<Vec<PaneSnapshot>>>,
}

struct Kind {
    name: String,
    detect: RegistryKey,
    label: RegistryKey,
    state: Option<RegistryKey>,
    color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybind {
    pub mode: String,
    pub key: String,
    pub command: Vec<String>,
    pub context: bool,
    pub popup: bool,
}

struct Panel {
    id: String,
    title: String,
    cards: RegistryKey,
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub kind: String,
    pub label: String,
    pub raw_state: Option<String>,
    pub color: Option<String>,
}

#[derive(Debug, Clone)]
struct LuaPane {
    id: String,
    pid: i32,
    cwd: String,
    cwd_basename: String,
    command: String,
    proc_tree: Vec<ProcessInfo>,
    window: String,
    session: String,
    active: bool,
    zoomed: bool,
    kind: String,
    label: String,
    tag: Option<String>,
    home: Option<String>,
    state: Option<String>,
}

#[derive(Debug, Clone)]
struct LuaProcTree(Vec<ProcessInfo>);

impl LuaRuntime {
    pub fn new(panes: Rc<RefCell<Vec<PaneSnapshot>>>) -> Result<Self> {
        let lua = Lua::new();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let commands = Rc::new(RefCell::new(HashMap::new()));
        let events = Rc::new(RefCell::new(HashMap::new()));
        let keybinds = Rc::new(RefCell::new(Vec::new()));
        let panels = Rc::new(RefCell::new(Vec::new()));
        let runtime = Self {
            lua,
            kinds,
            commands,
            events,
            keybinds,
            panels,
            panes,
        };
        runtime.install_api()?;
        runtime.load_prelude()?;
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
            command: pane.command.clone(),
            proc_tree,
            window: pane.window.clone(),
            session: pane.session.clone(),
            active: pane.active,
            zoomed: pane.zoomed,
            kind: String::new(),
            label: String::new(),
            tag: pane.tag.clone(),
            home: pane.home.clone(),
            state: pane.state.clone(),
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
                let raw_state = kind
                    .state
                    .as_ref()
                    .and_then(|state_key| self.lua.registry_value::<Function>(state_key).ok())
                    .and_then(|state_fn| state_fn.call::<String>(userdata.clone()).ok());
                return Ok(Some(Detection {
                    kind: kind.name.clone(),
                    label,
                    raw_state,
                    color: kind.color.clone(),
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
                let matcher: Option<String> = table.get("match")?;
                let detect: Function = match table.get::<Option<Function>>("detect")? {
                    Some(detect) => detect,
                    None => match matcher {
                        Some(pattern) => lua.create_function(move |_, pane: Value| match pane {
                            Value::Table(table) => {
                                let running: Function = table.get("running")?;
                                running.call((table, pattern.clone()))
                            }
                            Value::UserData(userdata) => {
                                userdata.call_method("running", pattern.clone())
                            }
                            _ => Ok(false),
                        })?,
                        None => {
                            return Err(mlua::Error::RuntimeError(
                                "kind requires detect or match".to_string(),
                            ));
                        }
                    },
                };
                let label: Function = match table.get::<Option<Function>>("label")? {
                    Some(label) => label,
                    None => {
                        let label = name.clone();
                        lua.create_function(move |_, _pane: Value| Ok(label.clone()))?
                    }
                };
                let detect = lua.create_registry_value(detect)?;
                let label = lua.create_registry_value(label)?;
                let state = table
                    .get::<Option<Function>>("state")?
                    .map(|state| lua.create_registry_value(state))
                    .transpose()?;
                let color = table.get::<Option<String>>("color")?;
                kinds.borrow_mut().push(Kind {
                    name,
                    detect,
                    label,
                    state,
                    color,
                });
                Ok(())
            })
            .map_err(lua_err)?;
        castr
            .set("register_kind", register_kind.clone())
            .map_err(lua_err)?;
        castr.set("kind", register_kind).map_err(lua_err)?;

        let commands = Rc::clone(&self.commands);
        let register_command = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let values = args.into_iter().collect::<Vec<_>>();
                let (name, handler) = match values.as_slice() {
                    [Value::Table(table)] => (table.get::<String>("name")?, table.get::<Function>("handler")?),
                    [Value::String(name), Value::Function(handler)] => {
                        (name.to_string_lossy(), handler.clone())
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "expected castr.command{name=..., handler=...} or castr.command(name, fn)"
                                .to_string(),
                        ));
                    }
                };
                let handler = lua.create_registry_value(handler)?;
                commands.borrow_mut().insert(name, handler);
                Ok(())
            })
            .map_err(lua_err)?;
        castr
            .set("command", register_command.clone())
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

        let keybinds = Rc::clone(&self.keybinds);
        let key_commands = Rc::clone(&self.commands);
        let key_panes = Rc::clone(&self.panes);
        let generated_key_command = Rc::new(Cell::new(0usize));
        let bind_key = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let keybind =
                    parse_bind_key(lua, &key_commands, &key_panes, &generated_key_command, args)?;
                keybinds.borrow_mut().push(keybind);
                Ok(())
            })
            .map_err(lua_err)?;
        castr.set("bind_key", bind_key).map_err(lua_err)?;

        let panels = Rc::clone(&self.panels);
        let register_panel = self
            .lua
            .create_function(move |lua, table: Table| {
                let id: String = table.get("id")?;
                let title: String = table.get("title")?;
                let cards: Function = table.get("cards")?;
                panels.borrow_mut().push(Panel {
                    id,
                    title,
                    cards: lua.create_registry_value(cards)?,
                });
                Ok(())
            })
            .map_err(lua_err)?;
        castr
            .set("panel", register_panel.clone())
            .map_err(lua_err)?;
        castr
            .set("register_panel", register_panel)
            .map_err(lua_err)?;

        let panes = Rc::clone(&self.panes);
        let panes_fn = self
            .lua
            .create_function(move |lua, ()| snapshots_table(lua, &panes.borrow()))
            .map_err(lua_err)?;
        castr.set("panes", panes_fn).map_err(lua_err)?;

        let pane_fn = self
            .lua
            .create_function(move |lua, pane_id: String| pane_ref_table(lua, &pane_id))
            .map_err(lua_err)?;
        castr.set("pane", pane_fn).map_err(lua_err)?;

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

    fn load_prelude(&self) -> Result<()> {
        self.load_source("prelude.lua", PRELUDE)
            .map_err(|error| anyhow!("failed to load Lua prelude: {error}"))
    }

    pub fn load_builtins(&self) -> Result<()> {
        self.load_source("builtin-kinds.lua", BUILTIN_KINDS)
            .map_err(|error| anyhow!("failed to load built-in Lua kinds: {error}"))
    }

    pub fn kind_count(&self) -> usize {
        self.kinds.borrow().len()
    }

    pub fn keybinds(&self) -> Vec<Keybind> {
        self.keybinds.borrow().clone()
    }

    pub fn render_panels(&self) -> Result<Vec<PanelView>> {
        let panels = {
            let panels = self.panels.borrow();
            panels
                .iter()
                .filter_map(|panel| {
                    self.lua
                        .registry_value::<Function>(&panel.cards)
                        .ok()
                        .map(|cards| (panel.id.clone(), panel.title.clone(), cards))
                })
                .collect::<Vec<_>>()
        };

        panels
            .into_iter()
            .map(|(id, title, cards_fn)| {
                let cards = cards_fn.call::<Table>(()).map_err(lua_err)?;
                Ok(PanelView {
                    id,
                    title,
                    cards: parse_panel_cards(cards).map_err(lua_err)?,
                })
            })
            .collect()
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
        let payload = pane
            .map(|pane| snapshot_table(&self.lua, pane).map(Value::Table))
            .unwrap_or(Ok(Value::Nil));
        self.fire_event_value(event, payload)
    }

    pub fn fire_event_text(&self, event: &str, text: &str) -> Vec<String> {
        self.fire_event_value(event, self.lua.create_string(text).map(Value::String))
    }

    fn fire_event_value(&self, event: &str, payload: mlua::Result<Value>) -> Vec<String> {
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

        let payload = match payload {
            Ok(value) => value,
            Err(error) => return vec![format!("event {event}: {error}")],
        };

        for handler in handlers {
            if let Err(error) = handler.call::<()>(payload.clone()) {
                errors.push(format!("event {event}: {error}"));
            }
        }
        errors
    }
}

pub fn user_plugin_files() -> Vec<PathBuf> {
    let config = config_dir();
    ensure_starter_config(&config);
    let mut files = Vec::new();
    collect_lua_files(&config, &mut files);
    files.sort();
    files
}

fn ensure_starter_config(config: &Path) {
    let mut existing = Vec::new();
    collect_lua_files(config, &mut existing);
    if !existing.is_empty() {
        return;
    }
    if fs::create_dir_all(config).is_err() {
        return;
    }
    for (name, source) in STARTER_FILES {
        let _ = fs::write(config.join(name), source);
    }
}

fn collect_lua_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_lua_files(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            files.push(path);
        }
    }
}

fn parse_panel_cards(cards: Table) -> mlua::Result<Vec<PanelCard>> {
    cards
        .sequence_values::<Table>()
        .map(|card| {
            let card = card?;
            Ok(PanelCard {
                title: card.get("title")?,
                subtitle: card.get("subtitle")?,
                state: card.get("state")?,
                tag: card.get("tag")?,
                pane: card.get("pane")?,
                enter: parse_optional_command(card.get("enter")?)?,
                expand: parse_optional_command(card.get("expand")?)?,
            })
        })
        .collect()
}

fn parse_optional_command(value: Value) -> mlua::Result<Option<Vec<String>>> {
    match value {
        Value::Nil => Ok(None),
        other => parse_keybind_command(other).map(Some),
    }
}

fn parse_bind_key(
    lua: &Lua,
    commands: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    panes: &Rc<RefCell<Vec<PaneSnapshot>>>,
    generated: &Rc<Cell<usize>>,
    args: mlua::MultiValue,
) -> mlua::Result<Keybind> {
    let values = args.into_iter().collect::<Vec<_>>();
    match values.as_slice() {
        [key, command] => Ok(Keybind {
            mode: "prefix".to_string(),
            key: value_to_string(key, "key")?,
            command: parse_bind_command_value(lua, commands, panes, generated, command.clone())?,
            context: true,
            popup: false,
        }),
        [key, command, opts] if !matches!(command, Value::String(_)) => {
            let (context, popup) = parse_keybind_opts(opts, true)?;
            Ok(Keybind {
                mode: "prefix".to_string(),
                key: value_to_string(key, "key")?,
                command: parse_bind_command_value(lua, commands, panes, generated, command.clone())?,
                context,
                popup,
            })
        }
        [mode, key, command] => Ok(Keybind {
            mode: value_to_string(mode, "mode")?,
            key: value_to_string(key, "key")?,
            command: parse_bind_command_value(lua, commands, panes, generated, command.clone())?,
            context: true,
            popup: false,
        }),
        [mode, key, command, opts, ..] => {
            let (context, popup) = parse_keybind_opts(opts, true)?;
            Ok(Keybind {
                mode: value_to_string(mode, "mode")?,
                key: value_to_string(key, "key")?,
                command: parse_bind_command_value(lua, commands, panes, generated, command.clone())?,
                context,
                popup,
            })
        }
        _ => Err(mlua::Error::RuntimeError(
            "expected castr.bind_key(key, command[, opts]) or castr.bind_key(table, key, command[, opts])"
                .to_string(),
        )),
    }
}

fn parse_bind_command_value(
    lua: &Lua,
    commands: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    panes: &Rc<RefCell<Vec<PaneSnapshot>>>,
    generated: &Rc<Cell<usize>>,
    value: Value,
) -> mlua::Result<Vec<String>> {
    match value {
        Value::Function(function) => {
            let idx = generated.get() + 1;
            generated.set(idx);
            let name = format!("__castr_key_{idx}");
            let panes = Rc::clone(panes);
            let handler = lua.create_function(move |lua, args: Table| {
                let pane = args
                    .get::<Option<String>>(1)?
                    .map(|id| pane_from_snapshot_or_id(lua, &panes.borrow(), &id))
                    .transpose()?
                    .unwrap_or(Value::Nil);
                function.call::<Value>((pane, args))
            })?;
            commands
                .borrow_mut()
                .insert(name.clone(), lua.create_registry_value(handler)?);
            Ok(vec![name])
        }
        other => parse_keybind_command(other),
    }
}

fn parse_keybind_opts(value: &Value, default_context: bool) -> mlua::Result<(bool, bool)> {
    match value {
        Value::Table(table) => {
            for key in table
                .clone()
                .pairs::<String, Value>()
                .map(|pair| pair.map(|(key, _)| key))
            {
                match key?.as_str() {
                    "popup" | "context" => {}
                    other => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "unknown bind_key option: {other}"
                        )));
                    }
                }
            }
            let popup = table.get::<Option<bool>>("popup")?.unwrap_or(false);
            let context = table.get::<Option<bool>>("context")?.unwrap_or(if popup {
                false
            } else {
                default_context
            });
            Ok((context, popup))
        }
        Value::Nil => Ok((default_context, false)),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected keybind opts table, got {other:?}"
        ))),
    }
}

fn value_to_string(value: &Value, name: &str) -> mlua::Result<String> {
    match value {
        Value::String(value) => Ok(value.to_string_lossy()),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected {name} string, got {other:?}"
        ))),
    }
}

fn parse_keybind_command(value: Value) -> mlua::Result<Vec<String>> {
    match value {
        Value::String(command) => Ok(command
            .to_string_lossy()
            .split_whitespace()
            .map(str::to_string)
            .collect()),
        Value::Table(table) => table.sequence_values::<String>().collect(),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected keybind command string or table, got {other:?}"
        ))),
    }
}

pub(crate) fn config_dir() -> PathBuf {
    env::var_os("CASTR_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| env::var_os("XDG_CONFIG_HOME").map(|home| PathBuf::from(home).join("castr")))
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/castr")))
        .unwrap_or_else(|| PathBuf::from(".config/castr"))
}

fn pane_ref_table(lua: &Lua, pane_id: &str) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("id", pane_id)?;
    add_pane_table_methods(lua, &table, pane_id, Vec::new())?;
    Ok(table)
}

fn pane_from_snapshot_or_id(
    lua: &Lua,
    panes: &[PaneSnapshot],
    pane_id: &str,
) -> mlua::Result<Value> {
    panes
        .iter()
        .find(|pane| pane.id == pane_id)
        .map(|pane| snapshot_table(lua, pane).map(Value::Table))
        .unwrap_or_else(|| pane_ref_table(lua, pane_id).map(Value::Table))
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
    table.set("cwd_basename", pane.cwd_basename.clone())?;
    table.set("command", pane.command.clone())?;
    table.set("session", pane.session.clone())?;
    table.set("window", pane.window.clone())?;
    table.set("active", pane.active)?;
    table.set("zoomed", pane.zoomed)?;
    table.set("tag", pane.tag.clone())?;
    table.set("home", pane.home.clone())?;
    table.set("state", pane.state.clone())?;
    add_pane_table_methods(lua, &table, &pane.id, pane.processes.clone())?;
    Ok(table)
}

fn add_pane_table_methods(
    lua: &Lua,
    table: &Table,
    pane_id: &str,
    processes: Vec<ProcessInfo>,
) -> mlua::Result<()> {
    let id = pane_id.to_string();
    table.set(
        "var",
        lua.create_function(move |_, (_self, name): (Table, String)| {
            tmux::get_pane_var(&id, &name).map_err(mlua_external)
        })?,
    )?;

    let id = pane_id.to_string();
    table.set(
        "capture",
        lua.create_function(move |_, _self: Table| tmux::capture(&id).map_err(mlua_external))?,
    )?;

    let id = pane_id.to_string();
    table.set(
        "set",
        lua.create_function(move |_, (_self, values): (Table, Table)| {
            set_pane_fields(&id, values)
        })?,
    )?;

    let tree = processes.clone();
    table.set(
        "proc_tree",
        lua.create_function(move |_, _self: Table| Ok(LuaProcTree(tree.clone())))?,
    )?;

    let tree = processes;
    table.set(
        "running",
        lua.create_function(move |_, (_self, pattern): (Table, String)| {
            Ok(process_running(&tree, &pattern))
        })?,
    )?;

    Ok(())
}

fn set_pane_fields(pane_id: &str, table: Table) -> mlua::Result<()> {
    for name in ["kind", "label", "state", "tag", "home"] {
        if let Some(value) = table.get::<Option<String>>(name)? {
            tmux::set_pane_var(pane_id, &format!("@castr_{name}"), &value)
                .map_err(mlua_external)?;
        }
    }
    if let Some(title) = table.get::<Option<String>>("title")? {
        tmux::set_pane_title(pane_id, &title).map_err(mlua_external)?;
    }
    Ok(())
}

fn tmux_api(lua: &Lua) -> Result<Table> {
    let table = lua.create_table().map_err(lua_err)?;
    table
        .set(
            "select",
            lua.create_function(|_, pane: Value| {
                let pane_id = pane_id_from_value(pane)?;
                tmux::select_pane(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "zoom",
            lua.create_function(|_, pane: Value| {
                let pane_id = pane_id_from_value(pane)?;
                tmux::zoom(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "unzoom",
            lua.create_function(|_, target: String| tmux::unzoom(&target).map_err(mlua_external))
                .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "capture",
            lua.create_function(|_, pane: Value| {
                let pane_id = pane_id_from_value(pane)?;
                tmux::capture(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "kill_pane",
            lua.create_function(|_, pane: Value| {
                let pane_id = pane_id_from_value(pane)?;
                tmux::kill_pane(&pane_id).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "current_pane",
            lua.create_function(|_, ()| tmux::current_pane().map_err(mlua_external))
                .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "active_pane",
            lua.create_function(|_, target: String| {
                tmux::active_pane(&target).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "is_zoomed",
            lua.create_function(|_, target: String| {
                tmux::is_zoomed(&target).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "window_id",
            lua.create_function(|_, target: String| {
                tmux::window_id(&target).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "display",
            lua.create_function(|_, opts: Table| {
                let target: String = opts.get("target")?;
                let message: String = opts.get("message")?;
                tmux::display_message(&target, &message).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "split",
            lua.create_function(|_, opts: Table| {
                let target: String = opts.get("target")?;
                let dir = opts
                    .get::<Option<String>>("dir")?
                    .or(opts.get::<Option<String>>("direction")?)
                    .unwrap_or_else(|| "right".to_string());
                let (direction, before) = split_direction(&dir)?;
                tmux::split(
                    &target,
                    tmux::SplitOptions {
                        direction,
                        before: before || opts.get::<Option<bool>>("before")?.unwrap_or(false),
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
            lua.create_function(|_, opts: Table| {
                tmux::stash(tmux::StashOptions {
                    pane: opts.get("pane")?,
                    window: opts.get("window")?,
                    cwd: opts.get("cwd")?,
                    name: opts.get("name").unwrap_or_else(|_| "castr".to_string()),
                })
                .map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "unstash",
            lua.create_function(|_, opts: Table| {
                tmux::unstash(tmux::UnstashOptions {
                    pane: opts.get("pane")?,
                    target: opts.get("target")?,
                    horizontal: opts.get::<Option<bool>>("horizontal")?.unwrap_or(true),
                    size: opts.get("size")?,
                })
                .map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    let cleanup = lua
        .create_function(|_, window: String| tmux::cleanup_stash(&window).map_err(mlua_external))
        .map_err(lua_err)?;
    table
        .set("cleanup_stash", cleanup.clone())
        .map_err(lua_err)?;
    table.set("cleanup", cleanup).map_err(lua_err)?;
    Ok(table)
}

fn split_direction(dir: &str) -> mlua::Result<(tmux::SplitDirection, bool)> {
    match dir {
        "right" | "h" | "horizontal" => Ok((tmux::SplitDirection::Horizontal, false)),
        "left" => Ok((tmux::SplitDirection::Horizontal, true)),
        "below" | "down" | "v" | "vertical" => Ok((tmux::SplitDirection::Vertical, false)),
        "above" | "up" => Ok((tmux::SplitDirection::Vertical, true)),
        other => Err(mlua::Error::RuntimeError(format!(
            "unknown split dir: {other}"
        ))),
    }
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
        let window = tmux::window_id(pane_id)?;
        let active_before = tmux::active_pane(&window)?;
        let zoomed_before = tmux::is_zoomed(&window)?;
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
        fields.add_field_method_get("command", |_, this| Ok(this.command.clone()));
        fields.add_field_method_get("window", |_, this| Ok(this.window.clone()));
        fields.add_field_method_get("session", |_, this| Ok(this.session.clone()));
        fields.add_field_method_get("active", |_, this| Ok(this.active));
        fields.add_field_method_get("zoomed", |_, this| Ok(this.zoomed));
        fields.add_field_method_get("kind", |_, this| Ok(this.kind.clone()));
        fields.add_field_method_get("label", |_, this| Ok(this.label.clone()));
        fields.add_field_method_get("tag", |_, this| Ok(this.tag.clone()));
        fields.add_field_method_get("home", |_, this| Ok(this.home.clone()));
        fields.add_field_method_get("state", |_, this| Ok(this.state.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("proc_tree", |_, this, ()| {
            Ok(LuaProcTree(this.proc_tree.clone()))
        });
        methods.add_method("running", |_, this, pattern: String| {
            Ok(process_running(&this.proc_tree, &pattern))
        });
        methods.add_method("var", |_, this, name: String| {
            tmux::get_pane_var(&this.id, &name).map_err(mlua_external)
        });
        methods.add_method("capture", |_, this, ()| {
            tmux::capture(&this.id).map_err(mlua_external)
        });
        methods.add_method("set", |_, this, table: Table| {
            set_pane_fields(&this.id, table)
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

fn process_running(processes: &[ProcessInfo], name: &str) -> bool {
    processes
        .iter()
        .any(|process| argv_has_command(&process.argv, name))
}

fn argv_has_command(argv: &str, name: &str) -> bool {
    argv.split_whitespace()
        .any(|token| command_matches(token, name))
}

fn command_matches(token: &str, name: &str) -> bool {
    token == name
        || std::path::Path::new(token)
            .file_name()
            .and_then(|part| part.to_str())
            == Some(name)
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

const PRELUDE: &str = r#"
castr._pane_defs = {}

function castr.register_pane(name, opts)
  opts.tag = opts.tag or name
  opts.name = opts.name or name
  castr._pane_defs[name] = opts
  return opts
end

local function pane_opts(opts)
  if type(opts) == "string" then return castr._pane_defs[opts] end
  return opts
end

function castr.find(query)
  for _, pane in ipairs(castr.panes()) do
    local ok = true
    for key, expected in pairs(query) do
      if pane[key] ~= expected then
        ok = false
        break
      end
    end
    if ok then return pane end
  end
end

function castr.find_all(query)
  local found = {}
  for _, pane in ipairs(castr.panes()) do
    local ok = true
    for key, expected in pairs(query) do
      if pane[key] ~= expected then
        ok = false
        break
      end
    end
    if ok then found[#found + 1] = pane end
  end
  return found
end

function castr.resolve(target)
  if type(target) == "string" then return target end
  if target and target.id then return target.id end
  local pane = castr.find(target)
  return pane and pane.id
end

function castr.split(pane, opts)
  local id = castr.tmux.split {
    target = castr.resolve(pane),
    dir = opts.dir or opts.direction,
    size = opts.size,
    cwd = opts.cwd,
    command = opts.command,
    detached = opts.detached,
  }
  local created = castr.pane(id)
  if opts.tag then created:set { tag = opts.tag } end
  return created
end

local function companion_query(from, opts)
  return { tag = opts.tag, window = from.window, home = from.window }
end

local function companion_horizontal(opts)
  return opts.dir == "right" or opts.dir == "left" or opts.dir == "h" or opts.dir == "horizontal"
end

local function show_companion(from, opts)
  local visible = castr.find(companion_query(from, opts))
  if visible then return visible end

  local hidden = castr.find { session = "__pi-hidden-" .. from.window, tag = opts.tag, home = from.window }
  if hidden then
    castr.tmux.unstash {
      pane = hidden.id,
      target = from.id,
      horizontal = companion_horizontal(opts),
      size = opts.size,
    }
    castr.tmux.select(hidden.id)
    return hidden
  end

  local pane = castr.split(from, {
    dir = opts.dir,
    size = opts.size,
    cwd = from.cwd,
    command = opts.command,
    detached = true,
    tag = opts.tag,
  })
  pane:set { home = from.window, title = opts.title, label = opts.label }
  castr.tmux.select(pane.id)
  return pane
end

local raw_toggle = function(target)
  local id = castr.resolve(target)
  if not id then return false end
  castr.tmux.zoom(id)
  return true
end

function castr.toggle(target, opts)
  if not opts then return raw_toggle(target) end
  opts = pane_opts(opts)
  if not opts then return false end

  local visible = castr.find(companion_query(target, opts))
  if not visible then
    show_companion(target, opts)
    return true
  end

  if visible.state == "blocked" and opts.blocked_message then
    castr.tmux.display { target = visible.id, message = opts.blocked_message }
    return false
  end

  castr.tmux.stash {
    pane = visible.id,
    window = target.window,
    cwd = target.cwd,
    name = opts.name or opts.tag,
  }
  return true
end

function castr.expand(target, opts)
  if opts then
    opts = pane_opts(opts)
    if not opts then return false end
    target = show_companion(target, opts)
  end

  local id = castr.resolve(target)
  if not id then return false end

  local window = castr.tmux.window_id(id)
  if castr.tmux.is_zoomed(window) and castr.tmux.active_pane(window) == id then
    castr.tmux.unzoom(window)
    return true
  end

  castr.tmux.unzoom(window)
  castr.tmux.select(id)
  castr.tmux.zoom(id)
  return true
end
"#;

const STARTER_FILES: &[(&str, &str)] = &[
    (
        "10-psql.lua",
        r#"castr.kind { name = "psql", match = "psql" }
"#,
    ),
    (
        "20-hello.lua",
        r#"castr.command {
  name = "hello",
  handler = function()
    return "hi"
  end,
}
"#,
    ),
    (
        "30-panel.lua",
        r#"castr.panel {
  id = "panes",
  title = "Panes",
  cards = function()
    local cards = {}
    for _, p in ipairs(castr.panes()) do
      cards[#cards + 1] = {
        title = p.label,
        subtitle = p.window,
        state = p.state,
        tag = p.tag or p.kind,
        pane = p.id,
      }
    end
    return cards
  end,
}
"#,
    ),
];

const BUILTIN_KINDS: &str = r#"
local function agent_state(p)
  local pushed = p:var("@castr_push_state")
  if pushed == "blocked" then return "blocked" end
  local out = p:capture()
  if out:match("esc to interrupt") or out:match("[Ww]orking through it") then return "working" end
  return "idle"
end

castr.kind {
  name = "pi",
  detect = function(p)
    return p:running("pi-coding-agent")
        or p:running("@earendil-works/pi")
        or p:running("pi")
  end,
  label = function(_p)
    return "pi"
  end,
  color = "yellow",
  state = agent_state,
}

castr.kind { name = "nvim", match = "nvim" }

castr.kind {
  name = "claude",
  match = "claude",
  label = function(_p)
    return "claude"
  end,
  color = "yellow",
  state = agent_state,
}

castr.kind {
  name = "copilot",
  match = "copilot",
  label = function(_p)
    return "copilot"
  end,
  color = "yellow",
  state = agent_state,
}

castr.kind {
  name = "pane",
  detect = function(_p)
    return true
  end,
  label = function(p)
    if p.tag == "terminal" then return "bottom" end
    return p.command
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
            cwd_basename: "castr".to_string(),
            command: "zsh".to_string(),
            session: "s".to_string(),
            window: "@1".to_string(),
            active: true,
            zoomed: false,
            tag: Some("terminal".to_string()),
            home: Some("@1".to_string()),
            state: Some("idle".to_string()),
            processes: vec![ProcessInfo {
                pid: 123,
                ppid: 1,
                argv: "zsh".to_string(),
            }],
        }
    }

    fn runtime() -> (LuaRuntime, Rc<RefCell<Vec<PaneSnapshot>>>) {
        let panes = Rc::new(RefCell::new(Vec::new()));
        (LuaRuntime::new(Rc::clone(&panes)).unwrap(), panes)
    }

    #[test]
    fn ensure_starter_config_writes_files_only_when_empty() {
        let root = std::env::temp_dir().join(format!("castr-starter-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        ensure_starter_config(&root);
        let mut files = Vec::new();
        collect_lua_files(&root, &mut files);
        assert_eq!(files.len(), STARTER_FILES.len());

        std::fs::write(root.join("custom.lua"), "-- custom").unwrap();
        ensure_starter_config(&root);
        let mut files = Vec::new();
        collect_lua_files(&root, &mut files);
        assert_eq!(files.len(), STARTER_FILES.len() + 1);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn collect_lua_files_recurses_under_config_dir() {
        let root = std::env::temp_dir().join(format!("castr-lua-files-{}", std::process::id()));
        let nested = root.join("anywhere/deeper");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(root.join("a.lua"), "").unwrap();
        std::fs::write(nested.join("b.lua"), "").unwrap();
        std::fs::write(nested.join("ignore.txt"), "").unwrap();

        let mut files = Vec::new();
        collect_lua_files(&root, &mut files);
        files.sort();
        let names = files
            .iter()
            .map(|path| path.file_name().unwrap().to_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(names, ["a.lua", "b.lua"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn split_direction_maps_user_words() {
        assert!(matches!(
            split_direction("below").unwrap(),
            (tmux::SplitDirection::Vertical, false)
        ));
        assert!(matches!(
            split_direction("above").unwrap(),
            (tmux::SplitDirection::Vertical, true)
        ));
        assert!(matches!(
            split_direction("right").unwrap(),
            (tmux::SplitDirection::Horizontal, false)
        ));
        assert!(matches!(
            split_direction("left").unwrap(),
            (tmux::SplitDirection::Horizontal, true)
        ));
    }

    #[test]
    fn bind_key_matches_tmux_shape() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.bind_key("a", { "pi" })
                castr.bind_key("A", { "pi", "expand" })
                castr.bind_key("root", "M-a", "pi expand")
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.keybinds(),
            vec![
                Keybind {
                    mode: "prefix".to_string(),
                    key: "a".to_string(),
                    command: vec!["pi".to_string()],
                    context: true,
                    popup: false,
                },
                Keybind {
                    mode: "prefix".to_string(),
                    key: "A".to_string(),
                    command: vec!["pi".to_string(), "expand".to_string()],
                    context: true,
                    popup: false,
                },
                Keybind {
                    mode: "root".to_string(),
                    key: "M-a".to_string(),
                    command: vec!["pi".to_string(), "expand".to_string()],
                    context: true,
                    popup: false,
                },
            ]
        );
    }

    #[test]
    fn register_pane_stores_reusable_config_without_overloading_pane_handle() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_pane("agent", { command = "pi" })
                castr.command("check", function()
                  local pane = castr.pane("%1")
                  local cfg = castr._pane_defs.agent
                  return pane.id .. ":" .. cfg.tag .. ":" .. cfg.name
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.run_command("check", &[]).unwrap().as_deref(),
            Some("%1:agent:agent")
        );
    }

    #[test]
    fn user_config_can_override_prelude_helpers() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                function castr.expand()
                  return "custom"
                end
                castr.command("check", function()
                  return castr.expand()
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.run_command("check", &[]).unwrap().as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn find_and_find_all_match_query_fields() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        let mut second = pane("%2");
        second.tag = Some("agent".to_string());
        second.window = "@2".to_string();
        panes.borrow_mut().push(second);

        runtime
            .load_source(
                "test.lua",
                r#"
                castr.command("query", function()
                  local one = castr.find{ tag = "agent" }
                  local all = castr.find_all{ active = true }
                  return one.id .. ":" .. #all
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.run_command("query", &[]).unwrap().as_deref(),
            Some("%2:2")
        );
    }

    #[test]
    fn bind_key_accepts_function_and_registers_internal_command() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.bind_key("root", "M-e", function()
                  return "ok"
                end)
                "#,
            )
            .unwrap();

        let keybind = &runtime.keybinds()[0];
        assert_eq!(keybind.mode, "root");
        assert_eq!(keybind.key, "M-e");
        assert_eq!(keybind.command, ["__castr_key_1"]);
        assert_eq!(
            runtime
                .run_command("__castr_key_1", &[])
                .unwrap()
                .as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn bind_key_rejects_unknown_options() {
        let (runtime, _) = runtime();
        let error = runtime
            .load_source(
                "test.lua",
                r#"castr.bind_key("a", { "pi" }, { desc = "unused" })"#,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown bind_key option: desc"));
    }

    #[test]
    fn registered_panel_renders_cards() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_panel{
                  id = "workspace",
                  title = "Workspace",
                  cards = function()
                    local p = castr.panes()[1]
                    return {{ title = p.label, subtitle = p.window, state = p.state, tag = p.tag, pane = p.id }}
                  end,
                }
                "#,
            )
            .unwrap();

        let panels = runtime.render_panels().unwrap();
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].id, "workspace");
        assert_eq!(panels[0].cards[0].pane.as_deref(), Some("%1"));
    }

    #[test]
    fn short_command_and_panel_names_are_primary_api() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.command {
                  name = "hello",
                  handler = function() return "hi" end,
                }
                castr.panel {
                  id = "main",
                  title = "Main",
                  cards = function() return {} end,
                }
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.run_command("hello", &[]).unwrap().as_deref(),
            Some("hi")
        );
        assert_eq!(runtime.render_panels().unwrap()[0].id, "main");
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
    fn running_matches_exact_token_or_basename_not_substrings() {
        let processes = vec![
            ProcessInfo {
                pid: 1,
                ppid: 0,
                argv: "pip install thing".to_string(),
            },
            ProcessInfo {
                pid: 2,
                ppid: 0,
                argv: "compile pi".to_string(),
            },
            ProcessInfo {
                pid: 3,
                ppid: 0,
                argv: "/usr/bin/psql -h localhost".to_string(),
            },
        ];

        assert!(!process_running(&processes[..1], "pi"));
        assert!(process_running(&processes, "pi"));
        assert!(process_running(&processes, "psql"));
        assert!(!process_running(&processes, "sql"));
    }

    #[test]
    fn declarative_kind_match_uses_running_helper_and_default_label() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"castr.kind { name = "shell", match = "zsh" }"#,
            )
            .unwrap();

        let detected = runtime
            .detect(
                &PaneInfo {
                    id: "%1".to_string(),
                    pid: 1,
                    cwd: "/tmp/work".to_string(),
                    command: "zsh".to_string(),
                    session: "s".to_string(),
                    window: "@1".to_string(),
                    active: true,
                    zoomed: false,
                    tag: None,
                    migrate_tag: false,
                    home: None,
                    state: None,
                },
                vec![ProcessInfo {
                    pid: 1,
                    ppid: 0,
                    argv: "zsh".to_string(),
                }],
            )
            .unwrap()
            .unwrap();
        assert_eq!(detected.kind, "shell");
        assert_eq!(detected.label, "shell");
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
                    return panes[1].id .. ":" .. panes[1].kind .. ":" .. panes[1].pid .. ":" .. panes[1].tag .. ":" .. panes[1].home .. ":" .. panes[1].state
                  end,
                }
                "#,
            )
            .unwrap();

        let out = runtime.run_command("pane_id", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("%1:term:123:terminal:@1:idle"));
    }

    #[test]
    fn pane_ref_exposes_methods_for_fresh_split_ids() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "method_types",
                  handler = function()
                    local p = castr.pane("%9")
                    return p.id .. ":" .. type(p.set) .. ":" .. type(p.var) .. ":" .. type(p.capture)
                  end,
                }
                "#,
            )
            .unwrap();

        let out = runtime.run_command("method_types", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("%9:function:function:function"));
    }

    #[test]
    fn pane_tables_expose_running_and_proc_tree() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "running",
                  handler = function()
                    local p = castr.panes()[1]
                    return tostring(p:running("zsh")) .. ":" .. p:proc_tree():list()[1].argv .. ":" .. p.cwd_basename
                  end,
                }
                "#,
            )
            .unwrap();

        let out = runtime.run_command("running", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("true:zsh:castr"));
    }

    #[test]
    fn pane_tables_expose_methods() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                castr.register_command{
                  name = "method_types",
                  handler = function()
                    local p = castr.panes()[1]
                    return type(p.set) .. ":" .. type(p.var) .. ":" .. type(p.capture)
                  end,
                }
                "#,
            )
            .unwrap();

        let out = runtime.run_command("method_types", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("function:function:function"));
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
    fn text_events_pass_plain_string_payload() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                seen = ""
                castr.on("window:close", function(window) seen = window end)
                castr.register_command{
                  name = "seen",
                  handler = function() return seen end,
                }
                "#,
            )
            .unwrap();

        assert!(runtime.fire_event_text("window:close", "@9").is_empty());
        let out = runtime.run_command("seen", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("@9"));
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
                    command: "zsh".to_string(),
                    session: "s".to_string(),
                    window: "@1".to_string(),
                    active: true,
                    zoomed: false,
                    tag: None,
                    migrate_tag: false,
                    home: None,
                    state: None,
                },
                Vec::new(),
            )
            .unwrap()
            .unwrap();
        assert_eq!(detected.kind, "ok");
        assert_eq!(detected.label, "ok · work");
    }
}
