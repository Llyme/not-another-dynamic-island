//! The settings window: a web page (WebView2), made when it is opened and gone when it is shut, so it costs nothing the rest of the time.
//! The page says what the person did (`postMessage`); NADI answers with all it needs to draw (`state`).

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use webview2_com::Microsoft::Web::WebView2::Win32::*;
use webview2_com::*;
use windows::core::{w, HSTRING, PCWSTR, PWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

pub const WM_SETTINGS: u32 = WM_APP + 4;
const PAGE: &str = include_str!("settings.html");

thread_local! {
    /// what the page said, for the frame to read (the frame's thread is told by `WM_SETTINGS`)
    static INBOX: RefCell<VecDeque<String>> = RefCell::new(VecDeque::new());
}

/// how many messages the page has sent (for checking)
pub static HEARD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn take_messages() -> Vec<String> {
    INBOX.with(|i| i.borrow_mut().drain(..).collect())
}

/// (what went wrong making the page, kept beside NADI's other things)
fn log(what: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(nadi_core::paths::data_dir().unwrap_or_default().join("settings.log")) {
        let _ = writeln!(f, "{what}");
    }
}

pub struct Settings {
    pub hwnd: HWND,
    controller: Rc<RefCell<Option<ICoreWebView2Controller>>>,
    webview: Rc<RefCell<Option<ICoreWebView2>>>,
    /// a state that came before the page was ready
    pending: Rc<RefCell<Option<String>>>,
}

unsafe extern "system" fn proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if let Some(r) = crate::events::window_message(hwnd, msg, wp, lp) {
        return r;
    }
    DefWindowProcW(hwnd, msg, wp, lp)
}

