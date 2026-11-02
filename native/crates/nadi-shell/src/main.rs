//! NADI: widgets round the edges of the screen, merged into them. The plugins run on a thread of their own (`nadi-core`); this thread
//! draws the frame and hears the mouse.
//!
//!   nadi                         the frame
//!   nadi --preview out.png [w]   the frame drawn offscreen to a picture, from a file of widgets (or a sample), with one panel open
#![windows_subsystem = "windows"]

mod apps;
mod blocks;
mod builtin;
mod clip;
mod aura;
mod config;
mod director;
mod draw;
mod events;
mod eyes;
mod frame;
mod update;
mod cleanup;
mod adminkeys;
mod changes;
mod drop;
mod recycle;
mod perf;
mod gfx;
mod keyhook;
mod launcher;
mod menu;
mod modal;
mod icons;
mod icons_data;
mod icons_fluent;
mod radios;
mod procstats;
mod appaudio;
mod logo;
mod inputbox;
mod autostart;
mod images;
mod layout;
mod liquidgpu;
mod overlay;
mod paint;
mod peek;
mod preview;
mod settings;
mod shape;
mod taskbar;
mod thumbs;
mod tray;
mod trayhost;
mod viz;
mod volume;
mod winlist;
mod sysw;
mod audiodev;
mod explorer;
mod hotspot;
mod net;

use std::cell::RefCell;
use std::rc::Rc;
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::*;
use windows::Win32::UI::HiDpi::*;
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

const WM_MOUSELEAVE: u32 = 0x02A3;

