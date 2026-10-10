// Canvas-painted idle "eyes" -- a straight port of `_paint_eyes`/`_tick_eyes`
// from the Qt prototype (main.py), driven by the Rust-side `cursor-tick`
// event instead of a QTimer polling QCursor.pos(). No framework, no bundler:
// this is the whole idle-state UI, kept as cheap as the old QPainter path.

import { initFloats } from "./floats.js";
import { initPluginUi, pluginCard, setRings, whenText } from "./pluginui.js";
import { plugins } from "./plugins/index.js";
import {
	FLOATS,
	LLM_COLOR,
	LLM_LETTER,
	LLM_RING_C,
	llmCtxColor,
	llmStateIcon,
	ringEls,
	setRing,
	ICON,
	call,
	el,
	floatKeys,
	host,
	invoke,
	li,
	listen,
	mediaListOf,
	pickPrimaryMedia,
	pills,
	ringFrom,
	view,
	withIcon,
} from "./kit.js";

if (FLOATS) document.documentElement.classList.add("floats");

// ---- the island is not a web page: nothing of the browser should work on it ----
// (the webview's own accelerators, devtools, zoom and menus are switched off in Rust; this covers
// what is left: the keys, the wheel, files dropped on it, dragged links and images, middle click)
const BLOCKED_KEYS =
	/^(F\d{1,2}|BrowserBack|BrowserForward|BrowserRefresh|BrowserSearch|BrowserHome|PrintScreen)$/;
const BLOCKED_CTRL = /^[rpsufgjhdnotwebk\-=+0\[\]]$/i; // ctrl + any of these is a browser command; a, c, v, x, z, y stay
window.addEventListener(
	"keydown",
	(e) => {
		const typing =
			e.target instanceof HTMLInputElement ||
			e.target instanceof HTMLTextAreaElement;
		const block =
			BLOCKED_KEYS.test(e.key) ||
			((e.ctrlKey || e.metaKey) &&
				BLOCKED_CTRL.test(e.key)) ||
			(e.ctrlKey &&
				e.shiftKey &&
				/^[ijcnrdtw]$/i.test(e.key)) ||
			(e.altKey &&
				/^(ArrowLeft|ArrowRight|Home)$/.test(e.key)) ||
			(e.key === "Backspace" &&
				!typing &&
				!e.target.isContentEditable);
		if (block) {
			e.preventDefault();
			e.stopPropagation();
		}
	},
	true,
);
window.addEventListener(
	"wheel",
	(e) => {
		if (e.ctrlKey) e.preventDefault();
	},
	{ passive: false },
);
for (const t of [
	"dragstart",
	"dragover",
	"drop",
	"gesturestart",
	"gesturechange",
	"auxclick",
]) {
	window.addEventListener(t, (e) => e.preventDefault(), {
		passive: false,
	});
}
window.addEventListener(
	"mousedown",
	(e) => {
		if (e.button === 1) e.preventDefault();
	},
	true,
); // no autoscroll

const pill = document.getElementById("pill");
const canvas = document.getElementById("face");
const ctx = canvas.getContext("2d");
const hubEyesCanvas = document.getElementById("hub-eyes");
const hubEyesCtx = hubEyesCanvas.getContext("2d");

let dpr = window.devicePixelRatio || 1;

function sizeCanvasToElement(c) {
	// Only sets the drawing-buffer resolution (width/height attributes) --
	// never the inline style size. CSS alone controls the displayed size
	// (#face and #hub-eyes both already size themselves via their own rules);
	// writing inline style here bit us once already: #hub-eyes was
	// `display:none` (0x0) the first time this ran at page load, and setting
	// `style.height = "0px"` inline then permanently overrode its CSS
	// `height: 40px` rule -- inline style always wins over a class -- so it
	// stayed invisible forever after, even once re-measured on becoming visible.
	// layout size, not getBoundingClientRect: that one includes CSS transforms (the shrunken pinned
	// island, entrance animations), and the buffer must not follow those
	const width = c.offsetWidth;
	const height = c.offsetHeight;
	c.width = Math.round(width * dpr);
	c.height = Math.round(height * dpr);
}

function resizeCanvas() {
	dpr = window.devicePixelRatio || 1;
	sizeCanvasToElement(canvas);
	sizeCanvasToElement(hubEyesCanvas);
}
window.addEventListener("resize", resizeCanvas);
resizeCanvas();

// -- eye state: blink + squash/stretch (ported from the Qt prototype) plus a
// small mood system (happy / sleepy / wide) driven by springs, so expression
// changes overshoot and settle like everything else in the island --
const eyeState = {
	blinkState: "open", // "open" | "closing" | "opening"
	blinkStart: 0,
	nextBlinkAt: performance.now() + rand(2500, 5500),
	openness: 1.0,
	squish: 0.0,
	squishDir: [0, 0],
	lastCursor: { x: 0, y: 0 },
	smoothedCursor: { x: 0, y: 0 },
	doubleBlink: false,
	winkSide: 0, // -1 left eye only, +1 right eye only, 0 both
};

