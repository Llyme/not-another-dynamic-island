//! Thin wrappers around the handful of raw Win32 calls the island needs:
//! global cursor position (works even when the window doesn't have focus,
//! which Tauri/webview mouse events don't give you) and the geometry of
//! whichever monitor the cursor is currently on.

use windows::Win32::Foundation::{HWND, POINT, RECT};
use windows::Win32::Media::timeBeginPeriod;
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MonitorFromPoint, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowLongPtrW, GetWindowLongW, GetWindowRect, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, GWL_STYLE, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, WS_CAPTION, WS_EX_LAYERED, WS_EX_TRANSPARENT,
};

/// Ask Windows for 1 ms timer resolution. Without it a 16 ms sleep really lasts
/// 16 or 31 ms by turns, which makes every animation driven by the poll loop stutter.
pub fn fine_timer() {
    unsafe {
        timeBeginPeriod(1);
    }
}

/// Let mouse clicks fall through the whole window (or not). Tauri's own
/// `set_ignore_cursor_events` rebuilds the window's frame every time it is called
/// (SWP_FRAMECHANGED + ShowWindow), which shows as a twitch; this only flips the style
/// bits, and only when they are not already as wanted.
pub fn click_through(hwnd: isize, on: bool) {
    unsafe {
        let h = HWND(hwnd as *mut _);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let mut want = ex | WS_EX_LAYERED.0 as isize;
        if on {
            want |= WS_EX_TRANSPARENT.0 as isize;
        } else {
            want &= !(WS_EX_TRANSPARENT.0 as isize);
        }
        if want != ex {
            SetWindowLongPtrW(h, GWL_EXSTYLE, want);
        }
    }
}

/// Move and resize a window in one step. Doing the two separately lets a frame
/// show the new size at the old position, which reads as jitter during a resize.
pub fn place(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as *mut _),
            None,
            x,
            y,
            w,
            h,
            SWP_NOZORDER | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

/// Put the window back on top of the topmost band, without moving, sizing or activating it.
/// `SetWindowPos` with `SWP_NOZORDER` (see `place`) never changes the stacking, so a window
/// that another topmost window (or the taskbar, a popup...) covered would stay covered.
pub fn raise(hwnd: isize) {
    unsafe {
        let _ = SetWindowPos(
            HWND(hwnd as *mut _),
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MonitorGeometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    #[allow(dead_code)]
    pub height: i32,
}

/// Whether Alt is held, globally: it lifts the fullscreen guard while it is down.
pub fn alt_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_MENU;
    unsafe { GetAsyncKeyState(VK_MENU.0 as i32) as u16 & 0x8000 != 0 }
}

/// Whether Ctrl is held, globally (the other key that can lift the fullscreen guard).
pub fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_CONTROL;
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) as u16 & 0x8000 != 0 }
}

/// Whether the left mouse button is currently held, globally -- used to
/// detect a native `start_dragging()` move-loop ending, since that loop can
/// swallow the DOM `mouseup` the webview would otherwise get.
pub fn left_button_down() -> bool {
    unsafe { GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000 != 0 }
}

/// Whether the current foreground window is truly fullscreen on `geo`:
/// covers the whole monitor AND has no title bar (`WS_CAPTION`), i.e.
/// exclusive-fullscreen or borderless-fullscreen. A maximized browser/IDE
/// still carries its caption even filling the screen, so alt-tabbing from a
/// game to a maximized window summons the island normally. A 2 px tolerance
/// absorbs window borders; the island's own window never qualifies (it is a
/// small pill) but is excluded anyway via `exclude_hwnd`.
pub fn foreground_fullscreen_on(geo: MonitorGeometry, exclude_hwnd: isize) -> bool {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || fg.0 as isize == exclude_hwnd {
            return false;
        }
        // caption first: cheap, and rules out every maximized normal window
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 != 0 {
            return false;
        }
        let mut rect = RECT::default();
        if GetWindowRect(fg, &mut rect).is_err() {
            return false;
        }
        // the covering window may live on another monitor -- only block the
        // edge it actually covers (the cursor's own monitor)
        rect.left <= geo.x + 2
            && rect.top <= geo.y + 2
            && rect.right >= geo.x + geo.width - 2
            && rect.bottom >= geo.y + geo.height - 2
    }
}

