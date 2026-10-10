//! Notification mirror: other apps' Windows toast notifications (Discord, Viber, ...) shown on
//! the island, via the WinRT `UserNotificationListener`. A plugin (see native.rs). Every app's toasts are
//! shown, always. Polls every couple of seconds on its own
//! thread -- the listener's change event needs the same COM plumbing the media
//! bridge avoids, and a ~2 s delay on a chat ping is fine.
//!
//! Windows asks for permission once (Settings > Privacy & security >
//! Notifications > "Let apps access your notifications"). Until it is granted
//! this thread just retries quietly once a minute.

use crate::native::{manifest_of, old_flag, Adopted, Ctx, Gate, Native};
use crate::plugins::Manifest;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};
use windows::core::w;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, IsWindowVisible, SW_SHOWNORMAL,
};

const POLL_MS: u64 = 2000;
const RETRY_ACCESS_S: u64 = 60;

struct Parsed {
    app: String,
    title: String,
    body: String,
}

fn parse(n: &UserNotification) -> Option<Parsed> {
    let app = n.AppInfo().ok()?.DisplayInfo().ok()?.DisplayName().ok()?.to_string();
    let binding = n.Notification().ok()?.Visual().ok()?.GetBinding(&KnownNotificationBindings::ToastGeneric().ok()?).ok()?;
    let texts: Vec<String> = binding
        .GetTextElements()
        .ok()?
        .into_iter()
        .filter_map(|t| t.Text().ok())
        .map(|t| t.to_string())
        .filter(|t| !t.trim().is_empty())
        .collect();
    let mut it = texts.into_iter();
    let title = it.next().unwrap_or_default();
    let body = it.collect::<Vec<_>>().join("\n");
    Some(Parsed { app, title, body })
}

/// what clicking a mirrored toast should do, by the app that sent it
fn action_for_app(app: &str) -> Option<String> {
    let a = app.to_lowercase();
    if a.contains("discord") {
        Some("discord".into())
    } else if a.contains("viber") {
        Some("viber".into())
    } else {
        None
    }
}

fn run_toasts(ctx: Ctx) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut listener: Option<UserNotificationListener> = None;
        let mut seen: HashSet<u32> = HashSet::new();
        let mut primed = false; // first poll only records what is already there
        loop {
            if !ctx.on() {
                // (switched off: nothing is asked of Windows, and what was seen is forgotten)
                listener = None;
                seen.clear();
                primed = false;
                std::thread::sleep(Duration::from_secs(2));
                continue;
            }
            if listener.is_none() {
                let granted = UserNotificationListener::Current().ok().and_then(|l| {
                    let status = l.RequestAccessAsync().ok()?.get().ok()?;
                    (status == UserNotificationListenerAccessStatus::Allowed).then_some(l)
                });
                match granted {
                    Some(l) => listener = Some(l),
                    None => {
                        #[cfg(debug_assertions)]
                        eprintln!("notify_listener: notification access not granted");
                        std::thread::sleep(Duration::from_secs(RETRY_ACCESS_S));
                        continue;
                    }
                }
            }
            let l = listener.as_ref().unwrap();
            match l.GetNotificationsAsync(NotificationKinds::Toast).and_then(|op| op.get()) {
                Ok(list) => {
                    let mut current: HashSet<u32> = HashSet::new();
                    for n in list {
                        let Ok(id) = n.Id() else { continue };
                        current.insert(id);
                        if !seen.insert(id) || !primed {
                            continue;
                        }
                        let Some(p) = parse(&n) else { continue };
                        #[cfg(debug_assertions)]
                        eprintln!("notify_listener: {} | {} | {}", p.app, p.title, p.body);
                        let title = if p.title.is_empty() { p.app.clone() } else { p.title.clone() };
                        let body = if p.title.is_empty() || p.body.is_empty() {
                            if p.title.is_empty() { p.body.clone() } else { p.app.clone() }
                        } else {
                            p.body.clone()
                        };
                        // (muted: nothing; do not disturb: only in the list)
                        let gate = ctx.gate(0);
                        if gate == Gate::Muted {
                            continue;
                        }
                        let history = {
                            let mut notif = ctx.state.notif.lock().unwrap();
                            if gate == Gate::Held {
                                notif.push_priority(title, body, 0, true);
                            } else {
                                notif.push_action(title, body, action_for_app(&p.app));
                            }
                            notif.history()
                        };
                        ctx.emit("notification-history-tick", history);
                    }
                    // ids that left the action center can be forgotten
                    seen.retain(|id| current.contains(id));
                    primed = true;
                }
                Err(_e) => {
                    #[cfg(debug_assertions)]
                    eprintln!("notify_listener: GetNotificationsAsync failed: {_e}");
                    listener = None; // re-request access next round
                }
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }
    });
}