const mood = {
	happy: { p: 0, v: 0 },
	sleepy: { p: 0, v: 0 },
	wide: { p: 0, v: 0 },
	xd: { p: 0, v: 0 }, // > <  (excited, punchy music)
	bliss: { p: 0, v: 0 }, // u u  (chill)
	hover: false, // cursor is over the island
	happyUntil: 0,
	wideUntil: 0,
	lastMoveAt: performance.now(),
	// idle glances: after the cursor sits still a while the eyes look around
	glance: { x: 0, y: 0 },
	glanceTarget: { x: 0, y: 0 },
	glanceW: 0,
	nextGlanceAt: 0,
	lastFrame: performance.now(),
	force: null,
};
window.__mood = mood;

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
		vizBeat(
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
		vizBeat(
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

function rand(min, max) {
	return min + Math.random() * (max - min);
}

function springTo(s, target, dt) {
	// underdamped on purpose: an expression change pops past its target and settles
	const a = 340 * (target - s.p) - 19 * s.v;
	s.v += a * dt;
	s.p += s.v * dt;
}

// Which expression the music calls for. Picked when the director's vibe
// changes and then re-rolled only every 5-9 s, so it reads as a deliberate
// mood, never a flicker.
const emote = { name: "none", vibe: "", until: 0 };
function pickEmote(now) {
	if (emote.forced) return;
	if (director.state === emote.vibe && now < emote.until)
		return;
	emote.vibe = director.state;
	const options = {
		bang: ["xd", "arch", "xd", "arch", "open"],
		chill: ["bliss", "sleepy", "bliss", "open"],
		groove: ["open", "open", "arch"],
	}[emote.vibe] || ["none"];
	let next =
		options[Math.floor(Math.random() * options.length)];
	if (next === emote.name && options.length > 1)
		next =
			options[
				(options.indexOf(next) + 1) % options.length
			];
	emote.name = next;
	emote.until = now + rand(5000, 9000);
}

window.__setEmote = (name) => {
	// debug hook: pin an expression ("xd" | "bliss" | "arch" | "sleepy" | "open")
	emote.name = name;
	emote.vibe = director.state;
	emote.until = Infinity;
	emote.forced = true;
};

function updateMood(now) {
	const dt = Math.min((now - mood.lastFrame) / 1000, 0.05);
	mood.lastFrame = now;
	const idleFor = now - mood.lastMoveAt;
	const inIdle = currentView === "idle";

	const f = mood.force; // debug override (window.__mood.force = "happy" | "sleepy" | "wide")
	const wide =
		f === "wide" || (!f && now < mood.wideUntil) ? 1 : 0;
	const happy =
		f === "happy" ||
		(!f &&
			!wide &&
			((mood.hover && inIdle) || now < mood.happyUntil))
			? 1
			: 0;
	const sleepy =
		f === "sleepy" ||
		(!f &&
			!wide &&
			!happy &&
			idleFor > 25000 &&
			audio.mode === "silent")
			? 1
			: 0;
	springTo(mood.wide, wide, dt);
	// music-driven expressions (never over a hover / click reaction)
	pickEmote(now);
	const free = !happy && !wide && !f;
	const e = free ? emote.name : "none";
	springTo(
		mood.happy,
		Math.max(happy, e === "arch" ? 1 : 0),
		dt,
	);
	springTo(mood.xd, e === "xd" ? 1 : 0, dt);
	springTo(mood.bliss, e === "bliss" ? 1 : 0, dt);
	// relaxed, half-lidded eyes while the music is chill
	springTo(
		mood.sleepy,
		Math.max(
			sleepy,
			e === "sleepy"
				? 0.75
				: 0.3 * audio.chill * (e === "bliss" ? 0 : 1),
		),
		dt,
	);

	// glances: cursor parked for 4s+ -> look around now and then
	const glancing = idleFor > 4000 && !happy && !wide;
	if (glancing && now >= mood.nextGlanceAt) {
		const spots = [
			[-1, 0.1],
			[1, 0.1],
			[0.7, -0.8],
			[-0.7, -0.8],
			[0, 0],
			[-0.9, 0.5],
			[0.9, 0.5],
		];
		const [gx, gy] =
			spots[Math.floor(Math.random() * spots.length)];
		mood.glanceTarget = { x: gx, y: gy };
		mood.nextGlanceAt = now + rand(1400, 3400);
	}
	const k = 1 - Math.exp(-dt * 14);
	mood.glance.x += (mood.glanceTarget.x - mood.glance.x) * k;
	mood.glance.y += (mood.glanceTarget.y - mood.glance.y) * k;
	mood.glanceW +=
		((glancing ? 1 : 0) - mood.glanceW) *
		(1 - Math.exp(-dt * 8));
}

function tickBlink(now) {
	// no blinking while the face is doing an expression (> <, ^ ^, u u, sleepy lids):
	// it only blinks in its normal open-eyed mode. The next blink is pushed out so
	// it does not fire the instant the expression ends.
	const expressive =
		Math.max(
			mood.happy.p,
			mood.xd.p,
			mood.bliss.p,
			mood.sleepy.p,
		) > 0.35;
	if (expressive && eyeState.blinkState === "open") {
		eyeState.nextBlinkAt = Math.max(
			eyeState.nextBlinkAt,
			now + 1200,
		);
		return;
	}
	const slow =
		1 + Math.max(0, Math.min(mood.sleepy.p, 1)) * 1.8; // sleepy blinks are lazy
	if (
		eyeState.blinkState === "open" &&
		now >= eyeState.nextBlinkAt
	) {
		eyeState.blinkState = "closing";
		eyeState.blinkStart = now;
		// now and then: a wink instead of a blink
		if (
			!eyeState.doubleBlink &&
			mood.happy.p < 0.3 &&
			mood.sleepy.p < 0.3 &&
			Math.random() < 0.07
		) {
			eyeState.winkSide = Math.random() < 0.5 ? -1 : 1;
		} else if (!eyeState.doubleBlink) {
			eyeState.winkSide = 0;
		}
	} else if (eyeState.blinkState === "closing") {
		const t = (now - eyeState.blinkStart) / (80 * slow);
		if (t >= 1) {
			eyeState.blinkState = "opening";
			eyeState.blinkStart = now;
			eyeState.openness = 0;
		} else {
			eyeState.openness = 1 - t;
		}
	} else if (eyeState.blinkState === "opening") {
		const hold = eyeState.winkSide !== 0 ? 260 : 0; // a wink lingers
		const t =
			(now - eyeState.blinkStart - hold) / (120 * slow);
		if (t < 0) {
			eyeState.openness = 0;
		} else if (t >= 1) {
			eyeState.blinkState = "open";
			eyeState.openness = 1;
			if (
				!eyeState.doubleBlink &&
				eyeState.winkSide === 0 &&
				Math.random() < 0.22
			) {
				eyeState.doubleBlink = true; // quick second blink
				eyeState.nextBlinkAt = now + 110;
			} else {
				eyeState.doubleBlink = false;
				eyeState.winkSide = 0;
				eyeState.nextBlinkAt =
					now +
					(mood.sleepy.p > 0.5
						? rand(1800, 3400)
						: rand(2500, 5500));
			}
		} else {
			eyeState.openness = t;
		}
	}
}

// gaze vector (each axis -1..1) from one eye toward the cursor, blended with
// the idle-glance target when the cursor has been still for a while
function gazeFor(eyeX, eyeY) {
	const cx = Math.max(
		-1,
		Math.min(1, (eyeState.smoothedCursor.x - eyeX) / 60),
	);
	const cy = Math.max(
		-1,
		Math.min(1, (eyeState.smoothedCursor.y - eyeY) / 40),
	);
	const w = mood.glanceW;
	return [
		cx + (mood.glance.x - cx) * w,
		cy + (mood.glance.y - cy) * w,
	];
}

// Each eye is a closed shape built from two curves over x in [-1, 1]:
//   centerY(x) = c0 + k*x^2         (k > 0 bends the middle up: an arch)
//   top/bottom = centerY -/+ amp * sqrt(1 - x^2)
// A circle is c0=k=0 with top=bot=r; every mood is just a different set of
// numbers, so morphing between expressions is plain interpolation.
const EYE_SHAPES = {
	//            half-width, top, bottom, c0,   k
	neutral: {
		rx: 5.1,
		top: 5.8,
		bot: 5.8,
		c0: 0,
		k: 0,
		pw: 2,
	},
	happy: {
		rx: 6.0,
		top: 2.0,
		bot: 2.0,
		c0: -2.6,
		k: 5.2,
		pw: 2,
	}, // ^  ^ smiling arches
	sleepy: {
		rx: 5.6,
		top: 0.6,
		bot: 5.0,
		c0: -2.0,
		k: 0,
		pw: 2,
	}, // ◡  ◡ heavy lids
	wide: { rx: 4.6, top: 8.0, bot: 8.0, c0: 0, k: 0, pw: 2 }, // tall, startled ovals
	// >  < squeezed-shut chevrons: a sharp arch (pw ~ 1) drawn turned 90 degrees
	xd: {
		rx: 6.8,
		top: 1.5,
		bot: 1.5,
		c0: -3.5,
		k: 7.0,
		pw: 1.1,
	},
	// u  u blissfully closed
	bliss: {
		rx: 6.0,
		top: 2.0,
		bot: 2.0,
		c0: 2.6,
		k: -5.2,
		pw: 2,
	},
};
const EYE_KEYS = ["rx", "top", "bot", "c0", "k", "pw"];

function eyeParams(happy, sleepy, wide, xd = 0, bliss = 0) {
	const out = {};
	for (const key of EYE_KEYS) {
		const n = EYE_SHAPES.neutral[key];
		out[key] =
			n +
			happy * (EYE_SHAPES.happy[key] - n) +
			sleepy * (EYE_SHAPES.sleepy[key] - n) +
			wide * (EYE_SHAPES.wide[key] - n) +
			xd * (EYE_SHAPES.xd[key] - n) +
			bliss * (EYE_SHAPES.bliss[key] - n);
	}
	out.top = Math.max(out.top, 0.4); // springs overshoot; never invert a curve
	out.bot = Math.max(out.bot, 0.4);
	return out;
}

const EYE_STEPS = 32;

function eyePath(c, p, blink) {
	c.beginPath();
	for (let i = 0; i <= EYE_STEPS; i++) {
		const x = -1 + (2 * i) / EYE_STEPS;
		const s = Math.sqrt(Math.max(0, 1 - x * x));
		const y =
			p.c0 +
			p.k * Math.pow(Math.abs(x), p.pw) -
			p.top * blink * s;
		if (i === 0) c.moveTo(x * p.rx, y);
		else c.lineTo(x * p.rx, y);
	}
	for (let i = EYE_STEPS; i >= 0; i--) {
		const x = -1 + (2 * i) / EYE_STEPS;
		const s = Math.sqrt(Math.max(0, 1 - x * x));
		c.lineTo(
			x * p.rx,
			p.c0 +
				p.k * Math.pow(Math.abs(x), p.pw) +
				p.bot * blink * s,
		);
	}
	c.closePath();
}

// Draws the eyes centered at (centerX, cy) on the given context -- shared by
// the idle pill's canvas and the hub header. `opts.scale` enlarges them.
function paintEyesAt(targetCtx, centerX, cy, opts = {}) {
	const sc = opts.scale || 1;
	const wide = Math.max(0, Math.min(mood.wide.p, 1.3));
	const happy = Math.max(0, Math.min(mood.happy.p, 1.25));
	const sleepy = Math.max(0, Math.min(mood.sleepy.p, 1.1));
	const xd = Math.max(0, Math.min(mood.xd.p, 1.15));
	const bliss = Math.max(0, Math.min(mood.bliss.p, 1.15));
	const [dirx, diry] = eyeState.squishDir;
	// motion squash & stretch stays axis-aligned (no rotation): rotating the eye
	// made blinks close sideways when the cursor moved vertically
	const ax = Math.abs(dirx);
	const ay = Math.abs(diry);
	const sq = eyeState.squish;
	const stretchX = Math.max(
		1 + sq * (0.25 * ax - 0.12 * ay),
		0.6,
	);
	const stretchY = Math.max(
		1 + sq * (0.25 * ay - 0.12 * ax),
		0.6,
	);

	// audio-driven motion for the whole face
	const kickP = Math.max(0, Math.min(audio.kick.p, 1.2));
	const chill = audio.chill;
	const xdW = Math.min(xd, 1); // how much the eyes are currently the > < chevrons
	const bang = audio.bang;
	const nodP = Math.max(-0.4, Math.min(audio.nod.p, 1.4));
	// while head-banging the head only moves on the kick (the usual bass-band bob
	// and beat lift would also follow vocals and melody, so they fade out)
	const groupBob =
		-(
			audio.bands[0] * 1.5 * (1 - 0.8 * chill) +
			kickP * 1.7
		) *
			audio.dance *
			sc *
			(1 - bang) +
		nodP * 6.5 * bang * sc;
	// the > < chevrons rock gently side to side (~1.3 Hz, a happy wiggle rather
	// than a shake); not tied to the beat
	const tt = performance.now() / 1000;
	const shakeX =
		(Math.sin(tt * 8.2) + 0.3 * Math.sin(tt * 4.6 + 1.3)) *
		(1.3 + 0.5 * audio.level) *
		xdW *
		sc;
	const swayX =
		Math.sin(audio.sway) *
			2.6 *
			(1 - 0.35 * chill) *
			(1 - 0.8 * bang) *
			audio.dance *
			sc +
		shakeX;
	const tilt =
		Math.sin(audio.sway) *
		0.11 *
		(1 - 0.6 * chill) *
		audio.dance *
		(1 - xdW) *
		(1 - bang); // head-nod lean
	const kickSquash =
		1 +
		kickP * 0.16 * audio.dance * (1 - bang) -
		nodP * 0.16 * bang;
	const talkAmt =
		Math.max(-0.32, Math.min(audio.talkEnv - 0.32, 0.68)) *
		audio.talk *
		1.8;
	const talkY = 1 + talkAmt * 0.8;
	const talkX = 1 - talkAmt * 0.24;

	targetCtx.save();
	targetCtx.translate(centerX + swayX, cy + groupBob);
	targetCtx.rotate(tilt);
	targetCtx.translate(-centerX, -cy);

	for (const side of [-1, 1]) {
		const ex = centerX + side * 13.5 * sc;
		let [gx, gy] = gazeFor(ex, cy);
		// spatial awareness: glance toward wherever the sound is panned
		gx += audio.pan * 0.6 * Math.min(1, audio.level * 2);

		// a wink is just this one eye morphing into the happy arch and back
		const winking = eyeState.winkSide === side;
		const eyeHappy = winking
			? Math.max(happy, 1 - eyeState.openness)
			: happy;
		const p = eyeParams(eyeHappy, sleepy, wide, xd, bliss);
		const blink = winking
			? 1
			: Math.max(eyeState.openness, 0.06);

		targetCtx.save();
		// without pupils the whole eye does the looking
		targetCtx.translate(
			ex + gx * 3.2 * sc,
			cy + gy * 1.7 * sc,
		);
		targetCtx.scale(sc * talkX, sc * talkY * kickSquash);
		// eyelids always close vertically, whatever else the eye is doing
		targetCtx.scale(1, blink);
		// the ^ ^ arch / sleepy lids stay undistorted: squash fades out as they form
		const calm =
			1 -
			Math.min(Math.max(eyeHappy, sleepy, xd, bliss), 1);
		targetCtx.scale(
			1 + (stretchX - 1) * calm,
			1 + (stretchY - 1) * calm,
		);

		// the > < chevrons are the arch turned toward the middle (left eye >, right eye <)
		if (xd > 0.01)
			targetCtx.rotate(
				-side * (Math.PI / 2) * Math.min(xd, 1),
			);
		eyePath(targetCtx, p, 1);
		const g = targetCtx.createRadialGradient(
			-p.rx * 0.3,
			-p.top * 0.35,
			0,
			-p.rx * 0.3,
			-p.top * 0.35,
			p.rx * 2,
		);
		g.addColorStop(0, "rgb(252, 252, 254)");
		g.addColorStop(1, "rgb(222, 226, 236)");
		targetCtx.fillStyle = g;
		targetCtx.fill();
		targetCtx.restore();
	}

	targetCtx.restore();
}

function paintEyes() {
	const w = canvas.width / dpr;
	const h = canvas.height / dpr;
	ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
	ctx.clearRect(0, 0, w, h);
	// breathing: the eyes drift up and down a hair, on a slow cycle
	const t = performance.now();
	const cy =
		h / 2 + Math.sin((t / 3600) * Math.PI * 2) * 0.55;
	paintEyesAt(ctx, w / 2, cy);
}

function paintHubEyes() {
	const w = hubEyesCanvas.width / dpr;
	const h = hubEyesCanvas.height / dpr;
	hubEyesCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
	hubEyesCtx.clearRect(0, 0, w, h);

	const cx = w / 2;
	const cy = h / 2;

	paintEyesAt(hubEyesCtx, cx, cy, { scale: 1.3 });
}


// pop a view's children in with a small stagger (see .enter in style.css)
function playEnter(container) {
	container.classList.remove("enter");
	Array.from(container.children).forEach((c, i) =>
		c.style.setProperty("--i", i),
	);
	void container.offsetWidth;
	container.classList.add("enter");
}

// Authoritative view name ("idle" | "media" | "game"), driven by Rust's
// view-tick -- Rust owns the resize/priority decision (media beats game),
// the frontend just reflects it rather than re-deriving it from has_session/
// has_game flags that could race the resize.
let currentView = "idle";

// -- audio visualizer: an aura that lives in the island itself instead of bars.
// 16 soft gradient lights (one per frequency band, ~60 Hz..12 kHz) drift
// through the pill; the detected genre picks how they dance (talk huddles,
// chill drifts, groove/pop orbits, bang/rock slams). Bass-only kick hits
// send a ripple out from the eyes; a voice gets a warm halo that pulses
// with the syllables (halo only, never rings). Drawn under all content. --
const vizCanvas = document.getElementById("viz");
const vizCtx = vizCanvas.getContext("2d");
const VIZ_SCALE = 0.7; // render below device resolution: it's all soft glow anyway
const viz = { rings: [], hue: 150, on: false, fadeFrom: null, lightCur: [] };
const VIZ_FADE_MS = 700;

// the aura also bleeds outside the island into the window's margin (settings: Ambient, 0 = off)
const bleedCanvas = document.getElementById("viz-bleed");
const bleedCtx = bleedCanvas.getContext("2d");
let bleedLevel = 0.6; // 0..1

function sizeViz() {
	// (layout size: the pinned island may be drawn scaled down, the buffer must not follow)
	vizCanvas.width = Math.max(
		1,
		Math.round(pill.offsetWidth * VIZ_SCALE),
	);
	vizCanvas.height = Math.max(
		1,
		Math.round(pill.offsetHeight * VIZ_SCALE),
	);
	bleedCanvas.width = Math.max(
		1,
		Math.round(window.innerWidth * VIZ_SCALE),
	);
	bleedCanvas.height = Math.max(
		1,
		Math.round(window.innerHeight * VIZ_SCALE),
	);
}

// The light outside the island is the aura (see paintViz) drawn once over the island's shape and
// kept where a soft mask lets it through: bright at the island's edge, gone a few pixels away, so it
// reads as light spilling out, never as a slab. The mask is only rebuilt when the island's shape
// changes (while it resizes), not every frame.
const maskCanvas = document.createElement("canvas");
const maskCtx = maskCanvas.getContext("2d");
let maskKey = "";
function paintBleed() {
	const r = pill.getBoundingClientRect();
	const S = VIZ_SCALE;
	// reach follows the setting (it can never go past the margin the window has)
	const reach = Math.min(cursorPad * 0.5, 24) * bleedLevel;
	const grow = reach * 0.3;
	const x = (r.left - grow) * S;
	const y = (r.top - grow) * S;
	const w = (r.width + 2 * grow) * S;
	const h = (r.height + 2 * grow) * S;
	const radius = Math.min(
		((parseFloat(
			getComputedStyle(pill).borderTopLeftRadius,
		) || 0) +
			grow) *
			S,
		w / 2,
		h / 2,
	);
	const blur = Math.max(0.5, reach * 0.5 * S);
	const key = [
		x,
		y,
		w,
		h,
		radius,
		blur,
		bleedCanvas.width,
		bleedCanvas.height,
	]
		.map((v) => Math.round(v * 10))
		.join();
	if (key !== maskKey) {
		maskKey = key;
		maskCanvas.width = bleedCanvas.width;
		maskCanvas.height = bleedCanvas.height;
		maskCtx.filter = `blur(${blur}px)`;
		maskCtx.fillStyle = "#fff";
		maskCtx.beginPath();
		maskCtx.roundRect(x, y, w, h, radius);
		maskCtx.fill();
		maskCtx.filter = "none";
	}
	bleedCtx.setTransform(1, 0, 0, 1, 0, 0);
	bleedCtx.globalCompositeOperation = "source-over";
	bleedCtx.clearRect(
		0,
		0,
		bleedCanvas.width,
		bleedCanvas.height,
	);
	// the light has to exist as far out as the mask lets it reach (or the mask's soft edge is cut
	// off where the picture ends): the aura is stretched over the island grown by the full reach
	const ext = Math.max(
		grow,
		Math.min(cursorPad - 2, reach * 2.2),
	);
	bleedCtx.drawImage(
		vizCanvas,
		(r.left - ext) * S,
		(r.top - ext) * S,
		(r.width + 2 * ext) * S,
		(r.height + 2 * ext) * S,
	);
	bleedCtx.globalCompositeOperation = "destination-in";
	bleedCtx.drawImage(maskCanvas, 0, 0);
	bleedCtx.globalCompositeOperation = "source-over";
}

function clearBleed() {
	bleedCtx.clearRect(
		0,
		0,
		bleedCanvas.width,
		bleedCanvas.height,
	);
	document.body.classList.remove("bleeding");
}
new ResizeObserver(sizeViz).observe(pill);
sizeViz();

// where the eyes sit inside the pill, in CSS px
function vizEyeCenter() {
	return [
		pill.offsetWidth / 2,
		currentView === "hub" ? 38 : pill.offsetHeight / 2,
	];
}

// Bass-only rings: one ring per kick-drum / strong low-end onset -- no
// merging and no rate limit beyond the analyzer's onset spacing, so a fast
// run of kicks gives a fast run of rings, while nothing is drawn that the
// low end did not actually do. (A cap keeps a pathological input from
// piling up rings.)
function pushRing(now, s, hue, pan) {
	viz.rings.push({ t: now, s, sv: s, hue, pan: pan || 0 });
	if (viz.rings.length > 12) viz.rings.shift();
}

function vizBeat(strength, pan) {
	pushRing(performance.now(), strength, viz.hue + 20, pan);
}

// -- genre choreography: where each of the 16 lights sits, per vibe --
// talk = huddled murmur, chill = slow drift, groove/pop = orbit,
// bang/rock = slam, calm/silent = barely there.
// deterministic per-light 0..1 variation, so no two lights ever share a
// path -- every choreography is non-uniform by construction.
function lightVar(i, salt) {
	const x = Math.sin(i * 127.1 + salt * 311.7) * 43758.5453;
	return x - Math.floor(x);
}

function lightPose(i, n, t, vibe, w, h, kickP, env) {
	const frac = i / n;
	const TAU = Math.PI * 2;
	const v1 = lightVar(i, 1);
	const v2 = lightVar(i, 2);
	const v3 = lightVar(i, 3);
	const v4 = lightVar(i, 4);
	switch (vibe) {
		case "talk": {
			// simple talking: loose huddle -- wide enough that the
			// lights read as separate voices, not one blob
			const ax = 0.34 * (0.55 + 0.45 * v1);
			const fx = 0.9 * (0.7 + 0.6 * v2);
			const ay = 0.28 * (0.55 + 0.45 * v4);
			const fy = 1.1 * (0.7 + 0.6 * v1);
			return [
				w *
					(0.5 +
						ax * Math.sin(t * fx + i * 0.55 + v3 * TAU) +
						audio.pan * 0.1),
				h *
					(0.5 +
						ay * Math.sin(t * fy + i * 0.9 + v2 * TAU)),
			];
		}
		case "chill": {
			// soft drift, each light its own slow lane
			const ax = 0.42 * (0.55 + 0.45 * v1);
			const fx = 0.25 * (0.6 + 0.9 * v2);
			const ay = 0.36 * (0.55 + 0.45 * v3);
			const fy = 0.3 * (0.6 + 0.9 * v4);
			return [
				w *
					(0.5 +
						ax *
							Math.sin(
								t * fx + i * 2.4 + v3 * TAU,
							)),
				h *
					(0.5 +
						ay *
							Math.sin(
								t * fy + i * 1.8 + 1 + v4 * TAU,
							)),
			];
		}
		case "bang": {
			// rock/hard song: fast orbit + vertical slam, each light its
			// own speed, radius and phase -- never a ring in formation
			const spd = 2.2 * (0.7 + 0.6 * v1);
			const ang = t * spd + frac * TAU + v2 * TAU;
			const slam = kickP * 0.16 * (0.6 + 0.8 * v4);
			const rx = 0.44 * (0.6 + 0.4 * v3);
			const ry = 0.38 * (0.6 + 0.4 * v4);
			return [
				w *
					(0.5 +
						rx *
							Math.cos(
								ang +
									Math.sin(
										t * (5 + 3 * v1) +
											i +
											v2 * 6,
									) *
										0.35,
							) +
						audio.pan * 0.14),
				h *
					(0.5 +
						ry *
							Math.sin(
								ang * (1.1 + 0.5 * v2) +
									i +
									v3 * 6,
							) -
						slam),
			];
		}
		case "calm": {
			// video ambience: whisper, each light its own still point
			const ax = 0.26 * (0.55 + 0.45 * v1);
			const fx = 0.3 * (0.6 + 0.9 * v2);
			const ay = 0.2 * (0.55 + 0.45 * v3);
			const fy = 0.35 * (0.6 + 0.9 * v4);
			return [
				w *
					(0.5 +
						ax *
							Math.sin(
								t * fx + i * 1.1 + v3 * TAU,
							)),
				h *
					(0.5 +
						ay *
							Math.sin(
								t * fy + i * 1.7 + v4 * TAU,
							)),
			];
		}
		case "groove": // pop song and friends: loose orbit, no formation
		default: {
			const spd =
				(0.5 + Math.PI * (audio.bpm / 60) * 0.04) *
				(0.7 + 0.6 * v1);
			const ang =
				t * spd +
				frac * TAU +
				v2 * TAU +
				Math.sin(audio.sway + v3 * 6) * 0.35;
			const bounce = (env || 0) * 0.1 + kickP * 0.05;
			const rx = 0.38 * (0.6 + 0.4 * v3);
			const ry = (0.3 + bounce) * (0.6 + 0.4 * v4);
			return [
				w *
					(0.5 +
						rx * Math.cos(ang) +
						0.04 * Math.sin(t * (1.3 + v1) + i * 2.1) +
						audio.pan * 0.12),
				h *
					(0.5 +
						ry * Math.sin(ang) +
						0.04 * Math.sin(t * (1.7 + v2) + i * 1.3)),
			];
		}
	}
}

function lightHue(i, n, t, vibe, baseHue) {
	const frac = i / n;
	switch (vibe) {
		case "talk":
			return 28 + frac * 40 + Math.sin(t * 0.7 + i) * 5; // warm voices, still distinct
		case "chill":
			return (
				195 +
				Math.sin(t * 0.2 + i * 0.5) * 25 +
				frac * 30
			); // teal-blue wash
		case "bang":
			return (
				(8 +
					frac * 45 +
					t * 40 +
					Math.sin(t * 5 + i) * 12 +
					360) %
				360
			); // hot, fast cycle
		case "calm":
			return (baseHue + frac * 40) % 360;
		case "groove": // pop rainbow: full circle across the 16 lights
		default:
			return (baseHue + frac * 300) % 360;
	}
}

function paintViz(now, dt) {
	const c = vizCtx;
	const w = vizCanvas.width;
	const h = vizCanvas.height;
	const S = VIZ_SCALE;
	// stay live while any lingering band is still visible, so the slow fall
	// actually plays out instead of getting cut by the fade-out.
	let peakBand = 0;
	for (let i = 0; i < VIZ_BANDS; i++)
		if (audio.bands[i] > peakBand)
			peakBand = audio.bands[i];
	const live =
		audio.level > 0.03 ||
		peakBand > 0.04 ||
		viz.rings.length > 0;
	if (!live) {
		if (viz.on) {
			// the sound stopped: the light fades out over a moment instead of vanishing
			if (viz.fadeFrom === null) viz.fadeFrom = now;
			const f = 1 - (now - viz.fadeFrom) / VIZ_FADE_MS;
			if (f > 0) {
				vizCanvas.style.opacity = String(f);
				bleedCanvas.style.opacity = String(
					Math.min(1, bleedLevel * 1.1) * f,
				);
				return;
			}
			c.clearRect(0, 0, w, h);
			viz.on = false;
			viz.fadeFrom = null;
			vizCanvas.style.opacity = "";
			clearBleed();
		}
		return;
	}
	viz.fadeFrom = null;
	vizCanvas.style.opacity = "";
	viz.on = true;
	c.setTransform(1, 0, 0, 1, 0, 0);
	c.clearRect(0, 0, w, h);
	// source-over, not additive: stacked lights converge toward the cap
	// instead of summing past it, so a huddle never blows out white
	c.globalCompositeOperation = "source-over";

	viz.hue =
		(viz.hue + dt * 7 * (1 - 0.75 * audio.chill)) % 360;
	const chill = audio.chill;
	const vibe = director.state; // silent | talk | chill | groove | bang | calm: picks the dance
	const t =
		(now / 1000) *
		(vibe === "chill" ? 0.45 : vibe === "talk" ? 1 : 1);
	const baseHue = viz.hue;
	// the hub is mostly text -- keep the aura subtle there
	const gain =
		currentView === "hub"
			? 0.5
			: currentView === "idle"
				? 1
				: 0.7;
	const musicish = 0.35 + 0.65 * (1 - audio.voiceSm);
	const size = Math.max(w, h);
	// a narrow island packs the same 16 lights into less room, and they add up to a glare:
	// below the usual compact width they get smaller and fainter
	const crowd = Math.min(1, Math.max(0.5, w / (260 * S)));
	const kickP = Math.max(0, Math.min(audio.kick.p, 1.2));

	// view sizing: the expanded island is large, so lights roam it fully at
	// full size; the collapsed pill / notification banner / notification pills
	// are significantly smaller, so their lights shrink to match.
	const expanded = currentView === "hub";
	const compact =
		currentView === "idle" ||
		currentView === "notification" ||
		currentView === "brief" ||
		currentView === "usage_peek";
	const rMul = expanded ? 1 : compact ? 0.65 : 0.8;

	// 16 lights expanded, 8 elsewhere: the small views condense adjacent
	// band pairs so all 16 bands still show through 8 lights.
	// Genre switches glide ease-in-out: each light rides a critically
	// damped spring toward its choreography target -- it accelerates out
	// of the old dance and settles softly into the new one, instead of
	// jumping or easing only one way.
	const activeN = expanded ? VIZ_BANDS : 8;
	const STIFF = 20;
	const DAMP = 9;
	for (let i = 0; i < activeN; i++) {
		const env = expanded
			? audio.bands[i] || 0
			: ((audio.bands[2 * i] || 0) + (audio.bands[2 * i + 1] || 0)) / 2;
		const [tx, ty] = lightPose(i, activeN, t, vibe, w, h, kickP, env);
		// safety: never let a center leave the island (an off-canvas
		// light reads as a light that disappeared)
		const tnx = Math.min(0.98, Math.max(0.02, tx / Math.max(1, w)));
		const tny = Math.min(0.98, Math.max(0.02, ty / Math.max(1, h)));
		let cur = viz.lightCur[i];
		if (!cur) {
			cur = viz.lightCur[i] = { x: tnx, y: tny, vx: 0, vy: 0 };
		} else {
			cur.vx += ((tnx - cur.x) * STIFF - cur.vx * DAMP) * dt;
			cur.vy += ((tny - cur.y) * STIFF - cur.vy * DAMP) * dt;
			cur.x = Math.min(1.05, Math.max(-0.05, cur.x + cur.vx * dt));
			cur.y = Math.min(1.05, Math.max(-0.05, cur.y + cur.vy * dt));
		}
		const px = cur.x * w;
		const py = cur.y * h;
		// (expanded: full roam over the whole island, no clamping)
		const hue = lightHue(i, activeN, now / 1000, vibe, baseHue);
		// talk stays warm; bang runs hot; chill stays soft
		const sizeMul =
			vibe === "talk"
				? 0.16
				: vibe === "chill"
					? 0.3
					: vibe === "bang"
						? 0.2
						: vibe === "calm"
							? 0.16
							: 0.17;
		const r =
			size * sizeMul * (0.35 + 2.1 * env) * crowd * rMul;
		let a =
			(0.03 + 0.32 * env) *
			audio.level *
			gain *
			musicish *
			1.4 *
			crowd *
			crowd;
		// (brightness is band-driven only -- the genre picks the dance and
		// the hues, never the intensity)
		if (a < 0.004) continue;
		const g = c.createRadialGradient(
			px,
			py,
			0,
			px,
			py,
			Math.max(1, r),
		);
		g.addColorStop(
			0,
			`hsla(${hue}, 90%, 62%, ${Math.min(0.5, a)})`,
		);
		g.addColorStop(1, `hsla(${hue}, 90%, 62%, 0)`);
		c.fillStyle = g;
		c.fillRect(0, 0, w, h);
	}

	const [ecx, ecy] = vizEyeCenter();
	const cx = ecx * S;
	const cy = ecy * S;

	// voice: a warm halo breathing with the syllables (halo only -- syllables
	// never make rings; rings are bass-only)
	if (audio.talk > 0.03) {
		const env = audio.talkEnv;
		const r = size * (0.16 + 0.3 * env) * rMul;
		const g = c.createRadialGradient(cx, cy, 0, cx, cy, r);
		g.addColorStop(
			0,
			`hsla(38, 95%, 70%, ${Math.min(0.5, 0.3 * audio.talk * gain * (0.4 + env))})`,
		);
		g.addColorStop(1, "hsla(38, 95%, 70%, 0)");
		c.fillStyle = g;
		c.fillRect(0, 0, w, h);
	}

	// the light that goes outside the island is the lights only, taken before the rings are drawn:
	// the rings stay inside the island
	const bleeding = bleedLevel > 0 && cursorPad > 0;
	document.body.classList.toggle("bleeding", bleeding);
	if (bleeding) {
		bleedCanvas.style.opacity = String(
			Math.min(1, bleedLevel * 1.1),
		);
		paintBleed();
	}

	// bass ripples: very thin circular waves expanding from the middle between
	// the eyes. Sound panned to a side shows as a ring that is solid on that
	// side and translucent on the other.
	c.lineWidth = 0.7; // hairline, always -- only the fade changes
	c.lineCap = "round";
	for (let i = viz.rings.length - 1; i >= 0; i--) {
		const rg = viz.rings[i];
		// rAF timestamps can trail performance.now() by a hair: never let age go negative
		const age = Math.max(0, (now - rg.t) / 900);
		if (age >= 1) {
			viz.rings.splice(i, 1);
			continue;
		}
		const ease = 1 - Math.pow(1 - age, 2.2);
		// merged hits raise the target strength; the drawn strength eases toward it
		rg.sv += (rg.s - rg.sv) * Math.min(1, dt * 9);
		const side = Math.max(-1, Math.min(1, rg.pan || 0));
		const dirness = Math.max(
			0,
			Math.min((Math.abs(side) - 0.12) / 0.6, 1),
		); // 0 centered .. 1 hard-panned
		// how far it travels follows how strong the bass hit was: soft = a short
		// ripple, a hard kick = a wave that crosses the whole island
		const reach = 0.12 + 0.62 * rg.sv;
		const radius = Math.max(0, ease * size * reach);
		// always a full circle from the middle; the direction shows as the translucency on the far side
		const ox = cx;
		// near-opaque at birth, fading out slowly so the hairline stays easy to see
		const alpha =
			Math.min(
				1,
				(0.6 + 0.4 * rg.sv) * Math.pow(1 - age, 0.7),
			) * (currentView === "hub" ? 0.85 : 1);
		if (dirness > 0.02 && radius > 1) {
			// sound from one side: the ring is solid on the side it comes from and turns translucent
			// across to the other, more so the harder it is panned
			const far = alpha * (1 - 0.85 * dirness);
			const g = c.createLinearGradient(
				ox - side * radius,
				cy,
				ox + side * radius,
				cy,
			);
			g.addColorStop(
				0,
				`hsla(${rg.hue}, 95%, 76%, ${far})`,
			);
			g.addColorStop(
				0.5,
				`hsla(${rg.hue}, 95%, 76%, ${(far + alpha) / 2})`,
			);
			g.addColorStop(
				1,
				`hsla(${rg.hue}, 95%, 76%, ${alpha})`,
			);
			c.strokeStyle = g;
		} else {
			c.strokeStyle = `hsla(${rg.hue}, 95%, 76%, ${alpha})`;
		}
		c.beginPath();
		c.arc(ox, cy, radius, 0, Math.PI * 2);
		c.stroke();
	}
	c.globalCompositeOperation = "source-over";
}

let lastFrameT = performance.now();
let showEyes = true; // Settings > Eyes
let eyesDrawn = false;
function frame(now) {
	// one bad frame must never kill the loop (the eyes would freeze/vanish for good)
	try {
		const dt = Math.min(
			Math.max((now - lastFrameT) / 1000, 0),
			0.05,
		);
		lastFrameT = now;
		updateCall(now);
		updateAudio(now, dt);
		paintViz(now, dt);
		if (!showEyes) {
			// no eyes: nothing to draw, and what was drawn is cleared once
			if (eyesDrawn) {
				ctx.clearRect(
					0,
					0,
					canvas.width,
					canvas.height,
				);
				hubEyesCtx.clearRect(
					0,
					0,
					hubEyesCanvas.width,
					hubEyesCanvas.height,
				);
				eyesDrawn = false;
			}
		} else if (currentView === "idle") {
			eyesDrawn = true;
			updateMood(now);
			tickBlink(now);
			paintEyes();
		} else if (currentView === "hub") {
			eyesDrawn = true;
			updateMood(now);
			tickBlink(now);
			paintHubEyes();
		}
	} catch (err) {
		console.error(err);
	}
	requestAnimationFrame(frame);
}
if (!FLOATS) requestAnimationFrame(frame);

listen("view-tick", (event) => {
	currentView = event.payload;
	view.name = currentView;
	for (const [id, node] of pills) node.classList.toggle("hidden", currentView !== id);
	notifEl.classList.toggle(
		"hidden",
		currentView !== "notification",
	);
	briefEl.classList.toggle("hidden", currentView !== "brief");
	hubEl.classList.toggle("hidden", currentView !== "hub");
	pill.dataset.view = currentView;
	// only what happened by itself glows (a notification banner, a session pill): calling the
	// island by hovering the edge, or a media / game / work view that settles in, does not
	document.body.dataset.view = currentView;
	document.body.classList.remove("glow");
	if (
		currentView === "notification" ||
		currentView === "brief"
	) {
		void document.body.offsetWidth;
		document.body.classList.add("glow");
	}
	updateBg();
	const enterTarget = {
		notification: notifEl,
		brief: briefEl,
	}[currentView] || pills.get(currentView);
	if (enterTarget) playEnter(enterTarget);
	if (currentView !== "idle") {
		// the canvas stops repainting once a view takes over -- clear its last
		// frame so stale eye pixels don't linger visible through any gaps
		ctx.clearRect(0, 0, canvas.width, canvas.height);
	}
	if (currentView === "hub") {
		// hub-eyes was `display:none` (0x0) at the one point page-load sizing
		// ran, and toggling a CSS class doesn't fire a `resize` event to catch
		// it later -- size it explicitly now that it's actually visible
		sizeCanvasToElement(hubEyesCanvas);
		mood.happyUntil = performance.now() + 1600; // happy to see you
		// left-click opens the info pane; the tray's Settings item asks for settings
		setHubPane(pendingSettingsPane ? "settings" : "info");
		pendingSettingsPane = false;
		updateClock();
		playHubEntrance(document.getElementById("hub-head"));
		loadSettingsIntoForm();
		loadNotificationHistory();
		for (const p of plugins) p.hubOpened?.();
		loadCalendar();
		refreshActivity();
		pollSysStats();
	}
});

// -- cursor tracking + squash/stretch, driven by Rust's global cursor poll --
// -- the island being "called": a glow builds at the top edge while the cursor dwells --
const callGlow = document.getElementById("call-glow");
let cursorPad = 0;
let callTarget = 0;
let callSmooth = 0;
let callLast = 0;

function updateCall(nowMs) {
	// follows the charge exactly while it builds; the end is very quick (about 0.14 s to gone)
	const dt = Math.min(
		Math.max((nowMs - callLast) / 1000, 0),
		0.05,
	);
	callLast = nowMs;
	callSmooth =
		callTarget > callSmooth
			? callTarget
			: callTarget +
				(callSmooth - callTarget) *
					Math.exp(-dt / 0.025);
	if (callTarget === 0 && callSmooth < 0.004) callSmooth = 0;
	document.body.classList.toggle("calling", callSmooth > 0);
	// the pill stays out of sight while the glow charges; it appears with the drain
	document.body.classList.toggle(
		"call-hide-pill",
		callTarget > 0,
	);
	if (callSmooth > 0)
		callGlow.style.setProperty(
			"--charge",
			callSmooth.toFixed(3),
		);
	// the end fades as well as shrinks
	callGlow.style.setProperty(
		"--fade",
		callTarget < callSmooth ? callSmooth.toFixed(3) : "1",
	);
}

listen("cursor-tick", (event) => {
	// ease-in curve, as picked in the Edge Glow Lab
	const rawCharge = event.payload.charge || 0;
	callTarget = rawCharge * rawCharge;
	// the window is enlarged while charging and while the island lands: keep the pill at its own size
	// and the glow on the screen's top edge
	// pinned and left alone: drawn smaller (CSS scales the island, see body.shrunk)
	const shrunk = !!event.payload.shrunk;
	if (shrunk !== document.body.classList.contains("shrunk")) {
		document.documentElement.style.setProperty(
			"--shrink",
			String((event.payload.shrink || 75) / 100),
		);
		document.body.classList.toggle("shrunk", shrunk);
	}
	const big = !!event.payload.big;
	const pad = event.payload.pad || 0;
	if (pad !== cursorPad) {
		cursorPad = pad;
		document.documentElement.style.setProperty(
			"--pad",
			`${pad}px`,
		);
	}
	if (big !== document.body.classList.contains("big")) {
		document.body.classList.toggle("big", big);
		if (!big) pill.classList.remove("landing");
	}
	if (big) {
		const rs = document.documentElement.style;
		rs.setProperty("--edge", event.payload.edge.toFixed(1));
		rs.setProperty(
			"--pill-top",
			event.payload.pill_top.toFixed(1),
		);
		rs.setProperty("--pill-w", event.payload.pill_w);
		rs.setProperty("--pill-h", event.payload.pill_h);
	}
	const { local_x, local_y } = event.payload;
	const dx = local_x - eyeState.lastCursor.x;
	const dy = local_y - eyeState.lastCursor.y;
	eyeState.lastCursor = { x: local_x, y: local_y };

	const speed = Math.hypot(dx, dy);
	const nowMs = performance.now();
	if (speed > 2) {
		if (nowMs - mood.lastMoveAt > 25000)
			mood.wideUntil = nowMs + 500; // startled awake
		mood.lastMoveAt = nowMs;
	}
	if (islandShown !== !!event.payload.shown) {
		islandShown = !!event.payload.shown;
		pill.classList.toggle("hiding", !islandShown);
		document.body.classList.toggle(
			"pill-hidden",
			!islandShown,
		);
		if (
			islandShown &&
			document.body.classList.contains("big")
		) {
			// the island lands inside the enlarged window: animate it in place
			pill.classList.remove("landing");
			void pill.offsetWidth;
			pill.classList.add("landing");
		}
		updateBg();
	}
	mood.hover =
		!!event.payload.shown &&
		local_x >= cursorPad &&
		local_x <= window.innerWidth - cursorPad &&
		local_y >= cursorPad &&
		local_y <= window.innerHeight - cursorPad;
	const targetSquish = Math.min(speed / 140.0, 1.0);
	if (targetSquish > eyeState.squish) {
		eyeState.squish = targetSquish;
		if (speed > 0.5) {
			eyeState.squishDir = [dx / speed, dy / speed];
		}
	} else {
		eyeState.squish *= 0.78;
	}

	eyeState.smoothedCursor.x +=
		(local_x - eyeState.smoothedCursor.x) * 0.35;
	eyeState.smoothedCursor.y +=
		(local_y - eyeState.smoothedCursor.y) * 0.35;
});

// -- drag to move (native OS move, not a JS-computed one -- cheaper and
// matches exactly how the window manager drags any other window) --
// settings live behind the tray menu's "Settings" item so they aren't always
// on screen (Rust emits "open-settings", then opens the hub if it was closed).
// Left-click opens the hub on the info pane.
let pendingSettingsPane = false;
const paneSettings = document.getElementById("pane-settings");
const paneInfo = document.getElementById("pane-info");
const paneCalendar = document.getElementById("pane-calendar");
const hubBody = document.getElementById("hub-body");

function setHubPane(pane) {
	paneSettings.classList.toggle(
		"hidden",
		pane !== "settings",
	);
	paneInfo.classList.toggle("hidden", pane !== "info");
	paneCalendar.classList.toggle(
		"hidden",
		pane !== "calendar",
	);
	hubBody.scrollTop = 0;
	scheduleHubHeight();
	playHubEntrance(
		pane === "settings"
			? paneSettings
			: pane === "calendar"
				? paneCalendar
				: paneInfo,
	);
}

// -- adaptive hub height: the expanded island is only as tall as what it holds
// (up to Rust's max). Watch the panes and the header; whenever the visible
// pane's content height changes, tell Rust the height it should spring to. --
const hubHeadEl = document.getElementById("hub-head");
let hubHeightSent = 0;
let hubHeightRaf = 0;
function reportHubHeight() {
	hubHeightRaf = 0;
	if (currentView !== "hub") return;
	const pane = [paneSettings, paneInfo, paneCalendar].find(
		(p) => !p.classList.contains("hidden"),
	);
	if (!pane) return;
	// bottom edge of the last visible card. offsetTop/offsetHeight are layout
	// values (getBoundingClientRect would include the entrance animation's
	// transforms, and measure a pane mid-pop-in too tall)
	let bottom = 0;
	const visit = (el) => {
		for (const c of el.children) {
			if (getComputedStyle(c).display === "contents")
				visit(c);
			else if (c.offsetHeight > 0)
				bottom = Math.max(
					bottom,
					c.offsetTop + c.offsetHeight,
				);
		}
	};
	visit(pane);
	const pb = parseFloat(
		getComputedStyle(hubBody).paddingBottom,
	);
	const h = Math.ceil(hubHeadEl.offsetHeight + bottom + pb);
	if (Math.abs(h - hubHeightSent) < 2) return;
	hubHeightSent = h;
	invoke("set_hub_height", { h });
}
function scheduleHubHeight() {
	if (!hubHeightRaf)
		hubHeightRaf = requestAnimationFrame(reportHubHeight);
}
const hubHeightObserver = new ResizeObserver(scheduleHubHeight);
for (const el of [
	paneSettings,
	paneInfo,
	paneCalendar,
	hubHeadEl,
])
	hubHeightObserver.observe(el);

// stagger the pane's children springing up (CSS `rise` keyframes, --i = order)
function playHubEntrance(container) {
	container.classList.remove("enter");
	const items = [];
	for (const c of container.children) {
		if (c.classList.contains("flat"))
			items.push(...c.children);
		else items.push(c);
	}
	items.forEach((c, i) => c.style.setProperty("--i", i));
	void container.offsetWidth; // restart the animation
	container.classList.add("enter");
	// once played, clear it: cards inserted later (data arriving, list rebuilds) shouldn't re-pop
	clearTimeout(container._enterTimer);
	container._enterTimer = setTimeout(
		() => container.classList.remove("enter"),
		1100,
	);
}

// header clock (DOM, not canvas -- crisper and styleable)
const hubTimeEl = document.getElementById("hub-time");
const hubDateEl = document.getElementById("hub-date");
function updateClock() {
	const now = new Date();
	// military time (Settings > Time) shows it as 15:30
	const t = now.toLocaleTimeString(
		[],
		currentSettings?.time_24h
			? {
					hour: "2-digit",
					minute: "2-digit",
					hourCycle: "h23",
				}
			: { hour: "numeric", minute: "2-digit" },
	);
	if (hubTimeEl.textContent !== t) hubTimeEl.textContent = t;
	hubDateEl.textContent = now.toLocaleDateString([], {
		weekday: "long",
		month: "short",
		day: "numeric",
	});
}
setInterval(() => {
	if (currentView === "hub") updateClock();
}, 1000);

// cursor spotlight: every .spot card gets the cursor position (relative to
// itself) as --mx/--my, which its CSS radial-gradient follows like a flashlight
const hubRoot = document.getElementById("hub");
let spotRaf = 0;
let spotEvent = null;
function updateSpots() {
	spotRaf = 0;
	if (!spotEvent) return;
	for (const el of hubRoot.querySelectorAll(".spot")) {
		const r = el.getBoundingClientRect();
		el.style.setProperty(
			"--mx",
			`${spotEvent.clientX - r.left}px`,
		);
		el.style.setProperty(
			"--my",
			`${spotEvent.clientY - r.top}px`,
		);
	}
}
hubRoot.addEventListener("mousemove", (e) => {
	spotEvent = e;
	hubRoot.classList.add("spot-on");
	if (!spotRaf) spotRaf = requestAnimationFrame(updateSpots);
});
hubRoot.addEventListener("mouseleave", () => {
	// keep --mx/--my where they were: the light fades out in place (--spot-a)
	spotEvent = null;
	hubRoot.classList.remove("spot-on");
});

listen("open-settings", () => {
	if (currentView === "hub") setHubPane("settings");
	else pendingSettingsPane = true;
});

// no native webview context menu on the island
// right-click pins the island in place (no hiding, no following the cursor to
// other monitors, hub stays open on click-outside) until right-clicked again.
// Feedback is a material-style ink ripple that floods the island from the click.
function inkRipple(x, y) {
	const r = pill.getBoundingClientRect();
	// radius that reaches the farthest corner from the click
	// (x1.3 so the soft edge of the gradient is still past the corner)
	const reach =
		1.3 *
		Math.hypot(
			Math.max(x, r.width - x),
			Math.max(y, r.height - y),
		);
	const dot = document.createElement("div");
	dot.className = "ink";
	dot.style.left = `${x - reach}px`;
	dot.style.top = `${y - reach}px`;
	dot.style.width = dot.style.height = `${reach * 2}px`;
	pill.append(dot);
	dot.addEventListener("animationend", (e) => {
		if (e.animationName === "ink-fade") dot.remove(); // the last of its two animations
	});
}

pill.addEventListener("contextmenu", (e) => {
	e.preventDefault();
	const r = pill.getBoundingClientRect();
	inkRipple(e.clientX - r.left, e.clientY - r.top);
	invoke("toggle_pin");
});
const lockBadge = document.getElementById("lock-badge");
listen("pin-tick", (event) => {
	mood.happyUntil = performance.now() + 900; // a little happy blip either way
	// a lock (pinned) or an open lock (unpinned) pops up at the centre of the island
	const pinned = !!event.payload;
	lockBadge.innerHTML = li(pinned ? "lock" : "unlock");
	lockBadge.classList.toggle("unlocked", !pinned);
	lockBadge.classList.remove("show");
	void lockBadge.offsetWidth;
	lockBadge.classList.add("show");
});

pill.addEventListener("mousedown", (e) => {
	if (e.button !== 0) return;
	// the open hub is a panel to work in: only its head (the eyes) takes the click that closes it
	if (currentView === "hub" && !e.target.closest("#hub-head")) return;
	mood.wideUntil = performance.now() + 320; // boop!
	invoke("drag_start");
});
// No mouseup/drag_end handler: `start_dragging()`'s native OS move-loop can
// swallow the DOM mouseup, so end-of-drag is detected Rust-side instead by
// polling the mouse button's real state -- see spawn_edge_poll in lib.rs.
// Hover (for pausing the idle-hide timer) is computed on the Rust side from
// the global cursor vs. the window's own rect -- see IslandState's doc
// comment in lib.rs for why webview mouseenter/leave aren't used for that.

// -- notification banner: driven by notification-tick, and (for now) a
// pushNotification() global any future subsystem can call to show one --
const notifEl = document.getElementById("notification");
const notifTitle = document.getElementById("notif-title");
const notifBody = document.getElementById("notif-body");
const notifTimer = document.getElementById("notif-timer");
document.getElementById("notif-icon").innerHTML = ICON.bell;

let notifAction = null; // what clicking the current banner does (e.g. "viber")
let notifId = 0; // its entry in the scrollback
listen("notification-tick", (event) => {
	const n = event.payload;
	if (!n) return; // dismissed -- view-tick already hid the banner
	if (n.brief) {
		showBrief(n);
		return;
	}
	notifAction = n.action || null;
	notifId = n.id;
	notifTitle.textContent = n.title || "";
	notifBody.textContent = n.body || "";
	// restart the draining timer bar for this banner
	notifTimer.classList.remove("run");
	void notifTimer.offsetWidth;
	notifTimer.classList.add("run");
});

// clicking a banner dismisses it (and its card in the hub), running its action if it has one,
// instead of toggling the hub (its mousedown must not reach #pill's drag handler)
notifEl.addEventListener("mousedown", (e) =>
	e.stopPropagation(),
);
notifEl.addEventListener("click", () =>
	invoke("click_notification", {
		id: notifId,
		action: notifAction,
	}),
);

// -- a Claude Code session that needs you or finished: a compact pill from the same queue --
const briefEl = document.getElementById("brief");
const briefTile = document.getElementById("brief-tile");
const briefTitle = document.getElementById("brief-title");
const briefRight = document.getElementById("brief-right");
let briefCurrent = null;
function showBrief(n) {
	const b = n.brief;
	briefCurrent = b;
	// a plugin's pill: its icon, what it says, and a short text at the right; "big" is only light digits, glowing
	const plugin = b.state === "plugin";
	const big = plugin && b.look === "big";
	briefEl.classList.toggle("brief-time", big);
	briefEl.classList.toggle("brief-plugin", plugin);
	if (plugin) {
		briefTile.removeAttribute("style");
		briefTile.className = "pg-tile";
		briefTile.innerHTML = li(b.host_icon || "dot");
		const clock = big ? /^(\d+):(\d+)(?:\s*(AM|PM))?$/i.exec(n.title) : null;
		if (clock) {
			// the time: big light digits glowing in the theme colour, a blinking colon, nothing else
			briefTitle.replaceChildren(el("span", "", clock[1]), el("span", "time-sep", ":"), el("span", "", clock[2]));
			if (clock[3]) briefTitle.append(el("small", "time-suf", clock[3].toUpperCase()));
		} else {
			briefTitle.textContent = whenText(n.title);
		}
		briefRight.replaceChildren(el("span", "pg-right", whenText(b.project || "")));
		briefRight.classList.toggle("hidden", !b.project);
	} else {
		briefTile.className = "llm-tile";
		briefRight.classList.remove("hidden");
		briefTitle.textContent = n.title;
		briefTile.style.setProperty("--c", LLM_COLOR.claude);
		briefTile.innerHTML = `${LLM_LETTER.claude}<svg class="cr" viewBox="0 0 40 40"><circle class="rb" cx="20" cy="20" r="18"/><circle class="rf" cx="20" cy="20" r="18"/></svg>`;
		const rf = briefTile.querySelector(".rf");
		rf.style.strokeDasharray = String(LLM_RING_C);
		rf.style.strokeDashoffset = String(LLM_RING_C * (1 - b.ctx));
		rf.style.stroke = llmCtxColor(b.ctx);
		const host = el("span", "llm-host");
		host.innerHTML = li(b.host_icon);
		briefRight.replaceChildren(host, llmStateIcon(b.state));
	}
	document.body.dataset.brief = b.state;
	// already a brief on screen (the queue moved on): the glow and the entrance start again
	if (currentView === "brief") {
		document.body.classList.remove("glow");
		void document.body.offsetWidth;
		document.body.classList.add("glow");
		playEnter(briefEl);
	}
}
briefEl.addEventListener("mousedown", (e) =>
	e.stopPropagation(),
);
briefEl.addEventListener("click", () => {
	if (!briefCurrent) return;
	if (briefCurrent.state === "plugin") {
		invoke("open_notification_action", { action: "brief" }); // just closes it
		return;
	}
	invoke("focus_source", {
		exePath: briefCurrent.host_exe,
		titleHint: briefCurrent.project,
	});
	if (briefCurrent.state === "finished")
		call("claude-code", "dismiss", { id: briefCurrent.id }).catch(() => {});
	invoke("open_notification_action", { action: "brief" }); // ends the pill
});

window.pushNotification = (title, body) =>
	invoke("push_notification", { title, body });

// -- notification history: the hub's scrollback of past banners --
const notifHistoryEl = document.getElementById("notif-history");

function renderNotificationHistory(entries) {
	notifHistoryEl.innerHTML = "";
	if (!entries || entries.length === 0) {
		updateInfoEmpty();
		return;
	}
	for (const entry of entries) {
		const item = document.createElement("div");
		item.className = "notif-history-item spot";
		// dismiss: the card slides off and folds away, then Rust forgets it
		const dismissItem = () => {
			item.style.height = `${item.offsetHeight}px`;
			void item.offsetWidth; // commit the explicit height so it can transition to 0
			item.classList.add("leaving");
			setTimeout(
				() =>
					invoke("dismiss_notification", {
						id: entry.id,
					}),
				280,
			);
		};
		// clicking a card clears it (the x does the same without going anywhere); one with an
		// action also jumps to its source app
		item.classList.add("clickable");
		item.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
		item.addEventListener("click", () => {
			if (entry.action)
				invoke("open_notification_action", {
					action: entry.action,
				});
			dismissItem();
		});

		const dismiss = document.createElement("button");
		dismiss.className = "notif-x";
		dismiss.setAttribute("aria-label", "Dismiss");
		dismiss.innerHTML =
			'<svg viewBox="0 0 12 12"><path d="M3 3l6 6M9 3l-6 6" /></svg>';
		dismiss.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
		dismiss.addEventListener("click", (e) => {
			e.stopPropagation(); // dismiss only, don't also open the source
			dismissItem();
		});
		item.appendChild(dismiss);

		const title = document.createElement("div");
		title.className = "notif-history-title";
		title.textContent = entry.title || "";
		item.appendChild(title);

		if (entry.body) {
			const body = document.createElement("div");
			body.className = "notif-history-body";
			body.textContent = entry.body;
			item.appendChild(body);
		}

		const time = document.createElement("div");
		time.className = "notif-history-time";
		time.textContent = new Date(
			entry.time_ms,
		).toLocaleString([], {
			month: "short",
			day: "numeric",
			hour: "2-digit",
			minute: "2-digit",
		});
		item.appendChild(time);

		notifHistoryEl.appendChild(item);
	}
	updateInfoEmpty();
}

async function loadNotificationHistory() {
	renderNotificationHistory(
		await invoke("get_notification_history"),
	);
}

listen("notification-history-tick", (event) => {
	// only bother re-rendering if the hub is actually open to see it
	if (currentView === "hub") {
		renderNotificationHistory(event.payload);
	}
});

// -- hub: the pill expanded into a panel, toggled by clicking the pill
// (see drag_end in lib.rs -- a click is just a drag that went nowhere).
// Its eyes + clock are canvas-painted every frame in paintHubEyes() above,
// same as the idle pill's, so they stay live (blink/gaze-track) while open.
const hubEl = document.getElementById("hub");

// -- icons on the static markup: clear buttons, and the plugin list's tools --
(function decorate() {
	for (const b of document.querySelectorAll(".hub-btn-x"))
		b.innerHTML = li("x");
	document.getElementById("plugin-folder").innerHTML = li("folder");
	document.getElementById("plugin-rescan").innerHTML = li("refresh");
})();

// -- hub settings: loaded fresh each time the hub opens, saved back to
// Rust (which persists to disk and live-updates the detection threads)
// on every change --
const setStartWithWindows = document.getElementById(
	"set-start-with-windows",
);
const setIdleHideDelay = document.getElementById(
	"set-idle-hide-delay",
);
const setIdleHideDelayLabel = document.getElementById(
	"set-idle-hide-delay-label",
);

const setShowAtCursor = document.getElementById(
	"set-show-at-cursor",
);
const setReactToAudio = document.getElementById(
	"set-react-to-audio",
);
const setShowEyes = document.getElementById("set-show-eyes");
// the theme colour: a handful of swatches; the accent of the hub, the glow and the pills follows
const ACCENTS = [
	"#5ac88c",
	"#4cc9c0",
	"#5aa9f0",
	"#a487ee",
	"#ee7fb5",
	"#e8776f",
	"#ee9a4d",
	"#e3c457",
	"#e8e8ee",
];
let accentColor = ACCENTS[0];
function applyAccent(hex) {
	if (!/^#[0-9a-f]{6}$/i.test(hex || "")) hex = ACCENTS[0];
	accentColor = hex.toLowerCase();
	const n = parseInt(accentColor.slice(1), 16);
	const [r, g, b] = [
		(n >> 16) & 255,
		(n >> 8) & 255,
		n & 255,
	];
	const rs = document.documentElement.style;
	rs.setProperty("--accent", accentColor);
	rs.setProperty("--accent-rgb", `${r}, ${g}, ${b}`);
	// a lighter tint of it, for the edge glow
	rs.setProperty(
		"--accent-light-rgb",
		`${Math.round(r + (255 - r) * 0.28)}, ${Math.round(g + (255 - g) * 0.28)}, ${Math.round(b + (255 - b) * 0.28)}`,
	);
	// the darker end of the accent's gradients
	rs.setProperty(
		"--accent-deep",
		`rgb(${Math.round(r * 0.62)}, ${Math.round(g * 0.62)}, ${Math.round(b * 0.62)})`,
	);
	for (const sw of document.querySelectorAll(
		"#set-accent .swatch",
	))
		sw.classList.toggle(
			"on",
			sw.dataset.color === accentColor,
		);
}
for (const hex of ACCENTS) {
	const sw = document.createElement("button");
	sw.type = "button";
	sw.className = "swatch";
	sw.dataset.color = hex;
	sw.style.background = hex;
	sw.setAttribute("aria-label", hex);
	sw.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
	sw.addEventListener("click", () => {
		applyAccent(hex);
		saveSettingsFromForm();
	});
	document.getElementById("set-accent").append(sw);
}
const setCursorFollow = document.getElementById(
	"set-cursor-follow",
);
const setPeekDuration = document.getElementById(
	"set-peek-duration",
);
const setPeekDurationLabel = document.getElementById(
	"set-peek-duration-label",
);
const setCompactWidth = document.getElementById(
	"set-compact-width",
);
const setCompactWidthLabel = document.getElementById(
	"set-compact-width-label",
);
const setHubWidth = document.getElementById("set-hub-width");
const setHubWidthLabel = document.getElementById(
	"set-hub-width-label",
);
const setTopMargin = document.getElementById("set-top-margin");
const setTopMarginLabel = document.getElementById(
	"set-top-margin-label",
);
const setPinShrink = document.getElementById("set-pin-shrink");
const setPinShrinkLabel = document.getElementById(
	"set-pin-shrink-label",
);
const pinShrinkText = (v) =>
	Number(v) >= 100 ? "off" : `${v}%`;
const setEdgeDwell = document.getElementById("set-edge-dwell");
const setEdgeDwellLabel = document.getElementById(
	"set-edge-dwell-label",
);
// the Fullscreen Guard: No, or on with Alt (or Ctrl) held to use the island anyway
const setFullscreenGuard = document.getElementById(
	"set-fullscreen-guard",
);
const guardValue = () =>
	setFullscreenGuard.querySelector('[aria-pressed="true"]')
		?.dataset.v || "alt";
const showGuard = (v) => {
	for (const b of setFullscreenGuard.querySelectorAll("button"))
		b.setAttribute("aria-pressed", String(b.dataset.v === v));
};
for (const b of setFullscreenGuard.querySelectorAll("button"))
	b.addEventListener("click", () => {
		showGuard(b.dataset.v);
		saveSettingsFromForm();
	});
const edgeDwellText = (ms) =>
	Number(ms) === 0 ? "off" : `${ms} ms`;
const setAudioBleed = document.getElementById(
	"set-audio-bleed",
);
const setAudioBleedLabel = document.getElementById(
	"set-audio-bleed-label",
);
const setGlow = document.getElementById("set-glow");
const setGlowLabel = document.getElementById("set-glow-label");
const setBgDim = document.getElementById("set-bg-dim");
const setBgDimLabel = document.getElementById(
	"set-bg-dim-label",
);

let currentSettings = null;

async function loadSettingsIntoForm() {
	currentSettings = await invoke("get_settings");
	setStartWithWindows.checked =
		currentSettings.start_with_windows;
	setIdleHideDelay.value = currentSettings.idle_hide_delay_s;
	setIdleHideDelayLabel.textContent = `${currentSettings.idle_hide_delay_s}s`;
	setShowAtCursor.checked = currentSettings.show_at_cursor;
	setCursorFollow.checked = currentSettings.cursor_follow;
	setReactToAudio.checked = currentSettings.react_to_audio;
	setShowEyes.checked = currentSettings.show_eyes ?? true;
	applyAccent(currentSettings.accent_color);
	showEyes = setShowEyes.checked;
	setCompactWidth.value =
		currentSettings.compact_width ?? 260;
	setCompactWidthLabel.textContent = `${setCompactWidth.value}px`;
	setHubWidth.value = currentSettings.hub_width ?? 420;
	setHubWidthLabel.textContent = `${setHubWidth.value}px`;
	setTopMargin.value = currentSettings.top_margin ?? 4;
	setTopMarginLabel.textContent = `${setTopMargin.value}px`;
	setPinShrink.value = currentSettings.pin_shrink ?? 75;
	setPinShrinkLabel.textContent = pinShrinkText(
		setPinShrink.value,
	);
	setEdgeDwell.value = currentSettings.edge_dwell_ms;
	setEdgeDwellLabel.textContent = edgeDwellText(
		currentSettings.edge_dwell_ms,
	);
	showGuard(
		currentSettings.fullscreen_guard === false
			? "off"
			: currentSettings.guard_key === "ctrl"
				? "ctrl"
				: "alt",
	);
	setPeekDuration.value = currentSettings.peek_duration_s;
	setPeekDurationLabel.textContent = `${currentSettings.peek_duration_s}s`;
	setAudioBleed.value = currentSettings.audio_bleed ?? 60;
	setAudioBleedLabel.textContent = `${setAudioBleed.value}%`;
	bleedLevel = Number(setAudioBleed.value) / 100;
	setGlow.value = currentSettings.glow_intensity ?? 70;
	setGlowLabel.textContent = `${setGlow.value}%`;
	applyGlow(Number(setGlow.value));
	setBgDim.value = currentSettings.bg_dim;
	setBgDimLabel.textContent = `${currentSettings.bg_dim}%`;
	applyBackgrounds(currentSettings);
	syncSliderResets();
	updateSizePreview();
}

// every slider gets a small reset button that shows only while it is off its default
function syncSliderResets() {
	for (const input of document.querySelectorAll(
		'input[type="range"][data-default]',
	)) {
		const btn = input.previousElementSibling;
		if (btn?.classList.contains("slider-reset"))
			btn.classList.toggle(
				"is-default",
				input.value === input.dataset.default,
			);
		// the filled part of the track
		const lo = Number(input.min);
		const span = Number(input.max) - lo;
		input.style.setProperty(
			"--p",
			`${(((Number(input.value) - lo) / span) * 100).toFixed(1)}%`,
		);
	}
}
for (const input of document.querySelectorAll(
	'input[type="range"][data-default]',
)) {
	const btn = document.createElement("button");
	btn.type = "button";
	btn.className = "slider-reset is-default";
	btn.innerHTML = li("reset");
	btn.dataset.tip = "Reset to default";
	btn.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
	btn.addEventListener("click", (e) => {
		e.preventDefault();
		e.stopPropagation();
		input.value = input.dataset.default;
		input.dispatchEvent(
			new Event("input", { bubbles: true }),
		);
		input.dispatchEvent(
			new Event("change", { bubbles: true }),
		);
	});
	input.addEventListener("input", syncSliderResets);
	input.before(btn);
}

function saveSettingsFromForm() {
	if (!currentSettings) return;
	currentSettings = {
		...currentSettings,
		start_with_windows: setStartWithWindows.checked,
		idle_hide_delay_s: Number(setIdleHideDelay.value),
		show_at_cursor: setShowAtCursor.checked,
		cursor_follow: setCursorFollow.checked,
		react_to_audio: setReactToAudio.checked,
		show_eyes: setShowEyes.checked,
		peek_duration_s: Number(setPeekDuration.value),
		edge_dwell_ms: Number(setEdgeDwell.value),
		fullscreen_guard: guardValue() !== "off",
		guard_key:
			guardValue() === "off"
				? currentSettings.guard_key || "alt"
				: guardValue(),
		pin_shrink: Number(setPinShrink.value),
		compact_width: Number(setCompactWidth.value),
		hub_width: Number(setHubWidth.value),
		top_margin: Number(setTopMargin.value),
		bg_dim: Number(setBgDim.value),
		glow_intensity: Number(setGlow.value),
		audio_bleed: Number(setAudioBleed.value),
	};
	showEyes = setShowEyes.checked;
	currentSettings.accent_color = accentColor;
	updateClock();
	invoke("save_settings", { settings: currentSettings });
}

for (const el of [
	setStartWithWindows,
	setShowAtCursor,
	setCursorFollow,
	setReactToAudio,
	setShowEyes,
]) {
	el.addEventListener("change", saveSettingsFromForm);
}
setIdleHideDelay.addEventListener("input", () => {
	setIdleHideDelayLabel.textContent = `${setIdleHideDelay.value}s`;
});
setIdleHideDelay.addEventListener(
	"change",
	saveSettingsFromForm,
);
setCompactWidth.addEventListener("input", () => {
	setCompactWidthLabel.textContent = `${setCompactWidth.value}px`;
});
setCompactWidth.addEventListener(
	"change",
	saveSettingsFromForm,
);
setHubWidth.addEventListener("input", () => {
	setHubWidthLabel.textContent = `${setHubWidth.value}px`;
});
setHubWidth.addEventListener("change", saveSettingsFromForm);
setTopMargin.addEventListener("input", () => {
	setTopMarginLabel.textContent = `${setTopMargin.value}px`;
});
setTopMargin.addEventListener("change", saveSettingsFromForm);
// ---- the settings tabs: four, each with its name ----
const SET_TABS = [
	{ id: "general", icon: "sliders", tip: "General" },
	{ id: "size", icon: "expand", tip: "Size" },
	{ id: "look", icon: "palette", tip: "Look" },
	{ id: "plugins", icon: "plug", tip: "Plugins" },
];
function showSettingsTab(id) {
	if (id === "plugins") renderPlugins();
	if (id === "size") updateSizePreview();
	for (const p of document.querySelectorAll(
		"#pane-settings .set-tab",
	))
		p.classList.toggle("hidden", p.dataset.tab !== id);
	for (const b of document.querySelectorAll(
		"#set-tabs button",
	)) {
		b.classList.toggle("on", b.dataset.tab === id);
		b.setAttribute(
			"aria-selected",
			String(b.dataset.tab === id),
		);
	}
	try {
		localStorage.setItem("nadi.settingsTab", id);
	} catch (_) {}
	scheduleHubHeight();
}
{
	const bar = document.getElementById("set-tabs");
	for (const t of SET_TABS) {
		const b = document.createElement("button");
		b.type = "button";
		b.dataset.tab = t.id;
		b.setAttribute("role", "tab");
		b.innerHTML = `${li(t.icon)}<span>${t.tip}</span>`;
		b.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
		b.addEventListener("click", () =>
			showSettingsTab(t.id),
		);
		bar.append(b);
	}
	let first = "general";
	try {
		const saved = localStorage.getItem("nadi.settingsTab");
		if (SET_TABS.some((t) => t.id === saved)) first = saved;
	} catch (_) {}
	showSettingsTab(first);
}
setPinShrink.addEventListener("input", () => {
	setPinShrinkLabel.textContent = pinShrinkText(
		setPinShrink.value,
	);
});
setPinShrink.addEventListener("change", saveSettingsFromForm);
setEdgeDwell.addEventListener("input", () => {
	setEdgeDwellLabel.textContent = edgeDwellText(
		setEdgeDwell.value,
	);
});
setEdgeDwell.addEventListener("change", saveSettingsFromForm);
setPeekDuration.addEventListener("input", () => {
	setPeekDurationLabel.textContent = `${setPeekDuration.value}s`;
});
setPeekDuration.addEventListener(
	"change",
	saveSettingsFromForm,
);
function applyGlow(pct) {
	document.documentElement.style.setProperty(
		"--gi",
		(pct / 100).toFixed(2),
	);
}
setAudioBleed.addEventListener("input", () => {
	setAudioBleedLabel.textContent = `${setAudioBleed.value}%`;
	bleedLevel = Number(setAudioBleed.value) / 100; // live: the next frame uses it
});
setAudioBleed.addEventListener("change", saveSettingsFromForm);
let glowPreviewTimer = 0;
setGlow.addEventListener("input", () => {
	setGlowLabel.textContent = `${setGlow.value}%`;
	applyGlow(Number(setGlow.value)); // live preview while dragging
	document.body.classList.add("glow-preview");
	clearTimeout(glowPreviewTimer);
	glowPreviewTimer = setTimeout(
		() => document.body.classList.remove("glow-preview"),
		900,
	);
});
setGlow.addEventListener("change", saveSettingsFromForm);
setBgDim.addEventListener("input", () => {
	setBgDimLabel.textContent = `${setBgDim.value}%`;
	applyBgDim(Number(setBgDim.value)); // live preview while dragging
});
setBgDim.addEventListener("change", saveSettingsFromForm);

// the Size tab's preview: the collapsed island and the expanded one behind it, at half their size
function updateSizePreview() {
	const k = 0.5;
	const top = 6 + Number(setTopMargin.value) * k * 0.5;
	const hub = document.getElementById("sp-hub");
	const pill = document.getElementById("sp-pill");
	hub.style.width = `${Number(setHubWidth.value) * k}px`;
	hub.style.top = pill.style.top = `${top}px`;
	pill.style.width = `${Number(setCompactWidth.value) * k}px`;
}
paneSettings.addEventListener("input", updateSizePreview);

// -- the calendar (a main feature): the events come from whichever plugin brought them, pushed by Rust --
function applyCalendar(c) {
	calSetData(c);
}

// -- hub calendar view: click the clock to swap the cards for a month grid.
// Pick a day to list that day's events from the .ics feed. Collapsing the
// island drops back to the cards (the hub always reopens on the info pane). --
const calTitleEl = document.getElementById("cal-title");
const calWeekEl = document.getElementById("cal-week");
const calGridEl = document.getElementById("cal-grid");
const calDayTitleEl = document.getElementById("cal-day-title");
const calStatusEl = document.getElementById("cal-status");
const calEventsEl = document.getElementById("cal-events");
const cal = {
	data: null,
	byDay: new Map(),
	month: null,
	sel: null,
};

const dayKey = (d) =>
	d.getFullYear() * 10000 +
	(d.getMonth() + 1) * 100 +
	d.getDate();
const startOfDay = (d) =>
	new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d, n) =>
	new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

// first weekday of the user's locale: 0 = Sunday .. 6 = Saturday
function calFirstDow() {
	try {
		const loc = new Intl.Locale(navigator.language);
		const info = loc.getWeekInfo
			? loc.getWeekInfo()
			: loc.weekInfo;
		if (info && info.firstDay) return info.firstDay % 7;
	} catch (_) {}
	return 0;
}
const CAL_FIRST_DOW = calFirstDow();

function calIndex() {
	cal.byDay = new Map();
	if (!cal.data || !cal.data.events) return;
	for (const ev of cal.data.events) {
		const first = startOfDay(new Date(ev.start_ms));
		// end is exclusive: a 00:00-00:00 all-day event covers exactly one day
		const lastMs =
			ev.end_ms > ev.start_ms
				? ev.end_ms - 1
				: ev.start_ms;
		const last = startOfDay(new Date(lastMs));
		let d = first;
		for (
			let n = 0;
			d <= last && n < 62;
			n++, d = addDays(d, 1)
		) {
			const k = dayKey(d);
			if (!cal.byDay.has(k)) cal.byDay.set(k, []);
			cal.byDay.get(k).push(ev);
		}
	}
}

function calSetData(c) {
	cal.data = c;
	calIndex();
	if (!paneCalendar.classList.contains("hidden")) {
		buildCalGrid(null);
		renderCalDay(false);
	}
}

function buildCalWeekHeader() {
	calWeekEl.textContent = "";
	const base = new Date(2023, 0, 1); // a Sunday
	for (let i = 0; i < 7; i++) {
		const s = document.createElement("span");
		s.textContent = addDays(
			base,
			(CAL_FIRST_DOW + i) % 7,
		).toLocaleDateString([], { weekday: "narrow" });
		calWeekEl.append(s);
	}
}
buildCalWeekHeader();

function buildCalGrid(dir) {
	const first = new Date(
		cal.month.getFullYear(),
		cal.month.getMonth(),
		1,
	);
	calTitleEl.textContent = first.toLocaleDateString([], {
		month: "long",
		year: "numeric",
	});
	const offset = (first.getDay() - CAL_FIRST_DOW + 7) % 7;
	const gridStart = addDays(first, -offset);
	const todayKey = dayKey(new Date());
	const selKey = dayKey(cal.sel);
	calGridEl.textContent = "";
	for (let i = 0; i < 42; i++) {
		const d = addDays(gridStart, i);
		const k = dayKey(d);
		const btn = document.createElement("button");
		btn.className = "cal-day";
		if (d.getMonth() !== first.getMonth())
			btn.classList.add("other");
		if (k === todayKey) btn.classList.add("today");
		if (k === selKey) btn.classList.add("sel");
		btn.dataset.key = k;
		const num = document.createElement("span");
		num.textContent = d.getDate();
		const dots = document.createElement("span");
		dots.className = "cal-dots";
		const n = Math.min((cal.byDay.get(k) || []).length, 3);
		for (let j = 0; j < n; j++)
			dots.append(document.createElement("i"));
		btn.append(num, dots);
		btn.addEventListener("click", () => selectCalDay(d));
		calGridEl.append(btn);
	}
	if (dir) {
		calGridEl.classList.remove("slide-next", "slide-prev");
		void calGridEl.offsetWidth; // restart the slide
		calGridEl.classList.add(
			dir > 0 ? "slide-next" : "slide-prev",
		);
	}
}

function calTimeLabel(ev, day) {
	if (ev.all_day) return "All day";
	const fmt = (ms) =>
		new Date(ms).toLocaleTimeString([], {
			hour: "numeric",
			minute: "2-digit",
		});
	const dayStart = day.getTime();
	const dayEnd = addDays(day, 1).getTime();
	const startsToday = ev.start_ms >= dayStart;
	const endsToday = ev.end_ms <= dayEnd;
	if (ev.end_ms <= ev.start_ms) return fmt(ev.start_ms);
	if (startsToday && endsToday)
		return `${fmt(ev.start_ms)} – ${fmt(ev.end_ms)}`;
	if (startsToday) return `${fmt(ev.start_ms)} →`;
	return endsToday ? `→ ${fmt(ev.end_ms)}` : "All day";
}

function renderCalDay(animate) {
	const day = startOfDay(cal.sel);
	const list = (cal.byDay.get(dayKey(day)) || [])
		.slice()
		.sort((a, b) => {
			return (
				(b.all_day ? 1 : 0) - (a.all_day ? 1 : 0) ||
				a.start_ms - b.start_ms
			);
		});
	const label = day.toLocaleDateString([], {
		weekday: "long",
		month: "short",
		day: "numeric",
	});
	// a day with nothing on it shows nothing: no date heading, no placeholder
	calDayTitleEl.textContent = list.length
		? `${label} · ${list.length} ${list.length === 1 ? "event" : "events"}`
		: "";

	calStatusEl.textContent = "";
	calEventsEl.textContent = "";
	const d = cal.data;
	if (!d || !d.configured) {
		// nobody brings events: say where they come from
		calStatusEl.textContent = "No calendar plugin is on. Switch one on in Settings > Plugins.";
		return;
	}
	if (d.error) {
		calStatusEl.textContent = d.error;
		return;
	}
	if (!list.length) return;
	const now = Date.now();
	list.forEach((ev, i) => {
		const row = document.createElement("div");
		row.className = "cal-event spot";
		if (
			ev.end_ms < now &&
			!(ev.all_day && ev.end_ms >= now)
		)
			row.classList.add("past");
		if (animate) {
			row.classList.add("fresh");
			row.style.setProperty("--i", i);
		}
		const title = document.createElement("div");
		title.className = "cal-event-title";
		title.textContent = ev.summary;
		const time = document.createElement("div");
		time.className = "cal-event-time";
		withIcon(time, "clock", calTimeLabel(ev, day));
		row.append(title, time);
		calEventsEl.append(row);
	});
}

function selectCalDay(d) {
	const prevMonth =
		cal.month.getFullYear() * 12 + cal.month.getMonth();
	cal.sel = startOfDay(d);
	const newMonth = d.getFullYear() * 12 + d.getMonth();
	if (newMonth !== prevMonth) {
		cal.month = new Date(d.getFullYear(), d.getMonth(), 1);
		buildCalGrid(newMonth > prevMonth ? 1 : -1);
	} else {
		// same month: just move the highlight (the disc springs between cells)
		const k = String(dayKey(cal.sel));
		for (const el of calGridEl.children)
			el.classList.toggle("sel", el.dataset.key === k);
	}
	renderCalDay(true);
}

function shiftCalMonth(delta) {
	const prev = cal.month;
	cal.month = new Date(
		prev.getFullYear(),
		prev.getMonth() + delta,
		1,
	);
	// keep the same day-of-month selected where it exists (clamped to month end)
	const lastDay = new Date(
		cal.month.getFullYear(),
		cal.month.getMonth() + 1,
		0,
	).getDate();
	cal.sel = new Date(
		cal.month.getFullYear(),
		cal.month.getMonth(),
		Math.min(cal.sel.getDate(), lastDay),
	);
	buildCalGrid(delta);
	renderCalDay(true);
}

function openCalendar() {
	const today = startOfDay(new Date());
	cal.sel = today;
	cal.month = new Date(
		today.getFullYear(),
		today.getMonth(),
		1,
	);
	buildCalGrid(null);
	renderCalDay(false);
	setHubPane("calendar");
}

document
	.getElementById("hub-clock")
	.addEventListener("click", () => {
		mood.happyUntil = performance.now() + 700;
		if (paneCalendar.classList.contains("hidden"))
			openCalendar();
		else setHubPane("info");
	});
document
	.getElementById("cal-prev")
	.addEventListener("click", () => shiftCalMonth(-1));
document
	.getElementById("cal-next")
	.addEventListener("click", () => shiftCalMonth(1));
calTitleEl.addEventListener("click", () => {
	const today = startOfDay(new Date());
	if (
		dayKey(today) === dayKey(cal.sel) &&
		cal.month.getMonth() === today.getMonth()
	)
		return;
	const dir =
		today.getFullYear() * 12 + today.getMonth() >
		cal.month.getFullYear() * 12 + cal.month.getMonth()
			? 1
			: -1;
	cal.sel = today;
	cal.month = new Date(
		today.getFullYear(),
		today.getMonth(),
		1,
	);
	buildCalGrid(dir);
	renderCalDay(true);
});
let calWheelAt = 0;
paneCalendar.addEventListener(
	"wheel",
	(e) => {
		if (
			!e.target.closest("#cal-grid, #cal-head") ||
			performance.now() - calWheelAt < 260
		)
			return;
		calWheelAt = performance.now();
		e.preventDefault();
		shiftCalMonth(e.deltaY > 0 ? 1 : -1);
	},
	{ passive: false },
);
// clicks in here must not reach #pill's mousedown (it would start a window
// drag / read the click as "toggle the hub")
paneCalendar.addEventListener("mousedown", (e) =>
	e.stopPropagation(),
);
document
	.getElementById("hub-clock")
	.addEventListener("mousedown", (e) => e.stopPropagation());

async function loadCalendar() {
	applyCalendar(await invoke("get_calendar"));
}
listen("calendar-tick", (e) => applyCalendar(e.payload));

// -- custom backgrounds: an image or video behind the collapsed island, and
// optionally a different one behind the expanded hub. Files live in the app's
// data folder and load via the asset protocol (video streams with range
// requests). Videos only play while the island is actually on screen, so a
// hidden island costs no decoding. --
const bgLayers = {
	compact: {
		el: document.getElementById("bg-compact"),
		path: "",
		media: null,
	},
	hub: {
		el: document.getElementById("bg-hub"),
		path: "",
		media: null,
	},
};
let bgDim = 0.5;
let islandShown = false;
const BG_VIDEO_RE = /\.(mp4|webm|mov|m4v)$/i;

function bgLabel(path) {
	// stored as "<kind>-<timestamp>-<original name>.<ext>"
	const file = path.split(/[\\/]/).pop() || "";
	return file.replace(/^(compact|hub)-\d+-/, "");
}

function setBgLayer(layer, path) {
	if (layer.path === path) return;
	layer.path = path;
	layer.el.textContent = "";
	layer.media = null;
	if (!path) return;
	const src = window.__TAURI__.core.convertFileSrc(path);
	let media;
	if (BG_VIDEO_RE.test(path)) {
		media = document.createElement("video");
		media.muted = true;
		media.loop = true;
		media.playsInline = true;
		media.disablePictureInPicture = true;
		media.preload = "auto";
	} else {
		media = document.createElement("img");
		media.alt = "";
		media.decoding = "async";
		media.draggable = false;
	}
	media.src = src;
	layer.media = media;
	layer.el.append(media);
}

function applyBgDim(pct) {
	bgDim = pct / 100;
	bgLayers.compact.el.style.setProperty("--dim", bgDim);
	// the hub is mostly text: a little more scrim than the compact views
	bgLayers.hub.el.style.setProperty(
		"--dim",
		Math.min(0.92, bgDim + 0.12),
	);
}

// which layer shows, and which videos are allowed to run
function updateBg() {
	const hubWanted =
		currentView === "hub" && !!bgLayers.hub.path;
	const compactWanted = !!bgLayers.compact.path && !hubWanted;
	bgLayers.hub.el.classList.toggle("active", hubWanted);
	bgLayers.compact.el.classList.toggle(
		"active",
		compactWanted,
	);
	const visible = islandShown && !document.hidden;
	for (const [layer, wanted] of [
		[bgLayers.hub, hubWanted],
		[bgLayers.compact, compactWanted],
	]) {
		const v = layer.media;
		if (!v || v.tagName !== "VIDEO") continue;
		if (wanted && visible) v.play().catch(() => {});
		else v.pause();
	}
}

function applyBackgrounds(s) {
	if (!s) return;
	setBgLayer(bgLayers.compact, s.bg_compact || "");
	setBgLayer(bgLayers.hub, s.bg_hub || "");
	applyBgDim(s.bg_dim ?? 50);
	applyGlow(s.glow_intensity ?? 70);
	bleedLevel = (s.audio_bleed ?? 60) / 100;
	updateBg();
	for (const kind of ["compact", "hub"]) {
		const path = s["bg_" + kind] || "";
		document.getElementById(`bg-name-${kind}`).textContent =
			path ? bgLabel(path) : "None";
		document.getElementById(`bg-pick-${kind}`).textContent =
			path ? "Change" : "Choose";
		document
			.getElementById(`bg-clear-${kind}`)
			.classList.toggle("hidden", !path);
	}
}

const bgErrorEl = document.getElementById("bg-error");
for (const kind of ["compact", "hub"]) {
	document
		.getElementById(`bg-pick-${kind}`)
		.addEventListener("click", async () => {
			bgErrorEl.textContent = "";
			try {
				const s = await invoke("pick_background", {
					kind,
				});
				if (s) {
					currentSettings = s;
					applyBackgrounds(s);
				}
			} catch (err) {
				bgErrorEl.textContent = String(err);
			}
		});
	document
		.getElementById(`bg-clear-${kind}`)
		.addEventListener("click", async () => {
			bgErrorEl.textContent = "";
			currentSettings = await invoke("clear_background", {
				kind,
			});
			applyBackgrounds(currentSettings);
		});
}
for (const id of [
	"bg-pick-compact",
	"bg-pick-hub",
	"bg-clear-compact",
	"bg-clear-hub",
]) {
	document
		.getElementById(id)
		.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
}
document.addEventListener("visibilitychange", updateBg);
// at startup: the backgrounds apply before the hub (and its form) is ever opened
invoke("get_settings").then((s) => {
	applyBackgrounds(s);
	applyAccent(s.accent_color); // from the start, not only once the settings were opened
});

// every settings row is a <label>, so clicking anywhere on it (not just the
// control) toggles/focuses it -- that click also bubbles up to #pill's own
// mousedown handler, which would otherwise read it as "click the hub
// background" and close the hub right as the user tries to change a setting
for (const row of document.querySelectorAll(".hub-row")) {
	row.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
}

// -- hub stat rings: CPU/RAM/GPU (polled each second while the hub is open); the plugins add their own. Percent
// lives in the tooltip only. --
for (const el of document.querySelectorAll(".ring-wrap")) ringFrom(el);

async function pollSysStats() {
	if (currentView !== "hub") return;
	const s = await invoke("get_sys_stats");
	setRing(
		"cpu",
		s.cpu_pct,
		`CPU · ${Math.round(s.cpu_pct)}%`,
	);
	setRing(
		"ram",
		s.ram_pct,
		`RAM · ${Math.round(s.ram_pct)}%\n${s.ram_used_gb.toFixed(1)} / ${s.ram_total_gb.toFixed(1)} GB`,
	);
}
if (!FLOATS) setInterval(pollSysStats, 1000);

// GPU: Windows perf counters (Rust-side); null until the counter has a
// baseline or if the machine has no GPU counters -- ring stays hidden then
async function pollGpu() {
	if (currentView !== "hub") return;
	const pct = await invoke("get_gpu_pct");
	const ring = ringEls.gpu;
	if (pct == null) return;
	ring.el.classList.remove("hidden");
	setRing("gpu", pct, `GPU · ${Math.round(pct)}%`);
}
if (!FLOATS) setInterval(pollGpu, 1000);

// -- hub "Now" section: one media card per live session (live, from media-tick) + game / work /
// coding cards (polled from Rust while the hub is showing the info pane) --
const nowCardsEl = document.getElementById("now-cards");
let hubHasActivity = false;
// (an empty list just stays empty -- no placeholder text; callers still call this)
function updateInfoEmpty() {}


// ---- the plugins screen: every plugin in one list (the native ones, and the folders with a manifest)

// what a plugin asks to see, and a way to say no: shown before it is switched on
function confirmPlugin(p, item) {
	return new Promise((resolve) => {
		// inline, under the plugin's row: the island's window is only as tall as the hub, so an overlay would be cropped
		const box = el("div", "pg-dialog spot");
		box.append(el("h4", null, `Switch on ${p.name}?`));
		box.append(
			el(
				"p",
				"pg-dialog-note",
				p.kind === "native"
					? "This is part of the app. To do its job it looks at this:"
					: p.kind === "wasm"
						? `${p.bundled ? "It ships with the app." : "It is a module: code that runs here."} It runs in a sandbox, with no network, and can only see this:`
						: `${p.bundled ? "This ships with the app. " : ""}It can only ask the hosts listed, and draws only what it is given here:`,
			),
		);
		const list = el("div", "pg-dialog-perms");
		for (const x of p.permissions) {
			const r = el("div", "pg-dialog-perm");
			r.append(el("b", null, permName(x)), el("span", null, x.why));
			list.append(r);
		}
		if (!p.permissions.length) list.append(el("div", "pg-dialog-perm", "Nothing but a card of its own."));
		box.append(list);
		const buttons = el("div", "pg-dialog-buttons");
		const no = el("button", "set-btn", "Cancel");
		const yes = el("button", "set-btn go", "Switch on");
		for (const b of [no, yes]) b.type = "button";
		const done = (v) => {
			box.remove();
			scheduleHubHeight();
			resolve(v);
		};
		no.addEventListener("click", () => done(false));
		yes.addEventListener("click", () => done(true));
		buttons.append(no, yes);
		box.append(buttons);
		item.append(box);
		scheduleHubHeight();
		box.scrollIntoView({ block: "nearest" });
		yes.focus({ preventScroll: true });
	});
}

// a permission as a badge reads it: what, and (when there is one) of what
const permName = (x) => (x.detail ? `${x.name} · ${x.detail}` : x.name);

// a plugin's tile: its first letter, in a colour its id picks
const PLUGIN_TINTS = ["#7fd1a0", "#8fb6f0", "#e6b070", "#b79af0", "#7fd1d1", "#d98fa8", "#c9c9d0"];
function tintOf(id) {
	let h = 0;
	for (const c of id) h = (h * 31 + c.charCodeAt(0)) >>> 0;
	return PLUGIN_TINTS[h % PLUGIN_TINTS.length];
}
// the plugins whose row is open stay open when the list is drawn again
const openPlugins = new Set();

// one plugin: a row (its name, one line about it, its switch) that opens to its settings and what it can see
function pluginItem(p, reload) {
	const item = el("div", `pl${p.enabled ? "" : " off"}`);
	const row = el("div", "prow");
	const open = openPlugins.has(p.id);
	const main = el("button", "pmain");
	main.type = "button";
	main.setAttribute("aria-expanded", String(open));
	const tile = el("span", "tile", (p.name || "?").trim().charAt(0).toUpperCase());
	tile.style.background = tintOf(p.id);
	const text = el("span", "ptext");
	const name = el("span", "pname", p.name);
	if (!p.bundled) name.append(el("span", "pg-tag", p.kind));
	// one line: what is wrong, else what it says of itself, else what it is
	const line = p.error || p.note || p.description || "";
	text.append(name, el("span", `pdesc${p.error ? " pg-err" : ""}`, line));
	const chev = el("span", `chev${open ? " open" : ""}`);
	chev.innerHTML = li("chevron");
	main.append(tile, text, chev);
	const cb = document.createElement("input");
	cb.type = "checkbox";
	cb.checked = p.enabled;
	cb.disabled = !!p.error && !p.enabled;
	cb.setAttribute("aria-label", p.name);
	cb.addEventListener("change", async () => {
		if (cb.checked) {
			// (the switch stays off until the answer is yes)
			cb.checked = false;
			if (!(await confirmPlugin(p, item))) return;
			cb.checked = true;
		}
		await invoke("plugin_set", { id: p.id, on: cb.checked });
		refreshActivity();
		reload();
	});
	row.append(main, cb);
	item.append(row);

	// what it says, what it can see, and (while it is on) what the user can set and its banners
	const more = el("div", "pg-more");
	more.hidden = !open;
	main.addEventListener("click", () => {
		const now = more.hidden;
		more.hidden = !now;
		main.setAttribute("aria-expanded", String(now));
		chev.classList.toggle("open", now);
		if (now) openPlugins.add(p.id);
		else openPlugins.delete(p.id);
		scheduleHubHeight();
	});
	if (p.description) more.append(el("div", "pg-about", p.description));
	if (p.error) more.append(el("div", "pg-about pg-err", p.error));
	// what it may do: a badge each, and the words of the question before it is switched on in a tooltip
	if (p.permissions.length) {
		const perms = el("div", "pg-perms");
		perms.append(el("span", "pg-perms-title", "Permissions"));
		for (const x of p.permissions) {
			const badge = el("span", "perm-badge", permName(x));
			badge.dataset.tip = x.why;
			perms.append(badge);
		}
		more.append(perms);
	}
	if (p.enabled) {
		for (const s of p.settings) {
			const f = el("label", "pg-field");
			f.append(el("span", null, s.label));
			if (s.type === "choice") {
				// one of a few: the island's button group
				const group = el("span", "opt-group pg-choice");
				group.setAttribute("role", "group");
				const buttons = s.options.map((o) => {
					const b = el("button", null, o.label);
					b.type = "button";
					b.setAttribute("aria-pressed", String(o.value === s.value));
					b.addEventListener("click", async (e) => {
						e.preventDefault();
						for (const x of buttons) x.setAttribute("aria-pressed", String(x === b));
						await invoke("plugin_value", { id: p.id, key: s.key, value: o.value });
					});
					group.append(b);
					return b;
				});
				f.append(group);
				more.append(f);
				continue;
			}
			const inp = document.createElement("input");
			if (s.type === "toggle") {
				inp.type = "checkbox";
				inp.checked = !!s.value;
			} else {
				inp.type = s.type === "number" ? "number" : "text";
				if (s.type === "number") inp.step = "any";
				inp.value = s.value ?? "";
				inp.spellcheck = false;
			}
			inp.addEventListener("keydown", (e) => e.stopPropagation());
			inp.addEventListener("change", async () => {
				const v = s.type === "toggle" ? inp.checked : s.type === "number" ? Number(inp.value) : inp.value;
				await invoke("plugin_value", { id: p.id, key: s.key, value: v });
			});
			f.append(inp);
			more.append(f);
		}
		const foot = el("div", "pg-foot");
		// An action may need a word from the user (a code to paste): its field is always there, and what is put in it is kept
		// by itself (a pasted code, or Enter, or leaving the field); emptying the field takes it back. It is a secret, so its
		// letters are dots, and a field that holds something kept shows a row of dots.
		const asking = (p.actions || []).find((a) => a.ask);
		const ask = el("div", "pg-ask");
		const askErr = el("div", "pg-err");
		if (asking) {
			const SECRET = "•".repeat(12);
			let kept = asking.filled ? SECRET : "";
			const row = el("div", "pg-ask-row");
			const input = document.createElement("input");
			input.type = "password";
			input.autocomplete = "off";
			input.spellcheck = false;
			input.placeholder = asking.ask;
			input.setAttribute("aria-label", asking.ask);
			input.value = kept;
			const keep = async (text) => {
				if (text === kept) return;
				if (/^•+$/.test(text)) {
					input.value = kept; // (a dot or two deleted from the dots: nothing new)
					return;
				}
				askErr.textContent = "";
				try {
					await invoke("plugin_call", { id: p.id, cmd: asking.then, args: { text } });
				} catch (e) {
					askErr.textContent = String(e);
					return;
				}
				kept = text ? SECRET : "";
				input.value = kept;
				setTimeout(reload, 1500);
			};
			input.addEventListener("focus", () => {
				if (input.value === SECRET) input.select();
			});
			input.addEventListener("keydown", (e) => e.stopPropagation());
			input.addEventListener("paste", (e) => {
				const text = (e.clipboardData?.getData("text") || "").trim();
				if (!text) return;
				e.preventDefault();
				input.value = text;
				keep(text);
			});
			input.addEventListener("change", () => keep(input.value.trim()));
			const go = el("button", "set-btn", asking.label);
			go.type = "button";
			go.addEventListener("click", async () => {
				askErr.textContent = "";
				try {
					await invoke("plugin_call", { id: p.id, cmd: asking.id, args: null });
				} catch (e) {
					askErr.textContent = String(e);
					return;
				}
				input.focus();
			});
			row.append(input, go);
			ask.append(row, askErr);
		}
		for (const a of p.actions || []) {
			if (a.ask) continue; // (its button is by its field)
			const b = el("button", "set-btn", a.label);
			b.type = "button";
			b.addEventListener("click", async () => {
				try {
					await invoke("plugin_call", { id: p.id, cmd: a.id, args: null });
				} catch (e) {
					askErr.textContent = String(e);
					return;
				}
				setTimeout(reload, 1500);
			});
			foot.append(b);
		}
		if (p.banners) {
			// (a switch like the plugin's own settings: it is on or off)
			const f = el("label", "pg-field");
			f.append(el("span", null, "Mute banners"));
			const mute = document.createElement("input");
			mute.type = "checkbox";
			mute.checked = !!p.muted;
			mute.addEventListener("change", async () => {
				await invoke("plugin_mute", { id: p.id, muted: mute.checked });
				reload();
			});
			f.append(mute);
			more.append(f);
		}
		if (p.cost) foot.append(el("span", "pg-cost", p.cost));
		if (foot.childElementCount) more.append(foot);
		if (asking) more.append(ask);
	}
	item.append(more);
	return item;
}

async function renderPlugins() {
	let listing = { plugins: [], dnd: false };
	try {
		listing = (await invoke("plugin_list")) || listing;
	} catch (_) {}
	document.getElementById("plugin-dnd").checked = !!listing.dnd;
	document.getElementById("plugin-count").textContent = `Plugins · ${listing.plugins.length}`;
	// one list: the ones that ship with the app come first (the host orders them), then the folders
	document.getElementById("plugin-list").replaceChildren(...listing.plugins.map((p) => pluginItem(p, renderPlugins)));
	scheduleHubHeight();
}
document.getElementById("plugin-dnd").addEventListener("change", (e) => invoke("plugin_dnd", { on: e.target.checked }));
document.getElementById("plugin-folder").addEventListener("click", () => invoke("plugin_folder"));
document.getElementById("plugin-rescan").addEventListener("click", async () => {
	try {
		await invoke("plugin_rescan");
	} catch (_) {}
	renderPlugins();
});

let lastActivitySignature = "";
// All the cards of the moment, built: what the hub lists and what the floating cards are made from.
async function fetchCards() {
	const a = await invoke("get_activity");
	const cards = [];
	for (const p of plugins) if (a.cards?.[p.id]) cards.push(...(await p.cards(a.cards[p.id])));
	for (const p of a.plugins || []) cards.push(pluginCard(p));
	// (a stable sort: cards of one rank keep the order their plugin gave them)
	cards.sort((x, y) => (x._rank ?? 50) - (y._rank ?? 50));
	return { a, cards };
}

async function refreshActivity() {
	if (FLOATS) return refreshFloats();
	if (
		currentView !== "hub" ||
		paneInfo.classList.contains("hidden")
	)
		return;
	const built = await fetchCards();
	const a = built.a;
	setRings(a.rings || {});
	// the cards that float on the screen are not listed here
	floatKeys.clear();
	for (const i of await invoke("float_list")) floatKeys.add(i.key);
	const cards = built.cards.filter((c) => !floatKeys.has(c._key));
	const signature = cards
		.map((c) => c._sig ?? c.outerHTML)
		.join("");
	if (signature !== lastActivitySignature) {
		lastActivitySignature = signature;
		nowCardsEl.replaceChildren(...cards);
	} else {
		// same cards, new numbers: update in place so a click is never lost to a re-render
		cards.forEach((c, i) => {
			const old = nowCardsEl.children[i];
			if (old && old._update && c._data)
				old._update(c._data);
		});
	}
	hubHasActivity = cards.length > 0;
	updateInfoEmpty();
}
const refreshFloats = FLOATS
	? initFloats({ fetchCards, el, li, invoke, listen })
	: null;
setInterval(refreshActivity, 1000);

// -- tooltips: a single glass bubble instead of the native Win32 one. Any
// element with data-tip="Headline\nsub line" gets it (hub only -- the small
// pill has no room for one). Ring elements carry data-color for the dot. --
const tipEl = document.createElement("div");
tipEl.id = "tip";
document.body.append(tipEl);
let tipTarget = null;
let tipTimer = 0;

function renderTip(target) {
	const [head, ...rest] = (target.dataset.tip || "").split(
		"\n",
	);
	const h = el("div", "tip-head");
	if (target.dataset.color) {
		const dot = el("span", "tip-dot");
		dot.style.background = target.dataset.color;
		h.append(dot);
	}
	h.append(document.createTextNode(head));
	tipEl.replaceChildren(h);
	if (rest.length)
		tipEl.append(el("div", "tip-sub", rest.join(" ")));
}

function placeTip(target) {
	const r = target.getBoundingClientRect();
	const tw = tipEl.offsetWidth;
	const th = tipEl.offsetHeight;
	const x = Math.max(
		8,
		Math.min(
			r.left + r.width / 2 - tw / 2,
			window.innerWidth - tw - 8,
		),
	);
	const below = r.bottom + 10 + th <= window.innerHeight - 4;
	const y = below ? r.bottom + 10 : r.top - th - 10;
	tipEl.style.left = `${x}px`;
	tipEl.style.top = `${y}px`;
	tipEl.style.transformOrigin = `${r.left + r.width / 2 - x}px ${below ? "0" : "100%"}`;
	tipEl.style.setProperty(
		"--tip-shift",
		below ? "-4px" : "4px",
	);
}

function showTip(target) {
	if (!target.dataset.tip || currentView !== "hub") return;
	tipTarget = target;
	renderTip(target);
	placeTip(target);
	tipEl.classList.add("show");
}

function hideTip() {
	clearTimeout(tipTimer);
	tipTarget = null;
	tipEl.classList.remove("show");
}

function refreshTip(target) {
	if (tipTarget === target) {
		renderTip(target);
		placeTip(target);
	}
}

document.addEventListener("mouseover", (e) => {
	const target = e.target.closest?.("[data-tip]");
	if (target === tipTarget) return;
	hideTip();
	if (target)
		tipTimer = setTimeout(() => showTip(target), 80);
});
document.addEventListener("mouseleave", hideTip);
document.addEventListener("mousedown", hideTip);
hubBody.addEventListener("scroll", hideTip);

// Every native title="..." (the plain Windows tooltip) becomes the glass tooltip above: its text moves
// to data-tip. Done for what is in the page now and for anything added or retitled later.
function adoptTitle(node) {
	if (node.nodeType !== 1 || !node.hasAttribute("title"))
		return;
	const text = node.getAttribute("title");
	if (text) node.dataset.tip = text;
	node.removeAttribute("title");
	if (node === tipTarget) refreshTip(node);
}
function adoptTitles(root) {
	adoptTitle(root);
	if (root.querySelectorAll)
		root.querySelectorAll("[title]").forEach(adoptTitle);
}
adoptTitles(document.body);
new MutationObserver((muts) => {
	for (const m of muts) {
		if (m.type === "attributes") adoptTitle(m.target);
		else m.addedNodes.forEach(adoptTitles);
	}
}).observe(document.body, {
	subtree: true,
	childList: true,
	attributes: true,
	attributeFilter: ["title"],
});

// what only this page can do, for the plugins' files (see kit.js)
host.scheduleHubHeight = scheduleHubHeight;
host.refreshTip = refreshTip;
host.invalidate = () => (lastActivitySignature = "");
host.refresh = () => refreshActivity();
host.enter = playEnter;
for (const p of plugins) p.init?.();
initPluginUi();
