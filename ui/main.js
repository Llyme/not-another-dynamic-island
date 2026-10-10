// Canvas-painted idle "eyes" -- a straight port of `_paint_eyes`/`_tick_eyes`
// from the Qt prototype (main.py), driven by the Rust-side `cursor-tick`
// event instead of a QTimer polling QCursor.pos(). No framework, no bundler:
// this is the whole idle-state UI, kept as cheap as the old QPainter path.

import { initFloats } from "./floats.js";
import { initPluginUi, pluginCard, setRings, whenText } from "./pluginui.js";
import { plugins } from "./plugins/index.js";
import { eyes } from "./plugins/eyes.js";
import { soundLight } from "./plugins/sound-light.js";
import { updateAudio } from "./sound.js";
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
// (the eyes are in plugins/eyes.js, the sound light in plugins/sound-light.js, and the sound they borrow in sound.js)



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


let lastFrameT = performance.now();
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
		soundLight.frame(now, dt);
		eyes.frame(now);
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
		eyes.clearFace();
	}
	if (currentView === "hub") {
		// hub-eyes was `display:none` (0x0) at the one point page-load sizing
		// ran, and toggling a CSS class doesn't fire a `resize` event to catch
		// it later -- size it explicitly now that it's actually visible
		eyes.sizeHub();
		eyes.mood.happyUntil = performance.now() + 1600; // happy to see you
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
		soundLight.setCursorPad(pad);
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
	const { dx, dy, speed } = eyes.cursor(local_x, local_y);
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
	eyes.track(
		local_x,
		local_y,
		dx,
		dy,
		speed,
		!!event.payload.shown &&
			local_x >= cursorPad &&
			local_x <= window.innerWidth - cursorPad &&
			local_y >= cursorPad &&
			local_y <= window.innerHeight - cursorPad,
	);
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
	eyes.mood.happyUntil = performance.now() + 900; // a little happy blip either way
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
	eyes.mood.wideUntil = performance.now() + 320; // boop!
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
	applyAccent(currentSettings.accent_color);
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
	};
	currentSettings.accent_color = accentColor;
	updateClock();
	invoke("save_settings", { settings: currentSettings });
}

for (const el of [
	setStartWithWindows,
	setShowAtCursor,
	setCursorFollow,
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
		eyes.mood.happyUntil = performance.now() + 700;
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
			if (s.type === "range") {
				// a slider: the value follows the thumb at once (the plugin may show it live), and is kept as it is let go
				const wrap = el("span", "pg-range");
				const r = document.createElement("input");
				r.type = "range";
				r.min = s.min ?? 0;
				r.max = s.max ?? 100;
				r.step = s.step ?? 1;
				r.value = s.value ?? s.min ?? 0;
				const val = el("span", "set-val");
				const paint = () => {
					val.textContent = `${r.value}${s.unit || ""}`;
					const span = Number(r.max) - Number(r.min) || 1;
					r.style.setProperty("--p", `${(((Number(r.value) - Number(r.min)) / span) * 100).toFixed(1)}%`);
				};
				paint();
				let sentAt = 0;
				r.addEventListener("input", () => {
					paint();
					const now = performance.now();
					if (now - sentAt > 80) {
						sentAt = now;
						invoke("plugin_value", { id: p.id, key: s.key, value: Number(r.value) });
					}
				});
				r.addEventListener("change", () => invoke("plugin_value", { id: p.id, key: s.key, value: Number(r.value) }));
				wrap.append(r, val);
				f.append(wrap);
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
