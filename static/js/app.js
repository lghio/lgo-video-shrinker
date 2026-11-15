let videosData = [];
let activeWebSockets = {}; // Stocker les WebSockets actifs pour pouvoir les annuler
let batchCancelled = false; // Flag pour arrêter la compression par lots

document.addEventListener('DOMContentLoaded', () => {
    document.getElementById('scanBtn')?.addEventListener('click', scanVideos);
    document.getElementById('selectAllBtn')?.addEventListener('click', selectAll);
    document.getElementById('deselectAllBtn')?.addEventListener('click', deselectAll);
    document.getElementById('batchCompressBtn')?.addEventListener('click', batchCompress);
    document.getElementById('batchStopBtn')?.addEventListener('click', stopBatchCompress);
    document.getElementById('batchDeleteOriginalsBtn')?.addEventListener('click', openBatchDeleteModal);
    document.getElementById('logClearBtn')?.addEventListener('click', clearLogs);
    document.getElementById('logToggleBtn')?.addEventListener('click', toggleLogPanel);
    document.getElementById('browseDirectoryBtn')?.addEventListener('click', browseDirectory);
    console.log('[Init] Tous les listeners enregistrés');
    initLogWebSocket();
});

async function browseDirectory() {
    const btn     = document.getElementById('browseDirectoryBtn');
    const spinner = document.getElementById('browseSpinner');
    const text    = document.getElementById('browseText');

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = 'En attente…';

    try {
        const response = await fetch('/api/browse-directory');
        const data = await response.json();

        if (data.status === 'ok' && data.path) {
            document.getElementById('scanDirectory').value = data.path;
        }
        // Si annulé (status === 'cancelled'), on ne fait rien
    } catch (error) {
        console.error('Erreur browse-directory:', error);
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
        const response = await fetch(`/api/scan?directory=${encodeURIComponent(directory)}&min_size_gb=${minSize}`);
        const data = await response.json();

        if (!response.ok) {
            throw new Error(data.detail || `Erreur ${response.status}`);
        }

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
        document.getElementById('batchActions').classList.remove('d-none');
        updateSelectedCount();
        
        text.textContent = '✅ Scan terminé';
    } catch (error) {
        console.error('Erreur:', error);
        text.textContent = '❌ Erreur lors du scan';
        alert('Erreur: ' + error.message);
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
    
    const selectedIndices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.index));
    const total = selectedIndices.length;
    let completed = 0;
    
    batchCancelled = false; // Réinitialiser le flag
    
    for (const index of selectedIndices) {
        // Vérifier si l'utilisateur a annulé
        if (batchCancelled) {
            currentVideoText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
            progressBar.classList.remove('progress-bar-animated');
            progressBar.classList.add('bg-warning');
            break;
        }
        
        const video = videosData[index];
        currentVideoText.textContent = `Compression : ${video.name}`;
        progressText.textContent = `${completed} / ${total}`;
        
        // Trouver la carte correspondante
        const cardElement = document.querySelector(`[data-index="${index}"]`).closest('.col');
        const qualityBtn = cardElement.querySelector('.quality-btn.active');
        const quality = qualityBtn.getAttribute('data-quality');
        
        try {
            const result = await compressVideoAsync(index, cardElement, quality);
            completed++;
            
            // Décocher la checkbox après compression réussie
            const checkbox = cardElement.querySelector('.video-checkbox');
            checkbox.checked = false;
            checkbox.nextElementSibling.textContent = '✓ Déjà compressé';
            
            // Afficher le chemin du fichier compressé et le bouton delete
            if (result && result.output_path) {
                const compressedPathItem = cardElement.querySelector('.compressed-path-item');
                const compressedPathDisplay = compressedPathItem.querySelector('.path-display');
                compressedPathDisplay.textContent = result.output_path;
                compressedPathItem.classList.remove('d-none');
                
                const deleteBtn = cardElement.querySelector('.delete-btn');
                deleteBtn.classList.remove('d-none');
                deleteBtn.dataset.index = index;
                
                // Afficher le bouton de lecture du fichier compressé
                const playCompressedBtn = cardElement.querySelector('.play-compressed-btn');
                if (playCompressedBtn) {
                    playCompressedBtn.classList.remove('d-none');
                }
            }
            
        } catch (error) {
            console.error(`Erreur lors de la compression de ${video.name}:`, error);
            // Si c'est une annulation, on arrête la boucle
            if (error.message && error.message.includes('annulée')) {
                break;
            }
            // Continuer avec la vidéo suivante même en cas d'erreur
            completed++;
        }
        
        // Mettre à jour la barre de progression globale
        const overallProgress = (completed / total) * 100;
        progressBar.style.width = overallProgress + '%';
        progressBar.textContent = Math.round(overallProgress) + '%';
        progressText.textContent = `${completed} / ${total}`;
    }
    
    // Terminé
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
    return new Promise((resolve, reject) => {
        const video = videosData[index];
        const statusDiv = cardElement.querySelector('.compression-status');
        const resultDiv = cardElement.querySelector('.compression-result');
        const progressBar = statusDiv.querySelector('.progress-bar');
        const statusText = statusDiv.querySelector('.status-text');
        
        statusDiv.classList.remove('d-none');
        resultDiv.classList.add('d-none');
        
        // WebSocket connection
        const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
        const ws = new WebSocket(`${protocol}//${window.location.host}/ws/compress/${index}`);
        
        console.log(`[DEBUG] WebSocket créé (batch) pour la vidéo ${index}: ${video.name}`);
        
        // Stocker le WebSocket dans le dictionnaire des WebSockets actifs
        activeWebSockets[index] = ws;
        
        ws.onopen = () => {
            console.log(`[DEBUG] WebSocket connecté (batch) pour la vidéo ${index}`);
            ws.send(JSON.stringify({
                input_path: video.path,
                quality: quality
            }));
        };
        
        ws.onmessage = (event) => {
            const data = JSON.parse(event.data);
            console.log(`[DEBUG] Message WebSocket (batch) reçu:`, data);
            
            if (data.status === 'started') {
                statusText.textContent = 'Compression démarrée...';
                progressBar.style.width = '0%';
                progressBar.textContent = '0%';
            } else if (data.status === 'progress') {
                const progress = data.progress;
                console.log(`[DEBUG] Mise à jour progression (batch): ${progress}%`);
                progressBar.style.width = progress + '%';
                progressBar.textContent = progress + '%';
                statusText.textContent = `${data.current_time.toFixed(0)}s / ${data.total_duration.toFixed(0)}s`;
                console.log(`[DEBUG] Barre mise à jour (batch): width=${progressBar.style.width}`);
            } else if (data.status === 'completed') {
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
                
                // Retirer du dictionnaire
                delete activeWebSockets[index];
                
                resolve(data);
            } else if (data.status === 'error') {
                console.error(`[ERROR] Erreur de compression (batch) pour ${video.name}:`, data.message);
                statusText.textContent = 'Erreur: ' + data.message;
                progressBar.classList.remove('progress-bar-animated');
                progressBar.classList.add('bg-danger');
                
                // Retirer du dictionnaire
                delete activeWebSockets[index];
                
                reject(new Error(data.message));
            }
        };
        
        ws.onerror = (error) => {
            console.error(`[ERROR] Erreur WebSocket (batch) pour la vidéo ${index}:`, error);
            statusText.textContent = 'Erreur de connexion';
            
            // Retirer du dictionnaire
            delete activeWebSockets[index];
            
            reject(error);
        };
        
        ws.onclose = () => {
            console.log(`[DEBUG] WebSocket fermé (batch) pour la vidéo ${index}`);
            // Si le WebSocket se ferme pendant la compression, c'est probablement une annulation
            if (activeWebSockets[index]) {
                delete activeWebSockets[index];
                statusText.textContent = 'Compression annulée';
                progressBar.classList.remove('progress-bar-animated');
                progressBar.classList.add('bg-warning');
                reject(new Error('Compression annulée'));
            }
        };
    });
}