impl Settings {
    /// Make the window and its page; `notify` hears when the page says something.
    pub fn open(notify: HWND) -> windows::core::Result<Settings> {
        unsafe {
            let inst = GetModuleHandleW(None)?;
            let wc = WNDCLASSW {
                lpfnWndProc: Some(proc),
                hInstance: inst.into(),
                lpszClassName: w!("NadiSettings"),
                hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
                hIcon: crate::logo::icon(GetSystemMetrics(SM_CXICON)),
                hbrBackground: windows::Win32::Graphics::Gdi::CreateSolidBrush(COLORREF(0x000d0b0b)),
                ..Default::default()
            };
            RegisterClassW(&wc);
            let (w, h) = (1000, 700);
            let sx = GetSystemMetrics(SM_CXSCREEN);
            let sy = GetSystemMetrics(SM_CYSCREEN);
            let hwnd = CreateWindowExW(WINDOW_EX_STYLE(0), w!("NadiSettings"), w!("NADI Settings"), WS_OVERLAPPEDWINDOW, (sx - w) / 2, (sy - h) / 2, w, h, None, None, inst, None)?;
            // (a dark title bar)
            let dark: BOOL = TRUE;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const BOOL as *const _, 4);
            // (`NADI_TEST`: shown without taking the keyboard from whoever has it)
            if std::env::var_os("NADI_TEST").is_some() {
                let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            } else {
                let _ = ShowWindow(hwnd, SW_SHOW);
                let _ = SetForegroundWindow(hwnd);
            }
            let s = Settings { hwnd, controller: Rc::default(), webview: Rc::default(), pending: Rc::default() };
            s.make_view(notify)?;
            Ok(s)
        }
    }

    fn make_view(&self, notify: HWND) -> windows::core::Result<()> {
        let hwnd = self.hwnd;
        let controller = self.controller.clone();
        let webview = self.webview.clone();
        let pending = self.pending.clone();
        let data = nadi_core::paths::data_dir().unwrap_or_default().join("webview");
        let data_w = HSTRING::from(data.to_string_lossy().as_ref());
        let notify_raw = notify.0 as isize;
        let handler = CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |err: windows::core::Result<()>, env: Option<ICoreWebView2Environment>| {
            if let Err(e) = &err {
                log(&format!("environment: {e}"));
            }
            let Some(env) = env else { return Ok(()) };
            let (controller, webview, pending) = (controller.clone(), webview.clone(), pending.clone());
            let done = CreateCoreWebView2ControllerCompletedHandler::create(Box::new(move |err: windows::core::Result<()>, c: Option<ICoreWebView2Controller>| {
                if let Err(e) = &err {
                    log(&format!("controller: {e}"));
                }
                let Some(c) = c else { return Ok(()) };
                unsafe {
                    let mut r = RECT::default();
                    let _ = GetClientRect(hwnd, &mut r);
                    c.SetBounds(r)?;
                    if let Ok(c2) = windows::core::Interface::cast::<ICoreWebView2Controller2>(&c) {
                        let _ = c2.SetDefaultBackgroundColor(COREWEBVIEW2_COLOR { A: 255, R: 11, G: 11, B: 13 });
                    }
                    let wv = c.CoreWebView2()?;
                    if let Ok(s) = wv.Settings() {
                        let _ = s.SetAreDevToolsEnabled(false);
                        let _ = s.SetIsStatusBarEnabled(false);
                        let _ = s.SetAreDefaultContextMenusEnabled(false);
                    }
                    let mut token = windows::Win32::System::WinRT::EventRegistrationToken::default();
                    wv.add_WebMessageReceived(
                        &WebMessageReceivedEventHandler::create(Box::new(move |_wv, args: Option<ICoreWebView2WebMessageReceivedEventArgs>| {
                            if let Some(args) = args {
                                let mut text = PWSTR::null();
                                if args.TryGetWebMessageAsString(&mut text).is_ok() {
                                    let s = take(text);
                                    HEARD.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                    INBOX.with(|i| i.borrow_mut().push_back(s));
                                    let _ = PostMessageW(HWND(notify_raw as *mut _), WM_SETTINGS, WPARAM(0), LPARAM(0));
                                }
                            }
                            Ok(())
                        })),
                        &mut token,
                    )?;
                    if let Err(e) = wv.NavigateToString(&HSTRING::from(PAGE)) {
                        log(&format!("navigate: {e}"));
                    }
                    log("page asked for");
                    if let Some(p) = pending.borrow_mut().take() {
                        let _ = wv.PostWebMessageAsJson(&HSTRING::from(p));
                    }
                    *webview.borrow_mut() = Some(wv);
                    *controller.borrow_mut() = Some(c);
                }
                Ok(())
            }));
            unsafe { env.CreateCoreWebView2Controller(hwnd, &done) }
        }));
        let r = unsafe { CreateCoreWebView2EnvironmentWithOptions(PCWSTR::null(), &data_w, None, &handler) };
        if let Err(e) = &r {
            log(&format!("create: {e}"));
        }
        r
    }

    /// Tell the page (JSON).
    pub fn post(&self, json: &str) {
        match &*self.webview.borrow() {
            Some(wv) => unsafe {
                let _ = wv.PostWebMessageAsJson(&HSTRING::from(json));
            },
            None => *self.pending.borrow_mut() = Some(json.to_string()),
        }
    }

    /// The window changed size: the page follows.
    pub fn resized(&self) {
        if let Some(c) = &*self.controller.borrow() {
            unsafe {
                let mut r = RECT::default();
                let _ = GetClientRect(self.hwnd, &mut r);
                let _ = c.SetBounds(r);
            }
        }
    }

    pub fn raise(&self) {
        unsafe {
            if IsIconic(self.hwnd).as_bool() {
                let _ = ShowWindow(self.hwnd, SW_RESTORE);
            }
            let _ = SetForegroundWindow(self.hwnd);
        }
    }

    /// Shut: the page and its browser go.
    pub fn close(self) {
        if let Some(c) = self.controller.borrow_mut().take() {
            unsafe {
                let _ = c.Close();
            }
        }
        self.webview.borrow_mut().take();
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

fn take(p: PWSTR) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe {
        let s = p.to_string().unwrap_or_default();
        windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}
