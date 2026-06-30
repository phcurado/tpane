mod daemon;
mod exe_identity;
mod lua_runtime;
mod plugins;
mod process;
mod protocol;
mod store;
mod tmux;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet, hash_map::DefaultHasher};
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use protocol::{DaemonInfo, PaneSnapshot, PanelView, Request, Response};

#[derive(Debug, Parser)]
#[command(name = "tpane", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Internal daemon entrypoint.
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        socket: PathBuf,
    },

    /// Force plugin reload and pane scan now.
    Refresh,

    /// Reload Lua plugins now.
    Reload,

    /// Show Lua plugin load status.
    Status,

    /// Check daemon health.
    Ping,

    /// Update tpane itself.
    Update {
        #[arg(long)]
        version: Option<String>,
    },

    /// Set pane state.
    SetState { id: String, state: String },

    /// Check hidden session consistency.
    Doctor {
        #[arg(long)]
        clean: bool,
    },

    /// List built-in themes.
    Themes,

    /// Manage Lua plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },

    /// Show or act on live tpane pane state.
    Control {
        #[arg(long)]
        once: bool,
        action: Option<String>,
        id: Option<String>,
    },

    /// Run a Lua command.
    Run {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

#[derive(Debug, Subcommand)]
enum PluginCommand {
    /// List installed plugins.
    List,
    /// Show plugin install, lock, and reference status.
    Status,
    /// Install missing referenced plugins and update referenced plugins.
    Sync,
    /// Update one plugin, or all plugins when no name is given.
    Update { name: Option<String> },
    /// Remove installed plugins not referenced by config.
    Clean,
    /// Remove an installed plugin.
    Remove { name: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Commands::Daemon { socket }) => daemon::run(socket),
        Some(Commands::Refresh) => {
            let response = request(Request::Refresh)?;
            print_response(response)
        }
        Some(Commands::Reload) => {
            let response = request(Request::Reload)?;
            print_response(response)
        }
        Some(Commands::Status) => {
            let response = request(Request::Status)?;
            print_response(response)
        }
        Some(Commands::Ping) => {
            let response = request(Request::Ping)?;
            print_response(response)
        }
        Some(Commands::Update { version }) => self_update(version),
        Some(Commands::SetState { id, state }) => {
            let response = request(Request::SetState { id, state })?;
            print_response(response)
        }
        Some(Commands::Doctor { clean }) => {
            let response = request(Request::Doctor { clean })?;
            print_response(response)
        }
        Some(Commands::Themes) => themes(),
        Some(Commands::Plugin { command }) => plugin(command),
        Some(Commands::Control { once, action, id }) => control(once, action, id),
        Some(Commands::Run { name, args }) => run_lua_command(name, args),
        None => launch(),
    }
}

fn launch() -> Result<()> {
    if env::var_os("TMUX").is_none() {
        bail!("tpane must be run from tmux. Add this to tmux.conf: run-shell -b 'tpane'");
    }

    ensure_daemon()
}

fn ensure_daemon() -> Result<()> {
    let socket = socket_path()?;
    ensure_current_daemon(&socket)?;
    reload_at(&socket)
}

fn ensure_current_daemon(socket: &PathBuf) -> Result<()> {
    if socket.exists() {
        match daemon_info_at(socket) {
            Ok(info) if daemon_matches_cli(&info) => return Ok(()),
            Ok(_) => {
                let _ = request_at(socket, Request::Shutdown);
                wait_for_socket_exit(socket);
            }
            Err(_) => {}
        }
        if socket.exists() {
            fs::remove_file(socket)
                .with_context(|| format!("failed to remove stale socket {}", socket.display()))?;
        }
    }

    if let Some(parent) = socket.parent() {
        fs::create_dir_all(parent)?;
    }

    let exe = env::current_exe().context("failed to resolve current executable")?;
    Command::new(exe)
        .arg("daemon")
        .arg("--socket")
        .arg(socket)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("failed to spawn tpane daemon")?;

    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if request_at(socket, Request::Ping).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("tpane daemon did not become ready at {}", socket.display())
}

