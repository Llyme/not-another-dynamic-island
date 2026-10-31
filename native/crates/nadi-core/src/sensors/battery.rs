//! The battery, for a module that may see it (`permissions.battery`): how full it is, whether the charger is in, how long it will last or
//! take to fill, how fast it charges or drains, how worn it is. Windows says it: the system's power status for the number, the charger and
//! battery saver, and the battery's own report (Windows.Devices.Power) for the rate and the capacities. Nothing is kept or sent anywhere,
//! and a module cannot change anything of the power (it cannot turn battery saver on).

use serde_json::{json, Value};
use windows::Devices::Power::Battery;
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};

/// what Windows said, before it is made into what a module is told
#[derive(Default, Clone, Debug, PartialEq)]
pub struct Raw {
    /// `BatteryFlag`: 128 is no battery, 255 is not known, 8 is charging
    pub flag: u8,
    /// 0 to 100, or 255 when not known
    pub percent: u8,
    /// `ACLineStatus`: 1 is the charger in
    pub ac: u8,
    /// `SystemStatusFlag`: 1 is battery saver on
    pub saver: u8,
    /// seconds left on battery, or `u32::MAX` when not known
    pub life_s: u32,
    /// from the battery's own report: milliwatts in (positive) or out (negative), and milliwatt-hours
    pub rate_mw: Option<i32>,
    pub design_mwh: Option<i32>,
    pub full_mwh: Option<i32>,
    pub remaining_mwh: Option<i32>,
}

/// What a module is told: `{ present: false }` for a PC with no battery.
pub fn shape(r: &Raw) -> Value {
    if r.flag == 128 || (r.flag == 255 && r.percent > 100) {
        return json!({ "present": false });
    }
    let percent = (r.percent <= 100).then_some(r.percent as f64);
    let plugged = r.ac == 1;
    // (charging: the flag says so, or the charger is in and the battery is taking power)
    let charging = r.flag & 8 != 0 || (plugged && r.rate_mw.map_or(false, |w| w > 0));
    let rate_w = r.rate_mw.map(|w| (w as f64 / 100.0).round() / 10.0);
    // (how long it will last: the system's guess, else what is left at the rate it goes)
    let minutes_left = if charging || plugged {
        None
    } else if r.life_s != u32::MAX {
        Some((r.life_s / 60) as i64)
    } else {
        match (r.remaining_mwh, r.rate_mw) {
            (Some(rem), Some(rate)) if rate < 0 && rem > 0 => Some((rem as f64 / -(rate as f64) * 60.0).round() as i64),
            _ => None,
        }
    };
    let minutes_to_full = match (charging, r.full_mwh, r.remaining_mwh, r.rate_mw) {
        (true, Some(full), Some(rem), Some(rate)) if rate > 0 && full > rem => Some(((full - rem) as f64 / rate as f64 * 60.0).round() as i64),
        _ => None,
    };
    let health = match (r.full_mwh, r.design_mwh) {
        (Some(full), Some(design)) if design > 0 && full > 0 => Some((full as f64 / design as f64 * 100.0).round().min(100.0)),
        _ => None,
    };
    json!({
        "present": true,
        "percent": percent,
        "plugged": plugged,
        "charging": charging,
        "saver": r.saver == 1,
        "minutes_left": minutes_left,
        "minutes_to_full": minutes_to_full,
        "rate_w": rate_w,
        "health": health,
        "full_wh": r.full_mwh.map(|v| (v as f64 / 100.0).round() / 10.0),
        "design_wh": r.design_mwh.map(|v| (v as f64 / 100.0).round() / 10.0),
    })
}

