//! What you play with, and how full its battery is: the controllers on (XInput ones: their battery as Windows
//! tells it, or that they are wired), and the Bluetooth devices that tell their battery (a headset, a mouse, a keyboard). Read on a
//! thread of its own (Bluetooth is slow to ask); what is known is told.

use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Bluetooth asked again at most this often (controllers: every time; that is quick)
const BLUETOOTH_EVERY: Duration = Duration::from_secs(30);

static BLUETOOTH: Mutex<(Option<Instant>, Vec<Value>)> = Mutex::new((None, Vec::new()));

/// The controllers on: `{ slot, level (0..1, none when wired or not told), wired }`.
fn controllers() -> Vec<Value> {
    use windows::Win32::UI::Input::XboxController::*;
    let mut out = Vec::new();
    for slot in 0..4u32 {
        let mut st = XINPUT_STATE::default();
        if unsafe { XInputGetState(slot, &mut st) } != 0 {
            continue;
        }
        let mut b = XINPUT_BATTERY_INFORMATION::default();
        let told = unsafe { XInputGetBatteryInformation(slot, BATTERY_DEVTYPE_GAMEPAD, &mut b) } == 0;
        let wired = told && b.BatteryType == BATTERY_TYPE_WIRED;
        let level = if told && !wired && b.BatteryType != BATTERY_TYPE_UNKNOWN && b.BatteryType != BATTERY_TYPE_DISCONNECTED {
            json!(match b.BatteryLevel {
                BATTERY_LEVEL_EMPTY => 0.1,
                BATTERY_LEVEL_LOW => 0.35,
                BATTERY_LEVEL_MEDIUM => 0.65,
                _ => 1.0,
            })
        } else {
            Value::Null
        };
        out.push(json!({ "slot": slot + 1, "level": level, "wired": wired }));
    }
    out
}

/// What a device is, from its name: a headset, a mouse, a keyboard, a controller, else something.
fn kind_of(name: &str) -> &'static str {
    let n = name.to_lowercase();
    if ["headphone", "headset", "earbud", "buds", "airpods", "earphone", "arctis", "hyperx", "kraken", "wh-", "wf-"].iter().any(|w| n.contains(w)) {
        "headset"
    } else if n.contains("mouse") || n.contains("mx master") || n.contains("mx anywhere") {
        "mouse"
    } else if n.contains("keyboard") || n.contains("keys") {
        "keyboard"
    } else if n.contains("controller") || n.contains("gamepad") || n.contains("dualsense") || n.contains("dualshock") || n.contains("pro controller") {
        "controller"
    } else {
        "device"
    }
}

/// The Bluetooth devices that tell their battery (Windows keeps it as a property of the device).
fn read_bluetooth() -> Vec<Value> {
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
    use windows::Foundation::Collections::IIterable;
    const BATTERY: &str = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5} 2";
    let go = || -> windows::core::Result<Vec<Value>> {
        // (the devices there now, each with its battery when it tells one: the query cannot ask for that, so it is looked at here)
        let aqs = HSTRING::from("System.Devices.Present:=System.StructuredQueryType.Boolean#True");
        let props: IIterable<HSTRING> = vec![HSTRING::from(BATTERY)].try_into()?;
        let all = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(&aqs, &props, DeviceInformationKind::Device)?.get()?;
        let mut out: Vec<Value> = Vec::new();
        for d in all {
            let name = d.Name()?.to_string();
            let Ok(v) = d.Properties()?.Lookup(&HSTRING::from(BATTERY)) else { continue };
            let Ok(r) = windows::core::Interface::cast::<windows::Foundation::IReference<u8>>(&v) else { continue };
            let Ok(pct) = r.Value() else { continue };
            if name.is_empty() || out.iter().any(|o| o["name"] == name.as_str()) {
                continue;
            }
            out.push(json!({ "name": name, "level": (pct.min(100) as f64) / 100.0, "kind": kind_of(&name) }));
        }
        Ok(out)
    };
    go().unwrap_or_default()
}

/// What is known now: `{ controllers: [{ slot, level, wired }], devices: [{ name, level, kind }] }`. Bluetooth is asked again,
/// on a thread, when what is known of it is old.
pub fn state() -> Value {
    let stale = {
        let mut b = BLUETOOTH.lock().unwrap();
        let stale = b.0.map_or(true, |t| t.elapsed() >= BLUETOOTH_EVERY);
        if stale {
            b.0 = Some(Instant::now());
        }
        stale
    };
    if stale {
        std::thread::spawn(|| {
            unsafe {
                let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
            }
            let list = read_bluetooth();
            BLUETOOTH.lock().unwrap().1 = list;
        });
    }
    let devices = BLUETOOTH.lock().unwrap().1.clone();
    json!({ "controllers": controllers(), "devices": devices })
}

#[cfg(test)]
mod tests {
    #[test]
    #[ignore]
    fn reads_this_pc() {
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        println!("controllers {:?}", super::controllers());
        println!("bluetooth {:?}", super::read_bluetooth());
    }

    #[test]
    fn names_say_what_a_device_is() {
        assert_eq!(super::kind_of("WH-1000XM4"), "headset");
        assert_eq!(super::kind_of("MX Master 3S"), "mouse");
        assert_eq!(super::kind_of("Keychron K2"), "device");
        assert_eq!(super::kind_of("DualSense Wireless Controller"), "controller");
    }
}

