@echo off
setlocal enabledelayedexpansion

:: Auto-élévation en Administrateur (requis pour créer les symlinks de winCodeSign)
net session >nul 2>&1
if %errorlevel% neq 0 (
    echo Elevation des privileges administrateur requise...
    powershell -Command "Start-Process -FilePath '%~f0' -Verb RunAs -Wait" 2>nul
    exit /b
)

echo ============================================
echo  VideoShrinker — Build Electron uniquement
echo ============================================

cd /d "%~dp0electron"

:: Ajouter les chemins Node.js en tête du PATH courant
set "PATH=C:\Program Files\nodejs;%APPDATA%\npm;%PATH%"

:: Vérifier Node.js
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERREUR] Node.js introuvable meme apres refresh du PATH.
    echo Verifiez l'installation depuis https://nodejs.org/ puis relancez une nouvelle fenetre cmd.
    pause & exit /b 1
)

:: Installer les dépendances npm si nécessaire
if not exist node_modules (
    echo [1/2] Installation des dependances npm...
    call npm install
    if %errorlevel% neq 0 ( echo [ERREUR] npm install a echoue & pause & exit /b 1 )
) else (
    echo [1/2] Dependances npm deja installees.
)

:: Vérifier que le backend PyInstaller existe
if not exist "..\dist\VideoShrinker\VideoShrinker.exe" (
    echo.
    echo [AVERTISSEMENT] Backend introuvable dans dist\VideoShrinker\VideoShrinker.exe
    echo Lancez build.bat d'abord pour construire le backend Python.
    echo.
    set /p CONTINUE="Continuer quand meme ? (o/N) : "
    if /i not "!CONTINUE!"=="o" ( echo Annule. & pause & exit /b 1 )
)

:: Désactiver la signature de code (pas de certificat disponible)
set "CSC_IDENTITY_AUTO_DISCOVERY=false"
set "WIN_CSC_LINK="
set "CSC_LINK="

:: Build electron-builder
echo [2/2] Construction du package Electron...
call npm run dist
if %errorlevel% neq 0 ( echo [ERREUR] electron-builder a echoue & pause & exit /b 1 )

echo.
echo ============================================
echo  Build Electron termine !
echo  Installeur disponible dans : electron\dist\
echo ============================================
dir /b "dist\*.exe" 2>nul

cd ..
pause
endlocal
