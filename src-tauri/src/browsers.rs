//! Which programs are web browsers, and where the Chromium ones keep their history. Plain facts about the PC that more
//! than one plugin needs (the page reader and the downloads).

use std::path::PathBuf;
use std::time::SystemTime;

/// by exe path or file name
pub fn is_browser_exe(exe: &str) -> bool {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_lowercase();
    let base = file.strip_suffix(".exe").unwrap_or(&file);
    matches!(base, "chrome" | "msedge" | "firefox" | "brave" | "opera" | "vivaldi" | "arc")
}

/// Newest `History` file among a Chromium browser's profiles.
pub(crate) fn history_path(exe: &str) -> Option<PathBuf> {
    let file = exe.rsplit(['\\', '/']).next()?.to_lowercase();
    let local = PathBuf::from(std::env::var_os("LOCALAPPDATA")?);
    let roaming = std::env::var_os("APPDATA").map(PathBuf::from);
    let data_dir = match file.strip_suffix(".exe").unwrap_or(&file) {
        "chrome" => local.join("Google/Chrome/User Data"),
        "msedge" => local.join("Microsoft/Edge/User Data"),
        "brave" => local.join("BraveSoftware/Brave-Browser/User Data"),
        "vivaldi" => local.join("Vivaldi/User Data"),
        "opera" => roaming?.join("Opera Software/Opera Stable"),
        _ => return None,
    };
    let mut best: Option<(SystemTime, PathBuf)> = None;
    let mut consider = |p: PathBuf| {
        if let Ok(t) = std::fs::metadata(&p).and_then(|m| m.modified()) {
            if best.as_ref().map_or(true, |(bt, _)| t > *bt) {
                best = Some((t, p));
            }
        }
    };
    consider(data_dir.join("History")); // Opera keeps it at the top
    if let Ok(rd) = std::fs::read_dir(&data_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name == "Default" || name.starts_with("Profile ") {
                consider(e.path().join("History"));
            }
        }
    }
    best.map(|(_, p)| p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_a_browser_by_its_file() {
        assert!(is_browser_exe(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe"));
        assert!(is_browser_exe("Chrome.EXE"));
        assert!(!is_browser_exe(r"C:\Windows\explorer.exe"));
    }
}
