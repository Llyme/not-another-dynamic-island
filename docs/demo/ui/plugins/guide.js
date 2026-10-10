// A walkthrough or a FAQ laid out to be read in a card, so a guide can sit over the game while you play.
// The words come from the browser extension (see ext::Guide): headings, paragraphs, lists, tables and the
// sections around the one on screen. This file only draws them, and asks for another section when you press
// Previous, Next or an entry of the contents. Scrolling in the browser is not followed; the one thing it remembers
// is the size you like the text.

const SIZES = { S: "12.5px", M: "13.5px", L: "15px", XL: "17px" };

const store = {
	get(k, d) {
		try {
			const v = localStorage.getItem(k);
			return v == null ? d : v;
		} catch (_) {
			return d;
		}
	},
	set(k, v) {
		try {
			localStorage.setItem(k, v);
		} catch (_) {}
	},
};

// what each guide card keeps while the card itself is rebuilt (the hub builds its cards anew when any of them changes)
const states = new Map(); // page key -> { key, q, hit, top, applied, raw, toc }
const guides = new Map(); // guide key -> guide (the last few)

function stateOf(pageKey) {
	let s = states.get(pageKey);
	if (!s) {
		s = { key: "", q: "", hit: 0, top: 0, applied: false, raw: false, toc: false, busy: false, note: "", layout: "stack", more: false };
		states.set(pageKey, s);
		if (states.size > 24) states.delete(states.keys().next().value);
	}
	return s;
}

// ---- the pictures inside a guide (a table's icons, a figure). The text names each by a marker ( number.WxH
// ); the picture itself comes when it is near the screen, one at a time and a few at once at most, from the
// browser extension (see ext::guide_picture). A picture that came is kept, so a card that is drawn again has it at once.
const MARKER = /(\d+)\.(\d+)x(\d+)/g;
const pictures = new Map(); // guide key, number and size -> a data: address, or "" when it could not be had
const lineUp = []; // [img, fetch] waiting for a turn
// the pictures that come into view are asked for (the reader scrolls inside the card, and that clips what is seen)
const seen = new IntersectionObserver(
	(list) => {
		for (const e of list) {
			if (!e.isIntersecting || !e.target._want) continue;
			seen.unobserve(e.target);
			e.target._want();
			e.target._want = null;
		}
	},
	{ rootMargin: "120px" },
);
let inFlight = 0;
const AT_ONCE = 3;

function nextPicture() {
	while (inFlight < AT_ONCE && lineUp.length) {
		const [img, run] = lineUp.shift();
		if (!img.isConnected) continue; // (the card was drawn again, or closed)
		inFlight++;
		run().finally(() => {
			inFlight--;
			nextPicture();
		});
	}
}

async function load(invoke, key) {
	if (guides.has(key)) return guides.get(key);
	let g = null;
	try {
		g = (await invoke("page_guide", { key })) || null;
	} catch (_) {}
	if (g) {
		guides.set(key, g);
		if (guides.size > 8) guides.delete(guides.keys().next().value);
	}
	return g;
}

