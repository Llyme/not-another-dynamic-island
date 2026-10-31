//! What a window is working on, where Windows lets it be found: the folder a code editor has open (VS Code and its kin: the project
//! named in its title, among the folders it keeps a list of), the folder an Explorer window shows, the folder a terminal's shell is
//! in, or the document an app has open (its name in the title, found among Windows' recent files). Nothing, else. Kept for each
//! window until its title changes, so a dock that asks often costs nothing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub enum Context {
    /// a folder (a project, the one shown, the one a shell is in): opened
    Folder(PathBuf),
    /// a document: shown, chosen, in its folder
    File(PathBuf),
}

thread_local! {
    static KEPT: RefCell<HashMap<isize, (String, Option<Context>)>> = RefCell::new(HashMap::new());
}

/// What the window `hwnd` (of the program `exe`, titled `title`) is working on, if it can be found.
pub fn of(hwnd: isize, exe: &str, title: &str) -> Option<Context> {
    if let Some(c) = KEPT.with(|k| k.borrow().get(&hwnd).filter(|(t, _)| t == title).map(|(_, c)| c.clone())) {
        return c;
    }
    let stem = Path::new(exe).file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let found = match stem.as_str() {
        "code" | "code - insiders" | "cursor" | "vscodium" => editor(&stem, title),
        "explorer" => explorer(hwnd),
        "windowsterminal" | "cmd" | "powershell" | "pwsh" | "openconsole" | "conhost" | "wezterm-gui" | "alacritty" => terminal(hwnd),
        _ => document(title),
    };
    KEPT.with(|k| {
        let mut k = k.borrow_mut();
        if k.len() > 256 {
            k.clear();
        }
        k.insert(hwnd, (title.to_string(), found.clone()));
    });
    found
}

/// Open it: a folder in File Explorer; a document chosen in its folder.
pub fn open(c: &Context) {
    use std::os::windows::process::CommandExt;
    match c {
        Context::Folder(p) => {
            let _ = std::process::Command::new("explorer.exe").raw_arg(format!("\"{}\"", p.display())).spawn();
        }
        Context::File(p) => {
            let _ = std::process::Command::new("explorer.exe").raw_arg(format!("/select,\"{}\"", p.display())).spawn();
        }
    }
}

/// The parts of a title ("main.rs - NADI - Visual Studio Code": main.rs, NADI), its marks of unsaved work taken off.
fn parts(title: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in title.split([' ']).collect::<Vec<_>>().join(" ").split(" - ").flat_map(|p| p.split(" — ")).flat_map(|p| p.split(" – ")).flat_map(|p| p.split(" | ")) {
        let p = p.trim().trim_start_matches(['●', '*', '•']).trim_end_matches('*').trim();
        let p = p.strip_suffix(" (Workspace)").unwrap_or(p).trim();
        if !p.is_empty() {
            out.push(p.to_string());
        }
    }
    out
}

/// A `file:///c%3A/Users/x` address as a path (`c:\Users\x`).
fn path_of_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file:///")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let s = String::from_utf8_lossy(&out).replace('/', "\\");
    Some(PathBuf::from(s))
}

// ------------------------------------------------------------------------------------------------------------------- an editor

/// VS Code (and its kin): the folder it keeps in its list whose name the title says.
fn editor(stem: &str, title: &str) -> Option<Context> {
    let dir = match stem {
        "code - insiders" => "Code - Insiders",
        "cursor" => "Cursor",
        "vscodium" => "VSCodium",
        _ => "Code",
    };
    let storage = PathBuf::from(std::env::var_os("APPDATA")?).join(dir).join("User").join("globalStorage").join("storage.json");
    let text = std::fs::read_to_string(storage).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    let mut folders: Vec<PathBuf> = Vec::new();
    fn gather(v: &serde_json::Value, out: &mut Vec<PathBuf>) {
        match v {
            serde_json::Value::String(s) if s.starts_with("file:///") => {
                if let Some(p) = path_of_uri(s) {
                    // (a workspace's file: its folder)
                    let p = if p.extension().is_some_and(|e| e == "code-workspace") { p.parent().map(Path::to_path_buf).unwrap_or(p) } else { p };
                    if p.is_dir() && !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| gather(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| gather(x, out)),
            _ => {}
        }
    }
    gather(&v, &mut folders);
    let parts = parts(title);
    // (the last part is the editor's own name)
    for p in parts.iter().rev().skip(1) {
        if let Some(f) = folders.iter().find(|f| f.file_name().is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(p))) {
            return Some(Context::Folder(f.clone()));
        }
    }
    None
}

// ------------------------------------------------------------------------------------------------------------------ Explorer

/// The folder an Explorer window shows (as Windows tells it).
fn explorer(hwnd: isize) -> Option<Context> {
    use windows::core::Interface;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};
    use windows::core::VARIANT;
    use windows::Win32::UI::Shell::{IShellWindows, IWebBrowserApp, ShellWindows};
    unsafe {
        let sw: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let n = sw.Count().ok()?;
        for i in 0..n {
            let Ok(d) = sw.Item(&VARIANT::from(i)) else { continue };
            let Ok(app) = d.cast::<IWebBrowserApp>() else { continue };
            let Ok(h) = app.HWND() else { continue };
            if h.0 as isize != hwnd {
                continue;
            }
            let Ok(url) = app.LocationURL() else { continue };
            let url = url.to_string();
            if let Some(p) = path_of_uri(&url).filter(|p| p.is_dir()) {
                return Some(Context::Folder(p));
            }
        }
    }
    None
}

