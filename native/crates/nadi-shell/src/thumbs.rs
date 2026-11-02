//! Live pictures of windows (DWM thumbnails), drawn by the compositor into a box of a frame window: what a window shows, as it changes,
//! for nothing on this side.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;

/// A live picture of `src`, shown in `dest` (none: it could not be made).
pub fn register(dest: HWND, src: isize) -> Option<isize> {
    unsafe { DwmRegisterThumbnail(dest, HWND(src as *mut _)).ok() }
}

/// Put it in a box (physical px, in the window), filling it: the window's picture cut to the box's proportions (its middle across,
/// its top kept, where a window's title and tabs are); or hide it.
pub fn place(thumb: isize, r: RECT, visible: bool) {
    unsafe {
        let size = DwmQueryThumbnailSourceSize(thumb).unwrap_or_default();
        let (bw, bh) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let mut flags = DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY;
        let mut source = RECT::default();
        if size.cx > 0 && size.cy > 0 && bw > 0.0 && bh > 0.0 {
            let (sw, sh) = (size.cx as f32, size.cy as f32);
            let (cw, ch) = if sw / sh > bw / bh { (sh * bw / bh, sh) } else { (sw, sw * bh / bw) };
            let left = ((sw - cw) / 2.0).round() as i32;
            source = RECT { left, top: 0, right: left + cw.round() as i32, bottom: ch.round() as i32 };
            flags |= DWM_TNP_RECTSOURCE;
        }
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: flags,
            rcDestination: r,
            rcSource: source,
            opacity: 255,
            fVisible: visible.into(),
            fSourceClientAreaOnly: false.into(),
        };
        let _ = DwmUpdateThumbnailProperties(thumb, &props);
    }
}

pub fn unregister(thumb: isize) {
    unsafe {
        let _ = DwmUnregisterThumbnail(thumb);
    }
}