fn wait_for_socket_exit(socket: &PathBuf) {
    let deadline = Instant::now() + Duration::from_secs(1);
    while socket.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(50));
    }
}

fn daemon_info_at(socket: &PathBuf) -> Result<DaemonInfo> {
    let response = request_at(socket, Request::Info)?;
    if !response.ok {
        bail!(
            response
                .error
                .unwrap_or_else(|| "daemon info failed".to_string())
        );
    }
    Ok(serde_json::from_str(
        response.data.as_deref().unwrap_or("{}"),
    )?)
}

fn daemon_matches_cli(info: &DaemonInfo) -> bool {
    match compare_versions(env!("CARGO_PKG_VERSION"), &info.version) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => info.exe_hash == exe_identity::current_exe_hash(),
    }
}

fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };
    let a = parse(a);
    let b = parse(b);
    for idx in 0..a.len().max(b.len()) {
        match a.get(idx).unwrap_or(&0).cmp(b.get(idx).unwrap_or(&0)) {
            std::cmp::Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    std::cmp::Ordering::Equal
}

fn reload_at(socket: &PathBuf) -> Result<()> {
    let response = request_at(socket, Request::Reload)?;
    if response.ok {
        Ok(())
    } else {
        bail!(
            response
                .error
                .unwrap_or_else(|| "tpane reload failed".to_string())
        )
    }
}

fn request(request: Request) -> Result<Response> {
    let socket = socket_path()?;
    ensure_current_daemon(&socket)?;
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
                .unwrap_or_else(|| "tpane request failed".to_string())
        )
    }
}

