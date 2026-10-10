//! Claude usage, a part of the Claude Code plugin: how much of the 5-hour and 7-day quota windows is used (two rings in the
//! hub and a peek when the number moves).
//!
//! The plugin signs in to Claude on its own, the way the Claude Code CLI does (OAuth with PKCE: the browser opens, the page
//! shows a code, the code is pasted back). Its tokens are its own: kept encrypted for this Windows user (DPAPI) in
//! `%APPDATA%\NADI\claude-login.bin`, and renewed by the plugin itself, so the rings work whether or not Claude Code is
//! running. Claude Code's own login (`~/.claude/.credentials.json`) is never read, written or renewed: the CLI renews it only
//! while it runs, and a refresh token is good for one use, so sharing it would break the CLI's login.

use crate::native::Ctx;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const TOKEN_URL: &str = "https://console.anthropic.com/v1/oauth/token";
const AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const REDIRECT_URL: &str = "https://console.anthropic.com/oauth/code/callback";
const SCOPE: &str = "org:create_api_key user:profile user:inference";
// The public OAuth client of the Claude Code CLI.
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const BETA: &str = "oauth-2025-04-20";
// Cloudflare in front of both hosts blocks default HTTP-library UAs; a small
// honestly-named tool UA (same one the Python version settled on) gets through.
const USER_AGENT: &str = "claude-code/2.1.0";
// The token endpoint rate-limits that name for everyone (a plain 429 on the first try), so the sign-in and its renewal
// say who they are.
const OAUTH_AGENT: &str = "nadi/1.1";
const POLL_S: u64 = 120; // 5s got the endpoint rate-limited -- back off to 2min
const MANUAL_COOLDOWN_S: u64 = 60;
/// how long the peek stays: the bars sit at the old value, fill to the new one, and rest
const PEEK_TOTAL_MS: u64 = 500 + 1400 + 2500;
/// a token is renewed this long before it runs out
const RENEW_BEFORE_MS: u64 = 60_000;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
struct Tokens {
    access: String,
    refresh: String,
    expires_ms: u64,
}

#[derive(Serialize, Clone, Default)]
pub struct UsageWindow {
    pub pct: Option<f64>,
    pub resets_at: Option<String>,
}

#[derive(Serialize, Clone, Default)]
pub struct UsageSnapshot {
    /// false when there is no Claude login at all on this machine -- the
    /// frontend hides the rings entirely rather than showing an error
    pub available: bool,
    pub error: Option<String>,
    /// the last numbers are shown, and an error says why they did not move
    pub stale: bool,
    /// the plugin has its own sign-in (and does not lean on the Claude Code CLI's)
    pub signed_in: bool,
    /// only a new sign-in will do
    pub needs_login: bool,
    pub five_hour: Option<UsageWindow>,
    pub seven_day: Option<UsageWindow>,
}

/// why the numbers could not be had
struct Fail {
    msg: String,
    needs_login: bool,
}

impl Fail {
    fn soft(msg: impl Into<String>) -> Fail {
        Fail { msg: msg.into(), needs_login: false }
    }
    fn login(msg: impl Into<String>) -> Fail {
        Fail { msg: msg.into(), needs_login: true }
    }
}

#[derive(Default)]
pub struct UsageState {
    /// the plugin's own tokens, as last read from disk or renewed
    tokens: Mutex<Option<Tokens>>,
    last_manual: Mutex<Option<Instant>>,
    last: Mutex<UsageSnapshot>,
    /// last seen (5h, 7d) percentages, to detect a change worth peeking about
    last_pct: Mutex<(Option<f64>, Option<f64>)>,
    /// a sign-in that was begun: (code verifier, state)
    pending: Mutex<Option<(String, String)>>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---- the plugin's own login, on disk (encrypted for this Windows user)

fn login_path() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("NADI").join("claude-login.bin"))
}

fn protect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptProtectData(&input, PCWSTR::null(), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as *mut _));
        Some(bytes)
    }
}

fn unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out).ok()?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(HLOCAL(out.pbData as *mut _));
        Some(bytes)
    }
}

fn load_own() -> Option<Tokens> {
    let bytes = std::fs::read(login_path()?).ok()?;
    serde_json::from_slice(&unprotect(&bytes)?).ok()
}

