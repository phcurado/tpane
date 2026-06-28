use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use mlua::{
    Function, Lua, ObjectLike, RegistryKey, Table, UserData, UserDataFields, UserDataMethods, Value,
};
use serde_json::{Map as JsonMap, Number as JsonNumber, Value as JsonValue};

use crate::plugins::{self, PluginSpec};
use crate::process::ProcessInfo;
use crate::protocol::{PaneSnapshot, PanelCard, PanelView};
use crate::store::Store;
use crate::tmux::{self, PaneInfo};

pub struct LuaRuntime {
    lua: Lua,
    kinds: Rc<RefCell<Vec<Kind>>>,
    commands: Rc<RefCell<HashMap<String, RegistryKey>>>,
    events: Rc<RefCell<HashMap<String, Vec<RegistryKey>>>>,
    deferred: Rc<RefCell<Vec<RegistryKey>>>,
    keybinds: Rc<RefCell<Vec<Keybind>>>,
    unbinds: Rc<RefCell<Vec<Unbind>>>,
    panels: Rc<RefCell<Vec<Panel>>>,
    widgets: Rc<RefCell<HashMap<String, RegistryKey>>>,
    pane_border: Rc<RefCell<Option<RegistryKey>>>,
    states: Rc<RefCell<HashMap<String, StatePresentation>>>,
    statusline: Rc<RefCell<Option<StatusLineDef>>>,
    options: Rc<RefCell<Vec<(String, String)>>>,
    option_appends: Rc<RefCell<Vec<(String, String)>>>,
    jobs: Rc<RefCell<Vec<JobDef>>>,
    job_data: Rc<RefCell<HashMap<String, String>>>,
    used_plugins: Rc<RefCell<HashMap<String, PluginSpec>>>,
    load_plugins: bool,
    panes: Rc<RefCell<Vec<PaneSnapshot>>>,
    store: Rc<RefCell<Store>>,
}

