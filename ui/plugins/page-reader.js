// Page Reader, the face of the native plugin (src/builtin/pages): a card for each page you have open, and the pill
// that names the page in front of you.
import { FLOATS, KIND_GLYPH, addPill, call, collapsible, el, focusable, going, host, iconEl, invoke, li, line, listen, peek, textCol, formatDuration } from "../kit.js";
import { guideReader } from "./guide.js";

// the guide reader asks for words and pictures by the island's old names: they are calls to this plugin
const GUIDE_CALLS = { page_guide: "guide", page_guide_image: "guide_image", guide_go: "go" };
function guideInvoke(cmd, args) {
	const to = GUIDE_CALLS[cmd];
	if (!to) return invoke(cmd, args);
	return call("page-reader", to, cmd === "guide_go" ? { exe_path: args.exePath, page: args.page, url: args.url } : args);
}

// A shortcut to a button of the page in front of you: the click is delivered to
// the browser window, so it works without leaving the island.
function pageButton(b) {
	const btn = el("button", "page-btn", b.label);
	const stop = (e) => e.stopPropagation();
	btn.addEventListener("mousedown", stop);
	btn.addEventListener("click", async (e) => {
		e.stopPropagation(); // not "go to the browser"
		btn.disabled = true;
		const ok = await call("page-reader", "press", { label: b.label });
		btn.classList.add(ok ? "done" : "fail");
		setTimeout(
			() => btn.classList.remove("done", "fail"),
			900,
		);
		btn.disabled = false;
	});
	return btn;
}

// The bytes of a page's picture come once per picture (the card only holds its id); the hub keeps the last few.
const pageImages = new Map();
async function showPageImage(img, id) {
	let src = pageImages.get(id);
	if (src === undefined) {
		try {
			src = (await call("page-reader", "image", { id })) || null;
		} catch (_) {
			src = null;
		}
		pageImages.set(id, src);
		if (pageImages.size > 12)
			pageImages.delete(pageImages.keys().next().value);
	}
	// (the card may have moved on to another picture while this was fetched)
	if (img.dataset.id !== String(id)) return;
	if (src) {
		img.src = src;
		img.classList.add("ready");
	} else {
		img.classList.remove("ready");
	}
	host.scheduleHubHeight();
}

// ---- a post and its comments, read-only (see pagekind::Thread) ----
function threadHead(t) {
	const frag = document.createDocumentFragment();
	const by = el("div", "th-by");
	if (t.who) by.append(el("span", "th-who", t.who));
	if (t.handle) by.append(el("span", "th-dim", t.handle));
	if (t.ago) by.append(el("span", "th-dim", "\u00b7"), el("span", "th-dim", t.ago));
	if (t.chip) by.append(el("span", "th-chip", t.chip));
	if (by.childElementCount) frag.append(by);
	if (t.lead) frag.append(el("div", "th-lead", t.lead));
	return frag;
}

function threadTail(t) {
	const frag = document.createDocumentFragment();
	if (t.stats.length) {
		const row = el("div", "th-stats");
		for (const s of t.stats) {
			const n = el("span", "th-stat");
			n.innerHTML = li(s.icon);
			n.append(el("b", null, s.value));
			row.append(n);
		}
		frag.append(row);
	}
	if (t.comments.length) {
		const head = el("div", "th-head");
		head.append(el("span", "th-h", "Comments"));
		if (t.sort) head.append(el("span", "th-dim", t.sort));
		frag.append(head);
		const list = el("div", "th-list");
		for (const c of t.comments) {
			const n = el("div", "th-c");
			n.dataset.d = String(c.depth);
			n.style.setProperty("--d", String(c.depth));
			const m = el("div", "th-m");
			m.append(el("span", "th-a", c.who));
			if (c.op) m.append(el("span", "th-op", "OP"));
			if (c.score) {
				const sc = el("span", "th-sc");
				sc.innerHTML = li(c.heart ? "heart" : "up");
				sc.append(document.createTextNode(c.score));
				m.append(sc);
			}
			if (c.ago) m.append(el("span", null, c.ago));
			n.append(m, el("div", "th-tx", c.text));
			list.append(n);
		}
		frag.append(list);
		if (t.total && t.total > t.comments.length)
			frag.append(el("div", "th-fold", `${t.comments.length} of ${t.total} comments loaded on the page`));
	}
	return frag;
}

// what makes the thread a different card: its words and numbers (not the time that passes)
function threadSig(t) {
	return [
		t.who,
		t.lead,
		t.stats.map((s) => s.value).join(","),
		t.comments.map((c) => `${c.depth}${c.who}${c.text.length}${c.score}`).join(";"),
	].join("|");
}

// the kind's own summary line is a better lead than the page's first paragraph
function hasSummaryOf(b) {
	return (
		!!b.kind &&
		b.kind.fields.some(
			(f) => f.key === "Summary" || f.key === "Gist",
		)
	);
}

