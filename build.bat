@echo off
chcp 65001 >nul
echo ============================================
echo   Video Shrinker — Build PyInstaller
echo ============================================
echo.

:: Aller dans le répertoire du script
cd /d "%~dp0"

:: Vérifier que Python est disponible
python --version >nul 2>&1
if errorlevel 1 (
    echo [ERREUR] Python non trouvé dans le PATH.
    pause
    exit /b 1
)

:: Installer / mettre à jour les dépendances
echo [1/4] Installation des dépendances...
pip install -r requirements.txt --quiet
pip install pyinstaller --quiet
echo       OK

:: Nettoyer les builds précédents
echo [2/4] Nettoyage des anciens builds...
:: Tuer le process s'il tourne encore
taskkill /f /im VideoShrinker.exe >nul 2>&1
timeout /t 1 /nobreak >nul
if exist build rmdir /s /q build
if exist dist  rmdir /s /q dist
echo       OK

:: Lancer PyInstaller avec le fichier .spec
echo [3/4] Compilation avec PyInstaller...
pyinstaller VideoShrinker.spec --clean --noconfirm
if errorlevel 1 (
    echo.
    echo [ERREUR] La compilation a échoué. Voir les messages ci-dessus.
    pause
    exit /b 1
)

:: Vérifier que l'exe a été créé
if not exist "dist\VideoShrinker.exe" (
    echo [ERREUR] dist\VideoShrinker.exe non trouvé après la compilation.
    pause
    exit /b 1
)

echo [4/4] FFmpeg embarqué via imageio-ffmpeg (inclus automatiquement).
echo       Pas de copie manuelle nécessaire.

echo.
echo ============================================
echo   Build terminé avec succès !
echo   Exécutable : dist\VideoShrinker.exe
echo ============================================
echo.
echo Pour distribuer l'application, copiez le dossier dist\ complet.
echo (ffmpeg.exe et ffprobe.exe doivent être présents à côté de VideoShrinker.exe)
echo.
pause
