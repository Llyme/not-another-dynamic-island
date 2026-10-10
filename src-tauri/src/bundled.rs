//! Plugins that ship inside the app, and are plugins like any other: a manifest, and for some a module. Nothing here is
//! special to the host. They go through the same checks, the same sandbox, the same switch and the same question
//! before they are switched on as one a user drops in a folder; they are only carried by the app (so an update brings the
//! new version) and cannot be deleted, only switched off.
//!
//! Their sources are in `plugins/` (a module: `time`, `agenda`) and `src-tauri/plugins-bundled/` (the manifest, and the
//! built `.wasm` that `plugins/build.ps1` copies there).

use crate::native::{old_flag, old_value, Adopted};
use serde_json::{json, Value};

pub struct Bundled {
    pub id: &'static str,
    pub manifest: &'static str,
    /// a module plugin: its file's name and bytes
    pub module: Option<(&'static str, &'static [u8])>,
}

pub const ALL: &[Bundled] = &[
    Bundled { id: "time", manifest: include_str!("../plugins-bundled/time/manifest.json"), module: Some(("time.wasm", include_bytes!("../plugins-bundled/time/time.wasm"))) },
    Bundled { id: "ics-calendar", manifest: include_str!("../plugins-bundled/ics-calendar/manifest.json"), module: None },
    Bundled { id: "agenda", manifest: include_str!("../plugins-bundled/agenda/manifest.json"), module: Some(("agenda.wasm", include_bytes!("../plugins-bundled/agenda/agenda.wasm"))) },
];

pub fn find(id: &str) -> Option<&'static Bundled> {
    ALL.iter().find(|b| b.id == id)
}

/// the steps the old time setting offered, in minutes (it kept the index)
const OLD_INTERVALS: [u64; 8] = [5, 10, 15, 30, 60, 120, 180, 360];

/// What a plugin takes over from the settings the app had before it was a plugin: its switch and its settings.
pub fn adopt(id: &str, old: &Value, default_on: bool) -> Adopted {
    let mut values = serde_json::Map::new();
    match id {
        "time" => {
            if let Some(i) = old_value(old, "time_interval").and_then(|v| v.as_u64()) {
                values.insert("every".into(), json!(OLD_INTERVALS[(i as usize).min(OLD_INTERVALS.len() - 1)]));
            }
            if let Some(v) = old_value(old, "time_24h") {
                values.insert("h24".into(), v);
            }
            Adopted { on: old_flag(old, "time_announce", default_on), values }
        }
        "ics-calendar" => {
            let url = old.get("calendar_ics_url").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            values.insert("url".into(), json!(url));
            if let Some(v) = old_value(old, "calendar_poll_min") {
                values.insert("poll_min".into(), v);
            }
            // (it was on when a link was set)
            Adopted { on: !url.is_empty() || default_on, values }
        }
        "agenda" => {
            if let Some(v) = old_value(old, "calendar_reminder_lead_min") {
                values.insert("lead_min".into(), v);
            }
            let had_link = old.get("calendar_ics_url").and_then(|v| v.as_str()).map_or(false, |u| !u.trim().is_empty());
            Adopted { on: had_link || default_on, values }
        }
        _ => Adopted { on: default_on, values },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugins::Manifest;

    #[test]
    fn every_bundled_manifest_reads() {
        let mut ids = Vec::new();
        for b in ALL {
            let m: Manifest = serde_json::from_str(b.manifest).unwrap_or_else(|e| panic!("{}: {e}", b.id));
            assert_eq!(m.id, b.id);
            assert!(!m.name.is_empty() && !m.description.is_empty(), "{} needs a name and a description", b.id);
            assert_ne!(m.kind, "native", "{} is a plugin like any other", b.id);
            if m.kind == "wasm" {
                let (name, bytes) = b.module.expect("a module plugin has its module");
                assert_eq!(m.module, name);
                assert!(bytes.starts_with(b"\0asm"), "{name} is not a module");
            } else {
                assert!(b.module.is_none());
            }
            ids.push(b.id);
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    /// An install from before the plugins keeps what it had: the switches and the settings of the old file.
    #[test]
    fn an_old_settings_file_is_adopted() {
        let old = json!({ "time_announce": true, "time_interval": 1, "time_24h": true, "calendar_ics_url": "https://example.com/a.ics", "calendar_poll_min": 30, "calendar_reminder_lead_min": 5 });
        let time = adopt("time", &old, false);
        assert!(time.on && time.values["every"] == json!(10) && time.values["h24"] == json!(true));
        let ics = adopt("ics-calendar", &old, false);
        assert!(ics.on);
        assert_eq!(ics.values["url"], json!("https://example.com/a.ics"));
        assert_eq!(ics.values["poll_min"], json!(30));
        let agenda = adopt("agenda", &old, false);
        assert!(agenda.on);
        assert_eq!(agenda.values["lead_min"], json!(5));
        // a new install, or no calendar link: they start as their manifest says
        for id in ["time", "ics-calendar", "agenda"] {
            assert!(!adopt(id, &Value::Null, false).on, "{id}");
            assert!(!adopt(id, &json!({}), false).on, "{id}");
        }
        assert!(!adopt("time", &json!({ "time_announce": false }), true).on);
    }
}
