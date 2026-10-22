# Construit l'installeur Windows de WorkTable.
#   1. compile la version finale optimisée (cargo build --profile dist) ;
#   2. lit la version dans Cargo.toml ;
#   3. appelle Inno Setup (ISCC.exe) sur installer\WorkTable.iss.
# Résultat : installer\Output\WorkTable-<version>-setup.exe
# Prérequis : Rust (cargo) et Inno Setup 6 (https://jrsoftware.org/isdl.php).

$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    cargo build --profile dist
    if ($LASTEXITCODE -ne 0) { throw "La compilation a échoué." }

    $version = (Select-String -Path "Cargo.toml" -Pattern '^version\s*=\s*"([^"]+)"' | Select-Object -First 1).Matches[0].Groups[1].Value

    $iscc = @(
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { Test-Path $_ } | Select-Object -First 1
    if (-not $iscc) { throw "Inno Setup 6 introuvable : installez-le depuis https://jrsoftware.org/isdl.php" }

    & $iscc "/DAppVersion=$version" "installer\WorkTable.iss"
    if ($LASTEXITCODE -ne 0) { throw "La création de l'installeur a échoué." }
    Write-Host "Installeur prêt : installer\Output\WorkTable-$version-setup.exe"
}
finally {
    Pop-Location
}
