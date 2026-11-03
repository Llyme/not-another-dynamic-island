//! When NADI itself crashes (not a panic of its own, which the panic hook writes down, but a fault: a bad memory access in a graphics
//! driver it called, say): which module it was in and where, and what NADI was doing then, written to crash.log, and a small dump of
//! the process beside it (`crash-<time>.dmp`, the newest three kept) to look into later.

use std::cell::RefCell;
use std::io::Write;

thread_local! {
    /// what the frame is doing now (a widget being drawn, the liquid, the live pictures), for a crash to say
    static DOING: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Say what the frame is doing now (cheap: called each time it draws something).
pub fn doing(what: &str) {
    DOING.with(|d| {
        if let Ok(mut d) = d.try_borrow_mut() {
            d.clear();
            d.push_str(what);
        }
    });
}

/// Watch for crashes from now on.
pub fn watch() {
    use windows::Win32::System::Diagnostics::Debug::SetUnhandledExceptionFilter;
    unsafe {
        let _ = SetUnhandledExceptionFilter(Some(on_crash));
    }
}

unsafe extern "system" fn on_crash(info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) -> i32 {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT};
    let Some(dir) = nadi_core::paths::data_dir() else { return 0 };
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (code, addr) = if info.is_null() || (*info).ExceptionRecord.is_null() {
        (0u32, 0usize)
    } else {
        let r = &*(*info).ExceptionRecord;
        (r.ExceptionCode.0 as u32, r.ExceptionAddress as usize)
    };
    // (the module the fault was in, and how far into it)
    let mut module = HMODULE::default();
    let _ = GetModuleHandleExW(GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT, PCWSTR(addr as *const u16), &mut module);
    let mut buf = [0u16; 260];
    let n = GetModuleFileNameW(module, &mut buf) as usize;
    let path = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
    let name = path.rsplit(['\\', '/']).next().unwrap_or("?").to_string();
    let offset = addr.wrapping_sub(module.0 as usize);
    let doing = DOING.with(|d| d.try_borrow().map(|s| s.clone()).unwrap_or_default());
    let dump = format!("crash-{t}.dmp");
    let p = dir.join("crash.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
        let _ = writeln!(f, "{t} {} crashed: {code:#010x} in {name}+{offset:#x} while {} (dump: {dump})\n", env!("CARGO_PKG_VERSION"), if doing.is_empty() { "?" } else { &doing });
    }
    write_dump(&dir, &dump, info);
    // (the oldest dumps go: three kept)
    if let Ok(rd) = std::fs::read_dir(&dir) {
        let mut dumps: Vec<std::path::PathBuf> = rd.flatten().map(|e| e.path()).filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("crash-") && n.ends_with(".dmp"))).collect();
        dumps.sort();
        while dumps.len() > 3 {
            let _ = std::fs::remove_file(dumps.remove(0));
        }
    }
    // (on to Windows' own handling: the crash is reported as before)
    0
}

unsafe fn write_dump(dir: &std::path::Path, name: &str, info: *const windows::Win32::System::Diagnostics::Debug::EXCEPTION_POINTERS) {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Diagnostics::Debug::{MiniDumpNormal, MiniDumpWithIndirectlyReferencedMemory, MiniDumpWriteDump, MINIDUMP_EXCEPTION_INFORMATION};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentProcessId, GetCurrentThreadId};
    let Ok(file) = std::fs::File::create(dir.join(name)) else { return };
    let ex = MINIDUMP_EXCEPTION_INFORMATION { ThreadId: GetCurrentThreadId(), ExceptionPointers: info as *mut _, ClientPointers: false.into() };
    let _ = MiniDumpWriteDump(GetCurrentProcess(), GetCurrentProcessId(), HANDLE(file.as_raw_handle()), MiniDumpNormal | MiniDumpWithIndirectlyReferencedMemory, Some(&ex), None, None);
}
