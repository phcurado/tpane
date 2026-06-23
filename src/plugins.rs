use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::lua_runtime::config_dir;

pub fn plugin_root() -> PathBuf {
    config_dir().join("plugins")
}

pub fn plugin_dir(name: &str) -> PathBuf {
    plugin_root().join(name)
}

fn clone_args(url: &str, dest: &Path) -> Vec<String> {
    vec![
        "clone".to_string(),
        "--".to_string(),
        url.to_string(),
        dest.display().to_string(),
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

fn validate_plugin_name(name: &str) -> Result<()> {
    if name.is_empty() || name.contains('/') || name.contains('\\') {
        bail!("invalid plugin name: {name}");
    }
    Ok(())
}

pub fn add(url: &str, name: Option<&str>) -> Result<PathBuf> {
    if url.starts_with('-') {
        bail!("invalid plugin url: {url}");
    }
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
        .args(clone_args(url, &dest))
        .status()
        .with_context(|| format!("failed to run git clone for {url}"))?;
    if !status.success() {
        bail!("git clone failed for {url}");
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_paths_live_under_config_plugins() {
        let path = plugin_dir("foo");
        assert!(path.ends_with("plugins/foo"));
    }

    #[test]
    fn clone_args_match_git_clone_shape() {
        assert_eq!(
            clone_args("https://example.test/me/foo.git", Path::new("/tmp/foo")),
            vec!["clone", "--", "https://example.test/me/foo.git", "/tmp/foo"]
        );
    }

    #[test]
    fn plugin_names_reject_paths() {
        assert!(validate_plugin_name("foo").is_ok());
        assert!(validate_plugin_name("").is_err());
        assert!(validate_plugin_name("../foo").is_err());
        assert!(validate_plugin_name("foo\\bar").is_err());
    }

    #[test]
    fn infer_name_from_common_git_urls() {
        assert_eq!(infer_name("https://github.com/me/foo.git"), "foo");
        assert_eq!(infer_name("git@github.com:me/bar.git"), "bar");
        assert_eq!(infer_name("https://github.com/me/baz/"), "baz");
    }
}
