mod daemon;
mod lua_runtime;
mod process;
mod protocol;
mod tmux;

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use protocol::{DAEMON_SIGNATURE, Request, Response};

#[derive(Parser)]
#[command(name = "castr")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Internal daemon entrypoint.
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        socket: PathBuf,
    },

    /// Force one daemon scan now.
    Refresh,

    /// Check daemon health.
    Ping,

    /// Placeholder for a future detailed control view.
    Pick,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Daemon { socket }) => daemon::run(socket),
        Some(Commands::Refresh) => {
            let response = request(Request::Refresh)?;
            print_response(response)
        }
        Some(Commands::Ping) => {
            let response = request(Request::Ping)?;
            print_response(response)
        }
        Some(Commands::Pick) => {
            let response = request(Request::Pick)?;
            print_response(response)
        }
        None => launch(),
    }
}

fn launch() -> Result<()> {
    if env::var_os("TMUX").is_some() {
        ensure_daemon()?;
        tmux::install_render_options()?;
        return Ok(());
    }

    tmux::start_server()?;
    ensure_daemon()?;
    tmux::install_render_options()?;

    if tmux::has_session("castr") {
        tmux::attach_session("castr")
    } else {
        tmux::new_session("castr")
    }
}

fn ensure_daemon() -> Result<()> {
    let socket = socket_path()?;
    if socket.exists() {
        match request_at(&socket, Request::Ping) {
            Ok(response) if daemon_matches(&response) => return Ok(()),
            _ => {
                fs::remove_file(&socket).with_context(|| {
                    format!("failed to remove stale socket {}", socket.display())
                })?;
            }
        }
    }

    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = env::current_exe().context("failed to resolve current executable")?;
    Command::new(exe)
        .arg("daemon")
        .arg("--socket")
        .arg(&socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn castr daemon")?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let Ok(response) = request_at(&socket, Request::Ping) {
            if daemon_matches(&response) {
                return Ok(());
            }
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("castr daemon did not become ready at {}", socket.display())
}

fn daemon_matches(response: &Response) -> bool {
    response.ok && response.data.as_deref() == Some(DAEMON_SIGNATURE)
}

fn request(request: Request) -> Result<Response> {
    let socket = socket_path()?;
    request_at(&socket, request)
}

fn request_at(socket: &PathBuf, request: Request) -> Result<Response> {
    let mut stream = UnixStream::connect(socket)
        .with_context(|| format!("failed to connect to {}", socket.display()))?;
    serde_json::to_writer(&mut stream, &request)?;
    stream.write_all(b"\n")?;

    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let response = serde_json::from_str(&line)?;
    Ok(response)
}

fn print_response(response: Response) -> Result<()> {
    if response.ok {
        if let Some(data) = response.data {
            println!("{data}");
        }
        Ok(())
    } else {
        bail!(
            response
                .error
                .unwrap_or_else(|| "castr request failed".to_string())
        )
    }
}

fn socket_path() -> Result<PathBuf> {
    let key = tmux_server_key();
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("castr-{}", current_uid())));
    Ok(runtime_dir.join(format!("castr-{key}.sock")))
}

fn tmux_server_key() -> String {
    let server = env::var("TMUX")
        .ok()
        .and_then(|value| value.split(',').next().map(str::to_string))
        .unwrap_or_else(default_tmux_socket_path);
    let mut hasher = DefaultHasher::new();
    server.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn default_tmux_socket_path() -> String {
    let tmp = env::var("TMUX_TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    format!("{tmp}/tmux-{}/default", current_uid())
}

fn current_uid() -> String {
    env::var("UID")
        .ok()
        .or_else(|| {
            Command::new("id")
                .arg("-u")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        })
        .filter(|uid| !uid.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}
