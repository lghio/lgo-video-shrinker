import 'bootstrap/dist/css/bootstrap.min.css';
import * as bootstrap from 'bootstrap';
import '../css/style.css';

import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, ProgressBarStatus, UserAttentionType } from '@tauri-apps/api/window';
import { ask, open as openDialog } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';
import { initUpdater, checkForAppUpdates } from './updater.js';

const STORAGE_LAST_DIR = 'lastScanDirectory';
const STORAGE_RECENT_DIRS = 'recentScanDirectories';
const STORAGE_LAST_MIN_SIZE = 'lastMinSizeGb';
const STORAGE_SELECTED_CODEC = 'selectedVideoCodec';
const STORAGE_LIFETIME_STATS = 'lifetimeCompressionStats';
const SUPPORTED_VIDEO_EXTENSIONS = ['.mp4', '.mov', '.mkv', '.avi', '.webm'];
const MAX_RECENT_DIRS = 5;

let videosData = [];
let activeCompressions = {}; // { [videoIndex]: { unlisten: Function, cancel: Function } }
let batchCancelled = false;  // Flag pour arrêter la compression par lots
let isBatchRunning = false;  // Indique si une compression par lots est en cours
let currentBatchQuality = 'medium';
let currentVideoCodec = 'h264';
let currentAppInfo = null;

document.addEventListener('DOMContentLoaded', async () => {
    document.getElementById('scanBtn')?.addEventListener('click', scanVideos);
    document.getElementById('cancelScanBtn')?.addEventListener('click', cancelScan);
    document.getElementById('selectAllBtn')?.addEventListener('click', selectAll);
    document.getElementById('deselectAllBtn')?.addEventListener('click', deselectAll);
    document.getElementById('batchCompressBtn')?.addEventListener('click', batchCompress);
    document.getElementById('batchStopBtn')?.addEventListener('click', stopBatchCompress);
    document.getElementById('batchDeleteOriginalsBtn')?.addEventListener('click', openBatchDeleteModal);
    document.getElementById('logClearBtn')?.addEventListener('click', clearLogs);
    document.getElementById('logToggleBtn')?.addEventListener('click', toggleLogPanel);
    document.getElementById('browseDirectoryBtn')?.addEventListener('click', browseDirectory);

    // Boutons Menu, À propos, Changelog & Détection GPU RTX
    document.getElementById('menuAboutBtn')?.addEventListener('click', openAboutModal);
    document.getElementById('dropdownAboutBtn')?.addEventListener('click', openAboutModal);
    document.getElementById('hwEncoderBadge')?.addEventListener('click', openAboutModal);
    document.getElementById('appVersionBadge')?.addEventListener('click', openAboutModal);

    document.getElementById('menuChangelogBtn')?.addEventListener('click', openChangelogModal);
    document.getElementById('dropdownChangelogBtn')?.addEventListener('click', openChangelogModal);
    document.getElementById('aboutOpenChangelogBtn')?.addEventListener('click', () => {
        const aboutEl = document.getElementById('aboutModal');
        const aboutInstance = aboutEl ? bootstrap.Modal.getInstance(aboutEl) : null;
        if (aboutInstance) aboutInstance.hide();
        openChangelogModal();
    });

    document.getElementById('menuProbeGpuBtn')?.addEventListener('click', reprobeHardware);
    document.getElementById('dropdownProbeGpuBtn')?.addEventListener('click', reprobeHardware);
    document.getElementById('reprobeGpuBtn')?.addEventListener('click', reprobeHardware);
    document.getElementById('resetLifetimeStatsBtn')?.addEventListener('click', resetLifetimeStats);

    document.getElementById('dropdownCheckUpdateBtn')?.addEventListener('click', () => {
        checkForAppUpdates(true, appendLog);
    });

    document.getElementById('aboutRepoLink')?.addEventListener('click', async (e) => {
        e.preventDefault();
        const url = 'https://github.com/lghio/lgo-video-shrinker';
        try {
            await openUrl(url);
        } catch {
            window.open(url, '_blank', 'noopener,noreferrer');
        }
    });

    const minSizeSelect = document.getElementById('minSize');
    if (minSizeSelect) {
        const savedMinSize = localStorage.getItem(STORAGE_LAST_MIN_SIZE);
        if (savedMinSize !== null) {
            const hasOption = Array.from(minSizeSelect.options).some(opt => opt.value === savedMinSize);
            if (hasOption) {
                minSizeSelect.value = savedMinSize;
            }
        }
        minSizeSelect.addEventListener('change', (e) => {
            localStorage.setItem(STORAGE_LAST_MIN_SIZE, e.target.value);
        });
    }

    const codecSelect = document.getElementById('codecSelect');
    if (codecSelect) {
        const savedCodec = localStorage.getItem(STORAGE_SELECTED_CODEC);
        if (savedCodec === 'h264' || savedCodec === 'hevc') {
            codecSelect.value = savedCodec;
            currentVideoCodec = savedCodec;
        }
        codecSelect.addEventListener('change', (e) => {
            const selected = e.target.value === 'hevc' ? 'hevc' : 'h264';
            currentVideoCodec = selected;
            localStorage.setItem(STORAGE_SELECTED_CODEC, selected);
        });
    }

    initBatchQualitySelector();
    initFilterToolbar();
    renderLifetimeStats();
    await initDragAndDrop();
    await initLogListener();
    await initScanProgressListener();
    await initMenuListener();
    await loadAppInfo();
    initUpdater(appendLog);
    console.log('[Init] Tous les listeners Tauri enregistrés');
});

async function setTaskbarProgress(percentOrNull, status = ProgressBarStatus.Normal) {
    try {
        const win = getCurrentWindow();
        if (percentOrNull === null || percentOrNull === undefined) {
            await win.setProgressBar({ status: ProgressBarStatus.None, progress: 0 });
        } else {
            const clamped = Math.max(0, Math.min(100, Math.round(percentOrNull)));
            await win.setProgressBar({ status, progress: clamped });
        }
    } catch {
        // Ignorer si la plateforme ou le contexte WebView ne supporte pas setProgressBar
    }
}

async function notifyBatchCompleted(summaryText) {
    try {
        await getCurrentWindow().requestUserAttention(UserAttentionType.Informational);
    } catch {
        // Ignorer si non supporté
    }

    try {
        if ('Notification' in window) {
            if (Notification.permission === 'granted') {
                new Notification('VideoShrinker — Lot terminé', { body: summaryText });
            } else if (Notification.permission !== 'denied') {
                const perm = await Notification.requestPermission();
                if (perm === 'granted') {
                    new Notification('VideoShrinker — Lot terminé', { body: summaryText });
                }
            }
        }
    } catch {
        // Ignorer si Notification API non disponible
    }
}

function getLifetimeStats() {
    const defaultStats = {
        videosCount: 0,
        bytesSaved: 0,
        originalBytes: 0,
        compressedBytes: 0,
        lastUpdated: null,
    };
    try {
        const raw = localStorage.getItem(STORAGE_LIFETIME_STATS);
        if (!raw) return defaultStats;
        const parsed = JSON.parse(raw);
        if (!parsed || typeof parsed !== 'object') return defaultStats;
        return {
            videosCount: Math.max(0, Number(parsed.videosCount) || 0),
            bytesSaved: Math.max(0, Number(parsed.bytesSaved) || 0),
            originalBytes: Math.max(0, Number(parsed.originalBytes) || 0),
            compressedBytes: Math.max(0, Number(parsed.compressedBytes) || 0),
            lastUpdated: typeof parsed.lastUpdated === 'string' ? parsed.lastUpdated : null,
        };
    } catch {
        return defaultStats;
    }
}

