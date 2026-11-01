//! The Windows key, pressed and let go alone, opens the launcher instead of Start (when it is set to). Combinations (Win+E, Win+D...)
//! are left alone. Start stays shut the way other tools keep it shut: the key's release is held back, a key that does nothing is
//! pressed in between, and the release is let through after it.

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_LAUNCHER: u32 = WM_APP + 3;
/// Win and a number (1..9, in wParam): the dock's app of that number
pub const WM_DOCK_KEY: u32 = WM_APP + 7;
/// a volume key pressed (the Volume Mixer bumps to show the new level)
pub const WM_VOLUME_KEY: u32 = WM_APP + 8;
/// Win + Ctrl + an arrow (wParam: 0 left, 1 up, 2 right, 3 down): the desktop that way on NADI's map
pub const WM_DESK_KEY: u32 = WM_APP + 11;

static DESK_KEYS: AtomicBool = AtomicBool::new(false);
/// Ctrl held, as the keyboard says (not as Windows thinks after NADI's own presses)
static CTRL_DOWN: AtomicBool = AtomicBool::new(false);

/// Do Win + Ctrl + arrows follow NADI's map of the desktops?
pub fn set_desk_keys(on: bool) {
    DESK_KEYS.store(on, Ordering::Relaxed);
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static OTHER: AtomicBool = AtomicBool::new(false);
static NOTIFY: AtomicIsize = AtomicIsize::new(0);
/// when Win went down (ms since the PC started), while it is
static WIN_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Win held, alone, for a moment (its numbers show on the dock).
pub fn win_held() -> bool {
    let at = WIN_AT.load(Ordering::Relaxed);
    WIN_DOWN.load(Ordering::Relaxed) && !OTHER.load(Ordering::Relaxed) && at > 0 && unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }.saturating_sub(at) > 450
}
/// a key no keyboard has (it does nothing)
const MASK: u16 = 0xE8;

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    }
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = k.flags.0 & LLKHF_INJECTED.0 != 0;
        let vk = k.vkCode as u16;
        let win = vk == VK_LWIN.0 || vk == VK_RWIN.0;
        let down = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if !injected && (vk == VK_LCONTROL.0 || vk == VK_RCONTROL.0 || vk == VK_CONTROL.0) {
            CTRL_DOWN.store(down, Ordering::Relaxed);
        }
        if !injected {
            // (the volume keys: let through, and told)
            if down && (vk == VK_VOLUME_UP.0 || vk == VK_VOLUME_DOWN.0 || vk == VK_VOLUME_MUTE.0) {
                let to = NOTIFY.load(Ordering::Relaxed);
                let _ = PostMessageW(HWND(to as *mut _), WM_VOLUME_KEY, WPARAM(0), LPARAM(0));
            }
            if down {
                if win {
                    if !WIN_DOWN.swap(true, Ordering::Relaxed) {
                        OTHER.store(false, Ordering::Relaxed);
                        WIN_AT.store(windows::Win32::System::SystemInformation::GetTickCount64(), Ordering::Relaxed);
                    }
                } else if WIN_DOWN.load(Ordering::Relaxed) {
                    OTHER.store(true, Ordering::Relaxed);
                    // (Win + Ctrl + an arrow: NADI's map, not Windows' line)
                    let ctrl = CTRL_DOWN.load(Ordering::Relaxed);
                    let dir = match vk {
                        x if x == VK_LEFT.0 => Some(0),
                        x if x == VK_UP.0 => Some(1),
                        x if x == VK_RIGHT.0 => Some(2),
                        x if x == VK_DOWN.0 => Some(3),
                        _ => None,
                    };
                    if let (true, Some(d)) = (ctrl && DESK_KEYS.load(Ordering::Relaxed), dir) {
                        let to = NOTIFY.load(Ordering::Relaxed);
                        let _ = PostMessageW(HWND(to as *mut _), WM_DESK_KEY, WPARAM(d), LPARAM(0));
                        return LRESULT(1);
                    }
                    // (Win and a number: the dock's app, not the taskbar's)
                    if ENABLED.load(Ordering::Relaxed) && (0x31..=0x39).contains(&vk) {
                        let to = NOTIFY.load(Ordering::Relaxed);
                        let _ = PostMessageW(HWND(to as *mut _), WM_DOCK_KEY, WPARAM((vk - 0x30) as usize), LPARAM(0));
                        return LRESULT(1);
                    }
                }
            } else if win {
                let alone = WIN_DOWN.swap(false, Ordering::Relaxed) && !OTHER.load(Ordering::Relaxed);
                if alone && ENABLED.load(Ordering::Relaxed) {
                    // (held back: the mask key, then the release itself)
                    let seq = [key(MASK, false), key(MASK, true), key(vk, true)];
                    SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
                    let to = NOTIFY.load(Ordering::Relaxed);
                    let _ = PostMessageW(HWND(to as *mut _), WM_LAUNCHER, WPARAM(0), LPARAM(0));
                    return LRESULT(1);
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// Listen to the keyboard on this thread (it must pump messages); the launcher's message goes to `notify`.
pub fn install(notify: HWND) {
    NOTIFY.store(notify.0 as isize, Ordering::Relaxed);
    unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), inst, 0);
    }
}
