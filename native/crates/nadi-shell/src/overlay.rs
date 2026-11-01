//! The frame's window: one per monitor, as big as the monitor, always on top, never taking the focus, drawing nothing of its own. What it
//! shows are the pieces (each a DirectComposition visual with its own surface), and the mouse reaches it only over them (a window region,
//! the union of the pieces): everywhere else, clicks go to the windows below.

use crate::gfx::Gfx;
use windows::core::{w, Result};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, Debug)]
pub struct Monitor {
    pub handle: HMONITOR,
    /// physical px
    pub rect: RECT,
    /// the work area (without the taskbar), physical px
    pub work: RECT,
    /// DPI / 96
    pub scale: f32,
    pub primary: bool,
    /// the display's device name (`\\.\DISPLAY1`), UTF-16, 0-ended: how a widget pinned to it finds it again
    pub device: [u16; 32],
}

impl Monitor {
    pub fn name(&self) -> String {
        let n = self.device.iter().position(|c| *c == 0).unwrap_or(32);
        String::from_utf16_lossy(&self.device[..n])
    }
}

pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn each(h: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<Monitor>);
        let mut ex = MONITORINFOEXW::default();
        ex.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(h, &mut ex.monitorInfo).as_bool() {
            let info = ex.monitorInfo;
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            list.push(Monitor { handle: h, rect: info.rcMonitor, work: info.rcWork, scale: dx as f32 / 96.0, primary: info.dwFlags & MONITORINFOF_PRIMARY != 0, device: ex.szDevice });
        }
        TRUE
    }
    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(each), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

/// One piece on the screen: its visual, its surface (allocated at its largest, drawn at its size now), and where it is (physical px, in
/// the monitor).
pub struct Piece {
    pub visual: IDCompositionVisual,
    pub surface: IDCompositionSurface,
    pub alloc: (u32, u32),
    pub x: i32,
    pub y: i32,
}

pub struct Overlay {
    pub hwnd: HWND,
    pub monitor: Monitor,
    #[allow(dead_code)]
    target: IDCompositionTarget,
    #[allow(dead_code)]
    pub root: IDCompositionVisual,
    /// what is drawn behind the widgets (the liquid, the shadow of one held), and the widgets over it: two layers, so nothing made
    /// later for one ends up in the other's place
    under: IDCompositionVisual,
    layer: IDCompositionVisual,
    /// a window just under it, across the monitor, that the mouse goes through: what is drawn round the pieces (their glow), which
    /// the frame's window could not show (it shows only where it takes the mouse)
    glass: HWND,
    #[allow(dead_code)]
    glass_target: IDCompositionTarget,
    glass_root: IDCompositionVisual,
    /// the same, over it: what floats over the pieces without taking the mouse (tips)
    top: HWND,
    #[allow(dead_code)]
    top_target: IDCompositionTarget,
    top_root: IDCompositionVisual,
}

/// A window across a monitor that the mouse goes through, and the root of what is drawn in it.
unsafe fn glass_window(gfx: &Gfx, r: RECT, inst: windows::Win32::Foundation::HINSTANCE) -> Result<(HWND, IDCompositionTarget, IDCompositionVisual)> {
    let hwnd = CreateWindowExW(
        WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
        GLASS_NAME,
        w!("NADI Glass"),
        WS_POPUP,
        r.left,
        r.top,
        r.right - r.left,
        r.bottom - r.top,
        None,
        None,
        inst,
        None,
    )?;
    let _ = SetLayeredWindowAttributes(hwnd, windows::Win32::Foundation::COLORREF(0), 255, LWA_ALPHA);
    let target = gfx.target_for(hwnd)?;
    let root = gfx.dcomp.CreateVisual()?;
    target.SetRoot(&root)?;
    let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    Ok((hwnd, target, root))
}

static GLASS_CLASS: std::sync::Once = std::sync::Once::new();
const GLASS_NAME: windows::core::PCWSTR = w!("NadiGlass");

unsafe extern "system" fn glass_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_NCHITTEST => LRESULT(HTTRANSPARENT as isize),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