fn save_own(t: &Tokens) -> Result<(), String> {
    let path = login_path().ok_or("no APPDATA")?;
    let sealed = protect(&serde_json::to_vec(t).map_err(|e| e.to_string())?).ok_or("could not encrypt the sign-in")?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // (written whole, then moved into place: a crash never leaves half a login)
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, sealed).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())
}

fn forget_own() {
    if let Some(p) = login_path() {
        let _ = std::fs::remove_file(p);
    }
}

/// The plugin's own tokens: the ones in memory, else the ones on disk.
fn own_tokens(state: &UsageState) -> Option<Tokens> {
    let mut slot = state.tokens.lock().unwrap();
    if slot.is_none() {
        *slot = load_own();
    }
    slot.clone()
}

// ---- signing in (OAuth with PKCE, the code pasted back)

fn random_b64(bytes: usize) -> Result<String, String> {
    let mut buf = vec![0u8; bytes];
    getrandom::getrandom(&mut buf).map_err(|e| format!("no randomness: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(buf))
}

fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn authorize_url(challenge: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE_URL}?code=true&client_id={CLIENT_ID}&response_type=code&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}",
        encode(REDIRECT_URL),
        encode(SCOPE),
    )
}

/// What the page shows is `code#state`: only the code goes to Anthropic (with the state, which is its own).
fn code_of(pasted: &str) -> &str {
    let t = pasted.trim();
    t.split('#').next().unwrap_or(t).trim()
}

fn open_in_browser(url: &str) {
    use windows::core::{w, HSTRING};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(url), None, None, SW_SHOWNORMAL);
    }
}

/// what the token endpoint says, as tokens (a renewal that does not name a new refresh token keeps the old one)
fn tokens_of(data: &Value, keep_refresh: &str) -> Result<Tokens, String> {
    let access = data["access_token"].as_str().ok_or("the sign-in returned no access token")?.to_string();
    let expires_in = data["expires_in"].as_u64().unwrap_or(0);
    Ok(Tokens {
        access,
        refresh: data["refresh_token"].as_str().unwrap_or(keep_refresh).to_string(),
        expires_ms: if expires_in > 0 { now_ms() + expires_in * 1000 } else { 0 },
    })
}

fn post_token(body: Value) -> Result<Value, (u16, String)> {
    match ureq::post(TOKEN_URL)
        .set("User-Agent", OAUTH_AGENT)
        .set("Accept", "application/json")
        .timeout(Duration::from_secs(20))
        .send_json(body)
    {
        Ok(resp) => resp.into_json().map_err(|e| (0, e.to_string())),
        Err(ureq::Error::Status(code, _)) => Err((code, format!("HTTP {code}"))),
        Err(e) => Err((0, format!("Network error: {e}"))),
    }
}

fn exchange_code(code: &str, verifier: &str, state: &str) -> Result<Tokens, String> {
    let data = post_token(json!({
        "grant_type": "authorization_code",
        "client_id": CLIENT_ID,
        "code": code,
        "redirect_uri": REDIRECT_URL,
        "code_verifier": verifier,
        "state": state,
    }))
    .map_err(|(status, e)| match status {
        400 | 401 => "Claude did not accept that code. Start the sign-in again and paste the new one.".to_string(),
        429 => "Claude is rate limiting the sign-in. Try again in a minute.".to_string(),
        _ => e,
    })?;
    tokens_of(&data, "")
}

/// Renew the plugin's own tokens (and keep what comes back: the refresh token is good for one use).
fn renew_own(state: &UsageState, old: &Tokens) -> Result<Tokens, Fail> {
    if old.refresh.is_empty() {
        return Err(Fail::login("The sign-in has no way to renew itself. Sign in again."));
    }
    let data = post_token(json!({ "grant_type": "refresh_token", "refresh_token": old.refresh, "client_id": CLIENT_ID })).map_err(|(status, e)| match status {
        400 | 401 => Fail::login("The sign-in ran out. Sign in again."),
        429 => Fail::soft("Claude is rate limiting the sign-in renewal."),
        _ => Fail::soft(e),
    })?;
    let fresh = tokens_of(&data, &old.refresh).map_err(Fail::soft)?;
    *state.tokens.lock().unwrap() = Some(fresh.clone());
    save_own(&fresh).map_err(|e| Fail::soft(format!("Could not keep the renewed sign-in: {e}")))?;
    Ok(fresh)
}

