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

/// How a network is joined: with no password, with one (its authentication and encryption as a Wi-Fi profile names them), or
/// another way (a work network that signs in, an old WEP one): Windows' own settings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Auth {
    Open,
    Key(&'static str, &'static str),
    #[default]
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wifi {
    pub ssid: String,
    /// its name's own bytes, in hex (a profile names it so, whatever they are)
    pub hex: String,
    pub auth: Auth,
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
    /// the mobile hotspot (None: the connection can't be shared now)
    pub hotspot: Option<crate::hotspot::Hotspot>,
}

static NOW: Mutex<Option<State>> = Mutex::new(None);
static DIRTY: AtomicBool = AtomicBool::new(false);
static WAKE: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());
/// looking again, asked for (Refresh pressed): the Wi-Fi scanning, the paired devices asked again; and while it is
static FRESH: AtomicBool = AtomicBool::new(false);
static SCANNING: AtomicBool = AtomicBool::new(false);

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
            if looks % 5 == 0 || FRESH.swap(false, Ordering::Relaxed) {
                paired_now = paired();
            }
            looks = looks.wrapping_add(1);
            let (has_wifi, wifi) = networks();
            let s = State { online: online(), has_wifi, wifi, paired: paired_now.clone(), hotspot: crate::hotspot::read() };
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

/// Look again, all of it (Refresh pressed): the Wi-Fi asked to scan the air afresh (what is new in reach shows a few seconds later),
/// the paired devices asked again.
pub fn refresh() {
    if SCANNING.swap(true, Ordering::Relaxed) {
        return;
    }
    // (its button lit at once)
    DIRTY.store(true, Ordering::Relaxed);
    poke();
    std::thread::spawn(|| {
        if let Some(c) = Client::open() {
            if let Some(guid) = c.guid {
                unsafe {
                    let _ = WlanScan(c.h, &guid, None, None, None);
                }
            }
        }
        // (a scan takes a few seconds: looked at then)
        std::thread::sleep(Duration::from_secs(4));
        FRESH.store(true, Ordering::Relaxed);
        SCANNING.store(false, Ordering::Relaxed);
        DIRTY.store(true, Ordering::Relaxed);
        poke();
    });
}

/// Looking again now (its button lit)?
pub fn refreshing() -> bool {
    SCANNING.load(Ordering::Relaxed)
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
            let raw = &net.dot11Ssid.ucSSID[..(net.dot11Ssid.uSSIDLength as usize).min(32)];
            let hex: String = raw.iter().map(|b| format!("{b:02X}")).collect();
            let (a, c) = (net.dot11DefaultAuthAlgorithm, net.dot11DefaultCipherAlgorithm);
            let enc = if c == DOT11_CIPHER_ALGO_TKIP { "TKIP" } else { "AES" };
            let auth = if a == DOT11_AUTH_ALGO_80211_OPEN && c == DOT11_CIPHER_ALGO_NONE {
                Auth::Open
            } else if a == DOT11_AUTH_ALGO_RSNA_PSK {
                Auth::Key("WPA2PSK", enc)
            } else if a == DOT11_AUTH_ALGO_WPA_PSK {
                Auth::Key("WPAPSK", enc)
            } else if a == DOT11_AUTH_ALGO_WPA3_SAE {
                Auth::Key("WPA3SAE", "AES")
            } else {
                Auth::Other
            };
            let w = Wifi { ssid: ssid.clone(), hex, auth, signal: net.wlanSignalQuality, joined: net.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0, known: net.dwFlags & WLAN_AVAILABLE_NETWORK_HAS_PROFILE != 0, secured: net.bSecurityEnabled.as_bool() };
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
        // (its profile would not do: Windows' Wi-Fi settings, which need no taskbar)
        if known != Some(true) {
            crate::apps::launch_uri("ms-settings:network-wifi");
        }
        std::thread::sleep(Duration::from_millis(1500));
        poke();
    });
}

/// A network being joined now, and one that would not join a moment ago (a wrong password, most likely).
static JOINING: Mutex<Option<String>> = Mutex::new(None);
static FAILED: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);
static STATUS: AtomicBool = AtomicBool::new(false);

/// Did a network begin or end being joined since this was last asked?
pub fn status_changed() -> bool {
    STATUS.swap(false, Ordering::Relaxed) || FAILED.lock().unwrap().as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_secs(10)) && {
        *FAILED.lock().unwrap() = None;
        true
    }
}

fn joining() -> Option<String> {
    JOINING.lock().unwrap().clone()
}

fn failed() -> Option<String> {
    FAILED.lock().unwrap().as_ref().filter(|(_, t)| t.elapsed() <= Duration::from_secs(10)).map(|(s, _)| s.clone())
}

