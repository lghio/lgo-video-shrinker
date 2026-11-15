@echo off
setlocal enabledelayedexpansion
chcp 65001 >nul

cd /d "%~dp0"

:: ─────────────────────────────────────────────
:: Ce script s'exécute en deux phases :
::   Phase 1 (sans admin) : Python + PyInstaller
::   Phase 2 (avec admin) : npm + electron-builder
:: L'argument --electron-only est passé lors du
:: re-lancement en admin pour la phase 2.
:: ─────────────────────────────────────────────

if /i "%~1"=="--electron-only" goto ELECTRON_PHASE

:: ══════════════════════════════════════════════
:: PHASE 1 — Build Python (sans droits admin)
:: ══════════════════════════════════════════════
echo ============================================
echo  VideoShrinker — Build COMPLET (Python + Electron)
echo ============================================

echo.
echo [1/6] Verification de Python...
python --version >nul 2>&1
if errorlevel 1 (
    echo [ERREUR] Python non trouve dans le PATH.
    pause & exit /b 1
)
echo       OK

echo.
echo [2/6] Installation des dependances Python...
pip install -r requirements.txt --quiet
pip install pyinstaller --quiet
echo       OK

echo.
echo [3/6] Build PyInstaller...
taskkill /f /im VideoShrinker.exe >nul 2>&1
timeout /t 1 /nobreak >nul
if exist build rmdir /s /q build
if exist dist  rmdir /s /q dist

pyinstaller VideoShrinker.spec --clean --noconfirm
if errorlevel 1 (
    echo [ERREUR] PyInstaller a echoue.
    pause & exit /b 1
)
if not exist "dist\VideoShrinker.exe" (
    echo [ERREUR] dist\VideoShrinker.exe introuvable apres compilation.
    pause & exit /b 1
)
echo       OK — dist\VideoShrinker.exe cree

:: ─────────────────────────────────────────────
:: Re-lancement en admin pour la phase Electron
:: ─────────────────────────────────────────────
echo.
echo [4/6] Elevation des privileges pour electron-builder...
powershell -Command "Start-Process -FilePath '%~f0' -ArgumentList '--electron-only' -Verb RunAs -Wait"
exit /b

:: ══════════════════════════════════════════════
:: PHASE 2 — Build Electron (avec droits admin)
:: ══════════════════════════════════════════════
:ELECTRON_PHASE
echo ============================================
echo  VideoShrinker — Phase Electron (admin)
echo ============================================

echo.
echo [5/6] Verification de Node.js...
set "PATH=C:\Program Files\nodejs;%APPDATA%\npm;%PATH%"
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERREUR] Node.js introuvable. Installez-le depuis https://nodejs.org/
    pause & exit /b 1
)
echo       OK

cd electron
if not exist node_modules (
    call npm install
    if %errorlevel% neq 0 ( echo [ERREUR] npm install a echoue & pause & exit /b 1 )
) else (
    echo       Dependances deja installees.
)

echo.
echo [6/6] Construction du package Electron...
set "CSC_IDENTITY_AUTO_DISCOVERY=false"
set "WIN_CSC_LINK="
set "CSC_LINK="

call npm run dist
if %errorlevel% neq 0 ( echo [ERREUR] electron-builder a echoue & pause & exit /b 1 )

echo.
echo ============================================
echo  Build COMPLET termine avec succes !
echo  Backend   : dist\VideoShrinker.exe
echo  Installeur: electron\dist\
echo ============================================
dir /b "dist\*.exe" 2>nul

cd ..
pause
endlocal