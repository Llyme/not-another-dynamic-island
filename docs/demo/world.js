// The PC that the demo island watches. In the app, Rust reads this from Windows (the media session, the
// foreground window, running games, browser downloads, Claude Code's session files, Windows notifications);
// here it is a small simulation behind the mock desktop, answering with the same payloads and following the
// same rules (work.rs, activity.rs, llm.rs, downloads.rs, calendar.rs, usage.rs, clock.rs). Only the delays
// that would make a demo feel dead are shorter (a window has to be in front for 3 s, not 8).
(() => {
  "use strict";
  const { now, clamp, st, notif, cmds: C } = NADI;
  const S = () => NADI.settings();
  const W = (NADI.world = {
    fg: null, // id of the window in front (null = the desktop)
    win: {}, // id -> { open, min } (kept by desktop.js)
  });

  // ---------------------------------------------------------------- icons and cover art
  const svg = (inner, bg) =>
    "data:image/svg+xml;utf8," +
    encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='8' fill='${bg}'/>${inner}</svg>`);
  const stroke = "fill='none' stroke='#fff' stroke-width='2.3' stroke-linecap='round' stroke-linejoin='round'";
  const ICONS = (W.ICONS = {
    code: svg(`<path d='M12 11 7 16l5 5M20 11l5 5-5 5' ${stroke}/>`, "#2f7bd9"),
    term: svg(`<path d='M9 10l6 6-6 6M17 22h7' ${stroke}/>`, "#2a2a31"),
    browser: svg(`<circle cx='16' cy='16' r='8' ${stroke}/><path d='M8 16h16M16 8c3.5 3 3.5 13 0 16M16 8c-3.5 3-3.5 13 0 16' ${stroke}/>`, "#169c9c"),
    chat: svg(`<path d='M8 10h16v10H15l-5 4v-4H8z' fill='#fff'/>`, "#5865c8"),
    music: svg(`<path d='M13 21V10l9-2v11' ${stroke}/><circle cx='11' cy='21' r='2.4' fill='#fff'/><circle cx='20' cy='19' r='2.4' fill='#fff'/>`, "#1db36a"),
    game: svg(`<path d='M16 6l3 7 7 .8-5.3 4.7 1.6 7.2L16 22l-6.3 3.7 1.6-7.2L6 13.8 13 13z' fill='#fff'/>`, "#e8793a"),
  });
  const art = (h1, h2) =>
    "data:image/svg+xml;utf8," +
    encodeURIComponent(
      `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 64 64'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h1} 75% 55%)'/><stop offset='1' stop-color='hsl(${h2} 80% 45%)'/></linearGradient></defs><rect width='64' height='64' fill='url(#g)'/><circle cx='44' cy='20' r='16' fill='hsl(${h2} 90% 75%)' opacity='.55'/><circle cx='18' cy='46' r='22' fill='hsl(${h1} 90% 30%)' opacity='.45'/></svg>`,
    );

  // ---------------------------------------------------------------- the apps on this PC
  const APPS = (W.APPS = {
    code: { name: "Visual Studio Code", exe: "code.exe", cat: "coding" },
    term: { name: "Terminal", exe: "windowsterminal.exe", cat: "coding" },
    browser: { name: "Browser", exe: "msedge.exe", cat: "browsing" },
    chat: { name: "Discord", exe: "discord.exe", cat: "communication" },
    music: { name: "Music", exe: "spotify.exe", cat: "media" },
    game: { name: "Starfall", exe: "starfall.exe", cat: "other" },
  });
  const byExe = (exe) => Object.keys(APPS).find((id) => APPS[id].exe === exe);
  const visible = () => Object.keys(APPS).filter((id) => W.win[id] && W.win[id].open && !W.win[id].min);

  // ---------------------------------------------------------------- CPU / RAM / GPU
  const sys = { cpu: 12, ram: 44, gpu: 4 };
  setInterval(() => {
    const t = now();
    const game = st.hasGame ? 1 : 0;
    const target = 9 + Math.sin(t / 3.1) * 4 + Math.random() * 6 + game * 24 + (NADIAudio.playing ? 3 : 0);
    sys.cpu += (target - sys.cpu) * 0.4;
    sys.ram += (43 + Math.sin(t / 40) * 2 + game * 12 - sys.ram) * 0.1;
    sys.gpu += (3 + Math.random() * 3 + game * 62 + (NADIAudio.playing ? 2 : 0) - sys.gpu) * 0.4;
  }, 500);
  C.get_sys_stats = () => ({ cpu_pct: sys.cpu, ram_pct: sys.ram, ram_used_gb: (sys.ram / 100) * 32, ram_total_gb: 32 });
  C.get_gpu_pct = () => sys.gpu;

  // ---------------------------------------------------------------- media: the Music app is a media session
  const media = (W.media = { idx: 0, playing: false, pos: 0, rate: 1, lastTrack: "" });
  const track = () => NADIAudio.TRACKS[media.idx];
  const mediaSnap = () => {
    const open = !!(W.win.music && W.win.music.open);
    if (!open) return { has_session: false, title: "", artist: "", playing: false, position: 0, duration: 0, can_previous: false, can_next: false, can_seek: false, can_rate: false, rate: 1, art: null, source: "" };
    const t = track();
    return {
      has_session: true,
      title: t.title,
      artist: t.artist,
      playing: media.playing,
      position: media.pos,
      duration: t.dur,
      can_previous: true,
      can_next: true,
      can_seek: true,
      can_rate: true,
      rate: media.rate,
      art: art(t.hue[0], t.hue[1]),
      source: "music",
    };
  };
  // A second session for the two-players-at-once case (off until
  // `NADI.world.mediaDuet(true)` runs in the console): a video in the
  // browser with its own track, position and speed.
  const media2 = (W.media2 = { on: false, playing: false, pos: 0, rate: 1 });
  const mediaSnap2 = () => {
    if (!media2.on) return null;
    return {
      has_session: true,
      title: "Demo reel",
      artist: "Browser",
      playing: media2.playing,
      position: media2.pos,
      duration: 320,
      can_previous: false,
      can_next: false,
      can_seek: true,
      can_rate: false,
      rate: 1,
      art: art(210, 260),
      source: "msedge.exe",
    };
  };
  W.mediaDuet = (on = true) => {
    media2.on = !!on;
    media2.playing = media2.on;
    media2.pos = 0;
    mediaEmit();
  };
  const primaryOf = (snaps) => snaps.find((m) => m.playing) || snaps[0] || mediaSnap();
  function mediaEmit() {
    const snaps = [mediaSnap()].filter((m) => m.has_session);
    const second = mediaSnap2();
    if (second) snaps.push(second);
    st.hasMedia = snaps.length > 0;
    const key = snaps.map((m) => `${m.title}|${m.artist}`).join("\n");
    if (snaps.length > 0 && key !== media.lastTrack) st.peekRequest = true;
    media.lastTrack = snaps.length > 0 ? key : "";
    NADI.emit("media-tick", snaps);
    W.onMedia?.(primaryOf(snaps));
  }
  W.mediaEmit = mediaEmit;
  W.mediaSnap = mediaSnap;
  function setPlaying(p) {
    media.playing = p;
    if (p) NADIAudio.play(media.idx, media.rate);
    else NADIAudio.pause();
    mediaEmit();
  }
  W.musicClosed = () => {
    media.playing = false;
    NADIAudio.pause();
    mediaEmit();
  };
  function skip(d) {
    media.idx = (media.idx + d + NADIAudio.TRACKS.length) % NADIAudio.TRACKS.length;
    media.pos = 0;
    NADIAudio.setTrack(media.idx);
    mediaEmit();
  }
  const isSecond = (args) => args && args.source === "msedge.exe";
  C.media_play_pause = (args) => {
    if (isSecond(args)) {
      media2.playing = !media2.playing;
      mediaEmit();
      return;
    }
    setPlaying(!media.playing);
  };
  C.media_next = (args) => {
    if (isSecond(args)) return;
    skip(1);
  };
  C.media_previous = (args) => {
    if (isSecond(args)) return;
    if (media.pos > 4) {
      media.pos = 0;
      mediaEmit();
    } else skip(-1);
  };
  C.media_seek = ({ positionSeconds, source }) => {
    if (isSecond({ source })) {
      media2.pos = clamp(positionSeconds, 0, 320);
      mediaEmit();
      return;
    }
    media.pos = clamp(positionSeconds, 0, track().dur);
    mediaEmit();
  };
  C.media_seek_by = ({ deltaSeconds, source }) => {
    if (isSecond({ source })) {
      media2.pos = clamp(media2.pos + deltaSeconds, 0, 320);
      mediaEmit();
      return;
    }
    media.pos = clamp(media.pos + deltaSeconds, 0, track().dur);
    mediaEmit();
  };
  C.media_set_rate = ({ rate, source }) => {
    if (isSecond({ source })) return true;
    media.rate = clamp(rate, 0.1, 16);
    NADIAudio.setRate(media.rate);
    mediaEmit();
    return true;
  };
  setInterval(() => {
    if (media.playing && W.win.music && W.win.music.open) {
      media.pos += 0.1 * media.rate;
      if (media.pos >= track().dur) skip(1);
    }
    if (media2.on && media2.playing) {
      media2.pos += 0.1;
      if (media2.pos >= 320) media2.pos = 0;
    }
  }, 100);
  setInterval(mediaEmit, 1000);

  // ---------------------------------------------------------------- the game
  const game = (W.game = { running: false, since: 0, pid: 4242, frames: [] });
  const gameSnap = () => ({ has_game: game.running && S().game_detection, name: "Starfall", icon: ICONS.game, count: game.running ? 1 : 0, pid: game.running ? game.pid : 0 });
  function gameEmit() {
    const snap = gameSnap();
    st.hasGame = snap.has_game;
    NADI.emit("game-tick", snap);
  }
  W.gameEmit = gameEmit;
  W.gameStart = () => {
    game.running = true;
    game.since = now();
    game.frames.length = 0;
    gameEmit();
  };
  W.gameStop = () => {
    game.running = false;
    gameEmit();
  };
  setInterval(gameEmit, 1500);
  C.get_game_stats = ({ pids }) =>
    pids
      .filter((p) => p === game.pid && game.running)
      .map((pid) => {
        const f = game.frames.slice(-120);
        const avg = f.length ? f.reduce((a, b) => a + b, 0) / f.length : 16.7;
        const sorted = [...f].sort((a, b) => a - b);
        const p99 = sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * 0.99))] : avg;
        return {
          pid,
          cpu_pct: Math.min(100, 14 + Math.sin(now() / 2) * 4 + Math.random() * 3),
          ram_gb: 4.1 + Math.sin(now() / 30) * 0.05,
          ram_pct: 12.8,
          gpu_pct: clamp(58 + Math.sin(now() / 1.7) * 14 + Math.random() * 4, 0, 100),
          vram_gb: 3.4 + Math.sin(now() / 25) * 0.1,
          fps: f.length > 10 ? 1000 / avg : null,
          low_fps: f.length > 10 ? 1000 / p99 : null,
          frame_ms: f.slice(-60),
          fps_denied: false,
        };
      });

  // ---------------------------------------------------------------- editor + git state (the coding card)
  const code = (W.code = { file: "island.rs", unsaved: false, typedLines: 0 });
  const projectInfo = () => {
    const files = [
      { name: "island.rs", path: "src/island.rs", added: 42 + code.typedLines, deleted: 7, status: "M" },
      { name: "hub.css", path: "ui/hub.css", added: 28, deleted: 3, status: "M" },
      { name: "settings.rs", path: "src/settings.rs", added: 11, deleted: 0, status: "M" },
      { name: "notes.md", path: "notes.md", added: 9, deleted: 0, status: "A" },
    ];
    const added = files.reduce((s, f) => s + f.added, 0) + 31;
    const deleted = files.reduce((s, f) => s + f.deleted, 0) + 12;
    return { name: "nadi-demo", branch: "main", files, changed: 5, added, deleted };
  };
  const editorTitle = () => `${code.unsaved ? "● " : ""}${code.file} - nadi-demo - Visual Studio Code`;

  // ---------------------------------------------------------------- the browser page and downloads
  const page = (W.page = {
    title: "How to install NADI",
    domain: "nadi.dev",
    lead: "NADI is a small pill with a pair of eyes at the top of your screen. This guide takes about two minutes.",
    buttons: ["Download installer", "Download source", "Download demo reel"],
  });
  const FILES = {
    "Download installer": ["NADI_1.1.0_x64-setup.exe", 62_000_000, 5_400_000],
    "Download source": ["nadi-source.zip", 18_500_000, 2_100_000],
    "Download demo reel": ["nadi-demo-reel.mp4", 1_240_000_000, 9_800_000],
  };
  const dl = (W.dl = { items: [], n: 0 });
  W.startDownload = (label) => {
    const f = FILES[label];
    if (!f) return false;
    if (dl.items.some((i) => i.name === f[0] && !i.done)) return true;
    dl.items = dl.items.filter((i) => i.name !== f[0]);
    dl.items.push({ id: `dl${++dl.n}`, source: "Browser", kind: "browser", name: f[0], received: 0, total: f[1], speed: 0, done: false, exe_path: APPS.browser.exe, icon: ICONS.browser, base: f[2] });
    return true;
  };
  setInterval(() => {
    const t = now();
    for (const it of dl.items) {
      if (it.done) continue;
      it.speed = Math.max(300_000, it.base * (0.8 + 0.3 * Math.sin(t / 2 + it.base) + Math.random() * 0.2));
      it.received = Math.min(it.total, it.received + it.speed * 0.5);
      if (it.received >= it.total) {
        it.done = true;
        it.speed = 0;
      }
    }
  }, 500);
  const downloadCards = () => {
    if (!dl.items.length) return [];
    const items = dl.items.slice(-3).map((i) => ({ ...i }));
    return [{ speed: items.reduce((s, i) => s + (i.done ? 0 : i.speed), 0), items }];
  };
  C.download_item_click = ({ id }) => {
    dl.items = dl.items.filter((i) => i.id !== id);
  };
  C.click_page_button = ({ label }) => !!NADI.pressPageButton?.(label);
  // the guide page's picture: a drawing of the island (the real app shows the page's own photo)
  C.page_image = () =>
    "data:image/svg+xml;utf8," +
    encodeURIComponent(
      '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 480 200"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#2b3a67"/><stop offset="1" stop-color="#7a4fa3"/></linearGradient></defs><rect width="480" height="200" fill="url(#g)"/><rect x="150" y="30" width="180" height="52" rx="26" fill="#000"/><circle cx="215" cy="56" r="9" fill="#fff"/><circle cx="265" cy="56" r="9" fill="#fff"/><circle cx="217" cy="58" r="4" fill="#000"/><circle cx="267" cy="58" r="4" fill="#000"/><rect x="70" y="118" width="340" height="14" rx="7" fill="#fff" opacity=".25"/><rect x="110" y="146" width="260" height="14" rx="7" fill="#fff" opacity=".15"/></svg>',
    );

  // ---------------------------------------------------------------- Claude usage (the hub's two rings and the peek)
  const usage = (W.usage = { five: 38, seven: 61 });
  W.usageRings = true;
  const usageSnap = () => ({
    available: !!S().llm_detection && W.usageRings,
    error: null,
    five_hour: { pct: usage.five, resets_at: new Date(Date.now() + 2.2 * 3600e3).toISOString() },
    seven_day: { pct: usage.seven, resets_at: new Date(Date.now() + 3.2 * 86400e3).toISOString() },
  });
  C.get_usage = () => usageSnap();
  C.refresh_usage = () => NADI.emit("usage-tick", usageSnap());
  function bumpUsage(five, seven) {
    const old = [usage.five, usage.seven];
    usage.five = Math.min(99, usage.five + five);
    usage.seven = Math.min(99, usage.seven + seven);
    NADI.emit("usage-tick", usageSnap());
    // maybe_peek: not over the hub, not while something else is showing
    if (st.hubOpen || notif.hasCurrent()) return;
    NADI.emit("usage-peek", { before: { five_hour: old[0], seven_day: old[1] }, after: { five_hour: usage.five, seven_day: usage.seven } });
    st.usagePeekUntil = now() + 4.4;
  }
  W.bumpUsage = bumpUsage;

  // ---------------------------------------------------------------- calendar
  const cal = (() => {
    const d = new Date();
    const at = (days, h, m) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + days, h, m).getTime();
    const soon = Math.ceil((Date.now() + 25 * 60e3) / 300e3) * 300e3;
    return [
      { uid: "e1", summary: "Design review", start_ms: soon, end_ms: soon + 30 * 60e3, all_day: false },
      { uid: "e2", summary: "Lunch with Sam", start_ms: at(Date.now() > at(0, 12, 30) ? 1 : 0, 12, 30), end_ms: at(Date.now() > at(0, 12, 30) ? 1 : 0, 13, 30), all_day: false },
      { uid: "e3", summary: "Sprint planning", start_ms: at(1, 10, 0), end_ms: at(1, 11, 0), all_day: false },
      { uid: "e4", summary: "Dentist", start_ms: at(3, 15, 0), end_ms: at(3, 15, 45), all_day: false },
      { uid: "e5", summary: "Mia's birthday", start_ms: at(5, 0, 0), end_ms: at(6, 0, 0), all_day: true },
    ];
  })();
  W.calendar = cal;
  C.get_calendar = () => ({ configured: !!S().calendar_ics_url, error: null, events: S().calendar_ics_url ? cal : [] });
  const notified = new Set();
  setInterval(() => {
    if (!S().calendar_ics_url) return;
    const lead = S().calendar_reminder_lead_min * 60e3;
    for (const ev of cal) {
      const delta = ev.start_ms - Date.now();
      if (delta > 0 && delta <= lead && !notified.has(ev.uid)) {
        notified.add(ev.uid);
        notif.push("Upcoming event", `${ev.summary} in ${Math.max(1, Math.floor(delta / 60e3))} min`);
        NADI.pushHistory();
      }
    }
  }, 30000);

  // ---------------------------------------------------------------- the time announcement (clock.rs)
  const INTERVALS = [5, 10, 15, 30, 60, 120, 180, 360];
  function clockPush(h24) {
    const d = new Date();
    const time = h24
      ? `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`
      : `${d.getHours() % 12 || 12}:${String(d.getMinutes()).padStart(2, "0")} ${d.getHours() < 12 ? "AM" : "PM"}`;
    const date = d.toLocaleDateString("en-US", { weekday: "short", month: "short", day: "numeric" }).replace(/^(\w+)\s/, "$1, ");
    notif.pushBrief(time, { id: "plugin:time", state: "plugin", host_icon: "clock", host_exe: null, project: date, ctx: 0, look: "big", dwell_ms: 4000 });
  }
  C.time_preview = () => clockPush(S().time_24h);
  W.clockPush = clockPush;
  let lastMark = null;
  setInterval(() => {
    if (!S().time_announce) return;
    const every = INTERVALS[Math.min(S().time_interval, INTERVALS.length - 1)];
    const d = new Date();
    const minute = d.getHours() * 60 + d.getMinutes();
    const mark = `${d.getDate()}:${minute}`;
    if (minute % every !== 0 || lastMark === mark) return;
    lastMark = mark;
    if (st.hubOpen || st.hasGame) return;
    clockPush(S().time_24h);
  }, 10000);

  // ---------------------------------------------------------------- Claude Code sessions (llm.rs)
  const llm = (W.llm = {
    kind: "none", // none | busy | waiting | idle
    id: "demo-session",
    title: "Add a dark mode toggle",
    prompt: "",
    feed: [],
    activity: { icon: "sparkle", text: "Thinking" },
    started: 0,
    updated: 0,
    busySince: 0,
    tokens: 0,
    cost: 0,
    added: 0,
    removed: 0,
    dismissed: null,
    timers: [],
    turns: 0,
  });
  const llmClear = () => {
    for (const t of llm.timers) clearTimeout(t);
    llm.timers = [];
  };
  const llmAfter = (s, fn) => llm.timers.push(setTimeout(fn, s * 1000));
  const llmFeed = (icon, text) => {
    if (llm.activity && llm.kind === "busy" && llm.activity.text !== "Thinking") llm.feed.unshift(llm.activity);
    llm.feed = llm.feed.slice(0, 4);
    llm.activity = { icon, text };
    llm.updated = Date.now();
  };
  // a state change of the session: what llm.rs's watcher turns into a pill
  function llmKind(next) {
    const prev = llm.kind;
    if (prev === next) return;
    llm.kind = next;
    const ts = Date.now();
    if (prev === "waiting" && next !== "waiting") notif.cancelBrief(llm.id);
    if (next === "busy") llm.busySince = ts;
    llm.updated = ts;
    if (!(S().llm_detection && S().llm_brief)) return;
    if (next === "waiting") llmBrief("waiting");
    else if (prev === "busy" && next === "idle" && ts - llm.busySince >= 5000) {
      bumpUsage(3, 1);
      llmBrief("finished");
    }
  }
  function llmBrief(state) {
    if (st.hubOpen || st.hasGame) return; // the hub shows the cards already; a game is not interrupted
    if (W.fg === "term") return; // the terminal is in front: you are looking at it
    notif.pushBrief(llm.title, { id: llm.id, state, host_icon: "term", host_exe: APPS.term.exe, project: "nadi-demo", ctx: Math.min(1, llm.tokens / 200000) });
  }
  W.llmRun = (prompt) => {
    if (llm.kind === "busy" || llm.kind === "waiting") return;
    llmClear();
    llm.prompt = prompt;
    llm.title = "Add a dark mode toggle";
    llm.feed = [];
    llm.dismissed = null;
    if (!llm.started) llm.started = Date.now();
    llm.turns++;
    llm.tokens = Math.max(llm.tokens, 61000);
    llm.activity = { icon: "sparkle", text: "Thinking" };
    llmKind("busy");
    llmAfter(2, () => llmFeed("eye", "Reading src/settings.rs"));
    llmAfter(4, () => llmFeed("search", "Searching the code"));
    llmAfter(6, () => llmFeed("pen", "Editing ui/hub.css"));
    llmAfter(8, () => {
      llm.activity = { icon: "phone", text: "Waiting for your approval" };
      W.onLlm?.();
      llmKind("waiting");
    });
    W.onLlm?.();
  };
  W.llmAnswer = (yes) => {
    if (llm.kind !== "waiting") return;
    llmClear();
    if (!yes) {
      llm.activity = { icon: "moon", text: "Idle" };
      llmKind("idle");
      llm.updated = Date.now();
      W.onLlm?.();
      return;
    }
    llm.activity = { icon: "term", text: "Running cargo test" };
    llmKind("busy");
    llmAfter(3, () => llmFeed("pen", "Editing ui/main.js"));
    llmAfter(6, () => llmFeed("term", "Running cargo build --release"));
    llmAfter(10, () => {
      llm.activity = { icon: "check", text: "Finished" };
      llmKind("idle");
      W.onLlm?.();
    });
    W.onLlm?.();
  };
  W.llmReset = () => {
    llmClear();
    notif.cancelBrief(llm.id);
    llm.kind = "none";
    llm.feed = [];
    llm.tokens = 0;
    llm.cost = 0;
    llm.added = 0;
    llm.removed = 0;
    llm.started = 0;
    W.onLlm?.();
  };
  setInterval(() => {
    if (llm.kind === "busy") {
      llm.tokens += 1400 + Math.random() * 900;
      llm.cost += 0.012 + Math.random() * 0.01;
      if (Math.random() < 0.5) llm.added += 1 + Math.floor(Math.random() * 4);
      if (Math.random() < 0.2) llm.removed += 1;
    }
  }, 1000);
  const llmState = () => {
    if (llm.kind === "waiting") return "waiting";
    if (llm.kind === "busy") return "working";
    if (llm.kind === "idle" && llm.turns > 0 && llm.activity.icon === "check" && llm.dismissed !== llm.updated && Date.now() - llm.updated < 15 * 60e3) return "finished";
    return "idle";
  };
  W.llmState = llmState;
  const llmCards = () => {
    if (llm.kind === "none") return [];
    const state = llmState();
    if (state === "idle") return [];
    return [
      {
        id: llm.id,
        pid: 5150,
        provider: "claude",
        title: llm.title,
        project: "nadi-demo",
        host: "Terminal",
        host_icon: "term",
        host_exe: APPS.term.exe,
        state,
        activity: state === "waiting" ? { icon: "phone", text: "Waiting for your approval" } : state === "finished" ? { icon: "check", text: "Finished" } : llm.activity,
        context_tokens: Math.round(llm.tokens),
        context_limit: 200000,
        model: "Sonnet 5.5",
        running_secs: Math.floor((Date.now() - llm.started) / 1000),
        prompt: llm.prompt,
        feed: llm.feed.slice(),
        cost_usd: llm.cost,
        lines_added: llm.added,
        lines_removed: llm.removed,
      },
    ];
  };
  C.llm_dismiss = () => {
    llm.dismissed = llm.updated;
  };

  // ---------------------------------------------------------------- what is in front: the work pill and the cards (work.rs, activity.rs)
  const CONFIRM_S = 3;
  const LEAVE_GRACE_S = 8;
  const RECENT_ACTIVE_S = 30;
  const NON_SESSION = ["idle", "unknown", "other"];
  const LABELS = { coding: "Coding", writing: "Writing", design: "Designing", communication: "Messaging", media: "Watching/Listening", gaming: "Gaming", browsing: "Browsing", other: "On the Computer" };
  const recent = new Map(); // category -> { last, started, app, exe, id }
  let candidate = "idle";
  let candidateSince = now();
  let candidateApp = "";
  let confirmed = null;
  let confirmedApp = "";
  let confirmedSince = now();
  function observe(cat, appId) {
    if (cat === "other") return;
    const prev = recent.get(cat);
    const fresh = prev && now() - prev.last <= RECENT_ACTIVE_S;
    recent.set(cat, { last: now(), started: fresh ? prev.started : now(), app: APPS[appId].name, exe: APPS[appId].exe, id: appId });
  }
  function workPoll() {
    if (!S().work_detection) {
      if (confirmed !== null) {
        confirmed = null;
        candidate = "idle";
        st.hasWork = false;
        NADI.emit("work-tick", { has_session: false, category: "", label: "", app_name: "", started_at_secs: 0 });
      }
      return;
    }
    // every visible window keeps its category's card alive, focused or not
    const seen = [];
    for (const id of visible()) {
      const cat = APPS[id].cat;
      if (cat === "other" || seen.includes(cat)) continue;
      seen.push(cat);
      observe(cat, id);
    }
    let state;
    let app = "";
    if (W.fg && W.win[W.fg] && W.win[W.fg].open && !W.win[W.fg].min) {
      state = APPS[W.fg].cat;
      app = APPS[W.fg].name;
      observe(state, W.fg);
    } else {
      state = "other";
    }
    if (state !== candidate) {
      candidate = state;
      candidateSince = now();
      candidateApp = app;
    }
    const inSession = confirmed !== null && !NON_SESSION.includes(confirmed);
    const dwell = inSession && NON_SESSION.includes(state) ? LEAVE_GRACE_S : CONFIRM_S;
    if (now() - candidateSince >= dwell && confirmed !== state) {
      confirmed = state;
      confirmedApp = candidateApp;
      confirmedSince = now();
    }
    const category = confirmed || "idle";
    const has = !NON_SESSION.includes(category);
    st.hasWork = has;
    // browsing a page the island understands: the pill names the page, not the activity
    const k = category === "browsing" ? W.pageKind() : null;
    NADI.emit("work-tick", {
      has_session: has,
      category,
      label: LABELS[category] || "Away",
      app_name: confirmedApp,
      started_at_secs: now() - confirmedSince,
      page_kind: k ? k.id : null,
      page_main: k ? k.main : null,
      page_sub: k ? k.sub : null,
    });
  }
  setInterval(workPoll, 1000);
  W.setFg = (id) => {
    W.fg = id;
  };

  // What the island makes of the page in the browser window (pagekind.rs): a walkthrough, found by its numbered
  // steps. Where you are on the page is not followed.
  const blockedPage = () => {
    const list = String(S().page_blocklist || "").toLowerCase().split(/[,;\n]/).map((x) => x.trim()).filter(Boolean);
    return list.some((x) => page.domain.includes(x) || page.title.toLowerCase().includes(x));
  };
  function pageKind() {
    const r = NADI.browserReading && NADI.browserReading();
    if (!r || !r.steps.length) return null;
    const total = r.steps.length;
    const fields = [
      { key: "Steps", value: `${total}`, rough: false },
      { key: "Starts with", value: r.steps[0].name, rough: false },
    ];
    return { id: "walkthrough", label: "Walkthrough", confidence: 0.97, fields, main: page.title, sub: `${total} steps` };
  }
  W.pageKind = () => (S().page_preview && !blockedPage() ? pageKind() : null);

  C.get_activity = () => {
    const snap = { games: [], downloads: [], llm: [], work: [], coding: null, plugins: [] };
    if (game.running) snap.games.push({ pid: game.pid, name: "Starfall", exe_path: APPS.game.exe, icon: ICONS.game, playtime_secs: now() - game.since });
    if (S().llm_detection) snap.llm = llmCards();
    if (S().download_detection) snap.downloads = downloadCards();
    if (!S().work_detection) return snap;
    const list = [...recent.entries()].filter(([, r]) => now() - r.last <= RECENT_ACTIVE_S).sort((a, b) => b[1].started - a[1].started);
    for (const [cat, r] of list) {
      if (cat === "coding") {
        const session = confirmed === "coding";
        const editor = recent.get("coding");
        snap.coding = {
          exe_path: APPS.code.exe,
          today_secs: 4320 + Math.floor(now() - confirmedSince),
          active: session,
          project: "nadi-demo",
          active_secs: session ? now() - confirmedSince : 0,
          file: code.file,
          language: "Rust",
          language_color: "#DEA584",
          unsaved: code.unsaved,
          project_info: projectInfo(),
        };
        void editor;
      } else {
        const browsing = cat === "browsing";
        snap.work.push({
          exe_path: r.exe,
          category: cat,
          app_name: r.app,
          icon: ICONS[r.id] || null,
          going_secs: now() - r.started,
          page: browsing && !blockedPage() ? page.title : null,
          browse: browsing
            ? blockedPage()
              ? { domain: null, preview: null, kind: null, blocked: true }
              : {
                  domain: page.domain,
                  preview: S().page_preview ? { lead: page.lead, buttons: page.buttons.map((label, i) => ({ label, x: 40, y: 120 + i * 30 })) } : null,
                  kind: W.pageKind(),
                  image: S().page_images && S().page_preview ? 1 : null,
                  blocked: false,
                }
            : null,
        });
      }
    }
    return snap;
  };

  // ---------------------------------------------------------------- the plugins (the native ones, see src/builtin)
  // The demo keeps what each switch means in the old settings, so the world above stays as it was.
  const legacyActivity = C.get_activity;
  const local = (W.pluginOn = { "now-playing": true, toasts: true });
  const patch = (o) => C.save_settings({ settings: { ...S(), ...o } });
  // what each permission grants: the app's words, the same for every plugin that asks (see src/permissions.rs)
  const GRANTS = {
    windows: ["See which programs have a window open, which one is in front, and what the windows are titled."],
    performance: ["Read how much CPU, memory and GPU each program uses, from Windows."],
    "cursor position": ["Know where the mouse cursor is on the screen."],
    audio: ["Hear what is playing as levels and bands (never the sound itself)."],
    "idle time": ["Know how long it has been since the keyboard or mouse was last used (never what was typed)."],
    "read files and folders": ["Read files and folders on this PC."],
    network: ["Reach the internet, and only the addresses the plugin names.", "Reach the internet, and only these addresses: {}."],
    "use CLI": ["Run command-line programs on this PC.", "Run a command-line program on this PC: {}."],
    notifications: ["Read the notifications that other apps show (their title and text)."],
    "browser extension": ["Receive what the NADI browser extension sends about your open tabs."],
    media: ["See what is playing: the title, the artist and how far along it is."],
    "media pictures": ["See the cover picture of what is playing."],
    "media control": ["Press play, pause, next and previous for the player."],
    calendar: ["Read the events of the island's calendar (whichever plugin brought them)."],
    clock: ["Know the date and the time."],
  };
  const grants = (name, detail) => { const g = GRANTS[name] || [""]; return detail && g[1] ? g[1].replace("{}", detail) : g[0]; };
  W.eyesReact = true;
  W.eyesRings = true;
  W.lightBrightness = 100;
  C.look_state = () => ({ eyes: S().show_eyes !== false, eyes_react: W.eyesReact, eyes_rings: W.eyesRings, brightness: W.lightBrightness, sound: S().react_to_audio !== false, ambient: S().audio_bleed ?? 60 });
  const lookTick = () => NADI.emit("look-tick", C.look_state());
  const BUILT_IN = [
    { id: "games", name: "Games", description: "The game you are playing, for how long, and how it runs.", sees: [["windows"], ["performance"]], on: () => S().game_detection, set: (v) => patch({ game_detection: v }) },
    { id: "work", name: "Work", description: "What you work on: your editor, your hours, and a pill for the session you are in.", sees: [["windows"], ["idle time"], ["use CLI", "git"]], on: () => S().work_detection, set: (v) => patch({ work_detection: v }) },
    { id: "page-reader", name: "Page Reader", description: "A card for each page you have open, and what the page in front of you says (needs the NADI browser extension).", sees: [["windows"], ["browser extension"]], on: () => S().page_preview, set: (v) => patch({ page_preview: v }), note: () => (S().page_preview ? "Reading pages in Edge." : null), settings: [{ key: "images", label: "Show the page's main picture", type: "toggle", get: () => S().page_images, set: (v) => patch({ page_images: v }) }] },
    { id: "downloads", name: "Downloads", description: "What your browsers, Steam and qBittorrent are downloading right now.", sees: [["read files and folders"]], on: () => S().download_detection, set: (v) => patch({ download_detection: v }) },
    { id: "claude-code", name: "Claude Code", description: "Your Claude Code sessions as cards, a pill when one needs you or has finished, and how much of your Claude 5-hour and 7-day limits you have used.", sees: [["read files and folders"], ["windows"], ["network", "api.anthropic.com, console.anthropic.com"]], on: () => S().llm_detection, set: (v) => { patch({ llm_detection: v }); NADI.emit("usage-tick", usageSnap()); }, actions: [{ id: "usage_login", label: "Sign In", ask: "Paste the code the Claude page shows", then: "usage_finish" }], settings: [{ key: "alerts", label: "Pill when a session needs me or finishes", type: "toggle", get: () => S().llm_brief, set: (v) => patch({ llm_brief: v }) }, { key: "usage", label: "Rings for how much of my limits is used", type: "toggle", get: () => W.usageRings, set: (v) => { W.usageRings = v; NADI.emit("usage-tick", usageSnap()); } }] },
    { id: "now-playing", name: "Now playing", description: "A pill and a card for what is playing, with play, skip, seek and speed.", sees: [["media"], ["media pictures"], ["media control"]] },
    { id: "toasts", name: "Notification mirror", description: "Shows the notifications of other apps (Discord, Viber, ...) on the island.", sees: [["notifications"], ["windows"]] },
    { id: "eyes", name: "Eyes", description: "The two eyes of the island: they blink, look at the cursor, squash and stretch as it moves, glance around when it rests, and show moods. They can bounce to the beat of what you play.", sees: [["cursor position"], ["audio"]], on: () => S().show_eyes !== false, set: (v) => { patch({ show_eyes: v }); lookTick(); }, settings: [{ key: "react", label: "React to sound", type: "toggle", get: () => W.eyesReact, set: (v) => { W.eyesReact = v; lookTick(); } }, { key: "rings", label: "Ripples on the bass", type: "toggle", get: () => W.eyesRings, set: (v) => { W.eyesRings = v; lookTick(); } }] },
    { id: "sound-light", name: "Sound light", description: "An aura that lives in the island and dances to what you play: soft lights that follow the music, ripples on the bass, a warm halo for a voice. A little of it spills out of the island.", sees: [["audio"]], on: () => S().react_to_audio !== false, set: (v) => { patch({ react_to_audio: v }); lookTick(); }, settings: [{ key: "brightness", label: "Max brightness (0 = off)", type: "range", min: 0, max: 100, step: 5, unit: "%", get: () => W.lightBrightness, set: (v) => { W.lightBrightness = v; lookTick(); } }, { key: "ambient", label: "Light outside the island", type: "range", min: 0, max: 100, step: 5, unit: "%", get: () => S().audio_bleed ?? 60, set: (v) => { patch({ audio_bleed: v }); lookTick(); } }] },
    { id: "time", kind: "wasm", name: "Time", description: "The island says what time it is now and then, in a small pill.", sees: [["clock"]], on: () => S().time_announce, set: (v) => patch({ time_announce: v }), settings: [{ key: "every", label: "Every", type: "choice", options: INTERVALS.map((m, i) => ({ value: i, label: m < 60 ? `${m} minutes` : m === 60 ? "Hour" : `${m / 60} hours` })), get: () => S().time_interval, set: (v) => patch({ time_interval: v }) }, { key: "h24", label: "24-hour clock", type: "toggle", get: () => S().time_24h, set: (v) => patch({ time_24h: v }) }], actions: [{ id: "preview", label: "Show it now" }] },
    { id: "ics-calendar", kind: "wasm", name: "ICS Calendar", description: "Brings the events of a calendar link (.ics: Google Calendar, Outlook, iCloud) to the island's calendar, shows the next ones on a card, and reminds you with a banner shortly before one starts.", sees: [["network", "the address you give it (\"Calendar link (.ics)\")"], ["calendar"]], on: () => !!S().calendar_ics_url, set: (v) => patch({ calendar_ics_url: v ? "https://calendar.google.com/calendar/ical/you%40example.com/private-demo/basic.ics" : "" }), settings: [{ key: "url", label: "Calendar link (.ics)", type: "text", get: () => S().calendar_ics_url, set: (v) => patch({ calendar_ics_url: v }) }, { key: "lead_min", label: "How long before (minutes)", type: "number", get: () => S().calendar_reminder_lead_min, set: (v) => patch({ calendar_reminder_lead_min: v }) }], note: () => (S().calendar_ics_url ? `${cal.length} events read.` : "Paste the link of a calendar (.ics) in the setting below.") },
  ];
  const info = (id) => BUILT_IN.find((p) => p.id === id);
  const isOn = (id) => (info(id)?.on ? !!info(id).on() : local[id] !== false);
  C.plugin_list = () => ({
    dnd: false,
    plugins: BUILT_IN.map((p) => ({
      id: p.id,
      name: p.name,
      description: p.description,
      version: "1.0.0",
      kind: p.kind || "native",
      bundled: true,
      enabled: isOn(p.id),
      muted: false,
      permissions: p.sees.map(([name, detail]) => ({ name, detail: detail || "", why: grants(name, detail || "") })),
      settings: (p.settings || []).map((x) => ({ key: x.key, label: x.label, type: x.type, value: x.get(), options: x.options || [], min: x.min, max: x.max, step: x.step, unit: x.unit || "" })),
      actions: p.actions || [],
      cost: null,
      error: null,
      note: p.note ? p.note() : null,
    })),
  });
  C.plugin_rescan = () => C.plugin_list();
  C.plugin_pills = () => ({ pills: [], rings: {} });
  C.plugin_set = ({ id, on }) => {
    const p = info(id);
    if (p?.set) p.set(on);
    else local[id] = on;
  };
  C.plugin_value = ({ id, key, value }) => info(id)?.settings?.find((x) => x.key === key)?.set(value);
  C.plugin_mute = () => {};
  C.plugin_dnd = () => {};
  C.plugin_folder = () => {};
  C.plugin_call = ({ id, cmd, args }) => {
    const a = args || {};
    switch (`${id}.${cmd}`) {
      case "games.stats": return C.get_game_stats({ pids: a.pids });
      case "downloads.click": return C.download_item_click({ id: a.id });
      case "claude-code.dismiss": return C.llm_dismiss({ id: a.id });
      case "page-reader.press": return C.click_page_button({ label: a.label });
      case "page-reader.image": return C.page_image({ id: a.id });
      case "now-playing.play_pause": return C.media_play_pause({ source: a.source });
      case "now-playing.next": return C.media_next({ source: a.source });
      case "now-playing.previous": return C.media_previous({ source: a.source });
      case "now-playing.seek": return C.media_seek({ positionSeconds: a.position_seconds, source: a.source });
      case "now-playing.seek_by": return C.media_seek_by({ deltaSeconds: a.delta_seconds, source: a.source });
      case "now-playing.set_rate": return C.media_set_rate({ rate: a.rate, source: a.source });
      case "claude-code.usage_login": return true;
      case "claude-code.usage_finish": return true;
      case "claude-code.usage_state": return C.get_usage();
      case "claude-code.usage_refresh": C.refresh_usage(); return true;
      case "time.preview": return C.time_preview();
      default: return null;
    }
  };
  // what the hub and the floating cards draw: the cards of each plugin that is on, by plugin id
  C.get_activity = () => {
    const a = legacyActivity();
    const cards = {};
    if (isOn("games") && a.games.length) cards.games = a.games;
    if (isOn("downloads") && a.downloads.length) cards.downloads = a.downloads;
    if (isOn("claude-code") && a.llm.length) cards["claude-code"] = a.llm;
    if (isOn("work")) cards.work = { coding: a.coding, work: a.work.filter((w) => w.category !== "browsing") };
    const pages = a.work.filter((w) => w.category === "browsing" && w.browse);
    if (isOn("page-reader") && pages.length) cards["page-reader"] = pages;
    const plugins = [];
    if (isOn("ics-calendar") && S().calendar_ics_url) {
      const events = cal.filter((e) => e.end_ms >= Date.now() - 3600e3).slice(0, 6);
      const when = (e) => `{when:${e.start_ms}${e.all_day ? ":day" : ""}}`;
      if (events.length) {
        const [first, ...rest] = events;
        plugins.push({ plugin: "ics-calendar", name: "ICS Calendar", icon: "calendar", rank: 45, title: first.summary, sub: when(first), peek: "", body: rest.length ? [{ kind: "list", rows: rest.map((e) => ({ title: e.summary, sub: when(e), progress: null })) }] : [] });
      }
    }
    return { plugins, cards, rings: {}, pills: [] };
  };

  // ---------------------------------------------------------------- windows: which app a card or pill takes you to
  C.focus_source = ({ exePath, titleHint, sourceId }) => {
    const id = (exePath && byExe(exePath)) || (sourceId && (APPS[sourceId] ? sourceId : null)) || (titleHint === "nadi-demo" ? "code" : null);
    if (id) NADI.focusApp?.(id, true);
  };

  // ---------------------------------------------------------------- backgrounds: the visitor's own image or video
  async function pick(kind) {
    return new Promise((resolve, reject) => {
      const input = document.createElement("input");
      input.type = "file";
      input.accept = "image/*,video/mp4,video/webm";
      NADI.st.picking = true;
      input.addEventListener("change", () => {
        NADI.st.picking = false;
        const f = input.files && input.files[0];
        if (!f) return resolve(null);
        if (f.size > 60 * 1024 * 1024) return reject("That file is too large for the demo.");
        const url = `${URL.createObjectURL(f)}#/${kind}-${Date.now()}-${f.name.replace(/[\\/#?]/g, "_")}`;
        const s = { ...S(), ["bg_" + kind]: url };
        Object.assign(S(), s);
        resolve({ ...s });
      });
      input.addEventListener("cancel", () => {
        NADI.st.picking = false;
        resolve(null);
      });
      input.click();
    });
  }
  C.pick_background = ({ kind }) => pick(kind);
  C.clear_background = ({ kind }) => {
    S()["bg_" + kind] = "";
    return { ...S() };
  };

  // ---------------------------------------------------------------- go
  NADI.onStart = () => {
    mediaEmit();
    gameEmit();
    workPoll();
    NADI.emit("usage-tick", usageSnap());
    NADI.emit("calendar-tick", C.get_calendar());
  };
})();