fn referenced_plugin_specs(load_plugins: bool) -> Result<HashMap<String, plugins::PluginSpec>> {
    let panes = Rc::new(RefCell::new(Vec::new()));
    let runtime = if load_plugins {
        lua_runtime::LuaRuntime::new(panes)?
    } else {
        lua_runtime::LuaRuntime::collector(panes)?
    };
    for path in lua_runtime::user_plugin_files() {
        let source = fs::read_to_string(&path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        runtime
            .load_source(&path.display().to_string(), &source)
            .with_context(|| format!("failed to load {}", path.display()))?;
    }
    Ok(runtime.used_plugin_specs())
}

fn referenced_plugins(load_plugins: bool) -> Result<HashSet<String>> {
    Ok(referenced_plugin_specs(load_plugins)?
        .into_iter()
        .filter(|(name, spec)| spec.url.is_some() || !builtin_plugin_name(name))
        .map(|(name, _)| name)
        .collect())
}

fn plugin_status_lines(statuses: &[plugins::PluginStatus]) -> Vec<String> {
    let builtins = statuses
        .iter()
        .filter(|status| builtin_plugin_status(status))
        .map(|status| status.name.as_str())
        .collect::<Vec<_>>();
    let git_statuses = statuses
        .iter()
        .filter(|status| !builtin_plugin_status(status))
        .collect::<Vec<_>>();
    let mut lines = Vec::new();

    if git_statuses.is_empty() {
        lines.push("No git plugins installed.".to_string());
    } else {
        lines.extend(git_statuses.into_iter().map(git_plugin_status_line));
    }

    if !builtins.is_empty() {
        lines.push(format!("Built-in plugins: {}", builtins.join(", ")));
    }

    lines
}

fn builtin_plugin_status(status: &plugins::PluginStatus) -> bool {
    status.referenced && status.url.is_none() && builtin_plugin_name(&status.name)
}

fn builtin_plugin_name(name: &str) -> bool {
    matches!(
        name,
        "vim-navigator" | "yank" | "themes" | "sensible" | "pane-detection" | "open-url" | "agents"
    )
}

fn git_plugin_status_line(status: &plugins::PluginStatus) -> String {
    let installed = if status.installed {
        "installed"
    } else {
        "missing"
    };
    let referenced = if status.referenced {
        "referenced"
    } else {
        "unreferenced"
    };
    let mut parts = vec![format!("{}: {installed}, {referenced}", status.name)];
    if let Some(branch) = &status.branch {
        parts.push(format!("branch {branch}"));
    } else if let Some(tag) = &status.tag {
        parts.push(format!("tag {tag}"));
    } else if let Some(rev) = &status.rev {
        parts.push(format!("rev {}", short_commit(rev)));
    }
    if let Some(path) = &status.path {
        parts.push(format!("path {path}"));
    }
    if let Some(dirty) = status.dirty {
        parts.push(if dirty { "dirty" } else { "clean" }.to_string());
    }
    if let Some(available) = status.update_available {
        parts.push(
            if available {
                "update available"
            } else {
                "current"
            }
            .to_string(),
        );
    }
    if let Some(current) = &status.current {
        parts.push(format!("at {}", short_commit(current)));
    }
    if let Some(locked) = &status.locked
        && status.current.as_deref() != Some(locked)
    {
        parts.push(format!("locked {}", short_commit(locked)));
    }
    if let Some(url) = &status.url {
        parts.push(url.clone());
    }
    parts.join("; ")
}

fn short_commit(commit: &str) -> &str {
    commit.get(..8).unwrap_or(commit)
}

fn reload_plugins_silently() {
    if let Ok(response) = request(Request::Reload)
        && !response.ok
    {
        eprintln!(
            "reload failed: {}",
            response
                .error
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
}

fn self_update(version: Option<String>) -> Result<()> {
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg("curl -fsSL https://raw.githubusercontent.com/phcurado/tpane/main/install.sh | sh");
    if let Some(version) = version {
        command.env("VERSION", version);
    }
    let status = command.status().context("failed to run tpane updater")?;
    if !status.success() {
        bail!("tpane update failed");
    }
    Ok(())
}

fn themes() -> Result<()> {
    for name in lua_runtime::builtin_theme_names() {
        println!("{name}");
    }
    Ok(())
}

fn plugin(command: PluginCommand) -> Result<()> {
    match command {
        PluginCommand::List => {
            for name in plugins::list()? {
                println!("{name}");
            }
            Ok(())
        }
        PluginCommand::Status => {
            for line in plugin_status_lines(&plugins::status(&referenced_plugin_specs(false)?)?) {
                println!("{line}");
            }
            Ok(())
        }
        PluginCommand::Sync => {
            let synced = plugins::sync(&referenced_plugin_specs(true)?)?;
            if synced.is_empty() {
                println!("nothing to sync");
            } else {
                for name in synced {
                    println!("synced {name}");
                }
                reload_plugins_silently();
            }
            Ok(())
        }
        PluginCommand::Update { name } => {
            let updated = plugins::update(name.as_deref())?;
            if updated.is_empty() {
                println!("nothing to update");
            } else {
                for name in updated {
                    println!("updated {name}");
                }
                reload_plugins_silently();
            }
            Ok(())
        }
        PluginCommand::Clean => {
            let keep = referenced_plugins(false)?;
            let removed = plugins::clean(&keep)?;
            if removed.is_empty() {
                println!("nothing to clean");
            } else {
                for name in removed {
                    println!("removed {name}");
                }
                reload_plugins_silently();
            }
            Ok(())
        }
        PluginCommand::Remove { name } => {
            plugins::remove(&name)?;
            reload_plugins_silently();
            println!("removed {name}");
            Ok(())
        }
    }
}

fn control(once: bool, action: Option<String>, id: Option<String>) -> Result<()> {
    match (action.as_deref(), id) {
        (Some("jump"), Some(id)) => return print_response(request(Request::SelectPane { id })?),
        (Some("expand"), Some(id)) => return print_response(request(Request::ExpandPane { id })?),
        (Some(action), _) => bail!("unknown control action: {action}"),
        (None, _) if !once => return control_tui(),
        (None, _) => {}
    }

    let response = request(Request::Panels)?;
    if !response.ok {
        return print_response(response);
    }
    let panels: Vec<PanelView> = serde_json::from_str(response.data.as_deref().unwrap_or("[]"))?;
    if panels.is_empty() {
        let response = request(Request::Panes)?;
        if !response.ok {
            return print_response(response);
        }
        let panes: Vec<PaneSnapshot> =
            serde_json::from_str(response.data.as_deref().unwrap_or("[]"))?;
        let current_window = tmux::current_window().ok();
        print_control(&panes, current_window.as_deref());
    } else {
        print_panels(&panels);
    }
    Ok(())
}

#[derive(Clone)]
struct ControlRow {
    title: String,
    subtitle: String,
    state: Option<String>,
    pane: Option<String>,
    enter: Option<Vec<String>>,
    expand: Option<Vec<String>>,
    header: bool,
}

fn control_tui() -> Result<()> {
    let _guard = TerminalGuard::enter()?;
    let mut selected = 0usize;
    let mut filter = String::new();
    let mut filtering = false;

    loop {
        let rows = filtered_rows(control_rows()?, &filter);
        if selected >= rows.len() || !is_selectable(&rows, selected) {
            selected = first_selectable(&rows);
        }
        render_control_tui(&rows, selected, &filter, filtering)?;

        if event::poll(Duration::from_millis(1000))? {
            match event::read()? {
                Event::Key(key) if filtering => match key.code {
                    KeyCode::Esc => filtering = false,
                    KeyCode::Enter => filtering = false,
                    KeyCode::Backspace => {
                        filter.pop();
                    }
                    KeyCode::Char(c) => filter.push(c),
                    _ => {}
                },
                Event::Key(key) if matches!(key.code, KeyCode::Char('q') | KeyCode::Esc) => break,
                Event::Key(key) if matches!(key.code, KeyCode::Char('/')) => filtering = true,
                Event::Key(key) if matches!(key.code, KeyCode::Char('j') | KeyCode::Down) => {
                    selected = next_selectable(&rows, selected);
                }
                Event::Key(key) if matches!(key.code, KeyCode::Char('k') | KeyCode::Up) => {
                    selected = prev_selectable(&rows, selected);
                }
                Event::Key(key) if matches!(key.code, KeyCode::Enter) => {
                    if let Some(row) = rows.get(selected) {
                        run_control_row(row, false)?;
                        break;
                    }
                }
                Event::Key(key) if matches!(key.code, KeyCode::Char('x')) => {
                    if let Some(row) = rows.get(selected) {
                        run_control_row(row, true)?;
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(
            std::io::stdout(),
            terminal::EnterAlternateScreen,
            cursor::Hide
        )?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = execute!(
            std::io::stdout(),
            cursor::Show,
            terminal::LeaveAlternateScreen
        );
        let _ = terminal::disable_raw_mode();
    }
}

fn control_rows() -> Result<Vec<ControlRow>> {
    let response = request(Request::Panels)?;
    if !response.ok {
        bail!(
            response
                .error
                .unwrap_or_else(|| "control failed".to_string())
        );
    }
    let panels: Vec<PanelView> = serde_json::from_str(response.data.as_deref().unwrap_or("[]"))?;
    let mut rows = Vec::new();
    for panel in panels {
        rows.push(ControlRow {
            title: panel.title,
            subtitle: String::new(),
            state: None,
            pane: None,
            enter: None,
            expand: None,
            header: true,
        });
        for tag in ["agent", "layout", "key", ""] {
            let cards = panel
                .cards
                .iter()
                .filter(|card| card.tag.as_deref().unwrap_or("") == tag)
                .collect::<Vec<_>>();
            if cards.is_empty() {
                continue;
            }
            rows.push(ControlRow {
                title: group_title(tag).to_string(),
                subtitle: String::new(),
                state: None,
                pane: None,
                enter: None,
                expand: None,
                header: true,
            });
            for card in cards {
                rows.push(ControlRow {
                    title: card.title.clone(),
                    subtitle: card.subtitle.clone().unwrap_or_default(),
                    state: card.state.clone(),
                    pane: card.pane.clone(),
                    enter: card.enter.clone(),
                    expand: card.expand.clone(),
                    header: false,
                });
            }
        }
    }
    Ok(rows)
}

fn filtered_rows(rows: Vec<ControlRow>, filter: &str) -> Vec<ControlRow> {
    if filter.trim().is_empty() {
        return rows;
    }
    let filter = filter.to_lowercase();
    let mut out = Vec::new();
    let mut pending_headers = Vec::new();
    for row in rows {
        if row.header {
            pending_headers.push(row);
            continue;
        }
        let matched = row.title.to_lowercase().contains(&filter)
            || row.subtitle.to_lowercase().contains(&filter);
        if matched {
            out.append(&mut pending_headers);
            out.push(row);
        }
    }
    out
}

fn run_control_row(row: &ControlRow, expand: bool) -> Result<()> {
    if expand {
        if let Some(command) = &row.expand {
            return run_control_command(command);
        }
        if let Some(pane) = &row.pane {
            let response = request(Request::ExpandPane { id: pane.clone() })?;
            return print_response(response);
        }
        return Ok(());
    }

    if let Some(command) = &row.enter {
        return run_control_command(command);
    }
    if let Some(pane) = &row.pane {
        let response = request(Request::SelectPane { id: pane.clone() })?;
        return print_response(response);
    }
    Ok(())
}

fn run_control_command(command: &[String]) -> Result<()> {
    if command == ["reload"] {
        return print_response(request(Request::Reload)?);
    }
    if command == ["refresh"] {
        return print_response(request(Request::Refresh)?);
    }
    let Some((name, args)) = command.split_first() else {
        return Ok(());
    };
    let response = request(Request::Command {
        name: name.clone(),
        args: args.to_vec(),
    })?;
    print_response(response)
}

fn group_title(tag: &str) -> &'static str {
    match tag {
        "agent" => "Agents",
        "layout" => "Layout",
        "key" => "Keys",
        _ => "Other",
    }
}

fn render_control_tui(
    rows: &[ControlRow],
    selected: usize,
    filter: &str,
    filtering: bool,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    execute!(
        stdout,
        cursor::MoveTo(0, 0),
        terminal::Clear(ClearType::All)
    )?;
    write_raw_line(
        &mut stdout,
        "tpane control  q:quit  j/k:move  /:filter  enter:open  x:expand",
    )?;
    if filtering || !filter.is_empty() {
        write_raw_line(&mut stdout, &format!("filter: {filter}"))?;
    }
    for (idx, row) in rows.iter().enumerate() {
        if row.header {
            write_raw_line(&mut stdout, "")?;
            write_raw_line(&mut stdout, &row.title)?;
            continue;
        }
        let cursor = if idx == selected { ">" } else { " " };
        let marker = state_marker(row.state.as_deref());
        let action = if row.pane.is_some() || row.enter.is_some() {
            ""
        } else {
            " (info)"
        };
        write_raw_line(
            &mut stdout,
            &format!(
                "{cursor} {marker} {:<12} {}{action}",
                row.title, row.subtitle
            ),
        )?;
    }
    stdout.flush()?;
    Ok(())
}

fn write_raw_line(output: &mut impl Write, line: &str) -> Result<()> {
    output.write_all(line.as_bytes())?;
    output.write_all(b"\r\n")?;
    Ok(())
}

fn is_selectable(rows: &[ControlRow], idx: usize) -> bool {
    rows.get(idx)
        .map(|row| !row.header && (row.pane.is_some() || row.enter.is_some()))
        .unwrap_or(false)
}

fn first_selectable(rows: &[ControlRow]) -> usize {
    rows.iter()
        .position(|row| !row.header && (row.pane.is_some() || row.enter.is_some()))
        .unwrap_or(0)
}

fn next_selectable(rows: &[ControlRow], selected: usize) -> usize {
    (selected + 1..rows.len())
        .find(|idx| is_selectable(rows, *idx))
        .unwrap_or(selected)
}

fn prev_selectable(rows: &[ControlRow], selected: usize) -> usize {
    (0..selected)
        .rev()
        .find(|idx| is_selectable(rows, *idx))
        .unwrap_or(selected)
}

fn print_panels(panels: &[PanelView]) {
    for panel in panels {
        println!("{}", panel.title);
        if panel.cards.is_empty() {
            println!("  empty");
            continue;
        }

        print_panel_group(panel, "agent", "Agents");
        print_panel_group(panel, "layout", "Layout");
        print_panel_group(panel, "key", "Keys");
        print_panel_group(panel, "", "Other");
    }
}

fn print_panel_group(panel: &PanelView, tag: &str, title: &str) {
    let cards = panel
        .cards
        .iter()
        .filter(|card| card.tag.as_deref().unwrap_or("") == tag)
        .collect::<Vec<_>>();
    if cards.is_empty() {
        return;
    }

    println!("\n{title}");
    for card in cards {
        let marker = state_marker(card.state.as_deref());
        let subtitle = card.subtitle.as_deref().unwrap_or("");
        println!("  {marker} {:<12} {}", card.title, subtitle);
    }
}

fn print_control(panes: &[PaneSnapshot], current_window: Option<&str>) {
    println!("tpane");
    if panes.is_empty() {
        println!("  no panes");
        return;
    }

    let mut panes = panes.to_vec();
    panes.sort_by(|a, b| {
        let a_current = current_window == Some(a.window.as_str());
        let b_current = current_window == Some(b.window.as_str());
        b_current.cmp(&a_current).then_with(|| {
            (&a.session, &a.window, a.tag.as_deref().unwrap_or(""), &a.id).cmp(&(
                &b.session,
                &b.window,
                b.tag.as_deref().unwrap_or(""),
                &b.id,
            ))
        })
    });

    for pane in panes {
        let marker = state_marker(pane.state.as_deref());
        let tag = pane.tag.as_deref().unwrap_or(&pane.kind);
        let active = if current_window == Some(pane.window.as_str()) && pane.active {
            "*"
        } else {
            " "
        };
        println!(
            "  {marker} {active} {:<9} {:<8} {:<12} {}",
            pane.id, tag, pane.window, pane.label
        );
    }
}

fn state_marker(state: Option<&str>) -> &'static str {
    match state {
        Some("blocked") => "🔴",
        Some("working") => "🟡",
        Some("done_unseen") => "🔵",
        Some("idle_seen") => "🟢",
        _ => " ",
    }
}

fn run_lua_command(name: String, args: Vec<String>) -> Result<()> {
    let response = request(Request::Command { name, args })?;
    print_response(response)
}

fn socket_path() -> Result<PathBuf> {
    let key = tmux_server_key();
    let runtime_dir = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join(format!("tpane-{}", current_uid())));
    Ok(runtime_dir.join(format!("tpane-{key}.sock")))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_subcommand_parses_as_builtin() {
        let cli = Cli::try_parse_from(["tpane", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn lua_commands_run_under_run_subcommand() {
        let cli = Cli::try_parse_from(["tpane", "run", "hello", "a", "b"]).unwrap();
        match cli.command {
            Some(Commands::Run { name, args }) => {
                assert_eq!(name, "hello");
                assert_eq!(args, ["a", "b"]);
            }
            other => panic!("expected run command, got wrong variant: {other:?}"),
        }
    }

    #[test]
    fn unknown_subcommands_are_rejected() {
        assert!(Cli::try_parse_from(["tpane", "hello", "a", "b"]).is_err());
    }

    #[test]
    fn control_is_a_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "control"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Control { .. })));
    }

    #[test]
    fn control_actions_parse_as_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "control", "expand", "%1"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Control {
                action: Some(_),
                id: Some(_),
                ..
            })
        ));
    }

    #[test]
    fn themes_parses_as_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "themes"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Themes)));
    }

    #[test]
    fn plugin_sync_parses_as_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "plugin", "sync"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Plugin {
                command: PluginCommand::Sync
            })
        ));
    }

    #[test]
    fn plugin_status_parses_as_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "plugin", "status"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Plugin {
                command: PluginCommand::Status
            })
        ));
    }

    #[test]
    fn update_parses_as_builtin_command() {
        let cli = Cli::try_parse_from(["tpane", "update", "--version", "v1.2.3"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Commands::Update {
                version: Some(version)
            }) if version == "v1.2.3"
        ));
    }

    #[test]
    fn state_markers_match_rendered_states() {
        assert_eq!(state_marker(Some("blocked")), "🔴");
        assert_eq!(state_marker(Some("working")), "🟡");
        assert_eq!(state_marker(Some("done_unseen")), "🔵");
        assert_eq!(state_marker(Some("idle_seen")), "🟢");
        assert_eq!(state_marker(None), " ");
    }
}
