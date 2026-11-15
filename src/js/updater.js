import { check } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { invoke } from '@tauri-apps/api/core';

let pendingUpdate = null;

/**
 * Initialise la vérification automatique et manuelle des mises à jour Tauri.
 * @param {(level: string, msg: string) => void} logFn Fonction d'écriture dans le panneau de logs
 */
export function initUpdater(logFn = () => {}) {
    const banner = document.getElementById('updateBanner');
    const versionText = document.getElementById('updateVersionText');
    const notesText = document.getElementById('updateNotesText');
    const progressContainer = document.getElementById('updateProgressContainer');
    const progressBar = document.getElementById('updateProgressBar');
    const statusText = document.getElementById('updateStatusText');
    const installBtn = document.getElementById('updateInstallBtn');
    const dismissBtn = document.getElementById('updateDismissBtn');
    const checkBtn = document.getElementById('checkUpdateBtn');

    if (dismissBtn && banner) {
        dismissBtn.addEventListener('click', () => {
            banner.classList.add('d-none');
        });
    }

    if (installBtn) {
        installBtn.addEventListener('click', async () => {
            if (!pendingUpdate) return;

            installBtn.disabled = true;
            if (dismissBtn) dismissBtn.disabled = true;
            if (progressContainer) progressContainer.classList.remove('d-none');

            let downloaded = 0;
            let contentLength = 0;

            try {
                logFn('INFO', `[Updater] Téléchargement de la mise à jour v${pendingUpdate.version}...`);
                await pendingUpdate.downloadAndInstall((event) => {
                    switch (event.event) {
                        case 'Started':
                            contentLength = event.data.contentLength || 0;
                            if (statusText) statusText.textContent = 'Téléchargement démarré...';
                            break;
                        case 'Progress': {
                            downloaded += event.data.chunkLength || 0;
                            const pct = contentLength > 0
                                ? Math.min(100, Math.round((downloaded / contentLength) * 100))
                                : 50;
                            if (progressBar) {
                                progressBar.style.width = `${pct}%`;
                                progressBar.textContent = `${pct}%`;
                            }
                            if (statusText) {
                                const mb = (downloaded / (1024 * 1024)).toFixed(1);
                                statusText.textContent = `Téléchargement : ${mb} MB (${pct}%)`;
                            }
                            break;
                        }
                        case 'Finished':
                            if (progressBar) {
                                progressBar.style.width = '100%';
                                progressBar.textContent = '100%';
                            }
                            if (statusText) statusText.textContent = 'Installation terminée, redémarrage...';
                            break;
                    }
                });

                logFn('SUCCESS', '[Updater] Mise à jour installée. Nettoyage des processus avant redémarrage...');
                try {
                    await invoke('cleanup_all_compressions');
                } catch (cleanupErr) {
                    console.warn('[Updater] Erreur cleanup_all_compressions:', cleanupErr);
                }

                await relaunch();
            } catch (err) {
                const msg = err?.message || String(err);
                logFn('ERROR', `[Updater] Échec de l'installation de la mise à jour : ${msg}`);
                if (statusText) statusText.textContent = `Erreur : ${msg}`;
                installBtn.disabled = false;
                if (dismissBtn) dismissBtn.disabled = false;
            }
        });
    }

    if (checkBtn) {
        checkBtn.addEventListener('click', () => checkForAppUpdates(true, logFn));
    }

    // Vérification automatique silencieuse au démarrage
    checkForAppUpdates(false, logFn);
}

/**
 * Vérifie si une nouvelle version est publiée sur GitHub Releases.
 * @param {boolean} isManual Vrai si déclenché manuellement par l'utilisateur
 * @param {(level: string, msg: string) => void} logFn
 */
export async function checkForAppUpdates(isManual = false, logFn = () => {}) {
    const banner = document.getElementById('updateBanner');
    const versionText = document.getElementById('updateVersionText');
    const notesText = document.getElementById('updateNotesText');
    const checkBtn = document.getElementById('checkUpdateBtn');

    if (isManual && checkBtn) {
        checkBtn.disabled = true;
        checkBtn.textContent = '⏳ Vérification...';
    }

    try {
        if (isManual) {
            logFn('INFO', '[Updater] Vérification des mises à jour en cours...');
        }
        const update = await check();
        if (update?.available) {
            pendingUpdate = update;
            if (versionText) versionText.textContent = `v${update.version}`;
            if (notesText) {
                notesText.textContent = update.body
                    ? update.body.replace(/\r?\n/g, ' — ')
                    : 'Améliorations de performance et corrections de bugs.';
            }
            if (banner) banner.classList.remove('d-none');
            logFn('SUCCESS', `[Updater] Nouvelle version disponible : v${update.version}`);
        } else {
            logFn('INFO', '[Updater] Application à jour.');
            if (isManual && checkBtn) {
                checkBtn.textContent = '✅ À jour';
                setTimeout(() => {
                    checkBtn.textContent = '🔄 Vérifier les mises à jour';
                }, 2500);
            }
        }
    } catch (err) {
        const msg = err?.message || String(err);
        // En mode dev ou si aucune release n'est encore publiée sur GitHub, on loggue sobrement
        logFn('INFO', `[Updater] Vérification distante indisponible (${msg})`);
        if (isManual && checkBtn) {
            checkBtn.textContent = 'ℹ️ Aucune release publiée';
            setTimeout(() => {
                checkBtn.textContent = '🔄 Vérifier les mises à jour';
            }, 2500);
        }
    } finally {
        if (isManual && checkBtn) {
            checkBtn.disabled = false;
            if (checkBtn.textContent === '⏳ Vérification...') {
                checkBtn.textContent = '🔄 Vérifier les mises à jour';
            }
        }
    }
}
