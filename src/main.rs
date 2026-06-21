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
use crossterm::{
    cursor,
    event::{self, Event, KeyCode},
    execute,
    terminal::{self, ClearType},
};
use protocol::{PaneSnapshot, PanelView, Request, Response};

#[derive(Debug, Parser)]
#[command(name = "castr")]
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

    /// Set pane state.
    SetState { id: String, state: String },

    /// Check hidden session consistency.
    Doctor {
        #[arg(long)]
        clean: bool,
    },

    /// Show or act on live castr pane state.
    Control {
        #[arg(long)]
        once: bool,
        action: Option<String>,
        id: Option<String>,
    },

    /// Placeholder for a future detailed control view.
    Pick,

    #[command(external_subcommand)]
    External(Vec<String>),
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
        Some(Commands::SetState { id, state }) => {
            let response = request(Request::SetState { id, state })?;
            print_response(response)
        }
        Some(Commands::Doctor { clean }) => {
            let response = request(Request::Doctor { clean })?;
            print_response(response)
        }
        Some(Commands::Control { once, action, id }) => control(once, action, id),
        Some(Commands::Pick) => {
            let response = request(Request::Pick)?;
            print_response(response)
        }
        Some(Commands::External(parts)) => run_external(parts),
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
        match reload_at(&socket) {
            Ok(()) => return Ok(()),
            Err(_) => {
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
        if reload_at(&socket).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }

    bail!("castr daemon did not become ready at {}", socket.display())
}

fn reload_at(socket: &PathBuf) -> Result<()> {
    let response = request_at(socket, Request::Reload)?;
    if response.ok {
        Ok(())
    } else {
        bail!(
            response
                .error
                .unwrap_or_else(|| "castr reload failed".to_string())
        )
    }
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
        "castr control  q:quit  j/k:move  /:filter  enter:open  x:expand",
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
    println!("castr");
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

fn run_external(parts: Vec<String>) -> Result<()> {
    let Some((name, args)) = parts.split_first() else {
        bail!("missing command name");
    };
    let response = request(Request::Command {
        name: name.clone(),
        args: args.to_vec(),
    })?;
    print_response(response)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_subcommand_keeps_priority_over_external() {
        let cli = Cli::try_parse_from(["castr", "status"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Status)));
    }

    #[test]
    fn unknown_subcommand_is_forwarded_as_external_command() {
        let cli = Cli::try_parse_from(["castr", "hello", "a", "b"]).unwrap();
        match cli.command {
            Some(Commands::External(parts)) => assert_eq!(parts, ["hello", "a", "b"]),
            other => panic!("expected external command, got wrong variant: {other:?}"),
        }
    }

    #[test]
    fn control_is_a_builtin_command() {
        let cli = Cli::try_parse_from(["castr", "control"]).unwrap();
        assert!(matches!(cli.command, Some(Commands::Control { .. })));
    }

    #[test]
    fn control_actions_parse_as_builtin_command() {
        let cli = Cli::try_parse_from(["castr", "control", "expand", "%1"]).unwrap();
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
    fn state_markers_match_rendered_states() {
        assert_eq!(state_marker(Some("blocked")), "🔴");
        assert_eq!(state_marker(Some("working")), "🟡");
        assert_eq!(state_marker(Some("done_unseen")), "🔵");
        assert_eq!(state_marker(Some("idle_seen")), "🟢");
        assert_eq!(state_marker(None), " ");
    }
}