fn run() -> windows::core::Result<()> {
    let data = nadi_core::paths::data_dir().unwrap_or_default();
    let _ = std::fs::create_dir_all(data.join("plugins"));
    // (one NADI for each place it keeps its things)
    let name = windows::core::HSTRING::from(format!("Local\\NADI-{:x}", data.to_string_lossy().bytes().fold(0u64, |h, b| h.wrapping_mul(31).wrapping_add(b as u64))));
    unsafe {
        let m = windows::Win32::System::Threading::CreateMutexW(None, true, &name)?;
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let _ = CloseHandle(m);
            // (started from File Explorer's menu: the NADI running takes what was left for it now)
            explorer::nudge();
            return Ok(());
        }
    }
    let msg_hwnd = tray::window()?;
    let pc = nadi_core::pc::Pc::new();
    nadi_core::pc::start(&pc);
    let wake_to = msg_hwnd.0 as isize;
    let (tx, rx) = nadi_core::host::spawn(data.join("plugins"), Some(data.join("plugins.json")), pc.clone(), move || unsafe {
        let _ = PostMessageW(HWND(wake_to as _), tray::WM_WAKE, WPARAM(0), LPARAM(0));
    });
    let g = gfx::Gfx::new()?;
    keyhook::install(msg_hwnd);
    let frame = Rc::new(RefCell::new(frame::Frame::new(g, pc, tx.clone(), msg_hwnd)));
    // (a taskbar left hidden by a NADI that did not exit cleanly comes back, unless it is wanted hidden)
    taskbar::recover(frame.borrow().taskbar_wanted_hidden());
    if frame.borrow().taskbar_wanted_hidden() || frame.borrow().tray_held() {
        spawn_watchdog();
    }
    // (Explorer started again: its taskbar is back, and the icon in its tray is gone)
    let taskbar_created = unsafe { RegisterWindowMessageW(windows::core::w!("TaskbarCreated")) };
    // (Windows tells this window of others: one that flashes for you, one that comes to the front)
    let shell_hook = unsafe {
        let _ = windows::Win32::UI::WindowsAndMessaging::RegisterShellHookWindow(msg_hwnd);
        RegisterWindowMessageW(windows::core::w!("SHELLHOOK"))
    };
    tray::add(msg_hwnd);
    unsafe {
        SetTimer(msg_hwnd, 1, 250, None);
    }
    let f = frame.clone();
    events::listen(move |hwnd, msg, wp, lp| {
        let mut fr = f.try_borrow_mut().ok()?;
        let t_msg = std::time::Instant::now();
        if hwnd == fr.msg_hwnd {
            let what = match msg {
                WM_TIMER => "",
                tray::WM_WAKE => "msg plugin updates",
                m if m == shell_hook => "msg shell event",
                frame::WM_JOB => "msg jobs done",
                trayhost::WM_TRAY_CHANGED => "msg tray changed",
                _ => "msg other",
            };
            let r = (|| -> Option<LRESULT> {
            match msg {
                WM_TIMER => fr.tick(),
                tray::WM_WAKE => {
                    let ups: Vec<_> = rx.try_iter().collect();
                    if !ups.is_empty() {
                        fr.apply(ups);
                    }
                }
                tray::WM_TRAY => {
                    let what = (lp.0 & 0xffff) as u32;
                    if what == WM_RBUTTONUP || what == WM_LBUTTONUP {
                        fr.open_menu_at_cursor();
                    }
                }
                WM_DISPLAYCHANGE | WM_DPICHANGED => fr.make_screens(),
                keyhook::WM_LAUNCHER => fr.toggle_launcher(),
                keyhook::WM_DOCK_KEY => fr.dock_key(wp.0),
                keyhook::WM_VOLUME_KEY => fr.volume_key(),
                keyhook::WM_DESK_KEY => fr.desk_key(wp.0),
                m if m == shell_hook => fr.shell_event(wp.0, lp.0),
                settings::WM_SETTINGS => fr.settings_messages(),
                drop::WM_DROP_HOVER => fr.drop_hover_changed(),
                drop::WM_DROPPED => fr.dropped(),
                update::WM_UPDATE => fr.update_changed(),
                changes::WM_WHATS_NEW => fr.whats_new(),
                tray::WM_PINS => fr.take_pins(),
                frame::WM_JOB => fr.jobs_done(),
                trayhost::WM_TRAY_CHANGED => fr.tray_changed(),
                // (the taskbar moved or changed size: the frame keeps clear of it)
                WM_SETTINGCHANGE => {
                    fr.areas();
                    fr.relayout();
                }
                m if m == taskbar_created => {
                    tray::add(hwnd);
                    fr.areas();
                    fr.relayout();
                }
                _ => return None,
            }
            Some(LRESULT(0))
            })();
            if !what.is_empty() {
                fr.prof_add(what, t_msg);
                fr.busy_ms += t_msg.elapsed().as_secs_f64() * 1000.0;
            }
            return r;
        }
        if fr.settings_hwnd() == Some(hwnd) {
            return match msg {
                WM_SIZE => {
                    fr.settings_resized();
                    Some(LRESULT(0))
                }
                WM_CLOSE => {
                    fr.close_settings();
                    Some(LRESULT(0))
                }
                _ => None,
            };
        }
        if fr.input_hwnd() == Some(hwnd) {
            return fr.input_message(msg, wp, lp);
        }
        if fr.launcher_hwnd() == Some(hwnd) {
            return fr.launcher_message(msg, wp, lp);
        }
        if fr.modal_hwnd() == Some(hwnd) {
            return fr.modal_message(msg, wp, lp);
        }
        let si = fr.is_frame_window(hwnd)?;
        let s = fr.screen_scale(si);
        match msg {
            WM_MOUSEMOVE => {
                let (x, y) = frame::point_of(lp, s);
                fr.mouse_move(si, x, y);
            }
            // (`NADI_TEST`: the mouse is posted by a test, and the real one is elsewhere: its leaving is not heard)
            WM_MOUSELEAVE if std::env::var_os("NADI_TEST").is_none() => fr.mouse_leave(si),
            WM_MOUSELEAVE => {}
            WM_LBUTTONUP => {
                let (x, y) = frame::point_of(lp, s);
                fr.click(si, x, y);
            }
            WM_MOUSEWHEEL => {
                let mut pt = POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
                unsafe {
                    let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
                }
                let delta = ((wp.0 >> 16) & 0xffff) as i16 as f32;
                fr.wheel(si, pt.x as f32 / s, pt.y as f32 / s, delta);
            }
            WM_RBUTTONUP => {
                let (x, y) = frame::point_of(lp, s);
                fr.context_menu(si, x, y);
            }
            WM_LBUTTONDOWN => {
                let (x, y) = frame::point_of(lp, s);
                fr.mouse_down(si, x, y);
            }
            WM_RBUTTONDOWN => {}
            // (near a side of a panel: the arrow to resize it by)
            WM_SETCURSOR => match fr.sizing_cursor().or_else(|| fr.grip_cursor()) {
                Some(c) => unsafe {
                    if let Ok(h) = LoadCursorW(None, c) {
                        SetCursor(h);
                    }
                    return Some(LRESULT(1));
                },
                None => return None,
            },
            _ => return None,
        }
        Some(LRESULT(0))
    });
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // (the settings page shut while NADI still runs: WebView2 left to tear it down as the process ends would call back into what is gone)
    frame.borrow_mut().close_settings();
    tray::remove(msg_hwnd);
    // (the tray given back to Explorer)
    trayhost::stop();
    // (the taskbar is given back: NADI is gone)
    if frame.borrow().taskbar_wanted_hidden() {
        taskbar::restore();
    }
    let _ = (VK_ESCAPE, tx);
    Ok(())
}

/// A second NADI that waits for the first to end, and gives the taskbar back if the first left it hidden (it crashed, or was killed).
pub fn spawn_watchdog() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let mut first = false;
    ONCE.call_once(|| first = true);
    if !first {
        return;
    }
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).args(["--watchdog", &std::process::id().to_string()]).spawn();
    }
}

fn watchdog(pid: u32) {
    use windows::Win32::System::Threading::*;
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_SYNCHRONIZE, false, pid) else { return };
        WaitForSingleObject(h, INFINITE);
        let _ = CloseHandle(h);
    }
    let mark = nadi_core::paths::data_dir().unwrap_or_default().join("taskbar-hidden");
    if mark.exists() {
        taskbar::restore();
    }
    // (it held the tray: the apps send their icons to Explorer's again)
    if trayhost::mark().exists() {
        trayhost::announce();
        let _ = std::fs::remove_file(trayhost::mark());
    }
}

