use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginSpec {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLock {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub commit: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Lockfile {
    plugins: BTreeMap<String, PluginLock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStatus {
    pub name: String,
    pub referenced: bool,
    pub installed: bool,
    pub url: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub rev: Option<String>,
    pub path: Option<String>,
    pub current: Option<String>,
    pub locked: Option<String>,
    pub dirty: Option<bool>,
    pub update_available: Option<bool>,
}

pub fn plugin_root() -> PathBuf {
    data_dir().join("plugins")
}

fn data_dir() -> PathBuf {
    std::env::var_os("TPANE_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_DATA_HOME").map(|home| PathBuf::from(home).join("tpane")))
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share/tpane"))
        })
        .unwrap_or_else(|| PathBuf::from(".local/share/tpane"))
}

fn config_dir() -> PathBuf {
    std::env::var_os("TPANE_CONFIG_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME").map(|home| PathBuf::from(home).join("tpane"))
        })
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/tpane")))
        .unwrap_or_else(|| PathBuf::from(".config/tpane"))
}

pub fn lockfile_path() -> PathBuf {
    config_dir().join("tpane-lock.json")
}

pub fn plugin_dir(name: &str) -> PathBuf {
    plugin_root().join(name)
}

fn metadata_path(name: &str) -> PathBuf {
    plugin_dir(name).join(".tpane-plugin.json")
}

fn clone_args(url: &str, dest: &Path, spec: &PluginSpec) -> Vec<String> {
    let mut args = vec!["clone".to_string()];
    if let Some(branch) = &spec.branch {
        args.extend([
            "--branch".to_string(),
            branch.clone(),
            "--single-branch".to_string(),
        ]);
    }
    if spec.path.is_some() {
        args.extend(["--filter=blob:none".to_string(), "--sparse".to_string()]);
    }
    args.extend([
        "--".to_string(),
        url.to_string(),
        dest.display().to_string(),
    ]);
    args
}

fn checkout_args(rev: &str) -> Vec<String> {
    vec![
        "checkout".to_string(),
        "--detach".to_string(),
        rev.to_string(),
    ]
}

pub fn infer_name(url: &str) -> String {
    url.trim_end_matches('/')
        .rsplit(['/', ':'])
        .next()
        .unwrap_or(url)
        .trim_end_matches(".git")
        .to_string()
}

pub fn validate_plugin_name(name: &str) -> Result<()> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\\') {
        bail!("invalid plugin name: {name}");
    }
    Ok(())
}

fn validate_ref(name: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.starts_with('-') {
        bail!("invalid plugin {name}: {value}");
    }
    Ok(())
}

pub fn validate_spec(spec: &PluginSpec) -> Result<()> {
    if let Some(url) = &spec.url
        && url.starts_with('-')
    {
        bail!("invalid plugin url: {url}");
    }
    let refs = [
        spec.branch.is_some(),
        spec.tag.is_some(),
        spec.rev.is_some(),
    ]
    .into_iter()
    .filter(|set| *set)
    .count();
    if refs > 1 {
        bail!("plugin spec can set only one of branch, tag, or rev");
    }
    if let Some(branch) = &spec.branch {
        validate_ref("branch", branch)?;
    }
    if let Some(tag) = &spec.tag {
        validate_ref("tag", tag)?;
    }
    if let Some(rev) = &spec.rev {
        validate_ref("rev", rev)?;
    }
    if let Some(path) = &spec.path {
        validate_plugin_path(path)?;
    }
    Ok(())
}

fn validate_plugin_path(path: &str) -> Result<()> {
    let raw = path;
    let path = Path::new(path);
    if raw.is_empty()
        || raw.starts_with('-')
        || path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("invalid plugin path: {}", path.display());
    }
    Ok(())
}

pub fn ensure(name: &str, spec: &PluginSpec) -> Result<PathBuf> {
    validate_plugin_name(name)?;
    validate_spec(spec)?;
    let dir = plugin_dir(name);
    if dir.exists() {
        assert_compatible(name, spec)?;
        return Ok(dir);
    }
    let Some(url) = spec.url.as_deref() else {
        bail!("plugin {name} is not installed");
    };
    add(url, Some(name), spec.clone())
}

