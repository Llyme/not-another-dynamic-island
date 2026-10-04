# NADI — Not Another Dynamic Island

A Dynamic Island for the Windows desktop. A small pill sits at the top edge of the screen with a pair of
eyes, and shows what is going on: what is playing, which game or project you are in, what is downloading,
which notifications came in, what is next on your calendar. Click it and it expands into a hub panel.

NADI is built with [Tauri v2](https://v2.tauri.app/): a Rust backend and a vanilla JS/canvas frontend.
It is Windows-only for now. The goal is to stay light: it should never make the computer lag.

## What it shows

- **Eyes.** The idle pill. They follow the cursor and react to the sound playing on the system. The sound's
  light can bleed outside the island, like YouTube's ambient mode.
- **Media.** Now-playing with controls and a seek bar, from the Windows media sessions — one card per player when several play at once.
- **Games and work.** Detects running games, and what you are working on, even when the window is not
  focused. The coding card shows the git changes of the project (branch, files, added and deleted lines);
  the browsing card shows what is in the page and gives you buttons to click it from the island.
- **Guide reader.** A walkthrough, a FAQ or a wiki article (its infobox as facts, then its sections) is laid out to be read in its card, so it can sit over the game
  while you play: headings, paragraphs, lists and tables, a chip for each day or section, search, text size
  (one button: S, M, L, XL). The search, the text size and the contents show only while the mouse is over the card. The layout follows the card's width: narrow, the picture and the facts sit above the text; from
  480 px a small portrait with the facts that matter; from 640 px a rail on the side while the text scrolls beside it. Previous, Next and the guide's contents take the browser tab itself to that section (only within the
  site, only when you press), and the card follows the tab: what it shows is always what the page has. The pictures inside the text (a table's item icons, a figure) are shown too, each one as it scrolls near: the extension takes the browser's own copy when the site lets it, else fetches it without cookies, shrinks it, and sends it one at a time with a short pause between downloads; NADI never contacts the site. Plain-text FAQs are cut into headings and paragraphs, with the original text one press away.
- **Page reader.** The island knows what kind of page you are on (walkthrough, wiki article, news
  article, video, recipe, Q&A thread, API reference, product page, search results, web app such as a pull
  request) and says what matters for that kind: how many steps a guide has, the sections of an article, the
  answers on a question, a post with its comments. The work pill names the page. Where you are on the page is
  not followed. It
  needs the browser extension ([nadi-chrome](https://github.com/Llyme/nadi-chrome), Chrome, Edge, Brave, Opera,
  Vivaldi): the page describes itself (its metadata, headings, comments), exactly, and costs the
  browser next to nothing. Without the extension the Page Reader switch in Settings is locked, and a browser is
  not read at all (the card keeps to the window title). The extension connects over a WebSocket on 127.0.0.1
  (ports 47653 to 47657) and NADI only accepts a browser extension's origin. Private and incognito windows and
  banking, mail, health and password-manager sites are never read, nor are sign-in pages. Nothing is stored or sent.
  Without the extension the Page Reader switch in Settings > General is locked.
- **Downloads.** Active downloads from browsers, Steam and qBittorrent, in one card. Finished ones open
  their folder or can be put away with a click.
- **Notifications.** Captures the notifications of all apps.
- **Calendar.** Upcoming events from an `.ics` feed, with a reminder.
- **Claude usage.** Usage rings and a peek, using your existing Claude Code login.
- **Hub.** The expanded panel. Every card starts collapsed and opens with a click on its header.
- **Floating cards.** Drag a card out of the hub by its header and it becomes a card of its own on the screen:
  it lifts, tilts with your hand and snaps to screen edges and to other floating cards. Resize it from its
  corner (it snaps to the screen's edge, to other cards, to its own height and to the width it opens with; a double press on the corner puts it back to its first size), press its X (or let go of it over the island) to put it back. It stays over a game that runs in a
  window or borderless, and it never takes the focus from it; the mouse only reaches a card while it is over
  it, everywhere else it goes to the game. With the Fullscreen Guard on, cards also let every click through to a
  fullscreen game, like the island does; hold the guard's key (Alt, or Ctrl: it is a setting, and the guard can be off) to use them (or the island) anyway. Floating cards are not kept: after a
  restart every card is back in the island. All floating cards share one extra window, made the first time you drag one out.

To call the island, rest the cursor on the top edge of a monitor: a glow builds up, and the island lands.
Right-click the island to pin or unpin it. Settings are in the tray icon's menu.

## Website

`docs/index.html` is a static showcase page with a live demo: the app's own frontend (`ui/`) runs in an iframe
on a mock desktop. `docs/demo/backend.js` is a JavaScript port of the Rust window logic (springs, edge
dwell, notification queue, settings) and `docs/demo/world.js` simulates the PC the island watches (media,
game, downloads, Claude Code sessions...), so what you see and feel there is what the app does.

`docs/demo/ui/` is a copy of `ui/`. After changing the app's UI, run `docs/demo/sync.ps1` to refresh it.

To publish, set GitHub Pages to deploy from the `main` branch and the `/docs` folder; the page is then at
`https://<user>.github.io/<repository>/`. To try it locally, serve the folder (`python -m http.server` in `docs`):
the frame does not load from `file://`.

## Requirements

- Windows 10 or 11 (with the WebView2 runtime, which is part of Windows 11)
- [Rust](https://rustup.rs/) and the Tauri CLI: `cargo install tauri-cli --version "^2"`

## Run and build

```powershell
cd src-tauri
cargo tauri dev      # run it
cargo tauri build    # release exe and NSIS installer
```

The installer is written to `src-tauri/target/release/bundle/nsis/`. Tests: `cargo test` in `src-tauri/`
(a few tests that need real apps running are marked `#[ignore]`).

Debug aids, set as environment variables before starting the exe: `DI_OPEN_HUB=1` opens the hub,
`DI_OPEN_SETTINGS=1` opens the settings pane, `DI_DEMO_DOWNLOADS=1` shows fake downloads.

## Where things live

- `ui/` is the frontend: `index.html`, `main.js`, `style.css` for the compact island and `hub.css` for
  the hub. There is no build step.
- `src-tauri/src/` is the backend. `lib.rs` has the window state machine (reveal, hide, springs, resize);
  there is one module per feature: `media`, `audio`, `game`, `work`, `project`, `browse`, `pagetext`,
  `downloads`, `notify` and `notify_listener`, `calendar`, `usage`, `stats` and `gpu`, `settings`.
- `scripts/gen_icons.py` regenerates `src-tauri/icons/` (standard library only).
- Settings are stored in `%APPDATA%\NADI\settings.json`.

## How it works

The window is one frameless, transparent, always-on-top Tauri window. A 16 ms loop in `lib.rs` polls the
global cursor position (the webview only gets mouse events while the cursor is over it) and drives
everything: the edge dwell and glow, the slide in and out, the spring resize between views, and
click-through. The window is a little larger than the island, and the margin is click-through, so the
glow can shine outside the island without blocking the windows underneath.

Background work (game and work detection, downloads, reading the page) runs on low-priority threads
and only does what is needed while something is shown.
