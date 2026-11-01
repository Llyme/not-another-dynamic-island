//! Files dragged from File Explorer onto a widget that takes them (its content says `drop`: the action told the paths): the frame's
//! windows are drop targets; where a widget that takes them is, the drag may let go, and the widget hears of it (its lid lifts).
//! Its places are kept here by the frame as it lays them out (physical px, on the screen), so a drag is answered at once.

use std::sync::Mutex;
use windows::core::implement;
use windows::Win32::Foundation::{HWND, LPARAM, POINTL, WPARAM};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_MOVE, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// a drag came over a widget that takes it, or left it (the frame draws it again); something was let go on one
pub const WM_DROP_HOVER: u32 = WM_APP + 13;
pub const WM_DROPPED: u32 = WM_APP + 14;

static TARGETS: Mutex<Vec<(i32, i32, i32, i32, String)>> = Mutex::new(Vec::new());
static HOVER: Mutex<Option<String>> = Mutex::new(None);
static DROPPED: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
static NOTIFY: Mutex<isize> = Mutex::new(0);

/// Where the frame's message window is (told of drags and drops).
pub fn set_notify(hwnd: HWND) {
    *NOTIFY.lock().unwrap() = hwnd.0 as isize;
}

/// Where the widgets that take a drop are now (left, top, right, bottom on the screen, physical px; and each's key).
pub fn set_targets(t: Vec<(i32, i32, i32, i32, String)>) {
    *TARGETS.lock().unwrap() = t;
}

/// The widget a drag is over now.
pub fn hover() -> Option<String> {
    HOVER.lock().unwrap().clone()
}

/// What was let go since last asked: on which widget, which paths.
pub fn take() -> Vec<(String, Vec<String>)> {
    std::mem::take(&mut *DROPPED.lock().unwrap())
}

/// A window of the frame's: a drop target.
pub fn register(hwnd: HWND) {
    let t: IDropTarget = Target.into();
    unsafe {
        let _ = RegisterDragDrop(hwnd, &t);
    }
}

fn post(msg: u32) {
    let h = *NOTIFY.lock().unwrap();
    if h != 0 {
        unsafe {
            let _ = PostMessageW(HWND(h as *mut _), msg, WPARAM(0), LPARAM(0));
        }
    }
}

fn at(pt: &POINTL) -> Option<String> {
    TARGETS.lock().unwrap().iter().find(|(l, t, r, b, _)| pt.x >= *l && pt.x < *r && pt.y >= *t && pt.y < *b).map(|(.., k)| k.clone())
}

fn set_hover(k: Option<String>) {
    let mut h = HOVER.lock().unwrap();
    if *h != k {
        *h = k;
        drop(h);
        post(WM_DROP_HOVER);
    }
}

/// The paths a drag carries (files and folders).
unsafe fn paths(obj: &IDataObject) -> Vec<String> {
    let fmt = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
    let Ok(mut med) = obj.GetData(&fmt) else { return Vec::new() };
    let hdrop = HDROP(med.u.hGlobal.0);
    let n = DragQueryFileW(hdrop, u32::MAX, None);
    let mut out = Vec::new();
    for i in 0..n {
        let len = DragQueryFileW(hdrop, i, None) as usize;
        let mut buf = vec![0u16; len + 1];
        let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
        out.push(String::from_utf16_lossy(&buf[..got]));
    }
    ReleaseStgMedium(&mut med);
    out
}

#[implement(IDropTarget)]
struct Target;

/// (the effect shown: a move where it may move, else a copy; the files are thrown away here, so the one dragging from is told a copy
/// was made at the end, and deletes nothing itself)
fn effect(allowed: DROPEFFECT) -> DROPEFFECT {
    if allowed.0 & DROPEFFECT_MOVE.0 != 0 {
        DROPEFFECT_MOVE
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, _obj: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> windows::core::Result<()> {
        let k = at(pt);
        unsafe {
            *eff = if k.is_some() { effect(*eff) } else { DROPEFFECT_NONE };
        }
        set_hover(k);
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> windows::core::Result<()> {
        let k = at(pt);
        unsafe {
            *eff = if k.is_some() { effect(*eff) } else { DROPEFFECT_NONE };
        }
        set_hover(k);
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        set_hover(None);
        Ok(())
    }

    fn Drop(&self, obj: Option<&IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, eff: *mut DROPEFFECT) -> windows::core::Result<()> {
        set_hover(None);
        let (Some(k), Some(obj)) = (at(pt), obj) else {
            unsafe {
                *eff = DROPEFFECT_NONE;
            }
            return Ok(());
        };
        let ps = unsafe { paths(obj) };
        unsafe {
            *eff = if ps.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_COPY };
        }
        if !ps.is_empty() {
            DROPPED.lock().unwrap().push((k, ps));
            post(WM_DROPPED);
        }
        Ok(())
    }
}
