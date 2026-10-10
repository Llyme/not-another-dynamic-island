// Now playing, the face of the native plugin (src/builtin/now_playing.rs): the pill and the hub's card for what
// is playing, with its controls. What is playing comes from the media sensor (media-tick), which also moves the eyes.
import { FLOATS, ICON, addPill, call, collapsible, el, focusButton, host, invoke, listen, mediaListOf, pickPrimaryMedia, setMarquee, setPlayIcon } from "../kit.js";

// the jump buttons: a circular arrow with the number inside (the forward one is the mirror image)
const JUMP_SVG = (flip) =>
	`<svg viewBox="0 0 16 16"><g${flip ? ' transform="translate(16 0) scale(-1 1)"' : ""}><path d="M3.2 8a4.8 4.8 0 1 1 1.4 3.4M3 4v3.2h3.2"/></g><text x="8" y="10.2" text-anchor="middle">5</text></svg>`;
// speed: a dropdown of fixed steps from 0.1x to 16x
const RATES = [
	0.1, 0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 3, 4, 8, 16,
];
const rateText = (r) => `${Number(r.toFixed(2))}×`;
// one entry per live media card (built to one shape: classes, never ids)
const hubMediaCards = [];

function wrapHubMediaCard(root, index) {
	const q = (id, cls) =>
		root.querySelector(`#${CSS.escape(id)}, .${cls}`) ||
		root.querySelector(`.${cls}`);
	const card = {
		root,
		art: q("hub-media-art", "hub-media-art"),
		title: q("hub-media-title", "hub-media-title"),
		artist: q("hub-media-artist", "hub-media-artist"),
		play: q("hub-media-play", "hub-media-play"),
		prev: q("hub-media-prev", "hub-media-prev"),
		next: q("hub-media-next", "hub-media-next"),
		seek: q("hub-media-seek", "hub-media-seek"),
		seekFill: q("hub-media-seek-fill", "hub-media-seek-fill"),
		back: q("hub-media-back", "hub-media-back"),
		fwd: q("hub-media-fwd", "hub-media-fwd"),
		rate: q("hub-media-rate", "hub-media-rate"),
		rateLabel: q("hub-media-rate-label", "hub-media-rate-label"),
		rates: q("hub-media-rates", "hub-media-rates"),
		extra: q("hub-media-extra", "hub-media-extra"),
		source: "",
		titleText: "",
		duration: 0,
		rateNow: 1,
	};
	wireHubMediaCard(card, index);
	hubMediaCards[index] = card;
	return card;
}

function buildHubMediaCard(index) {
	const root = document.createElement("div");
	root.className = "now-card spot hub-media";
	root.innerHTML =
		`<div class="now-row">` +
		`<img class="now-icon hub-media-art" alt="" />` +
		`<div class="now-text"><div class="now-title hub-media-title"></div>` +
		`<div class="now-sub hub-media-artist"></div></div>` +
		`<div class="now-controls">` +
		`<button class="hub-media-prev" data-tip="Previous"></button>` +
		`<button class="hub-media-play" data-tip="Play / Pause"></button>` +
		`<button class="hub-media-next" data-tip="Next"></button>` +
		`</div></div>` +
		`<div class="media-extra hub-media-extra">` +
		`<button class="media-jump hub-media-back" data-tip="Back 5 seconds"></button>` +
		`<div class="now-seek hub-media-seek hidden"><div class="hub-media-seek-fill"></div></div>` +
		`<button class="media-jump hub-media-fwd" data-tip="Forward 5 seconds"></button>` +
		`<button class="media-rate hub-media-rate" data-tip="Playback speed">` +
		`<span class="hub-media-rate-label">1×</span></button>` +
		`</div><div class="rate-menu hub-media-rates hidden"></div>`;
	// (they lead the hub's list of cards)
	if (hubMediaCards.length > 0) hubMediaCards[hubMediaCards.length - 1].root.after(root);
	else document.getElementById("now-cards").before(root);
	return wrapHubMediaCard(root, index);
}

