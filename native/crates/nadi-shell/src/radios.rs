//! The PC's radios (Wi-Fi, Bluetooth): whether each is on, and turning one on or off. Windows answers in its own time, so each ask
//! runs on a thread of its own and what it learns is kept here for the next frame to draw.

use std::sync::Mutex;
use windows::Devices::Radios::{Radio, RadioKind, RadioState};

/// What is known of each radio (none: not known yet, or the PC has none).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct States {
    pub wifi: Option<bool>,
    pub bluetooth: Option<bool>,
}

static STATES: Mutex<States> = Mutex::new(States { wifi: None, bluetooth: None });

/// What is known now.
pub fn now() -> States {
    *STATES.lock().unwrap()
}

fn on_thread(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(move || {
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        f();
    });
}

fn read() -> windows::core::Result<States> {
    let mut st = States::default();
    for r in Radio::GetRadiosAsync()?.get()? {
        let on = r.State()? == RadioState::On;
        match r.Kind()? {
            RadioKind::WiFi => st.wifi = Some(on),
            RadioKind::Bluetooth => st.bluetooth = Some(on),
            _ => {}
        }
    }
    Ok(st)
}

/// Ask again what each radio is.
pub fn refresh() {
    on_thread(|| {
        if let Ok(s) = read() {
            *STATES.lock().unwrap() = s;
        }
    });
}

/// Turn a radio ("wifi" or "bluetooth") on or off (shown so at once; then asked again).
pub fn set(which: &'static str, on: bool) {
    {
        let mut s = STATES.lock().unwrap();
        match which {
            "wifi" => s.wifi = Some(on),
            _ => s.bluetooth = Some(on),
        }
    }
    on_thread(move || {
        let want = if which == "wifi" { RadioKind::WiFi } else { RadioKind::Bluetooth };
        let _ = (|| -> windows::core::Result<()> {
            let _ = Radio::RequestAccessAsync()?.get()?;
            for r in Radio::GetRadiosAsync()?.get()? {
                if r.Kind()? == want {
                    let _ = r.SetStateAsync(if on { RadioState::On } else { RadioState::Off })?.get();
                }
            }
            Ok(())
        })();
        if let Ok(s) = read() {
            *STATES.lock().unwrap() = s;
        }
    });
}