fn xml(t: &str) -> String {
    t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

/// Join a network not joined before: its profile made (with its password, when it has one) and given to Windows, then joined. Not
/// joined within a few seconds: the profile is taken back (a wrong password is not kept), and the network says so for a moment.
pub fn join_new(w: &Wifi, pass: Option<String>) {
    let w = w.clone();
    *JOINING.lock().unwrap() = Some(w.ssid.clone());
    *FAILED.lock().unwrap() = None;
    STATUS.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        let security = match (w.auth, &pass) {
            (Auth::Key(auth, enc), Some(key)) => format!(
                "<security><authEncryption><authentication>{auth}</authentication><encryption>{enc}</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey></security>",
                xml(key)
            ),
            _ => "<security><authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption></security>".to_string(),
        };
        let name = xml(&w.ssid);
        let profile = format!(
            "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{name}</name><SSIDConfig><SSID><hex>{}</hex><name>{name}</name></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>auto</connectionMode><MSM>{security}</MSM></WLANProfile>",
            w.hex
        );
        let started = (|| {
            let c = Client::open()?;
            let guid = c.guid?;
            unsafe {
                let xmlw = HSTRING::from(profile.as_str());
                let mut reason = 0u32;
                if WlanSetProfile(c.h, &guid, 0, PCWSTR(xmlw.as_ptr()), None, true, None, &mut reason) != 0 {
                    return Some(false);
                }
                let pname = HSTRING::from(w.ssid.as_str());
                let params = WLAN_CONNECTION_PARAMETERS { wlanConnectionMode: wlan_connection_mode_profile, strProfile: PCWSTR(pname.as_ptr()), pDot11Ssid: std::ptr::null_mut(), pDesiredBssidList: std::ptr::null_mut(), dot11BssType: dot11_BSS_type_infrastructure, dwFlags: 0 };
                Some(WlanConnect(c.h, &guid, &params, None) == 0)
            }
        })();
        // (joined? looked at once a second, for a while)
        let mut joined = false;
        if started == Some(true) {
            for _ in 0..15 {
                std::thread::sleep(Duration::from_millis(1000));
                if networks().1.iter().any(|x| x.ssid == w.ssid && x.joined) {
                    joined = true;
                    break;
                }
            }
        }
        if !joined {
            if pass.is_some() || started == Some(false) {
                if let Some(c) = Client::open() {
                    if let Some(guid) = c.guid {
                        let pname = HSTRING::from(w.ssid.as_str());
                        unsafe {
                            WlanDeleteProfile(c.h, &guid, PCWSTR(pname.as_ptr()), None);
                        }
                    }
                }
            }
            *FAILED.lock().unwrap() = Some((w.ssid.clone(), std::time::Instant::now()));
        }
        *JOINING.lock().unwrap() = None;
        STATUS.store(true, Ordering::Relaxed);
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

/// How many of the four bars a signal (0..100) fills.
fn level(signal: u32) -> u32 {
    match signal {
        75.. => 4,
        50.. => 3,
        25.. => 2,
        _ => 1,
    }
}

/// How many bytes have come in and gone out on the PC's own network adapters so far (none: not read).
pub fn octets() -> Option<(u64, u64)> {
    use windows::Win32::NetworkManagement::IpHelper::*;
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t).is_err() || t.is_null() {
            return None;
        }
        let rows = std::slice::from_raw_parts((*t).Table.as_ptr(), (*t).NumEntries as usize);
        let (mut i, mut o) = (0u64, 0u64);
        for r in rows {
            // (a real adapter, up: the filters and virtual ones over it would count the same bytes again)
            let hardware = r.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if !hardware || r.OperStatus != windows::Win32::NetworkManagement::Ndis::IfOperStatusUp {
                continue;
            }
            i += r.InOctets;
            o += r.OutOctets;
        }
        FreeMibTable(t as *const _);
        Some((i, o))
    }
}

/// A speed (bytes a second), short.
pub fn rate_text(b: f64) -> String {
    if b >= 1_000_000.0 {
        format!("{:.1} MB/s", b / 1_000_000.0)
    } else {
        format!("{} KB/s", (b / 1000.0).round() as i64)
    }
}

