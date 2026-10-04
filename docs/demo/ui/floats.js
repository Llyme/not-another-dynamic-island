// The floating cards: hub cards that were dragged out of the island and now sit on the screen by themselves.
// This runs in the second window (index.html?floats=1, see floats.rs), which is a transparent layer over the
// desktop: the native side lets the mouse reach it only over the cards' rectangles, so everything else is click-through. The cards
// are the island's own (main.js builds them); this file only places them and moves them around.

const SNAP = 14; // px from a screen edge, or from another card, that pulls a card to it
const MARGIN = 12; // where a card settles against an edge
const GAP = 8; // between two cards that snap together (the gap between the island's own cards)
const WIDTH = 360; // how wide a card opens out to when it leaves the island
const MIN_W = 120; // the least a card can be resized to
const MIN_H = 48;
const GONE_S = 60; // a card whose source has been gone this long is let go

export function initFloats({ fetchCards, el, li, invoke, listen }) {
	const layer = document.getElementById("float-layer");
	const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
	const k = () => window.devicePixelRatio || 1;
	/** key -> { item, wrap, card, sig, missing } */
	const floats = new Map();
	let drag = null;
	let zTop = 1;
	let lastRegion = "";
	let regionTimer = 0;
	let beginning = null; // a drag-out that waits for its card
	let latestCursor = null;
	// the usable area of each monitor, in the layer's px (a card stays on one of them)
	let mons = [{ l: 0, t: 0, r: window.innerWidth, b: window.innerHeight }];
	async function loadMons() {
		try {
			const s = k();
			const list = (await invoke("float_monitors")).map((r) => ({ l: r[0] / s, t: r[1] / s, r: r[2] / s, b: r[3] / s }));
			if (list.length) mons = list;
		} catch (_) {}
	}

	// ---- the native side lets the mouse reach the window only over these rectangles
	function pushRegion() {
		regionTimer = 0;
		const s = k();
		const rects = [];
		for (const f of floats.values()) {
			if (f.wrap.hidden) continue;
			const r = f.wrap.getBoundingClientRect();
			rects.push([r.left * s, r.top * s, r.right * s, r.bottom * s]);
		}
		const sig = rects.map((r) => r.map(Math.round).join(",")).join(";");
		if (sig === lastRegion) return;
		lastRegion = sig;
		invoke("float_region", { rects, radius: 16 * s });
	}
	function regionSoon() {
		if (!regionTimer) regionTimer = setTimeout(pushRegion, 12);
	}

	// a card that grew (a guide that loaded, a section that opened) is pulled back onto its monitor
	const observer = new ResizeObserver((entries) => {
		regionSoon();
		for (const e of entries) {
			const f = floats.get(e.target.dataset.key);
			if (!f || (drag && drag.f === f) || f.wrap.classList.contains("lifted")) continue;
			const { x, y } = f.item;
			fit(f);
			if (f.item.x !== x || f.item.y !== y) {
				box(f);
				save(f);
			}
		}
	});

	function box(f) {
		const { item, wrap } = f;
		wrap.style.left = item.x + "px";
		wrap.style.top = item.y + "px";
		wrap.style.width = item.w + "px";
		wrap.style.height = item.h ? item.h + "px" : "";
		wrap.classList.toggle("sized", !!item.h);
	}
	function save(f) {
		invoke("float_set", { item: { key: f.item.key, x: f.item.x, y: f.item.y, w: f.item.w, h: f.item.h || 0, mode: f.item.mode || 0 } });
	}

	// ---- normal, translucent, transparent
	const MODE_NAMES = ["Normal card", "Translucent card", "Transparent card (follows what is behind it)"];
	function applyMode(f) {
		const m = f.item.mode || 0;
		f.wrap.dataset.mode = String(m);
		if (m !== 2) delete f.wrap.dataset.tone;
		if (f.modeBtn) {
			f.modeBtn.dataset.m = String(m);
			f.modeBtn.title = MODE_NAMES[m];
		}
		f.toneAt = 0;
		sampleTones();
	}

	// A transparent card has nothing behind its text: the native side says how light the screen is where the card is,
	// and the text goes dark over a light background and light over a dark one. In between it keeps what it has, so
	// it does not flicker over a mid-grey.
	let sampling = false;
	async function sampleTones() {
		if (sampling) return;
		sampling = true;
		try {
			const s = k();
			for (const f of floats.values()) {
				if ((f.item.mode || 0) !== 2 || f.wrap.hidden) continue;
				const now = performance.now();
				if (now - (f.toneAt || 0) < (f.wrap.classList.contains("lifted") ? 200 : 1000)) continue;
				f.toneAt = now;
				const r = f.wrap.getBoundingClientRect();
				let l = null;
				try {
					l = await invoke("float_backdrop", { rect: [r.left * s, r.top * s, r.right * s, r.bottom * s] });
				} catch (_) {}
				if (l == null) {
					// nothing could be read, or pure black (a black screen, or a game the desktop cannot see): after a few
					// times, light text, which is right for black and is the default
					if (++f.nulls >= 3) f.wrap.dataset.tone = "light";
					continue;
				}
				f.nulls = 0;
				const dark = f.wrap.dataset.tone === "dark";
				const next = l > 0.58 ? true : l < 0.42 ? false : dark;
				f.wrap.dataset.tone = next ? "dark" : "light";
			}
		} finally {
			sampling = false;
		}
	}
	setInterval(sampleTones, 250);

	// ---- one floating card
	function make(item) {
		const wrap = el("div", "fl");
		wrap.dataset.key = item.key;
		wrap.hidden = true; // until its card has been built
		const rz = el("div", "fl-rz");
		rz.addEventListener("pointerdown", (e) => startResize(e, f));
		wrap.append(rz);
		wrap.addEventListener("pointerdown", () => front(f), true);
		layer.append(wrap);
		const f = { item, wrap, card: null, sig: null, missing: 0, nulls: 0 };
		floats.set(item.key, f);
		observer.observe(wrap);
		box(f);
		applyMode(f);
		return f;
	}

	function front(f) {
		f.wrap.style.zIndex = String(++zTop);
	}

	function setCard(f, card) {
		if (f.card) f.card.remove();
		f.card = card;
		f.sig = card._sig ?? card.outerHTML;
		// a floating card is always open, and its chevron gives way to the close button
		card.classList.add("open");
		const row = card.querySelector(".now-row");
		if (row) {
			const x = el("button", "fl-x");
			x.type = "button";
			x.title = "Put back in the island";
			x.innerHTML = li("x");
			x.addEventListener("pointerdown", (e) => e.stopPropagation());
			x.addEventListener("click", (e) => {
				e.stopPropagation();
				dock(f.item.key);
			});
			// left of it: normal card, translucent card, transparent card (it follows what is behind it)
			const m = el("button", "fl-mode");
			m.type = "button";
			m.innerHTML = li("sq");
			m.addEventListener("pointerdown", (e) => e.stopPropagation());
			m.addEventListener("click", (e) => {
				e.stopPropagation();
				f.item.mode = ((f.item.mode || 0) + 1) % 3;
				applyMode(f);
				save(f);
			});
			row.append(m, x);
			f.modeBtn = m;
			applyMode(f);
			row.addEventListener("pointerdown", (e) => startDrag(e, f));
		}
		// a click on the header does not open or close it, nor go to the source (a double click still does)
		card.addEventListener(
			"click",
			(e) => {
				if (e.target.closest("button, input")) return;
				if (e.target.closest(".now-row")) e.stopImmediatePropagation();
			},
			true,
		);
		f.wrap.prepend(card);
		f.wrap.hidden = false;
		regionSoon();
	}

	function dock(key) {
		const f = floats.get(key);
		if (!f) return;
		observer.unobserve(f.wrap);
		f.wrap.remove();
		floats.delete(key);
		invoke("float_remove", { key });
		pushRegion();
	}

	// ---- keeping the cards current
	let busy = false;
	async function refresh() {
		if (busy || (!floats.size && !beginning)) return;
		busy = true;
		try {
			const { cards } = await fetchCards();
			const byKey = new Map(cards.map((c) => [c._key, c]));
			for (const [key, f] of [...floats]) {
				const c = byKey.get(key);
				if (!c) {
					// The card it came from is gone from the island (the context was lost, the page was closed): it stays, with
					// what it last said, until it is closed here. One that was never drawn this time (the app was restarted and
					// its source has not come back) waits a minute, then lets go.
					if (f.card) f.wrap.classList.add("stale");
					else if (++f.missing >= GONE_S) dock(key);
					continue;
				}
				f.missing = 0;
				f.wrap.classList.remove("stale");
				const sig = c._sig ?? c.outerHTML;
				if (f.card && f.sig === sig) {
					if (f.card._update && c._data) f.card._update(c._data);
					f.wrap.hidden = false;
				} else {
					setCard(f, c);
				}
			}
			pushRegion();
		} finally {
			busy = false;
		}
	}

	// ---- dragging: by the header here, or handed over by the island (see `begin` below)
	// (a card that is not drawn yet has the size it was given)
	const sizeOf = (f) => [f.wrap.offsetWidth || f.item.w, f.wrap.offsetHeight || f.item.h || 0];

	// the monitor a card is mostly on (else the nearest)
	function monOf(f) {
		const { x, y } = f.item;
		const [w, h] = sizeOf(f);
		let best = mons[0];
		let area = -1;
		for (const m of mons) {
			const a = Math.max(0, Math.min(x + w, m.r) - Math.max(x, m.l)) * Math.max(0, Math.min(y + h, m.b) - Math.max(y, m.t));
			if (a > area) {
				area = a;
				best = m;
			}
		}
		if (area > 0) return best;
		const cx = x + w / 2;
		const cy = y + h / 2;
		let d = Infinity;
		for (const m of mons) {
			const dx = Math.max(m.l - cx, 0, cx - m.r);
			const dy = Math.max(m.t - cy, 0, cy - m.b);
			if (dx * dx + dy * dy < d) {
				d = dx * dx + dy * dy;
				best = m;
			}
		}
		return best;
	}

	const within = (v, lo, hi) => (hi < lo ? lo : Math.max(lo, Math.min(hi, v)));

	// while held: anywhere on the desktop, never off it
	function clampXY(f) {
		const [w, h] = sizeOf(f);
		const l = Math.min(...mons.map((m) => m.l));
		const t = Math.min(...mons.map((m) => m.t));
		const r = Math.max(...mons.map((m) => m.r));
		const b = Math.max(...mons.map((m) => m.b));
		f.item.x = within(f.item.x, l, r - w);
		f.item.y = within(f.item.y, t, b - h);
	}

	// at rest: wholly inside one monitor
	function fit(f) {
		const m = monOf(f);
		const [w, h] = sizeOf(f);
		f.item.x = within(f.item.x, m.l, m.r - w);
		f.item.y = within(f.item.y, m.t, m.b - h);
	}

	// Pulls a card against the screen edges and against the other cards: beside one (with a gap), or lined up with
	// its edge. Only the edges that are within SNAP move; the rest of the position stays what the hand says.
	function snap(f) {
		const w = f.wrap.offsetWidth;
		const h = f.wrap.offsetHeight;
		let { x, y } = f.item;
		let bx = SNAP + 1;
		let by = SNAP + 1;
		let nx = x;
		let ny = y;
		const tryX = (to, from) => {
			const d = to - from;
			if (Math.abs(d) < Math.abs(bx)) {
				bx = d;
				nx = x + d;
			}
		};
		const tryY = (to, from) => {
			const d = to - from;
			if (Math.abs(d) < Math.abs(by)) {
				by = d;
				ny = y + d;
			}
		};
		// (the edges of the monitor it is mostly on)
		const m = monOf(f);
		tryX(m.l + MARGIN, x);
		tryX(m.r - MARGIN, x + w);
		tryY(m.t + MARGIN, y);
		tryY(m.b - MARGIN, y + h);
		for (const o of floats.values()) {
			if (o === f || o.wrap.hidden) continue;
			const ox = o.item.x;
			const oy = o.item.y;
			const ow = o.wrap.offsetWidth;
			const oh = o.wrap.offsetHeight;
			// (only cards that are next to it, or above or below it, count along the other axis)
			const near = (a0, a1, b0, b1) => a0 < b1 + SNAP + GAP && a1 > b0 - SNAP - GAP;
			if (near(y, y + h, oy, oy + oh)) {
				tryX(ox + ow + GAP, x);
				tryX(ox - GAP, x + w);
				tryX(ox, x);
				tryX(ox + ow, x + w);
			}
			if (near(x, x + w, ox, ox + ow)) {
				tryY(oy + oh + GAP, y);
				tryY(oy - GAP, y + h);
				tryY(oy, y);
				tryY(oy + oh, y + h);
			}
		}
		if (Math.abs(bx) <= SNAP) f.item.x = nx;
		if (Math.abs(by) <= SNAP) f.item.y = ny;
		return Math.abs(bx) <= SNAP || Math.abs(by) <= SNAP;
	}

	// How tall the card is when nothing holds it to a height (what it is as first opened), at a width. It is measured
	// by letting go of the height for a moment, inside one frame (nothing is drawn meanwhile); the scrolled
	// places inside it are put back, as a shorter box would have moved them.
	function naturalHeight(f, w) {
		const wrap = f.wrap;
		const sized = wrap.classList.contains("sized");
		const keep = [wrap.style.height, wrap.style.width];
		const scrolled = [...wrap.querySelectorAll("*")].filter((n) => n.scrollTop).map((n) => [n, n.scrollTop]);
		wrap.classList.remove("sized");
		wrap.style.height = "";
		wrap.style.width = w + "px";
		const h = wrap.offsetHeight;
		wrap.style.height = keep[0];
		wrap.style.width = keep[1];
		if (sized) wrap.classList.add("sized");
		for (const [n, top] of scrolled) n.scrollTop = top;
		return h;
	}

	// The same pull for the corner being resized: the right and bottom edges meet the monitor's edge, or the edges
	// of the cards that are next to it (beside one with a gap, or lined up with it), and the card meets what it
	// is as first opened: its own height (the whole of what it has to say, as far as it says it by itself) and the
	// width it opens with. The size is f.item's.
	function snapSize(f) {
		const { x, y } = f.item;
		let { w, h } = f.item;
		h = h || f.wrap.offsetHeight;
		const m = monOf(f);
		let br = SNAP + 1;
		let bb = SNAP + 1;
		let nr = x + w;
		let nb = y + h;
		const tryR = (to) => {
			if (Math.abs(to - (x + w)) < Math.abs(br)) {
				br = to - (x + w);
				nr = to;
			}
		};
		const tryB = (to) => {
			if (Math.abs(to - (y + h)) < Math.abs(bb)) {
				bb = to - (y + h);
				nb = to;
			}
		};
		tryR(m.r - MARGIN);
		tryB(m.b - MARGIN);
		tryR(x + WIDTH);
		const natB = y + naturalHeight(f, w);
		tryB(natB);
		for (const o of floats.values()) {
			if (o === f || o.wrap.hidden) continue;
			const ox = o.item.x;
			const oy = o.item.y;
			const ow = o.wrap.offsetWidth;
			const oh = o.wrap.offsetHeight;
			const near = (a0, a1, b0, b1) => a0 < b1 + SNAP + GAP && a1 > b0 - SNAP - GAP;
			if (near(y, y + h, oy, oy + oh)) {
				tryR(ox - GAP);
				tryR(ox + ow);
			}
			if (near(x, x + w, ox, ox + ow)) {
				tryB(oy - GAP);
				tryB(oy + oh);
			}
		}
		if (Math.abs(br) <= SNAP) f.item.w = nr - x;
		if (Math.abs(bb) <= SNAP) f.item.h = nb - y;
		// (held to its own height: it goes on being as tall as what it says, as the content changes)
		f.snapNat = Math.abs(bb) <= SNAP && nb === natB;
	}

	function lift(f, gx, gy) {
		loadMons(); // (a monitor may have come or gone since)
		front(f);
		drag = { f, gx, gy, samples: [] };
		f.wrap.classList.remove("landed");
		f.wrap.classList.add("lifted");
		pushRegion();
	}

	function dragTo(cx, cy) {
		if (!drag) return;
		const { f } = drag;
		f.item.x = cx - drag.gx;
		f.item.y = cy - drag.gy;
		clampXY(f);
		const snapped = snap(f);
		box(f);
		// a held card leans a little the way it is being pulled
		const now = performance.now();
		drag.samples.push({ t: now, x: f.item.x });
		while (drag.samples.length > 2 && now - drag.samples[0].t > 100) drag.samples.shift();
		if (!reduce && drag.samples.length > 1) {
			const a = drag.samples[0];
			const b = drag.samples[drag.samples.length - 1];
			const vx = (b.x - a.x) / Math.max(1, b.t - a.t);
			f.wrap.style.setProperty("--tilt", Math.max(-5, Math.min(5, vx * 3.2)).toFixed(2) + "deg");
		}
		f.wrap.classList.toggle("snapping", snapped);
		pushRegion();
	}

	async function endDrag(cx, cy) {
		const d = drag;
		drag = null;
		if (!d) return;
		const { f } = d;
		f.wrap.classList.remove("lifted", "snapping");
		f.wrap.style.removeProperty("--tilt");
		// let go over the island: back in
		const s = k();
		try {
			const r = await invoke("float_island");
			if (r && cx * s >= r[0] && cx * s <= r[2] && cy * s >= r[1] && cy * s <= r[3]) {
				dock(f.item.key);
				return;
			}
		} catch (_) {}
		settle(f);
	}

	// the header of a floating card, grabbed in this window
	function startDrag(e, f) {
		if (e.button !== 0 || e.target.closest("button, input, .now-seek, .rate-menu")) return;
		const row = e.currentTarget;
		const r = f.wrap.getBoundingClientRect();
		let started = false;
		const sx = e.clientX;
		const sy = e.clientY;
		// (a press on the icon that does not move is the focus button's: see main.js)
		const onIcon = !!e.target.closest(".now-focus");
		f.card._iconPress = onIcon;
		row.setPointerCapture(e.pointerId);
		const move = (ev) => {
			if (!started) {
				if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
				started = true;
				lift(f, sx - r.left, sy - r.top);
			}
			dragTo(ev.clientX, ev.clientY);
		};
		const up = (ev) => {
			row.removeEventListener("pointermove", move);
			row.removeEventListener("pointerup", up);
			row.removeEventListener("pointercancel", up);
			if (started) endDrag(ev.clientX, ev.clientY);
			else if (onIcon && f.card._focus && ev.type === "pointerup") f.card._focus();
		};
		row.addEventListener("pointermove", move);
		row.addEventListener("pointerup", up);
		row.addEventListener("pointercancel", up);
	}

	// ---- settling against an edge
	function settle(f) {
		clampXY(f);
		fit(f);
		snap(f);
		box(f);
		f.wrap.classList.remove("landed");
		void f.wrap.offsetWidth;
		f.wrap.classList.add("landed");
		pushRegion();
		save(f);
	}

	// ---- resizing
	// a double press on the corner puts the card back to the size it opens with
	function resetSize(f) {
		f.item.w = WIDTH;
		f.item.h = 0;
		f.wrap.classList.add("opening");
		clearTimeout(f.openingTimer);
		f.openingTimer = setTimeout(() => f.wrap.classList.remove("opening"), 260);
		clampXY(f);
		fit(f);
		box(f);
		pushRegion();
		save(f);
	}

	function startResize(e, f) {
		e.preventDefault();
		e.stopPropagation();
		const at = performance.now();
		if (f.rzAt && at - f.rzAt < 400 && Math.hypot(e.clientX - f.rzX, e.clientY - f.rzY) < 8) {
			f.rzAt = 0;
			resetSize(f);
			return;
		}
		f.rzAt = at;
		f.rzX = e.clientX;
		f.rzY = e.clientY;
		const handle = e.currentTarget;
		handle.setPointerCapture(e.pointerId);
		const sx = e.clientX;
		const sy = e.clientY;
		const w0 = f.wrap.offsetWidth;
		const h0 = f.wrap.offsetHeight;
		f.wrap.classList.add("lifted");
		const move = (ev) => {
			const m = monOf(f);
			// (no ceiling but the monitor's edge; the floor only keeps the corner handle reachable)
			f.item.w = Math.max(MIN_W, Math.min(m.r - f.item.x, w0 + ev.clientX - sx));
			f.item.h = Math.max(MIN_H, Math.min(m.b - f.item.y, h0 + ev.clientY - sy));
			snapSize(f);
			f.item.w = Math.max(MIN_W, f.item.w);
			f.item.h = f.snapNat ? 0 : Math.max(MIN_H, f.item.h);
			box(f);
			pushRegion();
		};
		const up = () => {
			handle.removeEventListener("pointermove", move);
			handle.removeEventListener("pointerup", up);
			f.wrap.classList.remove("lifted");
			pushRegion();
			save(f);
		};
		handle.addEventListener("pointermove", move);
		handle.addEventListener("pointerup", up);
	}

	// ---- a card dragged out of the island: the native side follows the cursor for us
	async function begin(b, released = false) {
		const s = k();
		beginning = b;
		await refresh0();
		const key = b.key;
		const f = floats.get(key);
		beginning = null;
		if (!f || !f.card) {
			if (f) dock(key);
			return;
		}
		const w0 = b.w / s;
		// the card opens out as it leaves; the grab point keeps its place along the header
		const gx = (b.grab_x / s) * (f.item.w / w0);
		const gy = b.grab_y / s;
		f.item.x = b.x / s;
		f.item.y = b.y / s;
		const target = f.item.w;
		f.item.w = w0;
		box(f);
		void f.wrap.offsetWidth;
		f.wrap.classList.add("opening");
		f.item.w = target;
		lift(f, gx, gy);
		box(f);
		setTimeout(() => f.wrap.classList.remove("opening"), 260);
		if (latestCursor) onCursor(latestCursor);
		// the button came up before this page could follow it: the card lands where it was taken out
		else if (released) endDrag(b.x / s + gx, b.y / s + gy);
	}

	// build the card of a drag that has just begun, without waiting for the next tick
	async function refresh0() {
		const { cards } = await fetchCards();
		for (const c of cards) {
			if (c._key !== beginning.key) continue;
			let f = floats.get(c._key);
			if (!f) f = make({ key: c._key, x: 0, y: 0, w: WIDTH, h: 0 });
			f.item.w = WIDTH;
			setCard(f, c);
		}
	}

	function onCursor(c) {
		const s = k();
		if (!drag) {
			latestCursor = c;
			return;
		}
		dragTo(c.x / s, c.y / s);
		if (!c.down) {
			latestCursor = null;
			endDrag(c.x / s, c.y / s);
		}
	}

	// the card's tab went to another page of its guide: the same card, under the key of the page it is on now
	listen("float-rekey", (e) => {
		const { old, new: now } = e.payload;
		const f = floats.get(old);
		if (!f) return;
		floats.delete(old);
		floats.get(now)?.wrap.remove();
		floats.set(now, f);
		f.item.key = now;
		f.missing = 0;
		refresh();
	});
	listen("float-begin", (e) => begin(e.payload));
	listen("float-cursor", (e) => onCursor(e.payload));

	// ---- start: what floated last time, and a drag-out that came before this page had loaded
	(async () => {
		const r = await invoke("float_ready");
		await loadMons();
		for (const item of r.items) {
			if (item.key === r.begin?.key) continue; // (made by `begin`)
			const f = make(item);
			clampXY(f);
			fit(f);
			box(f);
		}
		if (r.begin) begin(r.begin, !r.down);
		refresh();
	})();

	return refresh;
}