fn call_usage(access: &str) -> Result<(u16, Value), Fail> {
    let result = ureq::get(USAGE_URL)
        .set("Authorization", &format!("Bearer {access}"))
        .set("anthropic-beta", BETA)
        .set("Accept", "application/json")
        .set("User-Agent", USER_AGENT)
        .timeout(Duration::from_secs(15))
        .call();
    match result {
        Ok(resp) => {
            let status = resp.status();
            Ok((status, resp.into_json().unwrap_or(Value::Null)))
        }
        Err(ureq::Error::Status(code, resp)) => Ok((code, resp.into_json().unwrap_or(Value::Null))),
        Err(e) => Err(Fail::soft(format!("Network error: {e}"))),
    }
}

fn parse_window(usage: &Value, key: &str) -> Option<UsageWindow> {
    let w = usage.get(key)?;
    if !w.is_object() {
        return None;
    }
    Some(UsageWindow {
        pct: w["utilization"].as_f64(),
        resets_at: w["resets_at"].as_str().map(str::to_string),
    })
}

fn snapshot_of(status: u16, data: Value) -> Result<UsageSnapshot, Fail> {
    match status {
        200 => {}
        401 => return Err(Fail::login("The sign-in was refused. Sign in again.")),
        429 => return Err(Fail::soft("Rate limited by the usage endpoint -- try again in a bit.")),
        other => return Err(Fail::soft(format!("HTTP {other}"))),
    }
    let usage = if data.get("five_hour").is_none() && data.get("seven_day").is_none() {
        data.get("usage").or_else(|| data.get("data")).unwrap_or(&data).clone()
    } else {
        data
    };
    Ok(UsageSnapshot {
        available: true,
        signed_in: true,
        five_hour: parse_window(&usage, "five_hour"),
        seven_day: parse_window(&usage, "seven_day"),
        ..Default::default()
    })
}

fn fetch(state: &UsageState) -> Result<UsageSnapshot, Fail> {
    if let Some(mut tokens) = own_tokens(state) {
        if tokens.expires_ms != 0 && tokens.expires_ms < now_ms() + RENEW_BEFORE_MS {
            tokens = renew_own(state, &tokens)?;
        }
        let (mut status, mut data) = call_usage(&tokens.access)?;
        if status == 401 {
            // (revoked, or run out before its time)
            tokens = renew_own(state, &tokens)?;
            (status, data) = call_usage(&tokens.access)?;
        }
        return snapshot_of(status, data);
    }
    // (no sign-in: nothing to ask with)
    Err(Fail::login("Sign in to see how much of your limits is used."))
}

/// Brief island peek when either usage percentage moved (rounded, so
/// sub-percent jitter is ignored). Skipped while the hub/a notification owns
/// the island, but the new baseline is still recorded so no delta is lost.
fn maybe_peek(ctx: &Ctx, state: &UsageState, snapshot: &UsageSnapshot) {
    if snapshot.error.is_some() || !snapshot.available {
        return;
    }
    let new = (
        snapshot.five_hour.as_ref().and_then(|w| w.pct),
        snapshot.seven_day.as_ref().and_then(|w| w.pct),
    );
    let old = std::mem::replace(&mut *state.last_pct.lock().unwrap(), new);
    let moved = |o: Option<f64>, n: Option<f64>| {
        matches!((o, n), (Some(o), Some(n)) if o.round() != n.round())
    };
    if !(moved(old.0, new.0) || moved(old.1, new.1)) {
        return;
    }
    if ctx.state.hub_is_open() || ctx.state.notif.lock().unwrap().has_current() {
        return;
    }
    ctx.emit(
        "usage-peek",
        json!({ "before": { "five_hour": old.0, "seven_day": old.1 },
                "after": { "five_hour": new.0, "seven_day": new.1 } }),
    );
    ctx.offer_pill(Some("usage_peek"));
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(PEEK_TOTAL_MS));
        ctx.offer_pill(None);
    });
}

fn refresh_now(ctx: &Ctx, state: &UsageState) {
    let prev = state.last.lock().unwrap().clone();
    let snapshot = if own_tokens(state).is_none() {
        // (not signed in: no rings, and the plugin's line says how to get them)
        UsageSnapshot { needs_login: true, ..Default::default() }
    } else {
        match fetch(state) {
            Ok(s) => s,
            // the last numbers stay, with the reason they did not move
            Err(f) => UsageSnapshot {
                available: true,
                error: Some(f.msg),
                stale: true,
                needs_login: f.needs_login,
                signed_in: own_tokens(state).is_some(),
                five_hour: prev.five_hour,
                seven_day: prev.seven_day,
            },
        }
    };
    *state.last.lock().unwrap() = snapshot.clone();
    maybe_peek(ctx, state, &snapshot);
    ctx.emit("usage-tick", snapshot);
}

