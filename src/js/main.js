import 'bootstrap/dist/css/bootstrap.min.css';
import * as bootstrap from 'bootstrap';
import '../css/style.css';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { ask, open as openDialog } from '@tauri-apps/plugin-dialog';
import { initUpdater } from './updater.js';

let videosData = [];
let activeCompressions = {}; // { [videoIndex]: { unlisten: Function, cancel: Function } }
let batchCancelled = false;  // Flag pour arrêter la compression par lots

document.addEventListener('DOMContentLoaded', async () => {
    document.getElementById('scanBtn')?.addEventListener('click', scanVideos);
    document.getElementById('selectAllBtn')?.addEventListener('click', selectAll);
    document.getElementById('deselectAllBtn')?.addEventListener('click', deselectAll);
    document.getElementById('batchCompressBtn')?.addEventListener('click', batchCompress);
    document.getElementById('batchStopBtn')?.addEventListener('click', stopBatchCompress);
    document.getElementById('batchDeleteOriginalsBtn')?.addEventListener('click', openBatchDeleteModal);
    document.getElementById('logClearBtn')?.addEventListener('click', clearLogs);
    document.getElementById('logToggleBtn')?.addEventListener('click', toggleLogPanel);
    document.getElementById('browseDirectoryBtn')?.addEventListener('click', browseDirectory);

    await initLogListener();
    await loadAppInfo();
    initUpdater(appendLog);
    console.log('[Init] Tous les listeners Tauri enregistrés');
});

async function loadAppInfo() {
    try {
        const info = await invoke('get_app_info');
        const scanInput = document.getElementById('scanDirectory');
        if (scanInput && !scanInput.value && info.default_directory) {
            scanInput.value = info.default_directory;
        }
        const versionBadge = document.getElementById('appVersionBadge');
        if (versionBadge && info.version) {
            versionBadge.textContent = `v${info.version}`;
        }
        const hwBadge = document.getElementById('hwEncoderBadge');
        if (hwBadge && info.hw_encoder) {
            hwBadge.textContent = `⚡ Encodeur : ${info.hw_encoder}`;
        }
        appendLog('INFO', `[Init] VideoShrinker v${info.version} prêt | Encodeur matériel : ${info.hw_encoder}`);
    } catch (error) {
        console.error('Erreur get_app_info:', error);
        appendLog('ERROR', `[Init] Impossible de récupérer les informations de l'application : ${error}`);
    }
}

async function browseDirectory() {
    const btn     = document.getElementById('browseDirectoryBtn');
    const spinner = document.getElementById('browseSpinner');
    const text    = document.getElementById('browseText');

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = 'En attente…';

    try {
        // Appel via la commande Tauri browse_directory (ou fallback plugin-dialog)
        const data = await invoke('browse_directory');
        if (data && data.status === 'ok' && data.path) {
            document.getElementById('scanDirectory').value = data.path;
        }
    } catch (error) {
        try {
            const selected = await openDialog({
                directory: true,
                multiple: false,
                title: 'Choisir le répertoire à scanner',
            });
            if (selected && typeof selected === 'string') {
                document.getElementById('scanDirectory').value = selected;
            }
        } catch (dialogErr) {
            console.error('Erreur browse-directory:', dialogErr);
        }
    } finally {
        btn.disabled = false;
        spinner.classList.add('d-none');
        text.textContent = '📁 Parcourir\u2026';
    }
}

async function scanVideos() {
    const btn = document.getElementById('scanBtn');
    const spinner = document.getElementById('scanSpinner');
    const text = document.getElementById('scanText');
    const minSize = document.getElementById('minSize').value;
    const directory = document.getElementById('scanDirectory').value.trim();

    if (!directory) {
        alert('Veuillez entrer un répertoire à scanner');
        return;
    }

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = ' Scan en cours...';

    try {
        const data = await invoke('scan_videos', {
            directory,
            minSizeGb: parseFloat(minSize),
        });

        videosData = data.videos || [];

        // Afficher les statistiques
        document.getElementById('statsCard').classList.remove('d-none');
        document.getElementById('videoCount').textContent = data.count ?? 0;
        document.getElementById('totalSize').textContent = (data.total_size_gb ?? 0) + ' GB';
        document.getElementById('potentialSaving').textContent = '~' + ((data.total_size_gb ?? 0) * 0.5).toFixed(2) + ' GB';

        // Afficher le répertoire scanné
        if (data.scanned_directory) {
            document.getElementById('scanDirectory').value = data.scanned_directory;
        }

        // Afficher les vidéos
        displayVideos(videosData);

        // Afficher les actions groupées
        if (videosData.length > 0) {
            document.getElementById('batchActions').classList.remove('d-none');
        }
        updateSelectedCount();

        text.textContent = '✅ Scan terminé';
    } catch (error) {
        console.error('Erreur:', error);
        text.textContent = '❌ Erreur lors du scan';
        const errMsg = typeof error === 'string' ? error : (error?.message || String(error));
        alert('Erreur: ' + errMsg);
    } finally {
        btn.disabled = false;
        spinner.classList.add('d-none');
        setTimeout(() => {
            text.textContent = '📁 Scanner à nouveau';
        }, 2000);
    }
}

