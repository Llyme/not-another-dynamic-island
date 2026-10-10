//! Now playing: the pill and the hub's card for what is playing, with its controls (play, skip, seek, speed). A plugin
//! (see native.rs). What is playing is read by the island's media sensor (`media.rs`: Windows' media sessions), which
//! also gives the eyes their mood; this plugin only shows it and passes the controls on.

use crate::media;
use crate::native::{manifest_of, old_flag, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::time::Duration;

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("now_playing.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    Adopted { on: old_flag(old, "now_playing", default_on), values: Default::default() }
}

pub struct NowPlaying;

impl Native for NowPlaying {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        let ctx = ctx.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(300));
            let playing = ctx.on() && ctx.state.has_media.load(Ordering::Relaxed);
            ctx.offer_pill(playing.then_some("media"));
        });
    }

    fn call(&self, _ctx: &Ctx, cmd: &str, args: &Value) -> Result<Value, String> {
        let source = args.get("source").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(str::to_string);
        let num = |k: &str| args.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        match cmd {
            "play_pause" => media::media_play_pause(source),
            "next" => media::media_next(source),
            "previous" => media::media_previous(source),
            "seek" => media::media_seek(num("position_seconds"), source),
            "seek_by" => media::media_seek_by(num("delta_seconds"), source),
            "set_rate" => return Ok(json!(media::media_set_rate(num("rate"), source))),
            _ => return Err(format!("no such call: {cmd}")),
        }
        Ok(Value::Null)
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}
