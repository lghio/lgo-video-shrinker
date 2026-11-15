const { app, BrowserWindow, Menu } = require('electron');
const { spawn } = require('child_process');
const path = require('path');
const http = require('http');
const fs = require('fs');

// ──────────────────────────────────────────────
// Configuration
// ──────────────────────────────────────────────
const BACKEND_PORT = 8000;
const BACKEND_URL  = `http://127.0.0.1:${BACKEND_PORT}`;
const READY_TIMEOUT_MS = 30_000;   // 30 s max pour démarrer le backend

let backendProcess = null;
let mainWindow     = null;

// ──────────────────────────────────────────────
// Résolution du chemin vers le backend
// ──────────────────────────────────────────────
function getBackendPath() {
    if (app.isPackaged) {
        // Dans l'exe distribué, le backend est dans resources/backend/
        return path.join(process.resourcesPath, 'backend', 'VideoShrinker.exe');
    }
    // En développement, chercher le .exe PyInstaller dans dist/
    return path.join(__dirname, '..', 'dist', 'VideoShrinker', 'VideoShrinker.exe');
}

// ──────────────────────────────────────────────
// Démarrer le process backend
// ──────────────────────────────────────────────
function startBackend() {
    const backendPath = getBackendPath();
    const exeExists = fs.existsSync(backendPath);
    console.log('[Electron] Backend exe :', backendPath);
    console.log('[Electron] Exe trouvé :', exeExists);

    if (app.isPackaged || exeExists) {
        console.log('[Electron] Démarrage en mode EXE');
        backendProcess = spawn(backendPath, [], {
            detached: false,
            windowsHide: true,
        });
    } else {
        // Fallback : lancer directement avec Python (dev sans exe)
        const pythonScript = path.join(__dirname, '..', 'app.py');
        const venvPython = path.join(__dirname, '..', 'venv', 'Scripts', 'python.exe');
        const pythonExe = fs.existsSync(venvPython) ? venvPython : 'python';
        console.log('[Electron] Démarrage en mode Python dev :', pythonExe, pythonScript);
        backendProcess = spawn(pythonExe, [pythonScript, '--no-browser'], {
            cwd: path.join(__dirname, '..'),
            windowsHide: true,
        });
    }

    backendProcess.stdout.on('data', (data) => {
        process.stdout.write(`[Backend] ${data}`);
    });
    backendProcess.stderr.on('data', (data) => {
        process.stderr.write(`[Backend ERR] ${data}`);
    });
    backendProcess.on('exit', (code) => {
        console.log(`[Electron] Backend terminé (code ${code})`);
    });
}

// ──────────────────────────────────────────────
// Attendre que le backend soit prêt
// ──────────────────────────────────────────────
function waitForBackend(timeout) {
    return new Promise((resolve, reject) => {
        const start = Date.now();

        function poll() {
            http.get(BACKEND_URL, (res) => {
                if (res.statusCode < 500) {
                    resolve();
                } else {
                    retry();
                }
                res.resume();
            }).on('error', () => {
                retry();
            });
        }

        function retry() {
            if (Date.now() - start > timeout) {
                reject(new Error('Backend timeout'));
                return;
            }
            setTimeout(poll, 500);
        }

        poll();
    });
}

// ──────────────────────────────────────────────
// Fenêtre principale
// ──────────────────────────────────────────────
function createWindow() {
    mainWindow = new BrowserWindow({
        width:  1400,
        height: 900,
        minWidth:  900,
        minHeight: 600,
        title: 'VideoShrinker',
        webPreferences: {
            preload: path.join(__dirname, 'preload.js'),
            contextIsolation: true,
            nodeIntegration: false,
        },
    });

    // Supprimer la barre de menu native
    Menu.setApplicationMenu(null);

    mainWindow.loadURL(BACKEND_URL);

    mainWindow.on('closed', () => {
        mainWindow = null;
    });
}

// ──────────────────────────────────────────────
// Cycle de vie Electron
// ──────────────────────────────────────────────
app.whenReady().then(async () => {
    startBackend();

    try {
        console.log('[Electron] En attente du backend...');
        await waitForBackend(READY_TIMEOUT_MS);
        console.log('[Electron] Backend prêt — ouverture de la fenêtre');
        createWindow();
    } catch (err) {
        console.error('[Electron] Impossible de joindre le backend :', err.message);
        app.quit();
    }
});

app.on('window-all-closed', () => {
    // Tuer le backend proprement
    if (backendProcess && !backendProcess.killed) {
        console.log('[Electron] Arrêt du backend...');
        try {
            // Sur Windows, kill() ne termine pas toujours les enfants
            const { execSync } = require('child_process');
            execSync(`taskkill /PID ${backendProcess.pid} /T /F`, { stdio: 'ignore' });
        } catch (_) {
            backendProcess.kill();
        }
        backendProcess = null;
    }

    if (process.platform !== 'darwin') {
        app.quit();
    }
});

app.on('activate', () => {
    if (mainWindow === null) createWindow();
});
