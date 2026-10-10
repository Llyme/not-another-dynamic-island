// What the island draws for a plugin that is not a page of its own: cards from blocks, a pill, rings in the hub. Every
// plugin that is not a Rust plugin with its own face (ui/plugins/*.js) goes through this, and so do those that want to:
// a module or a declarative plugin hands the island data (see plugins.rs: `CardOut`, `PillOut`, `RingOut`) and never any
// markup, so every text goes in as text, every picture is a `data:` raster, and a button only names an action.
import { FLOATS, addPill, addRing, call, collapsible, el, host, invoke, li, listen, peek, ringEls, setRing, textCol, view } from "./kit.js";

// ---------------------------------------------------------------------------------- a moment inside a text

const WHEN = /\{when:(-?\d+)(?::(day))?\}/g;

/** "Today 3:00 PM", "Tomorrow", "Fri, Oct 9 3:00 PM": how this viewer reads a moment (a plugin writes `{when:MS}` or `{when:MS:day}`) */
export function whenText(s) {
	if (typeof s !== "string" || !s.includes("{when:")) return s;
	return s.replace(WHEN, (_, ms, day) => {
		const d = new Date(Number(ms));
		const now = new Date();
		const same = d.toDateString() === now.toDateString();
		const tomorrow = new Date(now.getTime() + 86400000).toDateString() === d.toDateString();
		const name = same ? "Today" : tomorrow ? "Tomorrow" : d.toLocaleDateString([], { weekday: "short", month: "short", day: "numeric" });
		return day ? name : `${name} ${d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`;
	});
}

/** a text that has a moment in it changes by itself at midnight: it is built again by the day */
const hasWhen = (s) => typeof s === "string" && s.includes("{when:");

// ----------------------------------------------------------------------------------------------- the blocks

/** the blocks a plugin can ask for, each built once and then updated in place as the numbers change */
export function pluginBlock(b, plugin) {
	const block = pluginBlockOf(b, plugin);
	block.kind = b.kind;
	return block;
}

function buttonRow(items, plugin) {
	const box = el("div", "pg-buttons");
	for (const it of items) {
		const b = el("button", it.icon ? "pg-b icon" : "pg-b");
		b.type = "button";
		b.title = it.label || "";
		if (it.icon) b.innerHTML = li(it.icon);
		else b.textContent = it.label;
		// (the island's own press on a card must not take this one)
		b.addEventListener("mousedown", (e) => e.stopPropagation());
		b.addEventListener("click", (e) => {
			e.stopPropagation();
			call(plugin, it.action).catch(() => {});
		});
		box.append(b);
	}
	return box;
}

function pluginBlockOf(b, plugin) {
	if (b.kind === "progress") {
		const bar = el("div", "pg-bar");
		const fill = el("i");
		bar.append(fill);
		return { node: bar, update: (n) => (fill.style.width = `${Math.round(n.value * 100)}%`) };
	}
	if (b.kind === "bars") {
		// meters: the heights follow the numbers (a live card sends them as they change)
		const box = el("div", "pg-bars");
		const bars = b.values.map(() => {
			const i = el("i");
			box.append(i);
			return i;
		});
		return {
			node: box,
			update: (n) => n.values.forEach((v, i) => bars[i] && (bars[i].style.height = `${Math.max(6, Math.round(v * 100))}%`)),
		};
	}
	if (b.kind === "text" || b.kind === "badge") {
		const t = el("div", b.kind === "text" ? "pg-text" : "pg-badge");
		return { node: t, update: (n) => (t.textContent = whenText(n.text)) };
	}
	if (b.kind === "facts") {
		const box = el("div", "pk-fields");
		const vs = b.rows.map(([k]) => {
			const r = el("div", "pk-row");
			const v = el("div", "pk-v");
			r.append(el("div", "pk-k", k), v);
			box.append(r);
			return v;
		});
		return { node: box, update: (n) => n.rows.forEach((r, i) => vs[i] && (vs[i].textContent = whenText(r[1]))) };
	}
	if (b.kind === "image") {
		const img = el("img", "pg-img");
		img.alt = "";
		img.draggable = false;
		return { node: img, update: (n) => img.getAttribute("src") !== n.src && (img.src = n.src) };
	}
	if (b.kind === "buttons") {
		// (rebuilt when the buttons change: see the card's signature)
		return { node: buttonRow(b.items, plugin), update: () => {} };
	}
	// a list: a title and a line under it, and a bar when the row has a progress
	const list = el("div", "pg-list");
	const rows = b.rows.map((r) => {
		const row = el("div", "pg-li");
		const title = el("span", "pg-li-t");
		const sub = el("span", "pg-li-s");
		row.append(title, sub);
		let fill = null;
		if (r.progress != null) {
			const bar = el("div", "pg-bar");
			fill = el("i");
			bar.append(fill);
			row.append(bar);
		}
		list.append(row);
		return { title, sub, fill };
	});
	return {
		node: list,
		update: (n) =>
			n.rows.forEach((r, i) => {
				const x = rows[i];
				if (!x) return;
				x.title.textContent = whenText(r.title);
				x.sub.textContent = whenText(r.sub);
				if (x.fill && r.progress != null) x.fill.style.width = `${Math.round(r.progress * 100)}%`;
			}),
	};
}

