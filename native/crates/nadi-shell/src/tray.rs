//! The window nobody sees: it holds the icon in the tray (its menu: the plugins, on or off; the plugins' folder; exit), the clock of
//! the frame, and the knock of the plugins' thread when it has news.

use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_WAKE: u32 = WM_APP + 1;
pub const WM_TRAY: u32 = WM_APP + 2;
/// something was left to pin from File Explorer's menu
pub const WM_PINS: u32 = WM_APP + 12;

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_DESTROY {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    if let Some(r) = crate::events::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

pub fn window() -> Result<HWND> {
    unsafe {
        let inst = GetModuleHandleW(None)?;
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiTray"), ..Default::default() };
        RegisterClassW(&wc);
        CreateWindowExW(WS_EX_TOOLWINDOW, w!("NadiTray"), w!("NADI"), WS_POPUP, 0, 0, 0, 0, None, None, inst, None)
    }
}

fn data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut d = NOTIFYICONDATAW { cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32, hWnd: hwnd, uID: 1, ..Default::default() };
    d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = crate::logo::icon(unsafe { GetSystemMetrics(SM_CXSMICON) });
    let tip: Vec<u16> = "NADI".encode_utf16().collect();
    d.szTip[..tip.len()].copy_from_slice(&tip);
    d
}

pub fn add(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_ADD, &data(hwnd));
    }
}

pub fn remove(hwnd: HWND) {
    unsafe {
        let _ = Shell_NotifyIconW(NIM_DELETE, &data(hwnd));
    }
}
