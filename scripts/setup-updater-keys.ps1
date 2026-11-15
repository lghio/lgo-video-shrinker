param(
    [string]$Repo = "lghio/lgo-video-shrinker",
    [string]$KeyPath = "$env:USERPROFILE\.tauri\videoshrinker.key"
)

$ErrorActionPreference = "Stop"
$env:PATH = "$env:USERPROFILE\.cargo\bin;C:\Program Files\nodejs;$env:APPDATA\npm;$env:PATH"

$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot

$keyDir = Split-Path -Parent $KeyPath
if (-not (Test-Path $keyDir)) {
    New-Item -ItemType Directory -Force -Path $keyDir | Out-Null
}

if (-not (Test-Path $KeyPath)) {
    Write-Host "[1/3] Generation de la paire de cles Minisign Tauri dans $KeyPath ..." -ForegroundColor Cyan
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    & npx.cmd tauri signer generate -w $KeyPath --ci 2>&1 | Out-Null
    $ErrorActionPreference = $prevEap
} else {
    Write-Host "[1/3] Cle existante trouvee dans $KeyPath" -ForegroundColor Yellow
}

$pubKeyFile = "$KeyPath.pub"
if (-not (Test-Path $pubKeyFile)) {
    throw "Cle publique introuvable : $pubKeyFile"
}

$pubKey = (Get-Content -Raw $pubKeyFile).Trim()
$privKey = (Get-Content -Raw $KeyPath).Trim()

Write-Host "[2/3] Mise a jour de plugins.updater.pubkey dans src-tauri/tauri.conf.json ..." -ForegroundColor Cyan
$tauriConfPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$conf = Get-Content -Raw $tauriConfPath | ConvertFrom-Json
$conf.plugins.updater.pubkey = $pubKey
$conf.plugins.updater.endpoints = @("https://github.com/$Repo/releases/latest/download/latest.json")
$confJson = $conf | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText($tauriConfPath, $confJson + "`n", (New-Object System.Text.UTF8Encoding($false)))

if (Get-Command gh -ErrorAction SilentlyContinue) {
    Write-Host "[3/3] Envoi du secret TAURI_SIGNING_PRIVATE_KEY sur GitHub ($Repo) via gh CLI ..." -ForegroundColor Cyan
    $prevEap = $ErrorActionPreference
    $ErrorActionPreference = "Continue"
    $privKey | gh secret set TAURI_SIGNING_PRIVATE_KEY --repo $Repo
    if ($LASTEXITCODE -eq 0) {
        Write-Host "✅ Secret TAURI_SIGNING_PRIVATE_KEY configure sur GitHub ($Repo) !" -ForegroundColor Green
    } else {
        Write-Warning "Impossible d'envoyer le secret automatiquement via gh CLI."
    }
    $ErrorActionPreference = $prevEap
} else {
    Write-Warning "gh CLI non trouve. Ajoutez manuellement TAURI_SIGNING_PRIVATE_KEY dans les Secrets GitHub Actions de $Repo."
}

Write-Host "`nConfiguration terminee ! Votre cle publique est dans src-tauri/tauri.conf.json." -ForegroundColor Green