/// Global cursor position in physical screen pixels, regardless of which
/// window (if any) has focus.
pub fn cursor_pos() -> (i32, i32) {
    let mut pt = POINT::default();
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

/// The work-area-free full geometry of the monitor under the given point
/// (physical pixels), falling back to the nearest monitor if the point
/// isn't over any (matches Qt's `QApplication.screenAt` + nearest fallback
/// behavior the Python version relied on).
pub fn monitor_geometry_at(x: i32, y: i32) -> Option<MonitorGeometry> {
    let pt = POINT { x, y };
    let hmon: HMONITOR = unsafe { MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST) };
    if hmon.is_invalid() {
        return None;
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let ok = unsafe { GetMonitorInfoW(hmon, &mut info) };
    if ok.as_bool() {
        let RECT {
            left,
            top,
            right,
            bottom,
        } = info.rcMonitor;
        Some(MonitorGeometry {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        })
    } else {
        None
    }
}

/// The usable area of every monitor (not under the taskbar): left, top, right, bottom in physical px of the desktop.
pub fn monitor_work_areas() -> Vec<(i32, i32, i32, i32)> {
    use windows::Win32::Graphics::Gdi::EnumDisplayMonitors;
    use windows::Win32::Foundation::{BOOL, LPARAM};
    unsafe extern "system" fn each(
        hmon: HMONITOR,
        _dc: windows::Win32::Graphics::Gdi::HDC,
        _rc: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let out = &mut *(data.0 as *mut Vec<(i32, i32, i32, i32)>);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(hmon, &mut info).as_bool() {
            let r = info.rcWork;
            out.push((r.left, r.top, r.right, r.bottom));
        }
        BOOL(1)
    }
    let mut out: Vec<(i32, i32, i32, i32)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// How light the screen is inside a rectangle (physical px of the desktop): 0 (black) to 1 (white), the median of a
/// sparse grid of points, so a few text pixels do not count. The windows that are layered (the floating cards' own
/// layer is one) are not part of what a plain screen capture sees, so the cards do not see themselves. `None` when
/// nothing could be read, or when all of it is pure black (a game that bypasses the desktop compositor gives that).
pub fn screen_luma(l: i32, t: i32, r: i32, b: i32) -> Option<f32> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject, SetStretchBltMode,
        StretchBlt, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, COLORONCOLOR, DIB_RGB_COLORS, HGDIOBJ, SRCCOPY,
    };
    const W: i32 = 32;
    const H: i32 = 20;
    let (w, h) = (r - l, b - t);
    if w < 4 || h < 4 {
        return None;
    }
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return None;
        }
        let mem = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, W, H);
        let old = SelectObject(mem, HGDIOBJ(bmp.0));
        SetStretchBltMode(mem, COLORONCOLOR);
        let ok = StretchBlt(mem, 0, 0, W, H, screen, l, t, w, h, SRCCOPY).as_bool();
        let mut luma: Vec<f32> = Vec::new();
        if ok {
            let mut info = BITMAPINFO::default();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: W,
                biHeight: -H,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            };
            let mut px = vec![0u8; (W * H * 4) as usize];
            let got = GetDIBits(mem, bmp, 0, H as u32, Some(px.as_mut_ptr() as *mut _), &mut info, DIB_RGB_COLORS);
            if got != 0 {
                for p in px.chunks_exact(4) {
                    // (blue, green, red, unused)
                    luma.push((0.0722 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.2126 * p[2] as f32) / 255.0);
                }
            }
        }
        SelectObject(mem, old);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        if luma.is_empty() || luma.iter().all(|v| *v == 0.0) {
            return None;
        }
        luma.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Some(luma[luma.len() / 2])
    }
}

/// The window that has the keyboard (0 when none).
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Give the keyboard to a window.
pub fn set_foreground(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;
    unsafe {
        let _ = SetForegroundWindow(HWND(hwnd as *mut _));
    }
}

/// A window of `no_activate`'s kind may take the focus (`true`) or never does again (`false`).
pub fn can_activate(hwnd: isize, on: bool) {
    use windows::Win32::UI::WindowsAndMessaging::WS_EX_NOACTIVATE;
    unsafe {
        let h = HWND(hwnd as *mut _);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let want = if on { ex & !(WS_EX_NOACTIVATE.0 as isize) } else { ex | WS_EX_NOACTIVATE.0 as isize };
        if want != ex {
            SetWindowLongPtrW(h, GWL_EXSTYLE, want);
        }
    }
}

/// Make a window one that never takes the focus when it is shown or clicked, and has no taskbar button.
pub fn no_activate(hwnd: isize) {
    use windows::Win32::UI::WindowsAndMessaging::{WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW};
    unsafe {
        let h = HWND(hwnd as *mut _);
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let want = ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0 | WS_EX_LAYERED.0) as isize;
        if want != ex {
            SetWindowLongPtrW(h, GWL_EXSTYLE, want);
        }
    }
}
