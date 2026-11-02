//! Make Room: maximized windows kept out of the edges NADI's widgets stand on, as they are kept out of the taskbar's. Each such edge
//! of a monitor is one of Windows' app bars (a window of its own, invisible and click-through, registered with the shell): Windows
//! itself sizes maximized windows to what is left, moves the bars beside the taskbar's when they share an edge, and gives the room
//! back when they go (when NADI ends, or the setting is turned off). A full-screen app covers all of it regardless.

use crate::shape::Edge;
use std::cell::RefCell;
use windows::core::w;
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

/// what the shell tells a bar (its place changed: the taskbar moved, a monitor changed)
const WM_ROOM: u32 = WM_APP + 17;

/// One kept edge: its window, the monitor's rectangle (physical px), its edge, how deep (physical px).
#[derive(Clone)]
struct Bar {
    hwnd: HWND,
    mon: RECT,
    edge: Edge,
    depth: i32,
}

thread_local! {
    static BARS: RefCell<Vec<Bar>> = const { RefCell::new(Vec::new()) };
}

fn abe(e: Edge) -> u32 {
    match e {
        Edge::Left => ABE_LEFT,
        Edge::Top => ABE_TOP,
        Edge::Right => ABE_RIGHT,
        Edge::Bottom => ABE_BOTTOM,
    }
}

fn data(b: &Bar) -> APPBARDATA {
    APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, hWnd: b.hwnd, uCallbackMessage: WM_ROOM, uEdge: abe(b.edge), rc: b.mon, lParam: LPARAM(0) }
}

/// Where a bar goes: its whole edge, as deep as it is, beside whatever the shell keeps there already (the taskbar).
fn place(b: &Bar) {
    unsafe {
        let mut d = data(b);
        let _ = SHAppBarMessage(ABM_QUERYPOS, &mut d);
        match b.edge {
            Edge::Top => d.rc.bottom = d.rc.top + b.depth,
            Edge::Bottom => d.rc.top = d.rc.bottom - b.depth,
            Edge::Left => d.rc.right = d.rc.left + b.depth,
            Edge::Right => d.rc.left = d.rc.right - b.depth,
        }
        let _ = SHAppBarMessage(ABM_SETPOS, &mut d);
        let _ = SetWindowPos(b.hwnd, HWND_BOTTOM, d.rc.left, d.rc.top, (d.rc.right - d.rc.left).max(1), (d.rc.bottom - d.rc.top).max(1), SWP_NOACTIVATE);
    }
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_ROOM && wp.0 as u32 == ABN_POSCHANGED {
        let bar = BARS.with(|b| b.borrow().iter().find(|x| x.hwnd == hwnd).cloned());
        if let Some(b) = bar {
            place(&b);
        }
        return LRESULT(0);
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn make(mon: RECT, edge: Edge, depth: i32) -> Option<Bar> {
    unsafe {
        let inst = GetModuleHandleW(None).ok()?;
        let wc = WNDCLASSW { lpfnWndProc: Some(proc), hInstance: inst.into(), lpszClassName: w!("NadiRoom"), hbrBackground: HBRUSH::default(), ..Default::default() };
        RegisterClassW(&wc);
        // (there, for the shell; seen by no one, and the mouse goes through it)
        let hwnd = CreateWindowExW(WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TRANSPARENT, w!("NadiRoom"), w!("NADI Room"), WS_POPUP, mon.left, mon.top, 1, 1, None, None, inst, None).ok()?;
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let b = Bar { hwnd, mon, edge, depth };
        let mut d = data(&b);
        if SHAppBarMessage(ABM_NEW, &mut d) == 0 {
            let _ = DestroyWindow(hwnd);
            return None;
        }
        Some(b)
    }
}

fn remove(b: &Bar) {
    unsafe {
        let mut d = data(b);
        let _ = SHAppBarMessage(ABM_REMOVE, &mut d);
        let _ = DestroyWindow(b.hwnd);
    }
}

/// The edges to keep now (each: a monitor's rectangle, its edge, how deep, physical px). Those kept that are not asked for any more
/// are given back; `shrink` false: a kept one is only ever made deeper here, not given back or made shallower (a widget gone idle a
/// moment would make the windows jump).
pub fn set(want: &[(RECT, Edge, i32)], shrink: bool) {
    let mut bars = BARS.with(|b| b.borrow().clone());
    let mut changed = Vec::new();
    // (given back: not asked for)
    if shrink {
        bars.retain(|b| {
            let keep = want.iter().any(|(m, e, _)| *m == b.mon && *e == b.edge);
            if !keep {
                remove(b);
            }
            keep
        });
    }
    for (mon, edge, depth) in want {
        let depth = (*depth).max(1);
        match bars.iter_mut().find(|b| b.mon == *mon && b.edge == *edge) {
            Some(b) => {
                let to = if shrink { depth } else { b.depth.max(depth) };
                if to != b.depth {
                    b.depth = to;
                    changed.push(b.clone());
                }
            }
            None => {
                if let Some(b) = make(*mon, *edge, depth) {
                    changed.push(b.clone());
                    bars.push(b);
                }
            }
        }
    }
    BARS.with(|b| *b.borrow_mut() = bars);
    for b in changed {
        place(&b);
    }
}

/// Every kept edge given back (NADI ends, or the setting is off).
pub fn clear() {
    let bars = BARS.with(|b| std::mem::take(&mut *b.borrow_mut()));
    for b in &bars {
        remove(b);
    }
}
