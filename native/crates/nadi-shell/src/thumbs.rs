//! Live pictures of windows (DWM thumbnails), drawn by the compositor into a box of a frame window: what a window shows, as it changes,
//! for nothing on this side.

use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;

/// A live picture of `src`, shown in `dest` (none: it could not be made).
pub fn register(dest: HWND, src: isize) -> Option<isize> {
    unsafe { DwmRegisterThumbnail(dest, HWND(src as *mut _)).ok() }
}

/// Put it in a box (physical px, in the window), kept in proportion and centred; or hide it.
pub fn place(thumb: isize, r: RECT, visible: bool) {
    unsafe {
        let size = DwmQueryThumbnailSourceSize(thumb).unwrap_or_default();
        let (bw, bh) = ((r.right - r.left) as f32, (r.bottom - r.top) as f32);
        let mut dest = r;
        if size.cx > 0 && size.cy > 0 {
            let k = (bw / size.cx as f32).min(bh / size.cy as f32);
            let (w, h) = (size.cx as f32 * k, size.cy as f32 * k);
            dest = RECT { left: r.left + ((bw - w) / 2.0) as i32, top: r.top + ((bh - h) / 2.0) as i32, right: r.left + ((bw + w) / 2.0) as i32, bottom: r.top + ((bh + h) / 2.0) as i32 };
        }
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: dest,
            rcSource: RECT::default(),
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
