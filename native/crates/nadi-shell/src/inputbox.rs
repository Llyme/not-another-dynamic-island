//! Typing into a widget's panel: a field pressed opens a small window of its own right over it (the frame's windows never take the
//! keyboard), with what is typed and a caret. Enter gives the words to the plugin; Esc, or a click elsewhere, puts it away. A live
//! field (a search) gives them at every change.

use crate::draw::{self, Painter};
use crate::gfx::Gfx;
use crate::paint::Weight;
use crate::shape::Rect;
use nadi_core::pc::Pc;
use std::sync::Arc;
use std::time::Instant;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// What the box asks of the frame after a key.
pub enum Said {
    Nothing,
    /// the words changed (a live field: given now)
    Changed,
    /// Enter: given, and the box goes
    Done,
    /// Esc, or the focus went: it goes
    Gone,
    /// its x pressed: emptied (told at once, live or not)
    Cleared,
}

pub struct InputBox {
    pub hwnd: HWND,
    _target: IDCompositionTarget,
    visual: IDCompositionVisual,
    surface: IDCompositionSurface,
    s: f32,
    /// logical size
    w: f32,
    h: f32,
    pub text: String,
    placeholder: String,
    /// the widget it types for, what it is told, and whether at every change
    pub key: String,
    pub plugin: String,
    pub action: String,
    pub live: bool,
    /// a password: dots shown for what is typed
    pub secret: bool,
    pub shown: bool,
    since: Instant,
    /// how it looks: in a card's colours (its ground, its words), how big and bold its words, and words of several lines (Enter a
    /// new line; Ctrl + Enter, Esc or a click elsewhere: given)
    ground: Option<crate::paint::Color>,
    ink: Option<crate::paint::Color>,
    size: f32,
    bold: bool,
    pub area: bool,
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

impl InputBox {
    pub fn new(g: &Gfx) -> windows::core::Result<InputBox> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiInput"), hCursor: LoadCursorW(None, IDC_IBEAM).unwrap_or_default(), ..Default::default() };
            RegisterClassW(&wc);
            let hwnd = CreateWindowExW(WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW, w!("NadiInput"), w!("NADI Input"), WS_POPUP, 0, 0, 10, 10, None, None, inst, None)?;
            let target = g.target_for(hwnd)?;
            let visual = g.dcomp.CreateVisual()?;
            let surface = g.surface(1, 1)?;
            visual.SetContent(&surface)?;
            target.SetRoot(&visual)?;
            Ok(InputBox { hwnd, _target: target, visual, surface, s: 1.0, w: 0.0, h: 0.0, text: String::new(), placeholder: String::new(), key: String::new(), plugin: String::new(), action: String::new(), live: false, secret: false, shown: false, since: Instant::now(), ground: None, ink: None, size: 11.5, bold: false, area: false })
        }
    }

    /// Open over a field (`at`: physical px on the screen), for a widget's plugin, with what it holds already.
    #[allow(clippy::too_many_arguments)]
    pub fn open(&mut self, g: &Gfx, at: RECT, s: f32, key: String, plugin: String, action: String, live: bool, placeholder: String, text: String) {
        self.s = s;
        self.w = (at.right - at.left) as f32 / s;
        self.h = (at.bottom - at.top) as f32 / s;
        self.key = key;
        self.plugin = plugin;
        self.action = action;
        self.live = live;
        self.secret = false;
        self.placeholder = placeholder;
        self.text = text;
        if let Ok(surface) = g.surface((at.right - at.left) as u32 + 2, (at.bottom - at.top) as u32 + 2) {
            unsafe {
                let _ = self.visual.SetContent(&surface);
            }
            self.surface = surface;
        }
        self.shown = true;
        self.since = Instant::now();
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, at.left, at.top, at.right - at.left, at.bottom - at.top, SWP_SHOWWINDOW);
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

    /// Written in a card (its colours): what is typed is kept however the box goes, as on paper.
    pub fn in_card(&self) -> bool {
        self.ground.is_some()
    }

    /// How it looks (see `Press::Field`); none: the plain box.
    pub fn set_look(&mut self, look: &serde_json::Value) {
        let hex = |k: &str| look[k].as_str().filter(|v| v.len() == 7 && v.starts_with('#')).map(crate::paint::Color::hex);
        self.ground = hex("ground");
        self.ink = hex("ink");
        self.size = look["size"].as_f64().map_or(11.5, |v| v as f32).clamp(9.0, 20.0);
        self.bold = look["bold"] == true;
        self.area = look["area"] == true;
    }

    /// Its field moved (its panel grew or shrank): the box goes with it.
    pub fn follow(&mut self, at: RECT) {
        let mut now = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut now);
        }
        if now == at {
            return;
        }
        unsafe {
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, at.left, at.top, at.right - at.left, at.bottom - at.top, SWP_NOACTIVATE);
        }
    }

    /// Back over the frame's windows (they were put on top again: it would be under them).
    pub fn raise(&self) {
        if self.shown {
            unsafe {
                let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            }
        }
    }

    pub fn close(&mut self) {
        self.shown = false;
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }

    /// What its window hears.
    pub fn message(&mut self, msg: u32, wp: WPARAM, lp: LPARAM) -> Said {
        match msg {
            // (on its x, while it holds something: emptied)
            WM_LBUTTONDOWN if !self.text.is_empty() => {
                let x = (lp.0 & 0xffff) as i16 as f32 / self.s;
                if x >= self.w - 28.0 {
                    self.text.clear();
                    Said::Cleared
                } else {
                    Said::Nothing
                }
            }
            WM_ACTIVATE if (wp.0 & 0xffff) as u32 == WA_INACTIVE && self.shown => Said::Gone,
            // (words of several lines: Enter a new line, Ctrl + Enter given)
            WM_KEYDOWN if self.area && VIRTUAL_KEY(wp.0 as u16) == VK_RETURN && unsafe { GetKeyState(VK_CONTROL.0 as i32) } >= 0 => {
                if self.text.chars().count() < 4000 {
                    self.text.push('\n');
                }
                Said::Changed
            }
            WM_KEYDOWN => match VIRTUAL_KEY(wp.0 as u16) {
                VK_ESCAPE => Said::Gone,
                VK_RETURN => Said::Done,
                VK_BACK => {
                    self.text.pop();
                    Said::Changed
                }
                _ => Said::Nothing,
            },
            WM_CHAR => {
                let c = char::from_u32(wp.0 as u32).unwrap_or('\0');
                if !c.is_control() && self.text.chars().count() < if self.area { 4000 } else { 400 } {
                    self.text.push(c);
                    Said::Changed
                } else {
                    Said::Nothing
                }
            }
            _ => Said::Nothing,
        }
    }

    /// Draw it: the field, what is typed (else its hint), the caret.
    pub fn render(&self, g: &Gfx, pc: &Arc<Pc>) {
        let s = self.s;
        let p = Painter { g, dc: &g.dc, s, pc, t: 0.0, viz: [0.0; 3] };
        let (w, h) = (self.w, self.h);
        let text = if self.secret { "•".repeat(self.text.chars().count()) } else { self.text.clone() };
        let hint = self.placeholder.clone();
        // (in a card's colours: its ground a little lighter, its words in its ink, no x)
        if let (Some(ground), Some(ink)) = (self.ground, self.ink) {
            let (size, bold, area) = (self.size, self.bold, self.area);
            let _ = g.draw_surface(&self.surface, |_dc| {
                p.round(Rect::new(0.0, 0.0, w, h), 6.0, ground);
                p.round(Rect::new(0.0, 0.0, w, h), 6.0, crate::paint::Color(1.0, 1.0, 1.0, 0.06));
                let x = 4.0;
                if area {
                    let units = text.encode_utf16().count();
                    if text.is_empty() {
                        p.text(&hint, x, 11.0, size, Weight::Regular, w - 8.0, ink.alpha(0.4));
                        p.round(Rect::new(x, 4.0, 1.5, size + 4.0), 0.75, draw::accent());
                    } else {
                        p.paragraph_lit(&text, x, 2.0, size, w - 8.0, units, ink, ink, Some(draw::accent()));
                    }
                } else {
                    let weight = if bold { Weight::Semibold } else { Weight::Regular };
                    let tw = if text.is_empty() {
                        p.text(&hint, x, h / 2.0, size, weight, w - 8.0, ink.alpha(0.4));
                        0.0
                    } else {
                        p.text(&text, x, h / 2.0, size, weight, w - 12.0, ink)
                    };
                    p.round(Rect::new(x + tw + 1.0, h / 2.0 - size * 0.6, 1.5, size * 1.2), 0.75, draw::accent());
                }
            });
            return;
        }
        let _ = g.draw_surface(&self.surface, |_dc| {
            p.round(Rect::new(0.0, 0.0, w, h), 10.0, draw::FACE);
            p.round(Rect::new(0.0, 0.0, w, h), 10.0, crate::paint::Color(1.0, 1.0, 1.0, 0.12));
            let x = 10.0;
            let tw = if text.is_empty() {
                p.text(&hint, x, h / 2.0, 11.5, Weight::Regular, w - 20.0, draw::DIM);
                0.0
            } else {
                p.text(&text, x, h / 2.0, 11.5, Weight::Regular, w - 24.0, draw::TEXT)
            };
            p.round(Rect::new(x + tw + 1.0, h / 2.0 - 7.0, 1.5, 14.0), 0.75, draw::accent());
            // (its x, while it holds something)
            if !text.is_empty() {
                p.circle(w - 16.0, h / 2.0, 9.0, crate::paint::Color(1.0, 1.0, 1.0, 0.1));
                p.icon("x", w - 21.0, h / 2.0 - 5.0, 10.0, draw::DIM);
            }
        });
    }
}