static CLASS: std::sync::Once = std::sync::Once::new();
const CLASS_NAME: windows::core::PCWSTR = w!("NadiFrame");

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        // (it is never made the active window: a click on a widget does not take the focus from the app you are in)
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        // (a frame window goes when the screens are made again, a monitor goes, or the size changes: NADI goes on; only its tray
        // window's end is NADI's)
        _ => {
            if let Some(r) = crate::events::window_message(hwnd, msg, wp, lp) {
                return r;
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
    }
}

impl Overlay {
    pub fn new(gfx: &Gfx, monitor: Monitor) -> Result<Overlay> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            CLASS.call_once(|| {
                let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: CLASS_NAME, hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(), ..Default::default() };
                RegisterClassW(&wc);
            });
            let r = monitor.rect;
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("NADI"),
                WS_POPUP,
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                None,
                None,
                inst,
                None,
            )?;
            let target = gfx.target_for(hwnd)?;
            let root = gfx.dcomp.CreateVisual()?;
            target.SetRoot(&root)?;
            // (nothing takes the mouse until there are pieces)
            SetWindowRgn(hwnd, CreateRectRgn(0, 0, 0, 0), false);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            // the glass
            GLASS_CLASS.call_once(|| {
                let wc = WNDCLASSW { lpfnWndProc: Some(glass_proc), hInstance: inst.into(), lpszClassName: GLASS_NAME, ..Default::default() };
                RegisterClassW(&wc);
            });
            let (glass, glass_target, glass_root) = glass_window(gfx, r, inst.into())?;
            let (top, top_target, top_root) = glass_window(gfx, r, inst.into())?;
            // (DirectComposition's own words: with no visual to go by, "insert above" puts it at the bottom, and not, at the top)
            let under = gfx.dcomp.CreateVisual()?;
            let layer = gfx.dcomp.CreateVisual()?;
            root.AddVisual(&under, false, None)?;
            root.AddVisual(&layer, false, None)?;
            let o = Overlay { hwnd, monitor, target, root, under, layer, glass, glass_target, glass_root, top, top_target, top_root };
            o.raise();
            Ok(o)
        }
    }

    /// A new piece, with room for `w` × `h` physical px.
    pub fn piece(&self, gfx: &Gfx, w: u32, h: u32) -> Result<Piece> {
        unsafe {
            let visual = gfx.dcomp.CreateVisual()?;
            let surface = gfx.surface(w, h)?;
            visual.SetContent(&surface)?;
            self.layer.AddVisual(&visual, true, None)?;
            Ok(Piece { visual, surface, alloc: (w, h), x: 0, y: 0 })
        }
    }

    /// Put a piece over all the others (one held, a menu).
    pub fn to_front(&self, v: &IDCompositionVisual) {
        unsafe {
            let _ = self.layer.RemoveVisual(v);
            let _ = self.top_root.RemoveVisual(v);
            let _ = self.layer.AddVisual(v, false, None);
        }
    }

    /// Take in a piece from another monitor's frame (of the same device), over all the others.
    pub fn adopt(&self, v: &IDCompositionVisual) {
        unsafe {
            let _ = self.layer.AddVisual(v, false, None);
        }
    }

    /// Put a piece on the glass over the frame's window, over all there: seen as it is, the mouse going through it to what is under
    /// it (the frame's window is cut to where it takes the mouse, and what is cut away of it is not seen).
    pub fn to_glass_front(&self, v: &IDCompositionVisual) {
        unsafe {
            let _ = self.layer.RemoveVisual(v);
            let _ = self.top_root.RemoveVisual(v);
            let _ = self.top_root.AddVisual(v, false, None);
        }
    }

    /// A piece under all the others (what is drawn across the screen behind the widgets).
    pub fn piece_below(&self, gfx: &Gfx, w: u32, h: u32) -> Result<Piece> {
        unsafe {
            let visual = gfx.dcomp.CreateVisual()?;
            let surface = gfx.surface(w, h)?;
            visual.SetContent(&surface)?;
            self.under.AddVisual(&visual, false, None)?;
            Ok(Piece { visual, surface, alloc: (w, h), x: 0, y: 0 })
        }
    }

    pub fn remove(&self, p: &Piece) -> Result<()> {
        unsafe {
            if self.layer.RemoveVisual(&p.visual).is_ok() {
                return Ok(());
            }
            if self.top_root.RemoveVisual(&p.visual).is_ok() {
                return Ok(());
            }
            self.under.RemoveVisual(&p.visual)
        }
    }

    /// A piece on the glass under the frame's window (the mouse goes through it).
    pub fn glass_piece(&self, gfx: &Gfx, w: u32, h: u32) -> Result<Piece> {
        unsafe {
            let visual = gfx.dcomp.CreateVisual()?;
            let surface = gfx.surface(w, h)?;
            visual.SetContent(&surface)?;
            self.glass_root.AddVisual(&visual, true, None)?;
            Ok(Piece { visual, surface, alloc: (w, h), x: 0, y: 0 })
        }
    }

    /// A piece on the glass over all the others there (the liquid: the glows stay under it, never over a widget beside theirs).
    pub fn glass_piece_top(&self, gfx: &Gfx, w: u32, h: u32) -> Result<Piece> {
        unsafe {
            let visual = gfx.dcomp.CreateVisual()?;
            let surface = gfx.surface(w, h)?;
            visual.SetContent(&surface)?;
            self.glass_root.AddVisual(&visual, false, None)?;
            Ok(Piece { visual, surface, alloc: (w, h), x: 0, y: 0 })
        }
    }

    pub fn remove_glass(&self, p: &Piece) {
        unsafe {
            let _ = self.glass_root.RemoveVisual(&p.visual);
        }
    }

    /// The glass over the frame's window (live pictures of windows are put on it).
    pub fn top_hwnd(&self) -> HWND {
        self.top
    }

    /// A piece on the glass over the frame's window (the mouse goes through it).
    pub fn top_piece(&self, gfx: &Gfx, w: u32, h: u32) -> Result<Piece> {
        unsafe {
            let visual = gfx.dcomp.CreateVisual()?;
            let surface = gfx.surface(w, h)?;
            visual.SetContent(&surface)?;
            self.top_root.AddVisual(&visual, true, None)?;
            Ok(Piece { visual, surface, alloc: (w, h), x: 0, y: 0 })
        }
    }

    pub fn remove_top(&self, p: &Piece) {
        unsafe {
            let _ = self.top_root.RemoveVisual(&p.visual);
        }
    }

    /// Shown or hidden, its glass with it.
    pub fn show(&self, on: bool) {
        unsafe {
            let how = if on { SW_SHOWNOACTIVATE } else { SW_HIDE };
            let _ = ShowWindow(self.hwnd, how);
            let _ = ShowWindow(self.glass, how);
            let _ = ShowWindow(self.top, how);
        }
    }

    /// Gone, its glass with it.
    pub fn destroy(&self) {
        unsafe {
            let _ = DestroyWindow(self.glass);
            let _ = DestroyWindow(self.top);
            let _ = DestroyWindow(self.hwnd);
        }
    }

    /// The mouse reaches the window only over these boxes (physical px, in the monitor).
    pub fn set_hit(&self, boxes: &[RECT]) {
        unsafe {
            let rgn = CreateRectRgn(0, 0, 0, 0);
            for b in boxes {
                if b.right > b.left && b.bottom > b.top {
                    let one = CreateRectRgn(b.left, b.top, b.right, b.bottom);
                    CombineRgn(rgn, rgn, one, RGN_OR);
                    let _ = DeleteObject(one);
                }
            }
            // (the window owns the region from here)
            SetWindowRgn(self.hwnd, rgn, false);
        }
    }

    /// Back on top (a window that went full screen, or another topmost one, may have come over it).
    pub fn raise(&self) {
        unsafe {
            let _ = SetWindowPos(self.glass, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = SetWindowPos(self.hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = SetWindowPos(self.top, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
        }
    }
}

impl Piece {
    pub fn place(&mut self, x: i32, y: i32) -> Result<()> {
        self.x = x;
        self.y = y;
        unsafe {
            self.visual.SetOffsetX2(x as f32)?;
            self.visual.SetOffsetY2(y as f32)?;
        }
        Ok(())
    }
}