fn main() {
    // (a panic: what and where, kept in NADI's folder, the newest last)
    std::panic::set_hook(Box::new(|info| {
        if let Some(p) = nadi_core::paths::data_dir().map(|d| d.join("crash.log")) {
            use std::io::Write;
            if std::fs::metadata(&p).is_ok_and(|m| m.len() > 200_000) {
                let _ = std::fs::remove_file(&p);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&p) {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let bt = std::backtrace::Backtrace::force_capture();
                let _ = writeln!(f, "{t} {} {info}\n{bt}\n", env!("CARGO_PKG_VERSION"));
            }
        }
    }));
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        // (drag and drop: files dropped on a widget)
        let _ = windows::Win32::System::Ole::OleInitialize(None);
    }
    let args: Vec<String> = std::env::args().collect();
    let r = match args.get(1).map(String::as_str) {
        Some("--watchdog") => {
            watchdog(args.get(2).and_then(|p| p.parse().ok()).unwrap_or(0));
            Ok(())
        }
        // (a check: every app the launcher knows, and how long it took to find them, written to a file)
        Some("--apps") => {
            let t = std::time::Instant::now();
            let list = apps::all();
            let text = format!("{} apps in {} ms
{}", list.len(), t.elapsed().as_millis(), list.iter().take(40).map(|a| format!("{}  |  {}", a.name, a.id)).collect::<Vec<_>>().join("
"));
            let _ = std::fs::write(args.get(2).map(String::as_str).unwrap_or("apps.txt"), text);
            Ok(())
        }
        Some("--preview-launcher") => (|| {
            let g = gfx::Gfx::new()?;
            let pc = nadi_core::pc::Pc::new();
            let (w, h, px) = launcher::preview(&g, &pc, args.get(3).map(String::as_str).unwrap_or(""))?;
            gfx::save_png(std::path::Path::new(args.get(2).map(String::as_str).unwrap_or("launcher.png")), w, h, &px)
                .map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))
        })(),
        Some("--preview-menu") => (|| {
            let g = gfx::Gfx::new()?;
            let pc = nadi_core::pc::Pc::new();
            let (w, h, px) = menu::preview(&g, &pc)?;
            gfx::save_png(std::path::Path::new(args.get(2).map(String::as_str).unwrap_or("menu.png")), w, h, &px).map_err(|e| windows::core::Error::new(windows::Win32::Foundation::E_FAIL, e.to_string()))
        })(),
        // (File Explorer's menu: what to pin is left for the NADI that runs, and this one ends; none running: this one starts)
        Some(to @ ("--pin-dock" | "--pin-apps" | "--unpin-dock" | "--unpin-apps")) => {
            if let Some(path) = args.get(2) {
                // (several chosen: all of them, as File Explorer (or the desktop) has them chosen)
                let what = to.trim_start_matches("--").trim_start_matches("pin-").replace("unpin-", "un");
                for p in explorer::selection_with(path) {
                    explorer::leave(&what, &p);
                }
            }
            run()
        }
        // (for checking: what is chosen where this path is)
        Some("--selection") => {
            println!("desktop: {:?}", explorer::desktop_selection());
            for p in explorer::selection_with(args.get(2).map(String::as_str).unwrap_or("")) {
                println!("{p}");
            }
            Ok(())
        }
        // (for checking: what is in the Recycle Bin; one of them restored)
        // (for checking: NADI's newest release, and whether it is newer than this one)
        Some("--update-check") => {
            let r = nadi_core::repo::latest_nadi();
            println!("{r:?} newer: {:?}", r.as_ref().ok().and_then(|o| o.as_ref()).map(|(v, _)| update::newer(v)));
            Ok(())
        }
        // (for checking: every version's changes, as GitHub gives them now; and the modal after an update from the version named)
        Some("--changes") => {
            changes::refresh();
            println!("{}", serde_json::to_string_pretty(&changes::json()).unwrap_or_default());
            if let Some(since) = std::env::args().nth(2) {
                changes::after_update(windows::Win32::Foundation::HWND::default(), since);
                std::thread::sleep(std::time::Duration::from_secs(4));
                println!("{}", changes::take_modal().map(|m| m.to_string()).unwrap_or_else(|| "no modal".into()));
            }
            Ok(())
        }
        Some("--bin-list") => {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            for it in recycle::check_list() {
                println!("{}|{}|{}", it.name, it.from, it.parse);
            }
            Ok(())
        }
        Some("--bin-restore") => {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            }
            println!("{:?}", recycle::check_put_back(args.get(2).map(String::as_str).unwrap_or("")));
            Ok(())
        }
        Some("--preview") => preview::preview(std::path::Path::new(args.get(2).map(String::as_str).unwrap_or("preview.png")), args.get(3).map(std::path::Path::new)),
        _ => run(),
    };
    if let Err(e) = r {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