// Browsing: the page in front of you and what is in it, read from the page. When the island knows what kind
// of page it is (a walkthrough, an article, a video...) the card lists what is worth knowing about that kind.
function browsingCard(w) {
	const b = w.browse;
	const k = b.kind;
	const pv = b.preview;
	const card = el("div", "now-card spot");
	const row = el("div", "now-row");
	if (k) {
		const tile = el("div", "now-icon tile");
		tile.innerHTML = li(KIND_GLYPH[k.id] || "globe");
		row.append(tile);
	} else {
		row.append(iconEl(w.icon, "globe"));
	}
	// the two lines that change as you read: the sub-line, and what the collapsed card says at its right
	const subOf = (w) => {
		const b = w.browse;
		const k = b.kind;
		return k
			? b.domain
				? `${k.label} \u00b7 ${b.domain}`
				: k.label
			: b.domain || going(w.going_secs);
	};
	const peekOf = (w) => {
		const b = w.browse;
		const k = b.kind;
		if (k && k.peek) return k.peek;
		return b.domain || going(w.going_secs);
	};
	row.append(
		textCol(w.page || w.app_name || "Browsing", subOf(w)),
	);
	card.append(row);

	const body = el("div", "work-body");
	const th = k && k.thread;
	const guide = k && k.guide;
	// one card per recently focused page: per-page key keeps expand state separate,
	// titleHint focuses the right browser window (chrome vs edge, window vs window)
	const pageKey = `web:${w.exe_path || ""}:${w.page || ""}`;
	if (th) body.append(threadHead(th));
	let pic = null;
	if (b.image != null) {
		// the page's main picture: fetched once per picture (the id changes only when the picture does)
		pic = el("img", "pk-img");
		pic.alt = "";
		pic.draggable = false;
		pic.dataset.id = String(b.image);
		showPageImage(pic, b.image);
		// (a guide places it itself: see guide.js)
		if (!guide) body.append(pic);
	}
	if (th) body.append(threadTail(th));
	// a guide: laid out to be read, with the sections around it (see guide.js)
	if (guide)
		body.append(
			guideReader({
				el,
				li,
				invoke: guideInvoke,
				key: guide.key,
				exe: w.exe_path,
				page: w.page,
				pageKey,
				image: pic,
				onSize: host.scheduleHubHeight,
				onMoved: () => host.refresh(),
				onTyping: FLOATS ? (on) => invoke("float_typing", { on }) : null,
			}),
		);
	if (k && k.fields.length && !th && !guide) {
		const list = el("div", "pk-fields");
		for (const f of k.fields) {
			const r = el("div", "pk-row");
			const v = el(
				"span",
				f.rough ? "pk-v rough" : "pk-v",
				f.value,
			);
			r.append(el("span", "pk-k", f.key), v);
			list.append(r);
		}
		body.append(list);
	}
	if (pv && !guide) {
		// (the kind's own summary line is the better lead)
		if (pv.lead && !hasSummaryOf(b) && !th)
			body.append(el("div", "work-desc", pv.lead));
		if (pv.buttons.length) {
			const btns = el("div", "page-btns");
			for (const b of pv.buttons)
				btns.append(pageButton(b));
			body.append(btns);
		}
	}
	if (body.childElementCount) card.append(body);
	// wide enough, the picture moves to the side (see .wide in hub.css)
	if (b.image != null)
		new ResizeObserver(() =>
			card.classList.toggle("wide", card.clientWidth >= 520),
		).observe(card);
	const peekText = peekOf(w);
	// Rebuilt only when the card's shape changes (another kind, other fields, other buttons); the values that
	// move while you read (scroll, the video's clock) are written in place, so an open card does not replay
	// its entrance animation every time you scroll.
	card._sig = `${pageKey}:${b.image != null ? "img" : ""}:${guide ? guide.key : ""}:${th ? threadSig(th) : ""}:${k ? `${k.id}:${k.fields.map((f) => f.key).join("|")}` : ""}:${pv && pv.lead && !hasSummaryOf(b) ? "lead" : ""}:${pv ? pv.buttons.map((x) => x.label).join("|") : ""}`;
	card._data = w;
	card._update = (w) => {
		const b = w.browse;
		const k = b.kind;
		const sub = card.querySelector(".now-sub");
		const nextSub = line("now-sub", subOf(w));
		if (sub && sub.textContent !== nextSub.textContent) sub.replaceWith(nextSub);
		const pk = card.querySelector(".now-peek");
		const nextPeek = line("now-peek", peekOf(w));
		if (pk && pk.textContent !== nextPeek.textContent) pk.replaceWith(nextPeek);
		if (k) {
			const vals = card.querySelectorAll(".pk-v");
			k.fields.forEach((f, i) => {
				const v = vals[i];
				if (!v) return;
				if (v.textContent !== f.value) v.textContent = f.value;
				v.classList.toggle("rough", !!f.rough);
			});
		}
		const pic = card.querySelector(".pk-img");
		if (pic && b.image != null && pic.dataset.id !== String(b.image)) {
			pic.dataset.id = String(b.image);
			showPageImage(pic, b.image);
		}
		const lead = card.querySelector(".work-desc");
		if (lead && b.preview && lead.textContent !== b.preview.lead)
			lead.textContent = b.preview.lead;
	};
	return focusable(
		collapsible(peek(card, peekText), pageKey),
		{ exePath: w.exe_path, titleHint: w.page },
	);
}

// the pill: the page in front of you, named (the step of a walkthrough, the headline, the time left)
function initPill() {
	addPill(
		"page",
		`
		<div id="page-icon"></div>
		<div class="text-col">
			<div id="page-label" class="line-main"></div>
			<div id="page-app" class="line-sub"></div>
		</div>
		<span id="page-time"></span>
	`,
	);
	const icon = document.getElementById("page-icon");
	const label = document.getElementById("page-label");
	const app = document.getElementById("page-app");
	const time = document.getElementById("page-time");
	listen("page-tick", (event) => {
		const p = event.payload;
		icon.innerHTML = li(KIND_GLYPH[p.kind] || "globe");
		label.textContent = p.main || "";
		app.textContent = p.sub || "";
		time.textContent = formatDuration(p.since_secs);
	});
}

export default {
	id: "page-reader",
	init() {
		if (!FLOATS) initPill();
	},
	cards(data) {
		return data.map((w) => {
			const c = browsingCard(w);
			c._rank = 60;
			return c;
		});
	},
};