/// The widget: at rest, four bars as strong as the Wi-Fi is (a cable: a plug; offline: the bars hollow, a red dot; airplane mode: a
/// plane). Opened: what it is connected to and how fast it moves now, Wi-Fi and Airplane, then the other networks in reach (a star
/// for a saved one, a lock for a secured one), and Windows' network settings. (Bluetooth is the Bluetooth widget's.)
pub fn widget(s: &State, radios: crate::radios::States, speed: Option<(f64, f64)>, asking: Option<&str>) -> Value {
    let joined = s.wifi.iter().find(|w| w.joined);
    let airplane = radios.wifi == Some(false) && radios.bluetooth != Some(true) && !s.online;
    let peek = if airplane {
        json!({ "icon": "airplane" })
    } else if let Some(w) = joined {
        json!({ "signal": { "level": level(w.signal) } })
    } else if s.online {
        json!({ "icon": "plug" })
    } else {
        json!({ "signal": { "level": 0, "off": true } })
    };
    let label = joined.map(|w| w.ssid.clone()).unwrap_or_else(|| if airplane { "Airplane Mode".into() } else if s.online { "Wired".into() } else { "Offline".into() });
    let mut peek = peek;
    peek["label"] = json!(label);
    // (what it is connected to)
    let lines = |sp: Option<(f64, f64)>| match sp {
        Some((d, u)) => json!([{ "text": format!("↓ {}", rate_text(d)), "color": "#5ac8e6" }, { "text": format!("↑ {}", rate_text(u)) }]),
        None => json!([]),
    };
    let hero = if airplane {
        json!({ "kind": "hero", "icon": "airplane", "title": "Airplane Mode", "sub": "Radios off" })
    } else if let Some(w) = joined {
        json!({ "kind": "hero", "signal": level(w.signal), "tone": "ok", "title": w.ssid, "sub": "Connected", "sub_icon": if w.secured { "lock" } else { "" }, "lines": lines(speed) })
    } else if s.online {
        json!({ "kind": "hero", "icon": "plug", "tone": "ok", "title": "Wired", "sub": "Connected", "lines": lines(speed) })
    } else {
        json!({ "kind": "hero", "icon": "x", "tone": "no", "title": "Offline", "sub": if s.wifi.is_empty() { "No networks in reach" } else { "Not connected" } })
    };
    let mut toggles = Vec::new();
    if s.has_wifi || radios.wifi.is_some() {
        toggles.push(json!({ "icon": "wifi", "label": "Wi-Fi", "on": radios.wifi == Some(true), "action": "radio:wifi" }));
    }
    toggles.push(json!({ "icon": "airplane", "label": "Airplane", "on": airplane, "action": "radio:airplane" }));
    if let Some(h) = &s.hotspot {
        toggles.push(json!({ "icon": "hotspot", "label": "Hotspot", "on": h.on, "action": "hotspot" }));
    }
    let mut body = vec![hero, json!({ "kind": "toggles", "items": toggles })];
    // (the hotspot on: its network, its password (pressed: copied), and who is on it)
    if let Some(h) = s.hotspot.as_ref().filter(|h| h.on) {
        let right = if h.busy {
            "Starting".to_string()
        } else {
            match h.clients {
                0 => "No Devices".to_string(),
                1 => "1 Device".to_string(),
                n => format!("{n} Devices"),
            }
        };
        body.push(json!({ "kind": "list", "rows": [{ "icon": "hotspot", "title": h.ssid, "sub": h.pass, "right": right, "action": "hotspotcopy" }] }));
    }
    let others: Vec<&Wifi> = s.wifi.iter().filter(|w| !w.joined).collect();
    // (the networks' heading, and beside it: look again (lit while it looks), and Windows' own settings)
    let listed = radios.wifi != Some(false) && !others.is_empty();
    body.push(json!({ "kind": "row", "items": [
        { "kind": "label", "text": if listed { "Networks" } else { "" } },
        { "kind": "space" },
        { "kind": "button", "icon": "refresh", "tip": "Refresh", "on": refreshing(), "action": "netrefresh" },
        { "kind": "button", "icon": "gear", "tip": "Network Settings", "action": "netsettings" },
    ] }));
    if listed {
        let (joining, failed) = (joining(), failed());
        let row = |w: &Wifi| {
            let mut marks = Vec::new();
            if w.known {
                marks.push("star");
            }
            if w.secured {
                marks.push("lock");
            }
            let right = if joining.as_deref() == Some(w.ssid.as_str()) { "Joining" } else if failed.as_deref() == Some(w.ssid.as_str()) { "Not Joined" } else { "" };
            json!({ "signal": level(w.signal), "title": w.ssid, "marks": marks, "right": right, "action": format!("wifi:{}", w.ssid) })
        };
        // (the one asking for its password: its field under its row)
        let mut rows: Vec<Value> = Vec::new();
        for w in &others {
            rows.push(row(w));
            if asking == Some(w.ssid.as_str()) {
                body.push(json!({ "kind": "list", "rows": std::mem::take(&mut rows) }));
                body.push(json!({ "kind": "field", "action": format!("wifipass:{}", w.ssid), "placeholder": "Password", "secret": true }));
            }
        }
        if !rows.is_empty() {
            body.push(json!({ "kind": "list", "rows": rows }));
        }
    }
    json!({ "peek": peek, "panel": { "width": 340, "body": body }, "tip": label })
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
