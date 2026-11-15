@echo off
setlocal enabledelayedexpansion

echo ============================================
echo  VideoShrinker — Build Electron
echo ============================================

cd /d "%~dp0electron"

:: Vérifier Node.js
where node >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERREUR] Node.js introuvable. Installez Node.js depuis https://nodejs.org/
    pause & exit /b 1
)

:: Installer les dépendances npm si nécessaire
if not exist node_modules (
    echo [1/3] Installation des dependances npm...
    call npm install
    if %errorlevel% neq 0 ( echo [ERREUR] npm install a echoue & pause & exit /b 1 )
) else (
    echo [1/3] Dependances npm deja installees.
)

:: Vérifier que le backend PyInstaller existe
if not exist "..\dist\VideoShrinker\VideoShrinker.exe" (
    echo [AVERTISSEMENT] Backend PyInstaller introuvable dans dist\VideoShrinker\
    echo Lancez d'abord build.bat pour construire le backend Python.
    echo.
    echo Le lancement en mode DEV utilisera Python directement.
)

echo [2/3] Lancement de l'application Electron (mode dev)...
call npm start

endlocal