/// The usage part of the plugin: its state, and what the plugin does with it.
#[derive(Default)]
pub struct Usage {
    state: Arc<UsageState>,
}

impl Usage {
    /// whether the rings are wanted: the plugin is on, and so is its usage setting
    fn wanted(ctx: &Ctx) -> bool {
        ctx.on() && ctx.flag("usage", true)
    }

    pub fn start(&self, ctx: &Ctx) {
        let (ctx, state) = (ctx.clone(), self.state.clone());
        std::thread::spawn(move || {
            let mut last: Option<Instant> = None;
            loop {
                std::thread::sleep(Duration::from_secs(2));
                if !Self::wanted(&ctx) {
                    last = None;
                    continue;
                }
                // (poll again sooner while there is nothing to show: a sign-in brings it)
                let every = if state.last.lock().unwrap().available { POLL_S } else { 20 };
                if last.map_or(true, |t| t.elapsed() >= Duration::from_secs(every)) {
                    last = Some(Instant::now());
                    refresh_now(&ctx, &state);
                }
            }
        });
    }

    /// The plugin was switched, or its usage setting changed: with the rings gone, what was read is forgotten
    /// (the sign-in itself stays on disk, encrypted, until the user signs out).
    pub fn settled(&self, ctx: &Ctx) {
        if !Self::wanted(ctx) {
            *self.state.last.lock().unwrap() = UsageSnapshot::default();
            *self.state.tokens.lock().unwrap() = None;
            ctx.emit("usage-tick", UsageSnapshot::default());
        }
    }

    /// The plugin has a sign-in of its own (its field shows it as filled).
    pub fn signed_in(&self) -> bool {
        own_tokens(&self.state).is_some()
    }

    /// The line under the plugin's name, while the rings have something to say.
    pub fn status(&self, ctx: &Ctx) -> Option<String> {
        if !Self::wanted(ctx) {
            return None;
        }
        let last = self.state.last.lock().unwrap().clone();
        if let Some(e) = &last.error {
            return Some(format!("Usage rings: {e}"));
        }
        if last.needs_login {
            return Some("Usage rings: sign in below to see how much of your limits is used.".into());
        }
        None
    }

