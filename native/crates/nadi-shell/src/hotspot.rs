//! The mobile hotspot: whether the PC shares its internet connection over Wi-Fi, its network's name and password, and how many
//! devices are on it. Read with the network, on its thread; turned on or off on a thread of its own (it takes a few seconds).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::Networking::Connectivity::NetworkInformation;
use windows::Networking::NetworkOperators::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hotspot {
    pub on: bool,
    /// being turned on or off
    pub busy: bool,
    pub clients: u32,
    pub ssid: String,
    pub pass: String,
}

static BUSY: AtomicBool = AtomicBool::new(false);
/// what it was asked to be, while it is being turned
static WANT: Mutex<Option<bool>> = Mutex::new(None);

/// The manager of the connection to the internet's hotspot. None: no connection, or one that can't be shared.
fn manager() -> Option<NetworkOperatorTetheringManager> {
    let p = NetworkInformation::GetInternetConnectionProfile().ok()?;
    if NetworkOperatorTetheringManager::GetTetheringCapabilityFromConnectionProfile(&p).ok()? != TetheringCapability::Enabled {
        return None;
    }
    NetworkOperatorTetheringManager::CreateFromConnectionProfile(&p).ok()
}

/// The hotspot now. None: this PC can't share its connection now (offline, no Wi-Fi to share it over, or not allowed).
pub fn read() -> Option<Hotspot> {
    let m = manager()?;
    let st = m.TetheringOperationalState().ok()?;
    let busy = BUSY.load(Ordering::Relaxed);
    let on = match *WANT.lock().unwrap() {
        Some(w) if busy => w,
        _ => st == TetheringOperationalState::On,
    };
    let (ssid, pass) = m
        .GetCurrentAccessPointConfiguration()
        .ok()
        .map(|c| (c.Ssid().map(|s| s.to_string()).unwrap_or_default(), c.Passphrase().map(|s| s.to_string()).unwrap_or_default()))
        .unwrap_or_default();
    let clients = if st == TetheringOperationalState::On { m.ClientCount().unwrap_or(0) } else { 0 };
    Some(Hotspot { on, busy: busy || st == TetheringOperationalState::InTransition, clients, ssid, pass })
}

/// Turn it on or off (once at a time; a second ask while it turns is dropped).
pub fn set(on: bool) {
    if BUSY.swap(true, Ordering::Relaxed) {
        return;
    }
    *WANT.lock().unwrap() = Some(on);
    crate::net::poke();
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        if let Some(m) = manager() {
            let op = if on { m.StartTetheringAsync() } else { m.StopTetheringAsync() };
            if let Ok(op) = op {
                let _ = op.get();
            }
        }
        BUSY.store(false, Ordering::Relaxed);
        *WANT.lock().unwrap() = None;
        crate::net::poke();
    });
}
