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
    let _ = std::fs::write(mark(), b"1");
    unsafe {
        for h in &list {
            let _ = ShowWindow(*h, SW_HIDE);
        }
        // every monitor's work area: the whole monitor
        for m in crate::overlay::monitors() {
            let mut r = m.rect;
            r.bottom -= (reserve * m.scale).round() as i32;
            let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut r as *mut RECT as *mut _), SPIF_SENDCHANGE);
        }
    }
}

/// Show it, and give the work areas back (each monitor's, less the taskbar on it, on whichever edge it stands).
pub fn restore() {
    let list = bars();
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
