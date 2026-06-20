use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub id: String,
    pub pid: i32,
    pub cwd: String,
    pub session: String,
    pub window: String,
    pub active: bool,
    pub zoomed: bool,
}

pub fn start_server() -> Result<()> {
    tmux(&["start-server"]).map(|_| ())
}

pub fn has_session(name: &str) -> bool {
    Command::new("tmux")
        .args(["has-session", "-t", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

pub fn attach_session(name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["attach-session", "-t", name])
        .status()
        .context("failed to exec tmux attach-session")?;
    if status.success() {
        Ok(())
    } else {
        bail!("tmux attach-session failed")
    }
}

pub fn new_session(name: &str) -> Result<()> {
    let status = Command::new("tmux")
        .args(["new-session", "-s", name])
        .status()
        .context("failed to exec tmux new-session")?;
    if status.success() {
        Ok(())
    } else {
        bail!("tmux new-session failed")
    }
}

pub fn install_render_options() -> Result<()> {
    tmux(&["set-option", "-g", "pane-border-status", "top"])?;
    tmux(&[
        "set-option",
        "-g",
        "pane-border-format",
        "#{?@castr_label,#[fg=yellow]#{@castr_label}#[default],#{pane_current_command}}",
    ])?;
    Ok(())
}

pub fn list_panes() -> Result<Vec<PaneInfo>> {
    let output = tmux(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{pane_pid}\t#{pane_current_path}\t#{session_name}\t#{window_index}:#{window_name}\t#{pane_active}\t#{window_zoomed_flag}",
    ])?;

    let mut panes = Vec::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        let mut parts = line.splitn(8, '\t');
        let id = parts
            .next()
            .ok_or_else(|| anyhow!("missing pane id in tmux output"))?
            .to_string();
        let pid = parts
            .next()
            .ok_or_else(|| anyhow!("missing pane pid in tmux output"))?
            .parse::<i32>()
            .with_context(|| format!("invalid pane pid in tmux output: {line}"))?;
        let cwd = parts.next().unwrap_or_default().to_string();
        let session = parts.next().unwrap_or_default().to_string();
        let window = parts.next().unwrap_or_default().to_string();
        let active = parts.next() == Some("1");
        let zoomed = parts.next() == Some("1");
        panes.push(PaneInfo {
            id,
            pid,
            cwd,
            session,
            window,
            active,
            zoomed,
        });
    }

    Ok(panes)
}

pub fn set_pane_var(pane_id: &str, name: &str, value: &str) -> Result<()> {
    tmux(&["set-option", "-p", "-t", pane_id, name, value]).map(|_| ())
}

pub fn select_pane(pane_id: &str) -> Result<()> {
    tmux(&["select-window", "-t", pane_id])?;
    tmux(&["select-pane", "-t", pane_id]).map(|_| ())
}

pub fn server_alive() -> bool {
    Command::new("tmux")
        .args(["display-message", "-p", "ok"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn tmux(args: &[&str]) -> Result<String> {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .with_context(|| format!("failed to run tmux {}", args.join(" ")))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("tmux {} failed: {}", args.join(" "), stderr.trim())
    }
}