/** The guide of a card: a box that fills itself with the words once they are here. */
export function guideReader({ el, li, invoke, key, exe, page, pageKey, image, onSize, onMoved, onTyping }) {
	const root = el("div", "gd");
	const st = stateOf(pageKey);
	let current = null; // the guide on screen
	const size = () => (SIZES[store.get("nadi.guide.size", "M")] ? store.get("nadi.guide.size", "M") : "M");

	const draw = (g) => {
		if (st.key !== g.key) {
			// another section: from its top
			Object.assign(st, { key: g.key, q: "", hit: 0, top: 0, applied: false, toc: false, busy: false, note: "", more: false });
		}
		current = g;
		root.replaceChildren(...build(g));
		onSize && onSize();
	};

	// The layout follows the card's width: narrow, the picture and the facts sit above the text; a little wider,
	// a small portrait with the facts that matter; wide, a rail on the side while the text scrolls beside it.
	new ResizeObserver(() => {
		const w = (root.closest(".now-card") || root).clientWidth;
		if (!w) return; // (a collapsed card has no size)
		const l = w < 480 ? "stack" : w < 640 ? "strip" : "rail";
		if (l !== st.layout) {
			st.layout = l;
			if (current) draw(current);
		}
	}).observe(root);

	const cached = guides.get(key);
	if (cached) draw(cached);
	else {
		root.append(el("div", "gd-load", "Reading the guide…"));
		load(invoke, key).then((g) => {
			if (g) draw(g);
			else root.replaceChildren(el("div", "gd-load", "The guide is not available any more."));
		});
	}

	// ---- the pieces
	// label and value pairs, one to a row
	function factsList(pairs) {
		const box = el("div", "gd-facts");
		for (const [k, v] of pairs) {
			const f = el("div", "gd-fact");
			f.append(el("span", null, k), el("b", null, v));
			box.append(f);
		}
		return box;
	}

	// a picture's place in the text: its box has its size already, and it is filled in when it is near
	function picture(no, w, h) {
		const key = current ? current.key : "";
		// (a few sizes only, so the same picture at 60 and at 64 px is fetched once)
		const max = w ? [64, 128, 256, 480].find((b) => b >= w * 2) || 480 : 256;
		const id = `${key}:${no}:${max}`;
		const img = el("img", "gd-pic");
		img.alt = "";
		img.draggable = false;
		if (w && h) {
			img.style.width = `${w}px`;
			img.style.aspectRatio = `${w} / ${h}`;
		}
		const show = (data) => {
			if (!data) {
				img.classList.add("failed");
				return;
			}
			img.onload = () => {
				img.classList.add("ready");
				if (!(w && h)) onSize && onSize();
			};
			img.src = data;
		};
		if (pictures.has(id)) show(pictures.get(id));
		else {
			img._want = () => {
				lineUp.push([
					img,
					async () => {
						let data = "";
						try {
							data = (await invoke("page_guide_image", { key, no, max })) || "";
						} catch (_) {}
						pictures.set(id, data);
						if (pictures.size > 400) pictures.delete(pictures.keys().next().value);
						show(data);
					},
				]);
				nextPicture();
			};
			seen.observe(img);
		}
		return img;
	}

	function rich(text) {
		const frag = document.createDocumentFragment();
		// (a limit may have cut a marker short)
		const clean = String(text).replace(/[^]*$/, "");
		let last = 0;
		const words = (t) =>
			t.split(/\*\*(.+?)\*\*/g).forEach((part, i) => {
				if (!part) return;
				if (i % 2) frag.append(el("b", null, part));
				else frag.append(document.createTextNode(part));
			});
		for (const m of clean.matchAll(MARKER)) {
			words(clean.slice(last, m.index));
			frag.append(picture(Number(m[1]), Number(m[2]), Number(m[3])));
			last = m.index + m[0].length;
		}
		words(clean.slice(last));
		return frag;
	}

	// a line that is only pictures is a row of them
	const onlyPictures = (t) => /\d+\.\d+x\d+/.test(t) && t.replace(MARKER, "").trim() === "";

	function build(g) {
		const out = [];
		const heads = g.blocks.filter((b) => b[0] === "h");
		const levels = [...new Set(heads.map((b) => b[1]))].sort((a, b) => a - b);
		const count = (l) => heads.filter((b) => b[1] === l).length;
		const dayLevel = levels.find((l) => count(l) >= 3) ?? null;
		const days = []; // { text, node }

		// ---- the tools: search, the size of the text, the contents, the original text
		const tools = el("div", "gd-tools");
		tools.classList.toggle("has-q", !!st.q.trim());
		const search = el("div", "gd-search");
		search.innerHTML = li("search");
		const input = el("input");
		input.type = "search";
		input.placeholder = "Search this section";
		input.autocomplete = "off";
		input.value = st.q;
		const n = el("span", "gd-n");
		search.append(input, n);
		tools.append(search);
		// one button for the text size: it shows the size, and each press goes on to the next (S, M, L, XL, and S again)
		const ORDER = Object.keys(SIZES);
		const sizeBtn = el("button", "gd-ibtn", size());
		sizeBtn.type = "button";
		const sizeTip = () => {
			sizeBtn.title = `Text size ${size()} (press for ${ORDER[(ORDER.indexOf(size()) + 1) % ORDER.length]})`;
		};
		sizeTip();
		sizeBtn.addEventListener("click", () => {
			const next = ORDER[(ORDER.indexOf(size()) + 1) % ORDER.length];
			store.set("nadi.guide.size", next);
			root.style.setProperty("--read", SIZES[next]);
			sizeBtn.textContent = next;
			sizeTip();
			onSize && onSize();
		});
		tools.append(sizeBtn);
		if (g.text && g.raw) {
			const b = el("button", "gd-ibtn wide", st.raw ? "Reader" : "Original");
			b.type = "button";
			b.title = st.raw ? "Laid out for reading" : "The text exactly as the page has it";
			b.addEventListener("click", () => {
				st.raw = !st.raw;
				st.top = 0;
				st.applied = false;
				draw(g);
			});
			tools.append(b);
		}
		if (g.toc.length) {
			const b = el("button", "gd-ibtn");
			b.type = "button";
			b.innerHTML = li("steps");
			b.title = "Contents of the guide";
			b.classList.toggle("on", st.toc);
			b.addEventListener("click", () => {
				st.toc = !st.toc;
				draw(g);
			});
			tools.append(b);
		}
		out.push(tools);

		// ---- the contents: every section of the guide, one press away
		if (st.toc && g.toc.length) {
			const list = el("div", "gd-toc");
			for (const t of g.toc) {
				const b = el("button", t.u === g.url ? "gd-toc-i cur" : "gd-toc-i", t.t);
				b.type = "button";
				b.style.setProperty("--d", String(t.d || 0));
				b.addEventListener("click", () => go(t.u, g));
				list.append(b);
			}
			out.push(list);
		}

		// ---- the reader
		const reader = el("div", "gd-reader");
		// a wiki's infobox is not part of the text: it goes beside the picture (see the layouts below)
		const factsBlock = g.blocks.find((b) => b[0] === "facts");
		const profile = !!(image || factsBlock) && !st.raw;
		if (st.raw && g.raw) {
			reader.append(el("pre", "gd-pre", g.raw));
		} else {
			for (const b of g.blocks) {
				if (b === factsBlock) continue;
				const kind = b[0];
				if (kind === "h") {
					const lvl = b[1];
					const cls = dayLevel != null && lvl === dayLevel ? "gd-day" : dayLevel != null ? (lvl < dayLevel ? "gd-t" : "gd-sub") : lvl === levels[0] ? "gd-t" : "gd-sub";
					const h = el("div", `gd-h ${cls}`);
					h.append(el("span", null, b[2]));
					if (cls === "gd-day") days.push({ text: b[2], node: h });
					reader.append(h);
				} else if (kind === "p") {
					const p = el("p", "gd-p");
					if (onlyPictures(b[1])) p.classList.add("pics");
					p.append(rich(b[1]));
					reader.append(p);
				} else if (kind === "ul" || kind === "ol") {
					const ul = el(kind, "gd-l");
					for (const it of b[1]) {
						const l = el("li");
						l.append(rich(it));
						ul.append(l);
					}
					reader.append(ul);
				} else if (kind === "facts") {
					reader.append(factsList(b[1]));
				} else if (kind === "pre") {
					reader.append(el("pre", "gd-pre", b[1]));
				} else if (kind === "tbl") {
					const t = el("table", "gd-tbl");
					b[1].forEach((row, i) => {
						const tr = el("tr");
						for (const c of row) {
							const cell = el(i === 0 ? "th" : "td");
							cell.append(rich(c));
							tr.append(cell);
						}
						t.append(tr);
					});
					reader.append(t);
				}
			}
			if (g.cut) reader.append(el("div", "gd-cut", "The rest of this section is on the page."));
		}
		root.style.setProperty("--read", SIZES[size()]);

		// ---- the days of the section: a chip each
		if (days.length && !st.raw) {
			const row = el("div", "gd-days");
			for (const d of days) {
				const c = el("button", "gd-chip", d.text);
				c.type = "button";
				c.addEventListener("click", () => reader.scrollTo({ top: d.node.offsetTop - 2 }));
				row.append(c);
			}
			out.push(row);
		}

		// ---- the picture and the facts, and where they go (see the layout above)
		const lay = profile ? st.layout : "plain";
		root.className = lay === "plain" ? "gd" : `gd gd-l-${lay}`;
		const all = factsBlock ? factsBlock[1] : [];
		const first = factsList(all.slice(0, 4));
		const rest = all.slice(4);
		// the rest of the facts open with one press
		const more = el("div", "gd-more");
		if (rest.length) {
			if (st.more) more.append(factsList(rest));
			const b = el("button", "gd-morebtn", st.more ? "Fewer details" : `More details (${rest.length})`);
			b.type = "button";
			b.addEventListener("click", () => {
				st.more = !st.more;
				st.top = reader.scrollTop;
				draw(g);
			});
			more.append(b);
		}
		if (lay === "rail") {
			const body = el("div", "gd-railbody");
			const main = el("div", "gd-main");
			main.append(reader);
			const rail = el("div", "gd-rail");
			if (image) rail.append(image);
			if (all.length) rail.append(first);
			rail.append(more);
			body.append(main, rail);
			out.push(body);
		} else if (lay === "strip") {
			const strip = el("div", "gd-strip");
			if (image) strip.append(image);
			strip.append(first);
			out.push(strip, more, reader);
		} else {
			if (lay === "stack" && image) out.push(image);
			if (lay === "stack" && all.length) out.push(first, more);
			out.push(reader);
		}

		// ---- what the section did not load, or the section that is coming
		if (st.note || st.busy) out.push(el("div", "gd-note", st.busy ? "Opening the page…" : st.note));

		// ---- previous and next section
		if (g.prev || g.next) {
			const foot = el("div", "gd-foot");
			const nav = (cls, label, link) => {
				const b = el("button", `gd-nav ${cls}`);
				b.type = "button";
				b.append(el("small", null, label), el("span", null, link ? link.t : "End of the guide"));
				if (!link) b.disabled = true;
				else b.addEventListener("click", () => go(link.u, g));
				return b;
			};
			foot.append(nav("prev", "‹ Previous", g.prev), nav("next", "Next ›", g.next));
			out.push(foot);
		}

		// ---- scroll: put back where it was, once the reader is on screen (a collapsed card has no size)
		reader.addEventListener(
			"scroll",
			() => {
				if (reader.clientHeight) st.top = reader.scrollTop;
			},
			{ passive: true },
		);
		new ResizeObserver(() => {
			if (!reader.clientHeight) {
				st.applied = false;
			} else if (!st.applied) {
				st.applied = true;
				reader.scrollTop = st.top;
			}
		}).observe(reader);

		// ---- search
		const apply = (jump) => {
			reader.querySelectorAll("mark").forEach((m) => m.replaceWith(document.createTextNode(m.textContent)));
			reader.normalize();
			const q = st.q.trim();
			let total = 0;
			if (q.length >= 2) {
				const re = new RegExp(q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"), "gi");
				const walker = document.createTreeWalker(reader, NodeFilter.SHOW_TEXT, {
					acceptNode: (t) => (t.parentElement.closest("button") ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
				});
				const nodes = [];
				while (walker.nextNode()) nodes.push(walker.currentNode);
				for (const t of nodes) {
					const text = t.nodeValue;
					re.lastIndex = 0;
					if (!re.test(text)) continue;
					re.lastIndex = 0;
					const frag = document.createDocumentFragment();
					let last = 0;
					let m;
					while ((m = re.exec(text))) {
						frag.append(text.slice(last, m.index), el("mark", null, m[0]));
						last = m.index + m[0].length;
						total++;
						if (!m[0].length) re.lastIndex++;
					}
					frag.append(text.slice(last));
					t.replaceWith(frag);
				}
			}
			const marks = reader.querySelectorAll("mark");
			if (st.hit >= marks.length) st.hit = 0;
			marks.forEach((m, i) => m.classList.toggle("cur", i === st.hit));
			n.textContent = q.length >= 2 ? (total ? `${st.hit + 1}/${total}` : "none") : "";
			const cur = marks[st.hit];
			if (cur && jump) reader.scrollTo({ top: cur.offsetTop - reader.clientHeight / 3 });
		};
		input.addEventListener("input", () => {
			st.q = input.value;
			tools.classList.toggle("has-q", !!st.q.trim());
			st.hit = 0;
			apply(true);
		});
		// (a floating card takes the keyboard while this field is in use: see floats.rs)
		input.addEventListener("focus", () => onTyping && onTyping(true));
		input.addEventListener("blur", () => onTyping && onTyping(false));
		input.addEventListener("keydown", (e) => {
			e.stopPropagation();
			if (e.key === "Escape") {
				input.blur();
				return;
			}
			if (e.key !== "Enter") return;
			e.preventDefault();
			const total = reader.querySelectorAll("mark").length;
			if (total) {
				st.hit = (st.hit + (e.shiftKey ? total - 1 : 1)) % total;
				apply(true);
			}
		});
		// the reader exists only now: marks go in once it is in the card
		queueMicrotask(() => apply(false));
		return out;
	}

	// ---- another section: the tab itself goes there. The page is the source of truth: it is read again once it has
	// loaded, and this card (the same one, it follows the tab) is rebuilt with what the page says.
	async function go(url, g) {
		st.busy = true;
		st.note = "";
		draw(g);
		let ok = false;
		try {
			ok = await invoke("guide_go", { exePath: exe, page, url });
		} catch (_) {}
		const fail = (note) => {
			st.busy = false;
			st.note = note;
			if (root.isConnected) draw(g);
		};
		if (!ok) return fail("The tab could not be sent there.");
		setTimeout(() => st.busy && fail("The page did not load."), 15000);
	}

	return root;
}