function displayVideos(videos) {
    const videoList = document.getElementById('videoList');
    videoList.innerHTML = '';

    if (!videos || videos.length === 0) {
        videoList.innerHTML = '<div class="col"><div class="alert alert-info">Aucune vidéo trouvée.</div></div>';
        document.getElementById('batchActions').classList.add('d-none');
        return;
    }

    videos.forEach((video, index) => {
        const card = createVideoCard(video, index);
        videoList.appendChild(card);
    });
}

function createVideoCard(video, index) {
    const col = document.createElement('div');
    col.className = 'col';

    const duration = video.duration ? formatDuration(video.duration) : 'N/A';
    const resolution = video.resolution || 'N/A';
    const bitrate = video.bitrate ? formatBitrate(video.bitrate) : 'N/A';

    const isCompressed = video.is_already_compressed || false;
    const checkboxChecked = !isCompressed;

    col.innerHTML = `
        <div class="card video-card">
            <div class="video-header">
                <div class="d-flex justify-content-between align-items-center">
                    <div class="flex-grow-1">
                        <h5 class="mb-1">🎥 ${video.name}</h5>
                        <small>${video.relative_path}</small>
                    </div>
                    <div class="form-check">
                        <input class="form-check-input video-checkbox" type="checkbox" 
                               id="checkbox-${index}" data-index="${index}" 
                               ${checkboxChecked ? 'checked' : ''}>
                        <label class="form-check-label text-white" for="checkbox-${index}">
                            ${isCompressed ? '✓ Déjà compressé' : 'À compresser'}
                        </label>
                    </div>
                </div>
            </div>
            <div class="card-body">
                <div class="video-info">
                    <div class="info-item">
                        <strong>Taille</strong>
                        <span>${video.size_gb} GB</span>
                    </div>
                    <div class="info-item">
                        <strong>Durée</strong>
                        <span>${duration}</span>
                    </div>
                    <div class="info-item">
                        <strong>Résolution</strong>
                        <span>${resolution}</span>
                    </div>
                    <div class="info-item">
                        <strong>Bitrate</strong>
                        <span>${bitrate}</span>
                    </div>
                </div>
                
                <div class="file-paths mt-3">
                    <div class="path-item mb-2">
                        <div class="d-flex justify-content-between align-items-center mb-1">
                            <strong>📄 Fichier original :</strong>
                            <button class="btn btn-sm btn-primary play-original-btn" data-index="${index}" title="Lire dans VLC">
                                ▶️ Lire l'original
                            </button>
                        </div>
                        <div class="path-display">${video.path}</div>
                    </div>
                    <div class="path-item compressed-path-item ${isCompressed ? '' : 'd-none'}">
                        <div class="d-flex justify-content-between align-items-center mb-1">
                            <strong>📦 Fichier compressé :</strong>
                            <button class="btn btn-sm btn-success play-compressed-btn ${isCompressed ? '' : 'd-none'}" data-index="${index}" title="Lire dans VLC">
                                ▶️ Lire compressé
                            </button>
                        </div>
                        <div class="path-display">${video.compressed_path || 'Pas encore compressé'}</div>
                    </div>
                </div>
                
                <div class="quality-selector">
                    <button class="quality-btn" data-quality="low">
                        <div>🏃 Rapide</div>
                        <small>Qualité basse</small>
                    </button>
                    <button class="quality-btn active" data-quality="medium">
                        <div>⚖️ Équilibré</div>
                        <small>Qualité moyenne</small>
                    </button>
                    <button class="quality-btn" data-quality="high">
                        <div>🎯 Lent</div>
                        <small>Haute qualité</small>
                    </button>
                </div>
                
                <div class="d-flex gap-2 mb-2">
                    <button class="btn btn-info open-folder-btn" data-index="${index}" title="Ouvrir le répertoire dans l'explorateur">
                        📂 Ouvrir le dossier
                    </button>
                </div>
                
                <div class="d-flex gap-2">
                    <button class="btn btn-success flex-grow-1 compress-btn" data-index="${index}">
                        🗜️ Compresser la vidéo
                    </button>
                    <button class="btn btn-danger delete-btn ${isCompressed ? '' : 'd-none'}" data-index="${index}" title="Supprimer le fichier original">
                        🗑️ Supprimer l'original
                    </button>
                </div>
                
                <div class="compression-status mt-3 d-none">
                    <div class="d-flex justify-content-between align-items-center mb-2">
                        <small class="status-text">En attente...</small>
                        <button class="btn btn-danger btn-sm stop-compress-btn" data-index="${index}">
                            ⏹️ Arrêter
                        </button>
                    </div>
                    <div class="progress">
                        <div class="progress-bar progress-bar-striped progress-bar-animated" 
                             role="progressbar" style="width: 0%">0%</div>
                    </div>
                </div>
                
                <div class="compression-result d-none"></div>
            </div>
        </div>
    `;

    // Sélection de la qualité
    const qualityBtns = col.querySelectorAll('.quality-btn');
    qualityBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            qualityBtns.forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
        });
    });

    // Bouton de compression
    const compressBtn = col.querySelector('.compress-btn');
    compressBtn.addEventListener('click', () => compressVideo(index, col));

    // Checkbox pour la sélection
    const checkbox = col.querySelector('.video-checkbox');
    checkbox.addEventListener('change', updateSelectedCount);

    // Bouton de suppression
    const deleteBtn = col.querySelector('.delete-btn');
    if (deleteBtn) {
        deleteBtn.addEventListener('click', () => deleteOriginalFile(index, col));
    }

    // Bouton d'ouverture du dossier
    const openFolderBtn = col.querySelector('.open-folder-btn');
    if (openFolderBtn) {
        openFolderBtn.addEventListener('click', () => openFolder(index));
    }

    // Bouton de lecture fichier original
    const playOriginalBtn = col.querySelector('.play-original-btn');
    if (playOriginalBtn) {
        playOriginalBtn.addEventListener('click', () => playVideo(video.path, 'original'));
    }

    // Bouton de lecture fichier compressé
    const playCompressedBtn = col.querySelector('.play-compressed-btn');
    if (playCompressedBtn) {
        playCompressedBtn.addEventListener('click', () => {
            const compressedPath = col.querySelector('.compressed-path-item .path-display').textContent;
            if (compressedPath && compressedPath !== 'Pas encore compressé') {
                playVideo(compressedPath, 'compressé');
            }
        });
    }

    // Bouton d'arrêt de compression
    const stopCompressBtn = col.querySelector('.stop-compress-btn');
    if (stopCompressBtn) {
        stopCompressBtn.addEventListener('click', () => stopCompression(index));
    }

    return col;
}

