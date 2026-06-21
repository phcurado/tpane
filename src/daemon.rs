use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context, Result};

use crate::lua_runtime::{LuaRuntime, user_plugin_files};
use crate::process::{ProcessProvider, SystemProcessProvider};
use crate::protocol::{DAEMON_SIGNATURE, PaneSnapshot, Request, Response};
use crate::tmux;

const MAX_RUNTIME_ERRORS: usize = 50;

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

        if started.elapsed() > Duration::from_secs(5) && !tmux::server_alive() {
            break;
        }

        thread::sleep(Duration::from_millis(100));
    }

    let _ = fs::remove_file(socket);
    Ok(())
}

struct Daemon {
    lua: LuaRuntime,
    process_provider: SystemProcessProvider,
    panes: Rc<RefCell<Vec<PaneSnapshot>>>,
    prev_pane_ids: HashSet<String>,
    prev_active: Option<String>,
    last_good: HashMap<PathBuf, String>,
    load_errors: Vec<String>,
    runtime_errors: Vec<String>,
    config_sig: Vec<(PathBuf, SystemTime)>,
}

impl Daemon {
    fn new() -> Result<Self> {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let mut daemon = Self {
            lua: LuaRuntime::new(Rc::clone(&panes))?,
            process_provider: SystemProcessProvider,
            panes,
            prev_pane_ids: HashSet::new(),
            prev_active: None,
            last_good: HashMap::new(),
            load_errors: Vec::new(),
            runtime_errors: Vec::new(),
            config_sig: config_signature(),
        };
        daemon.reload_plugins()?;
        Ok(daemon)
    }

    fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Ping => Response::ok(Some(DAEMON_SIGNATURE.to_string())),
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
            Request::Pick | Request::Panes => match self.panes_data() {
                Ok(data) => Response::ok(Some(data)),
                Err(error) => Response::error(error),
            },
            Request::SelectPane { id } => match self.select_pane(&id) {
                Ok(()) => Response::ok(Some("selected".to_string())),
                Err(error) => Response::error(error),
            },
            Request::Command { name, args } => match self.lua.run_command(&name, &args) {
                Ok(data) => Response::ok(data),
                Err(error) => {
                    self.record_runtime_error(format!("command {name}: {error}"));
                    Response::error(error)
                }
            },
        }
    }

    fn reload_plugins(&mut self) -> Result<()> {
        let rt = LuaRuntime::new(Rc::clone(&self.panes))?;
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
                        if let Some(source) = self.last_good.get(&path) {
                            if let Err(fallback_error) = rt.load_source(&name, source) {
                                errors.push(format!("{name}: last-good failed: {fallback_error}"));
                            }
                        }
                    }
                },
                Err(error) => {
                    errors.push(format!("{name}: {error}"));
                    if let Some(source) = self.last_good.get(&path) {
                        if let Err(fallback_error) = rt.load_source(&name, source) {
                            errors.push(format!("{name}: last-good failed: {fallback_error}"));
                        }
                    }
                }
            }
        }

        if let Err(error) = rt.load_builtins() {
            self.load_errors = vec![format!("builtin-kinds.lua: {error}")];
            return Err(error);
        }

        self.lua = rt;
        self.load_errors = errors;
        self.runtime_errors.clear();
        self.config_sig = config_signature();
        Ok(())
    }

    fn config_changed(&self) -> Option<Vec<(PathBuf, SystemTime)>> {
        let sig = config_signature();
        (sig != self.config_sig).then_some(sig)
    }

    fn scan(&mut self) -> Result<usize> {
        let panes = tmux::list_panes()?;
        let count = panes.len();
        let mut snapshots = Vec::new();

        for pane in panes {
            let proc_tree = self
                .process_provider
                .proc_tree(pane.pid)
                .unwrap_or_default();
            if let Some(detection) = self.lua.detect(&pane, proc_tree)? {
                tmux::set_pane_var(&pane.id, "@castr_kind", &detection.kind)?;
                tmux::set_pane_var(&pane.id, "@castr_label", &detection.label)?;
                snapshots.push(PaneSnapshot {
                    id: pane.id.clone(),
                    pid: pane.pid,
                    kind: detection.kind,
                    label: detection.label,
                    cwd: pane.cwd.clone(),
                    session: pane.session.clone(),
                    window: pane.window.clone(),
                    active: pane.active,
                    zoomed: pane.zoomed,
                });
            }
        }

        self.update_events(&snapshots);
        *self.panes.borrow_mut() = snapshots;
        Ok(count)
    }

    fn update_events(&mut self, snapshots: &[PaneSnapshot]) {
        let current_ids = snapshots
            .iter()
            .map(|pane| pane.id.clone())
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
        if active != self.prev_active {
            if let Some(active_id) = &active {
                if let Some(pane) = snapshots.iter().find(|pane| &pane.id == active_id) {
                    self.record_runtime_errors(self.lua.fire_event("pane:focus", Some(pane)));
                }
            }
        }

        self.record_runtime_errors(self.lua.fire_event("tick", None));
        self.prev_pane_ids = current_ids;
        self.prev_active = active;
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

    fn panes_data(&mut self) -> Result<String> {
        self.scan()?;
        Ok(serde_json::to_string(&*self.panes.borrow())?)
    }

    fn select_pane(&mut self, id: &str) -> Result<()> {
        self.scan()?;
        if self.panes.borrow().iter().any(|pane| pane.id == id) {
            tmux::select_pane(id)
        } else {
            anyhow::bail!("unknown pane {id}")
        }
    }
}