pub fn add(url: &str, name: Option<&str>, mut spec: PluginSpec) -> Result<PathBuf> {
    if url.starts_with('-') {
        bail!("invalid plugin url: {url}");
    }
    if let Some(existing_url) = &spec.url
        && existing_url != url
    {
        bail!("plugin url does not match spec url");
    }
    spec.url = Some(url.to_string());
    validate_spec(&spec)?;
    let name = name.map(str::to_string).unwrap_or_else(|| infer_name(url));
    validate_plugin_name(&name)?;
    let dest = plugin_dir(&name);
    if dest.exists() {
        bail!("plugin already exists: {name}");
    }
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    git_status(
        Command::new("git").args(clone_args(url, &dest, &spec)),
        &format!("git clone failed for {url}"),
    )?;

    if let Some(commit) = locked_commit(&name, &spec)? {
        checkout(&dest, &commit)?;
    } else if let Some(tag) = &spec.tag {
        checkout(&dest, tag)?;
    } else if let Some(rev) = &spec.rev {
        checkout(&dest, rev)?;
    }
    configure_sparse_checkout(&dest, &spec)?;

    write_metadata(&name, &spec)?;
    write_lock_entry(&name, &spec, &current_commit(&dest)?)?;
    Ok(dest)
}

pub fn list() -> Result<Vec<String>> {
    let root = plugin_root();
    let Ok(entries) = fs::read_dir(&root) else {
        return Ok(Vec::new());
    };
    let mut names = entries
        .filter_map(std::result::Result::ok)
        .filter_map(|entry| {
            entry
                .file_type()
                .ok()
                .filter(|file_type| file_type.is_dir())
                .and_then(|_| entry.file_name().into_string().ok())
        })
        .collect::<Vec<_>>();
    names.sort();
    Ok(names)
}

pub fn remove(name: &str) -> Result<()> {
    validate_plugin_name(name)?;
    let dir = plugin_dir(name);
    if !dir.exists() {
        bail!("unknown plugin: {name}");
    }
    fs::remove_dir_all(&dir).with_context(|| format!("failed to remove {}", dir.display()))?;
    remove_lock_entry(name)
}

pub fn clean(keep: &HashSet<String>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in list()? {
        if !keep.contains(&name) {
            remove(&name)?;
            removed.push(name);
        }
    }
    Ok(removed)
}

pub fn sync(specs: &HashMap<String, PluginSpec>) -> Result<Vec<String>> {
    let mut synced = Vec::new();
    let mut names = specs.keys().cloned().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let spec = specs.get(&name).expect("name came from specs");
        validate_plugin_name(&name)?;
        validate_spec(spec)?;
        if plugin_dir(&name).exists() {
            update_one_with_spec(&name, spec)?;
            synced.push(name);
        } else if spec.url.is_some() {
            ensure(&name, spec)?;
            synced.push(name);
        }
    }
    Ok(synced)
}

pub fn update(name: Option<&str>) -> Result<Vec<String>> {
    let names = match name {
        Some(name) => {
            validate_plugin_name(name)?;
            vec![name.to_string()]
        }
        None => list()?,
    };
    let mut updated = Vec::new();
    for name in names {
        update_one(&name)?;
        updated.push(name);
    }
    Ok(updated)
}

pub fn status(specs: &HashMap<String, PluginSpec>) -> Result<Vec<PluginStatus>> {
    let mut names = list()?.into_iter().collect::<HashSet<_>>();
    names.extend(specs.keys().cloned());
    let mut names = names.into_iter().collect::<Vec<_>>();
    names.sort();

    names
        .into_iter()
        .map(|name| {
            let referenced = specs.contains_key(&name);
            let installed = plugin_dir(&name).exists();
            let spec = specs
                .get(&name)
                .cloned()
                .or_else(|| read_metadata(&name).ok())
                .unwrap_or_default();
            let lock = read_lockfile()?.plugins.remove(&name);
            let current = installed
                .then(|| current_commit(&plugin_dir(&name)))
                .transpose()?;
            let dirty = installed.then(|| dirty(&plugin_dir(&name))).transpose()?;
            let update_available = if installed {
                update_available(&plugin_dir(&name), &spec, current.as_deref())?
            } else {
                None
            };
            Ok(PluginStatus {
                name,
                referenced,
                installed,
                url: spec.url,
                branch: spec.branch,
                tag: spec.tag,
                rev: spec.rev,
                path: spec.path,
                current,
                locked: lock.map(|lock| lock.commit),
                dirty,
                update_available,
            })
        })
        .collect()
}

fn update_one(name: &str) -> Result<()> {
    let spec = read_metadata(name)?;
    update_one_with_spec(name, &spec)
}

fn update_one_with_spec(name: &str, spec: &PluginSpec) -> Result<()> {
    let dir = plugin_dir(name);
    if !dir.exists() {
        bail!("unknown plugin: {name}");
    }
    if let Some(metadata) = read_metadata(name).ok()
        && spec.url.is_some()
        && metadata.url != spec.url
    {
        bail!("plugin {name} is installed with a different url");
    }

    if let Some(branch) = &spec.branch {
        git(&dir, &["fetch", "origin", branch])?;
        git(
            &dir,
            &["checkout", "-B", branch, &format!("origin/{branch}")],
        )?;
    } else if let Some(tag) = &spec.tag {
        git(&dir, &["fetch", "--tags", "origin"])?;
        checkout(&dir, tag)?;
    } else if let Some(rev) = &spec.rev {
        git(&dir, &["fetch", "origin"])?;
        checkout(&dir, rev)?;
    } else {
        git(&dir, &["pull", "--ff-only"])?;
    }
    configure_sparse_checkout(&dir, spec)?;

    write_metadata(name, spec)?;
    write_lock_entry(name, spec, &current_commit(&dir)?)
}

