//! Plugins written in Luau (Roblox's Lua: typed, fast, made to run code it does not trust). A plugin is a script that defines
//! `handle(event)`: it is called with what happened (a table: the event's name, what it may see, its settings) and returns what the
//! island is to do (a table: its widgets, its banners, ...), the same shape the island has always spoken.
//!
//! The script runs in Luau's sandbox: its globals are read-only, there is no file, process or network access (what it may see comes in
//! the event, by its permissions), `require` does not reach the disk, and a call that runs too long or a script that takes too much memory
//! is stopped. The host gives it a small `nadi` library.

use mlua::serde::{DeserializeOptions, SerializeOptions};
use mlua::{Function, Lua, LuaOptions, LuaSerdeExt, StdLib, Table, Value as LuaValue, VmState};
use serde_json::Value;
use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// How much a plugin may use.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    /// one call (the plugin is stopped past it)
    pub call: Duration,
    /// all its memory
    pub memory: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { call: Duration::from_millis(50), memory: 16 * 1024 * 1024 }
    }
}

pub struct Script {
    lua: Lua,
    handle: Function,
    limits: Limits,
    deadline: Rc<Cell<Option<Instant>>>,
    /// what it printed (`nadi.log`) since it was last asked
    log: Rc<std::cell::RefCell<Vec<String>>>,
}

fn ser() -> SerializeOptions {
    // (a JSON null is Luau's nil: a plugin never meets a null of its own)
    SerializeOptions::new().serialize_none_to_null(false).serialize_unit_to_null(false)
}

fn de() -> DeserializeOptions {
    DeserializeOptions::new().deny_unsupported_types(false).deny_recursive_tables(true)
}

/// "3h 20m", "48 min", "2h"
fn duration(min: i64) -> String {
    let min = min.max(0);
    if min >= 60 {
        let (h, m) = (min / 60, min % 60);
        if m == 0 {
            format!("{h}h")
        } else {
            format!("{h}h {m:02}m")
        }
    } else {
        format!("{min} min")
    }
}

pub const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// The local date and time of a moment (`ms` since 1970, UTC), on a clock `tz` minutes from UTC:
/// (year, month 1-12, day, hour, minute, second, weekday 0 = Monday, days since 1970).
pub fn civil(ms: i64, tz: i64) -> (i64, i64, i64, i64, i64, i64, i64, i64) {
    let local = ms.div_euclid(1000) + tz * 60;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day, secs / 3600, (secs % 3600) / 60, secs % 60, (days + 3).rem_euclid(7), days)
}

/// The moment (ms since 1970, UTC) of a local date and time, on a clock `tz` minutes from UTC.
pub fn moment(year: i64, month: i64, day: i64, hour: i64, minute: i64, second: i64, tz: i64) -> i64 {
    let (y, m) = if month <= 2 { (year - 1, month + 9) } else { (year, month - 3) };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * m + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    ((days * 86_400 + hour * 3600 + minute * 60 + second) - tz * 60) * 1000
}

/// `2026-10-16T14:05:00.123456+00:00` (or `Z`, or a date alone) as ms since 1970
pub fn parse_iso(s: &str) -> Option<i64> {
    let (date, rest) = s.trim().split_once('T').unwrap_or((s.trim(), "00:00"));
    let mut d = date.split('-');
    let (y, m, day): (i64, i64, i64) = (d.next()?.parse().ok()?, d.next()?.parse().ok()?, d.next()?.parse().ok()?);
    let split = rest.find(['Z', '+', '-']).unwrap_or(rest.len());
    let (clock, zone) = rest.split_at(split);
    let mut c = clock.split(':');
    let (h, mi): (i64, i64) = (c.next()?.parse().ok()?, c.next()?.parse().ok()?);
    let sec: f64 = c.next().unwrap_or("0").parse().ok()?;
    let offset = match zone.chars().next() {
        Some(sign @ ('+' | '-')) => {
            let z = &zone[1..];
            let (zh, zm) = z.split_once(':').unwrap_or((z.get(..2).unwrap_or(z), z.get(2..).filter(|x| !x.is_empty()).unwrap_or("0")));
            let v = zh.parse::<i64>().ok()? * 60 + zm.parse::<i64>().ok()?;
            if sign == '+' { v } else { -v }
        }
        _ => 0,
    };
    Some(moment(y, m, day, h, mi, 0, offset) + (sec * 1000.0) as i64)
}

