//! The network, for its widget: the Wi-Fi networks in reach (which one is joined, how strong each is, which are known), the
//! Bluetooth devices paired (which are connected), and whether the PC is online at all. Looked at on a thread of its own every few
//! seconds; joining a known network, or leaving one, from here.

use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::NetworkManagement::WiFi::*;

pub const NET: &str = "net";

const EVERY: Duration = Duration::from_secs(4);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub ssid: String,
    /// 0..100
    pub signal: u32,
    pub joined: bool,
    pub known: bool,
    pub secured: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paired {
    pub name: String,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub online: bool,
    /// has the PC Wi-Fi at all
    pub has_wifi: bool,
    pub wifi: Vec<Wifi>,
    pub paired: Vec<Paired>,
}

static NOW: Mutex<Option<State>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

pub fn start() {
    static STARTED: AtomicBool = AtomicBool::new(false);
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(|| {
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        // (the paired Bluetooth devices: asked every few looks, it is slow)
        let mut paired_now: Vec<Paired> = Vec::new();
        let mut looks = 0u32;
        loop {
            if looks % 5 == 0 {
                paired_now = paired();
            }
            looks = looks.wrapping_add(1);
            let (has_wifi, wifi) = networks();
            let s = State { online: online(), has_wifi, wifi, paired: paired_now.clone() };
            let mut now = NOW.lock().unwrap();
            if now.as_ref() != Some(&s) {
                *now = Some(s);
                DIRTY.store(true, Ordering::Relaxed);
            }
            drop(now);
            let (m, cv) = &WAKE;
            let g = m.lock().unwrap();
            let (mut g, _) = cv.wait_timeout_while(g, EVERY, |poked| !*poked).unwrap();
            *g = false;
        }
    });
}

pub fn poke() {
    let (m, cv) = &WAKE;
    *m.lock().unwrap() = true;
    cv.notify_one();
}

pub fn take() -> Option<State> {
    if !DIRTY.swap(false, Ordering::Relaxed) {
        return None;
    }
    NOW.lock().unwrap().clone()
}

fn online() -> bool {
    use windows::Win32::Networking::NetworkListManager::*;
    unsafe {
        let Ok(nlm) = windows::Win32::System::Com::CoCreateInstance::<_, INetworkListManager>(&NetworkListManager, None, windows::Win32::System::Com::CLSCTX_ALL) else { return true };
        nlm.IsConnectedToInternet().map(|b| b.as_bool()).unwrap_or(true)
    }
}

/// The Wi-Fi client, open for one ask; its first adapter.
struct Client {
    h: HANDLE,
    guid: Option<GUID>,
}

impl Client {
    fn open() -> Option<Client> {
        let mut ver = 0u32;
        let mut h = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut ver, &mut h) } != 0 {
            return None;
        }
        let mut list: *mut WLAN_INTERFACE_INFO_LIST = std::ptr::null_mut();
        let mut guid = None;
        unsafe {
            if WlanEnumInterfaces(h, None, &mut list) == 0 && !list.is_null() {
                if (*list).dwNumberOfItems > 0 {
                    guid = Some((*list).InterfaceInfo[0].InterfaceGuid);
                }
                WlanFreeMemory(list as *const _);
            }
        }
        Some(Client { h, guid })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        unsafe {
            WlanCloseHandle(self.h, None);
        }
    }
}

fn ssid_of(s: &DOT11_SSID) -> String {
    String::from_utf8_lossy(&s.ucSSID[..(s.uSSIDLength as usize).min(32)]).to_string()
}

