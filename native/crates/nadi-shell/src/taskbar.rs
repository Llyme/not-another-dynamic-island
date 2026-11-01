//! The Windows taskbar, when the person wants it gone: hidden on every monitor, and the work area of each monitor given back whole (so
//! maximized windows fill the screen). It comes back when NADI exits, when the setting is turned off, and at the next start if NADI did
//! not exit cleanly (a mark is left in NADI's folder while it is hidden).

use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::{MonitorFromWindow, MONITOR_DEFAULTTONEAREST};
use windows::Win32::UI::WindowsAndMessaging::*;

fn mark() -> std::path::PathBuf {
    nadi_core::paths::data_dir().unwrap_or_default().join("taskbar-hidden")
}

/// The taskbar windows (the main one and the ones on other monitors).
fn bars() -> Vec<HWND> {
    let mut out = Vec::new();
    unsafe {
        // (Explorer's: NADI may hold a window of that class too, for the tray)
        if let Some(h) = crate::trayhost::explorer_tray() {
            out.push(h);
        }
        let mut after = HWND::default();
        while let Ok(h) = FindWindowExW(None, after, w!("Shell_SecondaryTrayWnd"), None) {
            if h.is_invalid() {
                break;
            }
            out.push(h);
            after = h;
        }
    }
    out
}

/// Where the taskbars are now (to know when one slid out or away).
pub fn rects() -> Vec<RECT> {
    bars()
        .into_iter()
        .map(|h| {
            let mut b = RECT::default();
            unsafe {
                let _ = GetWindowRect(h, &mut b);
            }
            b
        })
        .collect()
}

/// The part of a monitor its taskbar takes now, on whichever edge it stands: the monitor less that strip. One that hides itself
/// takes only what shows of it (a sliver while it is away, all of it while it is out).
pub fn free_of(m: &crate::overlay::Monitor) -> RECT {
    let mut free = m.rect;
    let (mw, mh) = (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
    for h in bars() {
        let mut b = RECT::default();
        unsafe {
            if GetWindowRect(h, &mut b).is_err() || MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST) != m.handle {
                continue;
            }
        }
        let (bw, bh) = (b.right - b.left, b.bottom - b.top);
        if bw >= mw * 3 / 4 {
            // along the top or the bottom: its full height, from that edge (a hidden one has slid mostly off the screen)
            if (b.top + b.bottom) / 2 > m.rect.top + mh / 2 {
                free.bottom = free.bottom.min(b.top.clamp(m.rect.top, m.rect.bottom));
            } else {
                free.top = free.top.max(b.bottom.clamp(m.rect.top, m.rect.bottom));
            }
        } else if bh >= mh * 3 / 4 {
            if (b.left + b.right) / 2 > m.rect.left + mw / 2 {
                free.right = free.right.min(b.left.clamp(m.rect.left, m.rect.right));
            } else {
                free.left = free.left.max(b.right.clamp(m.rect.left, m.rect.right));
            }
        }
    }
    free
}

pub fn hidden() -> bool {
    bars().first().is_some_and(|h| unsafe { !IsWindowVisible(*h).as_bool() })
}

/// Hide it (again: Explorer shows it when it restarts, and sometimes on the Start key). `reserve`: logical px kept free at the bottom
/// of every monitor (the dock's), so maximized windows end above it.
pub fn hide(reserve: f32) {
    let list = bars();
    if list.iter().all(|h| unsafe { !IsWindowVisible(*h).as_bool() }) {
        return;
    }
    note("hidden");
    // (the taskbar made to hide itself first: then Explorer gives the screen back by itself and keeps it so. Setting the work area
    // ourselves made Explorer take it back and show its taskbar again, every second, each time telling every window on the PC. Its
    // state as it was is kept in the mark, and put back when it comes back, here, at the next start, or by the watchdog)
    let first = !std::fs::read_to_string(mark()).is_ok_and(|t| t.starts_with("state="));
    if let Some(bar) = list.first() {
        let was = state(*bar);
        if first {
            let _ = std::fs::write(mark(), format!("state={was}"));
        }
        if was & ABS_AUTOHIDE == 0 {
            set_state(*bar, was | ABS_AUTOHIDE);
            note("made to hide itself");
        }
    } else if first {
        let _ = std::fs::write(mark(), b"1");
    }
    unsafe {
        for h in &list {
            let _ = ShowWindow(*h, SW_HIDE);
        }
    }
    // (the dock's room at the bottom: the work area set only when it is not so, now and then at most, off this thread: setting it
    // waits on every window on the PC)
    let want: Vec<RECT> = crate::overlay::monitors()
        .iter()
        .filter_map(|m| {
            let mut r = m.rect;
            r.bottom -= (reserve * m.scale).round() as i32;
            (m.work != r).then_some(r)
        })
        .collect();
    if want.is_empty() {
        return;
    }
    let now = std::time::Instant::now();
    let last = WORK_SET.with(|c| c.get());
    if last.is_some_and(|t| now.duration_since(t).as_secs() < 30) {
        return;
    }
    WORK_SET.with(|c| c.set(Some(now)));
    note("work area set");
    std::thread::spawn(move || unsafe {
        for mut r in want {
            let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut r as *mut RECT as *mut _), SPIF_SENDCHANGE);
        }
    });
}

