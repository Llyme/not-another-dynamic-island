# Builds NADI's installer: NADI first, then the installer that carries it. Out: dist\NADI-Setup-<version>.exe
# (CARGO_TARGET_DIR is used when it is set.)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $here "target" }
$exe = Join-Path $target "release\nadi.exe"
Push-Location $here
try {
  # (a nadi.exe that runs cannot be written over: moved aside, it is built anew)
  if (Test-Path $exe) {
    try { [IO.File]::Open($exe, 'Open', 'ReadWrite', 'None').Close() } catch { Rename-Item $exe ("nadi.running-" + [DateTimeOffset]::Now.ToUnixTimeSeconds() + ".exe") }
  }
  # (cargo writes its progress to stderr: not an error here)
  $ErrorActionPreference = "Continue"
  cargo build --release -p nadi-shell
  if ($LASTEXITCODE -ne 0) { throw "NADI did not build" }
  $env:NADI_EXE = $exe
  cargo build --release -p nadi-setup
  if ($LASTEXITCODE -ne 0) { throw "the installer did not build" }
  $ErrorActionPreference = "Stop"
  $version = (Select-String -Path (Join-Path $here "crates\nadi-setup\Cargo.toml") -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
  $dist = Join-Path $here "dist"
  New-Item -ItemType Directory -Force $dist | Out-Null
  $out = Join-Path $dist "NADI-Setup-$version.exe"
  Copy-Item (Join-Path $target "release\nadi-setup.exe") $out -Force
  "{0}  {1:N0} bytes" -f $out, (Get-Item $out).Length
} finally { Pop-Location }