/// The networks in reach: each name once (its strongest), the joined first, then the strongest.
fn networks() -> (bool, Vec<Wifi>) {
    let Some(c) = Client::open() else { return (false, Vec::new()) };
    let Some(guid) = c.guid else { return (false, Vec::new()) };
    let mut out: Vec<Wifi> = Vec::new();
    unsafe {
        let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
        if WlanGetAvailableNetworkList(c.h, &guid, 0, None, &mut list) != 0 || list.is_null() {
            return (true, out);
        }
        let n = (*list).dwNumberOfItems as usize;
        let first = (*list).Network.as_ptr();
        for i in 0..n {
            let net = &*first.add(i);
            let ssid = ssid_of(&net.dot11Ssid);
            if ssid.is_empty() {
                continue;
            }
            let w = Wifi { ssid: ssid.clone(), signal: net.wlanSignalQuality, joined: net.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0, known: net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0, secured: net.bSecurityEnabled.as_bool() };
            match out.iter_mut().find(|x| x.ssid == ssid) {
                Some(x) => {
                    x.signal = x.signal.max(w.signal);
                    x.joined |= w.joined;
                    x.known |= w.known;
                }
                None => out.push(w),
            }
        }
        WlanFreeMemory(list as *const _);
    }
    out.sort_by(|a, b| b.joined.cmp(&a.joined).then(b.signal.cmp(&a.signal)));
    out.truncate(8);
    (true, out)
}

/// Join a network: a known one at once (its saved profile); else Windows' own list, to type its password.
pub fn join(ssid: &str) {
    let ssid = ssid.to_string();
    std::thread::spawn(move || {
        let known = (|| {
            let c = Client::open()?;
            let guid = c.guid?;
            unsafe {
                let mut list: *mut WLAN_AVAILABLE_NETWORK_LIST = std::ptr::null_mut();
                if WlanGetAvailableNetworkList(c.h, &guid, 0, None, &mut list) != 0 || list.is_null() {
                    return None;
                }
                let first = (*list).Network.as_ptr();
                let mut done = None;
                for i in 0..(*list).dwNumberOfItems as usize {
                    let net = &*first.add(i);
                    if ssid_of(&net.dot11Ssid) == ssid && net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0 {
                        let profile = String::from_utf16_lossy(&net.strProfileName[..net.strProfileName.iter().position(|c| *c == 0).unwrap_or(0)]);
                        let wide = HSTRING::from(profile);
                        let params = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: PCWSTR(wide.as_ptr()), pDot11Ssid: std::ptr::null_mut(), pDesiredBssidList: std::ptr::null_mut(), dot11BssType: net.dot11BssType, dwFlags: 0 };
                        done = Some(WlanConnect(c.h, &guid, &params, None) == 0);
                        break;
                    }
                }
                WlanFreeMemory(list as *const _);
                done
            }
        })();
        if known != Some(true) {
            crate::apps::launch_uri("ms-availablenetworks:");
        }
        std::thread::sleep(Duration::from_millis(1500));
        poke();
    });
}

/// The Bluetooth devices paired with the PC, and whether each is connected now.
fn paired() -> Vec<Paired> {
    use windows::Devices::Enumeration::{DeviceInformation, DeviceInformationKind};
    use windows::Foundation::Collections::IIterable;
    let go = || -> windows::core::Result<Vec<Paired>> {
        // (Bluetooth and Bluetooth LE devices that are paired)
        let aqs = HSTRING::from("(System.Devices.Aep.ProtocolId:=\"{e0cbf06c-cd8b-4647-bb8a-263b43f0f974}\" OR System.Devices.Aep.ProtocolId:=\"{bb7bb05e-5972-42b5-94fc-76eaa7084d49}\") AND System.Devices.Aep.IsPaired:=System.StructuredQueryType.Boolean#True");
        let props: IIterable<HSTRING> = vec![HSTRING::from("System.Devices.Aep.IsConnected")].try_into()?;
        let all = DeviceInformation::FindAllAsyncWithKindAqsFilterAndAdditionalProperties(&aqs, &props, DeviceInformationKind::AssociationEndpoint)?.get()?;
        let mut out = Vec::new();
        for d in all {
            let name = d.Name()?.to_string();
            if name.is_empty() || out.iter().any(|p: &Paired| p.name == name) {
                continue;
            }
            let connected = d.Properties()?.Lookup(&HSTRING::from("System.Devices.Aep.IsConnected")).ok().and_then(|v| windows::core::Interface::cast::<windows::Foundation::IReference<bool>>(&v).ok()).and_then(|r| r.Value().ok()).unwrap_or(false);
            out.push(Paired { name, connected });
        }
        out.sort_by(|a, b| b.connected.cmp(&a.connected).then_with(|| a.name.cmp(&b.name)));
        out.truncate(6);
        Ok(out)
    };
    go().unwrap_or_default()
}

