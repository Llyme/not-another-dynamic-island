// What the island's own page and the plugins' pages (ui/plugins/*.js) share: icons, the pieces a card is made of, the
// header every card has (open and close, drag out of the hub, go to its app), and the pills.
//
// main.js fills in `host` with what only it can do (its layout, its tooltip) when it starts.

export const { invoke } = window.__TAURI__.core;
const { listen: listenAll } = window.__TAURI__.event;
// The floating cards live in a second window that runs this same page (see floats.rs): it draws nothing but
// those cards, so it only listens for its own events and skips the island's animations and polling.
export const FLOATS = new URLSearchParams(location.search).has("floats");
export const listen = FLOATS
	? (name, cb) => (name.startsWith("float-") ? listenAll(name, cb) : Promise.resolve(() => {}))
	: listenAll;

/** the island's view now ("idle", "hub", a plugin's pill...): main.js keeps it */
export const view = { name: "idle" };

/** a call to a plugin: a native plugin's command (see native.rs), or a button's action for a module (it is sent the action) */
export const call = (id, cmd, args = null) => invoke("plugin_call", { id, cmd, args });

export const host = {
	scheduleHubHeight() {},
	refreshTip() {},
	/** the list of cards must be built again, not just updated */
	invalidate() {},
	/** fetch the cards now (after a click that changed something) */
	refresh() {},
	/** play the entrance of a pill that the island has just moved to */
	enter() {},
};

// cards the user has opened (all start collapsed); kept by key so a re-render keeps them open
export const openCards = new Set();
// the cards that float on the screen by themselves: the hub's list leaves them out
export const floatKeys = new Set();

// the pills a plugin offers: the island shows the one the core names (view-tick), see `addPill`
export const pills = new Map();

/** A plugin's pill: its markup goes into the island, hidden until the island asks for it by name. */
export function addPill(id, html) {
	const node = document.createElement("div");
	node.id = id;
	node.className = "hidden";
	node.innerHTML = html;
	const pill = document.getElementById("pill");
	pill.insertBefore(node, document.getElementById("notification"));
	pills.set(id, node);
	return node;
}

// -- small shared helpers for the compact views --
export const ICON = {
	prev: '<svg viewBox="0 0 24 24"><path d="M6 6h2v12H6zM20 6v12L9.5 12z"/></svg>',
	next: '<svg viewBox="0 0 24 24"><path d="M16 6h2v12h-2zM4 6l10.5 6L4 18z"/></svg>',
	play: '<svg viewBox="0 0 24 24"><path d="M8 5v14l11-7z"/></svg>',
	pause: '<svg viewBox="0 0 24 24"><path d="M6 5h4v14H6zM14 5h4v14h-4z"/></svg>',
	bell: '<svg viewBox="0 0 24 24"><path d="M12 22a2.5 2.5 0 0 0 2.4-2h-4.8A2.5 2.5 0 0 0 12 22zm7-6V11a7 7 0 0 0-5-6.7V3a2 2 0 0 0-4 0v1.3A7 7 0 0 0 5 11v5l-2 2v1h18v-1z"/></svg>',
};

