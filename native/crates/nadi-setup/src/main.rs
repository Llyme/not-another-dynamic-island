//! NADI's installer. Run, and said yes to (`--quiet`: without asking): NADI is put in `%LOCALAPPDATA%\Programs\NADI` for this user (a NADI already there is closed first and
//! replaced), with a Start menu shortcut, a start at sign-in (if wanted), and an entry in Windows' installed apps; then it is started. NADI's
//! settings and plugins (in `%APPDATA%\NADI`) are kept.
//!
//! `--uninstall` (what Windows' installed apps runs): asks, then takes it all away again but the settings and plugins.

#![windows_subsystem = "windows"]

use std::path::{Path, PathBuf};
use std::time::Duration;
use windows::core::{w, Interface, HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegDeleteTreeW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD, REG_SZ};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowThreadProcessId, MessageBoxW, PostMessageW, IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_YESNO, WM_CLOSE};

/// NADI itself, carried inside
static NADI: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/nadi.exe"));
const VERSION: &str = env!("CARGO_PKG_VERSION");
const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const UNINSTALL: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\NADI");

fn main() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("--uninstall") => ask_uninstall(),
        Some("--uninstall-now") => uninstall(&install_dir()),
        Some("--quiet") => install(runs_at_sign_in().unwrap_or(true)),
        _ => match ask_install() {
            Some(start) => install(start),
            None => Ok(()),
        },
    };
    if let Err(e) = r {
        unsafe {
            MessageBoxW(None, &HSTRING::from(e), w!("NADI"), MB_ICONERROR);
        }
    }
}

fn install_dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Programs").join("NADI")
}

fn start_menu_link() -> PathBuf {
    PathBuf::from(std::env::var_os("APPDATA").unwrap_or_default()).join("Microsoft\\Windows\\Start Menu\\Programs\\NADI.lnk")
}

/// Asked first: to install, or (one is there already) to update, which restarts it; and whether it starts when you sign in (ticked
/// at first; for an update, as it is now). None: not to.
fn ask_install() -> Option<bool> {
    use windows::Win32::UI::Controls::*;
    let there = install_dir().join("nadi.exe").exists();
    let ticked = if there { runs_at_sign_in().unwrap_or(false) } else { true };
    let main = HSTRING::from(if there { format!("Update NADI to {VERSION}?") } else { format!("Install NADI {VERSION}?") });
    let body = HSTRING::from("NADI restarts.");
    let go = HSTRING::from(if there { "Update" } else { "Install" });
    let buttons = [TASKDIALOG_BUTTON { nButtonID: 100, pszButtonText: PCWSTR(go.as_ptr()) }];
    let cfg = TASKDIALOGCONFIG {
        cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
        dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | if ticked { TDF_VERIFICATION_FLAG_CHECKED } else { TASKDIALOG_FLAGS(0) },
        dwCommonButtons: TDCBF_CANCEL_BUTTON,
        pszWindowTitle: w!("NADI"),
        pszMainInstruction: PCWSTR(main.as_ptr()),
        pszContent: if there { PCWSTR(body.as_ptr()) } else { PCWSTR::null() },
        cButtons: buttons.len() as u32,
        pButtons: buttons.as_ptr(),
        nDefaultButton: 100,
        pszVerificationText: w!("Start With Windows"),
        ..Default::default()
    };
    let (mut pressed, mut start) = (0i32, windows::Win32::Foundation::BOOL(0));
    unsafe { TaskDialogIndirect(&cfg, Some(&mut pressed), None, Some(&mut start)) }.ok()?;
    (pressed == 100).then_some(start.as_bool())
}

/// Does NADI start at sign-in now?
fn runs_at_sign_in() -> Option<bool> {
    use windows::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let mut size = 0u32;
    Some(unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, w!("NADI"), RRF_RT_REG_SZ, None, None, Some(&mut size)) }.is_ok())
}

