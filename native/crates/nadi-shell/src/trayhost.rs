//! The tray, held by NADI: the icons other apps put in the notification area, so the frame can draw them.
//!
//! An app adds its icon by sending it to the window of class `Shell_TrayWnd` that Windows finds first, which is normally Explorer's
//! taskbar. NADI makes a window of that class of its own, kept above Explorer's (it is never shown), and tells every app the taskbar
//! was made again (`TaskbarCreated`, what Explorer says when it restarts): each app sends its icons again, and they come here. Every
//! message is passed on to Explorer's taskbar as well, so the real one stays whole: its tray keeps every icon, and what apps ask the
//! taskbar (where it is, whether it hides) Explorer still answers.
//!
//! It runs on a thread of its own, so a slow Explorer never stalls the frame. When it stops (NADI exits, or the setting is turned
//! off), its window goes and the apps are told again, so they send their icons to Explorer only. If NADI ends without stopping it,
//! the watchdog tells them.

use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

/// One icon in the tray.
#[derive(Clone, Debug)]
pub struct Icon {
    /// the window that owns it, and its number there (or its GUID): what names it
    pub owner: isize,
    pub uid: u32,
    pub guid: Option<[u8; 16]>,
    /// the message its owner hears clicks with, and the way it wants them said (0..4)
    pub callback: u32,
    pub version: u32,
    /// its picture: a copy of the app's icon, owned here (0: none yet)
    pub icon: isize,
    pub tip: String,
    pub hidden: bool,
    pub pid: u32,
    /// the order it came in
    pub seq: u64,
}

static ICONS: Mutex<Vec<Icon>> = Mutex::new(Vec::new());
/// counts every change (the frame draws the tray again when it moved)
static CHANGES: AtomicU64 = AtomicU64::new(0);
static SEQ: AtomicU64 = AtomicU64::new(0);
static HOST: AtomicIsize = AtomicIsize::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
/// the window told of changes (WM_TRAY_CHANGED)
static NOTIFY: AtomicIsize = AtomicIsize::new(0);

pub const WM_TRAY_CHANGED: u32 = WM_APP + 9;
const WM_HOST_STOP: u32 = WM_APP + 10;

const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_STATE: u32 = 0x8;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x1;
const SIGNATURE: u32 = 0x34753423;

/// What an app sends: the shell's 32-bit form of NOTIFYICONDATA (handles as 32 bits, on 64-bit Windows too).
#[repr(C)]
#[derive(Clone, Copy)]
struct Nid32 {
    cb_size: u32,
    hwnd: u32,
    uid: u32,
    flags: u32,
    callback: u32,
    icon: u32,
    tip: [u16; 128],
    state: u32,
    state_mask: u32,
    info: [u16; 256],
    timeout_or_version: u32,
    info_title: [u16; 64],
    info_flags: u32,
    guid: [u8; 16],
    balloon_icon: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct TrayData {
    signature: u32,
    message: u32,
    nid: Nid32,
}

/// The icons now (the hidden ones too), in the order they came.
pub fn icons() -> Vec<Icon> {
    let mut v = ICONS.lock().unwrap().clone();
    v.sort_by_key(|i| i.seq);
    v
}

/// Messages passed on to Explorer, and how many it did not answer (for checking).
pub static FORWARDED: AtomicU64 = AtomicU64::new(0);
pub static UNANSWERED: AtomicU64 = AtomicU64::new(0);

pub fn running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Explorer's taskbar: the `Shell_TrayWnd` that is not NADI's.
pub fn explorer_tray() -> Option<HWND> {
    let me = HOST.load(Ordering::Relaxed);
    let mut after = HWND::default();
    unsafe {
        while let Ok(h) = FindWindowExW(None, after, w!("Shell_TrayWnd"), None) {
            if h.is_invalid() {
                break;
            }
            if h.0 as isize != me {
                let mut pid = 0u32;
                GetWindowThreadProcessId(h, Some(&mut pid));
                if pid != std::process::id() {
                    return Some(h);
                }
            }
            after = h;
        }
    }
    None
}

/// While NADI holds the tray, a mark in its folder (gone when it lets go).
pub fn mark() -> std::path::PathBuf {
    nadi_core::paths::data_dir().unwrap_or_default().join("tray-held")
}

/// Explorer's own process (its taskbar's), if it has one now: its tray icons (the speaker, removable drives) are not apps' (they have
/// widgets of their own).
pub fn explorer_pid() -> Option<u32> {
    explorer_tray().and_then(|h| {
        let mut pid = 0u32;
        unsafe {
            GetWindowThreadProcessId(h, Some(&mut pid));
        }
        (pid != 0).then_some(pid)
    })
}

/// Every app: send your tray icons again (to whichever taskbar is found first now).
pub fn announce() {
    unsafe {
        let msg = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, msg, WPARAM(0), LPARAM(0));
    }
}

