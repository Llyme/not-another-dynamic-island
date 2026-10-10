# NADI plugins (modules)

`sdk/` is the Rust SDK for a module plugin (see [PLUGINS.md](../PLUGINS.md) for the format).

* `time/` and `agenda/` are plugins the app carries (the clock pill, and the card and reminders of the calendar). They are
  written like any plugin: read them to see how a module keeps time (`wake_in`), reads the calendar and answers a button.
* `audio-meter/` and `game-clock/` are two examples that are put in the plugins folder once.
* `busy-meter/` is an example that is not carried: a ring in the hub, a pill with a button, the `system` sensor. After
  `build.ps1`, `dist/busy-meter/` is a folder to drop in `%APPDATA%/NADI/plugins/`.

```
powershell ./build.ps1     # builds them all, and copies the .wasm into ../src-tauri/plugins-bundled (NADI embeds them)
cargo test                 # the modules' own logic runs as a normal Rust test: a module is a function from an event to a reply
```

Needs `rustup target add wasm32-unknown-unknown` (the script does it). A plugin of your own: copy `busy-meter`, change
`handle`, build, and put the `.wasm` and a `manifest.json` in `%APPDATA%/NADI/plugins/<id>/`.
