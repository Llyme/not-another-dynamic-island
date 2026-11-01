//! A modal: one question, in a card in the middle of the screen it was asked on, over everything (full-screen apps too). It grows
//! out of its own middle; behind it the screen dims. It takes the keyboard: Enter presses the button
//! that has the focus, Tab (or the arrows) moves it, Esc or a press outside the card is Cancel. It can count down and press its last
//! button by itself (power), ask for words (a field, its letters hidden for a password), or just tell. One at a time: one asked
//! while one is out waits for it.
//!
//! What it asks is a `spec`: `{ icon, tone ("red" | "theme" | "amber"), title, text, field: { hint, secret, value }, countdown (s),
//! buttons: [{ label, action, tone }] }`. A button with no action is Cancel. What was pressed comes back as (owner, action, args), the
//! field's words in `args.text`; Cancel, for a plugin, as `modal:cancel`.

use crate::draw::{self, Painter, FACE};
use crate::gfx::Gfx;
use crate::paint::{fill_shape, Color, Weight};
use crate::shape::{self, Rect};
use nadi_core::pc::Pc;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// the card: how wide, its padding, how round
const CW: f32 = 300.0;
const PAD: f32 = 22.0;
const RADIUS: f32 = 26.0;
const BTN_H: f32 = 38.0;
const ICON: f32 = 52.0;
const FIELD_H: f32 = 40.0;

/// One asked, waiting for the one out.
struct Asked {
    spec: Value,
    owner: String,
    mon: RECT,
    s: f32,
    from: (f32, f32),
}