struct Kind {
    name: String,
    detect: RegistryKey,
    label: RegistryKey,
    state: Option<RegistryKey>,
    color: Option<String>,
    tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keybind {
    pub mode: String,
    pub key: String,
    pub command: Vec<String>,
    pub raw: bool,
    pub context: bool,
    pub popup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unbind {
    pub mode: String,
    pub key: String,
}

struct Panel {
    id: String,
    title: String,
    cards: RegistryKey,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatePresentation {
    pub color: Option<String>,
    pub glyph: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusLineDef {
    position: Option<String>,
    interval: Option<u64>,
    left: Option<Vec<StatusItem>>,
    right: Option<Vec<StatusItem>>,
    rows: Vec<StatusRowDef>,
    separator: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StatusRowDef {
    left: Option<Vec<StatusItem>>,
    right: Option<Vec<StatusItem>>,
    separator: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StatusItem {
    Widget(String),
    Job(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StatusRender {
    pub active: bool,
    pub position: Option<String>,
    pub interval: Option<u64>,
    pub rows: Option<usize>,
    pub left: Option<String>,
    pub right: Option<String>,
    pub formats: Vec<(usize, String)>,
}

#[derive(Debug, Clone)]
pub struct Detection {
    pub kind: String,
    pub label: String,
    pub raw_state: Option<String>,
    pub color: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobDef {
    pub name: String,
    pub every: Duration,
    pub timeout: Duration,
    pub command: String,
}

#[derive(Clone)]
struct LuaJob {
    name: String,
    data: Rc<RefCell<HashMap<String, String>>>,
}

#[derive(Clone)]
struct LuaWidget {
    name: String,
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
        Self::with_store(panes, Rc::new(RefCell::new(Store::memory())))
    }

    pub fn collector(panes: Rc<RefCell<Vec<PaneSnapshot>>>) -> Result<Self> {
        Self::with_store_and_plugin_loading(panes, Rc::new(RefCell::new(Store::memory())), false)
    }

    pub fn with_store(
        panes: Rc<RefCell<Vec<PaneSnapshot>>>,
        store: Rc<RefCell<Store>>,
    ) -> Result<Self> {
        Self::with_store_and_data(panes, store, Rc::new(RefCell::new(HashMap::new())))
    }

    pub fn with_store_and_data(
        panes: Rc<RefCell<Vec<PaneSnapshot>>>,
        store: Rc<RefCell<Store>>,
        job_data: Rc<RefCell<HashMap<String, String>>>,
    ) -> Result<Self> {
        Self::with_store_data_and_plugin_loading(panes, store, job_data, true)
    }

    fn with_store_and_plugin_loading(
        panes: Rc<RefCell<Vec<PaneSnapshot>>>,
        store: Rc<RefCell<Store>>,
        load_plugins: bool,
    ) -> Result<Self> {
        Self::with_store_data_and_plugin_loading(
            panes,
            store,
            Rc::new(RefCell::new(HashMap::new())),
            load_plugins,
        )
    }

    fn with_store_data_and_plugin_loading(
        panes: Rc<RefCell<Vec<PaneSnapshot>>>,
        store: Rc<RefCell<Store>>,
        job_data: Rc<RefCell<HashMap<String, String>>>,
        load_plugins: bool,
    ) -> Result<Self> {
        let lua = Lua::new();
        let kinds = Rc::new(RefCell::new(Vec::new()));
        let commands = Rc::new(RefCell::new(HashMap::new()));
        let events = Rc::new(RefCell::new(HashMap::new()));
        let deferred = Rc::new(RefCell::new(Vec::new()));
        let keybinds = Rc::new(RefCell::new(Vec::new()));
        let unbinds = Rc::new(RefCell::new(Vec::new()));
        let panels = Rc::new(RefCell::new(Vec::new()));
        let widgets = Rc::new(RefCell::new(HashMap::new()));
        let pane_border = Rc::new(RefCell::new(None));
        let states = Rc::new(RefCell::new(HashMap::new()));
        let statusline = Rc::new(RefCell::new(None));
        let options = Rc::new(RefCell::new(Vec::new()));
        let option_appends = Rc::new(RefCell::new(Vec::new()));
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let used_plugins = Rc::new(RefCell::new(HashMap::new()));
        let runtime = Self {
            lua,
            kinds,
            commands,
            events,
            deferred,
            keybinds,
            unbinds,
            panels,
            widgets,
            pane_border,
            states,
            statusline,
            options,
            option_appends,
            jobs,
            job_data,
            used_plugins,
            load_plugins,
            panes,
            store,
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
                    tag: kind.tag.clone(),
                }));
            }
        }

        Ok(None)
    }

    fn install_api(&self) -> Result<()> {
        let tpane = self.lua.create_table().map_err(lua_err)?;
        let use_plugin_loaded = Rc::clone(&self.used_plugins);
        let load_plugins = self.load_plugins;
        let use_plugin = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let values = args.into_iter().collect::<Vec<_>>();
                let (name, spec) = match values.as_slice() {
                    [Value::String(name)] => (name.to_string_lossy(), PluginSpec::default()),
                    [Value::String(name), Value::Table(table)] => {
                        (name.to_string_lossy(), plugin_spec_from_lua(table)?)
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "expected tpane.use(name[, opts])".to_string(),
                        ));
                    }
                };
                if use_plugin_loaded.borrow().contains_key(&name) {
                    return Ok(());
                }
                if load_plugins {
                    load_plugin(lua, &name, &spec)?;
                } else {
                    plugins::validate_plugin_name(&name).map_err(mlua_external)?;
                    plugins::validate_spec(&spec).map_err(mlua_external)?;
                }
                use_plugin_loaded.borrow_mut().insert(name, spec);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("use", use_plugin).map_err(lua_err)?;

        let kinds = Rc::clone(&self.kinds);
        let kind = self
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
                let tag = table.get::<Option<String>>("tag")?;
                kinds.borrow_mut().push(Kind {
                    name,
                    detect,
                    label,
                    state,
                    color,
                    tag,
                });
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("kind", kind).map_err(lua_err)?;

        let states = Rc::clone(&self.states);
        let state = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let values = args.into_iter().collect::<Vec<_>>();
                match values.as_slice() {
                    [Value::String(name)] => {
                        let name = name.to_string_lossy();
                        match states.borrow().get(&name) {
                            Some(presentation) => state_presentation_table(lua, presentation),
                            None => Ok(Value::Nil),
                        }
                    }
                    [Value::String(name), Value::Table(table)] => {
                        let name = name.to_string_lossy();
                        let presentation = StatePresentation {
                            color: table.get("color")?,
                            glyph: table.get("glyph")?,
                        };
                        states.borrow_mut().insert(name, presentation);
                        Ok(Value::Nil)
                    }
                    _ => Err(mlua::Error::RuntimeError(
                        "expected tpane.state(name) or tpane.state(name, { color=..., glyph=... })"
                            .to_string(),
                    )),
                }
            })
            .map_err(lua_err)?;
        tpane.set("state", state).map_err(lua_err)?;

        #[cfg(test)]
        {
            let commands = Rc::clone(&self.commands);
            let generated_command = Rc::new(Cell::new(0usize));
            let command = self
                .lua
                .create_function(move |lua, handler: Function| {
                    let idx = generated_command.get() + 1;
                    generated_command.set(idx);
                    let name = format!("__tpane_command_{idx}");
                    let handler = lua.create_registry_value(handler)?;
                    commands.borrow_mut().insert(name.clone(), handler);
                    run_action_table(lua, &[name])
                })
                .map_err(lua_err)?;
            tpane.set("command", command).map_err(lua_err)?;
        }

        let events = Rc::clone(&self.events);
        let on = self
            .lua
            .create_function(move |lua, (event, handler): (String, Function)| {
                let handler = lua.create_registry_value(handler)?;
                events.borrow_mut().entry(event).or_default().push(handler);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("on", on).map_err(lua_err)?;

        let deferred = Rc::clone(&self.deferred);
        let defer = self
            .lua
            .create_function(move |lua, handler: Function| {
                deferred
                    .borrow_mut()
                    .push(lua.create_registry_value(handler)?);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("_defer", defer).map_err(lua_err)?;

        let keybinds = Rc::clone(&self.keybinds);
        let key_commands = Rc::clone(&self.commands);
        let key_panes = Rc::clone(&self.panes);
        let generated_key_command = Rc::new(Cell::new(0usize));
        let bind = self
            .lua
            .create_function(move |lua, args: mlua::MultiValue| {
                let keybind =
                    parse_bind(lua, &key_commands, &key_panes, &generated_key_command, args)?;
                keybinds.borrow_mut().push(keybind);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("bind", bind).map_err(lua_err)?;

        let unbinds = Rc::clone(&self.unbinds);
        let unbind = self
            .lua
            .create_function(move |_, args: mlua::MultiValue| {
                let values = args.into_iter().collect::<Vec<_>>();
                let (key, mode) = match values.as_slice() {
                    [key] => (value_to_string(key, "key")?, "prefix".to_string()),
                    [key, opts] => {
                        let opts = parse_keybind_opts(opts, false)?;
                        (value_to_string(key, "key")?, opts.mode)
                    }
                    _ => {
                        return Err(mlua::Error::RuntimeError(
                            "expected tpane.unbind(key[, opts])".to_string(),
                        ));
                    }
                };
                unbinds.borrow_mut().push(Unbind { mode, key });
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("unbind", unbind).map_err(lua_err)?;

        let panels = Rc::clone(&self.panels);
        let panel = self
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
        tpane.set("panel", panel).map_err(lua_err)?;

        let widget_handles = self.lua.create_table().map_err(lua_err)?;
        tpane.set("widgets", widget_handles).map_err(lua_err)?;
        let widgets = Rc::clone(&self.widgets);
        let widget_counter = Rc::new(Cell::new(0));
        let widget = self
            .lua
            .create_function(move |lua, handler: Function| {
                let id = widget_counter.get() + 1;
                widget_counter.set(id);
                let name = format!("__tpane_widget_{id}");
                widgets
                    .borrow_mut()
                    .insert(name.clone(), lua.create_registry_value(handler)?);
                lua.create_userdata(LuaWidget { name })
            })
            .map_err(lua_err)?;
        tpane.set("widget", widget).map_err(lua_err)?;

        let pane_border = Rc::clone(&self.pane_border);
        let pane_border_fn = self
            .lua
            .create_function(move |lua, handler: Function| {
                *pane_border.borrow_mut() = Some(lua.create_registry_value(handler)?);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("pane_border", pane_border_fn).map_err(lua_err)?;

        let statusline = Rc::clone(&self.statusline);
        let set_statusline = self
            .lua
            .create_function(move |_, table: Table| {
                let def = parse_statusline_def(table)?;
                *statusline.borrow_mut() = Some(def);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("statusline", set_statusline).map_err(lua_err)?;

        let options = Rc::clone(&self.options);
        let set_options = self
            .lua
            .create_function(move |_, table: Table| {
                options.borrow_mut().extend(flatten_options(table)?);
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("options", set_options).map_err(lua_err)?;

        let option_appends = Rc::clone(&self.option_appends);
        let append = self
            .lua
            .create_function(move |_, (name, value): (String, String)| {
                option_appends
                    .borrow_mut()
                    .push((name.replace('_', "-"), value));
                Ok(())
            })
            .map_err(lua_err)?;
        tpane.set("append", append).map_err(lua_err)?;

        let jobs = Rc::clone(&self.jobs);
        let job_data = Rc::clone(&self.job_data);
        let generated_job = Rc::new(Cell::new(0usize));
        let job = self
            .lua
            .create_function(move |lua, table: Table| {
                let idx = generated_job.get() + 1;
                generated_job.set(idx);
                let name = format!("__tpane_job_{idx}");
                jobs.borrow_mut().push(parse_job_def(name.clone(), table)?);
                lua.create_userdata(LuaJob {
                    name,
                    data: Rc::clone(&job_data),
                })
            })
            .map_err(lua_err)?;
        tpane.set("job", job).map_err(lua_err)?;

        let opt_options = Rc::clone(&self.options);
        let opt = self.lua.create_table().map_err(lua_err)?;
        let opt_meta = self.lua.create_table().map_err(lua_err)?;
        opt_meta
            .set(
                "__newindex",
                self.lua
                    .create_function(move |lua, (_table, key, value): (Table, String, Value)| {
                        let table = value_to_option_table(lua, &key, value)?;
                        opt_options.borrow_mut().extend(flatten_options(table)?);
                        Ok(())
                    })
                    .map_err(lua_err)?,
            )
            .map_err(lua_err)?;
        opt.set_metatable(Some(opt_meta));
        tpane.set("opt", opt).map_err(lua_err)?;

        let panes = Rc::clone(&self.panes);
        let panes_fn = self
            .lua
            .create_function(move |lua, ()| snapshots_table(lua, &panes.borrow()))
            .map_err(lua_err)?;
        tpane.set("panes", panes_fn).map_err(lua_err)?;

        let pane_fn = self
            .lua
            .create_function(move |lua, pane_id: String| pane_ref_table(lua, &pane_id))
            .map_err(lua_err)?;
        tpane.set("pane", pane_fn).map_err(lua_err)?;

        tpane
            .set("store", store_api(&self.lua, Rc::clone(&self.store))?)
            .map_err(lua_err)?;
        install_package_path(&self.lua)?;
        tpane.set("tmux", tmux_api(&self.lua)?).map_err(lua_err)?;
        tpane
            .set("with_pane", with_pane_fn(&self.lua)?)
            .map_err(lua_err)?;
        self.lua.globals().set("tpane", tpane).map_err(lua_err)?;
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
            .and_then(|_| self.load_source("widgets.lua", WIDGETS))
            .map_err(|error| anyhow!("failed to load Lua prelude: {error}"))
    }

    pub fn load_builtins(&self) -> Result<()> {
        self.load_source("builtin-kinds.lua", BUILTIN_KINDS)
            .map_err(|error| anyhow!("failed to load built-in Lua kinds: {error}"))
    }

    pub fn keybinds(&self) -> Vec<Keybind> {
        self.keybinds.borrow().clone()
    }

    pub fn unbinds(&self) -> Vec<Unbind> {
        self.unbinds.borrow().clone()
    }

    pub fn used_plugin_specs(&self) -> HashMap<String, PluginSpec> {
        self.used_plugins.borrow().clone()
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

    pub fn status_options(&self) -> StatusRender {
        self.statusline
            .borrow()
            .as_ref()
            .map(|def| StatusRender {
                active: true,
                position: def.position.clone(),
                interval: def.interval,
                rows: (!def.rows.is_empty()).then_some(def.rows.len()),
                left: None,
                right: None,
                formats: Vec::new(),
            })
            .unwrap_or_default()
    }

    pub fn options(&self) -> Vec<(String, String)> {
        let mut options = BTreeMap::new();
        for (name, value) in self.options.borrow().iter() {
            options.insert(name.clone(), value.clone());
        }
        options.into_iter().collect()
    }

    pub fn option_appends(&self) -> Vec<(String, String)> {
        let mut appends = self.option_appends.borrow().clone();
        appends.sort_by(|a, b| a.0.cmp(&b.0));
        appends
    }

    pub fn jobs(&self) -> Vec<JobDef> {
        self.jobs.borrow().clone()
    }

    pub fn state_presentation(&self, state: &str) -> Option<StatePresentation> {
        self.states.borrow().get(state).cloned()
    }

    pub fn render_pane_border(&self, pane: &PaneSnapshot) -> Result<Option<String>> {
        let handler: Function = {
            let pane_border = self.pane_border.borrow();
            let Some(handler_key) = pane_border.as_ref() else {
                return Ok(None);
            };
            self.lua.registry_value(handler_key).map_err(lua_err)?
        };
        let pane = snapshot_table(&self.lua, pane).map_err(lua_err)?;
        let value = handler.call::<Value>(pane).map_err(lua_err)?;
        render_widget_value(value).map_err(lua_err)
    }

    pub fn render_statusline(&self, current_pane_id: Option<&str>) -> (StatusRender, Vec<String>) {
        let Some(def) = self.statusline.borrow().clone() else {
            return (StatusRender::default(), Vec::new());
        };

        let mut errors = Vec::new();
        let ctx = match self.status_context(current_pane_id) {
            Ok(ctx) => Some(ctx),
            Err(error) => {
                errors.push(format!("status context: {error}"));
                None
            }
        };
        if !def.rows.is_empty() {
            let formats = def
                .rows
                .iter()
                .enumerate()
                .map(|(index, row)| {
                    let left = row.left.as_ref().map(|widgets| {
                        self.render_status_slot(widgets, &row.separator, ctx.clone(), &mut errors)
                    });
                    let right = row.right.as_ref().map(|widgets| {
                        self.render_status_slot(widgets, &row.separator, ctx.clone(), &mut errors)
                    });
                    (index, status_format_row(left.as_deref(), right.as_deref()))
                })
                .collect();

            return (
                StatusRender {
                    active: true,
                    position: def.position,
                    interval: def.interval,
                    rows: Some(def.rows.len()),
                    left: None,
                    right: None,
                    formats,
                },
                errors,
            );
        }

        let left = def.left.as_ref().map(|widgets| {
            self.render_status_slot(widgets, &def.separator, ctx.clone(), &mut errors)
        });
        let right = def.right.as_ref().map(|widgets| {
            self.render_status_slot(widgets, &def.separator, ctx.clone(), &mut errors)
        });

        (
            StatusRender {
                active: true,
                position: def.position,
                interval: def.interval,
                rows: None,
                left,
                right,
                formats: Vec::new(),
            },
            errors,
        )
    }

    fn render_status_slot(
        &self,
        items: &[StatusItem],
        separator: &str,
        ctx: Option<Table>,
        errors: &mut Vec<String>,
    ) -> String {
        items
            .iter()
            .filter_map(|item| self.render_status_item(item, ctx.clone(), errors))
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(separator)
    }

    fn render_status_item(
        &self,
        item: &StatusItem,
        ctx: Option<Table>,
        errors: &mut Vec<String>,
    ) -> Option<String> {
        match item {
            StatusItem::Widget(name) => self.render_widget(name, ctx, errors),
            StatusItem::Job(name) => self.job_data.borrow().get(name).cloned(),
        }
    }

    fn status_context(&self, current_pane_id: Option<&str>) -> mlua::Result<Table> {
        let panes = self.panes.borrow();
        let current = current_pane_id
            .and_then(|id| panes.iter().find(|pane| pane.id == id))
            .or_else(|| panes.first());
        let ctx = self.lua.create_table()?;
        ctx.set("panes", snapshots_table(&self.lua, &panes)?)?;
        if let Some(pane) = current {
            ctx.set("pane", snapshot_table(&self.lua, pane)?)?;
            ctx.set("session", pane.session.clone())?;
            ctx.set("window", pane.window.clone())?;
        } else {
            ctx.set("pane", Value::Nil)?;
            ctx.set("session", Value::Nil)?;
            ctx.set("window", Value::Nil)?;
        }
        Ok(ctx)
    }

    fn render_widget(
        &self,
        name: &str,
        ctx: Option<Table>,
        errors: &mut Vec<String>,
    ) -> Option<String> {
        let handler = {
            let widgets = self.widgets.borrow();
            let Some(handler_key) = widgets.get(name) else {
                errors.push(format!("status widget {name}: unknown widget"));
                return None;
            };
            match self.lua.registry_value::<Function>(handler_key) {
                Ok(handler) => handler,
                Err(error) => {
                    errors.push(format!("status widget {name}: {error}"));
                    return None;
                }
            }
        };

        let arg = ctx.map(Value::Table).unwrap_or(Value::Nil);
        match handler.call::<Value>(arg) {
            Ok(value) => match render_widget_value(value) {
                Ok(value) => value,
                Err(error) => {
                    errors.push(format!("status widget {name}: {error}"));
                    None
                }
            },
            Err(error) => {
                errors.push(format!("status widget {name}: {error}"));
                None
            }
        }
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

    pub fn run_deferred(&self) -> Vec<String> {
        let deferred = std::mem::take(&mut *self.deferred.borrow_mut());
        let mut errors = Vec::new();
        for key in deferred {
            match self.lua.registry_value::<Function>(&key) {
                Ok(handler) => {
                    if let Err(error) = handler.call::<()>(()) {
                        errors.push(format!("deferred: {error}"));
                    }
                }
                Err(error) => errors.push(format!("deferred: {error}")),
            }
        }
        errors
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
    let mut files = Vec::new();
    collect_lua_files(&config, &mut files);
    files.sort();
    files
}

pub fn config_lua_files() -> Vec<PathBuf> {
    let config = config_dir();
    let mut files = Vec::new();
    collect_lua_files_recursive(&config, &mut files);
    files.sort();
    files
}

pub fn builtin_theme_names() -> Vec<String> {
    BUILTIN_PLUGIN_THEMES_DATA
        .lines()
        .filter_map(|line| line.split('\t').next())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn collect_lua_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            files.push(path);
        }
    }
}

fn collect_lua_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.filter_map(std::result::Result::ok) {
        let path = entry.path();
        if path.is_dir() {
            collect_lua_files_recursive(&path, files);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("lua") {
            files.push(path);
        }
    }
}

fn status_format_row(left: Option<&str>, right: Option<&str>) -> String {
    match (left, right) {
        (Some(left), Some(right)) => format!("{left}#[align=right]{right}"),
        (Some(left), None) => left.to_string(),
        (None, Some(right)) => format!("#[align=right]{right}"),
        (None, None) => String::new(),
    }
}

fn parse_statusline_def(table: Table) -> mlua::Result<StatusLineDef> {
    let position = table.get::<Option<String>>("position")?;
    if let Some(position) = &position
        && !matches!(position.as_str(), "top" | "bottom")
    {
        return Err(mlua::Error::RuntimeError(format!(
            "statusline position must be top or bottom, got {position}"
        )));
    }
    let separator = table
        .get::<Option<String>>("separator")?
        .unwrap_or_else(|| " ".to_string());
    let rows_value: Value = table.get("rows")?;
    let rows = if matches!(rows_value, Value::Nil) && table.raw_len() > 0 {
        parse_status_rows(Value::Table(table.clone()), &separator)?
    } else {
        parse_status_rows(rows_value, &separator)?
    };
    let (left, right) = if rows.is_empty() {
        (
            parse_status_slot(table.get("left")?)?,
            parse_status_slot(table.get("right")?)?,
        )
    } else {
        (None, None)
    };
    Ok(StatusLineDef {
        position,
        interval: table.get("interval")?,
        left,
        right,
        rows,
        separator,
    })
}

fn parse_status_rows(value: Value, default_separator: &str) -> mlua::Result<Vec<StatusRowDef>> {
    match value {
        Value::Nil => Ok(Vec::new()),
        Value::Table(table) => {
            if table.raw_len() > 5 {
                return Err(mlua::Error::RuntimeError(
                    "statusline supports at most 5 rows".to_string(),
                ));
            }
            table
                .sequence_values::<Table>()
                .map(|row| {
                    let row = row?;
                    Ok(StatusRowDef {
                        left: parse_status_slot(row.get("left")?)?,
                        right: parse_status_slot(row.get("right")?)?,
                        separator: row
                            .get::<Option<String>>("separator")?
                            .unwrap_or_else(|| default_separator.to_string()),
                    })
                })
                .collect()
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "statusline rows must be a list of row tables, got {other:?}"
        ))),
    }
}

fn parse_status_slot(value: Value) -> mlua::Result<Option<Vec<StatusItem>>> {
    match value {
        Value::Nil => Ok(None),
        Value::Table(table) => {
            let mut items = Vec::new();
            for value in table.sequence_values::<Value>() {
                items.push(parse_status_item(value?)?);
            }
            Ok(Some(items))
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "statusline slot must be a list of widgets or jobs, got {other:?}"
        ))),
    }
}

fn parse_status_item(value: Value) -> mlua::Result<StatusItem> {
    match value {
        Value::UserData(value) if value.is::<LuaWidget>() => Ok(StatusItem::Widget(
            value.borrow::<LuaWidget>()?.name.clone(),
        )),
        Value::UserData(value) if value.is::<LuaJob>() => {
            Ok(StatusItem::Job(value.borrow::<LuaJob>()?.name.clone()))
        }
        other => Err(mlua::Error::RuntimeError(format!(
            "statusline item must be a widget or job; got {other:?}"
        ))),
    }
}

fn parse_job_def(name: String, table: Table) -> mlua::Result<JobDef> {
    let command = table
        .get::<Option<String>>("cmd")?
        .or(table.get::<Option<String>>("command")?)
        .ok_or_else(|| mlua::Error::RuntimeError("job requires cmd or command".to_string()))?;
    let every = parse_duration_value("every", table.get("every")?)?;
    let timeout = match table.get::<Option<Value>>("timeout")? {
        Some(value) => parse_duration_value("timeout", value)?,
        None => Duration::from_secs(10),
    };
    Ok(JobDef {
        name,
        every,
        timeout,
        command,
    })
}

fn parse_duration_value(name: &str, value: Value) -> mlua::Result<Duration> {
    match value {
        Value::Integer(seconds) if seconds >= 0 => Ok(Duration::from_secs(seconds as u64)),
        Value::Number(seconds) if seconds >= 0.0 => Ok(Duration::from_secs_f64(seconds)),
        Value::String(value) => parse_duration_string(name, &value.to_string_lossy()),
        other => Err(mlua::Error::RuntimeError(format!(
            "job {name} must be seconds or a string like 10s, 5m, or 1h; got {other:?}"
        ))),
    }
}

fn parse_duration_string(name: &str, value: &str) -> mlua::Result<Duration> {
    let value = value.trim();
    let (number, multiplier) = match value.chars().last() {
        Some('s') => (&value[..value.len() - 1], 1),
        Some('m') => (&value[..value.len() - 1], 60),
        Some('h') => (&value[..value.len() - 1], 60 * 60),
        Some(_) => (value, 1),
        None => {
            return Err(mlua::Error::RuntimeError(format!(
                "job {name} cannot be empty"
            )));
        }
    };
    let amount = number
        .parse::<u64>()
        .map_err(|_| mlua::Error::RuntimeError(format!("invalid job {name}: {value}")))?;
    Ok(Duration::from_secs(amount * multiplier))
}

fn value_to_option_table(lua: &Lua, key: &str, value: Value) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set(key, value)?;
    Ok(table)
}

fn flatten_options(table: Table) -> mlua::Result<Vec<(String, String)>> {
    let mut options = Vec::new();
    flatten_option_table(&table, &[], &mut options)?;
    options.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(options)
}

fn flatten_option_table(
    table: &Table,
    path: &[String],
    options: &mut Vec<(String, String)>,
) -> mlua::Result<()> {
    for pair in table.clone().pairs::<String, Value>() {
        let (key, value) = pair?;
        if !path.is_empty() && key.contains('-') {
            return Err(mlua::Error::RuntimeError(format!(
                "literal tmux option names are only supported at top level: {key}"
            )));
        }
        let segment = if path.is_empty() && key.contains('-') {
            key
        } else {
            key.replace('_', "-")
        };
        let mut next_path = path.to_vec();
        next_path.push(segment);
        let name = next_path.join("-");
        match value {
            Value::String(value) => options.push((name, value.to_string_lossy())),
            Value::Integer(value) => options.push((name, value.to_string())),
            Value::Number(value) => options.push((name, value.to_string())),
            Value::Boolean(value) => {
                options.push((name, if value { "on" } else { "off" }.to_string()))
            }
            Value::Table(table) if name.ends_with("-style") => {
                options.push((name, style_spec(&table)?));
            }
            Value::Table(table) if name.ends_with("-format") => {
                options.push((name, render_widget_table(table)?.unwrap_or_default()));
            }
            Value::Table(table) => flatten_option_table(&table, &next_path, options)?,
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unsupported option value for {name}: {other:?}"
                )));
            }
        }
    }
    Ok(())
}

fn render_widget_value(value: Value) -> mlua::Result<Option<String>> {
    match value {
        Value::Nil => Ok(None),
        Value::String(value) => Ok(Some(value.to_string_lossy())),
        Value::Table(table) => render_widget_table(table),
        Value::UserData(value) if value.is::<LuaJob>() => Ok(value.borrow::<LuaJob>()?.value()),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected string, table, job, or nil; got {other:?}"
        ))),
    }
}

fn render_widget_table(table: Table) -> mlua::Result<Option<String>> {
    if table.get::<Option<Value>>(1)?.is_some() {
        let mut out = String::new();
        for value in table.sequence_values::<Value>() {
            if let Some(value) = render_widget_value(value?)? {
                out.push_str(&value);
            }
        }
        return Ok(Some(out));
    }

    let text = table.get::<Option<String>>("text")?.unwrap_or_default();
    let attrs = style_attrs(&table)?;

    if attrs.is_empty() {
        Ok(Some(text))
    } else {
        Ok(Some(format!("#[{}]{}#[default]", attrs.join(","), text)))
    }
}

fn style_spec(table: &Table) -> mlua::Result<String> {
    Ok(style_attrs(table)?.join(","))
}

fn style_attrs(table: &Table) -> mlua::Result<Vec<String>> {
    for key in table
        .clone()
        .pairs::<String, Value>()
        .map(|pair| pair.map(|(key, _)| key))
    {
        match key?.as_str() {
            "text" | "fg" | "bg" | "bold" | "dim" | "italics" | "blink" | "reverse" | "hidden"
            | "strikethrough" | "underscore" | "align" | "fill" => {}
            other => {
                return Err(mlua::Error::RuntimeError(format!(
                    "unknown status style: {other}"
                )));
            }
        }
    }

    let mut attrs = Vec::new();
    for key in ["fg", "bg"] {
        if let Some(value) = table.get::<Option<String>>(key)? {
            attrs.push(format!("{key}={value}"));
        }
    }
    for key in [
        "bold",
        "dim",
        "italics",
        "blink",
        "reverse",
        "hidden",
        "strikethrough",
    ] {
        if table.get::<Option<bool>>(key)?.unwrap_or(false) {
            attrs.push(key.to_string());
        }
    }
    match table.get::<Value>("underscore")? {
        Value::Boolean(true) => attrs.push("underscore".to_string()),
        Value::Boolean(false) | Value::Nil => {}
        Value::String(style) => attrs.push(format!("underscore={}", style.to_string_lossy())),
        other => {
            return Err(mlua::Error::RuntimeError(format!(
                "status style underscore must be a boolean or string, got {other:?}"
            )));
        }
    }
    for key in ["align", "fill"] {
        if let Some(value) = table.get::<Option<String>>(key)? {
            attrs.push(format!("{key}={value}"));
        }
    }
    Ok(attrs)
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

fn parse_bind(
    lua: &Lua,
    commands: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    panes: &Rc<RefCell<Vec<PaneSnapshot>>>,
    generated: &Rc<Cell<usize>>,
    args: mlua::MultiValue,
) -> mlua::Result<Keybind> {
    let values = args.into_iter().collect::<Vec<_>>();
    match values.as_slice() {
        [key, command] => {
            let command =
                parse_bind_command_value(lua, commands, panes, generated, command.clone())?;
            Ok(Keybind {
                mode: "prefix".to_string(),
                key: value_to_string(key, "key")?,
                raw: command.raw,
                command: command.command,
                context: command.context.unwrap_or(true),
                popup: false,
            })
        }
        [key, command, opts] => {
            let opts = parse_keybind_opts(opts, true)?;
            let command =
                parse_bind_command_value(lua, commands, panes, generated, command.clone())?;
            Ok(Keybind {
                mode: opts.mode,
                key: value_to_string(key, "key")?,
                raw: command.raw,
                command: command.command,
                context: command.context.unwrap_or(opts.context),
                popup: opts.popup,
            })
        }
        _ => Err(mlua::Error::RuntimeError(
            "expected tpane.bind(key, action[, opts])".to_string(),
        )),
    }
}

struct ParsedBindCommand {
    command: Vec<String>,
    raw: bool,
    context: Option<bool>,
}

fn parse_bind_command_value(
    lua: &Lua,
    commands: &Rc<RefCell<HashMap<String, RegistryKey>>>,
    panes: &Rc<RefCell<Vec<PaneSnapshot>>>,
    generated: &Rc<Cell<usize>>,
    value: Value,
) -> mlua::Result<ParsedBindCommand> {
    match value {
        Value::Function(function) => {
            let idx = generated.get() + 1;
            generated.set(idx);
            let name = format!("__tpane_key_{idx}");
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
            Ok(ParsedBindCommand {
                command: vec![name],
                raw: false,
                context: None,
            })
        }
        Value::Table(table) => parse_action_table(table),
        Value::String(command) => Ok(ParsedBindCommand {
            command: vec![command.to_string_lossy()],
            raw: true,
            context: Some(false),
        }),
        other => Err(mlua::Error::RuntimeError(format!(
            "expected bind action, got {other:?}"
        ))),
    }
}

#[cfg(test)]
fn run_action_table(lua: &Lua, parts: &[String]) -> mlua::Result<Table> {
    let table = lua.create_table()?;
    table.set("__tpane_action", "run")?;
    let command = lua.create_table()?;
    for (idx, part) in parts.iter().enumerate() {
        command.set(idx + 1, part.as_str())?;
    }
    table.set("command", command)?;
    Ok(table)
}

fn parse_action_table(table: Table) -> mlua::Result<ParsedBindCommand> {
    match table.get::<Option<String>>("__tpane_action")?.as_deref() {
        Some("raw") => Ok(ParsedBindCommand {
            command: vec![table.get::<String>("command")?],
            raw: true,
            context: Some(false),
        }),
        Some("run") => Ok(ParsedBindCommand {
            command: table
                .get::<Table>("command")?
                .sequence_values::<String>()
                .collect::<mlua::Result<Vec<_>>>()?,
            raw: false,
            context: None,
        }),
        Some(other) => Err(mlua::Error::RuntimeError(format!(
            "unknown bind action: {other}"
        ))),
        None => Ok(ParsedBindCommand {
            command: table
                .sequence_values::<String>()
                .collect::<mlua::Result<Vec<_>>>()?,
            raw: false,
            context: None,
        }),
    }
}

struct KeybindOpts {
    mode: String,
    context: bool,
    popup: bool,
}

fn parse_keybind_opts(value: &Value, default_context: bool) -> mlua::Result<KeybindOpts> {
    match value {
        Value::Table(table) => {
            for key in table
                .clone()
                .pairs::<String, Value>()
                .map(|pair| pair.map(|(key, _)| key))
            {
                match key?.as_str() {
                    "popup" | "context" | "prefix" | "table" | "mode" => {}
                    other => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "unknown bind option: {other}"
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
            let mode = match (
                table.get::<Option<String>>("table")?,
                table.get::<Option<String>>("mode")?,
            ) {
                (Some(table), _) => table,
                (None, Some(mode)) if mode == "copy" => "copy-mode-vi".to_string(),
                (None, Some(mode)) => mode,
                (None, None) if table.get::<Option<bool>>("prefix")? == Some(false) => {
                    "root".to_string()
                }
                (None, None) => "prefix".to_string(),
            };
            Ok(KeybindOpts {
                mode,
                context,
                popup,
            })
        }
        Value::Nil => Ok(KeybindOpts {
            mode: "prefix".to_string(),
            context: default_context,
            popup: false,
        }),
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
    if let Some(path) = env::var_os("TPANE_CONFIG_DIR") {
        return PathBuf::from(path);
    }

    let home = config_home();
    let tmux = home.join("tmux/tpane");
    let legacy = home.join("tpane");
    if tmux.exists() || !legacy.exists() {
        tmux
    } else {
        legacy
    }
}

fn config_home() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"))
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
            tmux::set_pane_var(pane_id, &format!("@tpane_{name}"), &value)
                .map_err(mlua_external)?;
        }
    }
    if let Some(title) = table.get::<Option<String>>("title")? {
        tmux::set_pane_title(pane_id, &title).map_err(mlua_external)?;
    }
    Ok(())
}

fn state_presentation_table(lua: &Lua, presentation: &StatePresentation) -> mlua::Result<Value> {
    let table = lua.create_table()?;
    table.set("color", presentation.color.clone())?;
    table.set("glyph", presentation.glyph.clone())?;
    Ok(Value::Table(table))
}

fn plugin_spec_from_lua(table: &Table) -> mlua::Result<PluginSpec> {
    let url: Option<String> = table.get("url")?;
    let repo: Option<String> = table.get("repo")?;
    if url.is_some() && repo.is_some() && url != repo {
        return Err(mlua::Error::RuntimeError(
            "plugin spec cannot set different url and repo values".to_string(),
        ));
    }
    Ok(PluginSpec {
        url: url.or(repo),
        branch: table.get("branch")?,
        tag: table.get("tag")?,
        rev: table.get("rev")?,
        path: table.get("path")?,
    })
}

fn load_plugin(lua: &Lua, name: &str, spec: &PluginSpec) -> mlua::Result<()> {
    plugins::validate_plugin_name(name).map_err(mlua_external)?;
    plugins::validate_spec(spec).map_err(mlua_external)?;
    if spec.url.is_some() {
        let missing = !plugins::plugin_dir(name).exists();
        if missing {
            let _ = tmux::display_global_message(&format!("tpane: installing plugin {name}"));
        }
        plugins::ensure(name, spec).map_err(mlua_external)?;
        if missing {
            let _ = tmux::display_global_message(&format!("tpane: installed plugin {name}"));
        }
    } else {
        plugins::assert_compatible(name, spec).map_err(mlua_external)?;
    }

    let entrypoint = plugins::entrypoint(name, spec).map_err(mlua_external)?;
    if entrypoint.is_file() {
        let source = fs::read_to_string(&entrypoint).map_err(mlua::Error::external)?;
        if let Some(dir) = entrypoint.parent() {
            prepend_package_path(lua, dir)?;
        }
        return lua
            .load(&source)
            .set_name(entrypoint.display().to_string())
            .exec();
    }

    match name {
        "vim-navigator" => lua
            .load(BUILTIN_PLUGIN_VIM_NAVIGATOR)
            .set_name("builtin/plugins/vim-navigator/init.lua")
            .exec(),
        "yank" => lua
            .load(BUILTIN_PLUGIN_YANK)
            .set_name("builtin/plugins/yank/init.lua")
            .exec(),
        "sensible" => lua
            .load(BUILTIN_PLUGIN_SENSIBLE)
            .set_name("builtin/plugins/sensible/init.lua")
            .exec(),
        "themes" => {
            let tpane: Table = lua.globals().get("tpane")?;
            tpane.set("_theme_data", BUILTIN_PLUGIN_THEMES_DATA)?;
            lua.load(BUILTIN_PLUGIN_THEMES)
                .set_name("builtin/plugins/themes/init.lua")
                .exec()
        }
        _ => Err(mlua::Error::RuntimeError(format!("unknown plugin: {name}"))),
    }
}

fn prepend_package_path(lua: &Lua, dir: &Path) -> mlua::Result<()> {
    let package: Table = lua.globals().get("package")?;
    let current = package.get::<String>("path").unwrap_or_default();
    let dir = dir.display().to_string();
    let paths = [format!("{dir}/?.lua"), format!("{dir}/?/init.lua")].join(";");
    let next = if current.is_empty() {
        paths
    } else {
        format!("{paths};{current}")
    };
    package.set("path", next)
}

fn install_package_path(lua: &Lua) -> Result<()> {
    install_package_path_for(lua, &config_dir())
}

fn install_package_path_for(lua: &Lua, config_dir: &Path) -> Result<()> {
    let package: Table = lua.globals().get("package").map_err(lua_err)?;
    let current = package.get::<String>("path").unwrap_or_default();
    let config = config_dir.display().to_string();
    let paths = [
        format!("{config}/?.lua"),
        format!("{config}/?/init.lua"),
        format!("{}/?.lua", plugins::plugin_root().display()),
        format!("{}/?/init.lua", plugins::plugin_root().display()),
    ]
    .join(";");
    let next = if current.is_empty() {
        paths
    } else {
        format!("{paths};{current}")
    };
    package.set("path", next).map_err(lua_err)
}

fn store_api(lua: &Lua, store: Rc<RefCell<Store>>) -> Result<Table> {
    let table = lua.create_table().map_err(lua_err)?;

    let get_store = Rc::clone(&store);
    table
        .set(
            "get",
            lua.create_function(move |lua, key: String| {
                get_store
                    .borrow()
                    .get(&key)
                    .map(|value| json_to_lua(lua, &value))
                    .unwrap_or(Ok(Value::Nil))
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    let set_store = Rc::clone(&store);
    table
        .set(
            "set",
            lua.create_function(move |_, (key, value): (String, Value)| {
                let value = lua_to_json(value)?;
                set_store.borrow_mut().set(key, value);
                Ok(())
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    table
        .set(
            "delete",
            lua.create_function(move |_, key: String| {
                store.borrow_mut().delete(&key);
                Ok(())
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;

    Ok(table)
}

fn json_to_lua(lua: &Lua, value: &JsonValue) -> mlua::Result<Value> {
    Ok(match value {
        JsonValue::Null => Value::Nil,
        JsonValue::Bool(value) => Value::Boolean(*value),
        JsonValue::Number(value) => {
            if let Some(integer) = value.as_i64() {
                Value::Integer(integer)
            } else if let Some(number) = value.as_f64() {
                Value::Number(number)
            } else {
                Value::Nil
            }
        }
        JsonValue::String(value) => Value::String(lua.create_string(value)?),
        JsonValue::Array(values) => {
            let table = lua.create_table()?;
            for (idx, value) in values.iter().enumerate() {
                table.set(idx + 1, json_to_lua(lua, value)?)?;
            }
            Value::Table(table)
        }
        JsonValue::Object(values) => {
            let table = lua.create_table()?;
            for (key, value) in values {
                table.set(key.as_str(), json_to_lua(lua, value)?)?;
            }
            Value::Table(table)
        }
    })
}

fn lua_to_json(value: Value) -> mlua::Result<JsonValue> {
    lua_to_json_seen(value, &mut HashSet::new())
}

fn lua_to_json_seen(value: Value, seen: &mut HashSet<usize>) -> mlua::Result<JsonValue> {
    match value {
        Value::Nil => Ok(JsonValue::Null),
        Value::Boolean(value) => Ok(JsonValue::Bool(value)),
        Value::Integer(value) => Ok(JsonValue::Number(JsonNumber::from(value))),
        Value::Number(value) => JsonNumber::from_f64(value)
            .map(JsonValue::Number)
            .ok_or_else(|| mlua::Error::RuntimeError("cannot store non-finite number".to_string())),
        Value::String(value) => Ok(JsonValue::String(value.to_string_lossy())),
        Value::Table(table) => lua_table_to_json(table, seen),
        other => Err(mlua::Error::RuntimeError(format!(
            "cannot store Lua value: {other:?}"
        ))),
    }
}

fn lua_table_to_json(table: Table, seen: &mut HashSet<usize>) -> mlua::Result<JsonValue> {
    let pointer = table.to_pointer() as usize;
    if !seen.insert(pointer) {
        return Err(mlua::Error::RuntimeError(
            "cannot store recursive table".to_string(),
        ));
    }

    let result = (|| {
        let mut array_values: Vec<(usize, JsonValue)> = Vec::new();
        let mut object = JsonMap::new();
        let mut array_like = true;

        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair?;
            let value = lua_to_json_seen(value, seen)?;
            match key {
                Value::Integer(index) if index > 0 => {
                    array_values.push((index as usize, value));
                }
                Value::String(key) => {
                    array_like = false;
                    object.insert(key.to_string_lossy(), value);
                }
                other => {
                    return Err(mlua::Error::RuntimeError(format!(
                        "cannot store table key: {other:?}"
                    )));
                }
            }
        }

        if !array_values.is_empty() {
            if !array_like {
                return Err(mlua::Error::RuntimeError(
                    "cannot store table with mixed array and object keys".to_string(),
                ));
            }
            array_values.sort_by_key(|(idx, _)| *idx);
            if array_values
                .iter()
                .enumerate()
                .all(|(offset, (idx, _))| *idx == offset + 1)
            {
                return Ok(JsonValue::Array(
                    array_values.into_iter().map(|(_, value)| value).collect(),
                ));
            }
            return Err(mlua::Error::RuntimeError(
                "cannot store sparse array table".to_string(),
            ));
        }

        Ok(JsonValue::Object(object))
    })();

    seen.remove(&pointer);
    result
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
            "new_window",
            lua.create_function(|_, opts: Table| {
                tmux::new_window(tmux::NewWindowOptions {
                    name: opts.get("name")?,
                    cwd: opts.get("cwd")?,
                    command: opts.get("command")?,
                })
                .map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "select_window",
            lua.create_function(|_, target: String| {
                tmux::select_window(&target).map_err(mlua_external)
            })
            .map_err(lua_err)?,
        )
        .map_err(lua_err)?;
    table
        .set(
            "send_keys",
            lua.create_function(|_, opts: Table| {
                let target: String = opts.get("target")?;
                let keys: String = opts.get("keys")?;
                let enter = opts.get::<Option<bool>>("enter")?.unwrap_or(false);
                tmux::send_keys(&target, &keys, enter).map_err(mlua_external)
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
                        full: opts.get::<Option<bool>>("full")?.unwrap_or(false),
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
                        full: opts.get::<Option<bool>>("full")?.unwrap_or(false),
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
                let name: String = opts.get("name").unwrap_or_else(|_| "tpane".to_string());
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
                    name: opts.get("name").unwrap_or_else(|_| "tpane".to_string()),
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
                    full: opts.get::<Option<bool>>("full")?.unwrap_or(false),
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
        "below" | "bottom" | "down" | "v" | "vertical" => {
            Ok((tmux::SplitDirection::Vertical, false))
        }
        "above" | "top" | "up" => Ok((tmux::SplitDirection::Vertical, true)),
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
            tmux::set_pane_var(pane_id, "@tpane_state", &state)?;
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
        if let Ok(current_zoomed) = tmux::is_zoomed(&self.pane_id)
            && current_zoomed != self.zoomed_before
        {
            let _ = tmux::zoom(&self.pane_id);
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

impl LuaJob {
    fn value(&self) -> Option<String> {
        self.data.borrow().get(&self.name).cloned()
    }
}

impl UserData for LuaJob {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
        fields.add_field_method_get("value", |_, this| Ok(this.value()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get", |_, this, ()| Ok(this.value()));
    }
}

impl UserData for LuaWidget {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, this| Ok(this.name.clone()));
    }
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

const PRELUDE: &str = include_str!("lua/prelude.lua");
const WIDGETS: &str = include_str!("lua/widgets.lua");

const BUILTIN_PLUGIN_VIM_NAVIGATOR: &str = include_str!("../plugins/vim-navigator/init.lua");
const BUILTIN_PLUGIN_YANK: &str = include_str!("../plugins/yank/init.lua");
const BUILTIN_PLUGIN_SENSIBLE: &str = include_str!("../plugins/sensible/init.lua");
const BUILTIN_PLUGIN_THEMES: &str = include_str!("../plugins/themes/init.lua");
const BUILTIN_PLUGIN_THEMES_DATA: &str = include_str!("../plugins/themes/palettes.tsv");

const BUILTIN_KINDS: &str = include_str!("lua/builtin_kinds.lua");

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str) -> PaneSnapshot {
        PaneSnapshot {
            id: id.to_string(),
            pid: 123,
            kind: "term".to_string(),
            label: "term · tpane".to_string(),
            cwd: "/tmp/tpane".to_string(),
            cwd_basename: "tpane".to_string(),
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
    fn collect_lua_files_loads_only_top_level_config() {
        let root = std::env::temp_dir().join(format!("tpane-lua-files-{}", std::process::id()));
        let nested = root.join("lib");
        let plugin = root.join("plugins/foo");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(root.join("a.lua"), "").unwrap();
        std::fs::write(nested.join("helper.lua"), "").unwrap();
        std::fs::write(plugin.join("init.lua"), "").unwrap();
        std::fs::write(plugin.join("lib.lua"), "").unwrap();

        let mut files = Vec::new();
        collect_lua_files(&root, &mut files);
        files.sort();
        let rel = files
            .iter()
            .map(|path| path.strip_prefix(&root).unwrap().display().to_string())
            .collect::<Vec<_>>();
        assert_eq!(rel, ["a.lua"]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn split_direction_maps_user_words() {
        assert!(matches!(
            split_direction("below").unwrap(),
            (tmux::SplitDirection::Vertical, false)
        ));
        assert!(matches!(
            split_direction("bottom").unwrap(),
            (tmux::SplitDirection::Vertical, false)
        ));
        assert!(matches!(
            split_direction("above").unwrap(),
            (tmux::SplitDirection::Vertical, true)
        ));
        assert!(matches!(
            split_direction("top").unwrap(),
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
    fn job_registers_command_and_data_reads_cache() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let data = Rc::new(RefCell::new(HashMap::new()));
        data.borrow_mut()
            .insert("__tpane_job_1".to_string(), "up 1 hour".to_string());
        let runtime = LuaRuntime::with_store_and_data(
            Rc::clone(&panes),
            Rc::new(RefCell::new(Store::memory())),
            data,
        )
        .unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                local uptime = tpane.job({ every = "1m", timeout = "5s", cmd = "uptime" })
                tpane.statusline { left = { uptime } }
                value = uptime:get()
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime.jobs(),
            vec![JobDef {
                name: "__tpane_job_1".to_string(),
                every: Duration::from_secs(60),
                timeout: Duration::from_secs(5),
                command: "uptime".to_string(),
            }]
        );
        assert_eq!(
            runtime.lua.globals().get::<String>("value").unwrap(),
            "up 1 hour"
        );
        assert_eq!(
            runtime.render_statusline(None).0.left.as_deref(),
            Some("up 1 hour")
        );
    }

    #[test]
    fn bind_accepts_run_and_raw_actions() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.bind("a", tpane.run("pi"))
                tpane.bind("A", tpane.run({ "pi", "expand" }))
                tpane.bind("M-a", tpane.pane.select("left"), { prefix = false })
                tpane.bind("%", tpane.pane.split("right", { cwd = "pane" }))
                tpane.bind("C-S-l", tpane.window.swap("next"), { prefix = false })
                tpane.bind("p", tpane.window.previous())
                tpane.bind("n", tpane.window.next())
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
                    raw: false,
                    context: true,
                    popup: false,
                },
                Keybind {
                    mode: "prefix".to_string(),
                    key: "A".to_string(),
                    command: vec!["pi".to_string(), "expand".to_string()],
                    raw: false,
                    context: true,
                    popup: false,
                },
                Keybind {
                    mode: "root".to_string(),
                    key: "M-a".to_string(),
                    command: vec!["select-pane -L".to_string()],
                    raw: true,
                    context: false,
                    popup: false,
                },
                Keybind {
                    mode: "prefix".to_string(),
                    key: "%".to_string(),
                    command: vec!["split-window -h -c \"#{pane_current_path}\"".to_string()],
                    raw: true,
                    context: false,
                    popup: false,
                },
                Keybind {
                    mode: "root".to_string(),
                    key: "C-S-l".to_string(),
                    command: vec!["swap-window -t +1 ; select-window -t +1".to_string()],
                    raw: true,
                    context: false,
                    popup: false,
                },
                Keybind {
                    mode: "prefix".to_string(),
                    key: "p".to_string(),
                    command: vec!["previous-window".to_string()],
                    raw: true,
                    context: false,
                    popup: false,
                },
                Keybind {
                    mode: "prefix".to_string(),
                    key: "n".to_string(),
                    command: vec!["next-window".to_string()],
                    raw: true,
                    context: false,
                    popup: false,
                },
            ]
        );
    }

    #[test]
    fn bind_opts_select_prefix_root_or_table() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.bind("C-g", function() end, { prefix = false })
                tpane.bind("M-h", function() end)
                tpane.bind("v", function() end, { mode = "copy" })
                tpane.bind("x", function() end, { prefix = false, table = "copy-mode-vi" })
                tpane.bind("C-a", function() end, { prefix = true })
                "#,
            )
            .unwrap();

        let keybinds = runtime.keybinds();
        assert_eq!(keybinds[0].mode, "root");
        assert_eq!(keybinds[0].key, "C-g");
        assert_eq!(keybinds[1].mode, "prefix");
        assert_eq!(keybinds[1].key, "M-h");
        assert_eq!(keybinds[2].mode, "copy-mode-vi");
        assert_eq!(keybinds[2].key, "v");
        assert_eq!(keybinds[3].mode, "copy-mode-vi");
        assert_eq!(keybinds[3].key, "x");
        assert_eq!(keybinds[4].mode, "prefix");
        assert_eq!(keybinds[4].key, "C-a");
    }

    #[test]
    fn register_pane_stores_reusable_config_without_overloading_pane_handle() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.register_pane("agent", { command = "pi" })
                tpane.command(function()
                  local pane = tpane.pane("%1")
                  local cfg = tpane._pane_defs.agent
                  return pane.id .. ":" .. cfg.tag .. ":" .. cfg.name
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("%1:agent:agent")
        );
    }

    #[test]
    fn collector_records_plugin_specs_without_loading_plugins() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let runtime = LuaRuntime::collector(panes).unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("foo", { repo = "https://example.test/foo.git", branch = "main", path = "plugins/foo" })
                "#,
            )
            .unwrap();

        let specs = runtime.used_plugin_specs();
        let spec = specs.get("foo").unwrap();
        assert_eq!(spec.url.as_deref(), Some("https://example.test/foo.git"));
        assert_eq!(spec.branch.as_deref(), Some("main"));
        assert_eq!(spec.path.as_deref(), Some("plugins/foo"));
    }

    #[test]
    fn builtin_sensible_plugin_sets_defaults() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("sensible")
                "#,
            )
            .unwrap();

        let options = runtime.options();
        assert!(options.contains(&("escape-time".to_string(), "0".to_string())));
        assert!(options.contains(&("history-limit".to_string(), "50000".to_string())));
        assert!(options.contains(&("display-time".to_string(), "4000".to_string())));
        assert!(options.contains(&("status-interval".to_string(), "5".to_string())));
        assert!(options.contains(&("focus-events".to_string(), "on".to_string())));
        assert!(options.contains(&("status-keys".to_string(), "emacs".to_string())));
        assert!(options.contains(&("aggressive-resize".to_string(), "on".to_string())));

        assert!(runtime.keybinds().is_empty());
    }

    #[test]
    fn builtin_themes_plugin_applies_palette() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("themes")
                tpane.theme("catppuccin-mocha")
                "#,
            )
            .unwrap();

        assert!(runtime.options().contains(&(
            "status-style".to_string(),
            "fg=#cdd6f4,bg=#1e1e2e".to_string()
        )));
        assert_eq!(
            runtime
                .state_presentation("working")
                .unwrap()
                .color
                .as_deref(),
            Some("#f9e2af")
        );
    }

    #[test]
    fn deferred_theme_wins_after_later_status_config() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("themes")
                tpane.theme("Gruvbox Dark")
                tpane.options({ status = { style = { bg = "default" } } })
                "#,
            )
            .unwrap();

        assert!(
            runtime
                .options()
                .contains(&("status-style".to_string(), "bg=default".to_string()))
        );
        assert!(runtime.run_deferred().is_empty());
        assert!(runtime.options().contains(&(
            "status-style".to_string(),
            "fg=#ebdbb2,bg=#282828".to_string()
        )));
    }

    #[test]
    fn theme_transparent_uses_default_status_background() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("themes")
                tpane.theme("Gruvbox Dark", { transparent = true })
                "#,
            )
            .unwrap();
        assert!(runtime.run_deferred().is_empty());

        assert!(runtime.options().contains(&(
            "status-style".to_string(),
            "fg=#ebdbb2,bg=default".to_string()
        )));
    }

    #[test]
    fn plugin_specs_reject_conflicting_repo_aliases() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let runtime = LuaRuntime::collector(panes).unwrap();
        let error = runtime
            .load_source(
                "test.lua",
                r#"
                tpane.use("foo", { repo = "https://example.test/a.git", url = "https://example.test/b.git" })
                "#,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("plugin spec cannot set different url and repo values"));
    }

    #[test]
    fn user_config_can_override_prelude_helpers() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                function tpane.expand()
                  return "custom"
                end
                tpane.command(function()
                  return tpane.expand()
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("custom")
        );
    }

    #[test]
    fn lua_require_uses_config_dir_package_path() {
        let root = std::env::temp_dir().join(format!("tpane-require-{}", std::process::id()));
        let lib = root.join("lib");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&lib).unwrap();
        std::fs::write(lib.join("helper.lua"), "return { value = 'ok' }").unwrap();

        let panes = Rc::new(RefCell::new(Vec::new()));
        let runtime = LuaRuntime::new(panes).unwrap();
        install_package_path_for(&runtime.lua, &root).unwrap();

        runtime
            .load_source(
                "test.lua",
                r#"
                local helper = require("lib.helper")
                tpane.command(function() return helper.value end)
                "#,
            )
            .unwrap();
        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("ok")
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn store_api_round_trips_lua_values() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::memory()));
        let runtime = LuaRuntime::with_store(panes, store).unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  tpane.store.set("prefs", { count = 2, items = { "a", "b" } })
                end)
                tpane.command(function()
                  local prefs = tpane.store.get("prefs")
                  return prefs.count .. ":" .. prefs.items[2]
                end)
                "#,
            )
            .unwrap();

        runtime.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(
            runtime
                .run_command("__tpane_command_2", &[])
                .unwrap()
                .as_deref(),
            Some("2:b")
        );
    }

    #[test]
    fn store_rejects_sparse_lua_array_tables() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::memory()));
        let runtime = LuaRuntime::with_store(panes, store).unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  tpane.store.set("sparse", { [1] = "a", [3] = "c" })
                end)
                "#,
            )
            .unwrap();

        let error = runtime
            .run_command("__tpane_command_1", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot store sparse array table"));
    }

    #[test]
    fn store_rejects_mixed_lua_table_keys() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::memory()));
        let runtime = LuaRuntime::with_store(panes, store).unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  tpane.store.set("mixed", { "a", name = "b" })
                end)
                "#,
            )
            .unwrap();

        let error = runtime
            .run_command("__tpane_command_1", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot store table with mixed array and object keys"));
    }

    #[test]
    fn store_rejects_recursive_lua_tables() {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::memory()));
        let runtime = LuaRuntime::with_store(panes, store).unwrap();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  local value = {}
                  value.self = value
                  tpane.store.set("cycle", value)
                end)
                "#,
            )
            .unwrap();

        let error = runtime
            .run_command("__tpane_command_1", &[])
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot store recursive table"));
    }

    #[test]
    fn workspace_registers_declarative_layout() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.workspace { name = "dev", windows = { { name = "app" }, { name = "logs" } } }
                tpane.command(function()
                  return tostring(#tpane._workspaces.dev.windows)
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("2")
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
                tpane.command(function()
                  local one = tpane.find{ tag = "agent" }
                  local all = tpane.find_all{ active = true }
                  return one.id .. ":" .. #all
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("%2:2")
        );
    }

    #[test]
    fn bind_accepts_function_and_registers_internal_command() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.bind("M-e", function()
                  return "ok"
                end, { prefix = false })
                "#,
            )
            .unwrap();

        let keybind = &runtime.keybinds()[0];
        assert_eq!(keybind.mode, "root");
        assert_eq!(keybind.key, "M-e");
        assert_eq!(keybind.command, ["__tpane_key_1"]);
        assert_eq!(
            runtime
                .run_command("__tpane_key_1", &[])
                .unwrap()
                .as_deref(),
            Some("ok")
        );
    }

    #[test]
    fn bind_rejects_unknown_options() {
        let (runtime, _) = runtime();
        let error = runtime
            .load_source(
                "test.lua",
                r#"tpane.bind("a", tpane.run("pi"), { desc = "unused" })"#,
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown bind option: desc"));
    }

    #[test]
    fn registered_panel_renders_cards() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.panel{
                  id = "workspace",
                  title = "Workspace",
                  cards = function()
                    local p = tpane.panes()[1]
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
    fn style_builder_emits_spec_and_format_forms() {
        let (runtime, _) = runtime();
        let table = runtime.lua.create_table().unwrap();
        table.set("text", "x").unwrap();
        table.set("fg", "red").unwrap();
        table.set("bold", true).unwrap();

        assert_eq!(style_spec(&table).unwrap(), "fg=red,bold");
        assert_eq!(
            render_widget_table(table).unwrap().as_deref(),
            Some("#[fg=red,bold]x#[default]")
        );
    }

    #[test]
    fn nested_options_flatten_and_serialize_styles() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r##"
                tpane.options{
                  status = { left_length = 120 },
                  pane = { border = { lines = "heavy", style = { fg = "#51576d" } } },
                  ["pane-active-border-style"] = { fg = "#8caaee" },
                  window = { status = { current_format = { text = "#I", fg = "#8caaee", bold = true } } },
                }
                "##,
            )
            .unwrap();

        assert_eq!(
            runtime.options(),
            vec![
                (
                    "pane-active-border-style".to_string(),
                    "fg=#8caaee".to_string()
                ),
                (
                    "pane-border-format".to_string(),
                    "#{@tpane_border}".to_string()
                ),
                ("pane-border-lines".to_string(), "heavy".to_string()),
                ("pane-border-status".to_string(), "top".to_string()),
                ("pane-border-style".to_string(), "fg=#51576d".to_string()),
                ("status-left-length".to_string(), "120".to_string()),
                (
                    "window-status-current-format".to_string(),
                    "#[fg=#8caaee,bold]#I#[default]".to_string()
                ),
            ]
        );
    }

    #[test]
    fn tabline_helper_sets_window_status_formats() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r##"
                tpane.tabline{
                  label = "cwd",
                  inactive = { fg = "#777777" },
                  current = { fg = "#c6d0f5", bg = "#232634", bold = true },
                }
                "##,
            )
            .unwrap();

        let options = runtime.options();
        assert!(
            options.contains(&(
                "window-status-format".to_string(),
                "#[fg=#777777]#I:#(pwd=\"#{pane_current_path}\"; echo ${pwd####*/})#[default]"
                    .to_string(),
            ))
        );
        assert!(options.contains(&(
            "window-status-current-format".to_string(),
            "#[fg=#c6d0f5,bg=#232634,bold]#I:#(pwd=\"#{pane_current_path}\"; echo ${pwd####*/})#[default]"
                .to_string(),
        )));
    }

    #[test]
    fn fmt_helpers_return_tmux_conditionals() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r##"
                tpane.command(function()
                  return tpane.fmt.prefix("ON", "off") .. ";" .. tpane.fmt.when("window_zoomed_flag", "Z", "")
                end)
                "##,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("#{?client_prefix,ON,off};#{?window_zoomed_flag,Z,}")
        );
    }

    #[test]
    fn state_registry_registers_and_reads_presentations() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.state("approval", { color = "magenta", glyph = "?" })
                tpane.command(function()
                  return tpane.state("approval").color .. tpane.state("approval").glyph .. ";" .. tpane.state("idle_seen").color
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("magenta?;green")
        );
    }

    #[test]
    fn pane_border_renders_from_lua_and_state_registry() {
        let (runtime, _) = runtime();
        let mut pane = pane("%1");
        pane.state = Some("working".to_string());
        pane.label = "build".to_string();

        let border = runtime.render_pane_border(&pane).unwrap().unwrap();
        assert_eq!(
            border,
            "#[fg=yellow] #[default]#[fg=yellow]build#[default]"
        );
    }

    #[test]
    fn statusline_context_uses_current_pane_id() {
        let (runtime, panes) = runtime();
        let mut other = pane("%1");
        other.session = "other".to_string();
        other.window = "@1".to_string();
        other.active = true;
        panes.borrow_mut().push(other);
        let mut current = pane("%2");
        current.session = "current".to_string();
        current.window = "@2".to_string();
        current.active = true;
        panes.borrow_mut().push(current);
        runtime
            .load_source(
                "test.lua",
                r#"
                local s = tpane.widget(function(ctx) return "[" .. ctx.session .. "]" end)
                tpane.statusline { left = { s } }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(Some("%2"));
        assert!(errors.is_empty());
        assert_eq!(status.left.as_deref(), Some("[current]"));
    }

    #[test]
    fn builtin_session_widget_uses_tmux_client_session() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                "tpane.statusline { left = { tpane.widgets.session } }",
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(Some("%1"));
        assert!(errors.is_empty());
        assert_eq!(status.left.as_deref(), Some("[#{client_session}]"));
    }

    #[test]
    fn builtin_widget_pack_renders_common_status_parts() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.statusline {
                  left = { tpane.widgets.host, tpane.widgets.prefix }
                }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(Some("%1"));
        assert!(errors.is_empty());
        assert_eq!(
            status.left.as_deref(),
            Some("#H #{?client_prefix,  ,  }")
        );
    }

    #[test]
    fn unknown_builtin_widget_errors_with_name() {
        let (runtime, _) = runtime();
        let error = runtime
            .load_source(
                "test.lua",
                r#"
                tpane.statusline { right = { tpane.widgets.typo } }
                "#,
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains("unknown widget: tpane.widgets.typo"));
    }

    #[test]
    fn builtin_job_widgets_register_jobs() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                local battery = tpane.widgets.battery({ every = "20s", timeout = "1s", cmd = "printf battery" })
                local player = tpane.widgets.player({ every = "5s", timeout = "1s", cmd = "printf song" })
                local cpu = tpane.widgets.cpu({ every = "2s", timeout = "1s", cmd = "printf cpu" })
                local memory = tpane.widgets.memory({ every = "3s", timeout = "1s", cmd = "printf memory" })
                tpane.statusline { right = { battery, player, cpu, memory } }
                "#,
            )
            .unwrap();

        let jobs = runtime.jobs();
        assert_eq!(jobs.len(), 4);
        assert_eq!(jobs[0].every, Duration::from_secs(20));
        assert_eq!(jobs[0].timeout, Duration::from_secs(1));
        assert_eq!(jobs[0].command, "printf battery");
        assert_eq!(jobs[1].every, Duration::from_secs(5));
        assert_eq!(jobs[1].timeout, Duration::from_secs(1));
        assert_eq!(jobs[1].command, "printf song");
        assert_eq!(jobs[2].every, Duration::from_secs(2));
        assert_eq!(jobs[2].command, "printf cpu");
        assert_eq!(jobs[3].every, Duration::from_secs(3));
        assert_eq!(jobs[3].command, "printf memory");
    }

    #[test]
    fn statusline_renders_raw_and_styled_widgets() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r##"
                local raw = tpane.widget(function() return "#{session_name}" end)
                local styled = tpane.widget(function() return { text = "x", fg = "red", bold = true } end)
                tpane.statusline { position = "top", interval = 1, left = { raw }, right = { styled }, separator = " | " }
                "##,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(None);
        assert!(errors.is_empty());
        assert_eq!(status.position.as_deref(), Some("top"));
        assert_eq!(status.interval, Some(1));
        assert_eq!(status.left.as_deref(), Some("#{session_name}"));
        assert_eq!(status.right.as_deref(), Some("#[fg=red,bold]x#[default]"));
    }

    #[test]
    fn statusline_rows_render_extra_status_formats() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                local session = tpane.widget(function() return "session" end)
                local cwd = tpane.widget(function() return "cwd" end)
                local clock = tpane.widget(function() return "clock" end)
                tpane.statusline {
                  rows = {
                    { left = { session }, right = { clock } },
                    { left = { cwd, tpane.widgets.tabs }, right = { tpane.widgets.prefix } },
                  }
                }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(None);
        assert!(errors.is_empty());
        assert_eq!(status.rows, Some(2));
        assert_eq!(status.left, None);
        assert_eq!(status.right, None);
        assert_eq!(
            status.formats,
            vec![
                (0, "session#[align=right]clock".to_string()),
                (
                    1,
                    "cwd #{W:#{E:window-status-format} ,#{E:window-status-current-format} }#[align=right]#{?client_prefix,  ,  }".to_string()
                )
            ]
        );
    }

    #[test]
    fn statusline_accepts_implicit_top_level_rows() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                local session = tpane.widget(function() return "session" end)
                local clock = tpane.widget(function() return "clock" end)
                tpane.statusline {
                  position = "top",
                  interval = 1,
                  { left = { session } },
                  { right = { clock } },
                }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(None);
        assert!(errors.is_empty());
        assert_eq!(status.position.as_deref(), Some("top"));
        assert_eq!(status.interval, Some(1));
        assert_eq!(status.rows, Some(2));
        assert_eq!(
            status.formats,
            vec![
                (0, "session".to_string()),
                (1, "#[align=right]clock".to_string())
            ]
        );
    }

    #[test]
    fn statusline_rows_are_limited_to_tmux_max() {
        let (runtime, _) = runtime();
        let error = runtime
            .load_source(
                "test.lua",
                r#"
                local w = tpane.widget(function() return "x" end)
                tpane.statusline { rows = {
                  { left = { w } },
                  { left = { w } },
                  { left = { w } },
                  { left = { w } },
                  { left = { w } },
                  { left = { w } },
                } }
                "#,
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains("statusline supports at most 5 rows"));
    }

    #[test]
    fn statusline_skips_failing_widgets_and_reports_errors() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                local bad = tpane.widget(function() error("boom") end)
                local ok = tpane.widget(function() return "ok" end)
                tpane.statusline { right = { bad, ok } }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(None);
        assert_eq!(status.right.as_deref(), Some("ok"));
        assert_eq!(errors.len(), 1);
        assert!(errors.iter().any(|error| error.contains("boom")));
    }

    #[test]
    fn statusline_rejects_unknown_style_keys() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                local bad = tpane.widget(function() return { text = "x", nope = true } end)
                tpane.statusline { right = { bad } }
                "#,
            )
            .unwrap();

        let (status, errors) = runtime.render_statusline(None);
        assert_eq!(status.right.as_deref(), Some(""));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("unknown status style: nope"));
    }

    #[test]
    fn named_commands_are_rejected() {
        let (runtime, _) = runtime();
        let error = runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command("deploy", function() end)
                "#,
            )
            .unwrap_err()
            .to_string();

        assert!(error.contains("error converting Lua string to function"));
    }

    #[test]
    fn short_panel_names_are_primary_api() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.panel {
                  id = "main",
                  title = "Main",
                  cards = function() return {} end,
                }
                "#,
            )
            .unwrap();

        assert_eq!(runtime.render_panels().unwrap()[0].id, "main");
    }

    #[test]
    fn registered_command_returns_string_and_receives_args() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function(args) return "hi " .. args[1] end)
                "#,
            )
            .unwrap();

        let out = runtime
            .run_command("__tpane_command_1", &["there".to_string()])
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
                tpane.command(function() error("nope") end)
                "#,
            )
            .unwrap();

        let error = runtime
            .run_command("__tpane_command_1", &[])
            .unwrap_err()
            .to_string();
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
                r#"tpane.kind { name = "shell", match = "zsh" }"#,
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
    fn command_can_call_command_without_refcell_panic() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  tpane.command(function() return "inner ok" end)
                  return "outer ok"
                end)
                "#,
            )
            .unwrap();

        assert_eq!(
            runtime
                .run_command("__tpane_command_1", &[])
                .unwrap()
                .as_deref(),
            Some("outer ok")
        );
        assert_eq!(
            runtime
                .run_command("__tpane_command_2", &[])
                .unwrap()
                .as_deref(),
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
                tpane.on("tick", function()
                  tpane.on("tick", function() end)
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
                tpane.command(function()
                  local panes = tpane.panes()
                  return panes[1].id .. ":" .. panes[1].kind .. ":" .. panes[1].pid .. ":" .. panes[1].tag .. ":" .. panes[1].home .. ":" .. panes[1].state
                end)
                "#,
            )
            .unwrap();

        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("%1:term:123:terminal:@1:idle"));
    }

    #[test]
    fn pane_ref_exposes_methods_for_fresh_split_ids() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  local p = tpane.pane("%9")
                  return p.id .. ":" .. type(p.set) .. ":" .. type(p.var) .. ":" .. type(p.capture)
                end)
                "#,
            )
            .unwrap();

        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
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
                tpane.command(function()
                  local p = tpane.panes()[1]
                  return tostring(p:running("zsh")) .. ":" .. p:proc_tree():list()[1].argv .. ":" .. p.cwd_basename
                end)
                "#,
            )
            .unwrap();

        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("true:zsh:tpane"));
    }

    #[test]
    fn pane_tables_expose_methods() {
        let (runtime, panes) = runtime();
        panes.borrow_mut().push(pane("%1"));
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.command(function()
                  local p = tpane.panes()[1]
                  return type(p.set) .. ":" .. type(p.var) .. ":" .. type(p.capture)
                end)
                "#,
            )
            .unwrap();

        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
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
                tpane.on("pane:new", function(p) seen = p.id end)
                tpane.on("pane:new", function() error("bad event") end)
                tpane.command(function() return seen end)
                "#,
            )
            .unwrap();

        let errors = runtime.fire_event("pane:new", Some(&pane("%9")));
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad event"));
        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
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
                tpane.on("window:close", function(window) seen = window end)
                tpane.command(function() return seen end)
                "#,
            )
            .unwrap();

        assert!(runtime.fire_event_text("window:close", "@9").is_empty());
        let out = runtime.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(out.as_deref(), Some("@9"));
    }

    #[test]
    fn detect_skips_throwing_kind_and_uses_next_match() {
        let (runtime, _) = runtime();
        runtime
            .load_source(
                "test.lua",
                r#"
                tpane.kind{
                  name = "broken",
                  detect = function() error("bad detect") end,
                  label = function() return "broken" end,
                }
                tpane.kind{
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