// -- Viber ---------------------------------------------------------------
// Viber draws its own popup (a small always-on-top Qt window titled "ViberPC"
// in the bottom-right corner) instead of a Windows toast, and hides its
// content from capture/accessibility, so the sender and text cannot be read.
// All that can be detected is that a popup appeared: show a generic banner.

const VIBER_POLL_MS: u64 = 400;

fn is_viber_popup(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        let mut title = [0u16; 16];
        let n = GetWindowTextW(h, &mut title) as usize;
        if String::from_utf16_lossy(&title[..n.min(title.len())]) != "ViberPC" {
            return false;
        }
        let mut cls = [0u16; 64];
        let n = GetClassNameW(h, &mut cls) as usize;
        let cls = String::from_utf16_lossy(&cls[..n.min(64)]);
        if !cls.starts_with("Qt6") {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() {
            return false;
        }
        let w = r.right - r.left;
        w > 50 && w < 600 // the popup, not the big chat window
    }
}

fn viber_popups() -> HashSet<isize> {
    unsafe extern "system" fn cb(h: HWND, lp: LPARAM) -> BOOL {
        let out = &mut *(lp.0 as *mut HashSet<isize>);
        if is_viber_popup(h) {
            out.insert(h.0 as isize);
        }
        BOOL(1)
    }
    let mut out: HashSet<isize> = HashSet::new();
    unsafe {
        let _ = EnumWindows(Some(cb), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn run_viber(ctx: Ctx) {
    std::thread::spawn(move || {
        let mut seen: HashSet<isize> = HashSet::new();
        loop {
            std::thread::sleep(Duration::from_millis(VIBER_POLL_MS));
            if !ctx.on() || !ctx.flag("viber", true) {
                seen.clear();
                continue;
            }
            let now = viber_popups();
            let fresh = now.iter().any(|h| !seen.contains(h));
            seen = now;
            if fresh && ctx.gate(0) != Gate::Muted {
                let held = ctx.gate(0) == Gate::Held;
                let history = {
                    let mut notif = ctx.state.notif.lock().unwrap();
                    if held {
                        notif.push_priority("Viber".into(), "New message".into(), 0, true);
                    } else {
                        notif.push_action("Viber".into(), "New message".into(), Some("viber".into()));
                    }
                    notif.history()
                };
                ctx.emit("notification-history-tick", history);
            }
        }
    });
}

/// Raises Viber (or starts it if it is only in the tray) through its URL
/// scheme, which hands the request to the running instance.
pub fn open_viber() {
    unsafe {
        ShellExecuteW(None, w!("open"), w!("viber://chats"), None, None, SW_SHOWNORMAL);
    }
}

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("toasts.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(_old: &Value, default_on: bool) -> Adopted {
    Adopted { on: old_flag(_old, "mirror_notifications", default_on), values: Default::default() }
}

pub struct Toasts;

impl Native for Toasts {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        run_toasts(ctx.clone());
        run_viber(ctx.clone());
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}
