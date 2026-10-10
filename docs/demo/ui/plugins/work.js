// Work, the face of the native plugin (src/builtin/work): the cards of what you work on (the coding card, a card
// for each kind of program you used lately) and the pill for the session you are in.
import { FLOATS, addPill, collapsible, el, focusable, formatDuration, going, iconEl, li, line, listen, peek, textCol, withIcon } from "../kit.js";

const WORK_GLYPH = {
	coding: "code",
	writing: "pen",
	design: "sparkle",
	communication: "mail",
	media: "music",
	gaming: "gamepad",
	browsing: "globe",
	other: "dot",
};

const WORK_CARD_LABEL = {
	coding: "Coding",
	writing: "Writing",
	design: "Designing",
	communication: "Messaging",
	media: "Watching/Listening",
	gaming: "Gaming",
	browsing: "Browsing",
};

function workCard(w) {
	const card = el("div", "now-card spot");
	const row = el("div", "now-row");
	const label = WORK_CARD_LABEL[w.category] || "Working";
	row.append(
		iconEl(w.icon, WORK_GLYPH[w.category] || "dot"),
		textCol(
			{
				icon: WORK_GLYPH[w.category] || "dot",
				text: w.app_name || label,
			},
			going(w.going_secs),
		),
	);
	card.append(row);
	return focusable(
		collapsible(
			peek(card, going(w.going_secs)),
			`work:${w.category}`,
		),
		{ exePath: w.exe_path },
	);
}

function chip(text, cls) {
	return el("span", cls ? `chip ${cls}` : "chip", text);
}

// What you're working on: the project (with its git branch), the file in front
// of you and the uncommitted changes, summed up at the right edge.
const BRANCH_ICON =
	'<svg viewBox="0 0 16 16" width="10" height="10" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"><circle cx="4.5" cy="3.5" r="1.7"/><circle cx="4.5" cy="12.5" r="1.7"/><circle cx="11.5" cy="5.5" r="1.7"/><path d="M4.5 5.2v5.6M11.5 7.2c0 3-4.5 2-7 4"/></svg>';

function codingCard(c) {
	const p =
		c.project_info && c.project_info.branch
			? c.project_info
			: null;
	const card = el("div", "now-card spot");
	const row = el("div", "now-row");
	const tile = el("div", "now-icon tile");
	tile.innerHTML = li("code");
	row.append(tile);

	// the icon says it: a pen while editing a file, a slashed eye when nothing is going on
	// (the file itself is on its own line below)
	const status = c.active
		? { icon: c.file ? "pen" : "code", text: "" }
		: { icon: "eyeoff", text: "" };
	const col = el("div", "now-text");
	const head = el("div", "work-head");
	head.append(el("span", "now-title", c.project || "Coding"));
	if (p) {
		const branch = chip("", "branch");
		branch.innerHTML = BRANCH_ICON;
		branch.append(document.createTextNode(` ${p.branch}`));
		head.append(branch);
	}
	col.append(head, line("now-sub", status));
	row.append(col);

	const total = el("div", "now-total");
	if (p && p.changed) {
		const nums = el("div", "now-total-value diff-sum");
		nums.append(
			el("span", "add", `+${p.added.toLocaleString()}`),
			el(
				"span",
				"del",
				`\u2212${p.deleted.toLocaleString()}`,
			),
		);
		const files = el("div", "now-total-label");
		withIcon(files, "file", String(p.changed));
		total.append(nums, files);
	} else if (p) {
		const ok = el("div", "now-total-value");
		ok.innerHTML = li("check");
		ok.title = "No changes";
		total.append(ok);
	} else {
		// not a git project: fall back to the time spent
		const today = el("div", "now-total-label");
		today.innerHTML = li("clock");
		today.title = "Today";
		total.append(
			el(
				"div",
				"now-total-value",
				formatDuration(c.today_secs),
			),
			today,
		);
	}
	row.append(total);
	card.append(row);

	const body = el("div", "work-body");
	if (c.file) {
		const line = el("div", "work-line");
		if (c.language) {
			const dot = el("span", "lang-dot");
			dot.style.background =
				c.language_color || "#8a8a92";
			line.append(
				dot,
				el("span", "work-lang", c.language),
			);
		}
		line.append(el("span", "work-file", c.file));
		if (c.unsaved) {
			const dot = chip("", "warn");
			dot.innerHTML = li("dot");
			dot.title = "Unsaved";
			line.append(dot);
		}
		body.append(line);
	}
	if (p && p.files.length) {
		const peak = Math.max(
			1,
			...p.files.map((f) => f.added + f.deleted),
		);
		const list = el("div", "diff-list");
		for (const f of p.files) {
			const fileRow = el("div", "diff-row");
			fileRow.title = f.path;
			const st = el("span", `diff-st st-${f.status}`);
			st.innerHTML = li(
				{ A: "plus", D: "minus" }[f.status] || "pen",
			);
			st.title =
				{ A: "Added", D: "Deleted" }[f.status] ||
				"Modified";
			fileRow.append(st, el("span", "diff-name", f.name));
			const bar = el("span", "diff-bar");
			const sum = f.added + f.deleted;
			if (sum) {
				bar.style.width = `${Math.max(12, Math.round((sum / peak) * 100))}%`;
				const add = el("i", "add");
				add.style.flex = String(f.added);
				const del = el("i", "del");
				del.style.flex = String(f.deleted);
				bar.append(add, del);
			}
			const barBox = el("span", "diff-barbox");
			barBox.append(bar);
			fileRow.append(
				barBox,
				el(
					"span",
					"diff-num",
					sum
						? `+${f.added} \u2212${f.deleted}`
						: "\u2014",
				),
			);
			list.append(fileRow);
		}
		if (p.changed > p.files.length)
			list.append(
				el(
					"div",
					"diff-more",
					`+${p.changed - p.files.length} more`,
				),
			);
		body.append(list);
	}
	if (body.childElementCount) card.append(body);
	// the editor window for the active project (falls back to any editor window)
	return focusable(collapsible(card, "code"), {
		exePath: c.exe_path,
		titleHint: c.project,
	});
}

// the pill: what you are doing, in what, and for how long
function initPill() {
	addPill(
		"work",
		`
		<div id="work-icon"></div>
		<div class="text-col">
			<div id="work-label" class="line-main"></div>
			<div id="work-app" class="line-sub"></div>
		</div>
		<span id="work-time"></span>
	`,
	);
	const workIcon = document.getElementById("work-icon");
	const workLabel = document.getElementById("work-label");
	const workApp = document.getElementById("work-app");
	const workTime = document.getElementById("work-time");
	listen("work-tick", (event) => {
		const w = event.payload;
		if (!w.has_session) return;
		workIcon.innerHTML = li(WORK_GLYPH[w.category] || "dot");
		workLabel.textContent = w.label || "";
		workApp.textContent = w.app_name || "";
		workTime.textContent = formatDuration(w.started_at_secs);
	});
}

export default {
	id: "work",
	init() {
		if (!FLOATS) initPill();
	},
	cards(data) {
		const out = [];
		if (data.coding) {
			const c = codingCard(data.coding);
			c._rank = 40;
			out.push(c);
		}
		for (const w of data.work || []) {
			const c = workCard(w);
			c._rank = 50;
			out.push(c);
		}
		return out;
	},
};