function recordLifetimeCompression(origSizeGb, compSizeGb) {
    const origBytes = Math.max(0, Math.round((Number(origSizeGb) || 0) * 1073741824));
    const compBytes = Math.max(0, Math.round((Number(compSizeGb) || 0) * 1073741824));
    const savedBytes = origBytes - compBytes;
    if (savedBytes <= 0) return;

    const stats = getLifetimeStats();
    stats.videosCount += 1;
    stats.bytesSaved += savedBytes;
    stats.originalBytes += origBytes;
    stats.compressedBytes += compBytes;
    stats.lastUpdated = new Date().toISOString();

    try {
        localStorage.setItem(STORAGE_LIFETIME_STATS, JSON.stringify(stats));
    } catch {
        // Ignorer en cas de quota localStorage atteint
    }
    renderLifetimeStats();
}

function renderLifetimeStats() {
    const stats = getLifetimeStats();
    const savedGb = (stats.bytesSaved / 1073741824).toFixed(2);
    const reductionPct = stats.originalBytes > 0
        ? ((stats.bytesSaved / stats.originalBytes) * 100).toFixed(1)
        : '0.0';

    const lifetimeSavedGbEl = document.getElementById('lifetimeSavedGb');
    const lifetimeVideosCountEl = document.getElementById('lifetimeVideosCount');
    if (lifetimeSavedGbEl) lifetimeSavedGbEl.textContent = `${savedGb} GB`;
    if (lifetimeVideosCountEl) lifetimeVideosCountEl.textContent = String(stats.videosCount);

    const aboutSavedGbEl = document.getElementById('aboutLifetimeSavedGb');
    const aboutVideosCountEl = document.getElementById('aboutLifetimeVideosCount');
    const aboutReductionPctEl = document.getElementById('aboutLifetimeReductionPct');
    if (aboutSavedGbEl) aboutSavedGbEl.textContent = `${savedGb} GB`;
    if (aboutVideosCountEl) aboutVideosCountEl.textContent = `${stats.videosCount} vidéo(s)`;
    if (aboutReductionPctEl) aboutReductionPctEl.textContent = `-${reductionPct}%`;
}

function resetLifetimeStats() {
    const emptyStats = {
        videosCount: 0,
        bytesSaved: 0,
        originalBytes: 0,
        compressedBytes: 0,
        lastUpdated: null,
    };
    try {
        localStorage.setItem(STORAGE_LIFETIME_STATS, JSON.stringify(emptyStats));
    } catch {
        // Ignorer
    }
    renderLifetimeStats();
    appendLog('INFO', '[Stats] Historique cumulé des compressions réinitialisé.');
}

async function initScanProgressListener() {
    try {
        await listen('scan://progress', (event) => {
            const data = event.payload;
            if (!data) return;
            const container = document.getElementById('scanProgressContainer');
            const textEl = document.getElementById('scanProgressText');
            const barEl = document.getElementById('scanProgressBar');

            if (container) {
                container.classList.remove('d-none');
            }

            const current = data.current ?? 0;
            const total = data.total ?? 0;
            const pct = total > 0 ? Math.round((current / total) * 100) : 0;

            if (textEl) {
                const fileSuffix = data.current_file ? ` (${data.current_file})` : '';
                textEl.textContent = `Analyse : ${current} / ${total} vidéos...${fileSuffix}`;
            }
            if (barEl) {
                barEl.style.width = `${pct}%`;
                barEl.textContent = `${pct}%`;
                barEl.setAttribute('aria-valuenow', String(pct));
            }
        });
    } catch (err) {
        console.warn('Listener scan://progress non disponible:', err);
    }
}

async function cancelScan() {
    const cancelBtn = document.getElementById('cancelScanBtn');
    const textEl = document.getElementById('scanProgressText');
    if (cancelBtn) {
        cancelBtn.disabled = true;
    }
    if (textEl) {
        textEl.textContent = '⏹️ Arrêt du scan en cours...';
    }
    try {
        await invoke('cancel_scan');
    } catch (err) {
        console.error('Erreur cancel_scan:', err);
    }
}

function initFilterToolbar() {
    const searchInput = document.getElementById('videoSearchInput');
    const clearBtn = document.getElementById('clearSearchBtn');
    const statusFilter = document.getElementById('videoStatusFilter');
    const sortSelect = document.getElementById('videoSortSelect');

    searchInput?.addEventListener('input', () => {
        applyFiltersAndSort();
    });

    clearBtn?.addEventListener('click', () => {
        if (searchInput) {
            searchInput.value = '';
            applyFiltersAndSort();
            searchInput.focus();
        }
    });

    statusFilter?.addEventListener('change', () => {
        applyFiltersAndSort();
    });

    sortSelect?.addEventListener('change', () => {
        applyFiltersAndSort();
    });
}

function normalizeSearchString(str) {
    return String(str || '')
        .normalize('NFD')
        .replace(/[\u0300-\u036f]/g, '')
        .toLowerCase();
}

function applyFiltersAndSort() {
    const videoList = document.getElementById('videoList');
    if (!videoList) return;

    const cardCols = Array.from(videoList.querySelectorAll('.col[data-video-index]'));
    const countBadge = document.getElementById('visibleVideosCount');

    if (cardCols.length === 0) {
        if (countBadge) {
            countBadge.textContent = 'Affichées : 0 / 0';
        }
        return;
    }

    const rawSearch = document.getElementById('videoSearchInput')?.value || '';
    const searchQuery = normalizeSearchString(rawSearch.trim());
    const statusFilter = document.getElementById('videoStatusFilter')?.value || 'all';
    const sortSelect = document.getElementById('videoSortSelect')?.value || 'size_desc';

    const items = cardCols.map(col => {
        const idx = parseInt(col.dataset.videoIndex, 10);
        return {
            col,
            idx,
            video: videosData[idx] || {},
        };
    });

    items.sort((a, b) => {
        const va = a.video;
        const vb = b.video;
        switch (sortSelect) {
            case 'size_asc':
                return (va.size_gb || 0) - (vb.size_gb || 0);
            case 'duration_desc':
                return (vb.duration || 0) - (va.duration || 0);
            case 'duration_asc':
                return (va.duration || 0) - (vb.duration || 0);
            case 'bitrate_desc':
                return (vb.bitrate || 0) - (va.bitrate || 0);
            case 'name_asc':
                return String(va.name || '').localeCompare(String(vb.name || ''), 'fr', {
                    sensitivity: 'base',
                    numeric: true,
                });
            case 'size_desc':
            default:
                return (vb.size_gb || 0) - (va.size_gb || 0);
        }
    });

    let visibleCount = 0;

    items.forEach(({ col, video }) => {
        const isCompressed = Boolean(video.is_already_compressed);
        let statusMatch = true;
        if (statusFilter === 'to_compress') {
            statusMatch = !isCompressed;
        } else if (statusFilter === 'compressed') {
            statusMatch = isCompressed;
        }

        let searchMatch = true;
        if (searchQuery) {
            const normName = normalizeSearchString(video.name);
            const normRelPath = normalizeSearchString(video.relative_path);
            searchMatch = normName.includes(searchQuery) || normRelPath.includes(searchQuery);
        }

        const isVisible = statusMatch && searchMatch;
        col.classList.toggle('d-none', !isVisible);
        if (isVisible) {
            visibleCount++;
        }

        videoList.appendChild(col);
    });

    let noMatchEl = document.getElementById('noFilterMatchAlert');
    if (visibleCount === 0 && videosData.length > 0) {
        if (!noMatchEl) {
            noMatchEl = document.createElement('div');
            noMatchEl.id = 'noFilterMatchAlert';
            noMatchEl.className = 'col-12';
            noMatchEl.innerHTML =
                '<div class="alert alert-warning mb-0">🔍 Aucune vidéo ne correspond aux critères de recherche ou de filtre.</div>';
        }
        noMatchEl.classList.remove('d-none');
        videoList.appendChild(noMatchEl);
    } else if (noMatchEl) {
        noMatchEl.classList.add('d-none');
    }

    if (countBadge) {
        countBadge.textContent = `Affichées : ${visibleCount} / ${videosData.length}`;
    }
}