function updateSelectedCount() {
    const checkboxes = document.querySelectorAll('.video-checkbox:checked');
    document.getElementById('selectedCount').textContent = checkboxes.length;
}

function selectAll() {
    document.querySelectorAll('.video-checkbox').forEach(cb => cb.checked = true);
    updateSelectedCount();
}

function deselectAll() {
    document.querySelectorAll('.video-checkbox').forEach(cb => cb.checked = false);
    updateSelectedCount();
}

async function batchCompress() {
    const checkboxes = document.querySelectorAll('.video-checkbox:checked');
    if (checkboxes.length === 0) {
        alert('Veuillez sélectionner au moins une vidéo à compresser');
        return;
    }

    const btn = document.getElementById('batchCompressBtn');
    const spinner = document.getElementById('batchSpinner');
    const text = document.getElementById('batchText');
    const progressDiv = document.getElementById('batchProgress');
    const progressBar = document.getElementById('batchProgressBar');
    const currentVideoText = document.getElementById('currentVideoText');
    const progressText = document.getElementById('progressText');

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = ' Compression en cours...';
    progressDiv.classList.remove('d-none');

    const selectedIndices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.index, 10));
    const total = selectedIndices.length;
    let completed = 0;

    batchCancelled = false;

    for (const index of selectedIndices) {
        if (batchCancelled) {
            currentVideoText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
            progressBar.classList.remove('progress-bar-animated');
            progressBar.classList.add('bg-warning');
            break;
        }

        const video = videosData[index];
        currentVideoText.textContent = `Compression : ${video.name}`;
        progressText.textContent = `${completed} / ${total}`;

        const cardElement = document.querySelector(`[data-index="${index}"]`).closest('.col');
        const qualityBtn = cardElement.querySelector('.quality-btn.active');
        const quality = qualityBtn ? qualityBtn.getAttribute('data-quality') : 'medium';

        try {
            const result = await compressVideoAsync(index, cardElement, quality);
            completed++;

            const checkbox = cardElement.querySelector('.video-checkbox');
            checkbox.checked = false;
            checkbox.nextElementSibling.textContent = '✓ Déjà compressé';

            if (result && result.output_path) {
                videosData[index].is_already_compressed = true;
                videosData[index].compressed_path = result.output_path;

                const compressedPathItem = cardElement.querySelector('.compressed-path-item');
                const compressedPathDisplay = compressedPathItem.querySelector('.path-display');
                compressedPathDisplay.textContent = result.output_path;
                compressedPathItem.classList.remove('d-none');

                const deleteBtn = cardElement.querySelector('.delete-btn');
                deleteBtn.classList.remove('d-none');
                deleteBtn.dataset.index = index;

                const playCompressedBtn = cardElement.querySelector('.play-compressed-btn');
                if (playCompressedBtn) {
                    playCompressedBtn.classList.remove('d-none');
                }
            }
        } catch (error) {
            console.error(`Erreur lors de la compression de ${video.name}:`, error);
            if (error.message && error.message.includes('annulée')) {
                break;
            }
            completed++;
        }

        const overallProgress = (completed / total) * 100;
        progressBar.style.width = overallProgress + '%';
        progressBar.textContent = Math.round(overallProgress) + '%';
        progressText.textContent = `${completed} / ${total}`;
    }

    if (!batchCancelled) {
        progressBar.classList.remove('progress-bar-animated');
        currentVideoText.textContent = '✅ Toutes les compressions sont terminées!';
        text.textContent = '✅ Terminé';
    } else {
        text.textContent = '⏹️ Arrêté';
    }

    btn.disabled = false;
    spinner.classList.add('d-none');
    updateSelectedCount();

    setTimeout(() => {
        progressDiv.classList.add('d-none');
        text.textContent = '🗜️ Compresser la sélection';
        progressBar.classList.add('progress-bar-animated');
        progressBar.classList.remove('bg-warning');
        progressBar.style.width = '0%';
    }, 5000);
}

