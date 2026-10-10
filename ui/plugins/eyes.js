// Eyes, the face of the native plugin (src/builtin/eyes): the two eyes of the island. They blink, look at the cursor,
// squash and stretch as it moves, glance around when it rests, and show moods (happy, sleepy, wide awake, and, when they
// react to sound, the beat of the music). Painted on a canvas every frame, on the idle pill and in the hub's header.

import { audio as heard, director as heardDirector, onBeat } from "../sound.js";
import { FLOATS, invoke, listen, view } from "../kit.js";

// What the eyes know of the sound: all of it while they react to it, none otherwise (they only ever read it).
const SILENT_AUDIO = {
	target: 0, level: 0, mode: "silent", kind: "silent", voice: 0, voiceSm: 0, bang: 0, bands: new Array(16).fill(0),
	bpm: 100, conf: 0, energy: 0, chill: 0, pan: 0, dance: 0, talk: 0, talkEnv: 0, kick: { p: 0, v: 0 }, nod: { p: 0, v: 0 }, sway: 0,
};
const SILENT_DIRECTOR = { state: "silent", video: false };
let audio = heard;
let director = heardDirector;
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
	const inIdle = view.name === "idle";

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

// ---- ripples: very thin circular waves expanding from the middle between the eyes, one for each bass hit. Sound panned to
// a side shows as a ring that is solid on that side and translucent on the other. Drawn on their own canvas (under the
// eyes, over the sound light), inside the island. ----
const pillEl = document.getElementById("pill");
const ringsCanvas = document.getElementById("rings");
const ringsCtx = ringsCanvas.getContext("2d");
const RINGS_SCALE = 0.7; // soft hairlines: no need for the device's resolution
const rings = [];
let ringsDrawn = false;
let lastRingT = performance.now();

function sizeRings() {
	// (layout size: the pinned island may be drawn scaled down, the buffer must not follow)
	ringsCanvas.width = Math.max(1, Math.round(pillEl.offsetWidth * RINGS_SCALE));
	ringsCanvas.height = Math.max(1, Math.round(pillEl.offsetHeight * RINGS_SCALE));
}
new ResizeObserver(sizeRings).observe(pillEl);
sizeRings();

// the colour drifts slowly, as the sound light's does
const ringHue = () => ((performance.now() / 1000) * 7) % 360 + 20;

// Bass only: one ring per kick-drum / strong low-end onset -- no merging and no rate limit beyond the analyzer's onset
// spacing. (A cap keeps a pathological input from piling up rings.)
onBeat((strength, pan) => {
	if (!on || !ringsOn) return;
	rings.push({ t: performance.now(), s: strength, sv: strength, hue: ringHue(), pan: pan || 0 });
	if (rings.length > 12) rings.shift();
});

function paintRings(now) {
	const dt = Math.min(Math.max((now - lastRingT) / 1000, 0), 0.05);
	lastRingT = now;
	const c = ringsCtx;
	const w = ringsCanvas.width;
	const h = ringsCanvas.height;
	if (!rings.length) {
		if (ringsDrawn) {
			c.clearRect(0, 0, w, h);
			ringsDrawn = false;
		}
		return;
	}
	ringsDrawn = true;
	c.setTransform(1, 0, 0, 1, 0, 0);
	c.clearRect(0, 0, w, h);
	const S = RINGS_SCALE;
	const size = Math.max(w, h);
	// where the eyes sit inside the pill
	const cx = (pillEl.offsetWidth / 2) * S;
	const cy = (view.name === "hub" ? 38 : pillEl.offsetHeight / 2) * S;
	c.lineWidth = 0.7; // hairline, always -- only the fade changes
	c.lineCap = "round";
	for (let i = rings.length - 1; i >= 0; i--) {
		const rg = rings[i];
		// rAF timestamps can trail performance.now() by a hair: never let age go negative
		const age = Math.max(0, (now - rg.t) / 900);
		if (age >= 1) {
			rings.splice(i, 1);
			continue;
		}
		const ease = 1 - Math.pow(1 - age, 2.2);
		// merged hits raise the target strength; the drawn strength eases toward it
		rg.sv += (rg.s - rg.sv) * Math.min(1, dt * 9);
		const side = Math.max(-1, Math.min(1, rg.pan || 0));
		const dirness = Math.max(0, Math.min((Math.abs(side) - 0.12) / 0.6, 1)); // 0 centered .. 1 hard-panned
		// how far it travels follows how strong the bass hit was: soft = a short ripple, a hard kick = a wave that
		// crosses the whole island
		const reach = 0.12 + 0.62 * rg.sv;
		const radius = Math.max(0, ease * size * reach);
		// near-opaque at birth, fading out slowly so the hairline stays easy to see
		const alpha = Math.min(1, (0.6 + 0.4 * rg.sv) * Math.pow(1 - age, 0.7)) * (view.name === "hub" ? 0.85 : 1);
		if (dirness > 0.02 && radius > 1) {
			// sound from one side: the ring is solid on the side it comes from and turns translucent across to the other,
			// more so the harder it is panned
			const far = alpha * (1 - 0.85 * dirness);
			const g = c.createLinearGradient(cx - side * radius, cy, cx + side * radius, cy);
			g.addColorStop(0, `hsla(${rg.hue}, 95%, 76%, ${far})`);
			g.addColorStop(0.5, `hsla(${rg.hue}, 95%, 76%, ${(far + alpha) / 2})`);
			g.addColorStop(1, `hsla(${rg.hue}, 95%, 76%, ${alpha})`);
			c.strokeStyle = g;
		} else {
			c.strokeStyle = `hsla(${rg.hue}, 95%, 76%, ${alpha})`;
		}
		c.beginPath();
		c.arc(cx, cy, radius, 0, Math.PI * 2);
		c.stroke();
	}
}

