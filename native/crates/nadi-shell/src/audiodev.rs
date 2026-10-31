//! The sound's devices: the outputs and the microphones there are, which one is used, and making another the one used (as Windows'
//! own sound settings do, by its policy object); the microphone's volume.

use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub on: bool,
}

fn enumerator() -> Option<IMMDeviceEnumerator> {
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok() }
}

fn id_of(dev: &IMMDevice) -> Option<String> {
    unsafe {
        let p = dev.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// The devices of a kind (outputs: `eRender`; microphones: `eCapture`), the one used marked.
pub fn list(flow: EDataFlow) -> Vec<Device> {
    let Some(en) = enumerator() else { return Vec::new() };
    let mut out = Vec::new();
    unsafe {
        let now = en.GetDefaultAudioEndpoint(flow, eConsole).ok().and_then(|d| id_of(&d));
        let Ok(all) = en.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { return out };
        for i in 0..all.GetCount().unwrap_or(0) {
            let Ok(dev) = all.Item(i) else { continue };
            let Some(id) = id_of(&dev) else { continue };
            let name = dev.OpenPropertyStore(STGM_READ).ok().and_then(|ps| ps.GetValue(&PKEY_Device_FriendlyName).ok()).map(|v| v.to_string()).unwrap_or_default();
            out.push(Device { on: now.as_deref() == Some(id.as_str()), id, name: short_name(&name) });
        }
    }
    out
}

/// "Speakers (Realtek(R) Audio)": what is in the brackets says which, when there is one; else the whole.
fn short_name(name: &str) -> String {
    match (name.find('('), name.rfind(')')) {
        (Some(a), Some(b)) if b > a + 1 => {
            let kind = name[..a].trim();
            let inner = name[a + 1..b].trim();
            // (a headset's own name says more than "Headphones"; "Realtek(R) Audio" says less than "Speakers")
            if inner.contains("Audio") && !kind.is_empty() {
                kind.to_string()
            } else {
                inner.to_string()
            }
        }
        _ => name.to_string(),
    }
}

/// Make a device the one used (for everything: games, music, calls).
pub fn use_device(id: &str) {
    // (Windows' policy object, the one its sound settings use: not in its published headers)
    const CLSID_POLICY: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    const IID_POLICY: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    unsafe {
        let Ok(unk) = CoCreateInstance::<_, windows::core::IUnknown>(&CLSID_POLICY, None, CLSCTX_ALL) else { return };
        let mut p: *mut core::ffi::c_void = std::ptr::null_mut();
        if unk.query(&IID_POLICY, &mut p).is_err() || p.is_null() {
            return;
        }
        // (its 14th call, after IUnknown's three and ten of its own: SetDefaultEndpoint(id, role))
        let vtable = *(p as *const *const usize);
        let set: extern "system" fn(*mut core::ffi::c_void, PCWSTR, ERole) -> windows::core::HRESULT = std::mem::transmute(*vtable.add(13));
        let wide = windows::core::HSTRING::from(id);
        for role in [eConsole, eMultimedia, eCommunications] {
            let _ = set(p, PCWSTR(wide.as_ptr()), role);
        }
        let _ = windows::core::IUnknown::from_raw(p);
    }
}

fn mic() -> Option<IAudioEndpointVolume> {
    unsafe { enumerator()?.GetDefaultAudioEndpoint(eCapture, eConsole).ok()?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok() }
}

/// The microphone's volume (0..1, muted); none: no microphone.
pub fn mic_get() -> Option<(f32, bool)> {
    let v = mic()?;
    unsafe { Some((v.GetMasterVolumeLevelScalar().ok()?, v.GetMute().ok()?.as_bool())) }
}

pub fn mic_set(level: f32) {
    if let Some(v) = mic() {
        unsafe {
            let _ = v.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null());
            if level > 0.0 {
                let _ = v.SetMute(false, std::ptr::null());
            }
        }
    }
}

pub fn mic_mute() {
    if let Some(v) = mic() {
        unsafe {
            let m = v.GetMute().map(|b| b.as_bool()).unwrap_or(false);
            let _ = v.SetMute(!m, std::ptr::null());
        }
    }
}

/// Is it a headset (what its name says)?
pub fn is_headset(name: &str) -> bool {
    let n = name.to_lowercase();
    ["headphone", "headset", "earbud", "buds", "airpods", "earphone", "arctis", "hyperx", "kraken"].iter().any(|w| n.contains(w))
}