fn changed() {
    CHANGES.fetch_add(1, Ordering::Relaxed);
    let to = NOTIFY.load(Ordering::Relaxed);
    if to != 0 {
        unsafe {
            let _ = PostMessageW(HWND(to as *mut _), WM_TRAY_CHANGED, WPARAM(0), LPARAM(0));
        }
    }
}

fn text(w: &[u16]) -> String {
    let n = w.iter().position(|c| *c == 0).unwrap_or(w.len());
    String::from_utf16_lossy(&w[..n])
}

/// A 32-bit handle as a handle of this process (sign-extended, as Windows does).
fn handle(h: u32) -> isize {
    h as i32 as isize
}

fn free_icon(h: isize) {
    if h != 0 {
        unsafe {
            let _ = DestroyIcon(HICON(h as *mut _));
        }
    }
}

/// What an app said about its icon: kept.
fn take(message: u32, nid: &Nid32) -> bool {
    let owner = handle(nid.hwnd);
    let guid = if nid.flags & NIF_GUID != 0 { Some(nid.guid) } else { None };
    let mut list = ICONS.lock().unwrap();
    // (by its GUID when both say one; else by its window and number: an app may name it one way when it adds it and the other way
    // after, as Qt does)
    let at = list.iter().position(|i| match (guid, i.guid) {
        (Some(g), Some(h)) => g == h,
        _ => i.owner == owner && i.uid == nid.uid,
    });
    match message {
        NIM_ADD | NIM_MODIFY => {
            if message == NIM_ADD && at.is_some() && list[at.unwrap()].owner != owner {
                return false;
            }
            let i = match at {
                Some(i) => i,
                None => {
                    if message == NIM_MODIFY {
                        return false;
                    }
                    let mut pid = 0u32;
                    unsafe {
                        GetWindowThreadProcessId(HWND(owner as *mut _), Some(&mut pid));
                    }
                    list.push(Icon { owner, uid: nid.uid, guid, callback: 0, version: 0, icon: 0, tip: String::new(), hidden: false, pid, seq: SEQ.fetch_add(1, Ordering::Relaxed) });
                    list.len() - 1
                }
            };
            let it = &mut list[i];
            it.owner = owner;
            if nid.flags & NIF_MESSAGE != 0 {
                it.callback = nid.callback;
            }
            if nid.flags & NIF_ICON != 0 {
                // (a copy of its own: the app may let its icon go)
                let copy = unsafe { CopyIcon(HICON(handle(nid.icon) as *mut _)) }.map(|h| h.0 as isize).unwrap_or(0);
                free_icon(std::mem::replace(&mut it.icon, copy));
            }
            if nid.flags & NIF_TIP != 0 {
                it.tip = text(&nid.tip);
            }
            if nid.flags & NIF_STATE != 0 && nid.state_mask & NIS_HIDDEN != 0 {
                it.hidden = nid.state & NIS_HIDDEN != 0;
            }
            true
        }
        NIM_DELETE => {
            if let Some(i) = at {
                let it = list.remove(i);
                free_icon(it.icon);
                true
            } else {
                false
            }
        }
        NIM_SETVERSION => {
            if let Some(i) = at {
                list[i].version = nid.timeout_or_version;
                true
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Icons whose window is gone (its app ended without saying so): let go.
fn sweep() {
    let mut list = ICONS.lock().unwrap();
    let before = list.len();
    list.retain(|i| {
        let alive = unsafe { IsWindow(HWND(i.owner as *mut _)).as_bool() };
        if !alive {
            free_icon(i.icon);
        }
        alive
    });
    if list.len() != before {
        drop(list);
        changed();
    }
}

/// Pass a message on to Explorer's taskbar, and give back its answer.
unsafe fn forward(msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    let to = explorer_tray()?;
    let mut out = 0usize;
    FORWARDED.fetch_add(1, Ordering::Relaxed);
    let ok = SendMessageTimeoutW(to, msg, wp, lp, SMTO_ABORTIFHUNG | SMTO_NORMAL, 400, Some(&mut out));
    if ok.0 == 0 {
        UNANSWERED.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(LRESULT(out as isize))
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_COPYDATA => {
            let cds = &*(lp.0 as *const COPYDATASTRUCT);
            if cds.dwData == 1 && cds.cbData as usize >= std::mem::size_of::<TrayData>() - std::mem::size_of::<Nid32>() + 24 {
                // (a tray icon: kept here, and passed on)
                let mut data: TrayData = std::mem::zeroed();
                let n = (cds.cbData as usize).min(std::mem::size_of::<TrayData>());
                std::ptr::copy_nonoverlapping(cds.lpData as *const u8, &mut data as *mut TrayData as *mut u8, n);
                let mine = data.signature == SIGNATURE && take(data.message, &data.nid);
                if mine {
                    changed();
                }
                return forward(msg, wp, lp).unwrap_or(LRESULT(if mine { 1 } else { 0 }));
            }
            // (the rest: where the taskbar is, an icon's place; Explorer answers)
            forward(msg, wp, lp).unwrap_or(LRESULT(0))
        }
        WM_HOST_STOP => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_TIMER => {
            // (kept above Explorer's taskbar, which comes up over it when it shows itself: an app adding its icon finds this one)
            if wp.0 == 3 {
                let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING);
                return LRESULT(0);
            }
            // (Explorer restarted, a moment ago: the apps are asked again)
            if wp.0 == 2 {
                let _ = KillTimer(hwnd, 2);
                *SAID_AT.lock().unwrap() = Some(Instant::now());
                let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                announce();
                return LRESULT(0);
            }
            // kept above Explorer's (so apps find this one first), and icons of apps that ended let go
            let _ = SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            sweep();
            LRESULT(0)
        }
        // (what other apps send the taskbar, such as its commands: Explorer's to answer)
        m if m == WM_COMMAND || m >= WM_USER && m < 0xC000 => {
            if let Some(to) = explorer_tray() {
                let _ = PostMessageW(to, m, wp, lp);
            }
            LRESULT(0)
        }
        _ => {
            // Explorer restarted (it says so to everyone): this window goes back on top and the apps are asked again, a moment
            // later (once its taskbar is up). What NADI says itself is not heard again.
            if msg == TASKBAR_CREATED.load(Ordering::Relaxed) as u32 && msg != 0 {
                let quiet = SAID_AT.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_secs(3));
                if !quiet {
                    *SAID_AT.lock().unwrap() = Some(Instant::now());
                    let _ = SetTimer(hwnd, 2, 2000, None);
                }
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

static TASKBAR_CREATED: AtomicIsize = AtomicIsize::new(0);
static SAID_AT: Mutex<Option<Instant>> = Mutex::new(None);

unsafe extern "system" fn child_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// Start holding the tray (on a thread of its own); changes are told to `notify`.
pub fn start(notify: HWND) {
    NOTIFY.store(notify.0 as isize, Ordering::Relaxed);
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| unsafe {
        let Ok(inst) = GetModuleHandleW(None) else {
            RUNNING.store(false, Ordering::SeqCst);
            return;
        };
        TASKBAR_CREATED.store(RegisterWindowMessageW(w!("TaskbarCreated")) as isize, Ordering::Relaxed);
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("Shell_TrayWnd"), ..Default::default() };
        RegisterClassW(&wc);
        let child = WNDCLASSW { lpfnWndProc: Some(child_proc), hInstance: inst.into(), lpszClassName: w!("TrayNotifyWnd"), ..Default::default() };
        RegisterClassW(&child);
        // (never shown: found, not seen)
        let Ok(h) = CreateWindowExW(WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("Shell_TrayWnd"), w!(""), WS_POPUP | WS_CLIPCHILDREN | WS_CLIPSIBLINGS, 0, 0, 0, 0, None, None, inst, None) else {
            RUNNING.store(false, Ordering::SeqCst);
            return;
        };
        let _ = CreateWindowExW(WINDOW_EX_STYLE(0), w!("TrayNotifyWnd"), w!(""), WS_CHILD, 0, 0, 0, 0, h, None, inst, None);
        // (the taskbar's place, so an app that measures it finds the real one)
        if let Some(ex) = explorer_tray() {
            let mut r = RECT::default();
            let _ = GetWindowRect(ex, &mut r);
            let _ = SetWindowPos(h, HWND_TOPMOST, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOACTIVATE);
        }
        HOST.store(h.0 as isize, Ordering::Relaxed);
        let _ = std::fs::write(mark(), b"1");
        let _ = SetTimer(h, 1, 1000, None);
        let _ = SetTimer(h, 3, 100, None);
        *SAID_AT.lock().unwrap() = Some(Instant::now());
        announce();
        let mut m = MSG::default();
        while GetMessageW(&mut m, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&m);
            DispatchMessageW(&m);
        }
        HOST.store(0, Ordering::Relaxed);
        for i in ICONS.lock().unwrap().drain(..) {
            free_icon(i.icon);
        }
        RUNNING.store(false, Ordering::SeqCst);
        changed();
        // (the apps: back to Explorer's taskbar only)
        announce();
        let _ = std::fs::remove_file(mark());
    });
}

/// Stop holding the tray (the apps go back to Explorer's).
pub fn stop() {
    let h = HOST.load(Ordering::Relaxed);
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), WM_HOST_STOP, WPARAM(0), LPARAM(0));
        }
        // (a moment for the thread to let go, as NADI exits)
        let until = Instant::now() + Duration::from_millis(600);
        while RUNNING.load(Ordering::SeqCst) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// How an icon is pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Click {
    Left,
    Right,
}

