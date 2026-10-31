//! Windows SMTC (System Media Transport Controls) media session bridge --
//! the Rust equivalent of `platform_backend/windows.py`'s `MediaBridge`.
//!
//! Unlike the Python version (which wires the SDK's own change-notification
//! events), this polls once a second on a dedicated OS thread. SMTC's WinRT
//! objects need a COM apartment initialized on whatever thread touches them,
//! and wiring `TypedEventHandler` callbacks across that boundary safely is a
//! lot of unsafe-adjacent plumbing for a value that's fine to be up to ~1s
//! stale. A plain thread + blocking calls is far simpler and just as
//! correct for this app's purposes (nothing here is a tight loop).

use crate::pc::Pc;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as SessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as PlaybackStatus,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

const POLL_MS: u64 = 1000;

#[derive(Serialize, Clone, Default)]
pub struct MediaSnapshot {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub playing: bool,
    pub position: f64,
    pub duration: f64,
    pub can_previous: bool,
    pub can_next: bool,
    /// the player takes seeks (jump 5 s, the seek bar)
    pub can_seek: bool,
    /// the player takes a playback speed
    pub can_rate: bool,
    /// playback speed, 1.0 = normal
    pub rate: f64,
    /// data: URL (PNG), ready to drop straight into an <img src>.
    pub art: Option<String>,
    /// the playing app's AppUserModelId (used to focus its window from the hub card)
    pub source: String,
}

fn all_sessions() -> Vec<Session> {
    let manager = match SessionManager::RequestAsync().and_then(|op| op.get()) {
        Ok(m) => m,
        Err(_) => return Vec::new(),
    };
    // All sessions with usable metadata, not just the focused one --
    // two players at once (e.g. Spotify + a browser) means two cards.
    let mut out = Vec::new();
    if let Ok(list) = manager.GetSessions() {
        for i in 0..list.Size().unwrap_or(0) {
            if let Ok(session) = list.GetAt(i) {
                if read_snapshot(&session).ok().flatten().is_some() {
                    out.push(session);
                }
            }
        }
    }
    // Fall back to the current session when enumeration itself found nothing.
    if out.is_empty() {
        if let Ok(session) = manager.GetCurrentSession() {
            out.push(session);
        }
    }
    out
}

/// Some plugin that is on may see the cover pictures (set by the poll thread).
static WANT_ART: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn read_snapshot(session: &Session) -> windows::core::Result<Option<MediaSnapshot>> {
    let playback = session.GetPlaybackInfo()?;
    let status = playback.PlaybackStatus()?;
    if status == PlaybackStatus::Closed {
        return Ok(None);
    }

    let props = session.TryGetMediaPropertiesAsync()?.get()?;
    let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
    if title.is_empty() && artist.is_empty() {
        return Ok(None);
    }

    let (mut position, mut duration) = (0.0, 0.0);
    if let Ok(timeline) = session.GetTimelineProperties() {
        if let (Ok(pos), Ok(start), Ok(end)) = (
            timeline.Position(),
            timeline.StartTime(),
            timeline.EndTime(),
        ) {
            position = pos.Duration as f64 / 10_000_000.0;
            duration = (end.Duration - start.Duration) as f64 / 10_000_000.0;
        }
    }

    let rate = playback.PlaybackRate().ok().and_then(|r| r.Value().ok()).filter(|r| *r > 0.0).unwrap_or(1.0);
    let controls = playback.Controls().ok();
    let can_seek = controls.as_ref().and_then(|c| c.IsPlaybackPositionEnabled().ok()).unwrap_or(false);
    let can_rate = controls.as_ref().and_then(|c| c.IsPlaybackRateEnabled().ok()).unwrap_or(false);
    let can_previous = controls
        .as_ref()
        .and_then(|c| c.IsPreviousEnabled().ok())
        .unwrap_or(true);
    let can_next = controls
        .as_ref()
        .and_then(|c| c.IsNextEnabled().ok())
        .unwrap_or(true);

    // (a picture read and encoded each second is a lot of work and of garbage: only while a plugin wants it)
    let art = if WANT_ART.load(Ordering::Relaxed) { read_thumbnail(&props).ok().flatten() } else { None };

    Ok(Some(MediaSnapshot {
        has_session: true,
        title,
        artist,
        playing: status == PlaybackStatus::Playing,
        position,
        duration,
        can_previous,
        can_next,
        can_seek,
        can_rate,
        rate,
        art,
        source: session
            .SourceAppUserModelId()
            .map(|h| h.to_string())
            .unwrap_or_default(),
    }))
}

