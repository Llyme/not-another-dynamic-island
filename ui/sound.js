// The sound the island hears, understood: Rust analyzes the system audio (WASAPI loopback) and sends loudness, bands,
// beat pulses and a rough speech-vs-music call; this turns that into the numbers the visuals use (`audio`) and calls the
// vibe of the music (`director`). It is the app's, not a plugin's: the plugins that show something about the sound (the
// eyes' mood, the sound light) borrow it. Nothing is heard unless one of them is on (Rust only listens then).

import { listen, mediaListOf, pickPrimaryMedia } from "./kit.js";

// what is told about each bass hit: (strength 0..1, pan -1..1)
const beatHandlers = [];
export const onBeat = (fn) => beatHandlers.push(fn);
function beat(strength, pan) {
	for (const fn of beatHandlers) fn(strength, pan);
}

// -- listening: Rust analyzes the system audio (WASAPI loopback) and sends
// loudness, beat pulses and a rough speech-vs-music call. Music -> the eyes
// bob and sway on the beat (arches flash on strong hits); speech -> they
// "talk", opening and closing with the voice. --
const VIZ_BANDS = 16;
const audio = {
	target: 0,
	level: 0,
	mode: "silent", // "silent" | "music" | "speech" (hard call, for logic that needs one)
	lastTick: 0,
	kind: "silent",
	voice: 0, // raw 0..1 how voice-like (jumps around on vocal music)
	voiceSm: 0, // what the visuals use: eased toward the director's call (see below)
	bang: 0, // 0..1 fast, hard, punchy music (picks the > < expression)
	bands: new Array(VIZ_BANDS).fill(0), // 16 log-spaced bands, ~60 Hz .. ~12 kHz, each 0..1
	bandTarget: new Array(VIZ_BANDS).fill(0),
	bpm: 100,
	conf: 0,
	lastBeatAt: 0,
	energy: 0, // 0..1 how energetic the sound is (loudness, hit sharpness, tempo)
	chill: 0, // 0..1 how relaxed: soft, slow, gentle music
	pan: 0, // -1 left .. 1 right, smoothed: where the sound is coming from
	panTarget: 0,
	dance: 0, // level * (1 - voice): how much "music energy" to show
	talk: 0, // level * voice
	talkEnv: 0, // fast envelope for the eyes' open/close
	kick: { p: 0, v: 0 },
	nod: { p: 0, v: 0 }, // head-bang nod: snaps down on each kick-drum hit
	lastKickAt: 0,
	sway: 0, // phase, advances with the estimated tempo
};
// -- the "director": once a second it takes a sample of the audio and calls a
// vibe: silent / talk / chill / groove / bang. The call is not made from that
// one second alone but from a rolling ~20 s PROFILE of the current track (tempo
// feel, how often the kick lands, energy, vocal share...), which is far steadier
// than any single second. A new call must also hold for two samples in a row.
// The profile resets when the song changes, and finished profiles are
// remembered per song, so a replay is understood from its first second. --
const PROFILE_SECS = 20;
const PROFILE_KEY = "di.trackProfiles";
const newAcc = () => ({
	n: 0,
	voice: 0,
	energy: 0,
	level: 0,
	bass: 0,
	beats: 0,
	kicks: 0,
});
const director = {
	state: "silent",
	cand: "silent",
	count: 0,
	since: 0,
	lastSample: 0,
	video: false, // what plays looks like a video/film (set from the media session)
	acc: newAcc(),
	history: [], // one-second summaries of the current track, oldest first
	track: "", // "title|artist" of what is playing
};

let savedProfiles = {};
try {
	savedProfiles = JSON.parse(
		localStorage.getItem(PROFILE_KEY) || "{}",
	);
} catch (_) {}
let profilesSavedAt = 0;

function avgHistory(list) {
	const o = {
		voice: 0,
		energy: 0,
		level: 0,
		bass: 0,
		beats: 0,
		kicks: 0,
	};
	for (const h of list) for (const k in o) o[k] += h[k];
	const n = Math.max(1, list.length);
	for (const k in o) o[k] /= n;
	return o;
}

function rememberProfile(P) {
	if (!director.track) return;
	delete savedProfiles[director.track]; // re-insert = most recent
	savedProfiles[director.track] = P;
	const keys = Object.keys(savedProfiles);
	for (let i = 0; i < keys.length - 200; i++)
		delete savedProfiles[keys[i]];
	const now = performance.now();
	if (now - profilesSavedAt > 15000) {
		profilesSavedAt = now;
		try {
			localStorage.setItem(
				PROFILE_KEY,
				JSON.stringify(savedProfiles),
			);
		} catch (_) {}
	}
}

function onTrackChange(key) {
	director.track = key;
	// a known song starts from what we learned last time; a new one starts empty
	const known = savedProfiles[key];
	director.history = known
		? Array.from({ length: 8 }, () => ({ ...known }))
		: [];
	director.since = 0; // no dwell wait: the new song may deserve a different vibe at once
}