impl Script {
    /// Load a plugin's script (`name` is what errors call it).
    pub fn load(name: &str, source: &str, limits: Limits) -> Result<Script, String> {
        let lua = Lua::new_with(StdLib::ALL_SAFE, LuaOptions::default()).map_err(|e| e.to_string())?;
        lua.set_memory_limit(limits.memory).map_err(|e| e.to_string())?;
        let deadline: Rc<Cell<Option<Instant>>> = Rc::new(Cell::new(None));
        let d = deadline.clone();
        lua.set_interrupt(move |_| {
            if d.get().is_some_and(|t| Instant::now() > t) {
                return Err(mlua::Error::runtime("it ran for too long"));
            }
            Ok(VmState::Continue)
        });
        let log: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
        Self::library(&lua, log.clone()).map_err(|e| e.to_string())?;
        // (from here the globals cannot be changed, and each script has a table of its own to keep things in)
        lua.sandbox(true).map_err(|e| e.to_string())?;
        deadline.set(Some(Instant::now() + limits.call * 4));
        lua.load(source).set_name(format!("={name}")).exec().map_err(|e| e.to_string())?;
        deadline.set(None);
        let handle: Function = lua.globals().get("handle").map_err(|_| format!("{name} has no handle(event) function."))?;
        Ok(Script { lua, handle, limits, deadline, log })
    }

