//! Floating cards: a hub card dragged out of the island becomes a card of its own on the screen.
//!
//! All floating cards live in one extra window, a transparent layer over the whole desktop (`floats`). It runs
//! the same page as the island (`index.html?floats=1`), which draws only the floating cards. The window is not
//! clipped (a clip trails the cards when they move and shows as a pale smear); instead it is click-through
//! except while the cursor is over a card (see `watch_cursor`), so every other click goes to what is underneath,
//! the game included. The window never takes focus, so using a card does not pull the game out of focus.
//!
//! One extra window however many cards float, and it is created only when the first card is dragged out. What floats
//! is not kept: after a restart every card is back in the island (a page that is open again is not a floating card).

use crate::winutil;
use crate::IslandState;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

const LABEL: &str = "floats";

/// A floating card: which card (its `key` in the hub) and where it is, in CSS px from the layer's top-left corner.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FloatItem {
    pub key: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    /// 0 = as tall as its content
    pub h: f64,
    /// 0 normal card, 1 translucent, 2 transparent (it follows what is behind it, see `float_backdrop`)
    #[serde(default)]
    pub mode: u8,
}

/// What the layer is told when a card is dragged out of the island (physical px from the layer's corner).
#[derive(Serialize, Clone, Debug)]
pub struct FloatBegin {
    pub key: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub grab_x: f64,
    pub grab_y: f64,
}

#[derive(Default)]
pub struct FloatState {
    items: Mutex<Vec<FloatItem>>,
    loaded: AtomicBool,
    /// the layer's page has loaded and listens
    ready: AtomicBool,
    /// a drag-out that arrived before the layer had loaded
    pending: Mutex<Option<FloatBegin>>,
    /// where the cards are (physical px from the layer's corner) and their corner radius
    rects: Mutex<(Vec<[f64; 4]>, f64)>,
    watching: AtomicBool,
    /// the window that had the keyboard before a card's text field took it (see `float_typing`)
    typing_from: AtomicIsize,
}

fn path() -> Option<std::path::PathBuf> {
    Some(crate::settings::data_dir()?.join("floats.json"))
}

impl FloatState {
    /// Starts empty; a list left by an earlier version is removed.
    fn load(&self) {
        if self.loaded.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Some(p) = path() {
            let _ = std::fs::remove_file(p);
        }
    }

    fn save(&self) {}

    /// at least one card floats on the screen
    pub fn any(&self) -> bool {
        !self.items.lock().unwrap().is_empty()
    }

    fn list(&self) -> Vec<FloatItem> {
        self.load();
        self.items.lock().unwrap().clone()
    }
}

/// The desktop as one rectangle, over every monitor: (x, y, w, h) in physical px.
fn desktop() -> (i32, i32, i32, i32) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetSystemMetrics, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN,
        SM_YVIRTUALSCREEN,
    };
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
    }
}

fn hwnd_of(w: &WebviewWindow) -> isize {
    w.hwnd().map(|h| h.0 as isize).unwrap_or(0)
}

/// The layer's window, made when it is first needed. It covers the desktop, never takes focus and has no
/// taskbar button; it stays hidden until it has a card to show.
fn ensure(app: &AppHandle) -> Option<WebviewWindow> {
    if let Some(w) = app.get_webview_window(LABEL) {
        return Some(w);
    }
    let w = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html?floats=1".into()))
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .skip_taskbar(true)
        .shadow(false)
        .resizable(false)
        .focused(false)
        .visible(false)
        .inner_size(400.0, 300.0)
        .build()
        .ok()?;
    crate::lock_down_webview(&w);
    let hwnd = hwnd_of(&w);
    winutil::no_activate(hwnd);
    let (x, y, cw, ch) = desktop();
    winutil::place(hwnd, x, y, cw, ch);
    winutil::click_through(hwnd, true);
    watch_cursor(app.clone());
    Some(w)
}

