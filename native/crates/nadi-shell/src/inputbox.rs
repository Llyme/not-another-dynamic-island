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
            Ok(InputBox { hwnd, _target: target, visual, surface, s: 1.0, w: 0.0, h: 0.0, text: String::new(), placeholder: String::new(), key: String::new(), plugin: String::new(), action: String::new(), live: false, secret: false, shown: false, since: Instant::now() })
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
                if !c.is_control() && self.text.chars().count() < 400 {
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