async function compressVideo(index, cardElement) {
    const video = videosData[index];
    
    console.log('[DEBUG] compressVideo appelé avec:', {
        index: index,
        cardElement: cardElement,
        cardElementType: cardElement?.tagName,
        cardElementClasses: cardElement?.className
    });
    
    const qualityBtn = cardElement.querySelector('.quality-btn.active');
    const quality = qualityBtn ? qualityBtn.getAttribute('data-quality') : 'high';
    
    const compressBtn = cardElement.querySelector('.compress-btn');
    const statusDiv = cardElement.querySelector('.compression-status');
    const resultDiv = cardElement.querySelector('.compression-result');
    
    console.log('[DEBUG] Recherche des éléments:', {
        compressBtn: compressBtn,
        statusDiv: statusDiv,
        statusDivFound: !!statusDiv,
        statusDivClasses: statusDiv?.className,
        allStatusDivs: cardElement.querySelectorAll('.compression-status').length
    });
    
    if (!statusDiv) {
        console.error('[ERROR] compression-status div non trouvée!');
        alert('Erreur: Impossible de trouver la zone de progression. Rechargez la page.');
        return;
    }
    
    const progressBar = statusDiv.querySelector('.progress-bar');
    const statusText = statusDiv.querySelector('.status-text');
    
    console.log('[DEBUG] Éléments enfants trouvés:', {
        progressBar: progressBar,
        statusText: statusText,
        progressBarClasses: progressBar?.className
    });
    
    compressBtn.disabled = true;
    
    console.log('[DEBUG] Avant classList.remove - statusDiv.classList:', Array.from(statusDiv.classList));
    statusDiv.classList.remove('d-none');
    console.log('[DEBUG] Après classList.remove - statusDiv.classList:', Array.from(statusDiv.classList));
    
    // Forcer l'affichage au cas où
    statusDiv.style.display = 'block';
    
    console.log('[DEBUG] statusDiv display style:', window.getComputedStyle(statusDiv).display);
    console.log('[DEBUG] statusDiv offsetHeight (0 = invisible):', statusDiv.offsetHeight);
    
    resultDiv.classList.add('d-none');
    
    // WebSocket connection
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const ws = new WebSocket(`${protocol}//${window.location.host}/ws/compress/${index}`);
    
    console.log(`[DEBUG] WebSocket créé pour la vidéo ${index}: ${video.name}`);
    console.log(`[DEBUG] Chemin: ${video.path}`);
    
    // Stocker le WebSocket dans le dictionnaire des WebSockets actifs
    activeWebSockets[index] = ws;
    
    ws.onopen = () => {
        console.log(`[DEBUG] WebSocket connecté pour la vidéo ${index}`);
        const payload = {
            input_path: video.path,
            quality: quality
        };
        console.log(`[DEBUG] Envoi des données:`, payload);
        ws.send(JSON.stringify(payload));
    };
    
    ws.onmessage = (event) => {
        const data = JSON.parse(event.data);
        console.log(`[DEBUG] Message WebSocket reçu:`, data);
        
        if (data.status === 'started') {
            console.log(`[DEBUG] Compression démarrée pour ${video.name}`);
            statusText.textContent = 'Compression démarrée...';
            progressBar.style.width = '0%';
            progressBar.textContent = '0%';
            console.log('[DEBUG] Après started - progressBar:', {
                width: progressBar.style.width,
                text: progressBar.textContent,
                element: progressBar
            });
        } else if (data.status === 'progress') {
            const progress = data.progress;
            console.log(`[DEBUG] Mise à jour progression: ${progress}%`);
            
            // Mettre à jour la barre de progression
            progressBar.style.width = progress + '%';
            progressBar.textContent = progress + '%';
            progressBar.setAttribute('aria-valuenow', progress);
            
            // Mettre à jour le texte de statut
            statusText.textContent = `${data.current_time.toFixed(0)}s / ${data.total_duration.toFixed(0)}s`;
            
            console.log(`[DEBUG] Barre mise à jour:`, {
                width: progressBar.style.width,
                text: progressBar.textContent,
                statusText: statusText.textContent,
                progressBarElement: progressBar,
                computedWidth: window.getComputedStyle(progressBar).width
            });
        } else if (data.status === 'completed') {
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
            
            compressBtn.textContent = '✅ Compressé';
            compressBtn.classList.remove('btn-success');
            compressBtn.classList.add('btn-secondary');
            
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
            
            // Retirer du dictionnaire
            delete activeWebSockets[index];
        } else if (data.status === 'error') {
            console.error(`[ERROR] Erreur de compression pour ${video.name}:`, data.message);
            statusText.textContent = 'Erreur: ' + data.message;
            progressBar.classList.remove('progress-bar-animated');
            progressBar.classList.add('bg-danger');
            compressBtn.disabled = false;
            
            // Afficher une alerte avec plus de détails
            alert(`❌ Erreur lors de la compression de "${video.name}":\n\n${data.message}\n\nVérifiez la console (F12) pour plus de détails.`);
            
            // Retirer du dictionnaire
            delete activeWebSockets[index];
        }
    };
    
    ws.onerror = (error) => {
        console.error(`[ERROR] Erreur WebSocket pour la vidéo ${index}:`, error);
        statusText.textContent = 'Erreur de connexion';
        compressBtn.disabled = false;
        
        // Retirer du dictionnaire
        delete activeWebSockets[index];
    };
    
    ws.onclose = (event) => {
        console.log(`[DEBUG] WebSocket fermé pour la vidéo ${index}, code: ${event.code}, raison: ${event.reason}`);
        // Si le WebSocket se ferme pendant la compression, c'est probablement une annulation
        if (activeWebSockets[index]) {
            delete activeWebSockets[index];
            statusText.textContent = 'Compression annulée';
            progressBar.classList.remove('progress-bar-animated');
            progressBar.classList.add('bg-warning');
            compressBtn.disabled = false;
        }
    };
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
        const response = await fetch('/api/open-folder', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                file_path: video.path
            })
        });
        
        const data = await response.json();
        
        if (!response.ok) {
            throw new Error(data.detail || 'Erreur lors de l\'ouverture du dossier');
        }
    } catch (error) {
        console.error('Erreur:', error);
        alert('❌ Erreur lors de l\'ouverture du dossier : ' + error.message);
    }
}

