// Claude Code, the face of the native plugin (src/builtin/claude_code): one card for every session, and the two rings of
// how much of the Claude limits is used (the 5-hour and the 7-day ones) with the peek that comes when one moves: the bars
// sit at the old value, then fill to the new one once the island has landed.
// A session that needs you leads, then the ones that finished, then the others.
import { FLOATS, LLM_COLOR, LLM_LETTER, LLM_RING_C, addPill, addRing, call, collapsible, el, focusable, formatDuration, host, invoke, li, listen, llmCtxColor, llmStateIcon, ringEls, setRing } from "../kit.js";

// ---- LLM sessions (Claude Code): one card per session. Needs you > finished > the others. ----
const formatTokens = (n) =>
	n >= 1e6
		? `${(n / 1e6).toFixed(1)}M`
		: `${Math.round(n / 1000)}k`;

function llmCard(c) {
	const card = el("div", "now-card spot llm-card");
	const row = el("div", "now-row");
	const tile = el("div", "llm-tile");
	tile.style.setProperty(
		"--c",
		LLM_COLOR[c.provider] || "#888",
	);
	tile.innerHTML = `${LLM_LETTER[c.provider] || "?"}<svg class="cr" viewBox="0 0 40 40"><circle class="rb" cx="20" cy="20" r="18"/><circle class="rf" cx="20" cy="20" r="18"/></svg>`;
	const rf = tile.querySelector(".rf");
	rf.style.strokeDasharray = String(LLM_RING_C);
	const col = el("div", "now-text");
	const title = el("div", "now-title");
	const liveLine = el("div", "llm-live");
	col.append(title, liveLine);
	const right = el("div", "llm-right");
	row.append(tile, col, right);
	card.append(row);

	// opened: the prompt, the latest tool calls, the context window, the numbers
	const body = el("div", "work-body llm-body");
	const prompt = el("p", "llm-prompt");
	const feed = el("div", "llm-feed");
	const ctx = el("div", "llm-ctx");
	const ctxBar = el("i");
	const ctxVal = el("em");
	ctx.append(el("span", "", "Context"), ctxBar, ctxVal);
	const stat = (label) => {
		const box = el("div", "llm-stat");
		const num = el("b");
		box.append(num, el("span", "", label));
		return { box, num };
	};
	const sRun = stat("Running");
	const sCost = stat("Cost");
	const sLines = stat("Lines");
	const sModel = stat("Model");
	const stats = el("div", "llm-stats");
	stats.append(sRun.box, sCost.box, sLines.box, sModel.box);
	const focus = el("button", "llm-focus", "Focus");
	focus.addEventListener("mousedown", (e) =>
		e.stopPropagation(),
	);
	body.append(prompt, feed, ctx, stats, focus);
	card.append(body);

	let lastState = "";
	const update = (d) => {
		title.textContent = d.title;
		card.classList.toggle(
			"llm-waiting",
			d.state === "waiting",
		);
		liveLine.className = `llm-live ${d.state}`;
		liveLine.innerHTML = li(d.activity.icon);
		liveLine.append(el("span", "", d.activity.text));
		// the host (VS Code, a terminal) as a small icon, its name on hover
		if (d.state !== lastState || !right.firstChild) {
			lastState = d.state;
			right.replaceChildren();
			const host = el("span", "llm-host");
			host.innerHTML = li(d.host_icon);
			host.dataset.tip = `${d.host}\n${d.project}`;
			right.append(host, llmStateIcon(d.state));
		}
		const frac = d.context_limit
			? Math.min(1, d.context_tokens / d.context_limit)
			: 0;
		rf.style.stroke = llmCtxColor(frac);
		rf.style.strokeDashoffset = String(
			LLM_RING_C * (1 - frac),
		);
		prompt.replaceChildren();
		if (d.prompt) {
			prompt.append(
				el("b", "", "Prompt"),
				document.createTextNode(d.prompt),
			);
		}
		prompt.hidden = !d.prompt;
		feed.replaceChildren();
		const rows =
			d.state === "working"
				? [d.activity, ...d.feed.slice(0, 2)]
				: d.feed.slice(0, 3);
		rows.forEach((f, i) => {
			const r = el(
				"div",
				i === 0 && d.state === "working"
					? "llm-fr now"
					: "llm-fr",
			);
			r.innerHTML = li(f.icon);
			r.append(el("span", "", f.text));
			feed.append(r);
		});
		ctxBar.style.setProperty("--v", `${frac * 100}%`);
		ctxBar.style.setProperty("--c", llmCtxColor(frac));
		ctxVal.textContent = `${formatTokens(d.context_tokens)} / ${formatTokens(d.context_limit)}`;
		sRun.num.textContent = formatDuration(d.running_secs);
		sCost.num.textContent = `$${d.cost_usd.toFixed(2)}`;
		sLines.num.textContent = `+${d.lines_added} \u2212${d.lines_removed}`;
		sModel.num.textContent = d.model || "\u2014";
		focus.hidden =
			d.state !== "waiting" && d.state !== "finished";
	};
	update(c);
	const args = () => ({
		exePath: c.host_exe,
		titleHint: c.project,
	});
	// to the host window; a finished session has been looked at then, so its card goes
	const go = () => {
		invoke("focus_source", args());
		if (lastState === "finished") {
			call("claude-code", "dismiss", { id: c.id }).then(() => host.refresh());
		}
	};
	focus.addEventListener("click", (e) => {
		e.stopPropagation();
		go();
	});
	card._sig = `llm:${c.id}`;
	card._data = c;
	card._update = update;
	return focusable(
		collapsible(card, `llm:${c.id}`),
		args(),
		go,
	);
}

