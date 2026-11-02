//! The windows of the apps that are open, as the taskbar would list them: visible, not owned, not tool windows, not cloaked (another
//! desktop, a suspended store app), with a title; grouped by program. And what can be done with one: bring it to the front, minimize it,
//! close it.

use std::collections::HashMap;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Debug, PartialEq)]
pub struct AppWindow {
    pub hwnd: isize,
    pub title: String,
    /// the program's full path, lower case ("" when it cannot be read)
    pub exe: String,
    pub minimized: bool,
    pub front: bool,
}

fn cloaked(h: HWND) -> bool {
    let mut v: u32 = 0;
    unsafe { DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut _, 4).is_ok() && v != 0 }
}

/// Every app window of every virtual desktop (those of the others are cloaked by the shell), top first: its handle, its program,
/// its box on the screen (physical px), and its desktop's id (as Windows keeps it, 16 bytes). Minimized ones are left out.
pub fn desktop_windows() -> Vec<(isize, String, RECT, [u8; 16], String)> {
    use windows::Win32::UI::Shell::{IVirtualDesktopManager, VirtualDesktopManager};
    struct Ctx {
        out: Vec<HWND>,
        me: u32,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() || GetWindowTextLengthW(h) == 0 {
            return TRUE;
        }
        if GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()) {
            return TRUE;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if ex & WS_EX_TOOLWINDOW.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0 {
            return TRUE;
        }
        // (cloaked by the shell: on another desktop; cloaked by its app: not really there)
        let mut v: u32 = 0;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut v as *mut u32 as *mut _, 4);
        if v != 0 && v != DWM_CLOAKED_SHELL {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == ctx.me {
            return TRUE;
        }
        ctx.out.push(h);
        TRUE
    }
    let mut ctx = Ctx { out: Vec::new(), me: std::process::id() };
    let mut cache = HashMap::new();
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut _ as isize));
        let Ok(vdm) = windows::Win32::System::Com::CoCreateInstance::<_, IVirtualDesktopManager>(&VirtualDesktopManager, None, windows::Win32::System::Com::CLSCTX_ALL) else { return out };
        for h in ctx.out {
            let Ok(g) = vdm.GetWindowDesktopId(h) else { continue };
            if g == windows::core::GUID::zeroed() {
                continue;
            }
            let mut id = [0u8; 16];
            id[..4].copy_from_slice(&g.data1.to_le_bytes());
            id[4..6].copy_from_slice(&g.data2.to_le_bytes());
            id[6..8].copy_from_slice(&g.data3.to_le_bytes());
            id[8..].copy_from_slice(&g.data4);
            let mut r = RECT::default();
            if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut RECT as *mut _, std::mem::size_of::<RECT>() as u32).is_err() {
                let _ = GetWindowRect(h, &mut r);
            }
            if r.right - r.left < 40 || r.bottom - r.top < 30 {
                continue;
            }
            let mut t = [0u16; 256];
            let n = GetWindowTextW(h, &mut t) as usize;
            out.push((h.0 as isize, exe_of(h, &mut cache), r, id, String::from_utf16_lossy(&t[..n.min(256)])));
        }
    }
    out
}

/// The program behind a window. A store app's frame (ApplicationFrameHost) holds the app's own window as a child: its program is that
/// child's.
fn exe_of(h: HWND, cache: &mut HashMap<u32, String>) -> String {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
    }
    let exe = cache.entry(pid).or_insert_with(|| nadi_core::win::exe_path_for_pid(pid).unwrap_or_default().to_lowercase()).clone();
    if exe.ends_with("applicationframehost.exe") {
        struct Ctx<'a> {
            parent_pid: u32,
            found: Option<u32>,
            _c: &'a (),
        }
        unsafe extern "system" fn child(c: HWND, lp: LPARAM) -> BOOL {
            let ctx = &mut *(lp.0 as *mut Ctx);
            let mut pid = 0u32;
            GetWindowThreadProcessId(c, Some(&mut pid));
            if pid != ctx.parent_pid {
                ctx.found = Some(pid);
                return FALSE;
            }
            TRUE
        }
        let unit = ();
        let mut ctx = Ctx { parent_pid: pid, found: None, _c: &unit };
        unsafe {
            let _ = EnumChildWindows(h, Some(child), LPARAM(&mut ctx as *mut _ as isize));
        }
        if let Some(p) = ctx.found {
            return cache.entry(p).or_insert_with(|| nadi_core::win::exe_path_for_pid(p).unwrap_or_default().to_lowercase()).clone();
        }
    }
    exe
}