async function playVideo(filePath, type) {
    try {
        const response = await fetch('/api/play-video', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                file_path: filePath
            })
        });
        
        const data = await response.json();
        
        if (!response.ok) {
            throw new Error(data.detail || 'Erreur lors de l\'ouverture du fichier');
        }
        
        // Message de confirmation (optionnel)
        console.log(`Lecture du fichier ${type} dans VLC : ${filePath}`);
    } catch (error) {
        console.error('Erreur:', error);
        alert(`❌ Erreur lors de l\'ouverture du fichier ${type} : ` + error.message);
    }
}

async function deleteOriginalFile(index, cardElement) {
    const video = videosData[index];
    
    // Confirmation de suppression avec tous les détails
    const confirmed = confirm(
        `⚠️ ATTENTION ⚠️\n\n` +
        `Vous êtes sur le point d'envoyer à la CORBEILLE le fichier original :\n\n` +
        `Nom du fichier : ${video.name}\n` +
        `Taille : ${video.size_gb} GB\n` +
        `Chemin complet :\n${video.path}\n\n` +
        `⚠️ Le fichier sera dans la corbeille (récupérable).\n\n` +
        `Voulez-vous vraiment envoyer ce fichier à la corbeille ?`
    );
    
    if (!confirmed) {
        return;
    }
    
    const deleteBtn = cardElement.querySelector('.delete-btn');
    const originalText = deleteBtn.textContent;
    deleteBtn.disabled = true;
    deleteBtn.textContent = '⏳ Suppression...';
    
    try {
        const response = await fetch('/api/delete', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                file_path: video.path
            })
        });
        
        const data = await response.json();
        
        if (response.ok) {
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
                <p class="mb-0">Le fichier original de ${video.size_gb} GB a été supprimé avec succès.</p>
            `;
            resultDiv.classList.remove('d-none');
            resultDiv.classList.remove('compression-result');
            resultDiv.classList.add('alert', 'alert-success');
            
            alert(`✅ Fichier envoyé à la corbeille !\n\nVous avez libéré ${video.size_gb} GB d'espace disque.`);
        } else {
            throw new Error(data.detail || 'Erreur lors de la suppression');
        }
    } catch (error) {
        console.error('Erreur:', error);
        deleteBtn.textContent = originalText;
        deleteBtn.disabled = false;
        alert('❌ Erreur lors de la suppression : ' + error.message);
    }
}

