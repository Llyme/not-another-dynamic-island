// Downloads, the face of the native plugin (src/builtin/downloads): one card for every download.
import { call, collapsible, el, host, invoke, li } from "../kit.js";

// ---- downloads: one card per source, up to three downloads on each ----
function formatBytes(n) {
	if (n < 1024) return `${Math.round(n)} B`;
	const units = ["KB", "MB", "GB", "TB"];
	let v = n / 1024;
	let u = 0;
	while (v >= 1024 && u < units.length - 1) {
		v /= 1024;
		u++;
	}
	return `${v >= 100 ? Math.round(v) : v.toFixed(1)} ${units[u]}`;
}

// small line icons: a folder (a browser download opens its folder) and a package (the
// delivery arrived: click to put it away)
const DL_ICON_FOLDER =
	'<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" stroke-linecap="round"><path d="M1.8 4.6c0-.7.5-1.2 1.2-1.2h3l1.4 1.6h5.6c.7 0 1.2.5 1.2 1.2v5.4c0 .7-.5 1.2-1.2 1.2H3c-.7 0-1.2-.5-1.2-1.2z"/></svg>';
const DL_ICON_PACKAGE =
	'<svg viewBox="0 0 16 16" width="15" height="15" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round" stroke-linecap="round"><path d="M8 1.6l5.6 2.9v6.9L8 14.3l-5.6-2.9V4.5z"/><path d="M2.6 4.7L8 7.5l5.4-2.8M8 7.5v6.6"/><path d="M5.2 3l5.4 2.8"/></svg>';

function downloadMeta(it) {
	if (it.done) return "";
	const speed =
		it.speed > 1024 ? ` · ${formatBytes(it.speed)}/s` : "";
	// Steam shows no progress anywhere, so neither does the island: size and bandwidth only
	if (it.kind === "steam")
		return (
			[
				it.total ? formatBytes(it.total) : null,
				speed ? speed.slice(3) : null,
			]
				.filter(Boolean)
				.join(" \u00b7 ") || "Downloading"
		);
	if (it.total)
		return `${Math.min(100, Math.floor((it.received / it.total) * 100))}%${speed}`;
	return `${formatBytes(it.received)}${speed}`;
}

const DL_RING_C = 2 * Math.PI * 15; // circumference of the progress ring (r = 15 in a 36 box)

// the combined download speed, one sample per refresh (about a second): the last half minute
const dlSpeedHistory = Array.from({ length: 30 }, () => 0);
function sparkSvg() {
	const h = dlSpeedHistory;
	// adaptive: the lowest and highest speed of the window are the bottom and top of the graph, so
	// the line always uses its whole height. A nearly flat stretch is not blown up into noise: the
	// range is at least 10% of the top speed (or 50 KB/s), and sits centred around it.
	const hi = Math.max(...h);
	const lo = Math.min(...h);
	const span = Math.max(hi - lo, hi * 0.1, 50_000);
	const base =
		hi === 0
			? 0
			: hi - lo >= span
				? lo
				: (hi + lo) / 2 - span / 2;
	const pts = h.map((v, i) => [
		(i / (h.length - 1)) * 120,
		25 - Math.max(0, Math.min(1, (v - base) / span)) * 22,
	]);
	const line = pts
		.map(
			(p, i) =>
				`${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`,
		)
		.join(" ");
	return `<svg viewBox="0 0 120 28" preserveAspectRatio="none"><path d="${line} L120 28 L0 28 Z" class="a"/><path d="${line}" class="l"/></svg>`;
}