// ---- the plugin: on or off, whether the eyes react to sound, and whether the beat makes ripples ----
let on = true;
let reactToSound = true;
let ringsOn = true;
let drawn = false;

function apply(s) {
	on = !!s.eyes;
	reactToSound = !!s.eyes_react;
	ringsOn = !!s.eyes_rings;
}

function clearRings() {
	ringsCtx.clearRect(0, 0, ringsCanvas.width, ringsCanvas.height);
	ringsDrawn = false;
}

// called every frame by the page (idle and hub only: the other views have no eyes; the ripples go on in all of them)
function frame(now) {
	if (on && ringsOn) paintRings(now);
	else if (rings.length || ringsDrawn) {
		// (off: the ripples go at once)
		rings.length = 0;
		clearRings();
	}
	audio = reactToSound ? heard : SILENT_AUDIO;
	director = reactToSound ? heardDirector : SILENT_DIRECTOR;
	if (!on) {
		// no eyes: nothing to draw, and what was drawn is cleared once
		if (drawn) {
			ctx.clearRect(0, 0, canvas.width, canvas.height);
			hubEyesCtx.clearRect(0, 0, hubEyesCanvas.width, hubEyesCanvas.height);
			drawn = false;
		}
	} else if (view.name === "idle") {
		drawn = true;
		updateMood(now);
		tickBlink(now);
		paintEyes();
	} else if (view.name === "hub") {
		drawn = true;
		updateMood(now);
		tickBlink(now);
		paintHubEyes();
	}
}

// the cursor moved (Rust's global poll): where it is in the window, and how far it went since the last
function cursor(local_x, local_y, cursorPad) {
	const dx = local_x - eyeState.lastCursor.x;
	const dy = local_y - eyeState.lastCursor.y;
	eyeState.lastCursor = { x: local_x, y: local_y };
	const speed = Math.hypot(dx, dy);
	const nowMs = performance.now();
	if (speed > 2) {
		if (nowMs - mood.lastMoveAt > 25000) mood.wideUntil = nowMs + 500; // startled awake
		mood.lastMoveAt = nowMs;
	}
	return { dx, dy, speed };
}

// the cursor is over the island or not, and how it moves the eyes: squash, stretch, gaze
function track(local_x, local_y, dx, dy, speed, hover) {
	mood.hover = hover;
	const targetSquish = Math.min(speed / 140.0, 1.0);
	if (targetSquish > eyeState.squish) {
		eyeState.squish = targetSquish;
		if (speed > 0.5) eyeState.squishDir = [dx / speed, dy / speed];
	} else {
		eyeState.squish *= 0.78;
	}
	eyeState.smoothedCursor.x += (local_x - eyeState.smoothedCursor.x) * 0.35;
	eyeState.smoothedCursor.y += (local_y - eyeState.smoothedCursor.y) * 0.35;
}

export const eyes = {
	mood,
	frame,
	cursor,
	track,
	/** the canvases follow the layout (the hub's one is only sized once it is on screen) */
	sizeHub: () => sizeCanvasToElement(hubEyesCanvas),
	/** the face canvas is left alone while another view has the island: clear its last frame */
	clearFace: () => ctx.clearRect(0, 0, canvas.width, canvas.height),
};

export default {
	id: "eyes",
	init() {
		if (FLOATS) return;
		listen("look-tick", (e) => apply(e.payload));
		invoke("look_state").then(apply).catch(() => {});
	},
	cards() {
		return [];
	},
};