async function stopCompression(index) {
    // Vérifier s'il y a un WebSocket actif pour cet index
    if (!activeWebSockets[index]) {
        alert('Aucune compression active pour cette vidéo');
        return;
    }
    
    try {
        // Appeler l'API en premier pour tuer le process FFmpeg côté serveur
        const response = await fetch('/api/stop-compression', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                video_index: index
            })
        });
        
        const data = await response.json();

        // Fermer le WebSocket après avoir arrêté le process
        const ws = activeWebSockets[index];
        if (ws) {
            // Supprimer d'abord pour éviter le reject dans onclose
            delete activeWebSockets[index];
            ws.close();
        }
        
        if (response.ok) {
            console.log('Compression arrêtée avec succès');
            
            // Mettre à jour l'UI
            const cardElement = document.querySelector(`[data-index="${index}"]`).closest('.col');
            const statusText = cardElement.querySelector('.status-text');
            const progressBar = cardElement.querySelector('.progress-bar');
            const compressBtn = cardElement.querySelector('.compress-btn');
            
            statusText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
            progressBar.classList.remove('progress-bar-animated');
            progressBar.classList.add('bg-warning');
            compressBtn.disabled = false;
        } else {
            console.error('Erreur lors de l\'arrêt:', data.message);
        }
    } catch (error) {
        console.error('Erreur:', error);
    }
}