/// The app windows, in the order of the screen (the one in front first).
pub fn app_windows() -> Vec<AppWindow> {
    struct Ctx {
        out: Vec<AppWindow>,
        cache: HashMap<u32, String>,
        me: u32,
        front: HWND,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if !IsWindowVisible(h).as_bool() || cloaked(h) {
            return TRUE;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let owner = GetWindow(h, GW_OWNER).unwrap_or_default();
        // (what the taskbar shows: an app window, or an owned one that asks to be shown)
        if ex & WS_EX_TOOLWINDOW.0 != 0 || (!owner.is_invalid() && ex & WS_EX_APPWINDOW.0 == 0) || ex & WS_EX_NOACTIVATE.0 != 0 {
            return TRUE;
        }
        let mut buf = [0u16; 256];
        let n = GetWindowTextW(h, &mut buf) as usize;
        if n == 0 {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == ctx.me {
            return TRUE;
        }
        let exe = exe_of(h, &mut ctx.cache);
        ctx.out.push(AppWindow { hwnd: h.0 as isize, title: String::from_utf16_lossy(&buf[..n]), exe, minimized: IsIconic(h).as_bool(), front: h == ctx.front });
        TRUE
    }
    // (each process's program, kept from one look to the next: asked of Windows once a process)
    thread_local! {
        static EXES: std::cell::RefCell<HashMap<u32, String>> = std::cell::RefCell::new(HashMap::new());
    }
    let cache = EXES.with(|c| std::mem::take(&mut *c.borrow_mut()));
    let mut ctx = Ctx { out: Vec::new(), cache, me: std::process::id(), front: unsafe { GetForegroundWindow() } };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut _ as isize));
    }
    // (only the processes that still have windows are kept: a number used again is another process)
    let alive: std::collections::HashSet<String> = ctx.out.iter().map(|w| w.exe.clone()).collect();
    let mut cache = std::mem::take(&mut ctx.cache);
    cache.retain(|_, e| alive.contains(e));
    EXES.with(|c| *c.borrow_mut() = cache);
    ctx.out
}

/// What is in front on a monitor: an app full screen (a game, a video), a maximized window, or neither. Not only the window with the
/// focus: the top window on each monitor.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Front {
    pub full: bool,
    pub maxed: bool,
}

/// The top window on each monitor, from the front: what it is. Windows that float over others without being an app's (tool windows,
/// click-through overlays, small always-on-top ones) are looked past; the desktop is neither.
/// Is another app's window on top of everything in front now (a menu it opened, from its tray icon or anywhere, or a popup of its
/// own)? The frame does not put itself back over it while it is.
pub fn other_on_top() -> bool {
    unsafe {
        // (a menu of the system's kind, open anywhere)
        if FindWindowW(windows::core::w!("#32768"), None).is_ok_and(|m| !m.is_invalid() && IsWindowVisible(m).as_bool()) {
            return true;
        }
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 || pid == std::process::id() || GetWindowLongW(fg, GWL_EXSTYLE) as u32 & WS_EX_TOPMOST.0 == 0 {
            return false;
        }
        // (a popup of its own: no title bar, and a small part of its monitor. A game kept on top, full screen or in a window, is not
        // one: the frame goes over it)
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return false;
        }
        let mut r = RECT::default();
        let _ = GetWindowRect(fg, &mut r);
        let mon = windows::Win32::Graphics::Gdi::MonitorFromWindow(fg, windows::Win32::Graphics::Gdi::MONITOR_DEFAULTTONEAREST);
        let mut mi = windows::Win32::Graphics::Gdi::MONITORINFO { cbSize: std::mem::size_of::<windows::Win32::Graphics::Gdi::MONITORINFO>() as u32, ..Default::default() };
        if !windows::Win32::Graphics::Gdi::GetMonitorInfoW(mon, &mut mi).as_bool() {
            return true;
        }
        let m = mi.rcMonitor;
        let area = ((r.right - r.left).max(0) as i64) * ((r.bottom - r.top).max(0) as i64);
        let whole = ((m.right - m.left).max(1) as i64) * ((m.bottom - m.top).max(1) as i64);
        area * 2 < whole
    }
}

/// Is one of Windows' own see-through panels in front (Start, Search, quick settings, the notification centre)?
pub fn shell_panel_open() -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        class == "Windows.UI.Core.CoreWindow" || class == "XamlExplorerHostIslandWindow" || class.starts_with("ControlCenterWindow")
    }
}


