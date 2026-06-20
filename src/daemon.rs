use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::lua_runtime::LuaRuntime;
use crate::process::{ProcessProvider, SystemProcessProvider};
use crate::protocol::{DAEMON_SIGNATURE, PaneSnapshot, Request, Response};
use crate::tmux;

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
    panes: Vec<PaneSnapshot>,
}

impl Daemon {
    fn new() -> Result<Self> {
        Ok(Self {
            lua: LuaRuntime::new()?,
            process_provider: SystemProcessProvider,
            panes: Vec::new(),
        })
    }

    fn handle(&mut self, request: Request) -> Response {
        match request {
            Request::Ping => Response::ok(Some(DAEMON_SIGNATURE.to_string())),
            Request::Refresh => match self.reload().and_then(|()| self.scan()) {
                Ok(count) => Response::ok(Some(format!("refreshed {count} panes"))),
                Err(error) => Response::error(error),
            },
            Request::Pick | Request::Panes => match self.panes_data() {
                Ok(data) => Response::ok(Some(data)),
                Err(error) => Response::error(error),
            },
            Request::SelectPane { id } => match self.select_pane(&id) {
                Ok(()) => Response::ok(Some("selected".to_string())),
                Err(error) => Response::error(error),
            },
        }
    }

    fn reload(&mut self) -> Result<()> {
        self.lua = LuaRuntime::new()?;
        Ok(())
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

        self.panes = snapshots;
        Ok(count)
    }

    fn panes_data(&mut self) -> Result<String> {
        self.scan()?;
        Ok(serde_json::to_string(&self.panes)?)
    }

    fn select_pane(&mut self, id: &str) -> Result<()> {
        self.scan()?;
        if self.panes.iter().any(|pane| pane.id == id) {
            tmux::select_pane(id)
        } else {
            anyhow::bail!("unknown pane {id}")
        }
    }
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