function stopBatchCompress() {
    // Définir le flag d'annulation
    batchCancelled = true;
    
    // Fermer tous les WebSockets actifs
    for (const index in activeWebSockets) {
        const ws = activeWebSockets[index];
        ws.close();
        
        // Appeler l'API pour arrêter le processus côté serveur
        fetch('/api/stop-compression', {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({
                video_index: parseInt(index)
            })
        }).catch(error => {
            console.error('Erreur lors de l\'arrêt:', error);
        });
    }
    
    // Vider le dictionnaire
    activeWebSockets = {};
    
    // Afficher un message
    const currentVideoText = document.getElementById('currentVideoText');
    currentVideoText.textContent = '⏹️ Arrêt en cours...';
}

function openBatchDeleteModal() {
    // Récupérer toutes les vidéos sélectionnées
    const checkboxes = document.querySelectorAll('.video-checkbox:checked');
    if (checkboxes.length === 0) {
        alert('Veuillez sélectionner au moins une vidéo.');
        return;
    }

    const selectedIndices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.index));

    // Ne garder que les vidéos dont le fichier compressé existe
    // (bouton "Supprimer l'original" visible = compressé au scan ou pendant la session)
    const validIndices = selectedIndices.filter(i => {
        const video = videosData[i];
        if (!video || !video.path) return false;
        const cardElement = document.querySelector(`[data-index="${i}"]`)?.closest('.col');
        if (!cardElement) return false;
        const deleteBtn = cardElement.querySelector('.delete-btn');
        return deleteBtn && !deleteBtn.classList.contains('d-none');
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

    // Remplir la liste de la modale
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

    // Enregistrer les données sur le bouton de confirmation
    const confirmBtn = document.getElementById('batchDeleteConfirmBtn');
    confirmBtn.dataset.indices = JSON.stringify(validIndices);
    confirmBtn.onclick = () => executeBatchDelete(validIndices);

    // Ouvrir la modale
    const modal = new bootstrap.Modal(document.getElementById('batchDeleteModal'));
    modal.show();
}

async function executeBatchDelete(selectedIndices) {    const confirmBtn = document.getElementById('batchDeleteConfirmBtn');
    confirmBtn.disabled = true;
    confirmBtn.textContent = '⏳ Suppression en cours...';

    const filePaths = selectedIndices
        .map(i => videosData[i])
        .filter(v => v && v.path)
        .map(v => v.path);

    try {
        const response = await fetch('/api/delete-batch', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ file_paths: filePaths })
        });

        const data = await response.json();

        if (!response.ok) {
            throw new Error(data.detail || 'Erreur lors de la suppression');
        }

        // Fermer la modale
        bootstrap.Modal.getInstance(document.getElementById('batchDeleteModal')).hide();

        // Mettre à jour l'UI pour chaque vidéo supprimée avec succès
        data.results.forEach(result => {
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

        const errors = data.results.filter(r => r.status !== 'success');
        if (errors.length > 0) {
            const errorMessages = errors.map(e => `• ${e.path}\n  → ${e.message}`).join('\n');
            alert(`✅ ${data.success_count} fichier(s) supprimé(s).\n\n❌ ${errors.length} erreur(s) :\n${errorMessages}`);
        } else {
            alert(`✅ ${data.success_count} fichier(s) original(aux) envoyé(s) à la corbeille avec succès !`);
        }

    } catch (error) {
        console.error('Erreur suppression en lot:', error);
        alert('❌ Erreur lors de la suppression en lot : ' + error.message);
    } finally {
        confirmBtn.disabled = false;
        confirmBtn.textContent = '🗑️ Supprimer définitivement';
    }
}
// =============================================
// Panneau de logs WebSocket
// =============================================