thread_local! {
    /// when the work area was last set (not again and again)
    static WORK_SET: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

const ABM_GETSTATE: u32 = 4;
const ABM_SETSTATE: u32 = 10;
const ABS_AUTOHIDE: u32 = 1;

/// The taskbar's state (hides itself, always on top), as Explorer has it.
fn state(bar: HWND) -> u32 {
    unsafe {
        let mut d = windows::Win32::UI::Shell::APPBARDATA { cbSize: std::mem::size_of::<windows::Win32::UI::Shell::APPBARDATA>() as u32, hWnd: bar, ..Default::default() };
        windows::Win32::UI::Shell::SHAppBarMessage(ABM_GETSTATE, &mut d) as u32
    }
}

fn set_state(bar: HWND, st: u32) {
    unsafe {
        let mut d = windows::Win32::UI::Shell::APPBARDATA { cbSize: std::mem::size_of::<windows::Win32::UI::Shell::APPBARDATA>() as u32, hWnd: bar, lParam: LPARAM(st as isize), ..Default::default() };
        windows::Win32::UI::Shell::SHAppBarMessage(ABM_SETSTATE, &mut d);
    }
}

/// (for finding out what goes on: each time it is hidden again, and each time the work area is set, a line in taskbar.log; kept small)
fn note(what: &str) {
    let Some(dir) = nadi_core::paths::data_dir() else { return };
    let f = dir.join("taskbar.log");
    if std::fs::metadata(&f).is_ok_and(|m| m.len() > 64 * 1024) {
        let _ = std::fs::remove_file(&f);
    }
    use std::io::Write;
    if let Ok(mut o) = std::fs::OpenOptions::new().create(true).append(true).open(&f) {
        let _ = writeln!(o, "{} {what}", chrono::Local::now().format("%H:%M:%S%.3f"));
    }
}

/// Show it, and give the work areas back (each monitor's, less the taskbar on it, on whichever edge it stands).
pub fn restore() {
    let list = bars();
    // (its state as it was before NADI hid it: back to staying, if it stayed)
    if let (Some(bar), Some(was)) = (list.first(), std::fs::read_to_string(mark()).ok().and_then(|t| t.trim().strip_prefix("state=").and_then(|v| v.parse::<u32>().ok()))) {
        if was & ABS_AUTOHIDE == 0 && state(*bar) & ABS_AUTOHIDE != 0 {
            set_state(*bar, was);
            note("made to stay again");
        }
    }
    unsafe {
        for h in &list {
            let _ = ShowWindow(*h, SW_SHOWNA);
        }
        for m in crate::overlay::monitors() {
            let mut work = m.rect;
            for h in &list {
                let mut b = RECT::default();
                if GetWindowRect(*h, &mut b).is_err() || MonitorFromWindow(*h, MONITOR_DEFAULTTONEAREST) != m.handle {
                    continue;
                }
                let (mw, mh) = (m.rect.right - m.rect.left, m.rect.bottom - m.rect.top);
                if b.right - b.left >= mw * 3 / 4 {
                    if b.top > m.rect.top + mh / 2 { work.bottom = work.bottom.min(b.top) } else { work.top = work.top.max(b.bottom) }
                } else if b.bottom - b.top >= mh * 3 / 4 {
                    if b.left > m.rect.left + mw / 2 { work.right = work.right.min(b.left) } else { work.left = work.left.max(b.right) }
                }
            }
            let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut work as *mut RECT as *mut _), SPIF_SENDCHANGE);
        }
    }
    let _ = std::fs::remove_file(mark());
}

thread_local! {
    /// the taskbar shown for a moment (its icons wanted), while it is hidden: since when
    static PEEK: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

/// Show the hidden taskbar for a moment (its tray icons are wanted): it goes again once it is left.
pub fn peek() {
    unsafe {
        for h in bars() {
            let _ = ShowWindow(h, SW_SHOWNA);
        }
        if let Some(h) = bars().first() {
            let _ = SetForegroundWindow(*h);
        }
    }
    PEEK.with(|p| p.set(Some(std::time::Instant::now())));
}

pub fn peeking() -> bool {
    PEEK.with(|p| p.get()).is_some()
}

/// While it peeks: hidden again when the mouse and the focus are elsewhere (after a moment). True when it was hidden again.
pub fn peek_done() -> bool {
    let Some(at) = PEEK.with(|p| p.get()) else { return false };
    if at.elapsed().as_millis() < 2500 {
        return false;
    }
    unsafe {
        let fg = GetForegroundWindow();
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        // (the taskbar, its overflow of icons, or a menu of one of them: still in use)
        let mut pt = POINT::default();
        let _ = GetCursorPos(&mut pt);
        let over = bars().iter().any(|h| {
            let mut r = RECT::default();
            GetWindowRect(*h, &mut r).is_ok() && pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom
        });
        let busy = over || class.contains("TrayWnd") || class.contains("Overflow") || class == "#32768" || class.contains("Xaml");
        if busy && at.elapsed().as_secs() < 60 {
            return false;
        }
        for h in bars() {
            let _ = ShowWindow(h, SW_HIDE);
        }
    }
    PEEK.with(|p| p.set(None));
    true
}

/// At the start: a mark left by a NADI that did not exit cleanly brings the taskbar back (when it is not wanted hidden).
pub fn recover(want_hidden: bool) {
    if mark().exists() && !want_hidden {
        restore();
    }
}