/// Whether a point (physical px from the layer's corner) is on a card with rounded corners.
fn on_card(rects: &[[f64; 4]], radius: f64, x: f64, y: f64) -> bool {
    rects.iter().any(|r| {
        if x < r[0] || x > r[2] || y < r[1] || y > r[3] {
            return false;
        }
        // (the corner: inside the circle of the radius, or not)
        let cx = x.clamp(r[0] + radius, r[2] - radius);
        let cy = y.clamp(r[1] + radius, r[3] - radius);
        (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius
    })
}

/// The layer is click-through everywhere but over a card: this follows the cursor and flips that. While the
/// button is down nothing changes, so a card that was grabbed keeps the mouse however far it is dragged, and a
/// press that began in the game stays the game's.
fn watch_cursor(app: AppHandle) {
    let state = app.state::<Arc<IslandState>>().inner().clone();
    if state.floats.watching.swap(true, Ordering::Relaxed) {
        return;
    }
    std::thread::spawn(move || {
        let mut through = true;
        loop {
            std::thread::sleep(Duration::from_millis(8));
            let Some(w) = app.get_webview_window(LABEL) else {
                continue;
            };
            if winutil::left_button_down() {
                continue;
            }
            let (ox, oy, _, _) = desktop();
            let (cx, cy) = winutil::cursor_pos();
            let on = {
                let g = state.floats.rects.lock().unwrap();
                on_card(&g.0, g.1, (cx - ox) as f64, (cy - oy) as f64)
            };
            // Over a card: it takes the mouse, unless the fullscreen guard is on and a fullscreen app covers this
            // monitor (then the cards yield to it, like the island does; its key lifts the guard, see `fullscreen_guard_on`).
            let want = !on
                || (state.fullscreen_guard_on()
                    && winutil::monitor_geometry_at(cx, cy).map_or(false, |geo| winutil::foreground_fullscreen_on(geo, 0)));
            if want != through {
                through = want;
                winutil::click_through(hwnd_of(&w), want);
            }
        }
    });
}

/// A card is dragged out of the island: show it on the layer under the cursor, and follow the cursor for it
/// until the button comes up (the island's page cannot hand over the mouse itself). `x`, `y`, `w`, `h` and the
/// grab point are CSS px in the island's window.
#[tauri::command]
pub async fn float_begin(
    app: AppHandle,
    window: WebviewWindow,
    key: String,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
    grab_x: f64,
    grab_y: f64,
) -> bool {
    let state = app.state::<Arc<IslandState>>().inner().clone();
    state.floats.load();
    let Some(layer) = ensure(&app) else {
        return false;
    };
    let Ok(pos) = window.outer_position() else {
        return false;
    };
    let k = window.scale_factor().unwrap_or(1.0);
    let (ox, oy, _, _) = desktop();
    let begin = FloatBegin {
        key: key.clone(),
        x: pos.x as f64 + x * k - ox as f64,
        y: pos.y as f64 + y * k - oy as f64,
        w: w * k,
        h: h * k,
        grab_x: grab_x * k,
        grab_y: grab_y * k,
    };
    {
        let mut items = state.floats.items.lock().unwrap();
        if !items.iter().any(|i| i.key == key) {
            items.push(FloatItem {
                key: key.clone(),
                x: begin.x / k,
                y: begin.y / k,
                w: (w).clamp(300.0, 480.0),
                h: 0.0,
                mode: 0,
            });
        }
    }
    if state.floats.ready.load(Ordering::Relaxed) {
        let _ = layer.emit("float-begin", begin);
    } else {
        *state.floats.pending.lock().unwrap() = Some(begin);
    }
    // the cursor, for as long as the button is held
    let pump = app.clone();
    std::thread::spawn(move || {
        // (the layer's page needs a moment to load the first time)
        for _ in 0..250 {
            if pump
                .state::<Arc<IslandState>>()
                .floats
                .ready
                .load(Ordering::Relaxed)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        loop {
            let (cx, cy) = winutil::cursor_pos();
            let down = winutil::left_button_down();
            if let Some(l) = pump.get_webview_window(LABEL) {
                let _ = l.emit(
                    "float-cursor",
                    serde_json::json!({ "x": cx - ox, "y": cy - oy, "down": down }),
                );
            }
            if !down {
                break;
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    });
    true
}

/// The layer's page has loaded: what floats, and a drag-out that was waiting for it.
#[tauri::command]
pub fn float_ready(state: tauri::State<'_, Arc<IslandState>>) -> serde_json::Value {
    state.floats.ready.store(true, Ordering::Relaxed);
    let pending = state.floats.pending.lock().unwrap().take();
    serde_json::json!({ "items": state.floats.list(), "begin": pending, "down": winutil::left_button_down() })
}

/// What floats right now (the island leaves those cards out of its list).
#[tauri::command]
pub fn float_list(state: tauri::State<'_, Arc<IslandState>>) -> Vec<FloatItem> {
    state.floats.list()
}

/// A floating card was moved or resized.
#[tauri::command]
pub fn float_set(state: tauri::State<'_, Arc<IslandState>>, item: FloatItem) {
    state.floats.load();
    {
        let mut items = state.floats.items.lock().unwrap();
        match items.iter_mut().find(|i| i.key == item.key) {
            Some(i) => *i = item,
            None => items.push(item),
        }
    }
    state.floats.save();
}

/// A card that floats has moved on with its tab (to the next section of a guide): it keeps its place, under its new key.
pub fn rekey(app: &AppHandle, state: &IslandState, old: &str, new: &str) {
    {
        let mut items = state.floats.items.lock().unwrap();
        if !items.iter().any(|i| i.key == old) {
            return;
        }
        items.retain(|i| i.key != new);
        if let Some(i) = items.iter_mut().find(|i| i.key == old) {
            i.key = new.to_string();
        }
    }
    if let Some(layer) = app.get_webview_window(LABEL) {
        let _ = layer.emit("float-rekey", serde_json::json!({ "old": old, "new": new }));
    }
}

/// A floating card goes back to the island.
#[tauri::command]
pub fn float_remove(state: tauri::State<'_, Arc<IslandState>>, key: String) {
    state.floats.load();
    state.floats.items.lock().unwrap().retain(|i| i.key != key);
    state.floats.save();
}

/// Where the cards are (physical px from the layer's corner): the layer is shown while there are any, and
/// takes the mouse only over them. An empty list hides it.
#[tauri::command]
pub fn float_region(app: AppHandle, state: tauri::State<'_, Arc<IslandState>>, rects: Vec<[f64; 4]>, radius: f64) {
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let hwnd = hwnd_of(&w);
    let empty = rects.is_empty();
    *state.floats.rects.lock().unwrap() = (rects, radius);
    if empty {
        let _ = w.hide();
    } else if !w.is_visible().unwrap_or(false) {
        let _ = w.show();
        winutil::raise(hwnd);
    }
}

/// A text field of a card has the focus (`on`) or has lost it. The layer never takes the focus, which is right for
/// moving and reading a card over a game, but a text field needs the keyboard: it is given to the layer while the
/// field is in use, and handed back after (unless you went to another window meanwhile).
#[tauri::command]
pub fn float_typing(app: AppHandle, state: tauri::State<'_, Arc<IslandState>>, on: bool) {
    let Some(w) = app.get_webview_window(LABEL) else {
        return;
    };
    let hwnd = hwnd_of(&w);
    if on {
        let prev = winutil::foreground();
        if prev != hwnd {
            state.floats.typing_from.store(prev, Ordering::Relaxed);
        }
        winutil::can_activate(hwnd, true);
        winutil::set_foreground(hwnd);
    } else {
        let ours = winutil::foreground() == hwnd;
        winutil::can_activate(hwnd, false);
        let prev = state.floats.typing_from.swap(0, Ordering::Relaxed);
        if ours && prev != 0 {
            winutil::set_foreground(prev);
        }
    }
}

/// How light the screen is behind a card (physical px from the layer's corner), 0 to 1, or nothing when it cannot be
/// told: a transparent card turns its text light or dark by it.
#[tauri::command]
pub fn float_backdrop(rect: [f64; 4]) -> Option<f32> {
    let (ox, oy, _, _) = desktop();
    winutil::screen_luma(
        rect[0].round() as i32 + ox,
        rect[1].round() as i32 + oy,
        rect[2].round() as i32 + ox,
        rect[3].round() as i32 + oy,
    )
}

/// The usable area of each monitor (physical px from the layer's corner): a floating card stays on one of them.
#[tauri::command]
pub fn float_monitors() -> Vec<[f64; 4]> {
    let (ox, oy, _, _) = desktop();
    winutil::monitor_work_areas()
        .into_iter()
        .map(|(l, t, r, b)| [(l - ox) as f64, (t - oy) as f64, (r - ox) as f64, (b - oy) as f64])
        .collect()
}

/// The island's rectangle (physical px from the layer's corner) while it is on screen: a card let go over it
/// goes back in.
#[tauri::command]
pub fn float_island(app: AppHandle, state: tauri::State<'_, Arc<IslandState>>) -> Option<[f64; 4]> {
    if !state.shown.load(Ordering::Relaxed) {
        return None;
    }
    let w = app.get_webview_window("island")?;
    let (p, s) = (w.outer_position().ok()?, w.outer_size().ok()?);
    let (ox, oy, _, _) = desktop();
    let pad = crate::GLOW_PAD * w.scale_factor().unwrap_or(1.0);
    Some([
        p.x as f64 - ox as f64 + pad,
        p.y as f64 - oy as f64 + pad,
        (p.x + s.width as i32) as f64 - ox as f64 - pad,
        (p.y + s.height as i32) as f64 - oy as f64 - pad,
    ])
}

/// At startup: nothing floats (see the top of this file); the leftover list of an earlier version is cleared.
pub fn spawn(_app: AppHandle, state: Arc<IslandState>) {
    state.floats.load();
}