function updateGlobalStats() {
    const statsCard = document.getElementById('statsCard');
    const videoCountEl = document.getElementById('videoCount');
    const totalSizeEl = document.getElementById('totalSize');
    const savingEl = document.getElementById('potentialSaving');
    const savingLabelEl = document.getElementById('potentialSavingLabel');

    const count = videosData.length;
    const totalSizeGb = videosData.reduce((sum, v) => sum + (v.size_gb || 0), 0);

    let compressedCount = 0;
    let realSavedGb = 0;
    let remainingPotentialGb = 0;

    videosData.forEach(v => {
        const origGb = v.size_gb || 0;
        if (v.is_already_compressed && v.compressed_size_gb != null) {
            compressedCount++;
            realSavedGb += Math.max(0, origGb - v.compressed_size_gb);
        } else {
            remainingPotentialGb += origGb * 0.5;
        }
    });

    if (statsCard) statsCard.classList.remove('d-none');
    if (videoCountEl) videoCountEl.textContent = String(count);
    if (totalSizeEl) totalSizeEl.textContent = `${totalSizeGb.toFixed(2)} GB`;

    if (savingEl) {
        if (compressedCount > 0) {
            savingEl.textContent = `${realSavedGb.toFixed(2)} GB`;
            if (savingLabelEl) {
                savingLabelEl.textContent = remainingPotentialGb > 0
                    ? `Économie réelle (${compressedCount}/${count}) • +~${remainingPotentialGb.toFixed(2)} GB potentiel`
                    : `Économie réelle réalisée (${compressedCount}/${count})`;
            }
        } else {
            savingEl.textContent = `~${(totalSizeGb * 0.5).toFixed(2)} GB`;
            if (savingLabelEl) {
                savingLabelEl.textContent = 'Économie potentielle (50%)';
            }
        }
    }
}

function getRecentDirectories() {
    try {
        const raw = localStorage.getItem(STORAGE_RECENT_DIRS);
        if (!raw) return [];
        const parsed = JSON.parse(raw);
        if (!Array.isArray(parsed)) return [];
        return parsed
            .filter(item => typeof item === 'string' && item.trim().length > 0)
            .map(item => item.trim());
    } catch {
        return [];
    }
}

function saveRecentDirectory(dirPath) {
    if (!dirPath || typeof dirPath !== 'string') return;
    const cleanDir = dirPath.trim();
    if (!cleanDir) return;

    localStorage.setItem(STORAGE_LAST_DIR, cleanDir);

    const existing = getRecentDirectories();
    const lowerTarget = cleanDir.toLowerCase();
    const deduplicated = [
        cleanDir,
        ...existing.filter(d => d.toLowerCase() !== lowerTarget),
    ].slice(0, MAX_RECENT_DIRS);

    localStorage.setItem(STORAGE_RECENT_DIRS, JSON.stringify(deduplicated));
    renderRecentDirectories();
}

function formatShortDirLabel(fullPath) {
    const parts = fullPath.replace(/[/\\]+$/, '').split(/[/\\]/).filter(Boolean);
    if (parts.length <= 2) return fullPath;
    return `…\\${parts.slice(-2).join('\\')}`;
}

function renderRecentDirectories() {
    const recentDirs = getRecentDirectories();
    const datalist = document.getElementById('recentDirectoriesList');
    const container = document.getElementById('recentDirsContainer');

    if (datalist) {
        datalist.innerHTML = '';
        recentDirs.forEach(dir => {
            const opt = document.createElement('option');
            opt.value = dir;
            datalist.appendChild(opt);
        });
    }

    if (!container) return;
    container.innerHTML = '';

    if (recentDirs.length === 0) {
        container.classList.add('d-none');
        return;
    }

    container.classList.remove('d-none');
    const label = document.createElement('span');
    label.className = 'recent-dirs-label';
    label.textContent = '🕒 Récents :';
    container.appendChild(label);

    recentDirs.forEach(dir => {
        const chip = document.createElement('button');
        chip.type = 'button';
        chip.className = 'recent-dir-chip';
        chip.title = `Scanner : ${dir}`;
        chip.dataset.dir = dir;
        chip.textContent = `📁 ${formatShortDirLabel(dir)}`;
        chip.addEventListener('click', () => {
            const scanInput = document.getElementById('scanDirectory');
            if (scanInput) {
                scanInput.value = dir;
            }
            saveRecentDirectory(dir);
            scanVideos();
        });
        container.appendChild(chip);
    });
}

function initBatchQualitySelector() {
    const batchBtns = document.querySelectorAll('.batch-quality-btn');
    batchBtns.forEach(btn => {
        btn.addEventListener('click', () => {
            const quality = btn.getAttribute('data-quality') || 'medium';
            batchBtns.forEach(b => b.classList.remove('active'));
            btn.classList.add('active');
            applyBatchQuality(quality);
        });
    });
}

function applyBatchQuality(quality) {
    currentBatchQuality = quality;
    const checkedBoxes = document.querySelectorAll('#videoList .col:not(.d-none) .video-checkbox:checked');
    const targetCards = checkedBoxes.length > 0
        ? Array.from(checkedBoxes).map(cb => cb.closest('.col')).filter(Boolean)
        : Array.from(document.querySelectorAll('#videoList .col:not(.d-none)'));

    targetCards.forEach(cardCol => {
        const qualityBtns = cardCol.querySelectorAll('.quality-btn');
        qualityBtns.forEach(b => {
            b.classList.toggle('active', b.getAttribute('data-quality') === quality);
        });
    });
}

