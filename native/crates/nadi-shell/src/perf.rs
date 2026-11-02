//! What NADI costs the PC, for the Settings window's Performance page: its share of the processor, its memory and how long it spends
//! drawing, once a second for the last hour; and what each plugin and widget cost over the last minute (from what the plugins' host
//! and the frame count as they go).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::Instant;

/// seconds kept (an hour), and points sent for a graph at most
const KEEP: usize = 3600;
const POINTS: usize = 120;

/// What was counted so far: each plugin's (ms gathering, ms running, events), each widget's (times drawn, ms).
type Plugins = BTreeMap<String, (f64, f64, u64)>;
type Widgets = HashMap<String, (u64, f64)>;

pub struct Perf {
    /// each second: processor (% of the whole PC), memory (MB), drawing (ms that second)
    pub samples: VecDeque<[f32; 3]>,
    last: Option<(Instant, f64, f64)>,
    /// what was counted, every ten seconds, the last minute and a little
    snaps: VecDeque<(Instant, Plugins, Widgets)>,
}

impl Perf {
    pub fn new() -> Perf {
        Perf { samples: VecDeque::new(), last: None, snaps: VecDeque::new() }
    }

    /// Is a sample due (a second since the last)?
    pub fn due(&self) -> bool {
        self.last.map_or(true, |(t, _, _)| t.elapsed().as_secs_f32() >= 1.0)
    }

    /// Once a second (called often): a sample, from `busy_ms` (all the frame has spent so far) and what was counted. Was one taken?
    pub fn sample(&mut self, busy_ms: f64, plugins: &Plugins, widgets: &Widgets) -> bool {
        let now = Instant::now();
        let cpu = process_ms();
        let Some((at, cpu0, busy0)) = self.last else {
            self.last = Some((now, cpu, busy_ms));
            self.snaps.push_back((now, plugins.clone(), widgets.clone()));
            return false;
        };
        let secs = now.duration_since(at).as_secs_f64();
        if secs < 1.0 {
            return false;
        }
        let cores = std::thread::available_parallelism().map_or(1, |n| n.get()) as f64;
        let pct = ((cpu - cpu0) / (secs * 1000.0) / cores * 100.0).max(0.0) as f32;
        let frame = ((busy_ms - busy0) / secs) as f32;
        self.samples.push_back([pct, private_mb(), frame]);
        while self.samples.len() > KEEP {
            self.samples.pop_front();
        }
        self.last = Some((now, cpu, busy_ms));
        if self.snaps.back().map_or(true, |(t, _, _)| now.duration_since(*t).as_secs_f32() >= 10.0) {
            self.snaps.push_back((now, plugins.clone(), widgets.clone()));
            while self.snaps.len() > 8 {
                self.snaps.pop_front();
            }
        }
        true
    }

    /// The last `secs` seconds, as `POINTS` points at most (each the mean of those it stands for).
    pub fn series(&self, secs: usize) -> Vec<[f32; 3]> {
        let n = self.samples.len().min(secs);
        let from = self.samples.len() - n;
        let per = n.div_ceil(POINTS).max(1);
        let all: Vec<[f32; 3]> = self.samples.range(from..).copied().collect();
        all.chunks(per)
            .map(|c| {
                let k = c.len() as f32;
                [c.iter().map(|v| v[0]).sum::<f32>() / k, c.iter().map(|v| v[1]).sum::<f32>() / k, c.iter().map(|v| v[2]).sum::<f32>() / k]
            })
            .collect()
    }

