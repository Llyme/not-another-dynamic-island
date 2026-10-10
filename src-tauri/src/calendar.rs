//! The calendar: a main feature of the island. It keeps the events that any source has brought (a plugin: the ICS
//! feed, or a module that knows where else a calendar lives), shows them in the hub's month view, and tells the UI
//! when they change. Where the events come from, and what is made of them (cards, reminders), is for plugins.

use crate::IslandState;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

/// events kept for the month view, over all the sources
pub const MAX_EVENTS: usize = 4000;

#[derive(Clone, Serialize)]
pub struct CalendarEvent {
    pub uid: String,
    pub summary: String,
    pub start_ms: i64,
    /// exclusive end (== start for zero-length events)
    pub end_ms: i64,
    pub all_day: bool,
}

#[derive(Clone, Serialize, Default, Debug, PartialEq)]
pub struct CalendarSnapshot {
    /// some source is on
    pub configured: bool,
    pub error: Option<String>,
    pub events: Vec<CalendarEvent>,
}

impl std::fmt::Debug for CalendarEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}@{}", self.summary, self.start_ms)
    }
}

impl PartialEq for CalendarEvent {
    fn eq(&self, o: &Self) -> bool {
        self.uid == o.uid && self.start_ms == o.start_ms && self.end_ms == o.end_ms && self.summary == o.summary
    }
}

struct Source {
    error: Option<String>,
    events: Vec<CalendarEvent>,
}

#[derive(Default)]
pub struct CalendarState {
    sources: Mutex<HashMap<String, Source>>,
    /// counts every change (a plugin that reads the calendar asks only when this moved)
    version: AtomicU64,
}

impl CalendarState {
    /// What every source has brought, in one list by time. A source with a problem says so (the first one shown).
    pub fn snapshot(&self) -> CalendarSnapshot {
        let sources = self.sources.lock().unwrap();
        let mut ids: Vec<&String> = sources.keys().collect();
        ids.sort();
        let mut events: Vec<CalendarEvent> = Vec::new();
        let mut error = None;
        for id in ids {
            let s = &sources[id];
            if error.is_none() {
                error = s.error.clone();
            }
            events.extend(s.events.iter().cloned());
        }
        events.sort_by_key(|e| e.start_ms);
        events.truncate(MAX_EVENTS);
        CalendarSnapshot { configured: !sources.is_empty(), error, events }
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Relaxed)
    }

    fn put(&self, id: &str, source: Option<Source>) {
        self.version.fetch_add(1, Ordering::Relaxed);
        let mut sources = self.sources.lock().unwrap();
        match source {
            Some(s) => {
                sources.insert(id.to_string(), s);
            }
            None => {
                sources.remove(id);
            }
        }
    }
}

/// A source brings its events (or says why it cannot): the month view shows them from now on.
pub fn set_source(app: &AppHandle, state: &IslandState, id: &str, result: Result<Vec<CalendarEvent>, String>) {
    state.calendar.put(
        id,
        Some(match result {
            Ok(events) => Source { error: None, events },
            Err(e) => Source { error: Some(e), events: Vec::new() },
        }),
    );
    let _ = app.emit("calendar-tick", state.calendar.snapshot());
}

/// A source has gone (switched off): its events go with it.
pub fn drop_source(app: &AppHandle, state: &IslandState, id: &str) {
    state.calendar.put(id, None);
    let _ = app.emit("calendar-tick", state.calendar.snapshot());
}

const MAX_SUMMARY: usize = 200;
/// the most events one plugin may bring
const MAX_PER_SOURCE: usize = 500;

/// The events a module brought, as JSON: at most 500, each with a summary (cut at 200 characters) and a start.
pub fn events_from_json(v: &Value) -> Vec<CalendarEvent> {
    let mut out: Vec<CalendarEvent> = Vec::new();
    for e in v.as_array().into_iter().flatten().take(MAX_PER_SOURCE) {
        let summary: String = e.get("summary").and_then(|s| s.as_str()).unwrap_or("").trim().chars().take(MAX_SUMMARY).collect();
        let Some(start) = e.get("start_ms").and_then(|s| s.as_i64()) else { continue };
        if summary.is_empty() {
            continue;
        }
        let end = e.get("end_ms").and_then(|s| s.as_i64()).filter(|end| *end >= start).unwrap_or(start);
        let uid = e.get("uid").and_then(|s| s.as_str()).map(str::to_string).unwrap_or_else(|| format!("{summary}|{start}"));
        out.push(CalendarEvent { uid, summary, start_ms: start, end_ms: end, all_day: e.get("all_day").and_then(|b| b.as_bool()).unwrap_or(false) });
    }
    out
}

#[tauri::command]
pub fn get_calendar(window: WebviewWindow) -> CalendarSnapshot {
    window.state::<Arc<IslandState>>().calendar.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(uid: &str, start: i64) -> CalendarEvent {
        CalendarEvent { uid: uid.into(), summary: uid.into(), start_ms: start, end_ms: start + 1, all_day: false }
    }

    #[test]
    fn events_from_a_module_are_cleaned() {
        let v = serde_json::json!([
            { "summary": "Raid night", "start_ms": 1000, "end_ms": 5000, "uid": "r1" },
            { "summary": "   ", "start_ms": 1 },
            { "summary": "No start" },
            { "summary": "Backwards", "start_ms": 900, "end_ms": 100, "all_day": true }
        ]);
        let ev = events_from_json(&v);
        assert_eq!(ev.len(), 2);
        assert_eq!((ev[0].uid.as_str(), ev[0].end_ms), ("r1", 5000));
        // an end before the start is a zero-length event
        assert_eq!((ev[1].start_ms, ev[1].end_ms, ev[1].all_day), (900, 900, true));
        assert!(events_from_json(&serde_json::json!("not a list")).is_empty());
    }

    #[test]
    fn events_of_every_source_are_one_list_by_time() {
        let c = CalendarState::default();
        assert_eq!(c.snapshot(), CalendarSnapshot::default());
        c.put("a", Some(Source { error: None, events: vec![ev("late", 30), ev("early", 10)] }));
        c.put("b", Some(Source { error: Some("down".into()), events: vec![ev("mid", 20)] }));
        let s = c.snapshot();
        assert!(s.configured);
        assert_eq!(s.error.as_deref(), Some("down"));
        assert_eq!(s.events.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), vec!["early", "mid", "late"]);
        c.put("a", None);
        assert_eq!(c.snapshot().events.len(), 1);
        c.put("b", None);
        assert!(!c.snapshot().configured);
    }
}
