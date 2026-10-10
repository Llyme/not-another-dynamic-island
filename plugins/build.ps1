# Builds the plugins the app carries (time, ics-calendar: their .wasm goes to src-tauri/plugins-bundled, where the app embeds it)
# and the examples (audio-meter, game-clock, busy-meter, weather: each becomes a folder in dist/, ready to be dropped in the
# plugins folder, %APPDATA%/NADI/plugins). The examples are not part of the app.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
Push-Location $here
try {
  rustup target add wasm32-unknown-unknown | Out-Null
  cargo build --release --target wasm32-unknown-unknown
  $out = Join-Path $here "target/wasm32-unknown-unknown/release"
  $dst = Join-Path $here "../src-tauri/plugins-bundled"
  Copy-Item (Join-Path $out "time.wasm") (Join-Path $dst "time/time.wasm") -Force
  Copy-Item (Join-Path $out "ics_calendar.wasm") (Join-Path $dst "ics-calendar/ics-calendar.wasm") -Force
  Get-ChildItem $dst -Recurse -Filter *.wasm | ForEach-Object { "{0}  {1:N0} bytes" -f $_.Name, $_.Length }
  # the examples: a module (its .wasm and manifest.json), or a manifest alone
  $examples = @(
    @{ dir = "audio-meter"; wasm = "audio_meter.wasm"; as = "audio-meter.wasm" },
    @{ dir = "game-clock"; wasm = "game_clock.wasm"; as = "game-clock.wasm" },
    @{ dir = "busy-meter"; wasm = "busy_meter.wasm"; as = "busy-meter.wasm" },
    @{ dir = "weather"; wasm = $null; as = $null }
  )
  foreach ($e in $examples) {
    $to = Join-Path $here ("dist/" + $e.dir)
    New-Item -ItemType Directory -Force $to | Out-Null
    Copy-Item (Join-Path $here ($e.dir + "/manifest.json")) (Join-Path $to "manifest.json") -Force
    if ($e.wasm) { Copy-Item (Join-Path $out $e.wasm) (Join-Path $to $e.as) -Force }
    Get-ChildItem $to | ForEach-Object { "dist/{0}/{1}  {2:N0} bytes" -f $e.dir, $_.Name, $_.Length }
  }
} finally { Pop-Location }
