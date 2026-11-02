//! NADI's Windows-key helper. Windows keeps the keyboard of an app run as administrator (most games with an anti-cheat) from programs
//! that are not, NADI among them: the Windows key pressed in one reaches Start. This helper runs as administrator, so it hears it: the
//! key pressed and let go alone over such an app, while NADI's launcher is to take it, is held back from Start and NADI is told to open
//! its launcher. Over any other app it does nothing (NADI hears the key itself there).
//!
//! It is set up once, from NADI's settings, with administrator rights: `--install <user>` copies it to `Program Files\NADI` (where only
//! an administrator can change it) and makes a task that starts it, as administrator, when that user signs in. `--remove` takes both
//! away again. Run with nothing: the helper itself (one at a time).

#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// NADI's: open the launcher (as its own keyboard hook says it)
const WM_LAUNCHER: u32 = WM_APP + 3;
/// a key no keyboard has (it does nothing): pressed between, so Windows does not take the key's release for Start
const MASK: u16 = 0xE8;
/// what NADI's own key presses carry (its hook lets them by; so do these)
const TAG: usize = 0x4E414449;
const TASK: &str = "NADI Keys";

static WIN_DOWN: AtomicBool = AtomicBool::new(false);
static OTHER: AtomicBool = AtomicBool::new(false);
/// whether NADI wants the key now (from its config), and when its config was last read
static WANTED: AtomicBool = AtomicBool::new(false);
static READ_AT: Mutex<Option<SystemTime>> = Mutex::new(None);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--install") => install(args.get(2).map(String::as_str).unwrap_or("")),
        Some("--remove") => remove(),
        _ => run(),
    }
}

fn program_files() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("NADI")
}

/// A command run hidden, waited for.
fn hidden(program: &str, args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    std::process::Command::new(program).args(args).creation_flags(0x0800_0000).status().is_ok_and(|s| s.success())
}

fn say(text: &str) {
    unsafe {
        MessageBoxW(None, &HSTRING::from(text), w!("NADI"), MB_ICONERROR);
    }
}

/// Copied to Program Files, a task made to start it at the user's sign-in (as administrator), and started now.
fn install(user: &str) {
    // (a name of letters, digits and a few marks only: it goes into a command)
    if user.is_empty() || !user.chars().all(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '_' | '-' | '\\' | '$')) {
        return say("The helper was not set up: no user named.");
    }
    let dir = program_files();
    let to = dir.join("nadi-keys.exe");
    // (one running from there: stopped first, so it can be written over)
    stop_running();
    let copied = std::env::current_exe().ok().is_some_and(|me| me == to || (std::fs::create_dir_all(&dir).is_ok() && std::fs::copy(&me, &to).is_ok()));
    if !copied {
        return say(&format!("The helper could not be copied to {}.", dir.display()));
    }
    let q = |s: &str| s.replace('\'', "''");
    let script = format!(
        "$a = New-ScheduledTaskAction -Execute '{exe}'; \
         $t = New-ScheduledTaskTrigger -AtLogOn -User '{user}'; \
         $p = New-ScheduledTaskPrincipal -UserId '{user}' -LogonType Interactive -RunLevel Highest; \
         $s = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries -ExecutionTimeLimit ([TimeSpan]::Zero) -MultipleInstances IgnoreNew; \
         Register-ScheduledTask -TaskName '{TASK}' -Action $a -Trigger $t -Principal $p -Settings $s -Force | Out-Null; \
         Start-ScheduledTask -TaskName '{TASK}'",
        exe = q(&to.display().to_string()),
        user = q(user),
    );
    if !hidden("powershell", &["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", &script]) {
        say("The helper was copied, but its task could not be made.");
    }
}

