//! Agenda: what is coming up, from the island's calendar, whoever brought the events (a plugin like ICS calendar). A card
//! in the hub with the next events, and a banner shortly before one starts.
//!
//! This plugin ships with NADI, and is written as any plugin can be: it asks for the `calendar` permission, is told when the
//! calendar changes, and asks the island to wake it every 15 seconds to look at the clock.

use nadi_plugin::*;
use std::cell::RefCell;
use std::collections::HashSet;

/// events the card lists
const LISTED: usize = 6;
/// how often it looks at the clock, ms
const LOOK_MS: u64 = 15_000;
/// the card sits among the others
const RANK: i32 = 45;

#[derive(Default)]
struct State {
    events: Vec<CalEvent>,
    told: HashSet<String>,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

/// the events that have not finished (an hour of grace after the end), first ones first
fn upcoming(events: &[CalEvent], now_ms: i64) -> Vec<&CalEvent> {
    events.iter().filter(|e| e.end_ms >= now_ms - 3_600_000).take(LISTED).collect()
}

/// the events that start within `lead_ms` from now and were not yet told (each is told once)
fn due<'a>(events: &'a [CalEvent], now_ms: i64, lead_ms: i64, told: &mut HashSet<String>) -> Vec<&'a CalEvent> {
    events
        .iter()
        .filter(|e| {
            let delta = e.start_ms - now_ms;
            delta > 0 && delta <= lead_ms
        })
        .filter(|e| told.insert(e.uid.clone()))
        .collect()
}

/// the first event leads the card; the others are what it opens to
fn card(list: &[&CalEvent]) -> Card {
    let first = list[0];
    let rest: Vec<(String, String, Option<f32>)> = list[1..].iter().map(|e| (e.summary.clone(), when(e.start_ms, e.all_day), None)).collect();
    let mut c = Card::new().icon("calendar").rank(RANK).title(first.summary.clone()).sub(when(first.start_ms, first.all_day));
    if !rest.is_empty() {
        c = c.list(&rest);
    }
    c
}

fn handle(ev: &Event) -> Reply {
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        match ev.event.as_str() {
            "calendar" => s.events = ev.events.clone(),
            "start" => {
                // (switched on again: what was told before is told again if it is still ahead)
                s.told.clear();
            }
            _ => {}
        }
        let now = ev.now as i64;
        let list = upcoming(&s.events, now);
        let mut r = if list.is_empty() { Reply::clear() } else { Reply::card(card(&list)) };
        r = r.wake_in(LOOK_MS);
        if ev.setting_bool("remind", true) {
            let lead_ms = ev.setting_f64("lead_min", 15.0).max(1.0) as i64 * 60_000;
            let events = s.events.clone();
            for e in due(&events, now, lead_ms, &mut s.told) {
                r = r.say(format!("{} in {} min", e.summary, ((e.start_ms - now) / 60_000).max(1)), 40);
            }
        }
        r
    })
}

plugin!(handle);

#[cfg(test)]
mod tests {
    use super::*;

    fn e(uid: &str, start: i64, end: i64) -> CalEvent {
        CalEvent::new(uid, uid, start, end)
    }

    #[test]
    fn a_reminder_comes_once_and_only_inside_the_lead() {
        let events = vec![e("soon", 10 * 60_000, 11 * 60_000), e("later", 90 * 60_000, 91 * 60_000), e("past", -5 * 60_000, -4 * 60_000)];
        let mut told = HashSet::new();
        let lead = 15 * 60_000;
        let first = due(&events, 0, lead, &mut told);
        assert_eq!(first.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), vec!["soon"]);
        assert!(due(&events, 60_000, lead, &mut told).is_empty());
        // later comes into the lead
        let next = due(&events, 80 * 60_000, lead, &mut told);
        assert_eq!(next.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), vec!["later"]);
    }

    #[test]
    fn the_card_lists_what_has_not_finished() {
        let hour = 3_600_000;
        let events = vec![e("old", -10 * hour, -9 * hour), e("just ended", -2 * hour, -hour - 1), e("going", -hour / 2, hour / 2), e("next", hour, 2 * hour)];
        let list = upcoming(&events, 0);
        assert_eq!(list.iter().map(|e| e.uid.as_str()).collect::<Vec<_>>(), vec!["going", "next"]);
    }

    #[test]
    fn the_module_tells_a_reminder_and_draws_the_card() {
        STATE.with(|s| *s.borrow_mut() = State::default());
        let events = vec![e("standup", 10 * 60_000, 40 * 60_000), e("lunch", 300 * 60_000, 360 * 60_000)];
        let ev = |kind: &str, now: u64| Event { event: kind.into(), now, events: events.clone(), settings: json!({ "remind": true, "lead_min": 15 }), ..Event::default() };
        let r = handle(&ev("calendar", 0)).to_json();
        assert_eq!(r["card"]["title"], "standup");
        assert_eq!(r["card"]["sub"], "{when:600000}");
        assert_eq!(r["card"]["rank"], 45);
        assert_eq!(r["card"]["body"][0]["rows"][0]["title"], "lunch");
        assert_eq!(r["say"][0]["text"], "standup in 10 min");
        assert_eq!(r["wake_ms"], 15_000);
        // the next call tells nothing again
        assert_eq!(handle(&ev("tick", 15_000)).to_json()["say"].as_array().unwrap().len(), 0);
        // the calendar is empty: the card goes
        let empty = Event { event: "calendar".into(), now: 0, events: vec![], settings: json!({}), ..Event::default() };
        assert_eq!(handle(&empty).to_json()["card"], Value::Null);
        // with the reminder switched off it says nothing
        STATE.with(|s| *s.borrow_mut() = State::default());
        let quiet = Event { settings: json!({ "remind": false, "lead_min": 15 }), ..ev("calendar", 0) };
        assert_eq!(handle(&quiet).to_json()["say"].as_array().unwrap().len(), 0);
    }
}