// ---- usage: the rings and the peek ----
function formatReset(iso) {
	if (!iso) return "";
	const mins = Math.max(
		0,
		Math.floor(
			(new Date(iso).getTime() - Date.now()) / 60000,
		),
	);
	if (mins < 60) return `${mins}m`;
	const hours = Math.floor(mins / 60);
	if (hours < 24) return `${hours}h ${mins % 60}m`;
	const days = Math.floor(hours / 24);
	return `${days}d ${hours % 24}h`;
}

function applyUsage(u) {
	const windows = [
		["five_hour", "5h", u?.five_hour],
		["seven_day", "7d", u?.seven_day],
	];
	for (const [key, label, w] of windows) {
		const ring = ringEls[key];
		// (an error that left the last numbers in place still shows them, and says so)
		const visible = !!(u && u.available && w && w.pct != null);
		ring.el.classList.toggle("hidden", !visible);
		ring.el.classList.toggle("clickable", visible);
		if (!visible) continue;
		const reset = formatReset(w.resets_at);
		const tip = `Claude ${label} · ${Math.round(w.pct)}%\n${reset ? `Resets in ${reset} · ` : ""}${u.error ? `Not up to date: ${u.error}` : "Click to refresh"}`;
		setRing(key, w.pct, tip);
	}
}

let usagePeekData = null;
let usagePeekTimer = null;
function playUsagePeek() {
	if (!usagePeekData) return;
	const { before, after } = usagePeekData;
	const keys = ["five_hour", "seven_day"];
	for (const k of keys) {
		const fill = document.getElementById(`peek-fill-${k}`);
		fill.classList.remove("animating");
		fill.style.width = `${before[k] ?? after[k] ?? 0}%`;
		document.getElementById(`peek-pct-${k}`).textContent =
			before[k] == null
				? ""
				: `${Math.round(before[k])}%`;
	}
	clearTimeout(usagePeekTimer);
	usagePeekTimer = setTimeout(() => {
		for (const k of keys) {
			const fill = document.getElementById(
				`peek-fill-${k}`,
			);
			fill.classList.add("animating");
			fill.style.width = `${after[k] ?? 0}%`;
			document.getElementById(
				`peek-pct-${k}`,
			).textContent =
				after[k] == null
					? ""
					: `${Math.round(after[k])}%`;
		}
	}, 500);
}

function loadUsage() {
	call("claude-code", "usage_state")
		.then(applyUsage)
		.catch(() => {});
}

export default {
	id: "claude-code",
	init() {
		// the rings: hidden until there is something to show (no Claude Code login, no rings)
		addRing("seven_day", "#E8963C");
		addRing("five_hour", "#E8963C");
		for (const key of ["five_hour", "seven_day"]) ringEls[key].el.addEventListener("click", () => call("claude-code", "usage_refresh"));
		listen("usage-tick", (event) => applyUsage(event.payload));
		if (FLOATS) return;
		const el = addPill(
			"usage_peek",
			`
<div class="peek-row"><span class="peek-label">5h</span><div class="peek-track"><div class="peek-fill" id="peek-fill-five_hour"></div></div><span class="peek-pct" id="peek-pct-five_hour"></span></div>
<div class="peek-row"><span class="peek-label">7d</span><div class="peek-track"><div class="peek-fill" id="peek-fill-seven_day"></div></div><span class="peek-pct" id="peek-pct-seven_day"></span></div>
		`,
		);
		listen("usage-peek", (e) => {
			usagePeekData = e.payload;
		});
		// the island has landed on the pill: play it
		new MutationObserver(() => {
			if (!el.classList.contains("hidden")) playUsagePeek();
		}).observe(el, { attributes: true, attributeFilter: ["class"] });
	},
	hubOpened: loadUsage,
	cards(data) {
		return data.map((d) => {
			const c = llmCard(d);
			c._rank = d.state === "waiting" || d.state === "finished" ? 0 : 30;
			return c;
		});
	},
};
