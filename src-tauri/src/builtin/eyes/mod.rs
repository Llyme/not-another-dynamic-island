//! Eyes: the two eyes of the island (a plugin, see native.rs). The eyes themselves are painted by the page
//! (`ui/plugins/eyes.js`); this is the plugin's switch and its setting, and the telling of the page and of the sound
//! listener (see `look`).

use crate::native::{manifest_of, old_flag, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use serde_json::Value;

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("eyes.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    let mut values = serde_json::Map::new();
    values.insert("react".into(), Value::Bool(old_flag(old, "react_to_audio", true)));
    Adopted { on: old_flag(old, "show_eyes", default_on), values }
}

pub struct Eyes;

impl Native for Eyes {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        super::look::sync(ctx);
    }

    fn switched(&self, ctx: &Ctx, _on: bool) {
        super::look::sync(ctx);
    }

    fn changed(&self, ctx: &Ctx, _key: &str) {
        super::look::sync(ctx);
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}
