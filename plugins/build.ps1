# Builds the plugins that ship with NADI (and the examples) and puts the modules where NADI embeds them (src-tauri/plugins-bundled).
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $here
try {
  rustup target add wasm32-unknown-unknown | Out-Null
  cargo build --release --target wasm32-unknown-unknown
  $out = Join-Path $here "target/wasm32-unknown-unknown/release"
  $dst = Join-Path $here "../src-tauri/plugins-bundled"
  Copy-Item (Join-Path $out "audio_meter.wasm") (Join-Path $dst "example-audio-meter/audio-meter.wasm") -Force
  Copy-Item (Join-Path $out "game_clock.wasm") (Join-Path $dst "example-game-clock/game-clock.wasm") -Force
  Copy-Item (Join-Path $out "time.wasm") (Join-Path $dst "time/time.wasm") -Force
  Copy-Item (Join-Path $out "agenda.wasm") (Join-Path $dst "agenda/agenda.wasm") -Force
  # an example the app does not carry: a folder to drop in the plugins folder (%APPDATA%/NADI/plugins)
  $busy = Join-Path $here "dist/busy-meter"
  New-Item -ItemType Directory -Force $busy | Out-Null
  Copy-Item (Join-Path $out "busy_meter.wasm") (Join-Path $busy "busy-meter.wasm") -Force
  Copy-Item (Join-Path $here "busy-meter/manifest.json") (Join-Path $busy "manifest.json") -Force
  Get-ChildItem $dst -Recurse -Filter *.wasm | ForEach-Object { "{0}  {1:N0} bytes" -f $_.Name, $_.Length }
  Get-ChildItem $busy -Filter *.wasm | ForEach-Object { "dist/busy-meter/{0}  {1:N0} bytes" -f $_.Name, $_.Length }
} finally { Pop-Location }
