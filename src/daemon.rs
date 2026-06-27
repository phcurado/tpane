use std::cell::RefCell;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::lua_runtime::{
    JobDef, LuaRuntime, StatePresentation, config_lua_files, user_plugin_files,
};
use crate::process::{ProcessProvider, SystemProcessProvider};
use crate::protocol::{PaneSnapshot, Request, Response};
use crate::store::Store;
use crate::tmux;

const MAX_RUNTIME_ERRORS: usize = 50;
const MAX_TMUX_LIVENESS_FAILURES: usize = 5;
const SERVER_SENTINEL: &str = "@tpane_applied";

#[derive(Debug, Clone)]
struct StateRecord {
    raw: String,
    value: String,
}

struct JobResult {
    name: String,
    result: std::result::Result<String, String>,
}

pub fn run(socket: PathBuf) -> Result<()> {
    if socket.exists() {
        fs::remove_file(&socket)
            .with_context(|| format!("failed to remove existing socket {}", socket.display()))?;
    }
    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(&socket)
        .with_context(|| format!("failed to bind socket {}", socket.display()))?;
    listener.set_nonblocking(true)?;

    let mut daemon = Daemon::new()?;
    let started = Instant::now();
    let mut last_scan = Instant::now();
    let mut tmux_liveness_failures = 0usize;

    loop {
        accept_ready(&listener, &mut daemon)?;

        if let Some(sig) = daemon.config_changed() {
            let _ = daemon.reload_plugins();
            daemon.config_sig = sig;
        }

        if last_scan.elapsed() >= Duration::from_secs(1) {
            let _ = daemon.scan();
            last_scan = Instant::now();
        }

        if started.elapsed() > Duration::from_secs(5) {
            if tmux::server_alive() {
                tmux_liveness_failures = 0;
            } else {
                tmux_liveness_failures += 1;
                if tmux_liveness_failures >= MAX_TMUX_LIVENESS_FAILURES {
                    break;
                }
            }
        }

        thread::sleep(Duration::from_millis(100));
    }

    let _ = fs::remove_file(socket);
    Ok(())
}

struct Daemon {
    lua: LuaRuntime,
    process_provider: SystemProcessProvider,
    store: Rc<RefCell<Store>>,
    panes: Rc<RefCell<Vec<PaneSnapshot>>>,
    prev_pane_ids: HashSet<String>,
    prev_windows: HashSet<String>,
    prev_active: Option<String>,
    last_good: HashMap<PathBuf, String>,
    load_errors: Vec<String>,
    runtime_errors: Vec<String>,
    states: HashMap<String, StateRecord>,
    status_strip: String,
    status_left: String,
    status_right: String,
    status_position: Option<String>,
    status_interval: Option<u64>,
    status_rows: Option<usize>,
    status_formats: HashMap<usize, String>,
    options: HashMap<String, String>,
    job_data: Rc<RefCell<HashMap<String, String>>>,
    job_last_run: HashMap<String, Instant>,
    job_running: HashSet<String>,
    job_tx: mpsc::Sender<JobResult>,
    job_rx: mpsc::Receiver<JobResult>,
    pane_borders: HashMap<String, String>,
    pane_vars: HashMap<(String, String), String>,
    config_sig: Vec<(PathBuf, SystemTime)>,
}

