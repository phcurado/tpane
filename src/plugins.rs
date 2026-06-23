use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

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
    let path = Path::new(path);
    if path.is_absolute()
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
    let status = Command::new("git")
        .args(clone_args(url, &dest, &spec))
        .status()
        .with_context(|| format!("failed to run git clone for {url}"))?;
    if !status.success() {
        bail!("git clone failed for {url}");
    }
    if let Some(tag) = &spec.tag {
        checkout(&dest, tag)?;
    }
    if let Some(rev) = &spec.rev {
        checkout(&dest, rev)?;
    }
    write_metadata(&name, &spec)?;
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
    fs::remove_dir_all(&dir).with_context(|| format!("failed to remove {}", dir.display()))
}

pub fn clean(keep: &std::collections::HashSet<String>) -> Result<Vec<String>> {
    let mut removed = Vec::new();
    for name in list()? {
        if !keep.contains(&name) {
            remove(&name)?;
            removed.push(name);
        }
    }
    Ok(removed)
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

fn update_one(name: &str) -> Result<()> {
    let spec = read_metadata(name)?;
    let dir = plugin_dir(name);
    if !dir.exists() {
        bail!("unknown plugin: {name}");
    }
    if let Some(branch) = &spec.branch {
        git(&dir, ["fetch", "origin", branch])?;
        git(&dir, ["checkout", branch])?;
        git(&dir, ["merge", "--ff-only", &format!("origin/{branch}")])?;
    } else if let Some(tag) = &spec.tag {
        git(&dir, ["fetch", "--tags", "origin"])?;
        git(&dir, ["checkout", tag])?;
    } else if let Some(rev) = &spec.rev {
        git(&dir, ["fetch", "origin"])?;
        git(&dir, ["checkout", "--detach", rev])?;
    } else {
        git(&dir, ["pull", "--ff-only"])?;
    }
    Ok(())
}

fn checkout(dir: &Path, rev: &str) -> Result<()> {
    let status = Command::new("git")
        .current_dir(dir)
        .args(checkout_args(rev))
        .status()
        .with_context(|| format!("failed to run git checkout for {rev}"))?;
    if !status.success() {
        bail!("git checkout failed for {rev}");
    }
    Ok(())
}

fn git<const N: usize>(dir: &Path, args: [&str; N]) -> Result<()> {
    let status = Command::new("git")
        .current_dir(dir)
        .args(args)
        .status()
        .with_context(|| format!("failed to run git in {}", dir.display()))?;
    if !status.success() {
        bail!("git command failed in {}", dir.display());
    }
    Ok(())
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
    if requested.branch.is_some() && requested.branch != installed.branch {
        bail!("plugin {name} is not installed with requested branch");
    }
    if requested.tag.is_some() && requested.tag != installed.tag {
        bail!("plugin {name} is not installed with requested tag");
    }
    if requested.rev.is_some() && requested.rev != installed.rev {
        bail!("plugin {name} is not installed with requested rev");
    }
    if requested.path.is_some() && requested.path != installed.path {
        bail!("plugin {name} is not installed with requested path");
    }
    Ok(())
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
    fn infer_name_from_common_git_urls() {
        assert_eq!(infer_name("https://github.com/me/foo.git"), "foo");
        assert_eq!(infer_name("git@github.com:me/bar.git"), "bar");
        assert_eq!(infer_name("https://github.com/me/baz/"), "baz");
    }
}