listen("media-tick", (e) => {
	const m = pickPrimaryMedia(mediaListOf(e.payload));
	// video context: a video player, or anything long enough to be an episode/film
	// rather than a song. Mostly dialogue -- the eyes must not "vibe" to it.
	director.video =
		!!m &&
		((m.duration || 0) > 900 ||
			/potplayer|vlc|mpv|mpc|wmplayer|kmplayer|gom|video|movies|netflix|primevideo|disney|hulu/i.test(
				m.source || "",
			));
	if (!m) return;
	const key = `${m.title || ""}|${m.artist || ""}`;
	if (key !== director.track) onTrackChange(key);
});

function directorSample(now, alive) {
	const a = director.acc;
	director.acc = newAcc();
	const secs = Math.max(
		0.5,
		(now - director.lastSample) / 1000,
	);
	director.lastSample = now;
	const n = a.n;
	let cand = "silent";
	if (alive && n > 0 && a.level / n >= 0.05) {
		director.history.push({
			voice: a.voice / n,
			energy: a.energy / n,
			level: a.level / n,
			bass: a.bass / n,
			beats: a.beats / secs,
			kicks: a.kicks / secs,
		});
		while (director.history.length > PROFILE_SECS)
			director.history.shift();
		const P = avgHistory(director.history);
		const recentVoice = avgHistory(
			director.history.slice(-4),
		).voice;
		const cur = director.state;
		if (director.video) {
			// movies / episodes: mostly dialogue over ambience. Lean hard toward "talk"
			// (the analyzer under-scores speech mixed with music and effects) and
			// otherwise stay "calm": no dancing, no rings, no head-banging.
			cand =
				recentVoice > (cur === "talk" ? 0.12 : 0.2)
					? "talk"
					: "calm";
		} else if (recentVoice > (cur === "talk" ? 0.3 : 0.5))
			cand = "talk";
		// punchy: a kick lands steadily, it is energetic and loud (vocals are fine -- the kick detector ignores them)
		else if (
			P.kicks >= (cur === "bang" ? 1.2 : 1.5) &&
			P.energy >= (cur === "bang" ? 0.25 : 0.3) &&
			P.level > 0.5
		)
			cand = "bang";
		else if (
			P.energy < (cur === "chill" ? 0.27 : 0.2) &&
			P.beats < 1.4
		)
			cand = "chill";
		else cand = "groove";
		if (director.history.length >= 12) rememberProfile(P);
	}
	if (cand === director.state) {
		director.count = 0;
		return;
	}
	if (cand === "silent") {
		// silence is not ambiguous: no waiting
		director.state = "silent";
		director.count = 0;
		director.since = now;
		return;
	}
	director.count =
		cand === director.cand ? director.count + 1 : 1;
	director.cand = cand;
	if (director.count >= 2 && now - director.since >= 2000) {
		director.state = cand;
		director.count = 0;
		director.since = now;
	}
}

listen("audio-tick", (e) => {
	const a = e.payload;
	director.acc.n++;
	director.acc.voice += a.voice;
	director.acc.energy += a.energy;
	// genre profiling reads the peak-normalized twin (quiet-but-active
	// still counts); visuals read the absolute `level` below
	director.acc.level += a.nlevel ?? a.level;
	director.acc.bass += a.bass;
	if (a.beat) director.acc.beats++;
	if (a.kick) director.acc.kicks++;
	audio.target = a.level;
	audio.kind = a.kind;
	audio.voice = a.voice;
	// Rust now sends 16 log-spaced bands; older payloads (or the demo page)
	// still send the 4 legacy splits -- expand those across the 16 lights.
	if (
		Array.isArray(a.bands) &&
		a.bands.length === VIZ_BANDS
	) {
		audio.bandTarget = a.bands.slice();
	} else {
		const legacy = [
			a.bass || 0,
			a.lowmid || 0,
			a.mid || 0,
			a.high || 0,
		];
		audio.bandTarget = Array.from(
			{ length: VIZ_BANDS },
			(_, i) =>
				legacy[
					Math.min(3, Math.floor((i / VIZ_BANDS) * 4))
				],
		);
	}
	audio.bpm = a.bpm;
	audio.conf = a.conf;
	audio.panTarget = a.pan;
	audio.energy = a.energy;
	audio.lastTick = performance.now();
	// head-bang: only a low-end kick-drum hit moves the head (vocals and melody
	// onsets don't), and the head is back up before the next hit
	if (
		a.kick &&
		audio.bang > 0.25 &&
		audio.lastTick - audio.lastKickAt > 170
	) {
		audio.lastKickAt = audio.lastTick;
		audio.nod.v += 40 * audio.bang;
	}
	// one reaction per hit: never re-trigger within 220 ms of the last one
	if (
		a.beat &&
		a.level > 0.15 &&
		audio.lastTick - audio.lastBeatAt > 220
	) {
		audio.lastBeatAt = audio.lastTick;
		// every beat bounces the eyes and sends a ripple through the island
		// chill music barely bounces: hits are scaled down by how relaxed it is
		audio.kick.v +=
			(14 + 8 * a.level) *
			(1 - 0.88 * audio.chill) *
			(1 - 0.9 * audio.bang);
	}
	// sound-wave rings are bass-only: a kick-drum hit, or a strong low-end
	// onset. Hats, snares, vocals and melody never make rings.
	const bassNow = Array.isArray(a.bands)
		? (a.bands[0] + a.bands[1] + a.bands[2]) / 3
		: a.bass || 0;
	if (
		a.kick &&
		audio.chill < 0.55 &&
		director.state !== "calm" &&
		director.state !== "talk"
	) {
		beat(
			0.25 + 0.75 * Math.min(1, 0.4 + 0.6 * a.level),
			a.pan,
		);
	} else if (
		a.hit > 0.25 &&
		bassNow > 0.45 &&
		audio.chill < 0.55 &&
		director.state !== "calm" &&
		director.state !== "talk"
	) {
		beat(
			0.12 + 0.88 * a.hit * (0.45 + 0.55 * a.level),
			a.pan,
		);
	}
});