/// Its task and its copy taken away.
fn remove() {
    stop_running();
    let _ = hidden("schtasks", &["/Delete", "/TN", TASK, "/F"]);
    let dir = program_files();
    for _ in 0..10 {
        let _ = std::fs::remove_file(dir.join("nadi-keys.exe"));
        if !dir.join("nadi-keys.exe").exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let _ = std::fs::remove_dir(&dir);
}

/// The helper that runs, asked to end, and waited for a moment.
fn stop_running() {
    unsafe {
        if let Ok(h) = FindWindowW(w!("NadiKeys"), None) {
            if !h.is_invalid() {
                let _ = PostMessageW(h, WM_CLOSE, WPARAM(0), LPARAM(0));
                std::thread::sleep(std::time::Duration::from_millis(400));
            }
        }
    }
}

/// Does NADI want the key now? Its config read again when it changed: the launcher on the Windows key (`win_key` "launcher", or
/// "auto" with the taskbar hidden), and this helper not turned off (`admin_keys`).
fn wanted() -> bool {
    let Some(path) = std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("NADI").join("nadi-frame.json")) else { return false };
    let when = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    let mut at = READ_AT.lock().unwrap();
    if *at != when || when.is_none() {
        *at = when;
        let c: serde_json::Value = std::fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str(t.trim_start_matches('\u{feff}')).ok()).unwrap_or_default();
        let key = c["win_key"].as_str().unwrap_or("auto");
        let launcher = key == "launcher" || (key == "auto" && c["taskbar"] == "hide");
        WANTED.store(launcher && c["admin_keys"] == true, Ordering::Relaxed);
    }
    WANTED.load(Ordering::Relaxed)
}

/// Is the app in front one Windows keeps the keyboard of from NADI (run as administrator, or letting no one look at it)?
fn front_hidden_from_nadi() -> bool {
    use windows::Win32::Security::*;
    use windows::Win32::System::Threading::*;
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(fg, Some(&mut pid));
        if pid == 0 {
            return false;
        }
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return true };
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(p, TOKEN_QUERY, &mut token).is_ok();
        let _ = CloseHandle(p);
        if !opened {
            return true;
        }
        let mut e = TOKEN_ELEVATION::default();
        let mut n = 0u32;
        let ok = GetTokenInformation(token, TokenElevation, Some(&mut e as *mut _ as *mut _), std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut n).is_ok();
        let _ = CloseHandle(token);
        !ok || e.TokenIsElevated != 0
    }
}

fn nadi() -> Option<HWND> {
    unsafe { FindWindowW(w!("NadiTray"), w!("NADI")).ok().filter(|h| !h.is_invalid()) }
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
        let vk = k.vkCode as u16;
        let win = vk == VK_LWIN.0 || vk == VK_RWIN.0;
        let down = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if k.dwExtraInfo != TAG {
            if down {
                if win {
                    if !WIN_DOWN.swap(true, Ordering::Relaxed) {
                        OTHER.store(false, Ordering::Relaxed);
                        // (the config looked at now, while the key is down: not as it is let go)
                        let _ = wanted();
                    }
                } else if WIN_DOWN.load(Ordering::Relaxed) {
                    OTHER.store(true, Ordering::Relaxed);
                }
            } else if win {
                let alone = WIN_DOWN.swap(false, Ordering::Relaxed) && !OTHER.load(Ordering::Relaxed);
                if alone && WANTED.load(Ordering::Relaxed) && front_hidden_from_nadi() {
                    if let Some(h) = nadi() {
                        let seq = [key(MASK, false), key(MASK, true), key(vk, true)];
                        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
                        let _ = PostMessageW(h, WM_LAUNCHER, WPARAM(0), LPARAM(0));
                        return LRESULT(1);
                    }
                }
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_CLOSE || msg == WM_ENDSESSION {
        PostQuitMessage(0);
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

/// The helper: a hidden window (to be found, and asked to end), and the keyboard listened to.
fn run() {
    unsafe {
        if FindWindowW(w!("NadiKeys"), None).is_ok_and(|h| !h.is_invalid()) {
            return;
        }
        let inst = GetModuleHandleW(None).unwrap_or_default();
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiKeys"), ..Default::default() };
        RegisterClassW(&wc);
        let Ok(_hwnd) = CreateWindowExW(WINDOW_EX_STYLE(0), w!("NadiKeys"), w!("NADI Keys"), WS_POPUP, 0, 0, 0, 0, None, None, inst, None) else { return };
        let _ = wanted();
        let Ok(hk) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), inst, 0) else { return };
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = UnhookWindowsHookEx(hk);
    }
}