// ---- line icons used across the island: one stroke style, coloured by the text around them ----
export const LI_PATHS = {
	plug: "M6 2.5v3M10 2.5v3M4.5 5.5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11.5V14",
	island: "M5 5.5h6a2.5 2.5 0 0 1 0 5H5a2.5 2.5 0 0 1 0-5z",
	palette:
		"M8 2.5a5.5 5.5 0 1 0 0 11c.9 0 1.2-.7.8-1.3-.4-.6 0-1.4.8-1.4h1.4a2 2 0 0 0 2-2A5.5 5.5 0 0 0 8 2.5zM5 8h.01M7 5.5h.01M10 5.5h.01",
	reset: "M3.2 8a4.8 4.8 0 1 0 1.5-3.5M3 2.8v2.4h2.4",
	gamepad:
		"M4.6 5.5h6.8a3 3 0 0 1 2.9 3.7l-.6 2.3a1.5 1.5 0 0 1-2.6.5L10 10.5H6l-1.1 1.5a1.5 1.5 0 0 1-2.6-.5l-.6-2.3a3 3 0 0 1 2.9-3.7zM5.2 7.3v2.6M3.9 8.6h2.6M10.6 7.8h.01M12.1 9.3h.01",
	briefcase:
		"M2.5 5.5h11v7.5h-11zM6 5.5V4a1 1 0 0 1 1-1h2a1 1 0 0 1 1 1v1.5M2.5 9h11",
	lock: "M4 7.2h8v6.3H4zM5.6 7.2V5.2a2.4 2.4 0 0 1 4.8 0v2M8 9.6v1.6",
	unlock: "M4 7.2h8v6.3H4zM5.6 7.2V5.2a2.4 2.4 0 0 1 4.6-.9M8 9.6v1.6",
	shrink: "M6 3v3H3M10 3v3h3M6 13v-3H3M10 13v-3h3",
	term: "M2.5 3.5h11v9h-11zM5 6.5 7 8.2 5 9.9M8.5 10h2.5",
	search: "M7 3a4 4 0 1 0 0 8 4 4 0 0 0 0-8zM10 10l3 3",
	phone: "M3.6 2.6h2.4l1.1 2.8-1.5 1a7.4 7.4 0 0 0 3.4 3.4l1-1.5 2.8 1.1v2.4a1.4 1.4 0 0 1-1.5 1.4A10.6 10.6 0 0 1 2.2 4.1a1.4 1.4 0 0 1 1.4-1.5zM10 2.4a3.6 3.6 0 0 1 3.6 3.6M10 4.7a1.4 1.4 0 0 1 1.3 1.3",
	moon: "M12.5 9.8A5 5 0 0 1 6.2 3.5a5 5 0 1 0 6.3 6.3z",
	chevron: "M4.5 6.5 8 10l3.5-3.5",
	folder: "M2 4.5A1.5 1.5 0 0 1 3.5 3h2.8l1.4 1.5h4.8A1.5 1.5 0 0 1 14 6v5.5a1.5 1.5 0 0 1-1.5 1.5h-9A1.5 1.5 0 0 1 2 11.5z",
	refresh: "M13 8a5 5 0 1 1-1.6-3.7M13 2.8v2.7h-2.7",
	download: "M8 2.5v7.6M4.8 7.3 8 10.5l3.2-3.2M3 13h10",
	up: "M8 13V3M3.5 7.5 8 3l4.5 4.5",
	repost: "M3 7V6a2 2 0 0 1 2-2h7M10 2l2 2-2 2M13 9v1a2 2 0 0 1-2 2H4M6 14l-2-2 2-2",
	heart: "M8 13.5S2.5 10 2.5 6.2A2.9 2.9 0 0 1 8 5a2.9 2.9 0 0 1 5.5 1.2C13.5 10 8 13.5 8 13.5z",
	eye: "M1.8 8S4 4 8 4s6.2 4 6.2 4-2.2 4-6.2 4S1.8 8 1.8 8zM8 6.4a1.6 1.6 0 1 0 0 3.2 1.6 1.6 0 0 0 0-3.2z",
	eyeoff: "M2 8s2.2-4 6-4c.9 0 1.7.2 2.4.5M14 8s-2.2 4-6 4c-.9 0-1.7-.2-2.4-.5M3 13 13 3",
	power: "M8 2v5.6M4.7 4.5a5 5 0 1 0 6.6 0",
	cursor: "M3.2 2.6 12.6 6.5 8.7 8.2 7 12.4z",
	sq: "M3.8 3.8h8.4v8.4H3.8z",
	focus:
		"M8 2.5v2.4M8 11.1v2.4M2.5 8h2.4M11.1 8h2.4M8 6.2a1.8 1.8 0 1 0 0 3.6 1.8 1.8 0 0 0 0-3.6z",
	follow: "M2.5 8h11M5 5.5 2.5 8 5 10.5M11 5.5 13.5 8 11 10.5",
	wave: "M3 6.2v3.6M6 3.6v8.8M9 5.6v4.8M12 4.2v7.6",
	timer: "M8 3.4a5 5 0 1 0 0 9.9 5 5 0 0 0 0-9.9zM8 5.8V8.4l1.7 1.1M6.6 1.6h2.8",
	hourglass:
		"M4.5 2.5h7M4.5 13.5h7M5 2.5c0 3 3 3.4 3 5.5s-3 2.5-3 5.5M11 2.5c0 3-3 3.4-3 5.5s3 2.5 3 5.5",
	image: "M2.5 3.5h11v9h-11zM2.5 10.6l3.2-3.2 3 3 2-2 2.8 2.8M10.6 6.3h.01",
	expand: "M3 6V3h3M13 6V3h-3M3 10v3h3M13 10v3h-3",
	contrast:
		"M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 2.5v11",
	calendar:
		"M2.5 4.5h11v9h-11zM2.5 7.5h11M5.5 2.5v3M10.5 2.5v3",
	bell: "M4.5 11V7.5a3.5 3.5 0 0 1 7 0V11l1 1.2h-9zM6.7 13.6a1.4 1.4 0 0 0 2.6 0",
	link: "M6.8 9.2a2.6 2.6 0 0 0 3.7 0l2-2a2.6 2.6 0 0 0-3.7-3.7l-.9.9M9.2 6.8a2.6 2.6 0 0 0-3.7 0l-2 2a2.6 2.6 0 0 0 3.7 3.7l.9-.9",
	code: "M5.5 4.5 2 8l3.5 3.5M10.5 4.5 14 8l-3.5 3.5M9 3.5l-2 9",
	pen: "M10.5 2.8l2.7 2.7-7.3 7.3-3.2.5.5-3.2zM9 4.3l2.7 2.7",
	sun: "M8 5.5a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5zM8 1.8v1.6M8 12.6v1.6M1.8 8h1.6M12.6 8h1.6M3.6 3.6l1.1 1.1M11.3 11.3l1.1 1.1M3.6 12.4l1.1-1.1M11.3 4.7l1.1-1.1",
	sparkle:
		"M8 2l1.4 3.6L13 7l-3.6 1.4L8 12l-1.4-3.6L3 7l3.6-1.4zM12.5 11.5v2.5M11.3 12.7h2.4",
	mail: "M2.5 4h11v8h-11zM2.5 4.6 8 9l5.5-4.4",
	music: "M6 12.2V3.8l7-1.3v8.4M6 12.2a1.7 1.7 0 1 1-3.4 0 1.7 1.7 0 0 1 3.4 0zM13 10.9a1.7 1.7 0 1 1-3.4 0 1.7 1.7 0 0 1 3.4 0z",
	globe: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2zM2 8h12M8 2c2 1.7 2.8 3.7 2.8 6S10 12.3 8 14c-2-1.7-2.8-3.7-2.8-6S6 3.7 8 2z",
	dot: "M8 6.2a1.8 1.8 0 1 0 0 3.6 1.8 1.8 0 0 0 0-3.6z",
	plus: "M8 3.5v9M3.5 8h9",
	minus: "M3.5 8h9",
	x: "M4 4l8 8M12 4l-8 8",
	clock: "M8 2.5a5.5 5.5 0 1 0 0 11 5.5 5.5 0 0 0 0-11zM8 5v3.2l2.2 1.3",
	sliders:
		"M3 5h6.5M12 5h1M3 11h1.5M7 11h6M9.5 3.4v3.2M4.5 9.4v3.2",
	file: "M4 2.5h5l3 3v8H4zM9 2.5v3h3",
	check: "M3.5 8.5 6.5 11.5 12.5 4.5",
	pillshape:
		"M4.5 5.5h7a2.5 2.5 0 0 1 0 5h-7a2.5 2.5 0 0 1 0-5z",
	steps: "M3 4h2M7 4h6M3 8h2M7 8h6M3 12h2M7 12h6",
	book: "M3 3.5h5v9H3zM8 3.5h5v9H8z",
	news: "M3 3h8v10H3zM11 6h2v7h-2M5 5.5h4M5 8h4M5 10.5h2",
	pot: "M3.5 7h9v4.5a1.5 1.5 0 0 1-1.5 1.5H5a1.5 1.5 0 0 1-1.5-1.5zM2.5 7h11M6 4.5c0-1 1-1 1-2M9 4.5c0-1 1-1 1-2",
	ask: "M8 13.2v.1M6.2 6.2a1.9 1.9 0 1 1 2.7 1.7c-.6.3-.9.7-.9 1.4",
	tag: "M2.5 8.5V3.5h5l6 6-5 5zM5.5 6h.01",
	play: "M4.5 3.5v9l8-4.5z",
	pause: "M5.5 3.5v9M10.5 3.5v9",
	prev: "M4 3.5v9M12 3.8v8.4L5.8 8z",
	next: "M12 3.5v9M4 3.8v8.4L10.2 8z",
	branch: "M4.5 3v10M11.5 5.5v1c0 2-3 2-7 4",
	chat: "M2.8 3.5h10.4v6.8H8l-2.6 2.2v-2.2H2.8zM5 6.2h6M5 8.2h4",
	shield: "M8 2.2 12.6 4v3.3c0 2.9-1.9 4.9-4.6 6.2-2.7-1.3-4.6-3.3-4.6-6.2V4z",
};
// which glyph stands for a kind of page (see pagekind.rs)
export const KIND_GLYPH = {
	walkthrough: "steps",
	wiki: "book",
	news: "news",
	video: "play",
	recipe: "pot",
	qna: "ask",
	api: "code",
	product: "tag",
	search: "search",
	webapp: "branch",
	social: "chat",
};