pub struct Modal {
    pub hwnd: HWND,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    surface: IDCompositionSurface,
    s: f32,
    /// the monitor it is over (physical px)
    mon: RECT,
    spec: Value,
    owner: String,
    field: String,
    focus: usize,
    /// how far out (0..1, springing past), and where it grows from (logical, in the monitor)
    t: f32,
    vel: f32,
    to: f32,
    from: (f32, f32),
    opened: Instant,
    pub shown: bool,
    result: Option<(String, String, Value)>,
    waiting: Vec<Asked>,
    /// the mouse (logical, in the monitor)
    mouse: (f32, f32),
    /// the keys moved the focus: the focused ghost shows its ground
    keyed: bool,
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_MOUSEACTIVATE {
        return LRESULT(MA_ACTIVATE as isize);
    }
    if let Some(r) = crate::events::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn inside(r: &Rect, x: f32, y: f32) -> bool {
    x >= r.x && x < r.x + r.w && y >= r.y && y < r.y + r.h
}

fn tone_of(t: &str) -> Color {
    match t {
        "red" | "no" => draw::RED,
        "amber" | "hot" => draw::AMBER,
        _ => draw::accent(),
    }
}

impl Modal {
    pub fn new(g: &Gfx) -> windows::core::Result<Modal> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiModal"), hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("NadiModal"), w!("NADI"), WS_POPUP, 0, 0, 10, 10, None, None, inst, None)?;
            let target = g.target_for(hwnd)?;
            let visual = g.dcomp.CreateVisual()?;
            let surface = g.surface(1, 1)?;
            visual.SetContent(&surface)?;
            target.SetRoot(&visual)?;
            Ok(Modal {
                hwnd,
                _target: target,
                visual,
                surface,
                s: 1.0,
                mon: RECT::default(),
                spec: Value::Null,
                owner: String::new(),
                field: String::new(),
                focus: 0,
                t: 0.0,
                vel: 0.0,
                to: 0.0,
                from: (0.0, 0.0),
                opened: Instant::now(),
                shown: false,
                result: None,
                waiting: Vec::new(),
                mouse: (-1.0, -1.0),
                keyed: false,
            })
        }
    }

    /// Ask: on monitor `mon` (physical px, `s` its scale), growing out of `from` (logical, in the monitor). One already out: this one
    /// waits for it.
    pub fn ask(&mut self, g: &Gfx, mon: RECT, s: f32, from: (f32, f32), spec: Value, owner: String) {
        if self.shown || self.t > 0.01 {
            if self.waiting.len() < 8 {
                self.waiting.push(Asked { spec, owner, mon, s, from });
            }
            return;
        }
        self.show(g, Asked { spec, owner, mon, s, from });
    }

    fn show(&mut self, g: &Gfx, a: Asked) {
        self.spec = a.spec;
        self.owner = a.owner;
        self.mon = a.mon;
        self.s = a.s;
        self.from = a.from;
        self.field = self.spec["field"]["value"].as_str().unwrap_or("").to_string();
        // (the focus: on Cancel when the other one destroys something; else on the last, the one asked for)
        let n = self.buttons().len();
        let red = self.buttons().iter().any(|b| b["tone"] == "red" || b["tone"] == "no");
        self.focus = if red { self.buttons().iter().position(|b| b["action"].as_str().unwrap_or("").is_empty()).unwrap_or(0) } else { n.saturating_sub(1) };
        let (w, h) = ((a.mon.right - a.mon.left).max(1), (a.mon.bottom - a.mon.top).max(1));
        if let Ok(surface) = g.surface(w as u32, h as u32) {
            unsafe {
                let _ = self.visual.SetContent(&surface);
            }
            self.surface = surface;
        }
        self.shown = true;
        self.to = 1.0;
        self.t = 0.0;
        self.vel = 0.0;
        self.opened = Instant::now();
        self.mouse = (-1.0, -1.0);
        self.keyed = false;
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, a.mon.left, a.mon.top, w, h, SWP_SHOWWINDOW);
            // (Windows lets only the app in front take the focus: joined to its input for a moment, NADI may)
            let fg = GetForegroundWindow();
            let theirs = GetWindowThreadProcessId(fg, None);
            let mine = windows::Win32::System::Threading::GetCurrentThreadId();
            let joined = theirs != 0 && theirs != mine && windows::Win32::System::Threading::AttachThreadInput(mine, theirs, true).as_bool();
            let _ = BringWindowToTop(self.hwnd);
            let _ = SetForegroundWindow(self.hwnd);
            let _ = SetFocus(self.hwnd);
            if joined {
                let _ = windows::Win32::System::Threading::AttachThreadInput(mine, theirs, false);
            }
        }
    }

    /// Its buttons (none given: OK).
    fn buttons(&self) -> Vec<Value> {
        match self.spec["buttons"].as_array() {
            Some(b) if !b.is_empty() => b.iter().take(3).cloned().collect(),
            _ => vec![json!({ "label": "OK" })],
        }
    }

    /// What was pressed (once): who asked, the button's action, and its args (the field's words).
    pub fn take_result(&mut self) -> Option<(String, String, Value)> {
        self.result.take()
    }

    fn press(&mut self, i: usize) {
        let Some(b) = self.buttons().get(i).cloned() else { return };
        let action = b["action"].as_str().unwrap_or("").to_string();
        let mut args = b["args"].clone();
        if !args.is_object() {
            args = json!({});
        }
        if self.spec["field"].is_object() {
            args["text"] = json!(self.field);
        }
        if action.is_empty() {
            return self.cancel();
        }
        self.result = Some((self.owner.clone(), action, args));
        self.close();
    }

    fn cancel(&mut self) {
        if self.owner != crate::builtin::PLUGIN && !self.owner.is_empty() {
            self.result = Some((self.owner.clone(), "modal:cancel".into(), json!({ "id": self.spec["id"] })));
        }
        self.close();
    }

    fn close(&mut self) {
        self.shown = false;
        self.to = 0.0;
    }

    /// Something in it moves: it springs out or away, its countdown runs, its caret blinks.
    pub fn moving(&self) -> bool {
        self.t != self.to || self.vel != 0.0 || (self.shown && (self.spec["countdown"].is_number() || self.spec["field"].is_object() || self.opened.elapsed().as_secs_f32() < 0.6))
    }

    /// One frame: its spring; its countdown run out, its last button pressed. True while it moves.
    pub fn step(&mut self, g: &Gfx, dt: f32) -> bool {
        if !self.moving() {
            return false;
        }
        if self.shown {
            if let Some(c) = self.spec["countdown"].as_f64() {
                if self.opened.elapsed().as_secs_f32() >= c as f32 {
                    let last = self.buttons().len().saturating_sub(1);
                    self.press(last);
                }
            }
        }
        let n = (dt.min(0.1) / (1.0 / 240.0)).ceil().max(1.0) as usize;
        let h = dt.min(0.1) / n as f32;
        for _ in 0..n {
            let a = -420.0 * (self.t - self.to) - 26.0 * self.vel;
            self.vel += a * h;
            self.t += self.vel * h;
        }
        if (self.t - self.to).abs() < 0.002 && self.vel.abs() < 0.02 {
            self.t = self.to;
            self.vel = 0.0;
            if self.to == 0.0 {
                unsafe {
                    let _ = ShowWindow(self.hwnd, SW_HIDE);
                }
                // (the next one waiting)
                if !self.waiting.is_empty() {
                    let a = self.waiting.remove(0);
                    self.show(g, a);
                }
            }
        }
        true
    }

    /// The card's box (logical, in the monitor), and where its things are: the icon's middle, the title's and text's lines, the
    /// field, the buttons.
    fn layout(&self, p: &Painter) -> (Rect, (f32, f32), f32, f32, Option<Rect>, Vec<Rect>) {
        let (mw, mh) = ((self.mon.right - self.mon.left) as f32 / self.s, (self.mon.bottom - self.mon.top) as f32 / self.s);
        let has_text = !self.spec["text"].as_str().unwrap_or("").is_empty() || (self.spec["countdown"].is_number() && self.spec["counting"].is_string());
        let has_field = self.spec["field"].is_object();
        let mut h = PAD + ICON + 14.0 + 20.0;
        if has_text {
            h += 22.0;
        }
        if has_field {
            h += 10.0 + FIELD_H;
        }
        h += 18.0 + BTN_H + 16.0;
        let card = Rect::new((mw - CW) / 2.0, (mh - h) / 2.0, CW, h);
        let icon = (card.x + CW / 2.0, card.y + PAD + ICON / 2.0);
        let title_y = card.y + PAD + ICON + 14.0 + 10.0;
        let text_y = title_y + 22.0;
        let mut y = title_y + 10.0 + if has_text { 22.0 } else { 0.0 };
        let field = has_field.then(|| {
            y += 10.0;
            let r = Rect::new(card.x + 20.0, y, CW - 40.0, FIELD_H);
            y += FIELD_H;
            r
        });
        y += 18.0;
        let bs = self.buttons();
        let n = bs.len() as f32;
        let bw = (CW - 40.0 - (n - 1.0) * 8.0) / n;
        let buttons = (0..bs.len()).map(|i| Rect::new(card.x + 20.0 + i as f32 * (bw + 8.0), y, bw, BTN_H)).collect();
        let _ = p;
        (card, icon, title_y, text_y, field, buttons)
    }

    /// What its window hears.
    pub fn message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM, g: &Gfx, pc: &Arc<Pc>) -> Option<LRESULT> {
        if !self.shown {
            return None;
        }
        match msg {
            WM_KEYDOWN => {
                let n = self.buttons().len();
                let vk = VIRTUAL_KEY(wp.0 as u16);
                if matches!(vk, VK_TAB | VK_LEFT | VK_RIGHT) {
                    self.keyed = true;
                }
                match vk {
                    VK_ESCAPE => self.cancel(),
                    VK_RETURN => self.press(self.focus),
                    VK_TAB => {
                        let back = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
                        self.focus = if back { (self.focus + n - 1) % n } else { (self.focus + 1) % n };
                    }
                    VK_LEFT => self.focus = self.focus.saturating_sub(1),
                    VK_RIGHT => self.focus = (self.focus + 1).min(n - 1),
                    VK_BACK => {
                        self.field.pop();
                    }
                    _ => return None,
                }
                Some(LRESULT(0))
            }
            WM_CHAR => {
                let c = char::from_u32(wp.0 as u32).unwrap_or('\0');
                if self.spec["field"].is_object() && !c.is_control() && self.field.chars().count() < 120 {
                    self.field.push(c);
                }
                Some(LRESULT(0))
            }
            WM_MOUSEMOVE | WM_LBUTTONUP => {
                let (x, y) = ((lp.0 & 0xffff) as i16 as f32 / self.s, ((lp.0 >> 16) & 0xffff) as i16 as f32 / self.s);
                self.mouse = (x, y);
                if msg == WM_LBUTTONUP {
                    let p = Painter { g, dc: &g.dc, s: self.s, pc, t: 0.0, viz: [0.0; 3] };
                    let (card, _, _, _, _, buttons) = self.layout(&p);
                    if let Some(i) = buttons.iter().position(|r| inside(r, x, y)) {
                        self.press(i);
                    } else if !inside(&card, x, y) {
                        self.cancel();
                    }
                }
                Some(LRESULT(0))
            }
            WM_ACTIVATE => {
                // (another window took the focus: it is put away, as Cancel)
                if (wp.0 & 0xffff) as u32 == WA_INACTIVE && self.opened.elapsed().as_millis() > 400 {
                    self.cancel();
                }
                None
            }
            _ => None,
        }
    }

    /// Draw it as it is now.
    pub fn render(&mut self, g: &Gfx, pc: &Arc<Pc>) {
        let s = self.s;
        let age = self.opened.elapsed().as_secs_f32();
        let p = Painter { g, dc: &g.dc, s, pc, t: age, viz: [0.0; 3] };
        let (card, icon, title_y, text_y, field, buttons) = self.layout(&p);
        let (mw, mh) = ((self.mon.right - self.mon.left) as f32 / s, (self.mon.bottom - self.mon.top) as f32 / s);
        let t = self.t.max(0.0);
        let Ok(shp) = shape::floating(&g.d2d_factory, card.w * s, card.h * s, RADIUS * s) else { return };
        let spec = self.spec.clone();
        let bs = self.buttons();
        let (focus, mouse, field_text, keyed) = (self.focus, self.mouse, self.field.clone(), self.keyed);
        let from = self.from;
        let _ = g.draw_surface(&self.surface, |dc| unsafe {
            use windows::Foundation::Numerics::Matrix3x2;
            // (the screen behind: dimmed, more as it comes out)
            p.round(Rect::new(0.0, 0.0, mw, mh), 0.0, Color(0.0, 0.0, 0.0, 0.38 * t.clamp(0.0, 1.0)));
            // (the card: growing out of its own middle, a little past its size and back; going, shrinking into it)
            let k = 0.6 + 0.4 * t;
            let (cx, cy) = (card.x + card.w / 2.0, card.y + card.h / 2.0);
            let (px, py) = (cx, cy);
            let _ = from;
            let mut base = Matrix3x2::default();
            dc.GetTransform(&mut base);
            let m = Matrix3x2::translation(-cx * s, -cy * s) * Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: 0.0, M32: 0.0 } * Matrix3x2::translation(px * s, py * s) * base;
            dc.SetTransform(&m);
            draw::faded(dc, t.clamp(0.0, 1.0), || {
                let mut b2 = Matrix3x2::default();
                dc.GetTransform(&mut b2);
                dc.SetTransform(&(Matrix3x2::translation(card.x * s, card.y * s) * b2));
                let _ = fill_shape(dc, &shp, FACE);
                dc.SetTransform(&b2);
                let tone = tone_of(spec["tone"].as_str().unwrap_or(""));
                // (its pictogram, in a ring of its colour; counting down: the ring runs out)
                p.circle(icon.0, icon.1, ICON / 2.0, tone.alpha(0.15));
                if let Some(c) = spec["countdown"].as_f64() {
                    let left = (1.0 - age / c as f32).clamp(0.0, 1.0);
                    p.ring(icon.0, icon.1, ICON / 2.0 - 1.5, 3.0, left, 0.0, tone, Color(0.0, 0.0, 0.0, 0.0));
                    draw::keep_moving();
                }
                let ic = spec["icon"].as_str().unwrap_or("info");
                p.icon(ic, icon.0 - 11.0, icon.1 - 11.0, 22.0, tone);
                // (its question, and a line of what happens; counting down, how long is left)
                let title = spec["title"].as_str().unwrap_or("");
                let tw = p.measure(title, 15.0, Weight::Semibold, CW - 40.0);
                p.text(title, card.x + CW / 2.0 - tw / 2.0, title_y, 15.0, Weight::Semibold, CW - 40.0, draw::TEXT);
                let mut text = spec["text"].as_str().unwrap_or("").to_string();
                if let (Some(c), Some(word)) = (spec["countdown"].as_f64(), spec["counting"].as_str()) {
                    text = format!("{word} in {} s.", (c as f32 - age).ceil().max(0.0) as u32);
                }
                if !text.is_empty() {
                    let xw = p.measure(&text, 12.0, Weight::Regular, CW - 40.0);
                    p.text(&text, card.x + CW / 2.0 - xw / 2.0, text_y, 12.0, Weight::Regular, CW - 40.0, draw::DIM);
                }
                // (its field: what is typed (dots for a password), else its hint; a caret)
                if let Some(fr) = field {
                    p.round(fr, FIELD_H / 2.0, Color(0.078, 0.082, 0.102, 1.0));
                    let secret = spec["field"]["secret"] == true;
                    let shown = if secret { "•".repeat(field_text.chars().count()) } else { field_text.clone() };
                    let hint = spec["field"]["hint"].as_str().unwrap_or("");
                    let fw = if shown.is_empty() {
                        p.text(hint, fr.x + 14.0, fr.y + FIELD_H / 2.0, 13.0, Weight::Regular, fr.w - 28.0, draw::DIM);
                        0.0
                    } else {
                        p.text(&shown, fr.x + 14.0, fr.y + FIELD_H / 2.0, 13.0, Weight::Regular, fr.w - 28.0, draw::TEXT)
                    };
                    if age.fract() < 0.5 {
                        p.round(Rect::new(fr.x + 15.0 + fw, fr.y + 11.0, 1.5, FIELD_H - 22.0), 0.75, draw::accent());
                    }
                }
                // (its buttons: the one asked for in its colour, the rest ghosts; the focus ringed)
                for (i, (b, r)) in bs.iter().zip(buttons.iter()).enumerate() {
                    let bt = b["tone"].as_str().unwrap_or("");
                    let solid = !bt.is_empty() || (b["action"].as_str().is_some_and(|a| !a.is_empty()) && i == bs.len() - 1);
                    let c = tone_of(bt);
                    // (a ghost: its ground only under the mouse, or when the keys moved the focus to it)
                    if solid {
                        p.round(*r, BTN_H / 2.0, c);
                    } else if inside(r, mouse.0, mouse.1) || (keyed && i == focus) {
                        p.round(*r, BTN_H / 2.0, Color(0.15, 0.15, 0.17, 1.0));
                    }
                    let label = b["label"].as_str().unwrap_or("");
                    let lw = p.measure(label, 12.5, Weight::Semibold, r.w - 12.0);
                    p.text(label, r.x + r.w / 2.0 - lw / 2.0, r.y + BTN_H / 2.0, 12.5, Weight::Semibold, r.w - 12.0, if solid { Color(0.06, 0.04, 0.04, 1.0) } else { Color(0.79, 0.79, 0.82, 1.0) });
                }
            });
            dc.SetTransform(&base);
        });
    }
}
