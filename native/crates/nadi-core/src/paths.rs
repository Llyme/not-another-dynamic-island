//! Where NADI keeps things: `%APPDATA%\NADI` (the plugins in `plugins\`, what is set in a few JSON files).

use std::path::PathBuf;

pub fn data_dir() -> Option<PathBuf> {
    let base = std::env::var_os("APPDATA").map(PathBuf::from)?;
    Some(base.join("NADI"))
}

pub fn plugins_dir() -> Option<PathBuf> {
    Some(data_dir()?.join("plugins"))
}

/// Write a file whole or not at all (a crash halfway leaves the old one).
pub fn write_atomic(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}
