//! The Windows key over apps run as administrator (most games): Windows keeps their keyboard from NADI, so the key reaches Start. A
//! small helper of NADI's (`nadi-keys.exe`, beside nadi.exe) can hear it there, run as administrator: set up once from the settings
//! (Windows asks for administrator rights), it is copied to `Program Files\NADI` and started at each sign-in. Turned off, it is left
//! there and does nothing (it reads `admin_keys` from the config).

use std::path::PathBuf;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, SW_HIDE};

/// Where the helper is set up (only an administrator can change it there).
fn installed_path() -> PathBuf {
    PathBuf::from(std::env::var_os("ProgramFiles").unwrap_or_else(|| "C:\\Program Files".into())).join("NADI").join("nadi-keys.exe")
}

/// The helper NADI carries (beside nadi.exe).
fn carried() -> Option<PathBuf> {
    let p = std::env::current_exe().ok()?.parent()?.join("nadi-keys.exe");
    p.exists().then_some(p)
}

/// Is it set up?
pub fn installed() -> bool {
    installed_path().exists()
}

/// Can it be set up (NADI carries it)?
pub fn available() -> bool {
    carried().is_some()
}

/// Run the helper with `args`, as administrator (Windows asks), and wait for it; then `notify` is told (the settings drawn again).
fn elevated(exe: PathBuf, args: String, notify: HWND) {
    let h = notify.0 as isize;
    std::thread::spawn(move || unsafe {
        let file = HSTRING::from(exe.as_os_str());
        let params = HSTRING::from(args);
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: w!("runas"),
            lpFile: windows::core::PCWSTR(file.as_ptr()),
            lpParameters: windows::core::PCWSTR(params.as_ptr()),
            nShow: SW_HIDE.0,
            ..Default::default()
        };
        if ShellExecuteExW(&mut info).is_ok() && !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
        let _ = PostMessageW(HWND(h as *mut _), crate::update::WM_UPDATE, WPARAM(0), LPARAM(0));
    });
}

/// Set it up (Windows asks for administrator rights), for this user.
pub fn install(notify: HWND) {
    let Some(exe) = carried() else { return };
    let user = match (std::env::var("USERDOMAIN"), std::env::var("USERNAME")) {
        (Ok(d), Ok(u)) if !d.is_empty() => format!("{d}\\{u}"),
        (_, Ok(u)) => u,
        _ => return,
    };
    elevated(exe, format!("--install \"{user}\""), notify);
}