export function li(name) {
	// (the name may come from a plugin: only the island's own icons are drawn, never a property of the table itself)
	const d = Object.hasOwn(LI_PATHS, name) ? LI_PATHS[name] : LI_PATHS.dot;
	return `<svg class="li" viewBox="0 0 16 16" aria-hidden="true"><path d="${d}"/></svg>`;
}

// an element that starts with an icon, then plain text (never HTML)
export function withIcon(node, name, text) {
	node.innerHTML = li(name);
	node.append(document.createTextNode(` ${text}`));
	return node;
}


export const RING_R = 8;
export const RING_C = 2 * Math.PI * RING_R;

export function formatDuration(secs) {
	const s = Math.max(0, Math.floor(secs));
	const h = Math.floor(s / 3600);
	const m = Math.floor((s % 3600) / 60);
	if (h > 0) return `${h}h ${m}m`;
	if (m > 0) return `${m}m`;
	return "<1m";
}

export function el(tag, cls, text) {
	const e = document.createElement(tag);
	if (cls) e.className = cls;
	if (text != null) e.textContent = text;
	return e;
}

export function iconEl(icon, glyph) {
	if (icon) {
		const img = el("img", "now-icon");
		img.src = icon;
		img.alt = "";
		return img;
	}
	const box = el("div", "now-icon glyph");
	box.innerHTML = li(glyph);
	return box;
}