    pub fn call(&self, ctx: &Ctx, cmd: &str, args: &Value) -> Result<Value, String> {
        match cmd {
            "usage_state" => Ok(serde_json::to_value(self.state.last.lock().unwrap().clone()).unwrap_or(Value::Null)),
            // Manual refresh from a ring click, with a cooldown so it can't be spammed
            // into the endpoint's rate limit. Returns whether a refresh actually ran.
            "usage_refresh" => {
                {
                    let mut last = self.state.last_manual.lock().unwrap();
                    if let Some(t) = *last {
                        if t.elapsed() < Duration::from_secs(MANUAL_COOLDOWN_S) {
                            return Ok(json!(false));
                        }
                    }
                    *last = Some(Instant::now());
                }
                let (ctx, state) = (ctx.clone(), self.state.clone());
                std::thread::spawn(move || refresh_now(&ctx, &state));
                Ok(json!(true))
            }
            // Sign in: the browser opens on Claude's page, which shows a code to paste back (`usage_finish`).
            "usage_login" => {
                let verifier = random_b64(32)?;
                let state = random_b64(32)?;
                let url = authorize_url(&challenge_of(&verifier), &state);
                *self.state.pending.lock().unwrap() = Some((verifier, state));
                open_in_browser(&url);
                Ok(json!(true))
            }
            "usage_finish" => {
                let pasted = args.get("text").and_then(Value::as_str).unwrap_or("");
                let code = code_of(pasted);
                if code.is_empty() {
                    // (nothing in the field: the sign-in is forgotten)
                    forget_own();
                    *self.state.tokens.lock().unwrap() = None;
                    *self.state.pending.lock().unwrap() = None;
                    *self.state.last.lock().unwrap() = UsageSnapshot::default();
                    let (ctx, state) = (ctx.clone(), self.state.clone());
                    std::thread::spawn(move || refresh_now(&ctx, &state));
                    return Ok(json!(true));
                }
                let (verifier, state) = self.state.pending.lock().unwrap().clone().ok_or("Start the sign-in first.")?;
                let tokens = exchange_code(code, &verifier, &state)?;
                save_own(&tokens)?;
                *self.state.tokens.lock().unwrap() = Some(tokens);
                *self.state.pending.lock().unwrap() = None;
                let (ctx, state) = (ctx.clone(), self.state.clone());
                std::thread::spawn(move || refresh_now(&ctx, &state));
                Ok(json!(true))
            }
            _ => Err(format!("no such call: {cmd}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_sha256_of_the_verifier() {
        // the example of RFC 7636, appendix B
        assert_eq!(challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn random_values_are_url_safe_and_differ() {
        let (a, b) = (random_b64(32).unwrap(), random_b64(32).unwrap());
        assert_ne!(a, b);
        assert_eq!(a.len(), 43);
        assert!(a.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn the_sign_in_page_is_asked_for_a_code() {
        let url = authorize_url("CHAL", "STATE");
        assert!(url.starts_with("https://claude.ai/oauth/authorize?code=true&client_id=9d1c250a"));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("redirect_uri=https%3A%2F%2Fconsole.anthropic.com%2Foauth%2Fcode%2Fcallback"));
        assert!(url.contains("scope=org%3Acreate_api_key%20user%3Aprofile%20user%3Ainference"));
        assert!(url.ends_with("code_challenge=CHAL&code_challenge_method=S256&state=STATE"));
    }

    #[test]
    fn the_pasted_code_loses_its_state_and_its_blanks() {
        assert_eq!(code_of("abc123#xyz"), "abc123");
        assert_eq!(code_of("  abc123#xyz \r\n"), "abc123");
        assert_eq!(code_of("abc123"), "abc123");
        assert_eq!(code_of("   "), "");
    }

    #[test]
    fn a_renewal_keeps_the_old_refresh_token_when_none_comes_back() {
        let t = tokens_of(&json!({ "access_token": "a", "expires_in": 3600 }), "old").unwrap();
        assert_eq!((t.access.as_str(), t.refresh.as_str()), ("a", "old"));
        assert!(t.expires_ms > now_ms());
        let t = tokens_of(&json!({ "access_token": "a", "refresh_token": "new" }), "old").unwrap();
        assert_eq!((t.refresh.as_str(), t.expires_ms), ("new", 0));
        assert!(tokens_of(&json!({}), "old").is_err());
    }

    #[test]
    fn what_is_sealed_for_this_user_opens_again() {
        let t = Tokens { access: "acc".into(), refresh: "ref".into(), expires_ms: 42 };
        let plain = serde_json::to_vec(&t).unwrap();
        let sealed = protect(&plain).expect("DPAPI seals");
        assert_ne!(sealed, plain);
        assert!(!String::from_utf8_lossy(&sealed).contains("acc"));
        let back: Tokens = serde_json::from_slice(&unprotect(&sealed).expect("DPAPI opens")).unwrap();
        assert_eq!(back, t);
        assert!(unprotect(b"not sealed").is_none());
    }

    #[test]
    fn a_usage_answer_is_read_from_either_shape() {
        let flat = json!({ "five_hour": { "utilization": 14.0, "resets_at": "x" }, "seven_day": { "utilization": 95.5 } });
        let s = snapshot_of(200, flat).ok().unwrap();
        assert_eq!(s.five_hour.unwrap().pct, Some(14.0));
        assert_eq!(s.seven_day.unwrap().pct, Some(95.5));
        assert!(s.signed_in && s.available && s.error.is_none());
        let nested = json!({ "usage": { "five_hour": { "utilization": 1.0 } } });
        assert_eq!(snapshot_of(200, nested).ok().unwrap().five_hour.unwrap().pct, Some(1.0));
    }

    #[test]
    fn a_refused_token_asks_for_a_sign_in_and_a_busy_endpoint_does_not() {
        assert!(snapshot_of(401, Value::Null).err().unwrap().needs_login);
        assert!(!snapshot_of(429, Value::Null).err().unwrap().needs_login);
        assert!(!snapshot_of(500, Value::Null).err().unwrap().needs_login);
    }
}
