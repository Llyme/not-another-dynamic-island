//! The sound's volume (the default output device's), read and set.

use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

fn endpoint() -> Option<IAudioEndpointVolume> {
    unsafe {
        let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
        let dev = e.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
        dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()
    }
}

thread_local! {
    /// the meters of the output and the microphone used now, and when they were last looked for (they may change)
    static METERS: std::cell::RefCell<(Option<IAudioMeterInformation>, Option<IAudioMeterInformation>, Option<std::time::Instant>)> = const { std::cell::RefCell::new((None, None, None)) };
}

/// How loud the output used now is, and the microphone (0..1 each, their peaks). The microphone reads nothing unless something
/// listens to it.
pub fn peaks() -> (f32, f32) {
    METERS.with(|m| {
        let mut m = m.borrow_mut();
        if m.2.is_none_or(|t| t.elapsed() > std::time::Duration::from_secs(2)) {
            let meter = |flow: EDataFlow| unsafe {
                let e: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
                e.GetDefaultAudioEndpoint(flow, eConsole).ok()?.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok()
            };
            *m = (meter(eRender), meter(eCapture), Some(std::time::Instant::now()));
        }
        let peak = |x: &Option<IAudioMeterInformation>| x.as_ref().and_then(|x| unsafe { x.GetPeakValue() }.ok()).unwrap_or(0.0);
        (peak(&m.0), peak(&m.1))
    })
}

/// (the volume 0..1, muted); none: no output device
pub fn get() -> Option<(f32, bool)> {
    let v = endpoint()?;
    unsafe { Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool())) }
}

pub fn set(level: f32) {
    if let Some(v) = endpoint() {
        unsafe {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            if level > 0.0 {
                let _ = v.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn toggle_mute() {
    if let Some(v) = endpoint() {
        unsafe {
            let m = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            let _ = v.SetMute(!m, std::ptr::null());
        }
    }
}