function smoothTo(cur, target, dt, up, down) {
	return (
		cur +
		(target - cur) *
			(1 - Math.exp(-dt * (target > cur ? up : down)))
	);
}

function updateAudio(now, dt) {
	const alive = now - audio.lastTick < 700;
	const tgt = alive ? audio.target : 0;
	// lights linger: fast attack, then a fixed slow linear fall (0..1/sec)
	// instead of exponential decay, so a dropped value fades evenly.
	const LEVEL_FALL = 1;
	audio.level =
		tgt > audio.level
			? smoothTo(audio.level, tgt, dt, 30, 8)
			: Math.max(tgt, audio.level - LEVEL_FALL * dt);
	const BAND_FALL = 0.1;
	for (let i = 0; i < VIZ_BANDS; i++) {
		const bt = alive ? audio.bandTarget[i] || 0 : 0;
		const cur = audio.bands[i];
		audio.bands[i] =
			bt > cur
				? smoothTo(cur, bt, dt, 40, 10)
				: Math.max(bt, cur - BAND_FALL * dt);
	}
	audio.pan = smoothTo(
		audio.pan,
		alive ? audio.panTarget : 0,
		dt,
		18,
		6,
	);
	if (!alive) {
		director.acc = newAcc();
		if (director.state !== "silent") {
			director.state = "silent";
			director.since = now;
		}
	} else if (now - director.lastSample >= 1000) {
		directorSample(now, alive);
	}
	const vibe = director.state;
	audio.voiceSm = smoothTo(
		audio.voiceSm,
		vibe === "talk" ? 1 : 0,
		dt,
		3,
		2.5,
	);
	const voice = audio.voiceSm;
	const chillTarget = vibe === "chill" ? 1 : 0;
	audio.bang = smoothTo(
		audio.bang,
		vibe === "bang" ? 1 : 0,
		dt,
		3.5,
		2.5,
	);
	audio.chill = smoothTo(
		audio.chill,
		chillTarget,
		dt,
		1.2,
		3,
	);
	// (no breath rings: rings are bass-only, chill stays as light)
	audio.mode =
		!alive || audio.level < 0.05
			? "silent"
			: vibe === "talk"
				? "speech"
				: "music";
	// "calm" (video without much speech right now): a whisper of movement only
	audio.dance = smoothTo(
		audio.dance,
		audio.level * (1 - voice) * (vibe === "calm" ? 0.2 : 1),
		dt,
		12,
		4,
	);
	audio.talk = smoothTo(
		audio.talk,
		audio.level * voice,
		dt,
		12,
		4,
	);
	// fast envelopes: the eyes follow syllables and hits, not just loudness.
	// Voice lives ~800 Hz..3 kHz, which is roughly the upper-middle third of
	// the 16 log bands -- average a few of them for a stable syllable signal.
	const midAvg =
		(audio.bands[8] +
			audio.bands[9] +
			audio.bands[10] +
			audio.bands[11] +
			audio.bands[12]) /
		5;
	const env = 0.5 * audio.level + 0.5 * midAvg;
	audio.talkEnv = smoothTo(audio.talkEnv, env, dt, 60, 16);
	// head-bang nod spring: quick down, settles well inside one beat
	audio.nod.v += (420 * -audio.nod.p - 26 * audio.nod.v) * dt;
	audio.nod.p += audio.nod.v * dt;
	// beat kick: an underdamped spring, so each hit bounces and settles
	const acc = 260 * -audio.kick.p - 15 * audio.kick.v;
	audio.kick.v += acc * dt;
	audio.kick.p += audio.kick.v * dt;
	// sway at half the beat rate (a nod every other beat), locked to the tempo estimate
	if (audio.dance > 0.04)
		audio.sway +=
			dt *
			Math.PI *
			(audio.bpm / 60) *
			(1 - 0.55 * audio.chill);
}


export { audio, director, smoothTo, updateAudio, VIZ_BANDS };
