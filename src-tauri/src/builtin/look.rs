//! What the eyes and the sound light share: the page is told which of them is on (and how far the light spills), and the
//! sound is only listened to while one of them wants it. They are two plugins (`eyes`, `sound_light`) and either may be
//! on without the other.

use crate::native::Ctx;
use crate::IslandState;
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Manager;

/// what the page draws by: the eyes on, whether they react to sound and make ripples, the sound light on, how bright
/// its lights may be and how far they spill (0..100)
pub fn state(state: &IslandState) -> Value {
    let p = &state.plugins;
    json!({
        "eyes": p.is_on_id("eyes"),
        "eyes_react": p.value("eyes", "react").as_bool().unwrap_or(true),
        "eyes_rings": p.value("eyes", "rings").as_bool().unwrap_or(true),
        "sound": p.is_on_id("sound-light"),
        "brightness": p.value("sound-light", "brightness").as_f64().unwrap_or(100.0).clamp(0.0, 100.0),
        "ambient": p.value("sound-light", "ambient").as_f64().unwrap_or(60.0).clamp(0.0, 100.0),
    })
}

/// One of the two was switched, or a setting of it changed: listen (or stop), and tell the page.
pub fn sync(ctx: &Ctx) {
    crate::plugins::sync_audio(&ctx.state);
    ctx.emit("look-tick", state(&ctx.state));
}

#[tauri::command]
pub fn look_state(window: tauri::WebviewWindow) -> Value {
    state(&window.state::<Arc<IslandState>>())
}