/// Press an icon, as Explorer would: its app hears the mouse (and may bring its own window or menu forward). `x`, `y`: where on the
/// screen (physical px), for the app's menu.
pub fn click(owner: isize, uid: u32, how: Click, x: i32, y: i32) {
    let Some(it) = ICONS.lock().unwrap().iter().find(|i| i.owner == owner && i.uid == uid).cloned() else { return };
    if it.callback == 0 {
        return;
    }
    let h = HWND(it.owner as *mut _);
    const NIN_SELECT: u32 = WM_USER;
    const WM_CONTEXTMENU_: u32 = 0x007B;
    unsafe {
        // (its menu or window may take the focus)
        let _ = AllowSetForegroundWindow(it.pid);
        let say = |m: u32| {
            let (wp, lp) = if it.version >= 4 {
                (WPARAM(((y as u32 & 0xffff) << 16 | (x as u32 & 0xffff)) as usize), LPARAM(((it.uid & 0xffff) << 16 | (m & 0xffff)) as isize))
            } else {
                (WPARAM(it.uid as usize), LPARAM(m as isize))
            };
            let _ = PostMessageW(h, it.callback, wp, lp);
        };
        match how {
            Click::Left => {
                say(WM_LBUTTONDOWN);
                say(WM_LBUTTONUP);
                // (said whatever way it asked: an app that only listens for the newer one, as Qt's do, hears it; an older one lets
                // it pass)
                say(NIN_SELECT);
            }
            Click::Right => {
                say(WM_RBUTTONDOWN);
                say(WM_RBUTTONUP);
                if it.version >= 4 {
                    say(WM_CONTEXTMENU_);
                }
            }
        }
    }
}
