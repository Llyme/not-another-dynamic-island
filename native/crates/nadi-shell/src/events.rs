//! What the windows hear (the mouse over a piece, a timer), handed to whoever listens on this thread.

use std::cell::RefCell;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};

type Handler = Box<dyn FnMut(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT>>;

thread_local! {
    static HANDLER: RefCell<Option<Handler>> = RefCell::new(None);
}

pub fn listen(h: impl FnMut(HWND, u32, WPARAM, LPARAM) -> Option<LRESULT> + 'static) {
    HANDLER.with(|x| *x.borrow_mut() = Some(Box::new(h)));
}

pub fn window_message(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> Option<LRESULT> {
    // (as NADI ends, a window torn down late (WebView2's own) still sends: the handler is gone by then, so it is the system's)
    HANDLER
        .try_with(|x| {
            // (a message that comes while the handler runs, from something it called, is the system's)
            let mut slot = x.try_borrow_mut().ok()?;
            slot.as_mut().and_then(|h| h(hwnd, msg, wp, lp))
        })
        .ok()
        .flatten()
}
