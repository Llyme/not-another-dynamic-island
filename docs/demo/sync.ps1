# Copies the app's real frontend (../ui) into demo/ui, unchanged, and plugs the demo shim in front
# of it. Run it again after the app's UI changes, so the site always shows what the app really is.
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$src = Join-Path $here "..\..\ui"
$dst = Join-Path $here "ui"
New-Item -ItemType Directory -Force $dst | Out-Null
foreach ($f in "style.css", "hub.css", "main.js", "kit.js", "pluginui.js", "floats.js") { Copy-Item (Join-Path $src $f) (Join-Path $dst $f) -Force }
# the faces of the native plugins
New-Item -ItemType Directory -Force (Join-Path $dst "plugins") | Out-Null
Get-ChildItem (Join-Path $dst "plugins") -Filter *.js | Where-Object { -not (Test-Path (Join-Path (Join-Path $src "plugins") $_.Name)) } | Remove-Item -Force
Get-ChildItem (Join-Path $src "plugins") -Filter *.js | ForEach-Object { Copy-Item $_.FullName (Join-Path $dst "plugins") -Force }
$html = Get-Content -Raw (Join-Path $src "index.html")
$html = $html.Replace('<link rel="stylesheet" href="style.css" />', '<script src="../shim.js"></script>' + "`n    " + '<link rel="stylesheet" href="style.css" />')
[System.IO.File]::WriteAllText((Join-Path $dst "index.html"), $html, (New-Object System.Text.UTF8Encoding $false))
Write-Host "synced ui -> docs/demo/ui"