thread_local! {
    static COM: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// the battery's own report: the rate, and what it holds
fn report() -> (Option<i32>, Option<i32>, Option<i32>, Option<i32>) {
    COM.with(|c| {
        if !c.get() {
            c.set(true);
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
        }
    });
    let Ok(r) = Battery::AggregateBattery().and_then(|b| b.GetReport()) else { return (None, None, None, None) };
    let get = |v: windows::core::Result<windows::Foundation::IReference<i32>>| v.ok().and_then(|x| x.Value().ok());
    (get(r.ChargeRateInMilliwatts()), get(r.DesignCapacityInMilliwattHours()), get(r.FullChargeCapacityInMilliwattHours()), get(r.RemainingCapacityInMilliwattHours()))
}

/// the battery now, as a module is told it
pub fn read() -> Value {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() {
        return json!({ "present": false });
    }
    let mut raw = Raw { flag: s.BatteryFlag, percent: s.BatteryLifePercent, ac: s.ACLineStatus, saver: s.SystemStatusFlag, life_s: s.BatteryLifeTime, ..Raw::default() };
    if raw.flag != 128 {
        let (rate, design, full, remaining) = report();
        raw.rate_mw = rate;
        raw.design_mwh = design;
        raw.full_mwh = full;
        raw.remaining_mwh = remaining;
    }
    shape(&raw)
}

/// Is this reading news? (the number, the charger, charging or battery saver changed, or the battery came or went: the rate and the time
/// left move all the time, and are only sent along with something else, or at the next regular time)
pub fn changed(old: &Value, new: &Value) -> bool {
    ["present", "percent", "plugged", "charging", "saver"].iter().any(|k| old.get(k) != new.get(k))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw() -> Raw {
        Raw { flag: 0, percent: 64, ac: 0, saver: 0, life_s: 12_000, ..Raw::default() }
    }

    #[test]
    fn a_pc_with_no_battery_says_so() {
        assert_eq!(shape(&Raw { flag: 128, percent: 255, ..Raw::default() }), json!({ "present": false }));
        assert_eq!(shape(&Raw { flag: 255, percent: 255, ..Raw::default() }), json!({ "present": false }));
    }

    #[test]
    fn on_battery_it_says_how_long_is_left() {
        let b = shape(&raw());
        assert_eq!((b["percent"].as_f64(), b["plugged"].as_bool(), b["charging"].as_bool(), b["minutes_left"].as_i64()), (Some(64.0), Some(false), Some(false), Some(200)));
        // (no guess from the system: what is left, at the rate it goes)
        let b = shape(&Raw { life_s: u32::MAX, rate_mw: Some(-7400), remaining_mwh: Some(24_000), ..raw() });
        assert_eq!(b["minutes_left"].as_i64(), Some(195));
        assert_eq!(b["rate_w"].as_f64(), Some(-7.4));
        // (nothing at all to go by)
        assert!(shape(&Raw { life_s: u32::MAX, ..raw() })["minutes_left"].is_null());
    }

    #[test]
    fn on_the_charger_it_says_how_long_to_full_and_how_worn_it_is() {
        let b = shape(&Raw { flag: 8, ac: 1, percent: 45, rate_mw: Some(18_000), full_mwh: Some(48_000), design_mwh: Some(52_700), remaining_mwh: Some(21_600), ..Raw::default() });
        assert_eq!((b["charging"].as_bool(), b["plugged"].as_bool(), b["minutes_to_full"].as_i64(), b["minutes_left"].is_null()), (Some(true), Some(true), Some(88), true));
        assert_eq!((b["health"].as_f64(), b["full_wh"].as_f64(), b["design_wh"].as_f64()), (Some(91.0), Some(48.0), Some(52.7)));
        // (plugged in and full: not charging)
        let b = shape(&Raw { flag: 0, ac: 1, percent: 100, rate_mw: Some(0), ..Raw::default() });
        assert_eq!((b["charging"].as_bool(), b["plugged"].as_bool()), (Some(false), Some(true)));
    }

    #[test]
    fn battery_saver_is_seen() {
        assert_eq!(shape(&Raw { saver: 1, ..raw() })["saver"], true);
    }

    #[test]
    fn only_what_matters_makes_a_reading_news() {
        let a = shape(&raw());
        // (the same, but the time left moved)
        assert!(!changed(&a, &shape(&Raw { life_s: 11_000, ..raw() })));
        assert!(changed(&a, &shape(&Raw { percent: 63, ..raw() })));
        assert!(changed(&a, &shape(&Raw { ac: 1, ..raw() })));
        assert!(changed(&a, &shape(&Raw { saver: 1, ..raw() })));
    }
}