    /// What the host gives every plugin: `nadi.log(...)`, `nadi.duration(minutes)`, `nadi.null` (a JSON null, when one is wanted).
    fn library(lua: &Lua, log: Rc<std::cell::RefCell<Vec<String>>>) -> mlua::Result<()> {
        let nadi: Table = lua.create_table()?;
        nadi.set(
            "log",
            lua.create_function(move |_, args: mlua::Variadic<LuaValue>| {
                let line: Vec<String> = args.iter().map(|v| v.to_string().unwrap_or_else(|_| format!("{v:?}"))).collect();
                let mut l = log.borrow_mut();
                if l.len() < 200 {
                    l.push(line.join(" "));
                }
                Ok(())
            })?,
        )?;
        nadi.set("duration", lua.create_function(|_, m: f64| Ok(duration(m.round() as i64)))?)?;
        nadi.set("null", lua.null())?;
        // nadi.time(ms, tz): the local date and time of a moment; nadi.moment(year, month, day, hour, minute, second, tz): back
        nadi.set(
            "time",
            lua.create_function(|lua, (ms, tz): (f64, Option<f64>)| {
                let (year, month, day, hour, minute, second, weekday, days) = civil(ms as i64, tz.unwrap_or(0.0) as i64);
                let t = lua.create_table()?;
                t.set("year", year)?;
                t.set("month", month)?;
                t.set("day", day)?;
                t.set("hour", hour)?;
                t.set("minute", minute)?;
                t.set("second", second)?;
                t.set("weekday", weekday)?;
                t.set("days", days)?;
                t.set("wday", WEEKDAYS[weekday as usize])?;
                t.set("mon", MONTHS[(month - 1) as usize])?;
                Ok(t)
            })?,
        )?;
        nadi.set(
            "moment",
            lua.create_function(|_, a: (i64, i64, i64, Option<i64>, Option<i64>, Option<i64>, Option<f64>)| {
                Ok(moment(a.0, a.1, a.2, a.3.unwrap_or(0), a.4.unwrap_or(0), a.5.unwrap_or(0), a.6.unwrap_or(0.0) as i64) as f64)
            })?,
        )?;
        // nadi.when(ms, all_day): a moment inside a text, written by the shell in the viewer's own way ("Today 3:00 PM")
        nadi.set(
            "when",
            lua.create_function(|_, (ms, day): (f64, Option<bool>)| Ok(if day.unwrap_or(false) { format!("{{when:{}:day}}", ms as i64) } else { format!("{{when:{}}}", ms as i64) }))?,
        )?;
        // nadi.json.decode(text) / nadi.json.encode(value): what a request came back with, or what is sent
        let json_t: Table = lua.create_table()?;
        json_t.set(
            "decode",
            lua.create_function(|lua, text: String| match serde_json::from_str::<Value>(&text) {
                Ok(v) => Ok((lua.to_value_with(&v, ser())?, LuaValue::Nil)),
                Err(e) => Ok((LuaValue::Nil, LuaValue::String(lua.create_string(e.to_string())?))),
            })?,
        )?;
        json_t.set(
            "encode",
            lua.create_function(|lua, v: LuaValue| {
                let j: Value = lua.from_value_with(v, de())?;
                Ok(serde_json::to_string(&j).unwrap_or_default())
            })?,
        )?;
        nadi.set("json", json_t)?;
        // bytes: nadi.sha256(s) (the 32 bytes of its digest), nadi.random(n) (n random bytes), nadi.b64url(s) (unpadded), nadi.urlencode(s)
        nadi.set(
            "sha256",
            lua.create_function(|lua, s: mlua::LuaString| {
                use sha2::Digest;
                lua.create_string(sha2::Sha256::digest(&*s.as_bytes()).as_slice())
            })?,
        )?;
        nadi.set(
            "random",
            lua.create_function(|lua, n: usize| {
                let mut b = vec![0u8; n.min(256)];
                getrandom::getrandom(&mut b).map_err(|e| mlua::Error::runtime(e.to_string()))?;
                lua.create_string(&b)
            })?,
        )?;
        nadi.set(
            "b64url",
            lua.create_function(|_, s: mlua::LuaString| {
                use base64::Engine;
                Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&*s.as_bytes()))
            })?,
        )?;
        nadi.set(
            "urlencode",
            lua.create_function(|_, s: mlua::LuaString| {
                let mut out = String::new();
                for b in s.as_bytes().iter() {
                    match b {
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(*b as char),
                        _ => out.push_str(&format!("%{b:02X}")),
                    }
                }
                Ok(out)
            })?,
        )?;
        // nadi.ics(text, now): the events of a calendar feed, recurrences expanded around now ({ uid, summary, start_ms, end_ms, all_day })
        nadi.set(
            "ics",
            lua.create_function(|lua, (text, now): (String, Option<f64>)| {
                let events = match now {
                    Some(n) => crate::ics::parse_ics_at(&text, n as i64),
                    None => crate::ics::parse_ics(&text),
                };
                lua.to_value_with(&events, ser())
            })?,
        )?;
        // nadi.iso("2026-10-16T14:05:00.123+00:00"): the moment, ms since 1970 (nil when it is not one)
        nadi.set("iso", lua.create_function(|_, s: String| Ok(parse_iso(&s).map(|v| v as f64)))?)?;
        lua.globals().set("nadi", nadi)?;
        Ok(())
    }

    /// Call `handle(event)`: what it returns, as JSON (nothing: an empty object).
    pub fn call(&self, event: &Value) -> Result<Value, String> {
        let ev = self.lua.to_value_with(event, ser()).map_err(|e| e.to_string())?;
        self.deadline.set(Some(Instant::now() + self.limits.call));
        let out = self.handle.call::<LuaValue>(ev);
        self.deadline.set(None);
        let out = out.map_err(|e| first_line(&e.to_string()))?;
        match out {
            LuaValue::Nil => Ok(Value::Object(Default::default())),
            v => self.lua.from_value_with(v, de()).map_err(|e| e.to_string()),
        }
    }

    /// What it printed since this was last asked.
    pub fn take_log(&self) -> Vec<String> {
        std::mem::take(&mut *self.log.borrow_mut())
    }

    /// How much memory it uses now, in bytes.
    pub fn memory(&self) -> usize {
        self.lua.used_memory()
    }

    /// Run a test script against the plugin (`test.luau`): it sees the plugin's `handle`, and `check(cond, what)`/`eq(a, b, what)`. The
    /// failures, one line each (none: it passed).
    pub fn test(&self, name: &str, source: &str) -> Result<Vec<String>, String> {
        let fails: Rc<std::cell::RefCell<Vec<String>>> = Rc::default();
        let env = self.lua.create_table().map_err(|e| e.to_string())?;
        let meta = self.lua.create_table().map_err(|e| e.to_string())?;
        meta.set("__index", self.lua.globals()).map_err(|e| e.to_string())?;
        let _ = env.set_metatable(Some(meta));
        let f = fails.clone();
        env.set(
            "check",
            self.lua
                .create_function(move |_, (ok, what): (bool, Option<String>)| {
                    if !ok {
                        f.borrow_mut().push(what.unwrap_or_else(|| "a check failed".into()));
                    }
                    Ok(())
                })
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let f = fails.clone();
        env.set(
            "eq",
            self.lua
                .create_function(move |lua, (a, b, what): (LuaValue, LuaValue, Option<String>)| {
                    let ja: Value = lua.from_value_with(a, de()).unwrap_or(Value::Null);
                    let jb: Value = lua.from_value_with(b, de()).unwrap_or(Value::Null);
                    if ja != jb {
                        f.borrow_mut().push(format!("{}: {} is not {}", what.unwrap_or_else(|| "eq".into()), ja, jb));
                    }
                    Ok(())
                })
                .map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        self.deadline.set(Some(Instant::now() + Duration::from_secs(10)));
        let r = self.lua.load(source).set_name(format!("={name}")).set_environment(env).exec();
        self.deadline.set(None);
        r.map_err(|e| e.to_string())?;
        let out = fails.borrow().clone();
        Ok(out)
    }
}

/// The tests of a plugin (`test.luau`, which fills a table `tests` with functions): each runs against a plugin of its own, freshly
/// loaded, and says what failed. (name, failures)
pub fn run_tests(name: &str, main: &str, tests: &str, limits: Limits) -> Result<Vec<(String, Vec<String>)>, String> {
    // (which tests there are)
    let first = Script::load(name, main, limits)?;
    let names: Vec<String> = {
        let env = first.lua.create_table().map_err(|e| e.to_string())?;
        let meta = first.lua.create_table().map_err(|e| e.to_string())?;
        meta.set("__index", first.lua.globals()).map_err(|e| e.to_string())?;
        let _ = env.set_metatable(Some(meta));
        first.lua.load(tests).set_name("=test").set_environment(env.clone()).exec().map_err(|e| e.to_string())?;
        let t: Table = env.get("tests").map_err(|_| "test.luau has no table `tests`.".to_string())?;
        let mut v: Vec<String> = t.pairs::<String, Function>().filter_map(|p| p.ok().map(|(k, _)| k)).collect();
        v.sort();
        v
    };
    let mut out = Vec::new();
    for t in names {
        let s = Script::load(name, main, limits)?;
        let src = format!("{tests}
local t = tests[{t:?}]
t()");
        let fails = match s.test(&t, &src) {
            Ok(f) => f,
            Err(e) => vec![first_line(&e)],
        };
        out.push((t, fails));
    }
    Ok(out)
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or(s).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn script(src: &str) -> Script {
        Script::load("test", src, Limits::default()).unwrap()
    }

    #[test]
    fn an_event_goes_in_and_a_reply_comes_out() {
        let s = script(
            r#"
            function handle(ev)
                if ev.event == "battery" then
                    return { card = { title = "Battery", sub = nadi.duration(ev.battery.minutes_left) .. " left" }, widgets = {} }
                end
                return nil
            end"#,
        );
        let r = s.call(&json!({ "event": "battery", "battery": { "present": true, "minutes_left": 200, "rate_w": null } })).unwrap();
        assert_eq!(r["card"], json!({ "title": "Battery", "sub": "3h 20m left" }));
        // (nothing to say: an empty answer)
        assert_eq!(s.call(&json!({ "event": "tick" })).unwrap(), json!({}));
    }

    #[test]
    fn a_plugin_keeps_its_state_between_calls() {
        let s = script("local n = 0\nfunction handle(ev) n += 1 return { n = n } end");
        s.call(&json!({})).unwrap();
        assert_eq!(s.call(&json!({})).unwrap()["n"], 2);
    }

    #[test]
    fn it_cannot_reach_the_pc() {
        // (no io, no loadstring of bytecode, globals are read-only)
        let s = script("function handle(ev) return { io = io == nil, os_exec = os.execute == nil } end");
        assert_eq!(s.call(&json!({})).unwrap(), json!({ "io": true, "os_exec": true }));
        let s = script("function handle(ev) string.upper = nil return {} end");
        assert!(s.call(&json!({})).is_err(), "a global cannot be changed");
    }

    #[test]
    fn a_plugin_that_runs_too_long_or_eats_memory_is_stopped() {
        let s = Script::load("loop", "function handle(ev) while true do end end", Limits { call: Duration::from_millis(30), memory: 16 << 20 }).unwrap();
        let t = Instant::now();
        let e = s.call(&json!({})).unwrap_err();
        assert!(e.contains("too long"), "{e}");
        assert!(t.elapsed() < Duration::from_millis(500));
        let s = Script::load("eat", "function handle(ev) local t = {} for i = 1, 1e8 do t[i] = i end end", Limits { call: Duration::from_secs(5), memory: 4 << 20 }).unwrap();
        let e = s.call(&json!({})).unwrap_err();
        assert!(e.to_lowercase().contains("memory"), "{e}");
    }

    #[test]
    fn the_library_reads_dates_and_json() {
        let s = script(r#"
            function handle(ev)
                local t = nadi.time(1791559845000, 0)
                local v, err = nadi.json.decode('{"a":[1,2]}')
                local _, bad = nadi.json.decode('{')
                return { t = t.wday .. " " .. t.mon .. " " .. t.day .. " " .. t.hour .. ":" .. t.minute, back = nadi.moment(2026, 10, 9, 15, 30, 45, 0),
                    a = v.a[2], bad = bad ~= nil, enc = nadi.json.encode({ x = 1 }), w = nadi.when(5) }
            end"#);
        let r = s.call(&json!({})).unwrap();
        assert_eq!(r["t"], "Fri Oct 9 15:30");
        assert_eq!(r["back"].as_f64(), Some(1_791_559_845_000.0));
        assert_eq!((r["a"].as_f64(), r["bad"].as_bool()), (Some(2.0), Some(true)));
        assert_eq!((r["enc"].as_str(), r["w"].as_str()), (Some("{\"x\":1}"), Some("{when:5}")));
        let s = script(r#"function handle(ev) return { h = nadi.b64url(nadi.sha256("abc")), r = #nadi.random(32), u = nadi.urlencode("a b/c"),
            i = nadi.iso("2026-10-09T15:30:45.5+01:00"), z = nadi.iso("2026-10-09T15:30:45Z") } end"#);
        let r = s.call(&json!({})).unwrap();
        assert_eq!(r["h"], "ungWv48Bz-pBQUDeXa4iI7ADYaOWF3qctBD_YfIAFa0");
        assert_eq!((r["r"].as_f64(), r["u"].as_str()), (Some(32.0), Some("a%20b%2Fc")));
        assert_eq!((r["i"].as_f64(), r["z"].as_f64()), (Some(1_791_559_845_000.0 - 3_600_000.0 + 500.0), Some(1_791_559_845_000.0)));
    }

    #[test]
    fn a_script_without_handle_is_refused() {
        assert!(Script::load("x", "local a = 1", Limits::default()).err().unwrap().contains("handle"));
    }

    #[test]
    fn a_test_script_checks_the_plugin() {
        let s = script("function handle(ev) return { twice = ev.n * 2 } end");
        let fails = s.test("t", "eq(handle({ n = 2 }).twice, 4, 'twice')\neq(handle({ n = 3 }).twice, 7, 'wrong')").unwrap();
        assert_eq!(fails, vec!["wrong: 6 is not 7".to_string()]);
    }
}
