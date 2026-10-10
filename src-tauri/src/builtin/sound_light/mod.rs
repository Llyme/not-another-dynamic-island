//! Sound light: the aura that dances to what plays (a plugin, see native.rs). The light itself is painted by the page
//! (`ui/plugins/sound-light.js`); this is the plugin's switch and its setting, and the telling of the page and of the
//! sound listener (see `look`).

use crate::native::{manifest_of, old_flag, old_value, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use serde_json::Value;

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("sound_light.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin (the sound used to be heard for both)
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    let mut values = serde_json::Map::new();
    if let Some(v) = old_value(old, "audio_bleed") {
        values.insert("ambient".into(), v);
    }
    Adopted { on: old_flag(old, "react_to_audio", default_on), values }
}

pub struct SoundLight;

impl Native for SoundLight {
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