function downloadCard(d) {
	const card = el("div", "now-card spot dl-card");
	// header: a ring of overall progress around the download icon (the count on its corner), a trace
	// of the combined speed over the last half minute, and the speed (a package when all are done)
	const head = el("div", "now-row");
	const ring = el("div", "dl-ring");
	ring.innerHTML = `<svg class="r" viewBox="0 0 36 36"><circle class="rb" cx="18" cy="18" r="15"/><circle class="rf" cx="18" cy="18" r="15"/></svg>${li("download")}<b class="dl-badge"></b>`;
	const ringFill = ring.querySelector(".rf");
	const badge = ring.querySelector(".dl-badge");
	const spark = el("div", "dl-spark");
	const headRight = el("div", "now-total-label dl-total");
	head.append(ring, spark, headRight);
	card.append(head);
	const list = el("div", "dl-list");
	const refs = [];
	for (const it of d.items) {
		const item = el(
			"div",
			`dl-item ${it.done ? "done" : "focus"}${it.kind === "steam" ? " nobar" : ""}`,
		);
		item.title = it.name;
		const bar = el("i", "dl-fill");
		const track = el("span", "dl-bar");
		track.append(bar);
		const meta = el("span", "dl-meta");
		const name = el("span", "dl-name");
		if (it.icon) {
			const app = el("img", "dl-app");
			app.src = it.icon;
			app.alt = "";
			app.title = it.source;
			name.append(app);
		} else {
			name.append(el("b", "dl-src", it.source));
		}
		name.append(document.createTextNode(it.name));
		item.append(name, meta, track);
		item.addEventListener("mousedown", (e) =>
			e.stopPropagation(),
		);
		item.addEventListener("click", async (e) => {
			e.stopPropagation();
			if (it.done) {
				// dismisses it (a browser download also opens its folder)
				await call("downloads", "click", { id: it.id });
				host.refresh();
			} else if (it.exe_path) {
				invoke("focus_source", {
					exePath: it.exe_path,
				}); // bring the app up
			}
		});
		list.append(item);
		refs.push({ bar, meta });
	}
	card.append(list);

	const update = (data) => {
		const live = data.items.filter((it) => !it.done);
		badge.textContent = String(data.items.length);
		// overall: what the items that report a size have got of it; none do (Steam) -> a turning sliver
		const sized = live.filter((it) => it.total);
		const total = sized.reduce((s, it) => s + it.total, 0);
		const got = sized.reduce(
			(s, it) => s + Math.min(it.received, it.total),
			0,
		);
		const overall =
			live.length === 0
				? 100
				: total
					? (got / total) * 100
					: null;
		ring.classList.toggle("busy", overall == null);
		ringFill.style.strokeDashoffset = String(
			DL_RING_C *
				(1 - (overall == null ? 0.28 : overall / 100)),
		);
		spark.innerHTML = sparkSvg();
		if (live.length) {
			const speed = live.reduce(
				(s, it) => s + (it.speed || 0),
				0,
			);
			headRight.classList.remove("dl-done-icon");
			headRight.textContent =
				speed > 1024 ? `${formatBytes(speed)}/s` : "";
		} else {
			headRight.classList.add("dl-done-icon");
			headRight.innerHTML = DL_ICON_PACKAGE;
		}
		data.items.forEach((it, i) => {
			const r = refs[i];
			if (!r) return;
			if (it.done) {
				// no words: a folder for a browser download (opens it), a package otherwise
				r.meta.innerHTML =
					it.kind === "browser"
						? DL_ICON_FOLDER
						: DL_ICON_PACKAGE;
				r.meta.title =
					it.kind === "browser"
						? "Open Folder"
						: "Put Away";
				r.meta.classList.add("dl-done-icon");
			} else {
				r.meta.textContent = downloadMeta(it);
				r.meta.classList.remove("dl-done-icon");
			}
			const pct = it.done
				? 100
				: it.total
					? Math.min(
							100,
							(it.received / it.total) * 100,
						)
					: null;
			r.bar.classList.toggle("busy", pct == null);
			r.bar.style.width = pct == null ? "" : `${pct}%`;
		});
	};
	update(d);
	card._sig = `dl:${d.items.map((i) => i.id + (i.done ? "!" : "")).join(",")}`;
	card._data = d;
	card._update = update;
	card.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	); // not "toggle the hub"
	return collapsible(card, "dl");
}

export default {
	id: "downloads",
	cards(data) {
		// the combined speed, one sample for each refresh: the card draws the trace
		const speed = data.reduce((s, d) => s + d.items.reduce((t, it) => t + (it.done ? 0 : it.speed || 0), 0), 0);
		dlSpeedHistory.push(speed);
		dlSpeedHistory.shift();
		return data.map((d) => {
			const c = downloadCard(d);
			c._rank = 20;
			return c;
		});
	},
};