async function initDragAndDrop() {
    const dropZone = document.getElementById('directoryDropZone');

    const handleDroppedPath = (rawPath) => {
        if (!rawPath || typeof rawPath !== 'string') return;
        let targetDir = rawPath.trim();
        if (!targetDir) return;

        // Si un fichier vidéo supporté (.mp4, .mov, .mkv, .avi, .webm) est déposé, utiliser son dossier parent
        const lowerTarget = targetDir.toLowerCase();
        if (SUPPORTED_VIDEO_EXTENSIONS.some(ext => lowerTarget.endsWith(ext))) {
            targetDir = targetDir.replace(/[/\\][^/\\]+$/, '');
        }
        if (!targetDir) return;

        const scanInput = document.getElementById('scanDirectory');
        if (scanInput) {
            scanInput.value = targetDir;
        }
        saveRecentDirectory(targetDir);
        appendLog('INFO', `[Drag&Drop] Dossier sélectionné : ${targetDir}`);
        scanVideos();
    };

    if (dropZone) {
        ['dragenter', 'dragover'].forEach(evtName => {
            dropZone.addEventListener(evtName, (e) => {
                e.preventDefault();
                e.stopPropagation();
                dropZone.classList.add('drag-over');
            });
        });

        ['dragleave', 'drop'].forEach(evtName => {
            dropZone.addEventListener(evtName, (e) => {
                e.preventDefault();
                e.stopPropagation();
                dropZone.classList.remove('drag-over');
            });
        });

        dropZone.addEventListener('drop', (e) => {
            const firstFile = e.dataTransfer?.files?.[0];
            if (firstFile && firstFile.path) {
                handleDroppedPath(firstFile.path);
            }
        });
    }

    try {
        await listen('tauri://drag-enter', () => {
            dropZone?.classList.add('drag-over');
        });
        await listen('tauri://drag-over', () => {
            dropZone?.classList.add('drag-over');
        });
        await listen('tauri://drag-leave', () => {
            dropZone?.classList.remove('drag-over');
        });
        await listen('tauri://drag-drop', (event) => {
            dropZone?.classList.remove('drag-over');
            const paths = event.payload?.paths || (Array.isArray(event.payload) ? event.payload : []);
            if (paths.length > 0) {
                handleDroppedPath(paths[0]);
            }
        });
    } catch (err) {
        console.warn('Listeners tauri://drag-* non disponibles:', err);
    }
}

async function initMenuListener() {
    try {
        await listen('menu://action', (event) => {
            const action = event.payload;
            if (action === 'about') {
                openAboutModal();
            } else if (action === 'changelog') {
                openChangelogModal();
            } else if (action === 'probe_gpu') {
                reprobeHardware();
            }
        });
    } catch (err) {
        console.warn('Listener menu://action non disponible:', err);
    }
}

function openAboutModal() {
    const modalEl = document.getElementById('aboutModal');
    if (!modalEl) return;
    renderLifetimeStats();
    const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
    modal.show();
}

function openChangelogModal() {
    const modalEl = document.getElementById('changelogModal');
    if (!modalEl) return;
    const modal = bootstrap.Modal.getOrCreateInstance(modalEl);
    modal.show();
}