fn read_thumbnail(
    props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> windows::core::Result<Option<String>> {
    use base64::Engine;
    use windows::Storage::Streams::{Buffer, DataReader, InputStreamOptions};

    let Some(thumb) = props.Thumbnail().ok() else {
        return Ok(None);
    };
    let stream = thumb.OpenReadAsync()?.get()?;
    let size = stream.Size()? as u32;
    if size == 0 {
        return Ok(None);
    }
    // Sources vary (album art is usually a JPEG or PNG, browser tab
    // thumbnails can be either too) -- trust the stream's own reported
    // type instead of assuming PNG, or a non-PNG thumbnail just silently
    // fails to decode as a broken-image icon in the <img>.
    let content_type = stream.ContentType().ok().map(|h| h.to_string());
    let mime = content_type
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| "image/png".to_string());
    let buffer = Buffer::Create(size)?;
    stream
        .ReadAsync(&buffer, size, InputStreamOptions::None)?
        .get()?;
    let reader = DataReader::FromBuffer(&buffer)?;
    let mut bytes = vec![0u8; size as usize];
    reader.ReadBytes(&mut bytes)?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(Some(format!("data:{mime};base64,{b64}")))
}

fn snapshots_now() -> Vec<MediaSnapshot> {
    all_sessions()
        .iter()
        .filter_map(|s| read_snapshot(s).ok().flatten())
        .collect()
}

/// What a module that may see what is playing is told (never the picture, and the position only to the second).
fn publish_for_plugins(state: &Pc, snaps: &[MediaSnapshot]) {
    if !state.want.media.load(Ordering::Relaxed) {
        return;
    }
    // (a picture is told once for each change: its id says which it is, and `media_arts` holds the pictures)
    let mut arts = state.media_arts.lock().unwrap();
    arts.retain(|source, _| snaps.iter().any(|s| &s.source == source));
    let sessions: Vec<serde_json::Value> = snaps
        .iter()
        .map(|s| {
            let art_id = match &s.art {
                Some(a) => {
                    let id = a.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
                    if arts.get(&s.source).map(|(i, _)| *i) != Some(id) {
                        arts.insert(s.source.clone(), (id, a.clone()));
                    }
                    id
                }
                None => {
                    arts.remove(&s.source);
                    0
                }
            };
            serde_json::json!({
                "source": s.source, "title": s.title, "artist": s.artist, "playing": s.playing,
                "position": s.position.round(), "duration": s.duration.round(),
                "can_previous": s.can_previous, "can_next": s.can_next,
                "can_seek": s.can_seek, "can_rate": s.can_rate, "rate": s.rate, "art_id": art_id,
            })
        })
        .collect();
    drop(arts);
    let value = serde_json::Value::Array(sessions);
    let mut last = state.media_seen.lock().unwrap();
    // (a module is told at every change; while something plays, the position moves each second)
    if *last != value {
        *last = value;
        state.media_seq.fetch_add(1, Ordering::Relaxed);
    }
}

/// The poll thread: once a second while what plays is wanted, else it idles.
pub fn spawn(state: Arc<Pc>) {
    std::thread::Builder::new()
        .name("nadi-media".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            loop {
                if !state.want.media.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_millis(POLL_MS));
                    continue;
                }
                WANT_ART.store(state.want.media_art.load(Ordering::Relaxed), Ordering::Relaxed);
                let snaps = snapshots_now();
                publish_for_plugins(&state, &snaps);
                std::thread::sleep(Duration::from_millis(POLL_MS));
            }
        })
        .expect("the media thread");
}

/// The session a control targets: the one from that card (`source` is its
/// AppUserModelId), else the focused one. `None` keeps the old pill behavior.
///
/// Matching uses only synchronous properties: `read_snapshot` (metadata +
/// thumbnail stream reads) must stay on the COM-initialized poll thread --
/// blocking `.get()`s for those on a command thread can hang it, and with
/// it the island.
fn find_session(source: Option<String>) -> Option<Session> {
    let manager = SessionManager::RequestAsync().and_then(|op| op.get()).ok()?;
    let want = source.filter(|s| !s.is_empty());
    if let Some(w) = want.as_deref() {
        if let Ok(list) = manager.GetSessions() {
            for i in 0..list.Size().unwrap_or(0) {
                if let Ok(s) = list.GetAt(i) {
                    if s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default() == w {
                        return Some(s);
                    }
                }
            }
        }
    }
    manager.GetCurrentSession().ok()
}

fn with_session<F: FnOnce(&Session) -> windows::core::Result<()>>(source: Option<String>, f: F) {
    if let Some(session) = find_session(source) {
        let _ = f(&session);
    }
}