// `v` is a string, or { icon, text } for a short text that an icon explains
export function line(cls, v) {
	const node = el("div", cls);
	if (v && typeof v === "object")
		withIcon(node, v.icon, v.text);
	else node.textContent = v;
	return node;
}

export function textCol(title, sub) {
	const col = el("div", "now-text");
	col.append(line("now-title", title), line("now-sub", sub));
	return col;
}

// how long something has been going: a stopwatch and the time, no words
export const going = (secs) => ({
	icon: "timer",
	text: formatDuration(secs),
});

// Every card has a collapsed state (the default): its header only. A chevron opens it to show
// the rest. Call this once the card is fully built; `key` identifies it across refreshes.
export function collapsible(card, key) {
	// the signature that decides whether the list must be rebuilt ignores the open/closed state
	if (card._sig === undefined) card._sig = card.outerHTML;
	card._key = key;
	card.classList.add("collapsible");
	if (openCards.has(key)) card.classList.add("open");
	const btn = el("button", "card-toggle");
	btn.innerHTML = li("chevron");
	const sync = () => {
		btn.title = card.classList.contains("open")
			? "Collapse"
			: "Expand";
	};
	sync();
	// the whole header toggles it (the chevron only shows the state); the header's own
	// buttons (media controls) keep doing their own thing
	const row = card.querySelector(".now-row") || card;
	row.classList.add("toggles");
	// the header is the row plus the card's padding around it: no dead spots
	card._inHeader = (e) => {
		if (
			e.target.closest(
				"button:not(.card-toggle), .now-seek, .media-extra, .rate-menu, .work-body, .dl-list",
			)
		)
			return false;
		const r = row.getBoundingClientRect();
		return e.clientY <= r.bottom + 12;
	};
	// a card with nothing more to show when opened (no body, no seek bar...) is not expandable: no
	// chevron, and its header goes to the app like the rest of the card (see .flat-card)
	card._syncBody = () => {
		const bodies = [
			...card.querySelectorAll(".work-body"),
		].some(
			(b) =>
				b.children.length > 0 || b.textContent.trim(),
		);
		const more =
			bodies ||
			card.querySelector(
				".dl-list, .now-seek:not(.hidden), .media-extra:not(.hidden)",
			);
		card.classList.toggle("flat-card", !more);
		if (!more) card.classList.remove("open");
	};
	card._syncBody();
	card.addEventListener("click", (e) => {
		// a press on the icon went to the app (see focusButton): it does not also open the card
		if (card._iconPress) {
			card._iconPress = false;
			e.stopImmediatePropagation();
			return;
		}
		if (e.target.closest(".gd")) return;
		if (
			card.classList.contains("flat-card") ||
			!card._inHeader(e)
		)
			return;
		e.stopImmediatePropagation(); // not "go to the app"
		const open = card.classList.toggle("open");
		if (open) openCards.add(key);
		else openCards.delete(key);
		sync();
		host.scheduleHubHeight();
	});
	row.append(btn);
	if (!FLOATS) dragOut(card, row, key);
	return card;
}