pub fn fronts() -> Vec<(isize, Front)> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONULL};
    struct Ctx {
        out: Vec<(isize, Front)>,
        me: u32,
    }
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let ctx = &mut *(lp.0 as *mut Ctx);
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() || cloaked(h) {
            return TRUE;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == ctx.me {
            return TRUE;
        }
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONULL);
        if mon.is_invalid() || ctx.out.iter().any(|(m, _)| *m == mon.0 as isize) {
            return TRUE;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        if ex & (WS_EX_TOOLWINDOW.0 | WS_EX_TRANSPARENT.0 | WS_EX_NOACTIVATE.0) != 0 {
            return TRUE;
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() || r.right - r.left < 50 || r.bottom - r.top < 50 {
            return TRUE;
        }
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return TRUE;
        }
        let m = mi.rcMonitor;
        let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
        // (always on top and not covering the screen: a little floating thing, looked past)
        if ex & WS_EX_TOPMOST.0 != 0 && !covers {
            return TRUE;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(h, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        let desktop = matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd");
        let zoomed = IsZoomed(h).as_bool();
        // (full screen has no title bar and is not zoomed: a maximized window covers the screen too while the taskbar hides itself)
        let framed = style & WS_CAPTION.0 == WS_CAPTION.0 || zoomed;
        let front = Front { full: !desktop && covers && !framed, maxed: !desktop && zoomed };
        ctx.out.push((mon.0 as isize, front));
        TRUE
    }
    let mut ctx = Ctx { out: Vec::new(), me: std::process::id() };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut ctx as *mut _ as isize));
    }
    ctx.out
}

/// Does Windows keep the keyboard of the window in front from NADI? So it is when that window's program runs as administrator and NADI
/// does not, or when it lets no one look at it (a game's anti-cheat): NADI then cannot tell whether a key is held in it.
pub fn keys_hidden_in_front() -> bool {
    keys_hidden(unsafe { GetForegroundWindow() })
}

/// Does Windows keep the keyboard of this window from NADI? (as `keys_hidden_in_front`)
pub fn keys_hidden(fg: HWND) -> bool {
    use windows::Win32::Security::*;
    use windows::Win32::System::Threading::*;
    fn elevated(process: HANDLE) -> Option<bool> {
        unsafe {
            let mut token = HANDLE::default();
            OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
            let mut e = TOKEN_ELEVATION::default();
            let mut n = 0u32;
            let ok = GetTokenInformation(token, TokenElevation, Some(&mut e as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut n);
            let _ = CloseHandle(token);
            ok.ok()?;
            Some(e.TokenIsElevated != 0)
        }
    }
    unsafe {
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return false;
        }
        if elevated(GetCurrentProcess()) == Some(true) {
            return false;
        }
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let theirs = elevated(h);
        let _ = CloseHandle(h);
        theirs.unwrap_or(true)
    }
}

/// Is the mouse cursor shown (a game playing hides it)?
pub fn cursor_showing() -> bool {
    let mut ci = CURSORINFO { cbSize: std::mem::size_of::<CURSORINFO>() as u32, ..Default::default() };
    unsafe { GetCursorInfo(&mut ci).is_ok() && ci.flags.0 & CURSOR_SHOWING.0 != 0 && !ci.hCursor.is_invalid() }
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

/// Bring a window to the front (restored if it was minimized).
pub fn activate(h: isize) {
    let w = hwnd(h);
    unsafe {
        if IsIconic(w).as_bool() {
            let _ = ShowWindow(w, SW_RESTORE);
        }
        // (a background process may not take the focus: the foreground thread's input is joined for a moment)
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        let fg = GetForegroundWindow();
        let fg_tid = if fg.is_invalid() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let me = GetCurrentThreadId();
        let joined = fg_tid != 0 && fg_tid != me && AttachThreadInput(me, fg_tid, true).as_bool();
        let _ = BringWindowToTop(w);
        let _ = SetForegroundWindow(w);
        if joined {
            let _ = AttachThreadInput(me, fg_tid, false);
        }
    }
}

pub fn minimize(h: isize) {
    unsafe {
        let _ = ShowWindow(hwnd(h), SW_MINIMIZE);
    }
}

/// Ask a window to close (as its close button would: it may ask to save).
pub fn close(h: isize) {
    unsafe {
        let _ = PostMessageW(hwnd(h), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

/// A program's name for people: its product name, else its file's name.
pub fn app_name(exe: &str) -> String {
    nadi_core::sensors::exeinfo::lookup(exe).name.filter(|n| !n.is_empty()).unwrap_or_else(|| {
        let f = exe.rsplit(['\\', '/']).next().unwrap_or(exe);
        let stem = f.strip_suffix(".exe").unwrap_or(f);
        let mut c = stem.chars();
        c.next().map(|x| x.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
    })
}