let logWs = null;
let logPanelCollapsed = false;
let logAutoScroll = true;

function initLogWebSocket() {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:';
    const statusEl = document.getElementById('logStatus');
    const logBody  = document.getElementById('logBody');

    function connect() {
        logWs = new WebSocket(`${protocol}//${window.location.host}/ws/logs`);

        logWs.onopen = () => {
            statusEl.textContent = 'Connected';
            statusEl.className = 'log-status log-status-connected';
            appendLog('INFO', '-- Connexion logs etablie --');
        };

        logWs.onmessage = (event) => {
            const data = JSON.parse(event.data);
            appendLog(data.level, data.msg);
        };

        logWs.onclose = () => {
            statusEl.textContent = 'Disconnected';
            statusEl.className = 'log-status log-status-disconnected';
            appendLog('ERROR', '-- Connexion perdue, reconnexion dans 3s... --');
            setTimeout(connect, 3000);
        };

        logWs.onerror = () => {
            statusEl.textContent = 'Error';
            statusEl.className = 'log-status log-status-disconnected';
        };
    }

    logBody.addEventListener('scroll', () => {
        const atBottom = logBody.scrollHeight - logBody.scrollTop - logBody.clientHeight < 30;
        logAutoScroll = atBottom;
    });

    connect();
}

function appendLog(level, msg) {
    const logBody = document.getElementById('logBody');
    const now = new Date();
    const time = now.toTimeString().slice(0, 8);

    const entry = document.createElement('div');
    const effectiveLevel = msg.includes('[PROGRESS]') ? 'PROGRESS'
                         : msg.includes('reussie') || msg.includes('success') ? 'SUCCESS'
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
    appendLog('INFO', '-- Logs effaces --');
}

function toggleLogPanel() {
    logPanelCollapsed = !logPanelCollapsed;
    const panel = document.getElementById('logPanel');
    const btn   = document.getElementById('logToggleBtn');
    panel.classList.toggle('collapsed', logPanelCollapsed);
    btn.textContent = logPanelCollapsed ? '>' : '<';
    btn.title = logPanelCollapsed ? 'Agrandir' : 'Reduire';
}

function escapeHtml(text) {
    return text
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

        // Restaurer la largeur sauvegardee
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
                // Le splitter est a gauche du panel : glisser a gauche = agrandir
                const delta = startX - e.clientX;
                let newWidth = Math.min(Math.max(startWidth + delta, MIN_WIDTH), MAX_WIDTH);
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

        // Masquer le splitter quand le panneau est collapse
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