function compressVideoAsync(index, cardElement, quality) {
    return new Promise(async (resolve, reject) => {
        const video = videosData[index];
        const compressBtn = cardElement.querySelector('.compress-btn');
        const statusDiv = cardElement.querySelector('.compression-status');
        const resultDiv = cardElement.querySelector('.compression-result');
        const progressBar = statusDiv.querySelector('.progress-bar');
        const statusText = statusDiv.querySelector('.status-text');

        if (compressBtn) compressBtn.disabled = true;
        statusDiv.classList.remove('d-none');
        statusDiv.style.display = 'block';
        resultDiv.classList.add('d-none');
        progressBar.classList.remove('bg-danger', 'bg-warning');
        progressBar.classList.add('progress-bar-animated');

        let settled = false;
        let unlistenFn = null;

        const cleanup = () => {
            if (unlistenFn) {
                unlistenFn();
                unlistenFn = null;
            }
            delete activeCompressions[index];
        };

        try {
            unlistenFn = await listen('compression://event', (event) => {
                const data = event.payload;
                if (!data || data.video_index !== index) return;

                if (data.status === 'started') {
                    statusText.textContent = 'Compression démarrée...';
                    progressBar.style.width = '0%';
                    progressBar.textContent = '0%';
                } else if (data.status === 'progress') {
                    const progress = data.progress ?? 0;
                    progressBar.style.width = progress + '%';
                    progressBar.textContent = progress + '%';
                    progressBar.setAttribute('aria-valuenow', progress);
                    const cur = (data.current_time ?? 0).toFixed(0);
                    const tot = (data.total_duration ?? 0).toFixed(0);
                    statusText.textContent = `${cur}s / ${tot}s`;
                } else if (data.status === 'completed') {
                    if (settled) return;
                    settled = true;
                    cleanup();

                    progressBar.style.width = '100%';
                    progressBar.textContent = '100%';
                    progressBar.classList.remove('progress-bar-animated');
                    statusText.textContent = 'Compression terminée !';

                    resultDiv.innerHTML = `
                        <h6>✅ Compression réussie !</h6>
                        <p class="mb-1"><strong>Fichier original :</strong> ${data.original_size_gb} GB</p>
                        <p class="mb-1"><strong>Fichier compressé :</strong> ${data.compressed_size_gb} GB</p>
                        <p class="mb-1"><strong>Réduction :</strong> ${data.reduction_percent}%</p>
                        <p class="mb-1"><strong>Temps de traitement :</strong> ${data.processing_time}</p>
                        <p class="mb-0"><strong>Enregistré dans :</strong><br><small>${data.output_path}</small></p>
                    `;
                    resultDiv.classList.remove('d-none');

                    if (compressBtn) {
                        compressBtn.textContent = '✅ Compressé';
                        compressBtn.classList.remove('btn-success');
                        compressBtn.classList.add('btn-secondary');
                    }

                    resolve(data);
                } else if (data.status === 'cancelled') {
                    if (settled) return;
                    settled = true;
                    cleanup();

                    statusText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
                    progressBar.classList.remove('progress-bar-animated');
                    progressBar.classList.add('bg-warning');
                    if (compressBtn) compressBtn.disabled = false;

                    reject(new Error('Compression annulée'));
                } else if (data.status === 'error') {
                    if (settled) return;
                    settled = true;
                    cleanup();

                    statusText.textContent = 'Erreur: ' + (data.message || 'Erreur inconnue');
                    progressBar.classList.remove('progress-bar-animated');
                    progressBar.classList.add('bg-danger');
                    if (compressBtn) compressBtn.disabled = false;

                    reject(new Error(data.message || 'Erreur FFmpeg'));
                }
            });

            activeCompressions[index] = {
                unlisten: unlistenFn,
                cancel: () => {
                    if (!settled) {
                        settled = true;
                        cleanup();
                        statusText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
                        progressBar.classList.remove('progress-bar-animated');
                        progressBar.classList.add('bg-warning');
                        if (compressBtn) compressBtn.disabled = false;
                        reject(new Error('Compression annulée'));
                    }
                },
            };

            await invoke('start_compression', {
                videoIndex: index,
                inputPath: video.path,
                quality,
            });
        } catch (err) {
            if (!settled) {
                settled = true;
                cleanup();
                const errMsg = typeof err === 'string' ? err : (err?.message || String(err));
                statusText.textContent = 'Erreur: ' + errMsg;
                progressBar.classList.remove('progress-bar-animated');
                progressBar.classList.add('bg-danger');
                if (compressBtn) compressBtn.disabled = false;
                reject(new Error(errMsg));
            }
        }
    });
}

