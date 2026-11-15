param(
    [Parameter(Mandatory = $true, HelpMessage = "Nouvelle version semver, ex: 1.0.1 ou 1.1.0")]
    [string]$Version,
    [string]$Repo = "lghio/lgo-video-shrinker",
    [switch]$Watch
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Format de version invalide '$Version'. Utilisez le format X.Y.Z (ex: 1.0.1)."
}

$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot

Write-Host "=== Preparation de la release VideoShrinker v$Version ===" -ForegroundColor Cyan

# 1. Mettre a jour package.json
$pkgPath = Join-Path $projectRoot "package.json"
$pkg = Get-Content -Raw $pkgPath | ConvertFrom-Json
$pkg.version = $Version
$pkgJson = $pkg | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText($pkgPath, $pkgJson + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[1/4] package.json -> $Version"

# 2. Mettre a jour src-tauri/tauri.conf.json
$tauriConfPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$tauriConf = Get-Content -Raw $tauriConfPath | ConvertFrom-Json
$tauriConf.version = $Version
$tauriConfJson = $tauriConf | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText($tauriConfPath, $tauriConfJson + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[2/4] src-tauri/tauri.conf.json -> $Version"

# 3. Mettre a jour src-tauri/Cargo.toml
$cargoPath = Join-Path $projectRoot "src-tauri\Cargo.toml"
$cargoContent = Get-Content -Raw $cargoPath
$cargoContent = [regex]::Replace($cargoContent, '(?m)^version\s*=\s*"[^"]+"', "version = `"$Version`"", 1)
[System.IO.File]::WriteAllText($cargoPath, $cargoContent, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[3/4] src-tauri/Cargo.toml -> $Version"

# 4. Commit, tag et push
Write-Host "[4/4] Creation du commit et du tag git v$Version ..." -ForegroundColor Cyan
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
git commit -m "chore(release): v$Version`n`nCo-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
git tag "v$Version"
git push origin HEAD
git push origin "v$Version"

Write-Host "✅ Tag v$Version pousse sur GitHub ! Le workflow '.github/workflows/release.yml' demarre." -ForegroundColor Green
Write-Host "🔗 Releases : https://github.com/$Repo/releases" -ForegroundColor Cyan

if ($Watch -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    Start-Sleep -Seconds 5
    gh run watch --repo $Repo --exit-status
}
