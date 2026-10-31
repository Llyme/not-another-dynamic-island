//! What each program uses: its share of the processor and its memory, all its processes together (a browser is many). Looked at
//! when asked (about once a second): the processor's share is how much time its processes ran since the last look.

use std::collections::HashMap;
use std::time::Instant;
use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::ProcessStatus::{EnumProcesses, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};

static LATEST: std::sync::Mutex<Option<HashMap<String, Use>>> = std::sync::Mutex::new(None);

/// when it was last asked for (ms): not asked for a while (the dock hidden), it does not look
static WANTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Look once a second, on a thread of its own (it opens every process: not on the thread that draws), while it is asked for (each
/// call asks again); not asked for a few seconds, it rests.
pub fn start() {
    WANTED.store(nadi_core::paths::now_ms(), std::sync::atomic::Ordering::Relaxed);
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    let _ = std::thread::Builder::new().name("nadi-procstats".into()).spawn(|| {
        let mut st = Stats::new();
        loop {
            let asked = WANTED.load(std::sync::atomic::Ordering::Relaxed);
            if nadi_core::paths::now_ms().saturating_sub(asked) > 3000 {
                std::thread::sleep(std::time::Duration::from_millis(500));
                continue;
            }
            let u = st.sample();
            *LATEST.lock().unwrap() = Some(u);
            std::thread::sleep(std::time::Duration::from_millis(1000));
        }
    });
}

/// What was seen last (none yet: none).
pub fn latest() -> Option<HashMap<String, Use>> {
    LATEST.lock().unwrap().take()
}

/// One program's use: its share of the whole processor (0..1), and its memory (bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Use {
    pub cpu: f32,
    pub mem: u64,
}

pub struct Stats {
    /// each process's program (lower case) and when it began (a number used again is another process)
    names: HashMap<u32, (String, u64)>,
    /// each process's time on the processor at the last look (100 ns)
    times: HashMap<u32, u64>,
    at: Option<Instant>,
    cores: f32,
}

fn ft(f: FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

impl Stats {
    pub fn new() -> Stats {
        let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1) as f32;
        Stats { names: HashMap::new(), times: HashMap::new(), at: None, cores }
    }

    /// Look now: each program's use (by its path, lower case).
    pub fn sample(&mut self) -> HashMap<String, Use> {
        let mut out: HashMap<String, Use> = HashMap::new();
        let mut pids = vec![0u32; 4096];
        let mut got = 0u32;
        if unsafe { EnumProcesses(pids.as_mut_ptr(), (pids.len() * 4) as u32, &mut got) }.is_err() {
            return out;
        }
        pids.truncate(got as usize / 4);
        let now = Instant::now();
        let wall = self.at.map(|a| now.duration_since(a).as_secs_f64() * 1e7).unwrap_or(0.0);
        let mut times = HashMap::new();
        for pid in pids.into_iter().filter(|p| *p != 0) {
            let Ok(h) = (unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }) else { continue };
            let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
            if unsafe { GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u) }.is_err() {
                close(h);
                continue;
            }
            let began = ft(c);
            let name = match self.names.get(&pid) {
                Some((n, b)) if *b == began => n.clone(),
                _ => {
                    let n = image(h).unwrap_or_default();
                    self.names.insert(pid, (n.clone(), began));
                    n
                }
            };
            let ran = ft(k) + ft(u);
            let mut mem = PROCESS_MEMORY_COUNTERS_EX::default();
            let ok = unsafe { GetProcessMemoryInfo(h, &mut mem as *mut _ as *mut PROCESS_MEMORY_COUNTERS, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32) }.is_ok();
            close(h);
            if name.is_empty() {
                continue;
            }
            let entry = out.entry(name).or_default();
            if ok {
                entry.mem += mem.PrivateUsage as u64;
            }
            if let Some(before) = self.times.get(&pid).filter(|_| wall > 0.0) {
                entry.cpu += ((ran.saturating_sub(*before)) as f64 / wall / self.cores as f64) as f32;
            }
            times.insert(pid, ran);
        }
        self.names.retain(|p, _| times.contains_key(p));
        self.times = times;
        self.at = Some(now);
        for u in out.values_mut() {
            u.cpu = u.cpu.clamp(0.0, 1.0);
        }
        out
    }
}

fn close(h: HANDLE) {
    unsafe {
        let _ = CloseHandle(h);
    }
}

fn image(h: HANDLE) -> Option<String> {
    let mut buf = [0u16; 1024];
    let mut n = buf.len() as u32;
    unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n) }.ok()?;
    Some(String::from_utf16_lossy(&buf[..n as usize]).to_lowercase())
}

/// A process's program (its path, lower case).
pub fn program_of(pid: u32) -> Option<String> {
    let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let n = image(h);
    close(h);
    n
}

/// A window's program ended at once (End Task).
pub fn end_task(hwnd: isize) {
    use windows::Win32::System::Threading::{TerminateProcess, PROCESS_TERMINATE};
    let mut pid = 0u32;
    unsafe {
        windows::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(windows::Win32::Foundation::HWND(hwnd as *mut _), Some(&mut pid));
        if pid == 0 || pid == std::process::id() {
            return;
        }
        if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
            let _ = TerminateProcess(h, 1);
            close(h);
        }
    }
}

/// How much memory the PC has (bytes).
pub fn total_memory() -> u64 {
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
        m.ullTotalPhys
    } else {
        0
    }
}