async function compressVideo(index, cardElement) {
    const video = videosData[index];
    const qualityBtn = cardElement.querySelector('.quality-btn.active');
    const quality = qualityBtn ? qualityBtn.getAttribute('data-quality') : 'medium';

    try {
        const data = await compressVideoAsync(index, cardElement, quality);

        videosData[index].is_already_compressed = true;
        videosData[index].compressed_path = data.output_path;

        // Décocher la checkbox et mettre à jour le label
        const checkbox = cardElement.querySelector('.video-checkbox');
        checkbox.checked = false;
        checkbox.nextElementSibling.textContent = '✓ Déjà compressé';
        updateSelectedCount();

        // Afficher le chemin du fichier compressé et le bouton delete
        const compressedPathItem = cardElement.querySelector('.compressed-path-item');
        const compressedPathDisplay = compressedPathItem.querySelector('.path-display');
        compressedPathDisplay.textContent = data.output_path;
        compressedPathItem.classList.remove('d-none');

        const deleteBtn = cardElement.querySelector('.delete-btn');
        deleteBtn.classList.remove('d-none');
        deleteBtn.dataset.index = index;

        // Afficher le bouton de lecture du fichier compressé
        const playCompressedBtn = cardElement.querySelector('.play-compressed-btn');
        if (playCompressedBtn) {
            playCompressedBtn.classList.remove('d-none');
        }
    } catch (error) {
        if (error.message && error.message.includes('annulée')) {
            return;
        }
        alert(`❌ Erreur lors de la compression de "${video.name}":\n\n${error.message}\n\nVérifiez les logs pour plus de détails.`);
    }
}

function formatDuration(seconds) {
    const hours = Math.floor(seconds / 3600);
    const minutes = Math.floor((seconds % 3600) / 60);
    const secs = Math.floor(seconds % 60);

    if (hours > 0) {
        return `${hours}h ${minutes}m ${secs}s`;
    } else if (minutes > 0) {
        return `${minutes}m ${secs}s`;
    } else {
        return `${secs}s`;
    }
}

function formatBitrate(bitrate) {
    const mbps = bitrate / 1000000;
    return mbps.toFixed(2) + ' Mbps';
}

async function openFolder(index) {
    const video = videosData[index];

    try {
        await invoke('open_folder', { filePath: video.path });
    } catch (error) {
        console.error('Erreur:', error);
        const errMsg = typeof error === 'string' ? error : (error?.message || String(error));
        alert('❌ Erreur lors de l\'ouverture du dossier : ' + errMsg);
    }
}