impl Daemon {
    fn new() -> Result<Self> {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::load(store_path())));
        let job_data = Rc::new(RefCell::new(HashMap::new()));
        let (job_tx, job_rx) = mpsc::channel();
        let mut daemon = Self {
            lua: LuaRuntime::with_store_and_data(
                Rc::clone(&panes),
                Rc::clone(&store),
                Rc::clone(&job_data),
            )?,
            process_provider: SystemProcessProvider,
            store,
            panes,
            prev_pane_ids: HashSet::new(),
            prev_windows: HashSet::new(),
            prev_active: None,
            last_good: HashMap::new(),
            load_errors: Vec::new(),
            runtime_errors: Vec::new(),
            states: HashMap::new(),
            status_strip: String::new(),
            status_left: String::new(),
            status_right: String::new(),
            status_position: None,
            status_interval: None,
            status_rows: None,
            status_formats: HashMap::new(),
            options: HashMap::new(),
            job_data,
            job_last_run: HashMap::new(),
            job_running: HashSet::new(),
            job_tx,
            job_rx,
            pane_borders: HashMap::new(),
            pane_vars: HashMap::new(),
            config_sig: config_signature(),
        };
        daemon.reload_plugins()?;
        Ok(daemon)
    }

    fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Ping => Response::ok(Some("ok".to_string())),
            Request::Refresh => match self.reload_plugins().and_then(|()| self.scan()) {
                Ok(count) => Response::ok(Some(format!("refreshed {count} panes"))),
                Err(error) => Response::error(error),
            },
            Request::Reload => match self.reload_plugins() {
                Ok(()) => Response::ok(Some(format!(
                    "reloaded {} kinds, {} errors",
                    self.lua.kind_count(),
                    self.load_errors.len()
                ))),
                Err(error) => Response::error(error),
            },
            Request::Status => {
                let errors = self.status_errors();
                if errors.is_empty() {
                    Response::ok(Some("ok".to_string()))
                } else {
                    Response::ok(Some(errors.join("\n")))
                }
            }
            Request::Panes => match self.panes_data() {
                Ok(data) => Response::ok(Some(data)),
                Err(error) => Response::error(error),
            },
            Request::Panels => match self.panels_data() {
                Ok(data) => Response::ok(Some(data)),
                Err(error) => Response::error(error),
            },
            Request::SelectPane { id } => match self.select_pane(&id) {
                Ok(()) => Response::ok(Some("selected".to_string())),
                Err(error) => Response::error(error),
            },
            Request::ExpandPane { id } => match self.expand_pane(&id) {
                Ok(()) => Response::ok(Some("expanded".to_string())),
                Err(error) => Response::error(error),
            },
            Request::SetState { id, state } => match self.set_state(&id, &state) {
                Ok(()) => Response::ok(Some("set".to_string())),
                Err(error) => Response::error(error),
            },
            Request::Doctor { clean } => match self.doctor(clean) {
                Ok(report) => Response::ok(Some(report)),
                Err(error) => Response::error(error),
            },
            Request::Command { name, args } => {
                match self.scan().and_then(|_| self.lua.run_command(&name, &args)) {
                    Ok(data) => Response::ok(data),
                    Err(error) => {
                        self.record_runtime_error(format!("command {name}: {error}"));
                        Response::error(error)
                    }
                }
            }
        }
    }

    fn reload_plugins(&mut self) -> Result<()> {
        let rt = match LuaRuntime::with_store_and_data(
            Rc::clone(&self.panes),
            Rc::clone(&self.store),
            Rc::clone(&self.job_data),
        ) {
            Ok(rt) => rt,
            Err(error) => {
                self.load_errors = vec![format!("prelude.lua: {error}")];
                self.surface_load_errors();
                return Err(error);
            }
        };
        let mut errors = Vec::new();

        for path in user_plugin_files() {
            let name = path.display().to_string();
            match fs::read_to_string(&path) {
                Ok(source) => match rt.load_source(&name, &source) {
                    Ok(()) => {
                        self.last_good.insert(path, source);
                    }
                    Err(error) => {
                        errors.push(format!("{name}: {error}"));
                        if let Some(source) = self.last_good.get(&path)
                            && let Err(fallback_error) = rt.load_source(&name, source)
                        {
                            errors.push(format!("{name}: last-good failed: {fallback_error}"));
                        }
                    }
                },
                Err(error) => {
                    errors.push(format!("{name}: {error}"));
                    if let Some(source) = self.last_good.get(&path)
                        && let Err(fallback_error) = rt.load_source(&name, source)
                    {
                        errors.push(format!("{name}: last-good failed: {fallback_error}"));
                    }
                }
            }
        }

        if let Err(error) = rt.load_builtins() {
            self.load_errors = vec![format!("builtin-kinds.lua: {error}")];
            self.surface_load_errors();
            return Err(error);
        }
        errors.extend(rt.run_deferred());

        for unbind in rt.unbinds() {
            if let Err(error) = tmux::unbind_key(&unbind.mode, &unbind.key) {
                errors.push(format!("unbind {} {}: {error}", unbind.mode, unbind.key));
            }
        }

        for keybind in rt.keybinds() {
            let command = if keybind.raw {
                keybind.command.join(" ")
            } else {
                keybind_command(&keybind.command, keybind.context)
            };
            if let Err(error) = tmux::bind_key(
                &keybind.mode,
                &keybind.key,
                &command,
                keybind.popup,
                keybind.raw,
            ) {
                errors.push(format!("keybind {} {}: {error}", keybind.mode, keybind.key));
            }
        }

        self.lua = rt;
        self.ensure_server_cache_current()?;
        self.apply_status_options()?;
        self.load_errors = errors;
        self.runtime_errors.clear();
        if !self.load_errors.is_empty() {
            self.surface_load_errors();
        }
        self.config_sig = config_signature();
        Ok(())
    }

    fn ensure_server_cache_current(&mut self) -> Result<()> {
        if tmux::get_global_var(SERVER_SENTINEL).unwrap_or_default() != "1" {
            self.reset_applied_cache();
            tmux::set_global_var(SERVER_SENTINEL, "1")?;
        }
        Ok(())
    }

    fn reset_applied_cache(&mut self) {
        self.prev_pane_ids.clear();
        self.prev_windows.clear();
        self.prev_active = None;
        self.status_strip.clear();
        self.status_left.clear();
        self.status_right.clear();
        self.status_position = None;
        self.status_interval = None;
        self.status_rows = None;
        self.status_formats.clear();
        self.options.clear();
        self.pane_borders.clear();
        self.pane_vars.clear();
    }

    fn config_changed(&self) -> Option<Vec<(PathBuf, SystemTime)>> {
        let sig = config_signature();
        (sig != self.config_sig).then_some(sig)
    }

    fn scan(&mut self) -> Result<usize> {
        self.ensure_server_cache_current()?;
        self.update_jobs();
        let panes = tmux::list_panes()?;
        let count = panes.len();
        let mut snapshots = Vec::new();

        let table = self.process_provider.snapshot().unwrap_or_default();

        for pane in panes {
            let proc_tree = table.tree(pane.pid);
            if let Some(detection) = self.lua.detect(&pane, proc_tree.clone())? {
                self.set_pane_var(&pane.id, "@tpane_kind", &detection.kind)?;
                self.set_pane_var(&pane.id, "@tpane_label", &detection.label)?;
                if let Some(color) = &detection.color {
                    self.set_pane_var(&pane.id, "@tpane_color", color)?;
                } else {
                    self.unset_pane_var(&pane.id, "@tpane_color")?;
                }
                let tag = self.update_pane_tag(&pane, detection.tag.as_deref())?;
                let state = detection
                    .raw_state
                    .as_deref()
                    .map(|raw| self.update_state(&pane.id, raw, pane.active))
                    .transpose()?
                    .flatten()
                    .or(pane.state.clone());
                snapshots.push(PaneSnapshot {
                    id: pane.id.clone(),
                    pid: pane.pid,
                    kind: detection.kind,
                    label: detection.label,
                    cwd: pane.cwd.clone(),
                    cwd_basename: basename(&pane.cwd),
                    command: pane.command.clone(),
                    session: pane.session.clone(),
                    window: pane.window.clone(),
                    active: pane.active,
                    zoomed: pane.zoomed,
                    tag,
                    home: pane.home.clone(),
                    state,
                    processes: proc_tree,
                });
            }
        }

        let current_pane_id = current_status_pane_id(&snapshots);
        let current_ids = snapshots
            .iter()
            .map(|pane| pane.id.clone())
            .collect::<HashSet<_>>();
        self.pane_vars
            .retain(|(pane_id, _), _| current_ids.contains(pane_id));
        self.pane_borders
            .retain(|pane_id, _| current_ids.contains(pane_id));
        *self.panes.borrow_mut() = snapshots.clone();

        self.update_pane_borders(&snapshots)?;

        let status = status_strip(&snapshots, !self.status_errors().is_empty(), |state| {
            self.lua.state_presentation(state)
        });
        if status != self.status_strip {
            tmux::set_global_var("@tpane_status", &status)?;
            self.status_strip = status;
        }
        self.update_events(&snapshots);
        self.update_statusline(current_pane_id.as_deref())?;
        self.store.borrow_mut().flush()?;
        Ok(count)
    }

    fn update_jobs(&mut self) {
        let jobs = self.lua.jobs();
        let names = jobs
            .iter()
            .map(|source| source.name.clone())
            .collect::<HashSet<_>>();
        self.drain_job_results(&names);
        self.job_data
            .borrow_mut()
            .retain(|name, _| names.contains(name));
        self.job_last_run.retain(|name, _| names.contains(name));
        self.job_running.retain(|name| names.contains(name));

        let now = Instant::now();
        for source in jobs {
            if self.job_running.contains(&source.name) {
                continue;
            }
            let due = self
                .job_last_run
                .get(&source.name)
                .is_none_or(|last| now.duration_since(*last) >= source.every);
            if due {
                self.start_job(source, now);
            }
        }
    }

    fn drain_job_results(&mut self, names: &HashSet<String>) {
        while let Ok(result) = self.job_rx.try_recv() {
            self.job_running.remove(&result.name);
            if !names.contains(&result.name) {
                continue;
            }
            match result.result {
                Ok(value) => {
                    self.job_data.borrow_mut().insert(result.name, value);
                }
                Err(error) => {
                    self.record_runtime_error(format!("job {}: {error}", result.name));
                }
            }
        }
    }

    fn start_job(&mut self, job: JobDef, now: Instant) {
        self.job_last_run.insert(job.name.clone(), now);
        self.job_running.insert(job.name.clone());
        let tx = self.job_tx.clone();
        thread::spawn(move || {
            let result = run_job_command(&job.command, job.timeout);
            let _ = tx.send(JobResult {
                name: job.name,
                result,
            });
        });
    }

    fn update_pane_tag(
        &mut self,
        pane: &tmux::PaneInfo,
        detected: Option<&str>,
    ) -> Result<Option<String>> {
        let key = (pane.id.clone(), "@tpane_tag".to_string());
        match detected {
            Some(tag) => {
                if let Some(existing) = pane.tag.as_deref() {
                    let owned = self
                        .pane_vars
                        .get(&key)
                        .is_some_and(|value| value == existing);
                    if !owned {
                        return Ok(Some(existing.to_string()));
                    }
                    if existing != tag {
                        self.set_pane_var(&pane.id, "@tpane_tag", tag)?;
                    }
                    return Ok(Some(tag.to_string()));
                }

                self.set_pane_var(&pane.id, "@tpane_tag", tag)?;
                Ok(Some(tag.to_string()))
            }
            None => {
                let owned = self
                    .pane_vars
                    .get(&key)
                    .is_some_and(|value| pane.tag.as_deref() == Some(value.as_str()));
                if owned {
                    self.unset_pane_var(&pane.id, "@tpane_tag")?;
                    Ok(None)
                } else {
                    Ok(pane.tag.clone())
                }
            }
        }
    }

    fn set_pane_var(&mut self, pane_id: &str, name: &str, value: &str) -> Result<()> {
        let key = (pane_id.to_string(), name.to_string());
        if self.pane_vars.get(&key).map(String::as_str) == Some(value) {
            return Ok(());
        }
        tmux::set_pane_var(pane_id, name, value)?;
        self.pane_vars.insert(key, value.to_string());
        Ok(())
    }

    fn unset_pane_var(&mut self, pane_id: &str, name: &str) -> Result<()> {
        let key = (pane_id.to_string(), name.to_string());
        if !self.pane_vars.contains_key(&key) {
            return Ok(());
        }
        tmux::unset_pane_var(pane_id, name)?;
        self.pane_vars.remove(&key);
        Ok(())
    }

    fn update_pane_borders(&mut self, snapshots: &[PaneSnapshot]) -> Result<()> {
        for pane in snapshots {
            match self.lua.render_pane_border(pane) {
                Ok(Some(border)) if self.pane_borders.get(&pane.id) != Some(&border) => {
                    tmux::set_pane_var(&pane.id, "@tpane_border", &border)?;
                    self.pane_borders.insert(pane.id.clone(), border);
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    if self.pane_borders.remove(&pane.id).is_some() {
                        tmux::unset_pane_var(&pane.id, "@tpane_border")?;
                    }
                }
                Err(error) => {
                    self.record_runtime_error(format!("pane border {}: {error}", pane.id));
                }
            }
        }
        Ok(())
    }

    fn apply_status_options(&mut self) -> Result<()> {
        let next_options = self.lua.options().into_iter().collect::<HashMap<_, _>>();
        for name in self
            .options
            .keys()
            .filter(|name| !next_options.contains_key(*name))
            .cloned()
            .collect::<Vec<_>>()
        {
            tmux::unset_global_var(&name)?;
            self.options.remove(&name);
        }
        for (name, value) in next_options {
            if self.options.get(&name) != Some(&value) {
                tmux::set_global_var(&name, &value)?;
                self.options.insert(name, value);
            }
        }

        for (name, value) in self.lua.option_appends() {
            let current = tmux::get_global_var(&name).unwrap_or_default();
            if !current.contains(&value) {
                tmux::append_global_var(&name, &value)?;
            }
        }

        let status = self.lua.status_options();
        self.apply_status_position_interval(status.position, status.interval)?;
        self.apply_status_rows(status.rows)
    }

    fn apply_status_position_interval(
        &mut self,
        position: Option<String>,
        interval: Option<u64>,
    ) -> Result<()> {
        if position != self.status_position {
            if let Some(position) = &position {
                tmux::set_status_position(position)?;
            } else {
                tmux::unset_global_var("status-position")?;
            }
            self.status_position = position;
        }
        if interval != self.status_interval {
            if let Some(interval) = interval {
                tmux::set_status_interval(interval)?;
            } else {
                tmux::unset_global_var("status-interval")?;
            }
            self.status_interval = interval;
        }
        Ok(())
    }

    fn apply_status_rows(&mut self, rows: Option<usize>) -> Result<()> {
        if rows != self.status_rows {
            if let Some(rows) = rows {
                let value = if rows == 1 {
                    "on".to_string()
                } else {
                    rows.to_string()
                };
                tmux::set_global_var("status", &value)?;
                self.status_rows = Some(rows);
            } else if self.status_rows.is_some() {
                tmux::unset_global_var("status")?;
                self.status_rows = None;
            }
        }
        Ok(())
    }

    fn update_status_formats(&mut self, formats: Vec<(usize, String)>) -> Result<()> {
        let next = formats.into_iter().collect::<HashMap<_, _>>();
        for index in self
            .status_formats
            .keys()
            .filter(|index| !next.contains_key(*index))
            .copied()
            .collect::<Vec<_>>()
        {
            tmux::unset_global_var(&format!("status-format[{index}]"))?;
            self.status_formats.remove(&index);
        }
        for (index, value) in next {
            if self.status_formats.get(&index) != Some(&value) {
                tmux::set_global_var(&format!("status-format[{index}]"), &value)?;
                self.status_formats.insert(index, value);
            }
        }
        Ok(())
    }

    fn update_statusline(&mut self, current_pane_id: Option<&str>) -> Result<()> {
        let (status, errors) = self.lua.render_statusline(current_pane_id);
        self.record_runtime_errors(errors);
        self.apply_status_position_interval(status.position, status.interval)?;
        self.apply_status_rows(status.rows)?;
        self.update_status_formats(status.formats)?;
        match status.left {
            Some(left) if left != self.status_left => {
                tmux::set_status("left", &left)?;
                self.status_left = left;
            }
            None if !self.status_left.is_empty() => {
                tmux::unset_global_var("status-left")?;
                self.status_left.clear();
            }
            _ => {}
        }
        match status.right {
            Some(right) if right != self.status_right => {
                tmux::set_status("right", &right)?;
                self.status_right = right;
            }
            None if !self.status_right.is_empty() => {
                tmux::unset_global_var("status-right")?;
                self.status_right.clear();
            }
            _ => {}
        }
        Ok(())
    }

    fn update_events(&mut self, snapshots: &[PaneSnapshot]) {
        let current_ids = snapshots
            .iter()
            .map(|pane| pane.id.clone())
            .collect::<HashSet<_>>();
        let current_windows = snapshots
            .iter()
            .map(|pane| pane.window.clone())
            .collect::<HashSet<_>>();
        for pane in snapshots {
            if !self.prev_pane_ids.contains(&pane.id) {
                self.record_runtime_errors(self.lua.fire_event("pane:new", Some(pane)));
            }
        }

        let active = snapshots
            .iter()
            .find(|pane| pane.active)
            .map(|pane| pane.id.clone());
        if active != self.prev_active
            && let Some(active_id) = &active
        {
            self.mark_seen(active_id);
            if let Some(pane) = snapshots.iter().find(|pane| &pane.id == active_id) {
                self.record_runtime_errors(self.lua.fire_event("pane:focus", Some(pane)));
            }
        }

        for window in self
            .prev_windows
            .difference(&current_windows)
            .cloned()
            .collect::<Vec<_>>()
        {
            self.record_runtime_errors(self.lua.fire_event_text("window:close", &window));
        }

        self.record_runtime_errors(self.lua.fire_event("tick", None));
        self.prev_pane_ids = current_ids;
        self.prev_windows = current_windows;
        self.prev_active = active;
    }

    fn surface_load_errors(&self) {
        if self.load_errors.is_empty() {
            return;
        }
        let first = self
            .load_errors
            .first()
            .map(|error| error.lines().next().unwrap_or(error))
            .unwrap_or("Lua load error");
        let message = if self.load_errors.len() == 1 {
            format!("tpane: {first}")
        } else {
            format!(
                "tpane: {} load errors; run tpane status",
                self.load_errors.len()
            )
        };
        let _ = tmux::display_global_message(&message);
    }

    fn status_errors(&self) -> Vec<String> {
        self.load_errors
            .iter()
            .chain(self.runtime_errors.iter())
            .cloned()
            .collect()
    }

    fn record_runtime_errors(&mut self, errors: Vec<String>) {
        for error in errors {
            self.record_runtime_error(error);
        }
    }

    fn record_runtime_error(&mut self, error: String) {
        if self.runtime_errors.contains(&error) {
            return;
        }
        self.runtime_errors.push(error);
        if self.runtime_errors.len() > MAX_RUNTIME_ERRORS {
            self.runtime_errors.remove(0);
        }
    }

    fn update_state(&mut self, pane_id: &str, raw: &str, active: bool) -> Result<Option<String>> {
        let previous = self.states.get(pane_id).cloned();
        let value = state_value(raw, active, previous.as_ref());
        let changed = previous.as_ref().map(|record| record.value.as_str()) != Some(value.as_str());
        self.states.insert(
            pane_id.to_string(),
            StateRecord {
                raw: raw.to_string(),
                value: value.clone(),
            },
        );
        self.set_pane_var(pane_id, "@tpane_state", &value)?;
        if changed {
            self.record_runtime_errors(self.lua.fire_event_text("state:change", pane_id));
        }
        Ok(Some(value))
    }

    fn mark_seen(&mut self, pane_id: &str) {
        let Some(record) = self.states.get_mut(pane_id) else {
            return;
        };
        if record.value != "done_unseen" {
            return;
        }
        mark_record_seen(record);
        let _ = self.set_pane_var(pane_id, "@tpane_state", "idle_seen");
        self.record_runtime_errors(self.lua.fire_event_text("state:change", pane_id));
    }

    fn set_state(&mut self, pane_id: &str, state: &str) -> Result<()> {
        if state == "idle" || state == "idle_seen" {
            tmux::unset_pane_var(pane_id, "@tpane_push_state")?;
        } else {
            tmux::set_pane_var(pane_id, "@tpane_push_state", state)?;
        }
        let active = self
            .panes
            .borrow()
            .iter()
            .find(|pane| pane.id == pane_id)
            .map(|pane| pane.active)
            .unwrap_or(false);
        self.update_state(pane_id, state, active)?;
        Ok(())
    }

    fn panes_data(&mut self) -> Result<String> {
        self.scan()?;
        Ok(serde_json::to_string(&*self.panes.borrow())?)
    }

    fn panels_data(&mut self) -> Result<String> {
        self.scan()?;
        Ok(serde_json::to_string(&self.lua.render_panels()?)?)
    }

    fn select_pane(&mut self, id: &str) -> Result<()> {
        self.scan()?;
        if self.panes.borrow().iter().any(|pane| pane.id == id) {
            tmux::select_pane(id)
        } else {
            anyhow::bail!("unknown pane {id}")
        }
    }

    fn expand_pane(&mut self, id: &str) -> Result<()> {
        self.scan()?;
        if !self.panes.borrow().iter().any(|pane| pane.id == id) {
            anyhow::bail!("unknown pane {id}");
        }
        let window = tmux::window_id(id)?;
        let active = tmux::active_pane(&window)?;
        if tmux::is_zoomed(&window)? {
            if active == id {
                tmux::zoom(id)?;
                return Ok(());
            }
            tmux::zoom(&active)?;
        }
        tmux::select_pane(id)?;
        tmux::zoom(id)
    }

    fn doctor(&mut self, clean: bool) -> Result<String> {
        self.scan()?;
        let panes = self.panes.borrow().clone();
        let hidden_sessions = panes
            .iter()
            .filter(|pane| is_hidden_session(&pane.session))
            .map(|pane| pane.session.clone())
            .collect::<HashSet<_>>();
        let stateful = panes.iter().filter(|pane| pane.state.is_some()).count();
        let mut issues = Vec::new();
        let mut cleaned = Vec::new();
        let mut seen: HashMap<(String, String, String), String> = HashMap::new();

        for pane in panes.iter().filter(|pane| is_hidden_session(&pane.session)) {
            let expected_home = hidden_session_home(&pane.session).unwrap_or_default();
            let tag = pane.tag.clone().unwrap_or_else(|| pane.kind.clone());
            let home = pane.home.clone().unwrap_or_default();
            if home != expected_home {
                issues.push(format!(
                    "wrong home: {} tag={} home={} session={}",
                    pane.id, tag, home, pane.session
                ));
                if clean && tmux::kill_pane(&pane.id).is_ok() {
                    cleaned.push(pane.id.clone());
                }
                continue;
            }

            let key = (pane.session.clone(), tag.clone(), home.clone());
            if let Some(first) = seen.insert(key, pane.id.clone()) {
                issues.push(format!(
                    "duplicate hidden pane: {} duplicates {} tag={} home={}",
                    pane.id, first, tag, home
                ));
                if clean && tmux::kill_pane(&pane.id).is_ok() {
                    cleaned.push(pane.id.clone());
                }
            }
        }

        let panels = self
            .lua
            .render_panels()
            .map(|panels| panels.len())
            .unwrap_or(0);
        let errors = self.status_errors();
        let mut report = vec![
            if issues.is_empty() {
                "ok".to_string()
            } else {
                "issues".to_string()
            },
            format!("panes: {}", panes.len()),
            format!("stateful panes: {stateful}"),
            format!("hidden sessions: {}", hidden_sessions.len()),
            format!("keybinds: {}", self.lua.keybinds().len()),
            format!("panels: {panels}"),
            format!(
                "status: {}",
                if errors.is_empty() { "ok" } else { "errors" }
            ),
        ];

        if !issues.is_empty() {
            report.push("".to_string());
            report.push("issues:".to_string());
            report.extend(issues.iter().map(|issue| format!("  {issue}")));
        }
        if clean && !cleaned.is_empty() {
            report.push("".to_string());
            report.push("cleaned:".to_string());
            report.extend(cleaned.iter().map(|pane| format!("  {pane}")));
        }
        if !errors.is_empty() {
            report.push("".to_string());
            report.push("errors:".to_string());
            report.extend(errors.iter().map(|error| format!("  {error}")));
        }

        Ok(report.join("\n"))
    }
}

