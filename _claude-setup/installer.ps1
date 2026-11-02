# Installe la config Claude Code (à lancer une seule fois)
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$docs = Split-Path -Parent (Split-Path -Parent $here)
$glob = Join-Path $env:USERPROFILE ".claude"

# 1. Global : règles + agents (dispo dans tous les projets)
New-Item -ItemType Directory -Force "$glob\agents" | Out-Null
if (Test-Path "$glob\CLAUDE.md") { Copy-Item "$glob\CLAUDE.md" "$glob\CLAUDE.md.bak" -Force }
Copy-Item "$here\global\CLAUDE.md" "$glob\CLAUDE.md" -Force
Copy-Item "$here\global\agents\*.md" "$glob\agents\" -Force

# 2. Permissions par projet
foreach ($p in @(@{src="worktable"; dst="$docs\WorkTable"}, @{src="roulette"; dst="$docs\RouletteApp"})) {
  New-Item -ItemType Directory -Force "$($p.dst)\.claude" | Out-Null
  $f = "$($p.dst)\.claude\settings.json"
  if (Test-Path $f) { Copy-Item $f "$f.bak" -Force }
  Copy-Item "$here\$($p.src)\settings.json" $f -Force
}
Write-Host "OK - config Claude Code installee. Tu peux supprimer le dossier _claude-setup." -ForegroundColor Green
