//! The plugins written in Rust, inside the app (see native.rs). Each is a file here with its manifest beside it; this list
//! is the only place the core knows them, and the order they have in Settings > Plugins. (The plugins that are a module or
//! a declarative plugin, and that the app only carries, are in bundled.rs.)

use crate::native::Native;
use std::sync::Arc;
#[cfg(test)]
use {crate::native::Adopted, crate::plugins::Manifest, serde_json::Value};

pub mod claude_code;
pub mod downloads;
pub mod games;
pub mod now_playing;
pub mod pages;
pub mod toasts;
pub mod work;

/// The list of them, in the order they have in Settings > Plugins.
macro_rules! built_in {
    ($($m:ident => $make:expr),* $(,)?) => {
        /// the plugins, made
        pub fn all() -> Vec<Arc<dyn Native>> {
            vec![$(Arc::new($make) as Arc<dyn Native>),*]
        }

        /// their manifests alone (nothing is made or run)
        #[cfg(test)]
        pub fn manifests() -> Vec<Manifest> {
            vec![$($m::manifest()),*]
        }

        /// what each takes over from an old settings file (`null` for a new install), by plugin id
        #[cfg(test)]
        pub fn adopted(old: &Value) -> Vec<(String, Adopted)> {
            vec![$({
                let m = $m::manifest();
                let a = $m::adopted(old, m.default_on);
                (m.id, a)
            }),*]
        }
    };
}

built_in! {
    games => games::Games::default(),
    work => work::Work::default(),
    pages => pages::Pages::default(),
    downloads => downloads::Downloads::default(),
    claude_code => claude_code::ClaudeCode::default(),
    now_playing => now_playing::NowPlaying,
    toasts => toasts::Toasts,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // (these never make a plugin: a plugin reaches the whole of the window code, which a test program cannot load)

    #[test]
    fn every_built_in_has_a_valid_manifest() {
        let mut ids: Vec<String> = Vec::new();
        for m in manifests() {
            assert_eq!(m.kind, "native", "{}", m.id);
            assert!(!m.id.is_empty() && m.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'), "id {:?}", m.id);
            assert!(!m.name.is_empty() && !m.description.is_empty(), "{} needs a name and a description", m.id);
            assert!(!m.permissions.sees.is_empty(), "{} must say what it looks at", m.id);
            // a setting has a label, a known type and a default of that type
            for (k, s) in &m.settings {
                assert!(!s.label.is_empty(), "{}.{k} needs a label", m.id);
                match s.kind.as_str() {
                    "toggle" => assert!(s.default.is_boolean(), "{}.{k}", m.id),
                    "number" => assert!(s.default.is_number(), "{}.{k}", m.id),
                    "text" => assert!(s.default.is_string(), "{}.{k}", m.id),
                    "choice" => assert!(s.options.iter().any(|o| o.value == s.default), "{}.{k}: the default is not one of the options", m.id),
                    other => panic!("{}.{k}: unknown setting type {other}", m.id),
                }
            }
            ids.push(m.id);
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids are unique: {ids:?}");
    }

    #[test]
    fn every_permission_is_one_the_app_knows() {
        for m in manifests() {
            for x in &m.permissions.sees {
                assert!(crate::permissions::known(&x.name), "{} asks for {:?}, which the app does not know", m.id, x.name);
            }
        }
    }

    #[test]
    fn pills_are_named_once_and_have_a_size() {
        let mut seen: Vec<String> = Vec::new();
        for m in manifests() {
            for p in m.pills {
                assert!(p.w > 0.0 && p.h > 0.0, "{} has no size", p.id);
                assert!(!seen.contains(&p.id), "pill {} is offered by two plugins", p.id);
                seen.push(p.id);
            }
        }
        // the pills the page knows by name
        for name in ["game", "work", "page", "media", "usage_peek"] {
            assert!(seen.iter().any(|s| s == name), "no plugin offers the {name} pill");
        }
    }

    /// An install from before the plugins keeps what it had: the switches and the settings of the old file.
    #[test]
    fn an_old_settings_file_is_adopted() {
        let old = json!({
            "game_detection": false, "work_detection": true, "download_detection": false, "llm_detection": true, "llm_brief": false,
            "page_preview": false, "page_images": false
        });
        let by_id = |list: &Vec<(String, Adopted)>, id: &str| -> usize { list.iter().position(|(i, _)| i == id).unwrap() };
        let list = adopted(&old);
        let get = |id: &str| &list[by_id(&list, id)].1;
        assert!(!get("games").on);
        assert!(get("work").on);
        assert!(!get("downloads").on);
        assert!(get("claude-code").on && get("claude-code").values["alerts"] == json!(false));
        assert!(!get("page-reader").on && get("page-reader").values["images"] == json!(false));

        // a new install (no old file): what each plugin says it starts as
        let fresh = adopted(&Value::Null);
        for m in manifests() {
            let on = fresh.iter().find(|(i, _)| *i == m.id).unwrap().1.on;
            assert_eq!(on, m.default_on, "{}", m.id);
        }
    }
}
