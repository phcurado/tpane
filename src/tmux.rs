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

pub fn get_pane_var(pane_id: &str, name: &str) -> Result<Option<String>> {
    let value = tmux(&[
        "display-message",
        "-p",
        "-t",
        pane_id,
        &format!("#{{{name}}}"),
    ])?;
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

pub fn select_pane(pane_id: &str) -> Result<()> {
    tmux(&["select-window", "-t", pane_id])?;
    tmux(&["select-pane", "-t", pane_id]).map(|_| ())
}

pub struct SplitOptions {
    pub direction: SplitDirection,
    pub size: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub detached: bool,
}

pub enum SplitDirection {
    Horizontal,
    Vertical,
}

pub fn split(target: &str, opts: SplitOptions) -> Result<String> {
    tmux_owned(split_args(target, &opts))
}

fn split_args(target: &str, opts: &SplitOptions) -> Vec<String> {
    let mut args = vec![
        "split-window".to_string(),
        "-P".to_string(),
        "-F".to_string(),
        "#{pane_id}".to_string(),
        "-t".to_string(),
        target.to_string(),
    ];
    match opts.direction {
        SplitDirection::Horizontal => args.push("-h".to_string()),
        SplitDirection::Vertical => args.push("-v".to_string()),
    }
    if opts.detached {
        args.push("-d".to_string());
    }
    if let Some(size) = &opts.size {
        args.push("-l".to_string());
        args.push(size.clone());
    }
    if let Some(cwd) = &opts.cwd {
        args.push("-c".to_string());
        args.push(cwd.clone());
    }
    if let Some(command) = &opts.command {
        args.push(command.clone());
    }
    args
}

pub struct JoinOptions {
    pub horizontal: bool,
    pub size: Option<String>,
}

pub fn join(src_pane: &str, target: &str, opts: JoinOptions) -> Result<()> {
    tmux_owned(join_args(src_pane, target, &opts)).map(|_| ())
}

fn join_args(src_pane: &str, target: &str, opts: &JoinOptions) -> Vec<String> {
    let mut args = vec![
        "join-pane".to_string(),
        "-s".to_string(),
        src_pane.to_string(),
        "-t".to_string(),
        target.to_string(),
    ];
    args.push(if opts.horizontal { "-h" } else { "-v" }.to_string());
    if let Some(size) = &opts.size {
        args.push("-l".to_string());
        args.push(size.clone());
    }
    args
}

pub fn break_pane(pane: &str, dst_session: &str, name: &str) -> Result<()> {
    tmux_owned(break_pane_args(pane, dst_session, name)).map(|_| ())
}

fn break_pane_args(pane: &str, dst_session: &str, name: &str) -> Vec<String> {
    vec![
        "break-pane".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        pane.to_string(),
        "-t".to_string(),
        dst_session.to_string(),
        "-n".to_string(),
        name.to_string(),
    ]
}

pub fn zoom(pane: &str) -> Result<()> {
    tmux(&["resize-pane", "-Z", "-t", pane]).map(|_| ())
}

pub fn is_zoomed(target: &str) -> Result<bool> {
    Ok(tmux(&[
        "display-message",
        "-p",
        "-t",
        target,
        "#{window_zoomed_flag}",
    ])? == "1")
}

pub fn active_pane(target: &str) -> Result<String> {
    tmux(&["display-message", "-p", "-t", target, "#{pane_id}"])
}

pub fn capture(pane: &str) -> Result<String> {
    tmux(&["capture-pane", "-p", "-t", pane])
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

fn tmux_owned(args: Vec<String>) -> Result<String> {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    tmux(&refs)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_args_are_built_without_running_tmux() {
        let args = split_args(
            "%1",
            &SplitOptions {
                direction: SplitDirection::Horizontal,
                size: Some("30%".to_string()),
                cwd: Some("/tmp/work".to_string()),
                command: Some("nvim".to_string()),
                detached: true,
            },
        );

        assert_eq!(
            args,
            vec![
                "split-window",
                "-P",
                "-F",
                "#{pane_id}",
                "-t",
                "%1",
                "-h",
                "-d",
                "-l",
                "30%",
                "-c",
                "/tmp/work",
                "nvim",
            ]
        );
    }

    #[test]
    fn join_args_are_built_without_running_tmux() {
        let args = join_args(
            "%2",
            "%1",
            &JoinOptions {
                horizontal: false,
                size: Some("40".to_string()),
            },
        );

        assert_eq!(
            args,
            vec!["join-pane", "-s", "%2", "-t", "%1", "-v", "-l", "40"]
        );
    }

    #[test]
    fn break_pane_args_are_built_without_running_tmux() {
        assert_eq!(
            break_pane_args("%2", "hidden", "agent"),
            vec![
                "break-pane",
                "-d",
                "-s",
                "%2",
                "-t",
                "hidden",
                "-n",
                "agent"
            ]
        );
    }
}