async function playVideo(filePath, type) {
    try {
        await invoke('play_video', { filePath });
        console.log(`Lecture du fichier ${type} : ${filePath}`);
    } catch (error) {
        console.error('Erreur:', error);
        const errMsg = typeof error === 'string' ? error : (error?.message || String(error));
        alert(`❌ Erreur lors de l'ouverture du fichier ${type} : ` + errMsg);
    }
}

async function confirmRecycleBinDialog(video) {
    const promptText =
        `⚠️ ATTENTION ⚠️\n\n` +
        `Vous êtes sur le point d'envoyer à la CORBEILLE le fichier original :\n\n` +
        `Nom du fichier : ${video.name}\n` +
        `Taille : ${video.size_gb} GB\n` +
        `Chemin complet :\n${video.path}\n\n` +
        `⚠️ Le fichier sera dans la corbeille Windows (récupérable).\n\n` +
        `Voulez-vous vraiment envoyer ce fichier à la corbeille ?`;

    try {
        return await ask(promptText, {
            title: '♻️ Envoyer à la corbeille Windows',
            kind: 'warning',
            okLabel: '♻️ Envoyer à la corbeille',
            cancelLabel: 'Annuler',
        });
    } catch {
        return window.confirm(promptText);
    }
}

async function deleteOriginalFile(index, cardElement) {
    const video = videosData[index];

    const confirmed = await confirmRecycleBinDialog(video);
    if (!confirmed) {
        return;
    }

    const deleteBtn = cardElement.querySelector('.delete-btn');
    const originalText = deleteBtn.textContent;
    deleteBtn.disabled = true;
    deleteBtn.textContent = '⏳ Suppression...';

    try {
        await invoke('delete_file', { filePath: video.path });

        deleteBtn.textContent = '✅ Supprimé';
        deleteBtn.classList.remove('btn-danger');
        deleteBtn.classList.add('btn-secondary');

        // Marquer la carte comme supprimée
        const card = cardElement.querySelector('.card');
        card.style.opacity = '0.6';

        // Afficher un message
        const resultDiv = cardElement.querySelector('.compression-result');
        resultDiv.innerHTML = `
            <h6 class="text-success">✅ Fichier original supprimé</h6>
            <p class="mb-0">Le fichier original de ${video.size_gb} GB a été envoyé à la corbeille avec succès.</p>
        `;
        resultDiv.classList.remove('d-none');
        resultDiv.classList.remove('compression-result');
        resultDiv.classList.add('alert', 'alert-success');

        alert(`✅ Fichier envoyé à la corbeille !\n\nVous avez libéré ${video.size_gb} GB d'espace disque.`);
    } catch (error) {
        console.error('Erreur:', error);
        deleteBtn.textContent = originalText;
        deleteBtn.disabled = false;
        const errMsg = typeof error === 'string' ? error : (error?.message || String(error));
        alert('❌ Erreur lors de la suppression : ' + errMsg);
    }
}

async function stopCompression(index) {
    if (!activeCompressions[index]) {
        alert('Aucune compression active pour cette vidéo');
        return;
    }

    try {
        await invoke('stop_compression', { videoIndex: index });
        if (activeCompressions[index]) {
            activeCompressions[index].cancel();
        }
    } catch (error) {
        console.error('Erreur lors de l\'arrêt:', error);
    }
}

async function stopBatchCompress() {
    batchCancelled = true;

    const currentVideoText = document.getElementById('currentVideoText');
    if (currentVideoText) {
        currentVideoText.textContent = '⏹️ Arrêt en cours...';
    }

    const activeIndices = Object.keys(activeCompressions).map(k => parseInt(k, 10));
    for (const index of activeIndices) {
        try {
            await invoke('stop_compression', { videoIndex: index });
        } catch (err) {
            console.error('Erreur lors de l\'arrêt:', err);
        }
        if (activeCompressions[index]) {
            activeCompressions[index].cancel();
        }
    }
}

