# NADI

Widgets round the edges of your Windows screen, merged into the edges like a notch, or floating just off them. NADI can take the
taskbar's place too: a dock, a launcher on the Windows key, and quick settings. It is a native Windows app, written in Rust with
DirectComposition and Direct2D. It aims never to make the computer lag: at rest it uses well under 1% of the CPU and about 50 MB.
Windows only. Alpha.

## What it does

- **Widgets on every edge and corner.**
  - Hover over a widget, or click it, to open its panel (or set panels to open on a click only, and shut with a click elsewhere).
    An open panel pushes the other widgets aside.
  - Right-click a widget to pin it to a monitor, keep it always shown, or keep its panel always open.
  - Hold a widget to enter Edit Mode: every widget comes out, and a tray in the middle holds the hidden ones and those of plugins
    that are off. Drag widgets between the edges, the corners and the tray; drag the dock's apps to move, pin or unpin them. A
    click on an empty spot ends it.
  - Right-click a widget for its menu: merged or floating, where it goes, icon only, pinned to a monitor, always shown, always open,
    hidden when idle.
  - Widgets can always show, or only come out when the mouse reaches an edge (that edge's, or all of them) or comes near.
  - Several widgets can share a corner: one in it, the others in a row along its top or bottom edge, a column along its side, or
    both. Drop one over the corner's row or column to join it.
  - They get out of the way of a full-screen app, and if you like, of a maximized one (on any monitor, focused or not). A key
    (Alt or Ctrl) can be made to be held for them to come out then.
  - A plugin's banners come out of its own widget: the widget becomes the banner for a moment, and the widgets beside it make room.
    Move the widget, and its banners come out there.
- **Plugins.** Every widget comes from a plugin, written in Luau and run sandboxed. They install from a GitHub repository in
  Settings. The official ones are in [nadi-plugins](https://github.com/Llyme/nadi-plugins):
  - Claude Code sessions, now playing, calendar, battery, system meters;
  - downloads, games, work, notifications, page reader;
  - two faces: eyes, and a sound light.
- **The taskbar's place.**
  - **Dock:** pinned and open apps, with live pictures of their windows.
  - **Launcher:** grows out of the dock: pinned apps, open windows (live pictures) and recent files; as you type, one list.
  - **Tray:** NADI can hold the notification area and draw other apps' icons in a Tray widget.
  - **Quick settings:** volume, power, the last notifications.
  - **Hidden taskbar:** NADI gives the screen back to the windows, and a watchdog brings the taskbar back if NADI ends.
- **One frame for several monitors.** It follows the mouse, or shows on every monitor, or on the main one only.

## Build and run

```powershell
cd native
cargo build --release -p nadi-shell
.\target\release\nadi.exe
```

The frame keeps its settings in `%APPDATA%\NADI`. Its plugins live in `%APPDATA%\NADI\plugins`. Right-click a widget, or the tray
icon, for the frame's settings.

## Layout

- `native/crates/nadi-core`: the plugin host (Luau), the sensors (media, sound, battery, system, windows, downloads, Claude Code, the
  browser extension), and the repository installer.
- `native/crates/nadi-shell`: the frame, the dock, the launcher, the faces, and the settings window (WebView2, made only while it is
  open).
- `native/PLUGINS.md`: how to write a plugin.
- `native/GPU-DRIVERS.md`: graphics driver crashes (AMD's), and what NADI does about them.

`cargo test` runs the core's and the shell's tests. `nadi-plugin test <folder>` runs a plugin's own tests.

## License

See [LICENSE](LICENSE).