// Cards that float on the screen by themselves (floating cards, see floats.rs): the island leaves them out of
// its list. A card is dragged out by its header; the other window takes it over from there.
export function dragOut(card, row, key) {
	row.addEventListener("pointerdown", (e) => {
		if (
			e.button !== 0 ||
			e.target.closest(
				"button:not(.card-toggle), input, .now-seek, .rate-menu",
			)
		)
			return;
		const sx = e.clientX;
		const sy = e.clientY;
		const onIcon = !!e.target.closest(".now-focus");
		card._iconPress = onIcon;
		row.setPointerCapture(e.pointerId);
		const done = () => {
			row.removeEventListener("pointermove", move);
			row.removeEventListener("pointerup", up);
			row.removeEventListener("pointercancel", done);
		};
		// let go without having moved: on the icon, that is a press of the focus button
		const up = () => {
			done();
			if (onIcon && card._focus) card._focus();
		};
		const move = async (ev) => {
			if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 6) return;
			done();
			const r = card.getBoundingClientRect();
			floatKeys.add(key);
			card.classList.add("leaving");
			const ok = await invoke("float_begin", {
				key,
				x: r.left,
				y: r.top,
				w: r.width,
				h: r.height,
				grabX: sx - r.left,
				grabY: sy - r.top,
			});
			if (ok) {
				card.remove();
				host.invalidate();
				host.scheduleHubHeight();
			} else {
				floatKeys.delete(key);
				card.classList.remove("leaving");
			}
		};
		row.addEventListener("pointermove", move);
		row.addEventListener("pointerup", up);
		row.addEventListener("pointercancel", done);
	});
}

// What a collapsed card still says at the right of its header (the sub-line is hidden then);
// `v` is a string or { icon, text } like `line`. Call before `collapsible`.
export function peek(card, v) {
	const row = card.querySelector(".now-row");
	if (row && v) row.append(line("now-peek", v));
	return card;
}

// a card that jumps to its source app when clicked (mousedown must not reach
// #pill, whose handler would read the click as "toggle the hub")
export function focusable(
	card,
	args,
	go = () => invoke("focus_source", args),
) {
	card.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
	// the header opens/closes the card (see collapsible); going to the app is the icon's job
	focusButton(card, go);
	return card;
}

// The card's icon is the button that goes to the app the card is about: on hover it shows a focus mark. The
// press itself is taken by the header's pointer handlers (they hold the pointer for dragging, so the icon would
// never see a click): they call `card._focus` for a press that did not move.
export function focusButton(card, go) {
	const icon = card.querySelector(".now-row .now-icon") || card.querySelector(".now-icon");
	if (!icon || icon.parentElement.classList.contains("now-focus")) return;
	const wrap = el("span", "now-focus");
	wrap.title = "Go to it";
	icon.replaceWith(wrap);
	const mark = el("span", "now-focus-i");
	mark.innerHTML = li("focus");
	wrap.append(icon, mark);
	card._focus = go;
}


// a small progress ring like the ones at the top right of the hub
export function statRing(color) {
	const el = document.createElement("span");
	el.className = "ring-wrap hidden";
	el.innerHTML = `<svg viewBox="0 0 20 20"><circle class="ring-bg" cx="10" cy="10" r="${RING_R}"></circle><circle class="ring-fg" cx="10" cy="10" r="${RING_R}" stroke="${color}" stroke-dasharray="0 ${RING_C}"></circle></svg>`;
	const fg = el.querySelector(".ring-fg");
	return {
		el,
		set(pct, tip) {
			if (pct == null) {
				el.classList.add("hidden");
				return;
			}
			el.classList.remove("hidden");
			fg.setAttribute(
				"stroke-dasharray",
				`${(Math.max(0, Math.min(100, pct)) / 100) * RING_C} ${RING_C}`,
			);
			el.dataset.tip = tip;
			host.refreshTip(el);
		},
	};
}