function openBatchDeleteModal() {
    const checkboxes = document.querySelectorAll('.video-checkbox:checked');
    if (checkboxes.length === 0) {
        alert('Veuillez sélectionner au moins une vidéo.');
        return;
    }

    const selectedIndices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.index, 10));

    // Ne garder que les vidéos dont le fichier compressé existe
    const validIndices = selectedIndices.filter(i => {
        const video = videosData[i];
        if (!video || !video.path) return false;
        const cardElement = document.querySelector(`[data-index="${i}"]`)?.closest('.col');
        if (!cardElement) return false;
        const deleteBtn = cardElement.querySelector('.delete-btn');
        return deleteBtn && !deleteBtn.classList.contains('d-none') && !deleteBtn.disabled;
    });

    const filesToDelete = validIndices.map(i => videosData[i]);

    if (filesToDelete.length === 0) {
        alert('Aucun fichier original à supprimer.\nSeuls les fichiers dont la version compressée existe peuvent être supprimés.');
        return;
    }

    const skipped = selectedIndices.length - validIndices.length;
    const skipNoticeEl = document.getElementById('batchDeleteSkipNotice');
    skipNoticeEl.innerHTML = skipped > 0
        ? `<div class="alert alert-info">ℹ️ ${skipped} vidéo(s) ignorée(s) car elles n'ont pas encore de version compressée.</div>`
        : '';

    const fileList = document.getElementById('batchDeleteFileList');
    fileList.innerHTML = '';
    let totalSize = 0;

    filesToDelete.forEach(video => {
        totalSize += video.size_gb || 0;
        const li = document.createElement('li');
        li.className = 'list-group-item list-group-item-danger d-flex justify-content-between align-items-start';
        li.innerHTML = `
            <div>
                <strong>${video.name}</strong><br>
                <small class="text-muted">${video.path}</small>
            </div>
            <span class="badge bg-danger rounded-pill ms-2">${video.size_gb} GB</span>
        `;
        fileList.appendChild(li);
    });

    document.getElementById('batchDeleteSummary').textContent =
        `Total : ${filesToDelete.length} fichier(s) — ${totalSize.toFixed(2)} GB libérés`;

    const confirmBtn = document.getElementById('batchDeleteConfirmBtn');
    confirmBtn.dataset.indices = JSON.stringify(validIndices);
    confirmBtn.onclick = () => executeBatchDelete(validIndices);

    const modal = bootstrap.Modal.getOrCreateInstance(document.getElementById('batchDeleteModal'));
    modal.show();
}

async function executeBatchDelete(selectedIndices) {
    const confirmBtn = document.getElementById('batchDeleteConfirmBtn');
    confirmBtn.disabled = true;
    confirmBtn.textContent = '⏳ Suppression en cours...';

    const filePaths = selectedIndices
        .map(i => videosData[i])
        .filter(v => v && v.path)
        .map(v => v.path);

    try {
        const data = await invoke('delete_batch', { filePaths });

        const modalEl = document.getElementById('batchDeleteModal');
        const modalInstance = bootstrap.Modal.getInstance(modalEl);
        if (modalInstance) modalInstance.hide();

        // Mettre à jour l'UI pour chaque vidéo supprimée avec succès
        (data.results || []).forEach(result => {
            if (result.status !== 'success') return;
            const idx = videosData.findIndex(v => v.path === result.path);
            if (idx === -1) return;
            const cardElement = document.querySelector(`[data-index="${idx}"]`)?.closest('.col');
            if (!cardElement) return;

            const card = cardElement.querySelector('.card');
            card.style.opacity = '0.5';

            const deleteBtn = cardElement.querySelector('.delete-btn');
            if (deleteBtn) {
                deleteBtn.textContent = '✅ Supprimé';
                deleteBtn.disabled = true;
                deleteBtn.classList.remove('btn-danger');
                deleteBtn.classList.add('btn-secondary');
            }

            const resultDiv = cardElement.querySelector('.compression-result');
            if (resultDiv) {
                resultDiv.innerHTML = `
                    <div class="alert alert-success mt-2 mb-0">
                        ✅ Fichier original supprimé (${videosData[idx].size_gb} GB libérés)
                    </div>`;
                resultDiv.classList.remove('d-none');
            }
        });

        const errors = (data.results || []).filter(r => r.status !== 'success');
        if (errors.length > 0) {
            const errorMessages = errors.map(e => `• ${e.path}\n  → ${e.message}`).join('\n');
            alert(`✅ ${data.success_count} fichier(s) supprimé(s).\n\n❌ ${errors.length} erreur(s) :\n${errorMessages}`);
        } else {
            alert(`✅ ${data.success_count} fichier(s) original(aux) envoyé(s) à la corbeille avec succès !`);
        }
    } catch (error) {
        console.error('Erreur suppression en lot:', error);
        const errMsg = typeof error === 'string' ? error : (error?.message || String(error));
        alert('❌ Erreur lors de la suppression en lot : ' + errMsg);
    } finally {
        confirmBtn.disabled = false;
        confirmBtn.textContent = '♻️ Envoyer à la corbeille';
    }
}