// -------------------------------------------------------------------------------------------------- the card

const blockSig = (b) =>
	b.kind +
	(b.rows ? `${b.rows.length}${b.rows.map((r) => (r.progress != null ? "p" : "")).join("")}` : b.values ? b.values.length : b.items ? b.items.map((i) => `${i.action}/${i.icon}/${i.label}`).join("|") : "");

export function pluginCard(c) {
	const card = el("div", "now-card spot pg-card");
	const row = el("div", "now-row");
	const tile = el("div", "now-icon tile");
	tile.innerHTML = li(c.icon);
	const col = textCol(c.title, c.sub);
	row.append(tile, col);
	card.append(row);
	const body = el("div", "work-body pg-body");
	const blocks = c.body.map((b) => pluginBlock(b, c.plugin));
	for (const b of blocks) body.append(b.node);
	if (blocks.length) card.append(body);
	const update = (d) => {
		col.querySelector(".now-title").textContent = whenText(d.title);
		col.querySelector(".now-sub").textContent = whenText(d.sub);
		const pk = card.querySelector(".now-peek");
		if (pk) pk.textContent = whenText(d.peek);
		d.body.forEach((n, i) => blocks[i] && blocks[i].kind === n.kind && blocks[i].update(n));
	};
	if (c.peek) peek(card, c.peek);
	update(c);
	// the cards are rebuilt only when their shape changes; new numbers go in place
	card._sig = `pg:${c.plugin}:${c.icon}:${c.peek ? "p" : ""}:${c.body.map(blockSig).join(",")}`;
	// (a moment in a text reads "Today" or "Tomorrow", which change at midnight: rebuilt by the day)
	if ([c.title, c.sub, c.peek, JSON.stringify(c.body)].some(hasWhen)) card._sig += `:${new Date().toDateString()}`;
	card._data = c;
	card._update = update;
	card._rank = c.rank || 70;
	card.dataset.plugin = c.plugin;
	card.addEventListener("mousedown", (e) => e.stopPropagation());
	return collapsible(card, `plugin:${c.plugin}`);
}

// a live card (a meter) is sent as it changes, not when the next poll comes
listen("plugin-frame", (e) => {
	const c = e.payload;
	for (const card of document.querySelectorAll(".pg-card[data-plugin]")) {
		if (card.dataset.plugin === c.plugin && card._update) {
			card._data = c;
			card._update(c);
		}
	}
});

// --------------------------------------------------------------------------------------------------- the pill

const pgPills = new Map(); // plugin id -> its pill's parts

