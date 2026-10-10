// Games, the face of the native plugin (src/builtin/games): its card in the hub and its pill.
import { FLOATS, addPill, call, collapsible, el, focusable, formatDuration, going, iconEl, li, listen, statRing, textCol, view, withIcon } from "../kit.js";

// What each running game uses (CPU, memory, GPU, video memory, frames), by pid
const stats = new Map();

// frame times (ms, newest last) as a line: the 60 fps (16.7 ms) and 30 fps (33.3 ms) levels are marked,
// and the scale is fixed at 50 ms so a hitch is a visible spike
function frameGraph(ms) {
	if (!ms || ms.length < 2) return "";
	const max = 50;
	const pts = ms.map((v, i) => [
		(i / (ms.length - 1)) * 120,
		52 - Math.min(v / max, 1) * 50,
	]);
	const line = pts
		.map(
			(p, i) =>
				`${i ? "L" : "M"}${p[0].toFixed(1)} ${p[1].toFixed(1)}`,
		)
		.join(" ");
	const y = (v) => (52 - (v / max) * 50).toFixed(1);
	return `<svg viewBox="0 0 120 54" preserveAspectRatio="none"><line class="g" x1="0" x2="120" y1="${y(16.7)}" y2="${y(16.7)}"/><line class="g" x1="0" x2="120" y1="${y(33.3)}" y2="${y(33.3)}"/><path class="a" d="${line} L120 54 L0 54 Z"/><path class="l" d="${line}"/></svg>`;
}

// Game: collapsed it is the name and three rings (GPU, CPU, memory -- the game's own share, in the
// colours of the hub's rings); opened it adds the meters and the time played.
function gameCard(g) {
	const card = el("div", "now-card spot");
	const row = el("div", "now-row");
	row.append(
		iconEl(g.icon, "gamepad"),
		textCol(g.name, going(g.playtime_secs)),
	);
	const rings = el("div", "g-rings");
	const gpu = statRing("#8ADB6E");
	const cpu = statRing("#5AC8E6");
	const ram = statRing("#B47EE6");
	// frames per second, when they can be counted: the number leads, the rings follow
	const fpsEl = el("span", "g-fps");
	fpsEl.hidden = true;
	const fpsNum = el("b");
	fpsEl.append(fpsNum, el("small", "", "FPS"));
	rings.append(fpsEl, gpu.el, cpu.el, ram.el);
	row.append(rings);
	card.append(row);

	// opened: FPS, 1% low and frame time, the frame-time graph, then the meters
	const vitals = el("div", "work-body g-vitals");
	vitals.hidden = true;
	const stat = (label) => {
		const box = el("div", "g-stat");
		const num = el("b");
		box.append(num, el("span", "", label));
		return { box, num };
	};
	const sFps = stat("FPS");
	const sLow = stat("1% low");
	const sMs = stat("Frame time");
	const statsRow = el("div", "g-stats");
	statsRow.append(sFps.box, sLow.box, sMs.box);
	const graph = el("div", "g-graph");
	vitals.append(statsRow, graph);
	card.append(vitals);

	const body = el("div", "work-body g-meters");
	const meters = {};
	for (const [key, label, color] of [
		["gpu", "GPU", "#8ADB6E"],
		["vram", "VRAM", "#8ADB6E"],
		["cpu", "CPU", "#5AC8E6"],
		["ram", "RAM", "#B47EE6"],
	]) {
		const m = el("div", "g-meter");
		const bar = el("i");
		bar.style.setProperty("--c", color);
		const val = el("em");
		m.append(el("span", "", label), bar, val);
		body.append(m);
		meters[key] = { m, bar, val };
	}
	card.append(body);

	const update = (data) => {
		const s = stats.get(data.pid);
		const sub = card.querySelector(".now-sub");
		if (sub)
			withIcon(
				sub,
				"timer",
				formatDuration(data.playtime_secs),
			);
		const put = (key, pct, text, tip) => {
			const mt = meters[key];
			mt.m.hidden = pct == null;
			mt.bar.style.setProperty(
				"--v",
				`${Math.max(0, Math.min(100, pct ?? 0))}%`,
			);
			mt.val.textContent = text;
		};
		const hasFps = !!(s && s.fps != null);
		fpsEl.hidden = !hasFps;
		vitals.hidden = !hasFps;
		if (hasFps) {
			fpsNum.textContent = String(Math.round(s.fps));
			sFps.num.textContent = String(Math.round(s.fps));
			sLow.num.textContent = String(
				Math.round(s.low_fps),
			);
			sMs.num.textContent = `${(1000 / Math.max(s.fps, 1)).toFixed(1)} ms`;
			graph.innerHTML = frameGraph(s.frame_ms);
		}
		gpu.set(
			s ? s.gpu_pct : null,
			s && s.gpu_pct != null
				? `GPU \u00b7 ${Math.round(s.gpu_pct)}%`
				: "",
		);
		cpu.set(
			s ? s.cpu_pct : null,
			s ? `CPU \u00b7 ${Math.round(s.cpu_pct)}%` : "",
		);
		ram.set(
			s ? s.ram_pct : null,
			s ? `RAM \u00b7 ${s.ram_gb.toFixed(1)} GB` : "",
		);
		put(
			"gpu",
			s ? s.gpu_pct : null,
			s && s.gpu_pct != null
				? `${Math.round(s.gpu_pct)}%`
				: "",
		);
		put(
			"vram",
			s && s.vram_gb != null
				? Math.min(100, (s.vram_gb / 12) * 100)
				: null,
			s && s.vram_gb != null
				? `${s.vram_gb.toFixed(1)} GB`
				: "",
		);
		put(
			"cpu",
			s ? s.cpu_pct : null,
			s ? `${Math.round(s.cpu_pct)}%` : "",
		);
		put(
			"ram",
			s ? s.ram_pct : null,
			s ? `${s.ram_gb.toFixed(1)} GB` : "",
		);
	};
	update(g);
	// rebuilt only when the game itself changes; the numbers update in place
	card._sig = `game:${g.pid}:${g.exe_path}`;
	card._data = g;
	card._update = update;
	return focusable(
		collapsible(card, `game:${g.exe_path || g.name}`),
		{ exePath: g.exe_path },
	);
}

