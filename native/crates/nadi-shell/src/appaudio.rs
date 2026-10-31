//! Which programs make sound, and how loud each is now: Windows' audio sessions on every output (a game may play on a headset that is
//! not the default). The sessions are looked for about once a second; how loud each is, as often as it is asked. Each program's own
//! volume, too (its sessions'), read and set.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
use windows::Win32::Media::Audio::{eRender, AudioSessionStateActive, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

/// a program that played lately keeps its volume in quick settings this long after it stops
const KEEP: Duration = Duration::from_secs(60);

/// A program's own volume, as quick settings shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct AppVolume {
    pub key: String,
    pub volume: f32,
    pub muted: bool,
}

pub struct AppAudio {
    /// each sounding session's meter, by its program (path, lower case)
    meters: Vec<(String, IAudioMeterInformation)>,
    /// each session's volume, by its program
    volumes: Vec<(String, ISimpleAudioVolume)>,
    /// when each program last played
    played: HashMap<String, Instant>,
    names: HashMap<u32, String>,
}

impl AppAudio {
    pub fn new() -> AppAudio {
        AppAudio { meters: Vec::new(), volumes: Vec::new(), played: HashMap::new(), names: HashMap::new() }
    }

    /// Look for the sessions that play now.
    pub fn refresh(&mut self) {
        let _ = self.sessions();
        let now = Instant::now();
        for (k, _) in &self.meters {
            self.played.insert(k.clone(), now);
        }
        self.played.retain(|_, t| t.elapsed() < KEEP);
    }

    fn sessions(&mut self) -> windows::core::Result<()> {
        let mut meters = Vec::new();
        let mut volumes = Vec::new();
        unsafe {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let devices = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
            for d in 0..devices.GetCount()? {
                let Ok(dev) = devices.Item(d) else { continue };
                let Ok(mgr) = dev.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { continue };
                let Ok(list) = mgr.GetSessionEnumerator() else { continue };
                for i in 0..list.GetCount().unwrap_or(0) {
                    let Ok(ctl) = list.GetSession(i) else { continue };
                    let state = ctl.GetState().ok();
                    if state.is_none() || state == Some(AudioSessionStateExpired) {
                        continue;
                    }
                    let Ok(c2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                    let pid = c2.GetProcessId().unwrap_or(0);
                    if pid == 0 || c2.IsSystemSoundsSession() == windows::Win32::Foundation::S_OK {
                        continue;
                    }
                    let name = match self.names.get(&pid) {
                        Some(n) => n.clone(),
                        None => {
                            let n = crate::procstats::program_of(pid).unwrap_or_default();
                            self.names.insert(pid, n.clone());
                            n
                        }
                    };
                    if name.is_empty() {
                        continue;
                    }
                    if let Ok(v) = ctl.cast::<ISimpleAudioVolume>() {
                        volumes.push((name.clone(), v));
                    }
                    if state == Some(AudioSessionStateActive) {
                        if let Ok(m) = ctl.cast::<IAudioMeterInformation>() {
                            meters.push((name, m));
                        }
                    }
                }
            }
        }
        if self.names.len() > 500 {
            self.names.clear();
        }
        self.meters = meters;
        self.volumes = volumes;
        Ok(())
    }

    /// How loud each program is now (0..1, its loudest session).
    pub fn peaks(&self) -> HashMap<String, f32> {
        let mut out: HashMap<String, f32> = HashMap::new();
        for (name, m) in &self.meters {
            let v = unsafe { m.GetPeakValue() }.unwrap_or(0.0);
            let e = out.entry(name.clone()).or_default();
            *e = e.max(v);
        }
        out
    }

    /// The programs that played lately, each with its own volume (its first session's), the latest first.
    pub fn apps(&self) -> Vec<AppVolume> {
        let mut keys: Vec<(&String, &Instant)> = self.played.iter().collect();
        keys.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
        keys.into_iter()
            .filter_map(|(k, _)| {
                let (_, v) = self.volumes.iter().find(|(n, _)| n == k)?;
                unsafe { Some(AppVolume { key: k.clone(), volume: v.GetMasterVolume().ok()?, muted: v.GetMute().ok()?.as_bool() }) }
            })
            .collect()
    }

    /// Set a program's volume (all its sessions).
    pub fn set_volume(&self, key: &str, level: f32) {
        for (_, v) in self.volumes.iter().filter(|(n, _)| n == key) {
            unsafe {
                let _ = v.SetMasterVolume(level.clamp(0.0, 1.0), std::ptr::null());
                if level > 0.0 {
                    let _ = v.SetMute(false, std::ptr::null());
                }
            }
        }
    }

    pub fn toggle_mute(&self, key: &str) {
        let muted = self.volumes.iter().find(|(n, _)| n == key).and_then(|(_, v)| unsafe { v.GetMute().ok() }).is_some_and(|m| m.as_bool());
        for (_, v) in self.volumes.iter().filter(|(n, _)| n == key) {
            unsafe {
                let _ = v.SetMute(!muted, std::ptr::null());
            }
        }
    }
}