function pillParts(id) {
	let p = pgPills.get(id);
	if (p) return p;
	const node = addPill(
		`pg-${id}`,
		`<div class="pg-tile"></div>
		<div class="text-col"><div class="line-main"></div><div class="line-sub"></div></div>
		<span class="pg-meters"></span>
		<span class="pg-right"></span>
		<span class="pg-pbuttons"></span>
		<div class="pg-pbar"><i></i></div>`,
	);
	node.classList.add("pg-pill");
	p = {
		node,
		tile: node.querySelector(".pg-tile"),
		main: node.querySelector(".line-main"),
		sub: node.querySelector(".line-sub"),
		meters: node.querySelector(".pg-meters"),
		right: node.querySelector(".pg-right"),
		buttons: node.querySelector(".pg-pbuttons"),
		bar: node.querySelector(".pg-pbar"),
		fill: node.querySelector(".pg-pbar i"),
		icon: "",
		sig: "",
	};
	node.addEventListener("mousedown", (e) => e.target.closest("button") && e.stopPropagation());
	pgPills.set(id, p);
	// the island may already be on this pill (the pill came before its content did)
	node.classList.toggle("hidden", view.name !== `pg-${id}`);
	if (view.name === `pg-${id}`) host.enter?.(node);
	return p;
}

function setPill(pill) {
	const p = pillParts(pill.plugin);
	if (p.icon !== pill.icon) {
		p.icon = pill.icon;
		p.tile.innerHTML = li(pill.icon);
	}
	p.node.classList.toggle("pg-big", pill.look === "big");
	p.main.textContent = whenText(pill.title);
	p.sub.textContent = whenText(pill.sub);
	p.sub.classList.toggle("hidden", !pill.sub);
	p.right.textContent = whenText(pill.right);
	p.right.classList.toggle("hidden", !pill.right);
	// meters: one bar each, the heights follow the numbers
	if (p.meters.childElementCount !== pill.bars.length) {
		p.meters.replaceChildren(...pill.bars.map(() => el("i")));
	}
	pill.bars.forEach((v, i) => (p.meters.children[i].style.height = `${Math.max(8, Math.round(v * 100))}%`));
	p.meters.classList.toggle("hidden", !pill.bars.length);
	// buttons: built again when they change
	const sig = pill.buttons.map((b) => `${b.action}/${b.icon}/${b.label}`).join("|");
	if (sig !== p.sig) {
		p.sig = sig;
		p.buttons.replaceChildren(...buttonRow(pill.buttons, pill.plugin).children);
	}
	p.buttons.classList.toggle("hidden", !pill.buttons.length);
	p.bar.classList.toggle("hidden", pill.progress == null);
	if (pill.progress != null) p.fill.style.width = `${Math.round(pill.progress * 100)}%`;
}

async function loadPills() {
	try {
		const a = await invoke("plugin_pills");
		for (const pill of a.pills || []) setPill(pill);
		setRings(a.rings || {});
	} catch (_) {}
}

// -------------------------------------------------------------------------------------------------- the rings

const pgRings = new Map(); // plugin id -> the ring keys it shows

/** the rings of one plugin: what it shows now (the ones it showed before and does not now are hidden) */
function setPluginRings(plugin, rings) {
	const had = pgRings.get(plugin) || new Set();
	const now = new Set();
	for (const r of rings) {
		const key = `pg-${plugin}-${r.id}`;
		now.add(key);
		if (!ringEls[key]) addRing(key, r.color);
		else {
			ringEls[key].fg.setAttribute("stroke", r.color);
			ringEls[key].el.dataset.color = r.color;
		}
		ringEls[key].el.classList.remove("hidden");
		setRing(key, r.pct, whenText(r.tip));
	}
	for (const key of had) if (!now.has(key)) ringEls[key]?.el.classList.add("hidden");
	pgRings.set(plugin, now);
}

/** every plugin's rings at once (what the hub's snapshot holds): a plugin that is not in it shows none */
export function setRings(byPlugin) {
	for (const plugin of new Set([...pgRings.keys(), ...Object.keys(byPlugin)])) setPluginRings(plugin, byPlugin[plugin] || []);
}

// ---------------------------------------------------------------------------------------------------- start

export function initPluginUi() {
	listen("plugin-rings", (e) => setPluginRings(e.payload.plugin, e.payload.rings || []));
	if (FLOATS) return;
	listen("plugin-pill", (e) => {
		if (e.payload.pill) setPill(e.payload.pill);
	});
	// a pill the island has just moved to: make sure it has what it says (the page may have opened after the plugin spoke)
	listen("view-tick", (e) => {
		if (typeof e.payload === "string" && e.payload.startsWith("pg-")) loadPills();
	});
	loadPills();
}
