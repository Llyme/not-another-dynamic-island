// Sound light, the face of the native plugin (src/builtin/sound_light): an aura that lives in the island itself instead of
// bars, and spills a little outside it. It paints what the sound is (see ../sound.js), and only while the plugin is on.

import { audio, director, VIZ_BANDS } from "../sound.js";
import { FLOATS, invoke, listen, view } from "../kit.js";

const pill = document.getElementById("pill");
// how far the window's margin goes around the island (set by the page from the cursor poll)
let cursorPad = 0;

// -- audio visualizer: an aura that lives in the island itself instead of bars.
// 16 soft gradient lights (one per frequency band, ~60 Hz..12 kHz) drift
// through the pill; the detected genre picks how they dance (talk huddles,
// chill drifts, groove/pop orbits, bang/rock slams). Bass-only kick hits
// send a ripple out from the eyes; a voice gets a warm halo that pulses
// with the syllables. Drawn under all content. (The ripples on the bass are the eyes'.) --
const vizCanvas = document.getElementById("viz");
const vizCtx = vizCanvas.getContext("2d");
const VIZ_SCALE = 0.7; // render below device resolution: it's all soft glow anyway
const viz = { hue: 150, on: false, fadeFrom: null, lightCur: [] };
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
		view.name === "hub" ? 38 : pill.offsetHeight / 2,
	];
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
		peakBand > 0.04;
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
		view.name === "hub"
			? 0.5
			: view.name === "idle"
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
	const expanded = view.name === "hub";
	const compact =
		view.name === "idle" ||
		view.name === "notification" ||
		view.name === "brief" ||
		view.name === "usage_peek";
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
			crowd *
			brightness;
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
			`hsla(38, 95%, 70%, ${Math.min(0.5, 0.3 * audio.talk * gain * (0.4 + env) * brightness)})`,
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

	c.globalCompositeOperation = "source-over";
}

// ---- the plugin: on or off, and how far the light spills out of the island ----
let on = true;
let brightness = 1; // the most the lights give: 1 = all of it, 0 = none
let drawnOnce = false;

function apply(s) {
	on = !!s.sound;
	brightness = Math.max(0, Math.min(100, s.brightness ?? 100)) / 100;
	bleedLevel = Math.max(0, Math.min(100, s.ambient ?? 60)) / 100;
}

// called every frame by the page
function frame(now, dt) {
	if (on && brightness > 0) {
		drawnOnce = true;
		paintViz(now, dt);
	} else if (drawnOnce) {
		// off: what was painted goes, once
		vizCtx.clearRect(0, 0, vizCanvas.width, vizCanvas.height);
		clearBleed();
		viz.on = false;
		viz.fadeFrom = null;
		drawnOnce = false;
	}
}

export const soundLight = {
	frame,
	setCursorPad: (pad) => {
		cursorPad = pad;
	},
};

export default {
	id: "sound-light",
	init() {
		if (FLOATS) return;
		listen("look-tick", (e) => apply(e.payload));
		invoke("look_state").then(apply).catch(() => {});
	},
	cards() {
		return [];
	},
};