/// Block at most `timeout_ms` for a WinRT control op. Some players take the
/// call but never finish it (a browser tab mid-navigation, a paused remote
/// session), and a bare `.get()` then hangs the command thread -- and the
/// island with it -- forever. True when the op actually completed.
fn wait_completed(
    mut get_status: impl FnMut() -> windows::core::Result<windows::Foundation::AsyncStatus>,
    timeout_ms: u64,
) -> bool {
    use windows::Foundation::AsyncStatus;
    let limit = Duration::from_millis(timeout_ms);
    let start = std::time::Instant::now();
    loop {
        match get_status() {
            Ok(AsyncStatus::Completed) => return true,
            Ok(AsyncStatus::Started) => {}
            _ => return false,
        }
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// A control op gets 2 s to finish; a stuck player is dropped, never hung on.
const OP_TIMEOUT_MS: u64 = 2000;

pub fn media_play_pause(source: Option<String>) {
    with_session(source, |s| {
        let op = s.TryTogglePlayPauseAsync()?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            op.get().map(|_| ())
        } else {
            Ok(())
        }
    });
}

pub fn media_next(source: Option<String>) {
    with_session(source, |s| {
        let op = s.TrySkipNextAsync()?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            op.get().map(|_| ())
        } else {
            Ok(())
        }
    });
}

pub fn media_previous(source: Option<String>) {
    with_session(source, |s| {
        let op = s.TrySkipPreviousAsync()?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            op.get().map(|_| ())
        } else {
            Ok(())
        }
    });
}

pub fn media_seek(position_seconds: f64, source: Option<String>) {
    with_session(source, |s| {
        let ticks = (position_seconds * 10_000_000.0) as i64;
        let op = s.TryChangePlaybackPositionAsync(ticks)?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            op.get().map(|_| ())
        } else {
            Ok(())
        }
    });
}

/// jump forward (positive) or back (negative) by some seconds, from where the player is now:
/// the timeline's position is the one it last reported, so the time since then is added (at
/// the playback speed) while it plays
pub fn media_seek_by(delta_seconds: f64, source: Option<String>) {
    with_session(source, |s| {
        let tl = s.GetTimelineProperties()?;
        let playback = s.GetPlaybackInfo()?;
        let playing = playback.PlaybackStatus()? == PlaybackStatus::Playing;
        let rate = playback.PlaybackRate().ok().and_then(|r| r.Value().ok()).filter(|r| *r > 0.0).unwrap_or(1.0);
        let start = tl.StartTime()?.Duration as f64 / 10_000_000.0;
        let end = tl.EndTime()?.Duration as f64 / 10_000_000.0;
        let mut pos = tl.Position()?.Duration as f64 / 10_000_000.0;
        if playing {
            // `LastUpdatedTime` is a Windows FILETIME-style count of 100 ns since 1601
            let updated = tl.LastUpdatedTime()?.UniversalTime as f64 / 10_000_000.0;
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) + 11_644_473_600.0;
            let since = (now - updated).clamp(0.0, 30.0);
            pos += since * rate;
        }
        let target = (pos + delta_seconds).clamp(start, if end > start { end } else { f64::MAX });
        let op = s.TryChangePlaybackPositionAsync((target * 10_000_000.0) as i64)?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            op.get().map(|_| ())
        } else {
            Ok(())
        }
    });
}

/// set the playback speed (0.1 to 16); true when the player took it
pub fn media_set_rate(rate: f64, source: Option<String>) -> bool {
    let rate = rate.clamp(0.1, 16.0);
    let mut ok = false;
    with_session(source, |s| {
        let op = s.TryChangePlaybackRateAsync(rate)?;
        if wait_completed(|| op.Status(), OP_TIMEOUT_MS) {
            ok = op.get()?;
        }
        Ok(())
    });
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Foundation::AsyncStatus;

    #[test]
    fn wait_completed_true_once_the_op_finishes() {
        let mut polls = 0;
        let done = wait_completed(
            || {
                polls += 1;
                Ok(if polls >= 3 { AsyncStatus::Completed } else { AsyncStatus::Started })
            },
            1000,
        );
        assert!(done);
    }

    #[test]
    fn wait_completed_false_when_the_op_never_finishes() {
        let start = std::time::Instant::now();
        let done = wait_completed(|| Ok(AsyncStatus::Started), 50);
        assert!(!done);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn wait_completed_false_on_error_status() {
        let done = wait_completed(|| Ok(AsyncStatus::Error), 1000);
        assert!(!done);
    }
}