fn wifi_icon(signal: u32) -> &'static str {
    match signal {
        75.. => "wifi",
        50.. => "wifi-3",
        25.. => "wifi-2",
        _ => "wifi-1",
    }
}

/// The widget: its icon how it is online (Wi-Fi's strength, a cable, none); open, the radios, the networks, the devices.
pub fn widget(s: &State, radios: crate::radios::States) -> Value {
    let joined = s.wifi.iter().find(|w| w.joined);
    let icon = if radios.wifi == Some(false) && radios.bluetooth != Some(true) && !s.online {
        "airplane"
    } else if let Some(w) = joined {
        wifi_icon(w.signal)
    } else if s.online {
        "globe"
    } else {
        "wifi-off"
    };
    let label = joined.map(|w| w.ssid.clone()).unwrap_or_else(|| if s.online { "Online".into() } else { "Offline".into() });
    let toggle = |id: &str, icon: &str, label: &str, on: Option<bool>| json!({ "icon": icon, "label": label, "action": format!("radio:{id}"), "tone": if on == Some(true) { "ok" } else { "" }, "tip": format!("{} {label}", if on == Some(true) { "Turn Off" } else { "Turn On" }) });
    let mut body = vec![json!({ "kind": "controls", "items": [
        toggle("wifi", "wifi", "Wi-Fi", radios.wifi),
        toggle("bluetooth", "bluetooth", "Bluetooth", radios.bluetooth),
        { "icon": "airplane", "label": "Airplane Mode", "action": "radio:airplane", "tip": "Airplane Mode" },
        { "icon": "gear", "label": "Settings", "action": "netsettings" },
    ] })];
    if radios.wifi != Some(false) && !s.wifi.is_empty() {
        let rows: Vec<Value> = s
            .wifi
            .iter()
            .map(|w| json!({ "icon": wifi_icon(w.signal), "title": w.ssid, "right": if w.joined { "Connected" } else if w.known { "Saved" } else { "" }, "action": if w.joined { String::new() } else { format!("wifi:{}", w.ssid) }, "lit": w.joined }))
            .collect();
        body.push(json!({ "kind": "list", "rows": rows }));
    }
    if radios.bluetooth != Some(false) && !s.paired.is_empty() {
        let rows: Vec<Value> = s
            .paired
            .iter()
            .map(|p| json!({ "icon": if crate::audiodev::is_headset(&p.name) { "headphones" } else { "bluetooth" }, "title": p.name, "right": if p.connected { "Connected" } else { "" }, "action": "btsettings", "lit": p.connected }))
            .collect();
        body.push(json!({ "kind": "list", "rows": rows }));
    }
    if !s.has_wifi && s.paired.is_empty() && radios.bluetooth.is_none() {
        return json!({ "peek": { "icon": icon, "label": label }, "tip": label });
    }
    json!({ "peek": { "icon": icon, "label": label }, "panel": { "body": body }, "tip": label })
}

#[cfg(test)]
mod tests {
    /// (on this PC: what it reads; changes nothing)
    #[test]
    #[ignore]
    fn reads_this_pc() {
        unsafe {
            let _ = windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED);
        }
        println!("online {}", super::online());
        println!("wifi {:?}", super::networks());
        println!("paired {:?}", super::paired());
        println!("out {:?}", crate::audiodev::list(windows::Win32::Media::Audio::eRender));
        println!("in {:?} mic {:?}", crate::audiodev::list(windows::Win32::Media::Audio::eCapture), crate::audiodev::mic_get());
    }
}