// ---- what is playing (read by the media sensor, src/media.rs)
// media-tick arrives as a list of sessions (one per player); older
// payloads and some harnesses still send a single session object.
export function mediaListOf(payload) {
	if (Array.isArray(payload))
		return payload.filter((m) => m && m.has_session);
	if (payload && payload.has_session) return [payload];
	return [];
}

// the pill and the eyes follow one session: the playing one, else the first.
export function pickPrimaryMedia(list) {
	return (
		(list || []).find((m) => m.playing) || (list || [])[0] || null
	);
}

export function setPlayIcon(btn, playing) {
	const want = playing ? "pause" : "play";
	if (btn.dataset.icon !== want) {
		btn.dataset.icon = want;
		btn.innerHTML = ICON[want];
	}
}

// long text scrolls back and forth instead of truncating
export function setMarquee(box, text) {
	if (box.dataset.text === text) return;
	box.dataset.text = text;
	box.classList.remove("marquee", "run");
	box.replaceChildren(document.createTextNode(text));
	requestAnimationFrame(() => {
		const over = box.scrollWidth - box.clientWidth;
		if (over > 4) {
			const span = document.createElement("span");
			span.textContent = text;
			box.replaceChildren(span);
			span.style.setProperty(
				"--dist",
				`${-(over + 8)}px`,
			);
			box.classList.add("marquee", "run");
		}
	});
}

// ---- the rings at the top right of the hub: a plugin may add its own (see `addRing`)
export const ringEls = {};

/** an element that is a ring (the static ones in the page, and those a plugin adds) */
export function ringFrom(el) {
	const key = el.id.replace("ring-", "");
	const color = el.dataset.color;
	el.innerHTML =
		`<svg viewBox="0 0 20 20">` +
		`<circle class="ring-bg" cx="10" cy="10" r="${RING_R}"></circle>` +
		`<circle class="ring-fg" cx="10" cy="10" r="${RING_R}" stroke="${color}" stroke-dasharray="0 ${RING_C}"></circle>` +
		`</svg>`;
	ringEls[key] = { el, fg: el.querySelector(".ring-fg") };
	// rings sit on the click-toggles-hub pill; don't let a ring click close it
	el.addEventListener("mousedown", (e) => e.stopPropagation());
	return ringEls[key];
}

/** a ring of the plugin's own, at the left of the others; hidden until it has something to say (`setRing`, then show) */
export function addRing(key, color) {
	const el = document.createElement("span");
	el.className = "ring-wrap hidden";
	el.id = `ring-${key}`;
	el.dataset.color = color;
	document.getElementById("hub-rings").prepend(el);
	return ringFrom(el);
}

export function setRing(key, pct, tooltip) {
	const ring = ringEls[key];
	if (!ring) return;
	const clamped = Math.max(0, Math.min(100, pct ?? 0));
	const len = (clamped / 100) * RING_C;
	ring.fg.setAttribute("stroke-dasharray", `${len} ${RING_C}`);
	ring.el.dataset.tip = tooltip;
	host.refreshTip(ring.el); // live-update if it is showing right now
}

// ---- a Claude Code session's look: its tile, the ring of its context, the icon of its state. The plugin's card and the
// session pill (a banner kind of the island) draw the same.
export const LLM_COLOR = { claude: "#e08a5a" };
export const LLM_LETTER = { claude: "C" };
export const LLM_RING_C = 2 * Math.PI * 18;
export const llmCtxColor = (p) =>
	p > 0.85 ? "#e87a7a" : p > 0.65 ? "#e8b04a" : "#5ac88c";
export const LLM_STATE_TIP = {
	waiting: "Waiting for you",
	finished: "Finished",
	working: "Working",
	idle: "Idle",
};
// the state as one icon: a ringing phone, a check, a spinner, a moon
export function llmStateIcon(state) {
	const box = el("span", `llm-ico ${state}`);
	box.dataset.tip = LLM_STATE_TIP[state] || state;
	if (state === "working") box.append(el("span", "llm-spin"));
	else
		box.innerHTML = li(
			{
				waiting: "phone",
				finished: "check",
				idle: "moon",
			}[state] || "moon",
		);
	return box;
}
