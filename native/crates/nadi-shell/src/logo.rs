//! NADI's logo (a dark rounded square, the island's two green eyes): the icon of its tray icon and of its windows. Kept in the exe as
//! an .ico (each size a PNG); the size nearest above what is asked is made into an icon once.

use std::collections::HashMap;
use std::sync::Mutex;
use windows::Win32::UI::WindowsAndMessaging::{CreateIconFromResourceEx, HICON, LR_DEFAULTCOLOR};

static ICO: &[u8] = include_bytes!("../assets/nadi.ico");
static MADE: Mutex<Option<HashMap<i32, isize>>> = Mutex::new(None);

/// The logo as an icon `size` px square (a handle kept for the life of NADI).
pub fn icon(size: i32) -> HICON {
    let mut made = MADE.lock().unwrap();
    let made = made.get_or_insert_with(HashMap::new);
    if let Some(h) = made.get(&size) {
        return HICON(*h as *mut _);
    }
    let h = make(size).unwrap_or_default();
    made.insert(size, h.0 as isize);
    h
}

fn make(size: i32) -> Option<HICON> {
    let count = u16::from_le_bytes([*ICO.get(4)?, *ICO.get(5)?]) as usize;
    // (each entry: its width (0 is 256), ..., its bytes' length and where they start)
    let mut best: Option<(i32, usize, usize)> = None;
    for i in 0..count {
        let e = ICO.get(6 + 16 * i..6 + 16 * (i + 1))?;
        let w = if e[0] == 0 { 256 } else { e[0] as i32 };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let at = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        let better = match best {
            None => true,
            Some((bw, _, _)) => (w >= size && (bw < size || w < bw)) || (bw < size && w > bw),
        };
        if better {
            best = Some((w, at, len));
        }
    }
    let (_, at, len) = best?;
    let bytes = ICO.get(at..at + len)?;
    unsafe { CreateIconFromResourceEx(bytes, true, 0x0003_0000, size, size, LR_DEFAULTCOLOR).ok() }
}
