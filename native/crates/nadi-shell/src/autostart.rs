//! Whether NADI starts when you sign in: its line in Windows' list of what starts then (this user's), pointing at this nadi.exe.

use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

const RUN: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Does NADI start at sign-in?
pub fn on() -> bool {
    let mut size = 0u32;
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN, w!("NADI"), RRF_RT_REG_SZ, None, None, Some(&mut size)) }.is_ok()
}

/// Start at sign-in (this nadi.exe), or not.
pub fn set(on: bool) {
    unsafe {
        if on {
            let Ok(me) = std::env::current_exe() else { return };
            let v: Vec<u16> = format!("\"{}\"", me.display()).encode_utf16().chain(std::iter::once(0)).collect();
            let _ = RegSetKeyValueW(HKEY_CURRENT_USER, RUN, w!("NADI"), REG_SZ.0, Some(v.as_ptr() as *const _), (v.len() * 2) as u32);
        } else {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, &HSTRING::from("NADI"));
        }
    }
}
