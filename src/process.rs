use std::collections::HashMap;
use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessInfo {
    pub pid: i32,
    pub ppid: i32,
    pub argv: String,
}

#[derive(Debug, Clone, Default)]
pub struct ProcessTable {
    by_pid: HashMap<i32, ProcessInfo>,
    children: HashMap<i32, Vec<i32>>,
}

impl ProcessTable {
    fn from_map(by_pid: HashMap<i32, ProcessInfo>) -> Self {
        let mut children: HashMap<i32, Vec<i32>> = HashMap::new();
        for process in by_pid.values() {
            children.entry(process.ppid).or_default().push(process.pid);
        }
        Self { by_pid, children }
    }

    pub fn tree(&self, root_pid: i32) -> Vec<ProcessInfo> {
        let mut tree = Vec::new();
        let mut stack = vec![root_pid];
        while let Some(pid) = stack.pop() {
            if let Some(process) = self.by_pid.get(&pid) {
                tree.push(process.clone());
                if let Some(child_pids) = self.children.get(&pid) {
                    stack.extend(child_pids.iter().copied());
                }
            }
        }
        tree
    }
}

pub trait ProcessProvider {
    fn snapshot(&self) -> Result<ProcessTable>;
}

pub struct SystemProcessProvider;

impl ProcessProvider for SystemProcessProvider {
    fn snapshot(&self) -> Result<ProcessTable> {
        snapshot()
    }
}

#[cfg(target_os = "linux")]
fn snapshot() -> Result<ProcessTable> {
    Ok(ProcessTable::from_map(read_linux_processes()?))
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

#[cfg(target_os = "macos")]
fn snapshot() -> Result<ProcessTable> {
    let output = std::process::Command::new("ps")
        .args(["-axo", "pid=,ppid=,command="])
        .output()
        .context("failed to run ps")?;
    if !output.status.success() {
        anyhow::bail!(
            "ps failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(ProcessTable::from_map(parse_ps_processes(
        &String::from_utf8_lossy(&output.stdout),
    )))
}

#[cfg(any(target_os = "macos", test))]
fn parse_ps_processes(output: &str) -> HashMap<i32, ProcessInfo> {
    let mut processes = HashMap::new();
    for line in output.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|part| part.parse::<i32>().ok()) else {
            continue;
        };
        let Some(ppid) = parts.next().and_then(|part| part.parse::<i32>().ok()) else {
            continue;
        };
        let argv = parts.collect::<Vec<_>>().join(" ");
        processes.insert(pid, ProcessInfo { pid, ppid, argv });
    }
    processes
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn snapshot() -> Result<ProcessTable> {
    anyhow::bail!("ProcessProvider is only implemented for Linux/macOS in this slice")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ps_processes_handles_commands_with_spaces() {
        let processes = parse_ps_processes(
            "  10   1 /bin/zsh -l\n  11  10 pi --project /tmp/tpane\n garbage\n",
        );
        assert_eq!(processes.get(&10).unwrap().ppid, 1);
        assert_eq!(processes.get(&10).unwrap().argv, "/bin/zsh -l");
        assert_eq!(processes.get(&11).unwrap().ppid, 10);
        assert_eq!(processes.get(&11).unwrap().argv, "pi --project /tmp/tpane");
        assert!(!processes.contains_key(&0));
    }

    #[test]
    fn process_table_tree_walks_descendants_from_shared_snapshot() {
        let table = ProcessTable::from_map(parse_ps_processes(
            "10 1 /bin/zsh\n11 10 pi --project x\n12 11 node\n20 1 other\n",
        ));
        let mut pids = table
            .tree(10)
            .into_iter()
            .map(|process| process.pid)
            .collect::<Vec<_>>();
        pids.sort();
        assert_eq!(pids, [10, 11, 12]);
        assert!(table.tree(20).iter().all(|process| process.pid != 11));
    }
}