    /// What each plugin and widget cost over the last minute (or since counting began), as so much a minute.
    pub fn minute(&self, plugins: &Plugins, widgets: &Widgets) -> (Plugins, Widgets) {
        let now = Instant::now();
        let Some((t, p0, w0)) = self.snaps.iter().find(|(t, _, _)| now.duration_since(*t).as_secs_f32() <= 65.0) else { return (Plugins::new(), Widgets::new()) };
        let secs = now.duration_since(*t).as_secs_f64().max(1.0);
        let k = 60.0 / secs;
        let p = plugins
            .iter()
            .map(|(id, (g, r, n))| {
                let (g0, r0, n0) = p0.get(id).copied().unwrap_or_default();
                (id.clone(), ((g - g0) * k, (r - r0) * k, (((n - n0.min(*n)) as f64) * k).round() as u64))
            })
            .collect();
        let w = widgets
            .iter()
            .map(|(id, (n, ms))| {
                let (n0, ms0) = w0.get(id).copied().unwrap_or_default();
                (id.clone(), ((((n - n0.min(*n)) as f64) * k).round() as u64, (ms - ms0) * k))
            })
            .collect();
        (p, w)
    }
}

/// All the processor time this process has had (ms).
fn process_ms() -> f64 {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
    let (mut a, mut b, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        if GetProcessTimes(GetCurrentProcess(), &mut a, &mut b, &mut k, &mut u).is_err() {
            return 0.0;
        }
    }
    let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
    t(k) + t(u)
}

/// The memory of NADI and of each program it started (its own, all together, MB): NADI itself, its settings window (its WebView, all
/// its processes), each plugin's helper (by the plugin's folder it runs from), its watchdog. Each: its name (or a plugin's id, as
/// "plugin:<id>"), MB.
pub fn memory_parts() -> Vec<(String, f32)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::*;
    let me = std::process::id();
    let mut procs: Vec<(u32, u32, String)> = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return Vec::new() };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let n = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
                procs.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase()));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    // (NADI's, and theirs, and theirs: every one it started)
    let mut mine: Vec<(u32, String, u32)> = Vec::new();
    let mut frontier = vec![me];
    while let Some(pid) = frontier.pop() {
        for (c, parent, exe) in &procs {
            if *parent == pid && *c != me && !mine.iter().any(|(m, _, _)| m == c) {
                mine.push((*c, exe.clone(), pid));
                frontier.push(*c);
            }
        }
    }
    let mb = |pid: u32| -> f32 {
        unsafe {
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid) else { return 0.0 };
            let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
            let ok = GetProcessMemoryInfo(h, &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb).is_ok();
            let _ = CloseHandle(h);
            if ok { (c.PrivateUsage as f64 / (1024.0 * 1024.0)) as f32 } else { 0.0 }
        }
    };
    // (a helper: the plugin whose folder it runs from)
    let plugin_of = |pid: u32| -> Option<String> {
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
            let mut buf = [0u16; 1024];
            let mut n = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
            let _ = CloseHandle(h);
            if !ok {
                return None;
            }
            let path = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..n as usize]));
            let plugins = nadi_core::paths::plugins_dir()?;
            let rel = path.strip_prefix(&plugins).ok()?;
            rel.components().next().map(|c| c.as_os_str().to_string_lossy().to_string())
        }
    };
    let mut parts: Vec<(String, f32)> = vec![("nadi".into(), private_mb())];
    let mut add = |name: String, v: f32| match parts.iter_mut().find(|(n, _)| *n == name) {
        Some(p) => p.1 += v,
        None => parts.push((name, v)),
    };
    for (pid, exe, _) in &mine {
        let v = mb(*pid);
        let name = if exe.starts_with("msedgewebview2") {
            "webview".to_string()
        } else if exe == "nadi.exe" {
            "watchdog".to_string()
        } else if let Some(id) = plugin_of(*pid) {
            format!("plugin:{id}")
        } else {
            exe.trim_end_matches(".exe").to_string()
        };
        add(name, v);
    }
    parts.sort_by(|a, b| b.1.total_cmp(&a.1));
    parts
}

/// This process's own memory (MB).
fn private_mb() -> f32 {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    let mut c = PROCESS_MEMORY_COUNTERS_EX { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32, ..Default::default() };
    unsafe {
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb).is_err() {
            return 0.0;
        }
    }
    (c.PrivateUsage as f64 / (1024.0 * 1024.0)) as f32
}