function wireHubMediaCard(card, index) {
	card.prev.innerHTML = ICON.prev;
	card.next.innerHTML = ICON.next;
	card.back.innerHTML = JUMP_SVG(false);
	card.fwd.innerHTML = JUMP_SVG(true);
	for (const r of RATES) {
		const o = el("button", "rate-opt", rateText(r));
		o.dataset.rate = String(r);
		o.addEventListener("click", async () => {
			card.rates.classList.add("hidden");
			card.rateLabel.textContent = rateText(r);
			host.scheduleHubHeight();
			await call("now-playing", "set_rate", {
				rate: r,
				source: card.source,
			}); // the next update shows the speed the player took
		});
		card.rates.append(o);
	}
	for (const btn of [
		card.play,
		card.prev,
		card.next,
		card.seek,
		card.back,
		card.fwd,
		card.rate,
		card.rates,
	]) {
		btn.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
	}
	// the art is the button that jumps to whoever is playing
	collapsible(card.root, `media:${index}`);
	card.root.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
	const focus = () =>
		invoke("focus_source", {
			titleHint: card.titleText,
			sourceId: card.source,
		});
	focusButton(card.root, focus);
	card.play.addEventListener("click", () =>
		call("now-playing", "play_pause", { source: card.source }),
	);
	card.prev.addEventListener("click", () =>
		call("now-playing", "previous", { source: card.source }),
	);
	card.next.addEventListener("click", () =>
		call("now-playing", "next", { source: card.source }),
	);
	card.back.addEventListener("click", () =>
		call("now-playing", "seek_by", {
			delta_seconds: -5,
			source: card.source,
		}),
	);
	card.fwd.addEventListener("click", () =>
		call("now-playing", "seek_by", {
			delta_seconds: 5,
			source: card.source,
		}),
	);
	card.rate.addEventListener("click", () => {
		card.rates.classList.toggle("hidden");
		host.scheduleHubHeight();
	});
	card.seek.addEventListener("click", (e) => {
		if (!(card.duration > 0)) return;
		const rect = card.seek.getBoundingClientRect();
		const frac = Math.max(
			0,
			Math.min(1, (e.clientX - rect.left) / rect.width),
		);
		call("now-playing", "seek", {
			position_seconds: frac * card.duration,
			source: card.source,
		});
		card.seekFill.style.width = `${frac * 100}%`;
	});
}

function applyHubMediaCard(card, m) {
	card.title.textContent = m.title || "";
	card.source = m.source || "";
	card.titleText = m.title || "";
	card.artist.textContent = m.artist || "";
	if (m.art) card.art.src = m.art;
	card.art.classList.toggle("hidden", !m.art);
	setPlayIcon(card.play, m.playing);
	card.prev.classList.toggle("hidden", !m.can_previous);
	card.next.classList.toggle("hidden", !m.can_next);
	// live streams report no duration -- nothing to seek in, so no bar
	card.duration = m.duration || 0;
	card.seek.classList.toggle(
		"hidden",
		!(card.duration > 0),
	);
	// jump buttons need a player that takes seeks; the speed slider one that takes a speed
	card.back.classList.toggle("hidden", !m.can_seek);
	card.fwd.classList.toggle("hidden", !m.can_seek);
	// the speed dropdown only exists for a player that takes a speed
	card.rate.classList.toggle("hidden", !m.can_rate);
	if (!m.can_rate) card.rates.classList.add("hidden");
	card.rateNow = m.rate || 1;
	card.rateLabel.textContent = rateText(card.rateNow);
	for (const o of card.rates.children)
		o.classList.toggle(
			"on",
			Math.abs(Number(o.dataset.rate) - card.rateNow) <
				0.01,
		);
	// nothing to show in the row (no jumps, no speed, no seek bar): the row goes
	card.extra.classList.toggle(
		"hidden",
		!(m.can_seek || m.can_rate || card.duration > 0),
	);
	if (card.duration > 0) {
		card.seekFill.style.width = `${Math.min(100, ((m.position || 0) / card.duration) * 100)}%`;
	}
	card.root._syncBody?.(); // nothing to open to (no seek bar, jumps or speed): not expandable
}

