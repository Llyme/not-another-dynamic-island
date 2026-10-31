//! A plugin's own program (its helper): one file in its folder, named by its manifest, which NADI starts when the plugin is switched on
//! and stops when it is switched off. NADI writes it lines of JSON (what the plugin asks it) on its input and reads lines of JSON from
//! its output (each becomes a `helper` event of the plugin). It runs in the plugin's folder, with `NADI_PLUGIN_DATA` naming a folder
//! of its own to keep what it needs (models, what it made).

use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// lines kept for the plugin at most (the oldest go)
const KEEP: usize = 512;

pub struct Helper {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Arc<Mutex<VecDeque<Value>>>,
    pub started: Instant,
}

impl Helper {
    /// Start `exe` (in `dir`, its plugin's folder), with `data` its own folder.
    pub fn start(exe: &Path, dir: &Path, data: &Path) -> std::io::Result<Helper> {
        let _ = std::fs::create_dir_all(data);
        let log = std::fs::File::create(data.join("helper.log")).map(Stdio::from).unwrap_or_else(|_| Stdio::null());
        let mut cmd = Command::new(exe);
        cmd.current_dir(dir).env("NADI_PLUGIN_DATA", data).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(log);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // (no window of its own)
            cmd.creation_flags(0x0800_0000);
        }
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take();
        let lines: Arc<Mutex<VecDeque<Value>>> = Arc::default();
        if let Some(out) = child.stdout.take() {
            let q = lines.clone();
            std::thread::Builder::new()
                .name("nadi-helper".into())
                .spawn(move || {
                    for line in BufReader::new(out).lines() {
                        let Ok(line) = line else { break };
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        // (a line that is not JSON: its words)
                        let v = serde_json::from_str::<Value>(line).unwrap_or_else(|_| json!({ "text": line.chars().take(2000).collect::<String>() }));
                        let mut q = q.lock().unwrap();
                        q.push_back(v);
                        while q.len() > KEEP {
                            q.pop_front();
                        }
                    }
                })
                .ok();
        }
        Ok(Helper { child, stdin, lines, started: Instant::now() })
    }

    /// Tell it something (a line of JSON). False: it is not listening.
    pub fn send(&mut self, v: &Value) -> bool {
        let Some(w) = self.stdin.as_mut() else { return false };
        let ok = writeln!(w, "{v}").and_then(|_| w.flush()).is_ok();
        if !ok {
            self.stdin = None;
        }
        ok
    }

    /// What it has said since the last look.
    pub fn take(&self) -> Vec<Value> {
        self.lines.lock().unwrap().drain(..).collect()
    }

    /// Still running?
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Helper {
    fn drop(&mut self) {
        // (asked to stop first, then made to)
        let _ = self.send(&json!({ "cmd": "quit" }));
        self.stdin = None;
        let deadline = Instant::now() + std::time::Duration::from_millis(400);
        while Instant::now() < deadline {
            if !matches!(self.child.try_wait(), Ok(None)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