// =============================================
// Panneau de logs Tauri Events (server://log)
// =============================================

let logPanelCollapsed = false;
let logAutoScroll = true;

async function initLogListener() {
    const statusEl = document.getElementById('logStatus');
    const logBody  = document.getElementById('logBody');

    logBody.addEventListener('scroll', () => {
        const atBottom = logBody.scrollHeight - logBody.scrollTop - logBody.clientHeight < 30;
        logAutoScroll = atBottom;
    });

    try {
        await listen('server://log', (event) => {
            const data = event.payload;
            if (data) {
                appendLog(data.level || 'INFO', data.msg || '');
            }
        });
        statusEl.textContent = 'Connected';
        statusEl.className = 'log-status log-status-connected';
        appendLog('INFO', '-- Connexion logs Tauri établie --');
    } catch (err) {
        statusEl.textContent = 'Disconnected';
        statusEl.className = 'log-status log-status-disconnected';
        console.error('Erreur listen server://log:', err);
    }
}

export function appendLog(level, msg) {
    const logBody = document.getElementById('logBody');
    if (!logBody) return;
    const now = new Date();
    const time = now.toTimeString().slice(0, 8);

    const entry = document.createElement('div');
    const effectiveLevel = msg.includes('[PROGRESS]') ? 'PROGRESS'
                         : msg.includes('reussie') || msg.includes('réussie') || msg.includes('success') ? 'SUCCESS'
                         : level;
    entry.className = `log-entry ${effectiveLevel}`;
    entry.innerHTML = `<span class="log-time">${time}</span>${escapeHtml(msg)}`;
    logBody.appendChild(entry);

    while (logBody.children.length > 500) {
        logBody.removeChild(logBody.firstChild);
    }

    if (logAutoScroll) {
        logBody.scrollTop = logBody.scrollHeight;
    }
}

function clearLogs() {
    document.getElementById('logBody').innerHTML = '';
    appendLog('INFO', '-- Logs effacés --');
}

function toggleLogPanel() {
    logPanelCollapsed = !logPanelCollapsed;
    const panel = document.getElementById('logPanel');
    const btn   = document.getElementById('logToggleBtn');
    panel.classList.toggle('collapsed', logPanelCollapsed);
    btn.textContent = logPanelCollapsed ? '>' : '<';
    btn.title = logPanelCollapsed ? 'Agrandir' : 'Réduire';
}

function escapeHtml(text) {
    return String(text)
        .replace(/&/g, '&amp;')
        .replace(/</g, '&lt;')
        .replace(/>/g, '&gt;');
}

// =============================================
// Splitter redimensionnable du panneau de logs
// =============================================

(function () {
    const MIN_WIDTH = 180;
    const MAX_WIDTH = Math.round(window.screen.width * 0.6);
    const STORAGE_KEY = 'logPanelWidth';

    function initSplitter() {
        const splitter = document.getElementById('logSplitter');
        const panel    = document.getElementById('logPanel');
        if (!splitter || !panel) return;

        // Restaurer la largeur sauvegardée
        const saved = parseInt(localStorage.getItem(STORAGE_KEY), 10);
        if (saved && saved >= MIN_WIDTH) {
            panel.style.width = saved + 'px';
        }

        let startX = 0;
        let startWidth = 0;

        splitter.addEventListener('mousedown', (e) => {
            if (panel.classList.contains('collapsed')) return;
            startX = e.clientX;
            startWidth = panel.offsetWidth;

            document.body.classList.add('log-resizing');
            splitter.classList.add('dragging');

            function onMouseMove(e) {
                const delta = startX - e.clientX;
                const newWidth = Math.min(Math.max(startWidth + delta, MIN_WIDTH), MAX_WIDTH);
                panel.style.width = newWidth + 'px';
            }

            function onMouseUp() {
                document.body.classList.remove('log-resizing');
                splitter.classList.remove('dragging');
                localStorage.setItem(STORAGE_KEY, panel.offsetWidth);
                document.removeEventListener('mousemove', onMouseMove);
                document.removeEventListener('mouseup', onMouseUp);
            }

            document.addEventListener('mousemove', onMouseMove);
            document.addEventListener('mouseup', onMouseUp);
            e.preventDefault();
        });

        // Masquer le splitter quand le panneau est réduit
        const observer = new MutationObserver(() => {
            const collapsed = panel.classList.contains('collapsed');
            splitter.classList.toggle('hidden', collapsed);
        });
        observer.observe(panel, { attributes: true, attributeFilter: ['class'] });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', initSplitter);
    } else {
        initSplitter();
    }
})();
