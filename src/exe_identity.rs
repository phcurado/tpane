use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};

pub fn current_exe_hash() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    let bytes = fs::read(exe).ok()?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(format!("{:016x}", hasher.finish()))
}