function applyHubMediaList(list) {
	const sessions = Array.isArray(list)
		? list.filter((m) => m && m.has_session)
		: mediaListOf(list);
	// one card per session; surplus cards leave the DOM
	while (hubMediaCards.length < sessions.length)
		buildHubMediaCard(hubMediaCards.length);
	while (hubMediaCards.length > sessions.length) hubMediaCards.pop().root.remove();
	sessions.forEach((m, i) => {
		const card = hubMediaCards[i];
		card.root.classList.remove("hidden");
		applyHubMediaCard(card, m);
	});
}

// the pill: the player's art, the title and the artist, and the controls
function initPill() {
	addPill(
		"media",
		`
<img id="media-art" alt="" />
<div class="text-col">
  <div id="media-title" class="line-main"></div>
  <div id="media-artist" class="line-sub"></div>
</div>
<div id="media-controls">
  <button id="media-prev"></button>
  <button id="media-play"></button>
  <button id="media-next"></button>
</div>
<div id="media-progress" class="hidden"><div></div></div>
	`,
	);
	const mediaArt = document.getElementById("media-art");
	const mediaTitle = document.getElementById("media-title");
	const mediaArtist = document.getElementById("media-artist");
	const mediaPlay = document.getElementById("media-play");
	const mediaPrev = document.getElementById("media-prev");
	const mediaNext = document.getElementById("media-next");

	mediaPrev.innerHTML = ICON.prev;
	mediaNext.innerHTML = ICON.next;
	const mediaProgress = document.getElementById("media-progress");
	const mediaProgressFill = mediaProgress.firstElementChild;

	let pillMediaSource = "";

	listen("media-tick", (event) => {
		const list = mediaListOf(event.payload);
		const m = pickPrimaryMedia(list);
		// the pill follows one session; the hub shows them all
		pillMediaSource = m ? m.source || "" : "";
		if (!m) return;

		setMarquee(mediaTitle, m.title || "");
		mediaArtist.textContent = m.artist || "";
		if (m.art) mediaArt.src = m.art;
		mediaArt.classList.toggle("hidden", !m.art);
		setPlayIcon(mediaPlay, m.playing);
		mediaPrev.classList.toggle("hidden", !m.can_previous);
		mediaNext.classList.toggle("hidden", !m.can_next);
		// live streams report no duration -- no progress line then
		const hasProgress = m.duration > 0;
		mediaProgress.classList.toggle("hidden", !hasProgress);
		if (hasProgress) {
			mediaProgressFill.style.width = `${Math.min(100, (m.position / m.duration) * 100)}%`;
		}
	});

	for (const btn of [mediaPlay, mediaPrev, mediaNext]) {
		// stop the pill's own mousedown handler from starting a native window
		// drag out from under the click
		btn.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
	}
	mediaPlay.addEventListener("click", () =>
		call("now-playing", "play_pause", { source: pillMediaSource }),
	);
	mediaPrev.addEventListener("click", () =>
		call("now-playing", "previous", { source: pillMediaSource }),
	);
	mediaNext.addEventListener("click", () =>
		call("now-playing", "next", { source: pillMediaSource }),
	);
}

export default {
	id: "now-playing",
	init() {
		if (FLOATS) return;
		initPill();
		listen("media-tick", (e) => applyHubMediaList(mediaListOf(e.payload)));
	},
	// (its card is not in the list of the others: it leads the hub, see applyHubMediaList)
	cards() {
		return [];
	},
};