function formatInlineMarkdown(rawText) {
    let html = escapeHtml(rawText);
    html = html.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
    html = html.replace(/`([^`]+)`/g, '<code>$1</code>');
    return html;
}

function renderChangelog(markdown) {
    const container = document.getElementById('changelogContainer');
    if (!container) return;

    if (!markdown || typeof markdown !== 'string') {
        container.innerHTML = '<div class="alert alert-secondary mb-0">Aucune entrée de changelog disponible.</div>';
        return;
    }

    const lines = markdown.split(/\r?\n/);
    const entries = [];

    for (const line of lines) {
        const trimmed = line.trim();
        if (!trimmed.startsWith('|') || !trimmed.endsWith('|')) continue;
        const cells = trimmed
            .slice(1, -1)
            .split('|')
            .map(c => c.trim());
        if (cells.length < 2) continue;
        const [dateCol, summaryCol] = cells;
        if (
            dateCol.toLowerCase() === 'date' ||
            /^[-:\s]+$/.test(dateCol) ||
            /^[-:\s]+$/.test(summaryCol)
        ) {
            continue;
        }
        entries.push({ date: dateCol, summary: summaryCol });
    }

    if (entries.length === 0) {
        container.innerHTML = `<pre class="small mb-0">${escapeHtml(markdown)}</pre>`;
        return;
    }

    container.innerHTML = entries
        .map((entry, idx) => {
            const versionMatch = entry.summary.match(/\bv\d+\.\d+\.\d+\b/);
            const versionBadge = versionMatch
                ? `<span class="badge bg-primary">${escapeHtml(versionMatch[0])}</span>`
                : idx === 0
                ? `<span class="badge bg-success">Dernière mise à jour</span>`
                : '';

            const typeMatch = entry.summary.match(/^([a-z]+)(\([^)]+\))?:\s*/i);
            let categoryBadge = '';
            let bodyText = entry.summary;
            if (typeMatch) {
                const commitType = typeMatch[1].toLowerCase();
                const scope = typeMatch[2] ? typeMatch[2] : '';
                const badgeColor =
                    commitType === 'feat'
                        ? 'bg-success'
                        : commitType === 'fix'
                        ? 'bg-warning text-dark'
                        : commitType === 'chore'
                        ? 'bg-info text-dark'
                        : 'bg-secondary';
                categoryBadge = `<span class="badge ${badgeColor} font-monospace">${escapeHtml(commitType + scope)}</span>`;
                bodyText = entry.summary.slice(typeMatch[0].length);
            }

            return `
                <div class="changelog-entry-card">
                    <div class="d-flex justify-content-between align-items-center flex-wrap gap-2 mb-1">
                        <div class="d-flex align-items-center gap-2">
                            <span class="badge bg-dark">📅 ${escapeHtml(entry.date)}</span>
                            ${versionBadge}
                            ${categoryBadge}
                        </div>
                    </div>
                    <div class="small text-dark">${formatInlineMarkdown(bodyText)}</div>
                </div>
            `;
        })
        .join('');
}

function applyAppInfoToUi(info) {
    if (!info) return;
    currentAppInfo = info;

    const versionBadge = document.getElementById('appVersionBadge');
    if (versionBadge && info.version) {
        versionBadge.textContent = `v${info.version}`;
    }
    const aboutVersionBadge = document.getElementById('aboutVersionBadge');
    if (aboutVersionBadge && info.version) {
        aboutVersionBadge.textContent = `v${info.version}`;
    }

    const hwBadge = document.getElementById('hwEncoderBadge');
    const hevcEncoder = info.hw_encoder_hevc || 'libx265';
    if (hwBadge && info.hw_encoder) {
        if (info.is_rtx) {
            const gpuLabel = info.gpu_name || 'NVIDIA RTX';
            hwBadge.textContent = `🟢 RTX NVENC : ${gpuLabel} (${info.hw_encoder} / ${hevcEncoder})`;
            hwBadge.classList.add('header-badge-rtx');
        } else {
            hwBadge.classList.remove('header-badge-rtx');
            if (info.gpu_name && info.hw_encoder !== 'libx264') {
                hwBadge.textContent = `⚡ Encodeur : ${info.hw_encoder} / ${hevcEncoder} (${info.gpu_name})`;
            } else {
                hwBadge.textContent = `⚡ Encodeur : ${info.hw_encoder} / ${hevcEncoder}`;
            }
        }
    }

    const aboutGpuName = document.getElementById('aboutGpuName');
    if (aboutGpuName) {
        aboutGpuName.textContent = info.gpu_name || 'Aucun GPU dédié détecté (CPU uniquement)';
    }

    const aboutRtxStatus = document.getElementById('aboutRtxStatus');
    if (aboutRtxStatus) {
        if (info.is_rtx) {
            aboutRtxStatus.innerHTML =
                '<span class="badge bg-success">🟢 NVIDIA RTX — Moteur NVENC avancé actif (H.264 &amp; HEVC)</span>';
        } else if (info.hw_encoder && info.hw_encoder !== 'libx264') {
            aboutRtxStatus.innerHTML = `<span class="badge bg-info text-dark">⚡ Accélération matérielle (${escapeHtml(info.hw_encoder)} / ${escapeHtml(hevcEncoder)})</span>`;
        } else {
            aboutRtxStatus.innerHTML =
                '<span class="badge bg-secondary">💻 Encodage logiciel CPU (libx264 / libx265)</span>';
        }
    }

    const aboutEncoderName = document.getElementById('aboutEncoderName');
    if (aboutEncoderName) {
        aboutEncoderName.textContent = `${info.hw_encoder || 'libx264'} (H.264) / ${hevcEncoder} (HEVC)`;
    }

    const aboutEncoderPipeline = document.getElementById('aboutEncoderPipeline');
    if (aboutEncoderPipeline) {
        if (info.hw_encoder === 'h264_nvenc') {
            aboutEncoderPipeline.textContent =
                'Décodage matériel (-hwaccel auto) + NVENC VBR HQ (-tune hq -multipass qres) + Spatial/Temporal AQ (H.264 & HEVC hvc1)';
        } else if (info.hw_encoder === 'h264_qsv') {
            aboutEncoderPipeline.textContent =
                'Accélération matérielle Intel QuickSync (-c:v h264_qsv / hevc_qsv -global_quality)';
        } else if (info.hw_encoder === 'h264_amf') {
            aboutEncoderPipeline.textContent =
                'Accélération matérielle AMD AMF (-c:v h264_amf / hevc_amf -qp_i/-qp_p)';
        } else {
            aboutEncoderPipeline.textContent =
                'Encodage logiciel CPU multi-thread (-c:v libx264 / libx265 -threads 0)';
        }
    }

    if (info.changelog_markdown) {
        renderChangelog(info.changelog_markdown);
    }
}

async function reprobeHardware() {
    const reprobeBtn = document.getElementById('reprobeGpuBtn');
    const menuProbeBtn = document.getElementById('menuProbeGpuBtn');

    if (reprobeBtn) {
        reprobeBtn.disabled = true;
        reprobeBtn.textContent = '⏳ Détection GPU en cours...';
    }
    if (menuProbeBtn) {
        menuProbeBtn.disabled = true;
    }

    try {
        appendLog('INFO', '[HW] Analyse de la carte graphique NVIDIA RTX et des encodeurs FFmpeg...');
        const info = await invoke('probe_hardware');
        applyAppInfoToUi(info);
        if (info.is_rtx) {
            appendLog(
                'SUCCESS',
                `[HW] Carte NVIDIA RTX confirmée : ${info.gpu_name} | Encodeur ${info.hw_encoder} (NVENC VBR HQ + Spatial/Temporal AQ actif)`
            );
        } else {
            appendLog(
                'INFO',
                `[HW] Détection terminée : GPU=${info.gpu_name || 'N/A'} | Encodeur=${info.hw_encoder}`
            );
        }
    } catch (error) {
        console.error('Erreur probe_hardware:', error);
        appendLog('ERROR', `[HW] Échec de la détection matérielle : ${error}`);
    } finally {
        if (reprobeBtn) {
            reprobeBtn.disabled = false;
            reprobeBtn.textContent = '🔍 Relancer la détection GPU / RTX';
        }
        if (menuProbeBtn) {
            menuProbeBtn.disabled = false;
        }
    }
}

async function loadAppInfo() {
    try {
        const info = await invoke('get_app_info');
        const scanInput = document.getElementById('scanDirectory');
        const savedLastDir = localStorage.getItem(STORAGE_LAST_DIR);
        if (scanInput) {
            if (savedLastDir && savedLastDir.trim()) {
                scanInput.value = savedLastDir.trim();
            } else if (!scanInput.value && info.default_directory) {
                scanInput.value = info.default_directory;
            }
        }
        renderRecentDirectories();
        applyAppInfoToUi(info);

        if (info.is_rtx) {
            appendLog(
                'SUCCESS',
                `[Init] VideoShrinker v${info.version} prêt | 🟢 GPU RTX détecté : ${info.gpu_name} (${info.hw_encoder} VBR HQ + AQ)`
            );
        } else {
            appendLog(
                'INFO',
                `[Init] VideoShrinker v${info.version} prêt | Encodeur matériel : ${info.hw_encoder}${info.gpu_name ? ' (' + info.gpu_name + ')' : ''}`
            );
        }
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
            saveRecentDirectory(data.path);
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
                saveRecentDirectory(selected);
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
    const scanProgressContainer = document.getElementById('scanProgressContainer');
    const scanProgressText = document.getElementById('scanProgressText');
    const scanProgressBar = document.getElementById('scanProgressBar');
    const cancelScanBtn = document.getElementById('cancelScanBtn');

    if (!directory) {
        alert('Veuillez entrer un répertoire à scanner');
        return;
    }

    localStorage.setItem(STORAGE_LAST_MIN_SIZE, minSize);

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = ' Scan en cours...';

    if (scanProgressContainer) {
        scanProgressContainer.classList.remove('d-none');
    }
    if (scanProgressText) {
        scanProgressText.textContent = 'Analyse : 0 / 0 vidéos...';
    }
    if (scanProgressBar) {
        scanProgressBar.style.width = '0%';
        scanProgressBar.textContent = '0%';
        scanProgressBar.setAttribute('aria-valuenow', '0');
    }
    if (cancelScanBtn) {
        cancelScanBtn.disabled = false;
    }

    try {
        const data = await invoke('scan_videos', {
            directory,
            minSizeGb: parseFloat(minSize),
        });

        videosData = data.videos || [];

        // Masquer l'ancien résumé de lot lors d'un nouveau scan
        document.getElementById('batchSummaryBanner')?.classList.add('d-none');

        // Afficher les statistiques globales
        updateGlobalStats();

        // Afficher et mémoriser le répertoire scanné
        const scannedDir = data.scanned_directory || directory;
        if (scannedDir) {
            document.getElementById('scanDirectory').value = scannedDir;
            saveRecentDirectory(scannedDir);
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
        if (scanProgressContainer) {
            scanProgressContainer.classList.add('d-none');
        }
        setTimeout(() => {
            text.textContent = '📁 Scanner à nouveau';
        }, 2000);
    }
}

function displayVideos(videos) {
    const videoList = document.getElementById('videoList');
    const filterToolbar = document.getElementById('filterToolbar');
    videoList.innerHTML = '';

    if (!videos || videos.length === 0) {
        videoList.innerHTML = '<div class="col"><div class="alert alert-info">Aucune vidéo trouvée.</div></div>';
        document.getElementById('batchActions').classList.add('d-none');
        filterToolbar?.classList.add('d-none');
        return;
    }

    filterToolbar?.classList.remove('d-none');

    videos.forEach((video, index) => {
        const card = createVideoCard(video, index);
        videoList.appendChild(card);
    });

    applyFiltersAndSort();
}

function createVideoCard(video, index) {
    const col = document.createElement('div');
    col.className = 'col';
    col.dataset.videoIndex = String(index);

    const isCompressed = Boolean(video.is_already_compressed);
    const isCloudOnly = Boolean(video.is_cloud_only);
    const isLowBitrate = Boolean(video.is_low_bitrate);

    const duration = video.duration
        ? formatDuration(video.duration)
        : (isCloudOnly ? '☁️ En ligne' : 'N/A');
    const resolution = video.resolution || (isCloudOnly ? '☁️ En ligne' : 'N/A');
    const bitrate = video.bitrate
        ? formatBitrate(video.bitrate)
        : (isCloudOnly ? '☁️ En ligne' : 'N/A');

    // Décocher par défaut si déjà compressé, Cloud-only OneDrive ou déjà à bas bitrate
    const checkboxChecked = !isCompressed && !isCloudOnly && !isLowBitrate;
    const checkboxLabel = isCompressed
        ? '✓ Déjà compressé'
        : isCloudOnly
        ? '☁️ En ligne uniquement'
        : isLowBitrate
        ? '⚠️ Déjà optimisé'
        : 'À compresser';

    const statusBadgesHtml = [
        isCloudOnly
            ? '<span class="badge badge-cloud-only" title="Ce fichier est stocké sur OneDrive (En ligne uniquement). Il ne sera téléchargé que si vous lancez sa compression.">☁️ OneDrive (En ligne uniquement)</span>'
            : '',
        isLowBitrate
            ? '<span class="badge badge-low-bitrate" title="Cette vidéo a déjà un débit binaire faible (< 1.5 Mbps). Une re-compression risque d\'apporter peu de gain.">⚠️ Déjà optimisé (bitrate bas)</span>'
            : '',
    ]
        .filter(Boolean)
        .join('');

    const compressedLabel = (isCompressed && video.compressed_size_gb != null)
        ? `📦 Fichier compressé (${video.compressed_size_gb} GB • -${video.reduction_percent ?? 0}%) :`
        : '📦 Fichier compressé :';

    col.innerHTML = `
        <div class="card video-card">
            <div class="video-header">
                <div class="d-flex justify-content-between align-items-center">
                    <div class="flex-grow-1">
                        <h5 class="mb-1">🎥 ${video.name}</h5>
                        <small>${video.relative_path}</small>
                        ${statusBadgesHtml ? `<div class="video-status-badges">${statusBadgesHtml}</div>` : ''}
                    </div>
                    <div class="form-check">
                        <input class="form-check-input video-checkbox" type="checkbox" 
                               id="checkbox-${index}" data-index="${index}" 
                               ${checkboxChecked ? 'checked' : ''}>
                        <label class="form-check-label text-white" for="checkbox-${index}">
                            ${checkboxLabel}
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
                            <strong class="compressed-path-label">${compressedLabel}</strong>
                            <button class="btn btn-sm btn-success play-compressed-btn ${isCompressed ? '' : 'd-none'}" data-index="${index}" title="Lire dans VLC">
                                ▶️ Lire compressé
                            </button>
                        </div>
                        <div class="path-display">${video.compressed_path || 'Pas encore compressé'}</div>
                    </div>
                </div>
                
                <div class="quality-selector">
                    <button class="quality-btn ${currentBatchQuality === 'low' ? 'active' : ''}" data-quality="low">
                        <div>🏃 Rapide</div>
                        <small>Qualité basse</small>
                    </button>
                    <button class="quality-btn ${currentBatchQuality === 'medium' ? 'active' : ''}" data-quality="medium">
                        <div>⚖️ Équilibré</div>
                        <small>Qualité moyenne</small>
                    </button>
                    <button class="quality-btn ${currentBatchQuality === 'high' ? 'active' : ''}" data-quality="high">
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

                <div class="form-check mt-2 rename-compressed-wrap ${isCompressed ? '' : 'd-none'}">
                    <input class="form-check-input rename-compressed-check" type="checkbox"
                           id="rename-check-${index}" data-index="${index}" checked>
                    <label class="form-check-label small text-muted" for="rename-check-${index}">
                        ✏️ Garder le nom original (renommer <code>_compressed.mp4</code> après mise à la corbeille)
                    </label>
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
    document.querySelectorAll('#videoList .col:not(.d-none) .video-checkbox').forEach(cb => {
        cb.checked = true;
    });
    updateSelectedCount();
}

