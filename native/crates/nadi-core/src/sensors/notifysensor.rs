//! What other apps notify: their Windows toasts (chat apps, mail, ...), read through the WinRT `UserNotificationListener`, for
//! a plugin that may see them (`notifications`). Polled every couple of seconds on a thread of its own, and only while such a
//! plugin is on: nothing is asked of Windows otherwise. Windows asks the user once (Settings > Privacy & security >
//! Notifications > "Let apps access your notifications"); until it is granted this retries quietly once a minute.
//!
//! The plugins are told by `pluginwasm.rs`: each toast has a number, and a module is told of the ones it has not seen. Each toast
//! carries its `id`; one that leaves Windows' list (read elsewhere, dismissed, withdrawn by its app) comes again as `{ id, gone }`.
//! A plugin may ask for its toasts to be taken off the list (`toast_dismiss`).

use crate::pc::Pc;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::UI::Notifications::Management::{UserNotificationListener, UserNotificationListenerAccessStatus};
use windows::UI::Notifications::{KnownNotificationBindings, NotificationKinds, UserNotification};

const POLL_MS: u64 = 2000;
const RETRY_ACCESS_S: u64 = 60;
/// toasts kept for the plugins that have not heard of them yet
const KEEP: usize = 40;

fn parse(n: &UserNotification) -> Option<Value> {
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
    Some(json!({ "app": app, "title": title, "body": body }))
}

pub fn spawn(state: Arc<Pc>) {
    std::thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        let mut listener: Option<UserNotificationListener> = None;
        let mut seen: HashSet<u32> = HashSet::new();
        let mut primed = false; // the first poll only records what is already there
        loop {
            if !state.want.notifications.load(Ordering::Relaxed) {
                // (nothing is asked of Windows, and what was seen is forgotten)
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
                        if let Some(mut t) = parse(&n) {
                            let seq = state.toast_seq.fetch_add(1, Ordering::Relaxed) + 1;
                            t["seq"] = json!(seq);
                            t["id"] = json!(id);
                            let mut q = state.toasts.lock().unwrap();
                            q.push(t);
                            let over = q.len().saturating_sub(KEEP);
                            q.drain(..over);
                        }
                    }
                    // ids that left the action center: told as gone, and forgotten
                    let gone: Vec<u32> = seen.iter().filter(|id| !current.contains(id)).copied().collect();
                    if primed && !gone.is_empty() {
                        let mut q = state.toasts.lock().unwrap();
                        for id in &gone {
                            let seq = state.toast_seq.fetch_add(1, Ordering::Relaxed) + 1;
                            q.push(json!({ "id": id, "gone": true, "seq": seq }));
                        }
                        let over = q.len().saturating_sub(KEEP);
                        q.drain(..over);
                    }
                    seen.retain(|id| current.contains(id));
                    // (the ones a plugin asked to be taken off the list)
                    let asked: Vec<u32> = std::mem::take(&mut *state.toast_dismiss.lock().unwrap());
                    for id in asked {
                        let _ = l.RemoveNotification(id);
                    }
                    primed = true;
                }
                Err(_) => listener = None, // re-request access next round
            }
            std::thread::sleep(Duration::from_millis(POLL_MS));
        }
    });
}
