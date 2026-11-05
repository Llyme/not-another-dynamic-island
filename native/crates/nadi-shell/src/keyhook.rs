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

/// when the hook last heard Win let go (ms since the PC started)
static WIN_UP_AT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// the window in front before the last one (Start not counted) keeps its keys from NADI (a program run as administrator)
static PREV_HIDDEN: AtomicBool = AtomicBool::new(false);

/// Win held, alone, for a moment (its numbers show on the dock).
pub fn win_held() -> bool {
    let at = WIN_AT.load(Ordering::Relaxed);
    WIN_DOWN.load(Ordering::Relaxed) && !OTHER.load(Ordering::Relaxed) && at > 0 && unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }.saturating_sub(at) > 450
}
/// a key no keyboard has (it does nothing)
const MASK: u16 = 0xE8;
/// what NADI's own key presses carry: the hook lets them by (others' made-up presses, a laptop's own keyboard program's, are heard)
pub const TAG: usize = 0x4E414449;

pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// Keys pressed together, as NADI's own (its hook lets them through): each down in order, then up the other way round.
pub fn chord(vks: &[u16]) {
    let mut ins: Vec<INPUT> = vks.iter().map(|v| key(*v, false)).collect();
    ins.extend(vks.iter().rev().map(|v| key(*v, true)));
    unsafe {
        let _ = SendInput(&ins, std::mem::size_of::<INPUT>() as i32);
    }
}

fn key(vk: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(vk), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: TAG } },
    }
}

unsafe extern "system" fn hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let injected = k.dwExtraInfo == TAG;
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
                WIN_UP_AT.store(windows::Win32::System::SystemInformation::GetTickCount64(), Ordering::Relaxed);
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

/// The program a window belongs to (its file name, small letters).
fn program_of(h: HWND) -> String {
    use windows::Win32::System::Threading::*;
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = CloseHandle(p);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..n as usize]).to_ascii_lowercase();
        path.rsplit('\\').next().unwrap_or("").to_string()
    }
}

/// Is this program Start's? (Windows 11's own, or, on newer builds, Start and search in one: SearchHost)
fn starts(program: &str) -> bool {
    matches!(program, "startmenuexperiencehost.exe" | "searchhost.exe")
}

/// Is this window Start?
fn is_start(h: HWND) -> bool {
    starts(&program_of(h))
}

/// Windows' own shell (the taskbar, the desktop, Start, search, quick settings): passed through on the way to Start, not counted as
/// what was in front.
fn is_shell(program: &str) -> bool {
    matches!(program, "explorer.exe" | "startmenuexperiencehost.exe" | "searchhost.exe" | "shellhost.exe" | "shellexperiencehost.exe" | "textinputhost.exe")
}

/// (what happened each time Start opened, in `winkey.log` in NADI's data folder, kept short: to see why Start was let be)
fn log(line: &str) {
    use std::io::Write;
    let Some(dir) = nadi_core::paths::data_dir() else { return };
    let p = dir.join("winkey.log");
    if std::fs::metadata(&p).map_or(0, |m| m.len()) > 32 * 1024 {
        let _ = std::fs::remove_file(&p);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{} {line}", chrono::Local::now().format("%m-%d %H:%M:%S"));
    }
}

/// the program in front before Start (Windows' shell not counted)
static PREV_PROGRAM: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// Is the mouse on a taskbar (Start pressed there, not with the key)?
fn over_taskbar() -> bool {
    unsafe {
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() {
            return false;
        }
        let root = GetAncestor(WindowFromPoint(p), GA_ROOT);
        let mut c = [0u16; 64];
        let n = GetClassNameW(root, &mut c) as usize;
        matches!(String::from_utf16_lossy(&c[..n]).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }
}

/// Another window came to the front. Over a program run as administrator Windows keeps the keyboard from NADI, so Win reaches Start
/// unheard: Start opening over one, with no Win heard, is shut again and the launcher opened in its place.
unsafe extern "system" fn foreground(_: windows::Win32::UI::Accessibility::HWINEVENTHOOK, _: u32, h: HWND, _: i32, _: i32, _: u32, _: u32) {
    if h.is_invalid() {
        return;
    }
    let program = program_of(h);
    if !starts(&program) {
        // (the taskbar or the desktop, on the way to Start: what was in front before them is kept)
        if !is_shell(&program) {
            PREV_HIDDEN.store(crate::winlist::keys_hidden(h), Ordering::Relaxed);
            *PREV_PROGRAM.lock().unwrap() = if program.is_empty() { "(hidden)".into() } else { program };
        }
        return;
    }
    let heard = windows::Win32::System::SystemInformation::GetTickCount64().saturating_sub(WIN_UP_AT.load(Ordering::Relaxed)) < 300;
    let (on, hidden, taskbar) = (ENABLED.load(Ordering::Relaxed), PREV_HIDDEN.load(Ordering::Relaxed), over_taskbar());
    let go = on && hidden && !heard && !taskbar;
    log(&format!("start over {} launcher-key {on} keys-hidden {hidden} win-heard {heard} on-taskbar {taskbar}: {}", PREV_PROGRAM.lock().unwrap(), if go { "shut, launcher" } else { "let be" }));
    if !go {
        return;
    }
    // (Esc shuts Start; once more if it is still there, then the launcher, so Start's closing does not take the front from it)
    std::thread::spawn(|| {
        let esc = [key(VK_ESCAPE.0, false), key(VK_ESCAPE.0, true)];
        for _ in 0..2 {
            let fg = GetForegroundWindow();
            if !is_start(fg) {
                break;
            }
            SendInput(&esc, std::mem::size_of::<INPUT>() as i32);
            std::thread::sleep(std::time::Duration::from_millis(120));
        }
        let to = NOTIFY.load(Ordering::Relaxed);
        let _ = PostMessageW(HWND(to as *mut _), WM_LAUNCHER, WPARAM(0), LPARAM(0));
    });
}

/// Listen to the keyboard, on a thread of its own that does nothing else: a hook on the window's thread, kept waiting while that
/// thread draws or Windows makes it wait, is dropped by Windows without a word, and the key stops opening the launcher. The launcher's
/// message goes to `notify`.
pub fn install(notify: HWND) {
    NOTIFY.store(notify.0 as isize, Ordering::Relaxed);
    std::thread::spawn(|| unsafe {
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), inst, 0);
        use windows::Win32::UI::Accessibility::SetWinEventHook;
        let _ = SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(foreground), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    });
}