function deselectAll() {
    document.querySelectorAll('#videoList .col:not(.d-none) .video-checkbox').forEach(cb => {
        cb.checked = false;
    });
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
    const batchEtaText = document.getElementById('batchEtaText');
    const summaryBanner = document.getElementById('batchSummaryBanner');

    btn.disabled = true;
    spinner.classList.remove('d-none');
    text.textContent = ' Compression en cours...';
    progressDiv.classList.remove('d-none');
    if (summaryBanner) {
        summaryBanner.classList.add('d-none');
    }
    if (batchEtaText) {
        batchEtaText.textContent = '';
        batchEtaText.classList.add('d-none');
    }

    const selectedIndices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.index, 10));
    const total = selectedIndices.length;
    let completed = 0;
    let successCount = 0;
    let totalBeforeGb = 0;
    let totalAfterGb = 0;
    let lastKnownSpeedX = null;
    const batchStartTime = Date.now();

    batchCancelled = false;
    isBatchRunning = true;
    await setTaskbarProgress(0);

    for (let pos = 0; pos < selectedIndices.length; pos++) {
        const index = selectedIndices[pos];
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

        const queuedDurationS = selectedIndices
            .slice(pos + 1)
            .reduce((acc, idx) => acc + (videosData[idx]?.duration || 0), 0);

        const handleBatchProgress = (data) => {
            if (data.speed != null && data.speed > 0) {
                lastKnownSpeedX = data.speed;
            }
            const currentEtaS = data.eta_seconds ?? null;
            let totalBatchEtaS = currentEtaS;
            if (lastKnownSpeedX && lastKnownSpeedX > 0.05 && queuedDurationS > 0) {
                const queuedEtaS = Math.round(queuedDurationS / lastKnownSpeedX);
                totalBatchEtaS = (currentEtaS ?? 0) + queuedEtaS;
            }
            if (totalBatchEtaS != null) {
                const etaFormatted = formatDuration(totalBatchEtaS);
                if (batchEtaText) {
                    batchEtaText.textContent = `⏳ Reste lot : ~${etaFormatted}`;
                    batchEtaText.classList.remove('d-none');
                }
                progressText.textContent = `${completed} / ${total} • ⏳ ~${etaFormatted}`;
            }
            const videoRatio = Math.min(1, Math.max(0, (data.progress ?? 0) / 100));
            const overallPct = ((completed + videoRatio) / total) * 100;
            progressBar.style.width = overallPct.toFixed(1) + '%';
            progressBar.textContent = Math.round(overallPct) + '%';
            setTaskbarProgress(overallPct);
        };

        try {
            const result = await compressVideoAsync(index, cardElement, quality, handleBatchProgress);
            completed++;

            const checkbox = cardElement.querySelector('.video-checkbox');
            checkbox.checked = false;

            if (result && result.status === 'skipped_no_gain') {
                checkbox.nextElementSibling.textContent = '🛡️ Original conservé';
            } else if (result && result.output_path) {
                checkbox.nextElementSibling.textContent = '✓ Déjà compressé';
                successCount++;
                const origGb = result.original_size_gb ?? video.size_gb ?? 0;
                const compGb = result.compressed_size_gb ?? 0;
                totalBeforeGb += origGb;
                totalAfterGb += compGb;

                videosData[index].is_already_compressed = true;
                videosData[index].compressed_path = result.output_path;
                videosData[index].compressed_size_gb = result.compressed_size_gb;
                videosData[index].reduction_percent = result.reduction_percent;

                const compressedPathItem = cardElement.querySelector('.compressed-path-item');
                const compressedPathDisplay = compressedPathItem.querySelector('.path-display');
                compressedPathDisplay.textContent = result.output_path;
                compressedPathItem.classList.remove('d-none');

                const compressedLabelEl = compressedPathItem.querySelector('.compressed-path-label');
                if (compressedLabelEl && result.compressed_size_gb != null) {
                    compressedLabelEl.textContent = `📦 Fichier compressé (${result.compressed_size_gb} GB • -${result.reduction_percent ?? 0}%) :`;
                }

                const deleteBtn = cardElement.querySelector('.delete-btn');
                deleteBtn.classList.remove('d-none');
                deleteBtn.dataset.index = index;

                const renameWrap = cardElement.querySelector('.rename-compressed-wrap');
                if (renameWrap) {
                    renameWrap.classList.remove('d-none');
                }

                const playCompressedBtn = cardElement.querySelector('.play-compressed-btn');
                if (playCompressedBtn) {
                    playCompressedBtn.classList.remove('d-none');
                }

                updateGlobalStats();
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
        setTaskbarProgress(overallProgress);
    }

    isBatchRunning = false;
    await setTaskbarProgress(null);

    if (batchEtaText) {
        batchEtaText.textContent = '';
        batchEtaText.classList.add('d-none');
    }

    const batchElapsedS = Math.max(1, Math.round((Date.now() - batchStartTime) / 1000));
    const savedGb = Math.max(0, totalBeforeGb - totalAfterGb);
    const savedPercent = totalBeforeGb > 0
        ? ((savedGb / totalBeforeGb) * 100).toFixed(1)
        : '0.0';
    const durationStr = formatDuration(batchElapsedS);

    if (!batchCancelled) {
        progressBar.classList.remove('progress-bar-animated');
        currentVideoText.textContent = '✅ Toutes les compressions sont terminées!';
        text.textContent = '✅ Terminé';
    } else {
        text.textContent = '⏹️ Arrêté';
    }

    const prefix = batchCancelled ? '⏹️ Lot interrompu' : '✅ Lot terminé';
    const summaryMsg =
        `${prefix} : ${successCount}/${total} vidéo(s) compressée(s) • ` +
        `Avant : ${totalBeforeGb.toFixed(2)} GB → Après : ${totalAfterGb.toFixed(2)} GB • ` +
        `Économie réelle : ${savedGb.toFixed(2)} GB (-${savedPercent}%) en ${durationStr}`;

    if (summaryBanner) {
        summaryBanner.textContent = summaryMsg;
        summaryBanner.classList.remove('d-none', 'alert-success', 'alert-warning');
        summaryBanner.classList.add(batchCancelled ? 'alert-warning' : 'alert-success');
        appendLog(batchCancelled ? 'WARNING' : 'SUCCESS', `[Batch] ${summaryMsg}`);
    }

    if (!batchCancelled) {
        notifyBatchCompleted(summaryMsg);
    }

    updateGlobalStats();
    applyFiltersAndSort();

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

function compressVideoAsync(index, cardElement, quality, onProgress = null) {
    return new Promise(async (resolve, reject) => {
        const video = videosData[index];
        const compressBtn = cardElement.querySelector('.compress-btn');
        const statusDiv = cardElement.querySelector('.compression-status');
        const resultDiv = cardElement.querySelector('.compression-result');
        const progressBar = statusDiv.querySelector('.progress-bar');
        const statusText = statusDiv.querySelector('.status-text');

        if (video.is_cloud_only) {
            appendLog('INFO', `[INFO] Téléchargement OneDrive requis avant compression : ${video.name} (${video.path})`);
        }

        if (compressBtn) compressBtn.disabled = true;
        statusDiv.classList.remove('d-none');
        statusDiv.style.display = 'block';
        resultDiv.classList.add('d-none');
        resultDiv.classList.remove('smart-skip-result');
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
                    statusText.textContent = video.is_cloud_only
                        ? '☁️ Rapatriement OneDrive & démarrage de la compression...'
                        : 'Compression démarrée...';
                    progressBar.style.width = '0%';
                    progressBar.textContent = '0%';
                } else if (data.status === 'progress') {
                    const progress = data.progress ?? 0;
                    progressBar.style.width = progress + '%';
                    progressBar.textContent = progress + '%';
                    progressBar.setAttribute('aria-valuenow', progress);
                    const cur = (data.current_time ?? 0).toFixed(1);
                    const tot = (data.total_duration ?? 0).toFixed(1);
                    const speedPart = (data.speed != null && data.speed > 0)
                        ? ` • ⚡ ${Number(data.speed).toFixed(1)}x`
                        : '';
                    const etaPart = (data.eta_seconds != null)
                        ? ` • ⏳ Reste : ~${formatDuration(data.eta_seconds)}`
                        : '';
                    statusText.textContent = `Compression en cours... (${cur}s / ${tot}s${speedPart}${etaPart})`;
                    if (!isBatchRunning) {
                        setTaskbarProgress(progress);
                    }
                    if (typeof onProgress === 'function') {
                        onProgress(data);
                    }
                } else if (data.status === 'completed') {
                    if (settled) return;
                    settled = true;
                    cleanup();

                    progressBar.style.width = '100%';
                    progressBar.textContent = '100%';
                    progressBar.classList.remove('progress-bar-animated');
                    statusText.textContent = 'Compression terminée !';

                    recordLifetimeCompression(
                        data.original_size_gb ?? video?.size_gb ?? 0,
                        data.compressed_size_gb ?? 0
                    );
                    if (!isBatchRunning) {
                        setTaskbarProgress(null);
                    }

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
                } else if (data.status === 'skipped_no_gain') {
                    if (settled) return;
                    settled = true;
                    cleanup();
                    if (!isBatchRunning) {
                        setTaskbarProgress(null);
                    }

                    progressBar.style.width = '100%';
                    progressBar.textContent = '100%';
                    progressBar.classList.remove('progress-bar-animated');
                    progressBar.classList.add('bg-warning');
                    statusText.textContent = '🛡️ Ignorée (Smart Skip) — original conservé';

                    resultDiv.innerHTML = `
                        <h6>🛡️ Ignorée (Smart Skip)</h6>
                        <p class="mb-0">🛡️ Ignorée (Smart Skip) : le fichier généré (${data.compressed_size_gb} GB) n'était pas plus léger que l'original (${data.original_size_gb} GB) — original conservé</p>
                    `;
                    resultDiv.classList.add('smart-skip-result');
                    resultDiv.classList.remove('d-none');

                    if (compressBtn) {
                        compressBtn.disabled = false;
                        compressBtn.textContent = '🛡️ Original conservé (Smart Skip)';
                    }

                    resolve(data);
                } else if (data.status === 'cancelled') {
                    if (settled) return;
                    settled = true;
                    cleanup();
                    if (!isBatchRunning) {
                        setTaskbarProgress(null);
                    }

                    statusText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
                    progressBar.classList.remove('progress-bar-animated');
                    progressBar.classList.add('bg-warning');
                    if (compressBtn) compressBtn.disabled = false;

                    reject(new Error('Compression annulée'));
                } else if (data.status === 'error') {
                    if (settled) return;
                    settled = true;
                    cleanup();
                    if (!isBatchRunning) {
                        setTaskbarProgress(null);
                    }

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
                        if (!isBatchRunning) {
                            setTaskbarProgress(null);
                        }
                        statusText.textContent = '⏹️ Compression arrêtée par l\'utilisateur';
                        progressBar.classList.remove('progress-bar-animated');
                        progressBar.classList.add('bg-warning');
                        if (compressBtn) compressBtn.disabled = false;
                        reject(new Error('Compression annulée'));
                    }
                },
            };

            const selectedCodec = document.getElementById('codecSelect')?.value || currentVideoCodec || 'h264';

            await invoke('start_compression', {
                videoIndex: index,
                inputPath: video.path,
                quality,
                codec: selectedCodec,
            });
        } catch (err) {
            if (!settled) {
                settled = true;
                cleanup();
                if (!isBatchRunning) {
                    setTaskbarProgress(null);
                }
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

    if (!isBatchRunning) {
        await setTaskbarProgress(0);
    }

    try {
        const data = await compressVideoAsync(index, cardElement, quality);

        const checkbox = cardElement.querySelector('.video-checkbox');
        if (data && data.status === 'skipped_no_gain') {
            if (checkbox) {
                checkbox.checked = false;
                checkbox.nextElementSibling.textContent = '🛡️ Original conservé';
            }
            updateSelectedCount();
            return;
        }

        videosData[index].is_already_compressed = true;
        videosData[index].compressed_path = data.output_path;
        videosData[index].compressed_size_gb = data.compressed_size_gb;
        videosData[index].reduction_percent = data.reduction_percent;

        // Décocher la checkbox et mettre à jour le label
        checkbox.checked = false;
        checkbox.nextElementSibling.textContent = '✓ Déjà compressé';
        updateSelectedCount();

        // Afficher le chemin du fichier compressé et le bouton delete
        const compressedPathItem = cardElement.querySelector('.compressed-path-item');
        const compressedPathDisplay = compressedPathItem.querySelector('.path-display');
        compressedPathDisplay.textContent = data.output_path;
        compressedPathItem.classList.remove('d-none');

        const compressedLabelEl = compressedPathItem.querySelector('.compressed-path-label');
        if (compressedLabelEl && data.compressed_size_gb != null) {
            compressedLabelEl.textContent = `📦 Fichier compressé (${data.compressed_size_gb} GB • -${data.reduction_percent ?? 0}%) :`;
        }

        const deleteBtn = cardElement.querySelector('.delete-btn');
        deleteBtn.classList.remove('d-none');
        deleteBtn.dataset.index = index;

        const renameWrap = cardElement.querySelector('.rename-compressed-wrap');
        if (renameWrap) {
            renameWrap.classList.remove('d-none');
        }

        // Afficher le bouton de lecture du fichier compressé
        const playCompressedBtn = cardElement.querySelector('.play-compressed-btn');
        if (playCompressedBtn) {
            playCompressedBtn.classList.remove('d-none');
        }

        updateGlobalStats();
        applyFiltersAndSort();
    } catch (error) {
        if (error.message && error.message.includes('annulée')) {
            return;
        }
        alert(`❌ Erreur lors de la compression de "${video.name}":\n\n${error.message}\n\nVérifiez les logs pour plus de détails.`);
    } finally {
        if (!isBatchRunning) {
            await setTaskbarProgress(null);
        }
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

async function confirmRecycleBinDialog(video, renameCompressed = false) {
    const renameNote = renameCompressed
        ? `✏️ Le fichier "_compressed.mp4" sera automatiquement renommé en "${video.name}".\n`
        : '';
    const promptText =
        `⚠️ ATTENTION ⚠️\n\n` +
        `Vous êtes sur le point d'envoyer à la CORBEILLE le fichier original :\n\n` +
        `Nom du fichier : ${video.name}\n` +
        `Taille : ${video.size_gb} GB\n` +
        `Chemin complet :\n${video.path}\n\n` +
        `⚠️ Le fichier sera dans la corbeille Windows (récupérable).\n` +
        renameNote + `\n` +
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
    const renameCheck = cardElement.querySelector('.rename-compressed-check');
    const renameCompressed = renameCheck ? renameCheck.checked : false;

    const confirmed = await confirmRecycleBinDialog(video, renameCompressed);
    if (!confirmed) {
        return;
    }

    const deleteBtn = cardElement.querySelector('.delete-btn');
    const originalText = deleteBtn.textContent;
    deleteBtn.disabled = true;
    deleteBtn.textContent = '⏳ Suppression...';

    try {
        const res = await invoke('delete_file', {
            filePath: video.path,
            renameCompressed,
        });

        deleteBtn.textContent = '✅ Supprimé';
        deleteBtn.classList.remove('btn-danger');
        deleteBtn.classList.add('btn-secondary');

        const renameWrap = cardElement.querySelector('.rename-compressed-wrap');
        if (renameWrap) {
            renameWrap.classList.add('d-none');
        }

        const renamedPath = res?.renamed_to || null;
        if (renamedPath) {
            videosData[index].compressed_path = renamedPath;
            const compPathDisplay = cardElement.querySelector('.compressed-path-item .path-display');
            if (compPathDisplay) {
                compPathDisplay.textContent = renamedPath;
            }
        }

        // Marquer la carte comme supprimée
        const card = cardElement.querySelector('.card');
        card.style.opacity = '0.6';

        // Afficher un message
        const resultDiv = cardElement.querySelector('.compression-result') || cardElement.querySelector('.alert-success');
        if (resultDiv) {
            resultDiv.innerHTML = renamedPath
                ? `<h6 class="text-success">✅ Fichier original supprimé & compressé renommé</h6>
                   <p class="mb-0">Le fichier original (${video.size_gb} GB) a été envoyé à la corbeille et le fichier compressé a été renommé en <strong>${escapeHtml(video.name)}</strong>.</p>`
                : `<h6 class="text-success">✅ Fichier original supprimé</h6>
                   <p class="mb-0">Le fichier original de ${video.size_gb} GB a été envoyé à la corbeille avec succès.</p>`;
            resultDiv.classList.remove('d-none');
            resultDiv.classList.remove('compression-result');
            resultDiv.classList.add('alert', 'alert-success');
        }

        const alertSuffix = renamedPath
            ? `\n✏️ Le fichier compressé a été renommé en « ${video.name} ».`
            : '';
        alert(`✅ Fichier envoyé à la corbeille !\n\nVous avez libéré ${video.size_gb} GB d'espace disque.${alertSuffix}`);
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
    const renameCheck = document.getElementById('batchRenameCompressedCheck');
    const renameCompressed = renameCheck ? renameCheck.checked : false;

    confirmBtn.disabled = true;
    confirmBtn.textContent = '⏳ Suppression en cours...';

    const filePaths = selectedIndices
        .map(i => videosData[i])
        .filter(v => v && v.path)
        .map(v => v.path);

    try {
        const data = await invoke('delete_batch', {
            filePaths,
            renameCompressed,
        });

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

            const renameWrap = cardElement.querySelector('.rename-compressed-wrap');
            if (renameWrap) {
                renameWrap.classList.add('d-none');
            }

            if (result.renamed_to) {
                videosData[idx].compressed_path = result.renamed_to;
                const compPathDisplay = cardElement.querySelector('.compressed-path-item .path-display');
                if (compPathDisplay) {
                    compPathDisplay.textContent = result.renamed_to;
                }
            }

            const resultDiv = cardElement.querySelector('.compression-result');
            if (resultDiv) {
                const renameBadge = result.renamed_to
                    ? ` • Compressé renommé en <strong>${escapeHtml(videosData[idx].name)}</strong>`
                    : '';
                resultDiv.innerHTML = `
                    <div class="alert alert-success mt-2 mb-0">
                        ✅ Fichier original supprimé (${videosData[idx].size_gb} GB libérés)${renameBadge}
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