#[cfg(test)]
fn should_exit_after_liveness_failure(failures: usize) -> bool {
    failures >= MAX_TMUX_LIVENESS_FAILURES
}

fn basename(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn mark_record_seen(record: &mut StateRecord) {
    record.raw = "idle".to_string();
    record.value = "idle_seen".to_string();
}

fn state_value(raw: &str, active: bool, previous: Option<&StateRecord>) -> String {
    match raw {
        "blocked" => "blocked".to_string(),
        "working" => "working".to_string(),
        "idle" => {
            if active {
                "idle_seen".to_string()
            } else if matches!(
                previous.map(|record| record.value.as_str()),
                Some("working" | "done_unseen")
            ) {
                "done_unseen".to_string()
            } else {
                "idle_seen".to_string()
            }
        }
        "done" => {
            if active {
                "idle_seen".to_string()
            } else {
                "done_unseen".to_string()
            }
        }
        other => other.to_string(),
    }
}

fn status_strip(
    panes: &[PaneSnapshot],
    has_errors: bool,
    presentation: impl Fn(&str) -> Option<StatePresentation>,
) -> String {
    let mut parts = Vec::new();
    if has_errors {
        parts.push("#[fg=red]tpane error#[default]".to_string());
    }
    parts.extend(
        panes
            .iter()
            .filter(|pane| pane.state.is_some())
            .map(|pane| {
                format!(
                    "{} {}",
                    status_dot(pane.state.as_deref(), &presentation),
                    pane.label
                )
            }),
    );
    parts.join("  ")
}

fn is_hidden_session(session: &str) -> bool {
    hidden_session_home(session).is_some()
}

fn hidden_session_home(session: &str) -> Option<&str> {
    session
        .strip_prefix("__tpane-hidden-")
        .or_else(|| session.strip_prefix("__pi-hidden-"))
}

fn status_dot(
    state: Option<&str>,
    presentation: &impl Fn(&str) -> Option<StatePresentation>,
) -> String {
    let Some(state) = state else {
        return String::new();
    };
    let Some(presentation) = presentation(state) else {
        return String::new();
    };
    let Some(color) = presentation.color else {
        return String::new();
    };
    let glyph = presentation.glyph.unwrap_or_else(|| "●".to_string());
    format!("#[fg={color}]{glyph}#[default]")
}

fn current_status_pane_id(snapshots: &[PaneSnapshot]) -> Option<String> {
    if let Ok(current) = tmux::current_pane()
        && snapshots.iter().any(|pane| pane.id == current)
    {
        return Some(current);
    }

    if let Ok(window) = tmux::current_window()
        && let Some(pane) = snapshots
            .iter()
            .find(|pane| pane.window == window && pane.active)
    {
        return Some(pane.id.clone());
    }

    snapshots.first().map(|pane| pane.id.clone())
}

fn keybind_command(command: &[String], context: bool) -> String {
    if command == ["reload"] {
        return "tpane reload".to_string();
    }

    let mut parts = vec!["tpane".to_string(), "run".to_string()];
    parts.extend(command.iter().cloned());
    if context {
        parts.push("#{pane_id}".to_string());
    }
    parts.join(" ")
}

fn store_path() -> PathBuf {
    let root = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .or_else(|| std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
        .unwrap_or_else(|| std::env::temp_dir().join("tpane-state"));
    root.join("tpane")
        .join(format!("tpane-{}.json", tmux_server_key()))
}

fn tmux_server_key() -> String {
    let server = std::env::var("TMUX")
        .ok()
        .and_then(|value| value.split(',').next().map(str::to_string))
        .unwrap_or_else(default_tmux_socket_path);
    let mut hasher = DefaultHasher::new();
    server.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn default_tmux_socket_path() -> String {
    let tmp = std::env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let uid = std::env::var("UID").unwrap_or_else(|_| "unknown".to_string());
    format!("{tmp}/tmux-{uid}/default")
}

fn config_signature() -> Vec<(PathBuf, SystemTime)> {
    config_lua_files()
        .into_iter()
        .map(|path| {
            let modified = fs::metadata(&path)
                .and_then(|metadata| metadata.modified())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            (path, modified)
        })
        .collect()
}

fn accept_ready(listener: &UnixListener, daemon: &mut Daemon) -> Result<()> {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => handle_stream(stream, daemon)?,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(error) => return Err(error.into()),
        }
    }
}

fn handle_stream(mut stream: UnixStream, daemon: &mut Daemon) -> Result<()> {
    let mut line = String::new();
    BufReader::new(stream.try_clone()?).read_line(&mut line)?;
    let response = match serde_json::from_str::<Request>(&line) {
        Ok(request) => daemon.handle(request),
        Err(error) => Response::error(error),
    };
    serde_json::to_writer(&mut stream, &response)?;
    stream.write_all(b"\n")?;
    Ok(())
}

fn run_job_command(command: &str, timeout: Duration) -> std::result::Result<String, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if started.elapsed() >= timeout => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {}s", timeout.as_secs()));
            }
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(error) => return Err(error.to_string()),
        }
    }

    let output = child
        .wait_with_output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end_matches(['\r', '\n'])
            .to_string())
    } else {
        let error = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if error.is_empty() {
            Err(output.status.to_string())
        } else {
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, active: bool) -> PaneSnapshot {
        pane_in_window(id, active, "@1")
    }

    fn pane_in_window(id: &str, active: bool, window: &str) -> PaneSnapshot {
        PaneSnapshot {
            id: id.to_string(),
            pid: 123,
            kind: "term".to_string(),
            label: "term".to_string(),
            cwd: "/tmp".to_string(),
            cwd_basename: "tmp".to_string(),
            command: "zsh".to_string(),
            session: "s".to_string(),
            window: window.to_string(),
            active,
            zoomed: false,
            tag: None,
            home: None,
            state: None,
            processes: Vec::new(),
        }
    }

    fn tmux_pane_with_tag(tag: Option<&str>) -> tmux::PaneInfo {
        tmux::PaneInfo {
            id: "%1".to_string(),
            pid: 123,
            cwd: "/tmp".to_string(),
            command: "zsh".to_string(),
            session: "s".to_string(),
            window: "@1".to_string(),
            active: true,
            zoomed: false,
            tag: tag.map(str::to_string),
            home: None,
            state: None,
        }
    }

    fn test_daemon(lua_source: &str) -> Daemon {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let store = Rc::new(RefCell::new(Store::memory()));
        let job_data = Rc::new(RefCell::new(HashMap::new()));
        let (job_tx, job_rx) = mpsc::channel();
        let lua = LuaRuntime::with_store_and_data(
            Rc::clone(&panes),
            Rc::clone(&store),
            Rc::clone(&job_data),
        )
        .unwrap();
        lua.load_source("test.lua", lua_source).unwrap();
        Daemon {
            lua,
            process_provider: SystemProcessProvider,
            store,
            panes,
            prev_pane_ids: HashSet::new(),
            prev_windows: HashSet::new(),
            prev_active: None,
            last_good: HashMap::new(),
            load_errors: Vec::new(),
            runtime_errors: Vec::new(),
            states: HashMap::new(),
            status_strip: String::new(),
            status_left: String::new(),
            status_right: String::new(),
            status_position: None,
            status_interval: None,
            status_rows: None,
            status_formats: HashMap::new(),
            options: HashMap::new(),
            job_data,
            job_last_run: HashMap::new(),
            job_running: HashSet::new(),
            job_tx,
            job_rx,
            pane_borders: HashMap::new(),
            pane_vars: HashMap::new(),
            config_sig: Vec::new(),
        }
    }

    #[test]
    fn detected_tag_does_not_overwrite_external_pane_tag() {
        let mut daemon = test_daemon("");
        let pane = tmux_pane_with_tag(Some("manual"));

        let tag = daemon.update_pane_tag(&pane, Some("detected")).unwrap();

        assert_eq!(tag.as_deref(), Some("manual"));
        assert!(daemon.pane_vars.is_empty());
    }

    #[test]
    fn detected_tag_keeps_owned_pane_tag() {
        let mut daemon = test_daemon("");
        daemon.pane_vars.insert(
            ("%1".to_string(), "@tpane_tag".to_string()),
            "detected".to_string(),
        );
        let pane = tmux_pane_with_tag(Some("detected"));

        let tag = daemon.update_pane_tag(&pane, Some("detected")).unwrap();

        assert_eq!(tag.as_deref(), Some("detected"));
        assert_eq!(
            daemon
                .pane_vars
                .get(&("%1".to_string(), "@tpane_tag".to_string()))
                .map(String::as_str),
            Some("detected")
        );
    }

    #[test]
    fn reset_applied_cache_clears_tmux_debounce_state() {
        let mut daemon = test_daemon("");
        daemon.prev_pane_ids.insert("%1".to_string());
        daemon.prev_windows.insert("@1".to_string());
        daemon.prev_active = Some("%1".to_string());
        daemon.status_strip = "strip".to_string();
        daemon.status_left = "left".to_string();
        daemon.status_right = "right".to_string();
        daemon.status_position = Some("top".to_string());
        daemon.status_interval = Some(1);
        daemon.status_rows = Some(2);
        daemon.status_formats.insert(1, "extra".to_string());
        daemon.options.insert("mouse".to_string(), "on".to_string());
        daemon
            .pane_borders
            .insert("%1".to_string(), "border".to_string());
        daemon.pane_vars.insert(
            ("%1".to_string(), "@tpane_kind".to_string()),
            "pi".to_string(),
        );

        daemon.reset_applied_cache();

        assert!(daemon.prev_pane_ids.is_empty());
        assert!(daemon.prev_windows.is_empty());
        assert!(daemon.prev_active.is_none());
        assert!(daemon.status_strip.is_empty());
        assert!(daemon.status_left.is_empty());
        assert!(daemon.status_right.is_empty());
        assert!(daemon.status_position.is_none());
        assert!(daemon.status_interval.is_none());
        assert!(daemon.status_rows.is_none());
        assert!(daemon.status_formats.is_empty());
        assert!(daemon.options.is_empty());
        assert!(daemon.pane_borders.is_empty());
        assert!(daemon.pane_vars.is_empty());
    }

    #[test]
    fn update_events_fires_new_focus_and_tick_without_tmux() {
        let mut daemon = test_daemon(
            r#"
            counts = { new = 0, focus = 0, tick = 0 }
            focused = ""
            tpane.on("pane:new", function(_) counts.new = counts.new + 1 end)
            tpane.on("pane:focus", function(p) counts.focus = counts.focus + 1; focused = p.id end)
            tpane.on("tick", function() counts.tick = counts.tick + 1 end)
            tpane.command(function()
              return counts.new .. ":" .. counts.focus .. ":" .. counts.tick .. ":" .. focused
            end)
            "#,
        );

        daemon.update_events(&[pane("%1", true)]);
        let first = daemon.lua.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(first.as_deref(), Some("1:1:1:%1"));

        daemon.update_events(&[pane("%1", true)]);
        let second = daemon.lua.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(second.as_deref(), Some("1:1:2:%1"));

        daemon.update_events(&[pane("%1", false), pane("%2", true)]);
        let third = daemon.lua.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(third.as_deref(), Some("2:2:3:%2"));
    }

    #[test]
    fn update_events_fires_window_close_with_window_id() {
        let mut daemon = test_daemon(
            r#"
            closed = ""
            tpane.on("window:close", function(window) closed = window end)
            tpane.command(function() return closed end)
            "#,
        );

        daemon.update_events(&[pane_in_window("%1", true, "@1")]);
        daemon.update_events(&[pane_in_window("%2", true, "@2")]);

        let closed = daemon.lua.run_command("__tpane_command_1", &[]).unwrap();
        assert_eq!(closed.as_deref(), Some("@1"));
    }

    #[test]
    fn event_errors_are_collected_without_crashing() {
        let mut daemon = test_daemon(
            r#"
            tpane.on("tick", function() error("tick failed") end)
            "#,
        );

        daemon.update_events(&[]);
        assert_eq!(daemon.runtime_errors.len(), 1);
        assert!(daemon.runtime_errors[0].contains("tick failed"));
    }

    #[test]
    fn daemon_exits_only_after_consecutive_liveness_failures() {
        assert!(!should_exit_after_liveness_failure(
            MAX_TMUX_LIVENESS_FAILURES - 1
        ));
        assert!(should_exit_after_liveness_failure(
            MAX_TMUX_LIVENESS_FAILURES
        ));
    }

    #[test]
    fn job_command_times_out() {
        let result = run_job_command("sleep 1", Duration::from_millis(100));

        assert!(result.unwrap_err().contains("timed out"));
    }

    #[test]
    fn status_strip_shows_pane_states() {
        let presentation = |state: &str| match state {
            "blocked" => Some(StatePresentation {
                color: Some("red".to_string()),
                glyph: Some("●".to_string()),
            }),
            "idle_seen" => Some(StatePresentation {
                color: Some("green".to_string()),
                glyph: Some("●".to_string()),
            }),
            _ => None,
        };
        assert_eq!(
            status_dot(Some("blocked"), &presentation),
            "#[fg=red]●#[default]"
        );
        assert!(status_strip(&[pane("%1", true)], false, presentation).is_empty());
        let mut stateful = pane("%2", false);
        stateful.label = "worker".to_string();
        stateful.state = Some("idle_seen".to_string());
        assert_eq!(
            status_strip(&[stateful], false, presentation),
            "#[fg=green]●#[default] worker"
        );
        assert_eq!(
            status_strip(&[], true, presentation),
            "#[fg=red]tpane error#[default]"
        );
    }

    #[test]
    fn keybind_command_injects_invoking_pane_context() {
        assert_eq!(
            keybind_command(&["pi".to_string(), "expand".to_string()], true),
            "tpane run pi expand #{pane_id}"
        );
        assert_eq!(
            keybind_command(&["control".to_string()], false),
            "tpane run control"
        );
        assert_eq!(
            keybind_command(&["reload".to_string()], false),
            "tpane reload"
        );
    }

    #[test]
    fn state_value_marks_finished_unseen_until_focus() {
        assert_eq!(state_value("working", false, None), "working");
        assert_eq!(
            state_value(
                "idle",
                false,
                Some(&StateRecord {
                    raw: "working".to_string(),
                    value: "working".to_string(),
                })
            ),
            "done_unseen"
        );
        assert_eq!(
            state_value(
                "idle",
                true,
                Some(&StateRecord {
                    raw: "working".to_string(),
                    value: "working".to_string(),
                })
            ),
            "idle_seen"
        );
        assert_eq!(state_value("done", false, None), "done_unseen");
        assert_eq!(state_value("done", true, None), "idle_seen");
    }

    #[test]
    fn mark_record_seen_sets_idle_seen() {
        let mut record = StateRecord {
            raw: "idle".to_string(),
            value: "done_unseen".to_string(),
        };
        mark_record_seen(&mut record);
        assert_eq!(record.value, "idle_seen");
    }

    #[test]
    fn runtime_errors_are_deduped_and_capped() {
        let mut daemon = test_daemon("");

        daemon.record_runtime_error("same".to_string());
        daemon.record_runtime_error("same".to_string());
        assert_eq!(daemon.runtime_errors, ["same"]);

        for idx in 0..(MAX_RUNTIME_ERRORS + 5) {
            daemon.record_runtime_error(format!("error {idx}"));
        }
        assert_eq!(daemon.runtime_errors.len(), MAX_RUNTIME_ERRORS);
        assert!(!daemon.runtime_errors.contains(&"same".to_string()));
    }
}