fn install(start: bool) -> Result<(), String> {
    let dir = install_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not make {}: {e}", dir.display()))?;
    close_running();
    // NADI: written in place (one still held by Windows for a moment is moved aside first, and goes next time)
    let exe = dir.join("nadi.exe");
    for f in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        if f.file_name().to_string_lossy().starts_with("nadi.old-") {
            let _ = std::fs::remove_file(f.path());
        }
    }
    if std::fs::write(&exe, NADI).is_err() {
        let aside = dir.join(format!("nadi.old-{}.exe", std::process::id()));
        std::fs::rename(&exe, &aside).map_err(|e| format!("NADI is in use and could not be replaced: {e}"))?;
        std::fs::write(&exe, NADI).map_err(|e| format!("Could not write {}: {e}", exe.display()))?;
    }
    // the uninstaller: this installer, kept beside it
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let uninstaller = dir.join("uninstall.exe");
    if me != uninstaller {
        let _ = std::fs::copy(&me, &uninstaller);
    }
    shortcut(&start_menu_link(), &exe, &dir).map_err(|e| format!("Could not make the Start menu shortcut: {e}"))?;
    if start {
        set_string(RUN, "NADI", &format!("\"{}\"", exe.display()));
    } else {
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("NADI"));
        }
    }
    set_string(UNINSTALL, "DisplayName", "NADI");
    set_string(UNINSTALL, "DisplayVersion", VERSION);
    set_string(UNINSTALL, "Publisher", "NADI");
    set_string(UNINSTALL, "DisplayIcon", &exe.display().to_string());
    set_string(UNINSTALL, "InstallLocation", &dir.display().to_string());
    set_string(UNINSTALL, "UninstallString", &format!("\"{}\" --uninstall", uninstaller.display()));
    set_dword(UNINSTALL, "NoModify", 1);
    set_dword(UNINSTALL, "NoRepair", 1);
    set_dword(UNINSTALL, "EstimatedSize", (NADI.len() / 1024) as u32 + 1024);
    std::process::Command::new(&exe).current_dir(&dir).spawn().map_err(|e| format!("Installed, but NADI did not start: {e}"))?;
    Ok(())
}

fn ask_uninstall() -> Result<(), String> {
    let yes = unsafe { MessageBoxW(None, w!("Uninstall NADI?"), w!("NADI"), MB_YESNO | MB_ICONQUESTION) } == IDYES;
    if !yes {
        return Ok(());
    }
    // (from a copy elsewhere: this one is in the folder that goes)
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let copy = std::env::temp_dir().join(format!("nadi-uninstall-{}.exe", std::process::id()));
    std::fs::copy(&me, &copy).map_err(|e| e.to_string())?;
    std::process::Command::new(&copy).arg("--uninstall-now").spawn().map_err(|e| e.to_string())?;
    Ok(())
}

fn uninstall(dir: &Path) -> Result<(), String> {
    close_running();
    let _ = std::fs::remove_file(start_menu_link());
    unsafe {
        let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, w!("NADI"));
        let _ = RegDeleteTreeW(HKEY_CURRENT_USER, UNINSTALL);
        // (File Explorer's menu items, if they were on)
        for k in [w!("Software\\Classes\\*\\shell\\NADI.PinDock"), w!("Software\\Classes\\*\\shell\\NADI.PinApps"), w!("Software\\Classes\\Directory\\shell\\NADI.PinDock"), w!("Software\\Classes\\Directory\\shell\\NADI.PinApps"), w!("Software\\Classes\\*\\shell\\NADI"), w!("Software\\Classes\\Directory\\shell\\NADI"), w!("Software\\Classes\\lnkfile\\shell\\NADI")] {
            let _ = RegDeleteTreeW(HKEY_CURRENT_USER, k);
        }
    }
    // (the uninstaller that started this may still be ending: a few tries)
    for _ in 0..20 {
        if std::fs::remove_dir_all(dir).is_ok() || !dir.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // this copy goes once it has ended
    if let Ok(me) = std::env::current_exe() {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("cmd").args(["/c", "timeout", "/t", "2", ">nul", "&", "del", "/f", "/q"]).arg(&me).creation_flags(0x0800_0000).spawn();
    }
    Ok(())
}

/// A NADI that runs is asked to close (so it gives back what it holds: the taskbar, the tray), and waited for; ended if it does not.
fn close_running() {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("NadiTray"), w!("NADI")) else { return };
        if hwnd == HWND::default() {
            return;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        if pid == 0 {
            std::thread::sleep(Duration::from_millis(1500));
            return;
        }
        if let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_TERMINATE, false, pid) {
            if WaitForSingleObject(h, 6000).0 != 0 {
                let _ = TerminateProcess(h, 0);
                let _ = WaitForSingleObject(h, 2000);
            }
            let _ = CloseHandle(h);
        }
    }
}

fn shortcut(link: &Path, target: &Path, dir: &Path) -> windows::core::Result<()> {
    unsafe {
        let l: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
        l.SetPath(&HSTRING::from(target.as_os_str()))?;
        l.SetWorkingDirectory(&HSTRING::from(dir.as_os_str()))?;
        l.SetDescription(w!("NADI"))?;
        let f: IPersistFile = l.cast()?;
        f.Save(&HSTRING::from(link.as_os_str()), true)
    }
}

fn set_string(key: PCWSTR, name: &str, value: &str) {
    let v: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, &HSTRING::from(name), REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32);
    }
}

fn set_dword(key: PCWSTR, name: &str, value: u32) {
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, key, &HSTRING::from(name), REG_DWORD.0, Some(&value as *const u32 as *const _), 4);
    }
}
