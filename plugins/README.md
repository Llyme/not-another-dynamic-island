# NADI plugins (modules)

`sdk/` is the Rust SDK for a module plugin (see [PLUGINS.md](../PLUGINS.md) for the format).

* `time/` and `ics-calendar/` are plugins the app carries (the clock pill; the calendar link with its card and reminders). They
  are written like any plugin: read them to see how a module keeps time (`wake_in`), reads the calendar and answers a button.
  `ics-calendar/` also shows how a module names a calendar link that the island reads for it (its `manifest.json`).
* The examples are not part of the app: they are here to read and to copy from. Each is a folder with a `manifest.json`
  (and, for a module, its Rust source). After `build.ps1`, `dist/<name>/` is a folder to drop in `%APPDATA%/NADI/plugins/`.
  * `weather/`: a declarative plugin (no code): a card, and a pill, from a web API.
  * `audio-meter/`: a module that hears the sound (levels and bands) and draws a card that moves with it.
  * `game-clock/`: a module that sees which programs have a window, and says how long a game has been on.
  * `busy-meter/`: a ring in the hub, a pill with a button, the `system` sensor.

```
powershell ./build.ps1     # builds them all: time and ics-calendar go into ../src-tauri/plugins-bundled (NADI embeds them), the examples into dist/
cargo test                 # the modules' own logic runs as a normal Rust test: a module is a function from an event to a reply
```

Needs `rustup target add wasm32-unknown-unknown` (the script does it). A plugin of your own: copy `busy-meter`, change
`handle`, build, and put the `.wasm` and a `manifest.json` in `%APPDATA%/NADI/plugins/<id>/`.