// the pill: the name of the game, and how it runs (FPS, then GPU / CPU / memory rings)
function initPill() {
	const gameEl = addPill(
		"game",
		`
<div id="game-icon">🎮</div>
<img id="game-icon-img" class="hidden" alt="" />
<div id="game-name"></div>
<span id="game-more" class="hidden"></span>
<span class="g-fps" id="game-fps" hidden><b></b><small>FPS</small></span>
<div class="g-rings" id="game-rings"></div>
	`,
	);
	document.getElementById("game-icon").innerHTML = li("gamepad");
	const gameName = document.getElementById("game-name");
	const gameMore = document.getElementById("game-more");
	const gameIconImg = document.getElementById("game-icon-img");
	const gameIconEmoji = document.getElementById("game-icon");
	let gamePid = 0;
	let pillGauges = null;
	const gameFpsEl = document.getElementById("game-fps");

	async function poll() {
		if (view.name !== "game" || !gamePid) return;
		const [s] = await call("games", "stats", { pids: [gamePid] });
		if (!pillGauges) {
			pillGauges = { gpu: statRing("#8ADB6E"), cpu: statRing("#5AC8E6"), ram: statRing("#B47EE6") };
			document.getElementById("game-rings").append(pillGauges.gpu.el, pillGauges.cpu.el, pillGauges.ram.el);
		}
		const hasFps = !!(s && s.fps != null);
		gameFpsEl.hidden = !hasFps;
		if (hasFps) gameFpsEl.firstElementChild.textContent = String(Math.round(s.fps));
		pillGauges.gpu.set(s ? s.gpu_pct : null, s && s.gpu_pct != null ? `GPU \u00b7 ${Math.round(s.gpu_pct)}%` : "");
		pillGauges.cpu.set(s ? s.cpu_pct : null, s ? `CPU \u00b7 ${Math.round(s.cpu_pct)}%` : "");
		pillGauges.ram.set(s ? s.ram_pct : null, s ? `RAM \u00b7 ${s.ram_gb.toFixed(1)} GB` : "");
	}
	setInterval(() => poll().catch(() => {}), 1000);

	listen("game-tick", (event) => {
		const g = event.payload;
		gamePid = g.has_game ? g.pid || 0 : 0;
		if (!g.has_game) return;
		gameName.textContent = g.name || "";
		gameMore.textContent = g.count > 1 ? `+${g.count - 1}` : "";
		gameMore.classList.toggle("hidden", !(g.count > 1));
		if (g.icon) {
			gameIconImg.src = g.icon;
			gameIconImg.classList.remove("hidden");
			gameIconEmoji.classList.add("hidden");
		} else {
			gameIconImg.classList.add("hidden");
			gameIconEmoji.classList.remove("hidden");
		}
	});
	return gameEl;
}

export default {
	id: "games",
	// the hub (not the floating cards' page) shows the pill
	init() {
		if (!FLOATS) initPill();
	},
	// (a game card sorts after the sessions that need you, before the downloads)
	async cards(games) {
		for (const s of await call("games", "stats", { pids: games.map((g) => g.pid) })) stats.set(s.pid, s);
		return games.map((g) => {
			const c = gameCard(g);
			c._rank = 10;
			return c;
		});
	},
};
