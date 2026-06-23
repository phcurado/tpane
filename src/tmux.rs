use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

#[derive(Debug, Clone)]
pub struct PaneInfo {
    pub id: String,
    pub pid: i32,
    pub cwd: String,
    pub command: String,
    pub session: String,
    pub window: String,
    pub active: bool,
    pub zoomed: bool,
    pub tag: Option<String>,
    pub home: Option<String>,
    pub state: Option<String>,
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

pub fn install_render_options() -> Result<()> {
    tmux(&["set-option", "-g", "pane-border-status", "top"])?;
    tmux(&["set-option", "-g", "pane-border-format", "#{@tpane_border}"])?;
    Ok(())
}

pub fn list_panes() -> Result<Vec<PaneInfo>> {
    let output = tmux(&[
        "list-panes",
        "-a",
        "-F",
        "#{pane_id}\t#{pane_pid}\t#{pane_current_path}\t#{pane_current_command}\t#{session_name}\t#{window_id}\t#{pane_active}\t#{window_zoomed_flag}\t#{@tpane_tag}\t#{@tpane_home}\t#{@tpane_state}",
    ])?;

    output
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_pane_info)
        .collect()
}

fn parse_pane_info(line: &str) -> Result<PaneInfo> {
    let mut parts = line.splitn(11, '\t');
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
    let command = parts.next().unwrap_or_default().to_string();
    let session = parts.next().unwrap_or_default().to_string();
    let window = parts.next().unwrap_or_default().to_string();
    let active = parts.next() == Some("1");
    let zoomed = parts.next() == Some("1");
    let tag = nonempty(parts.next().unwrap_or_default());
    let home = nonempty(parts.next().unwrap_or_default());
    let state = nonempty(parts.next().unwrap_or_default());
    Ok(PaneInfo {
        id,
        pid,
        cwd,
        command,
        session,
        window,
        active,
        zoomed,
        tag,
        home,
        state,
    })
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

pub fn bind_key(mode: &str, key: &str, command: &str, popup: bool) -> Result<()> {
    tmux_owned(bind_key_args(mode, key, command, popup)).map(|_| ())
}

pub struct NewWindowOptions {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
}

pub fn new_window(opts: NewWindowOptions) -> Result<String> {
    tmux_owned(new_window_args(&opts))
}

fn new_window_args(opts: &NewWindowOptions) -> Vec<String> {
    let mut args = vec![
        "new-window".to_string(),
        "-P".to_string(),
        "-F".to_string(),
        "#{window_id}".to_string(),
    ];
    if let Some(name) = &opts.name {
        args.push("-n".to_string());
        args.push(name.clone());
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

pub fn select_window(target: &str) -> Result<()> {
    tmux_owned(select_window_args(target)).map(|_| ())
}

fn select_window_args(target: &str) -> Vec<String> {
    vec![
        "select-window".to_string(),
        "-t".to_string(),
        target.to_string(),
    ]
}

pub fn send_keys(target: &str, keys: &str, enter: bool) -> Result<()> {
    tmux_owned(send_keys_args(target, keys, enter)).map(|_| ())
}

fn send_keys_args(target: &str, keys: &str, enter: bool) -> Vec<String> {
    let mut args = vec![
        "send-keys".to_string(),
        "-t".to_string(),
        target.to_string(),
        keys.to_string(),
    ];
    if enter {
        args.push("Enter".to_string());
    }
    args
}

fn bind_key_args(mode: &str, key: &str, command: &str, popup: bool) -> Vec<String> {
    let mut args = vec!["bind-key".to_string()];
    match mode {
        "prefix" | "normal" | "n" => {}
        "root" => args.push("-n".to_string()),
        table => {
            args.push("-T".to_string());
            args.push(table.to_string());
        }
    }
    args.push(key.to_string());
    if popup {
        args.extend([
            "display-popup".to_string(),
            "-E".to_string(),
            "-w".to_string(),
            "80%".to_string(),
            "-h".to_string(),
            "80%".to_string(),
            command.to_string(),
        ]);
    } else {
        args.extend([
            "run-shell".to_string(),
            "-b".to_string(),
            command.to_string(),
        ]);
    }
    args
}

pub fn set_global_var(name: &str, value: &str) -> Result<()> {
    tmux(&["set-option", "-g", name, value]).map(|_| ())
}

pub fn unset_global_var(name: &str) -> Result<()> {
    tmux(&["set-option", "-gu", name]).map(|_| ())
}

pub fn set_status(slot: &str, value: &str) -> Result<()> {
    tmux_owned(set_status_args(slot, value)).map(|_| ())
}

fn set_status_args(slot: &str, value: &str) -> Vec<String> {
    vec![
        "set-option".to_string(),
        "-g".to_string(),
        format!("status-{slot}"),
        value.to_string(),
    ]
}

pub fn set_status_position(position: &str) -> Result<()> {
    tmux_owned(set_status_position_args(position)).map(|_| ())
}

fn set_status_position_args(position: &str) -> Vec<String> {
    vec![
        "set-option".to_string(),
        "-g".to_string(),
        "status-position".to_string(),
        position.to_string(),
    ]
}

pub fn set_status_interval(interval: u64) -> Result<()> {
    tmux_owned(set_status_interval_args(interval)).map(|_| ())
}

fn set_status_interval_args(interval: u64) -> Vec<String> {
    vec![
        "set-option".to_string(),
        "-g".to_string(),
        "status-interval".to_string(),
        interval.to_string(),
    ]
}

pub fn set_pane_var(pane_id: &str, name: &str, value: &str) -> Result<()> {
    tmux(&["set-option", "-p", "-t", pane_id, name, value]).map(|_| ())
}

pub fn unset_pane_var(pane_id: &str, name: &str) -> Result<()> {
    tmux(&["set-option", "-u", "-p", "-t", pane_id, name]).map(|_| ())
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

pub fn kill_pane(pane_id: &str) -> Result<()> {
    tmux_owned(kill_pane_args(pane_id)).map(|_| ())
}

fn kill_pane_args(pane_id: &str) -> Vec<String> {
    vec![
        "kill-pane".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
    ]
}

pub fn set_pane_title(pane_id: &str, title: &str) -> Result<()> {
    tmux_owned(pane_title_args(pane_id, title)).map(|_| ())
}

fn pane_title_args(pane_id: &str, title: &str) -> Vec<String> {
    vec![
        "select-pane".to_string(),
        "-t".to_string(),
        pane_id.to_string(),
        "-T".to_string(),
        title.to_string(),
    ]
}

pub struct SplitOptions {
    pub direction: SplitDirection,
    pub before: bool,
    pub size: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub detached: bool,
    pub full: bool,
}

pub enum SplitDirection {
    Horizontal,
    Vertical,
}

pub fn split(target: &str, opts: SplitOptions) -> Result<String> {
    unzoom(target)?;
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
    if opts.before {
        args.push("-b".to_string());
    }
    if opts.detached {
        args.push("-d".to_string());
    }
    if opts.full {
        args.push("-f".to_string());
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
    pub full: bool,
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
    if opts.full {
        args.push("-f".to_string());
    }
    if let Some(size) = &opts.size {
        args.push("-l".to_string());
        args.push(size.clone());
    }
    args
}

pub fn break_pane(pane: &str, dst_session: &str, name: &str) -> Result<()> {
    tmux_owned(break_pane_args(pane, dst_session, name)).map(|_| ())
}

pub struct StashOptions {
    pub pane: String,
    pub window: String,
    pub cwd: String,
    pub name: String,
}

pub fn stash(opts: StashOptions) -> Result<()> {
    unzoom(&opts.window)?;
    let session = hidden_session(&opts.window);
    let created = !has_session(&session);
    if created {
        tmux_owned(new_hidden_session_args(&session, &opts.cwd))?;
    }
    tmux_owned(stash_break_args(&opts.pane, &session, &opts.name))?;
    if created {
        let _ = tmux_owned(kill_hidden_scratch_args(&session));
    }
    Ok(())
}

pub struct UnstashOptions {
    pub pane: String,
    pub target: String,
    pub horizontal: bool,
    pub size: Option<String>,
    pub full: bool,
}

pub fn unstash(opts: UnstashOptions) -> Result<()> {
    unzoom(&opts.target)?;
    join(
        &opts.pane,
        &opts.target,
        JoinOptions {
            horizontal: opts.horizontal,
            size: opts.size,
            full: opts.full,
        },
    )
}

pub fn cleanup_stash(window: &str) -> Result<()> {
    for session in [hidden_session(window), legacy_hidden_session(window)] {
        if has_session(&session) {
            kill_session(&session)?;
        }
    }
    Ok(())
}

pub fn kill_session(session: &str) -> Result<()> {
    tmux(&["kill-session", "-t", session]).map(|_| ())
}

pub fn hidden_session(window: &str) -> String {
    format!("__tpane-hidden-{window}")
}

fn legacy_hidden_session(window: &str) -> String {
    format!("__pi-hidden-{window}")
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

fn new_hidden_session_args(session: &str, cwd: &str) -> Vec<String> {
    vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        session.to_string(),
        "-n".to_string(),
        "scratch".to_string(),
        "-c".to_string(),
        cwd.to_string(),
    ]
}

fn stash_break_args(pane: &str, session: &str, name: &str) -> Vec<String> {
    vec![
        "break-pane".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        pane.to_string(),
        "-t".to_string(),
        format!("{session}:"),
        "-n".to_string(),
        name.to_string(),
    ]
}

fn kill_hidden_scratch_args(session: &str) -> Vec<String> {
    vec![
        "kill-window".to_string(),
        "-t".to_string(),
        format!("{session}:scratch"),
    ]
}

pub fn zoom(pane: &str) -> Result<()> {
    tmux(&["resize-pane", "-Z", "-t", pane]).map(|_| ())
}

pub fn unzoom(target: &str) -> Result<bool> {
    if !is_zoomed(target)? {
        return Ok(false);
    }
    let active = active_pane(target)?;
    zoom(&active)?;
    Ok(true)
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

pub fn current_pane() -> Result<String> {
    tmux(&["display-message", "-p", "#{pane_id}"])
}

pub fn current_window() -> Result<String> {
    tmux(&["display-message", "-p", "#{window_id}"])
}

pub fn active_pane(target: &str) -> Result<String> {
    tmux(&["display-message", "-p", "-t", target, "#{pane_id}"])
}

pub fn window_id(target: &str) -> Result<String> {
    tmux(&["display-message", "-p", "-t", target, "#{window_id}"])
}

pub fn display_message(target: &str, message: &str) -> Result<()> {
    tmux(&["display-message", "-t", target, message]).map(|_| ())
}

pub fn display_global_message(message: &str) -> Result<()> {
    tmux(&["display-message", message]).map(|_| ())
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
    fn parse_pane_info_reads_window_id_tag_and_home() {
        let pane =
            parse_pane_info("%1\t42\t/tmp/work\tzsh\tmain\t@7\t1\t0\tagent\t@7\tblocked").unwrap();
        assert_eq!(pane.id, "%1");
        assert_eq!(pane.pid, 42);
        assert_eq!(pane.command, "zsh");
        assert_eq!(pane.window, "@7");
        assert!(pane.active);
        assert!(!pane.zoomed);
        assert_eq!(pane.tag.as_deref(), Some("agent"));
        assert_eq!(pane.home.as_deref(), Some("@7"));
        assert_eq!(pane.state.as_deref(), Some("blocked"));
    }

    #[test]
    fn parse_pane_info_treats_empty_tag_and_home_as_none() {
        let pane = parse_pane_info("%1\t42\t/tmp/work\tzsh\tmain\t@7\t0\t1\t\t\t").unwrap();
        assert!(!pane.active);
        assert!(pane.zoomed);
        assert_eq!(pane.tag, None);
        assert_eq!(pane.home, None);
        assert_eq!(pane.state, None);
    }

    #[test]
    fn bind_key_args_are_built_without_running_tmux() {
        assert_eq!(
            bind_key_args("prefix", "A", "tpane pi expand", false),
            vec!["bind-key", "A", "run-shell", "-b", "tpane pi expand"]
        );
        assert_eq!(
            bind_key_args("root", "M-a", "tpane pi", false),
            vec!["bind-key", "-n", "M-a", "run-shell", "-b", "tpane pi"]
        );
        assert_eq!(
            bind_key_args("copy-mode-vi", "v", "tpane copy", false),
            vec![
                "bind-key",
                "-T",
                "copy-mode-vi",
                "v",
                "run-shell",
                "-b",
                "tpane copy"
            ]
        );
        assert_eq!(
            bind_key_args("prefix", "Space", "tpane control", true),
            vec![
                "bind-key",
                "Space",
                "display-popup",
                "-E",
                "-w",
                "80%",
                "-h",
                "80%",
                "tpane control"
            ]
        );
    }

    #[test]
    fn status_args_are_built_without_running_tmux() {
        assert_eq!(
            set_status_args("right", "#{session_name}"),
            vec!["set-option", "-g", "status-right", "#{session_name}"]
        );
        assert_eq!(
            set_status_position_args("top"),
            vec!["set-option", "-g", "status-position", "top"]
        );
        assert_eq!(
            set_status_interval_args(1),
            vec!["set-option", "-g", "status-interval", "1"]
        );
    }

    #[test]
    fn window_and_send_key_args_are_built_without_running_tmux() {
        assert_eq!(
            new_window_args(&NewWindowOptions {
                name: Some("logs".to_string()),
                cwd: Some("/tmp/work".to_string()),
                command: Some("zsh".to_string()),
            }),
            vec![
                "new-window",
                "-P",
                "-F",
                "#{window_id}",
                "-n",
                "logs",
                "-c",
                "/tmp/work",
                "zsh",
            ]
        );
        assert_eq!(select_window_args("@7"), vec!["select-window", "-t", "@7"]);
        assert_eq!(
            send_keys_args("%1", "npm test", true),
            vec!["send-keys", "-t", "%1", "npm test", "Enter"]
        );
        assert_eq!(
            send_keys_args("%1", "C-c", false),
            vec!["send-keys", "-t", "%1", "C-c"]
        );
    }

    #[test]
    fn split_args_are_built_without_running_tmux() {
        let args = split_args(
            "%1",
            &SplitOptions {
                direction: SplitDirection::Horizontal,
                before: false,
                size: Some("30%".to_string()),
                cwd: Some("/tmp/work".to_string()),
                command: Some("nvim".to_string()),
                detached: true,
                full: true,
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
                "-f",
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
                full: true,
            },
        );

        assert_eq!(
            args,
            vec!["join-pane", "-s", "%2", "-t", "%1", "-v", "-f", "-l", "40"]
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

    #[test]
    fn kill_pane_args_are_built_without_running_tmux() {
        assert_eq!(kill_pane_args("%9"), vec!["kill-pane", "-t", "%9"]);
    }

    #[test]
    fn pane_title_args_are_built_without_running_tmux() {
        assert_eq!(
            pane_title_args("%1", "agent"),
            vec!["select-pane", "-t", "%1", "-T", "agent"]
        );
    }

    #[test]
    fn stash_arg_builders_are_pure() {
        assert_eq!(hidden_session("@7"), "__tpane-hidden-@7");
        assert_eq!(legacy_hidden_session("@7"), "__pi-hidden-@7");
        assert_eq!(
            new_hidden_session_args("__tpane-hidden-@7", "/tmp/work"),
            vec![
                "new-session",
                "-d",
                "-s",
                "__tpane-hidden-@7",
                "-n",
                "scratch",
                "-c",
                "/tmp/work"
            ]
        );
        assert_eq!(
            stash_break_args("%2", "__tpane-hidden-@7", "agent-sidebar"),
            vec![
                "break-pane",
                "-d",
                "-s",
                "%2",
                "-t",
                "__tpane-hidden-@7:",
                "-n",
                "agent-sidebar"
            ]
        );
        assert_eq!(
            kill_hidden_scratch_args("__tpane-hidden-@7"),
            vec!["kill-window", "-t", "__tpane-hidden-@7:scratch"]
        );
    }
}
