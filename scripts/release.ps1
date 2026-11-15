param(
    [Parameter(Mandatory = $true, HelpMessage = "Nouvelle version semver, ex: 1.0.1 ou 1.1.0")]
    [string]$Version,
    [string]$Repo = "lghio/lgo-video-shrinker",
    [switch]$Watch,
    [switch]$Force
)

$ErrorActionPreference = "Stop"

if ($Version -notmatch '^\d+\.\d+\.\d+$') {
    throw "Format de version invalide '$Version'. Utilisez le format X.Y.Z (ex: 1.0.1)."
}

$projectRoot = Split-Path -Parent $PSScriptRoot
Set-Location $projectRoot

Write-Host "=== Preparation de la release VideoShrinker v$Version ===" -ForegroundColor Cyan

# 1. Pre-verifications Git (arbre propre & tag inexistant)
Write-Host "[1/6] Verification de l'arbre Git et du tag v$Version ..." -ForegroundColor Cyan
if (-not $Force) {
    $gitStatus = git status --porcelain
    if (-not [string]::IsNullOrWhiteSpace(($gitStatus -join ""))) {
        throw "L'arbre de travail Git n'est pas propre. Veuillez commiter ou remiser vos modifications avant de lancer une release (ou utilisez -Force)."
    }
}

$existingTag = git tag -l "v$Version"
if (-not [string]::IsNullOrWhiteSpace(($existingTag -join ""))) {
    throw "Le tag Git 'v$Version' existe deja localement. Choisissez un nouveau numero de version."
}

# 2. Execution de la suite TNR (build frontend + tests Rust)
Write-Host "[2/6] Execution des tests de non-regression (TNR) ..." -ForegroundColor Cyan
npm run build
if ($LASTEXITCODE -ne 0) {
    throw "Echec du build frontend ('npm run build'). Release annulee avant toute modification des fichiers de version."
}

cargo test --manifest-path src-tauri/Cargo.toml --lib
if ($LASTEXITCODE -ne 0) {
    throw "Echec des tests Rust ('cargo test'). Release annulee avant toute modification des fichiers de version."
}

# 3. Mettre a jour package.json
$pkgPath = Join-Path $projectRoot "package.json"
$pkg = Get-Content -Raw -Encoding UTF8 $pkgPath | ConvertFrom-Json
$pkg.version = $Version
$pkgJson = $pkg | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText($pkgPath, $pkgJson + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[3/6] package.json -> $Version"

# 4. Mettre a jour src-tauri/tauri.conf.json
$tauriConfPath = Join-Path $projectRoot "src-tauri\tauri.conf.json"
$tauriConf = Get-Content -Raw -Encoding UTF8 $tauriConfPath | ConvertFrom-Json
$tauriConf.version = $Version
$tauriConfJson = $tauriConf | ConvertTo-Json -Depth 10
[System.IO.File]::WriteAllText($tauriConfPath, $tauriConfJson + "`n", (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[4/6] src-tauri/tauri.conf.json -> $Version"

# 5. Mettre a jour src-tauri/Cargo.toml
$cargoPath = Join-Path $projectRoot "src-tauri\Cargo.toml"
$cargoContent = Get-Content -Raw -Encoding UTF8 $cargoPath
$cargoContent = [regex]::Replace($cargoContent, '(?m)^version\s*=\s*"[^"]+"', "version = `"$Version`"", 1)
[System.IO.File]::WriteAllText($cargoPath, $cargoContent, (New-Object System.Text.UTF8Encoding($false)))
Write-Host "[5/6] src-tauri/Cargo.toml -> $Version"

# 6. Commit, tag et push
Write-Host "[6/6] Creation du commit et du tag git v$Version ..." -ForegroundColor Cyan
git add package.json src-tauri/tauri.conf.json src-tauri/Cargo.toml
git commit -m "chore(release): v$Version`n`nCo-authored-by: Copilot <223556219+Copilot@users.noreply.github.com>"
git tag "v$Version"
git push origin HEAD
git push origin "v$Version"

Write-Host "[OK] Tag v$Version pousse sur GitHub ! Le workflow '.github/workflows/release.yml' demarre." -ForegroundColor Green
Write-Host "[URL] Releases : https://github.com/$Repo/releases" -ForegroundColor Cyan

if ($Watch -and (Get-Command gh -ErrorAction SilentlyContinue)) {
    Start-Sleep -Seconds 5
    gh run watch --repo $Repo --exit-status
}
