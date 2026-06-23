use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{Map, Value};

#[derive(Debug)]
pub struct Store {
    path: Option<PathBuf>,
    data: Map<String, Value>,
    dirty: bool,
}

impl Store {
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let data = fs::read_to_string(&path)
            .ok()
            .and_then(|source| serde_json::from_str::<Value>(&source).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        Self {
            path: Some(path),
            data,
            dirty: false,
        }
    }

    pub fn memory() -> Self {
        Self {
            path: None,
            data: Map::new(),
            dirty: false,
        }
    }

    pub fn get(&self, key: &str) -> Option<Value> {
        self.data.get(key).cloned()
    }

    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.data.insert(key.into(), value);
        self.dirty = true;
    }

    pub fn delete(&mut self, key: &str) {
        if self.data.remove(key).is_some() {
            self.dirty = true;
        }
    }

    pub fn flush(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let Some(path) = &self.path else {
            self.dirty = false;
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let tmp = tmp_path(path);
        let mut ordered = BTreeMap::new();
        for (key, value) in &self.data {
            ordered.insert(key, value);
        }
        let source = serde_json::to_string_pretty(&ordered)?;
        fs::write(&tmp, source).with_context(|| format!("failed to write {}", tmp.display()))?;
        fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
        self.dirty = false;
        Ok(())
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut tmp = path.to_path_buf();
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| format!("{ext}.tmp"))
        .unwrap_or_else(|| "tmp".to_string());
    tmp.set_extension(ext);
    tmp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_round_trips_through_disk() {
        let root = std::env::temp_dir().join(format!("tpane-store-{}", std::process::id()));
        let path = root.join("state.json");
        let _ = fs::remove_dir_all(&root);

        let mut store = Store::load(&path);
        store.set("counter", Value::from(2));
        store.set("name", Value::from("tpane"));
        store.flush().unwrap();

        let reloaded = Store::load(&path);
        assert_eq!(reloaded.get("counter"), Some(Value::from(2)));
        assert_eq!(reloaded.get("name"), Some(Value::from("tpane")));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn store_delete_persists() {
        let root = std::env::temp_dir().join(format!("tpane-store-delete-{}", std::process::id()));
        let path = root.join("state.json");
        let _ = fs::remove_dir_all(&root);

        let mut store = Store::load(&path);
        store.set("key", Value::from(true));
        store.flush().unwrap();
        store.delete("key");
        store.flush().unwrap();

        let reloaded = Store::load(&path);
        assert_eq!(reloaded.get("key"), None);

        let _ = fs::remove_dir_all(root);
    }
}