fn config_signature() -> Vec<(PathBuf, SystemTime)> {
    user_plugin_files()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, active: bool) -> PaneSnapshot {
        PaneSnapshot {
            id: id.to_string(),
            pid: 123,
            kind: "term".to_string(),
            label: "term".to_string(),
            cwd: "/tmp".to_string(),
            session: "s".to_string(),
            window: "1:w".to_string(),
            active,
            zoomed: false,
        }
    }

    fn test_daemon(lua_source: &str) -> Daemon {
        let panes = Rc::new(RefCell::new(Vec::new()));
        let lua = LuaRuntime::new(Rc::clone(&panes)).unwrap();
        lua.load_source("test.lua", lua_source).unwrap();
        Daemon {
            lua,
            process_provider: SystemProcessProvider,
            panes,
            prev_pane_ids: HashSet::new(),
            prev_active: None,
            last_good: HashMap::new(),
            load_errors: Vec::new(),
            runtime_errors: Vec::new(),
            config_sig: Vec::new(),
        }
    }

    #[test]
    fn update_events_fires_new_focus_and_tick_without_tmux() {
        let mut daemon = test_daemon(
            r#"
            counts = { new = 0, focus = 0, tick = 0 }
            focused = ""
            castr.on("pane:new", function(_) counts.new = counts.new + 1 end)
            castr.on("pane:focus", function(p) counts.focus = counts.focus + 1; focused = p.id end)
            castr.on("tick", function() counts.tick = counts.tick + 1 end)
            castr.register_command{
              name = "counts",
              handler = function()
                return counts.new .. ":" .. counts.focus .. ":" .. counts.tick .. ":" .. focused
              end,
            }
            "#,
        );

        daemon.update_events(&[pane("%1", true)]);
        let first = daemon.lua.run_command("counts", &[]).unwrap();
        assert_eq!(first.as_deref(), Some("1:1:1:%1"));

        daemon.update_events(&[pane("%1", true)]);
        let second = daemon.lua.run_command("counts", &[]).unwrap();
        assert_eq!(second.as_deref(), Some("1:1:2:%1"));

        daemon.update_events(&[pane("%1", false), pane("%2", true)]);
        let third = daemon.lua.run_command("counts", &[]).unwrap();
        assert_eq!(third.as_deref(), Some("2:2:3:%2"));
    }

    #[test]
    fn event_errors_are_collected_without_crashing() {
        let mut daemon = test_daemon(
            r#"
            castr.on("tick", function() error("tick failed") end)
            "#,
        );

        daemon.update_events(&[]);
        assert_eq!(daemon.runtime_errors.len(), 1);
        assert!(daemon.runtime_errors[0].contains("tick failed"));
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
