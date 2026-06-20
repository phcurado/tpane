use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct ProcessInfo {
    pub pid: i32,
    pub ppid: i32,
    pub argv: String,
}

pub trait ProcessProvider {
    fn proc_tree(&self, root_pid: i32) -> Result<Vec<ProcessInfo>>;
}

pub struct SystemProcessProvider;

impl ProcessProvider for SystemProcessProvider {
    fn proc_tree(&self, root_pid: i32) -> Result<Vec<ProcessInfo>> {
        proc_tree(root_pid)
    }
}

#[cfg(target_os = "linux")]
fn proc_tree(root_pid: i32) -> Result<Vec<ProcessInfo>> {
    let processes = read_linux_processes()?;
    let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
    for process in processes.values() {
        children.entry(process.ppid).or_default().push(process.pid);
    }

    let mut tree = Vec::new();
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if let Some(process) = processes.get(&pid) {
            tree.push(process.clone());
            if let Some(child_pids) = children.get(&pid) {
                stack.extend(child_pids.iter().copied());
            }
        }
    }

    Ok(tree)
}

#[cfg(target_os = "linux")]
fn read_linux_processes() -> Result<HashMap<i32, ProcessInfo>> {
    let mut processes = HashMap::new();
    for entry in fs::read_dir("/proc").context("failed to read /proc")? {
        let entry = entry?;
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse().ok())
        else {
            continue;
        };

        let stat_path = entry.path().join("stat");
        let cmdline_path = entry.path().join("cmdline");
        let Ok(stat) = fs::read_to_string(stat_path) else {
            continue;
        };
        let Some(ppid) = parse_ppid(&stat) else {
            continue;
        };

        let argv = fs::read(cmdline_path)
            .ok()
            .map(|bytes| decode_cmdline(&bytes))
            .filter(|argv| !argv.is_empty())
            .unwrap_or_else(|| comm_from_stat(&stat).unwrap_or_default());

        processes.insert(pid, ProcessInfo { pid, ppid, argv });
    }
    Ok(processes)
}

#[cfg(target_os = "linux")]
fn parse_ppid(stat: &str) -> Option<i32> {
    let end = stat.rfind(") ")?;
    let after = &stat[end + 2..];
    let mut parts = after.split_whitespace();
    parts.next()?; // state
    parts.next()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn comm_from_stat(stat: &str) -> Option<String> {
    let start = stat.find('(')? + 1;
    let end = stat.rfind(')')?;
    Some(stat[start..end].to_string())
}

#[cfg(target_os = "linux")]
fn decode_cmdline(bytes: &[u8]) -> String {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(not(target_os = "linux"))]
fn proc_tree(_root_pid: i32) -> Result<Vec<ProcessInfo>> {
    anyhow::bail!("ProcessProvider is only implemented for Linux in this slice")
}