fn checkout(dir: &Path, rev: &str) -> Result<()> {
    validate_ref("rev", rev)?;
    git_status(
        Command::new("git")
            .current_dir(dir)
            .args(checkout_args(rev)),
        &format!("git checkout failed for {rev}"),
    )
}

fn configure_sparse_checkout(dir: &Path, spec: &PluginSpec) -> Result<()> {
    if let Some(path) = &spec.path {
        git(dir, &["sparse-checkout", "set", "--", path])?;
    }
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<()> {
    git_status(
        Command::new("git").current_dir(dir).args(args),
        &format!("git command failed in {}", dir.display()),
    )
}

fn git_status(command: &mut Command, failure: &str) -> Result<()> {
    command
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if std::env::var_os("GIT_SSH_COMMAND").is_none() {
        command.env("GIT_SSH_COMMAND", "ssh -o BatchMode=yes");
    }
    let mut child = command.spawn().with_context(|| failure.to_string())?;
    let deadline = Instant::now() + GIT_TIMEOUT;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            bail!(failure.to_string());
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!(
                "{failure}: timed out after {} seconds",
                GIT_TIMEOUT.as_secs()
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn git_output(dir: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("failed to run git in {}", dir.display()))?;
    if !output.status.success() {
        bail!("git command failed in {}", dir.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn current_commit(dir: &Path) -> Result<String> {
    git_output(dir, &["rev-parse", "HEAD"])
}

fn dirty(dir: &Path) -> Result<bool> {
    Ok(!git_output(dir, &["status", "--porcelain"])?.is_empty())
}

fn update_available(dir: &Path, spec: &PluginSpec, current: Option<&str>) -> Result<Option<bool>> {
    let Some(branch) = &spec.branch else {
        return Ok(None);
    };
    let remote = git_output(dir, &["rev-parse", "--verify", &format!("origin/{branch}")])?;
    Ok(current.map(|current| current != remote))
}

pub fn entrypoint(name: &str, spec: &PluginSpec) -> Result<PathBuf> {
    validate_plugin_name(name)?;
    validate_spec(spec)?;
    let installed = plugin_dir(name);
    let base = if let Some(path) = &spec.path {
        installed.join(path)
    } else if metadata_path(name).exists() {
        match read_metadata(name)?.path {
            Some(path) => installed.join(path),
            None => installed,
        }
    } else {
        installed
    };
    Ok(base.join("init.lua"))
}

pub fn assert_compatible(name: &str, requested: &PluginSpec) -> Result<()> {
    if requested.url.is_none()
        && requested.branch.is_none()
        && requested.tag.is_none()
        && requested.rev.is_none()
        && requested.path.is_none()
    {
        return Ok(());
    }
    let Ok(installed) = read_metadata(name) else {
        bail!("plugin {name} is not installed with requested ref");
    };
    if requested.url.is_some() && requested.url != installed.url {
        bail!("plugin {name} is not installed with requested url");
    }
    Ok(())
}

fn locked_commit(name: &str, spec: &PluginSpec) -> Result<Option<String>> {
    let Some(lock) = read_lockfile()?.plugins.remove(name) else {
        return Ok(None);
    };
    if lock_matches_spec(&lock, spec) {
        validate_ref("locked commit", &lock.commit)?;
        Ok(Some(lock.commit))
    } else {
        Ok(None)
    }
}

fn lock_matches_spec(lock: &PluginLock, spec: &PluginSpec) -> bool {
    (spec.url.is_none() || spec.url == lock.url)
        && (spec.branch.is_none() || spec.branch == lock.branch)
        && (spec.tag.is_none() || spec.tag == lock.tag)
        && (spec.rev.is_none() || spec.rev == lock.rev)
        && (spec.path.is_none() || spec.path == lock.path)
}

fn read_metadata(name: &str) -> Result<PluginSpec> {
    let path = metadata_path(name);
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_metadata(name: &str, spec: &PluginSpec) -> Result<()> {
    let path = metadata_path(name);
    let source = serde_json::to_string_pretty(spec)?;
    fs::write(&path, source).with_context(|| format!("failed to write {}", path.display()))
}

fn read_lockfile() -> Result<Lockfile> {
    let path = lockfile_path();
    if !path.exists() {
        return Ok(Lockfile::default());
    }
    let source =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    serde_json::from_str(&source).with_context(|| format!("failed to parse {}", path.display()))
}

fn write_lockfile(lockfile: &Lockfile) -> Result<()> {
    let path = lockfile_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let source = serde_json::to_string_pretty(lockfile)?;
    fs::write(&path, format!("{source}\n"))
        .with_context(|| format!("failed to write {}", path.display()))
}

fn write_lock_entry(name: &str, spec: &PluginSpec, commit: &str) -> Result<()> {
    let mut lockfile = read_lockfile()?;
    lockfile.plugins.insert(
        name.to_string(),
        PluginLock {
            url: spec.url.clone(),
            branch: spec.branch.clone(),
            tag: spec.tag.clone(),
            rev: spec.rev.clone(),
            path: spec.path.clone(),
            commit: commit.to_string(),
        },
    );
    write_lockfile(&lockfile)
}

fn remove_lock_entry(name: &str) -> Result<()> {
    let mut lockfile = read_lockfile()?;
    if lockfile.plugins.remove(name).is_some() {
        write_lockfile(&lockfile)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_paths_live_under_data_plugins() {
        let path = plugin_dir("foo");
        assert!(path.ends_with("tpane/plugins/foo"));
    }

    #[test]
    fn clone_args_match_git_clone_shape() {
        assert_eq!(
            clone_args(
                "https://example.test/me/foo.git",
                Path::new("/tmp/foo"),
                &PluginSpec::default()
            ),
            vec!["clone", "--", "https://example.test/me/foo.git", "/tmp/foo"]
        );
    }

    #[test]
    fn clone_args_include_tag_only_as_metadata() {
        assert_eq!(
            clone_args(
                "https://example.test/me/foo.git",
                Path::new("/tmp/foo"),
                &PluginSpec {
                    tag: Some("v1.0.0".to_string()),
                    ..PluginSpec::default()
                },
            ),
            vec!["clone", "--", "https://example.test/me/foo.git", "/tmp/foo"]
        );
    }

    #[test]
    fn clone_args_include_branch_before_separator() {
        assert_eq!(
            clone_args(
                "https://example.test/me/foo.git",
                Path::new("/tmp/foo"),
                &PluginSpec {
                    branch: Some("main".to_string()),
                    ..PluginSpec::default()
                },
            ),
            vec![
                "clone",
                "--branch",
                "main",
                "--single-branch",
                "--",
                "https://example.test/me/foo.git",
                "/tmp/foo"
            ]
        );
    }

    #[test]
    fn clone_args_use_sparse_clone_for_plugin_paths() {
        assert_eq!(
            clone_args(
                "https://example.test/me/foo.git",
                Path::new("/tmp/foo"),
                &PluginSpec {
                    path: Some("plugins/foo".to_string()),
                    ..PluginSpec::default()
                },
            ),
            vec![
                "clone",
                "--filter=blob:none",
                "--sparse",
                "--",
                "https://example.test/me/foo.git",
                "/tmp/foo"
            ]
        );
    }

    #[test]
    fn plugin_names_reject_paths() {
        assert!(validate_plugin_name("foo").is_ok());
        assert!(validate_plugin_name("").is_err());
        assert!(validate_plugin_name(".").is_err());
        assert!(validate_plugin_name("..").is_err());
        assert!(validate_plugin_name("../foo").is_err());
        assert!(validate_plugin_name("foo\\bar").is_err());
    }

    #[test]
    fn plugin_specs_reject_multiple_refs_and_unsafe_paths() {
        assert!(
            validate_spec(&PluginSpec {
                branch: Some("main".to_string()),
                tag: Some("v1".to_string()),
                ..PluginSpec::default()
            })
            .is_err()
        );
        assert!(
            validate_spec(&PluginSpec {
                path: Some("../foo".to_string()),
                ..PluginSpec::default()
            })
            .is_err()
        );
    }

    #[test]
    fn lock_matches_requested_spec_fields() {
        let lock = PluginLock {
            url: Some("https://example.test/foo.git".to_string()),
            branch: Some("main".to_string()),
            path: Some("plugins/foo".to_string()),
            commit: "abc123".to_string(),
            ..PluginLock::default()
        };
        assert!(lock_matches_spec(
            &lock,
            &PluginSpec {
                url: Some("https://example.test/foo.git".to_string()),
                branch: Some("main".to_string()),
                path: Some("plugins/foo".to_string()),
                ..PluginSpec::default()
            },
        ));
        assert!(!lock_matches_spec(
            &lock,
            &PluginSpec {
                branch: Some("dev".to_string()),
                ..PluginSpec::default()
            },
        ));
    }

    #[test]
    fn infer_name_from_common_git_urls() {
        assert_eq!(infer_name("https://github.com/me/foo.git"), "foo");
        assert_eq!(infer_name("git@github.com:me/bar.git"), "bar");
        assert_eq!(infer_name("https://github.com/me/baz/"), "baz");
    }
}