// ------------------------------------------------------------------------------------------------------------------ a terminal

/// The folder a terminal's shell is in: its shell (the window's program, or the newest shell under it), and that shell's own folder.
fn terminal(hwnd: isize) -> Option<Context> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid));
    }
    if pid == 0 {
        return None;
    }
    let shells = ["cmd.exe", "pwsh.exe", "powershell.exe", "bash.exe", "nu.exe", "fish.exe", "zsh.exe"];
    let procs = processes();
    let mut found: Option<u32> = None;
    // (the window's own program, when it is a shell; else the shells under it, the newest kept)
    if procs.iter().any(|(p, _, n)| *p == pid && shells.contains(&n.as_str())) {
        found = Some(pid);
    }
    let mut todo = vec![pid];
    while let Some(parent) = todo.pop() {
        for (p, pp, n) in &procs {
            if *pp == parent && *p != parent {
                if shells.contains(&n.as_str()) {
                    found = Some(*p);
                }
                todo.push(*p);
            }
        }
    }
    let dir = cwd_of(found?)?;
    dir.is_dir().then_some(Context::Folder(dir))
}

/// Every process: its id, its parent's, its program's file name (lower case).
fn processes() -> Vec<(u32, u32, String)> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..len]).to_lowercase()));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
    }
    out
}

/// The folder a process is in (its own, as it keeps it: read from it, as debuggers read it).
fn cwd_of(pid: u32) -> Option<PathBuf> {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, ProcessBasicInformation};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid).ok()?;
        let read = |at: usize, buf: &mut [u8]| ReadProcessMemory(h, at as *const _, buf.as_mut_ptr() as *mut _, buf.len(), None).is_ok();
        let mut pbi = PROCESS_BASIC_INFORMATION::default();
        let mut got = 0u32;
        let ok = NtQueryInformationProcess(h, ProcessBasicInformation, &mut pbi as *mut _ as *mut _, std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32, &mut got).is_ok();
        let result = (|| {
            if !ok || pbi.PebBaseAddress.is_null() {
                return None;
            }
            // (64-bit: the PEB's parameters at 0x20; in them, the folder (a counted string) at 0x38)
            let mut ptr = [0u8; 8];
            if !read(pbi.PebBaseAddress as usize + 0x20, &mut ptr) {
                return None;
            }
            let params = usize::from_le_bytes(ptr);
            let mut us = [0u8; 16];
            if !read(params + 0x38, &mut us) {
                return None;
            }
            let len = u16::from_le_bytes([us[0], us[1]]) as usize;
            let buf_at = usize::from_le_bytes(us[8..16].try_into().ok()?);
            if len == 0 || len > 4096 {
                return None;
            }
            let mut text = vec![0u8; len];
            if !read(buf_at, &mut text) {
                return None;
            }
            let wide: Vec<u16> = text.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            Some(PathBuf::from(String::from_utf16_lossy(&wide)))
        })();
        let _ = CloseHandle(h);
        result
    }
}

// ------------------------------------------------------------------------------------------------------------------ a document

/// The document an app has open: a part of its title that is the name of one of Windows' recent files (with or without its
/// extension), that file.
fn document(title: &str) -> Option<Context> {
    let recent = PathBuf::from(std::env::var_os("APPDATA")?).join("Microsoft").join("Windows").join("Recent");
    let links: Vec<PathBuf> = std::fs::read_dir(&recent).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("lnk"))).collect();
    for part in parts(title) {
        let want = part.to_lowercase();
        let hit = links.iter().find(|l| {
            let name = l.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
            name == want || Path::new(&name).file_stem().is_some_and(|s| s.to_string_lossy() == want)
        });
        if let Some(l) = hit {
            if let Some(target) = link_target(l).filter(|t| t.is_file()) {
                return Some(Context::File(target));
            }
        }
    }
    None
}

/// Where a shortcut (.lnk) leads.
fn link_target(lnk: &Path) -> Option<PathBuf> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let file: IPersistFile = link.cast().ok()?;
        file.Load(&HSTRING::from(lnk.as_os_str()), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let len = buf.iter().position(|c| *c == 0).unwrap_or(0);
        (len > 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len])))
    }
}
