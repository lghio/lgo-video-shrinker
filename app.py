from fastapi import FastAPI, WebSocket, HTTPException
from fastapi.responses import HTMLResponse, JSONResponse
from fastapi.staticfiles import StaticFiles
from fastapi.templating import Jinja2Templates
from fastapi import Request
from contextlib import asynccontextmanager
import os
import tkinter as tk
from tkinter import filedialog
import subprocess
import json
from pathlib import Path
from typing import List, Dict
import asyncio
import re
import threading
import time
import sys
import logging
from send2trash import send2trash
import imageio_ffmpeg

# --- Résolution des chemins pour PyInstaller (mode .exe) ---
if getattr(sys, 'frozen', False):
    # Fichiers bundlés (templates, static) → extraits dans _MEIPASS par PyInstaller
    BUNDLE_DIR = Path(sys._MEIPASS)
    # Dossier du .exe → pour ffmpeg/ffprobe qui doivent être à côté de l'exe
    BASE_DIR = Path(sys.executable).parent
else:
    BUNDLE_DIR = Path(__file__).parent
    BASE_DIR   = Path(__file__).parent

# Ajouter le dossier de l'exe au PATH pour trouver ffmpeg/ffprobe
os.environ["PATH"] = str(BASE_DIR) + os.pathsep + os.environ.get("PATH", "")

# Binaires FFmpeg embarqués via imageio-ffmpeg (inclus automatiquement par PyInstaller)
FFMPEG_CMD  = imageio_ffmpeg.get_ffmpeg_exe()
# ffprobe est dans le même dossier que ffmpeg fourni par imageio-ffmpeg
FFPROBE_CMD = str(Path(FFMPEG_CMD).parent / "ffprobe.exe") if (Path(FFMPEG_CMD).parent / "ffprobe.exe").exists() else "ffprobe"

# ──────────────────────────────────────────────
# Détection de l'encodeur hardware disponible
# ──────────────────────────────────────────────
def detect_hw_encoder() -> str:
    """Détecte le meilleur encodeur disponible : NVENC > QuickSync > AMF > CPU."""
    no_window = subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0
    candidates = [
        ('h264_nvenc',   [FFMPEG_CMD, '-f', 'lavfi', '-i', 'nullsrc=s=64x64:d=0.1', '-c:v', 'h264_nvenc', '-f', 'null', '-']),
        ('h264_qsv',     [FFMPEG_CMD, '-f', 'lavfi', '-i', 'nullsrc=s=64x64:d=0.1', '-c:v', 'h264_qsv',   '-f', 'null', '-']),
        ('h264_amf',     [FFMPEG_CMD, '-f', 'lavfi', '-i', 'nullsrc=s=64x64:d=0.1', '-c:v', 'h264_amf',   '-f', 'null', '-']),
    ]
    for name, cmd in candidates:
        try:
            r = subprocess.run(cmd, capture_output=True, timeout=8,
                               creationflags=no_window)
            if r.returncode == 0:
                print(f"[HW] Encodeur hardware détecté : {name}")
                return name
        except Exception:
            pass
    print("[HW] Aucun encodeur hardware, utilisation CPU (libx264)")
    return 'libx264'

HW_ENCODER: str = detect_hw_encoder()

# ---- Système de logs WebSocket ----
log_ws_clients: List[WebSocket] = []

class WebSocketLogHandler(logging.Handler):
    """Handler qui diffuse les logs vers tous les clients WebSocket connectés."""
    def emit(self, record: logging.LogRecord):
        msg = self.format(record)
        level = record.levelname
        asyncio.run_coroutine_threadsafe(_broadcast_log(msg, level), _main_loop)

async def _broadcast_log(msg: str, level: str):
    dead = []
    for ws in log_ws_clients:
        try:
            await ws.send_json({"level": level, "msg": msg})
        except Exception:
            dead.append(ws)
    for ws in dead:
        log_ws_clients.remove(ws)

# Capture des print() via un stream personnalisé
class _PrintInterceptor:
    def __init__(self, level: str):
        self.level = level
        self._buf = ""
    def write(self, text: str):
        self._buf += text
        if "\n" in self._buf:
            lines = self._buf.split("\n")
            for line in lines[:-1]:
                if line.strip():
                    asyncio.run_coroutine_threadsafe(
                        _broadcast_log(line, self.level), _main_loop
                    )
            self._buf = lines[-1]
    def flush(self):
        pass

_main_loop: asyncio.AbstractEventLoop = None  # initialisé au démarrage

@asynccontextmanager
async def lifespan(app: FastAPI):
    global _main_loop
    _main_loop = asyncio.get_event_loop()
    # Intercepter les print() → logs WebSocket
    sys.stdout = _PrintInterceptor("INFO")
    sys.stderr = _PrintInterceptor("ERROR")
    yield  # l'application tourne ici

# ---- Fin système de logs ----

app = FastAPI(title="Video Shrinker", lifespan=lifespan)

# Monter les fichiers statiques
app.mount("/static", StaticFiles(directory=str(BUNDLE_DIR / "static")), name="static")
templates = Jinja2Templates(directory=str(BUNDLE_DIR / "templates"))

# Répertoire par défaut à scanner
DEFAULT_SCAN_DIRECTORY = r"C:\Users\ferie\OneDrive - English with Feriel\__Project\Neuro Language Coaching"

# Dictionnaire pour stocker les processus de compression actifs
active_processes: Dict[int, subprocess.Popen] = {}


@app.websocket("/ws/logs")
async def ws_logs(websocket: WebSocket):
    """WebSocket pour streamer les logs vers le navigateur."""
    await websocket.accept()
    log_ws_clients.append(websocket)
    try:
        while True:
            await websocket.receive_text()  # garder la connexion ouverte
    except Exception:
        pass
    finally:
        if websocket in log_ws_clients:
            log_ws_clients.remove(websocket)
# ---- Fin système de logs ----

def strip_long_prefix(p: str) -> str:
    """Supprime le préfixe \\\\?\\ des chemins longs Windows."""
    if p and p.startswith('\\\\?\\'):
        return p[4:]
    return p

def normalize_path(p: str) -> str:
    """Normalise un chemin : supprime les guillemets parasites, unifie les séparateurs Windows,
    et active les chemins longs (> 260 cars) via le préfixe \\\\?\\."""
    if not p:
        return p
    # Supprimer les guillemets parasites wrappant tout le chemin (pas dans le milieu)
    p = p.strip()
    if len(p) >= 2 and p[0] == '"' and p[-1] == '"':
        p = p[1:-1].strip()
    p = os.path.normpath(p)
    # Activer les chemins longs Windows (> 260 caractères)
    if sys.platform == 'win32' and len(p) > 240 and not p.startswith('\\\\?\\'):
        p = '\\\\?\\' + p
    return p

def get_file_size_gb(file_path: str) -> float:
    """Retourne la taille du fichier en GB"""
    size_bytes = os.path.getsize(file_path)
    return size_bytes / (1024 ** 3)

def get_video_info(file_path: str) -> Dict:
    """Récupère les informations de la vidéo via ffprobe"""
    try:
        cmd = [
            FFPROBE_CMD, '-v', 'quiet', '-print_format', 'json',
            '-show_format', '-show_streams', file_path
        ]
        # CREATE_NO_WINDOW : empêche l'apparition de fenêtre DOS sous Windows
        no_window = subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0
        result = subprocess.run(cmd, capture_output=True, text=True, creationflags=no_window)
        return json.loads(result.stdout)
    except Exception as e:
        return {}

@app.get("/", response_class=HTMLResponse)
async def read_root(request: Request):
    """Page principale"""
    return templates.TemplateResponse("index.html", {"request": request, "default_directory": DEFAULT_SCAN_DIRECTORY})

@app.get("/api/scan")
async def scan_videos(directory: str = None, min_size_gb: float = 0.0):
    """Scanne le répertoire pour trouver les MP4 (par défaut tous, ou > min_size_gb)"""
    videos = []
    
    # Utiliser le répertoire fourni ou le répertoire par défaut
    raw_dir = directory if directory else DEFAULT_SCAN_DIRECTORY
    # Normaliser : supprimer les guillemets parasites, normaliser les séparateurs
    scan_directory = normalize_path(raw_dir)
    
    if not os.path.exists(scan_directory):
        raise HTTPException(status_code=400, detail=f"Le répertoire '{scan_directory}' n'existe pas ou est inaccessible.")

    # Activer les chemins longs pour os.walk (préfixe \\?\ sur Windows)
    walk_root = scan_directory
    if sys.platform == 'win32' and not walk_root.startswith('\\\\?\\'):
        walk_root = '\\\\?\\' + walk_root

    try:
        # Parcourir récursivement le répertoire
        for root, dirs, files in os.walk(walk_root):
            for file in files:
                if file.lower().endswith('.mp4'):
                    file_path = normalize_path(os.path.join(root, file))
                    size_gb = get_file_size_gb(file_path)
                    
                    if size_gb >= min_size_gb:
                        # Vérifier si le fichier compressé existe déjà
                        path_obj = Path(file_path)
                        compressed_path = str(path_obj.parent / f"{path_obj.stem}_compressed{path_obj.suffix}")
                        is_already_compressed = os.path.exists(compressed_path)
                        
                        # Obtenir des infos supplémentaires
                        info = get_video_info(file_path)
                        duration = None
                        resolution = None
                        bitrate = None
                        
                        if info and 'format' in info:
                            duration = float(info['format'].get('duration', 0))
                            bitrate = int(info['format'].get('bit_rate', 0))
                        
                        if info and 'streams' in info:
                            for stream in info['streams']:
                                if stream.get('codec_type') == 'video':
                                    width = stream.get('width')
                                    height = stream.get('height')
                                    if width and height:
                                        resolution = f"{width}x{height}"
                                    break
                        
                        videos.append({
                            'path': file_path,
                            'name': file,
                            'size_gb': round(size_gb, 2),
                            'size_bytes': os.path.getsize(file_path),
                            'duration': duration,
                            'resolution': resolution,
                            'bitrate': bitrate,
                            'relative_path': os.path.relpath(
                                strip_long_prefix(file_path),
                                strip_long_prefix(scan_directory)
                            ),
                            'is_already_compressed': is_already_compressed,
                            'compressed_path': compressed_path if is_already_compressed else None
                        })
        
        # Trier par taille décroissante
        videos.sort(key=lambda x: x['size_gb'], reverse=True)
        
        return {
            'count': len(videos),
            'total_size_gb': round(sum(v['size_gb'] for v in videos), 2),
            'videos': videos,
            'scanned_directory': scan_directory
        }
    
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/api/delete")
async def delete_file(request: Request):
    """Supprime le fichier original"""
    try:
        data = await request.json()
        file_path = normalize_path(data.get('file_path', ''))
        
        if not file_path:
            raise HTTPException(status_code=400, detail="Chemin de fichier manquant")
        
        if not os.path.exists(file_path):
            raise HTTPException(status_code=404, detail="Fichier introuvable")
        
        # Vérifier que c'est bien un fichier MP4
        if not file_path.lower().endswith('.mp4'):
            raise HTTPException(status_code=400, detail="Seuls les fichiers MP4 peuvent être supprimés")
        
        # Envoyer le fichier dans la corbeille Windows
        send2trash(file_path)
        
        return {
            'status': 'success',
            'message': f'Fichier envoyé à la corbeille : {file_path}'
        }
    
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/api/delete-batch")
async def delete_batch(request: Request):
    """Supprime plusieurs fichiers originaux en lot"""
    try:
        data = await request.json()
        file_paths = data.get('file_paths', [])

        if not file_paths:
            raise HTTPException(status_code=400, detail="Aucun fichier à supprimer")

        results = []
        for file_path in file_paths:
            file_path = normalize_path(file_path)
            if not file_path.lower().endswith('.mp4'):
                results.append({'path': file_path, 'status': 'error', 'message': 'Seuls les fichiers MP4 peuvent être supprimés'})
                continue
            if not os.path.exists(file_path):
                results.append({'path': file_path, 'status': 'error', 'message': 'Fichier introuvable'})
                continue
            try:
                send2trash(file_path)
                results.append({'path': file_path, 'status': 'success'})
            except Exception as e:
                results.append({'path': file_path, 'status': 'error', 'message': str(e)})

        success_count = sum(1 for r in results if r['status'] == 'success')
        return {
            'status': 'done',
            'success_count': success_count,
            'total': len(file_paths),
            'results': results
        }

    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))


@app.get("/api/browse-directory")
async def browse_directory():
    """Ouvre une boîte de dialogue Windows pour choisir un répertoire"""
    result = {"path": None}

    def _pick():
        root = tk.Tk()
        root.withdraw()          # Masquer la fenêtre Tk principale
        root.attributes("-topmost", True)  # Mettre la boîte de dialogue au premier plan
        chosen = filedialog.askdirectory(title="Choisir le répertoire à scanner", parent=root)
        root.destroy()
        result["path"] = chosen if chosen else None

    # Exécuter dans un thread séparé pour ne pas bloquer asyncio
    t = threading.Thread(target=_pick)
    t.start()
    t.join()

    if result["path"] is None:
        return {"status": "cancelled", "path": None}
    return {"status": "ok", "path": result["path"]}


@app.post("/api/open-folder")
async def open_folder(request: Request):
    """Ouvre le répertoire contenant le fichier dans l'explorateur Windows"""
    try:
        data = await request.json()
        file_path = normalize_path(data.get('file_path', ''))
        
        if not file_path:
            raise HTTPException(status_code=400, detail="Chemin de fichier manquant")
        
        if not os.path.exists(file_path):
            raise HTTPException(status_code=404, detail="Fichier introuvable")
        
        # Ouvrir l'explorateur Windows et sélectionner le fichier
        # Utiliser une liste pour éviter les problèmes de guillemets dans le nom
        subprocess.Popen(['explorer', f'/select,{file_path}'])
        
        return {
            'status': 'success',
            'message': f'Dossier ouvert : {os.path.dirname(file_path)}'
        }
    
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/api/play-video")
async def play_video(request: Request):
    """Ouvre le fichier vidéo avec le lecteur par défaut (VLC)"""
    try:
        data = await request.json()
        file_path = normalize_path(data.get('file_path', ''))
        
        if not file_path:
            raise HTTPException(status_code=400, detail="Chemin de fichier manquant")
        
        if not os.path.exists(file_path):
            raise HTTPException(status_code=404, detail="Fichier introuvable")
        
        # Ouvrir le fichier avec l'application par défaut (VLC dans votre cas)
        os.startfile(file_path)
        
        return {
            'status': 'success',
            'message': f'Lecture de : {file_path}'
        }
    
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

@app.websocket("/ws/compress/{video_index}")
async def compress_video(websocket: WebSocket, video_index: int):
    """WebSocket pour compresser une vidéo avec progression en temps réel"""
    await websocket.accept()
    
    try:
        # Recevoir les données de la vidéo
        data = await websocket.receive_json()
        input_path = normalize_path(data.get('input_path', ''))
        quality = data.get('quality', 'medium')  # low, medium, high
        
        print(f"[DEBUG] Compression demandée pour: {input_path}")
        print(f"[DEBUG] Qualité: {quality}")
        
        if not os.path.exists(input_path):
            print(f"[ERROR] Fichier introuvable: {input_path}")
            await websocket.send_json({'status': 'error', 'message': 'Fichier introuvable'})
            await websocket.close()
            return
        
        # Créer le nom du fichier de sortie et le fichier temporaire
        path_obj = Path(input_path)
        output_path = str(path_obj.parent / f"{path_obj.stem}_compressed{path_obj.suffix}")
        temp_path = str(path_obj.parent / f"{path_obj.stem}_compressed.tmp.mp4")
        
        # Paramètres de compression selon la qualité
        quality_presets = {
            'low':    {'crf': '28', 'preset': 'fast',   'scale': '480:-2'},
            'medium': {'crf': '23', 'preset': 'fast',   'scale': '1280:-2'},
            'high':   {'crf': '20', 'preset': 'medium', 'scale': '-2:-2'},
        }
        # Correspondance preset CPU → hardware
        hw_preset_map = {
            'h264_nvenc': {'fast': 'p4', 'medium': 'p5', 'slow': 'p6'},
            'h264_qsv':   {'fast': 'faster', 'medium': 'medium', 'slow': 'slow'},
            'h264_amf':   {'fast': 'speed', 'medium': 'balanced', 'slow': 'quality'},
            'libx264':    {'fast': 'fast', 'medium': 'fast', 'slow': 'medium'},
        }
        # CRF/QP selon l'encodeur (NVENC/AMF/QSV utilisent -qp, pas -crf)
        hw_quality_map = {
            'h264_nvenc': {'low': '32', 'medium': '26', 'high': '22'},
            'h264_qsv':   {'low': '32', 'medium': '26', 'high': '22'},
            'h264_amf':   {'low': '32', 'medium': '26', 'high': '22'},
            'libx264':    {'low': '28', 'medium': '23', 'high': '20'},
        }

        preset = quality_presets.get(quality, quality_presets['medium'])
        encoder = HW_ENCODER
        enc_preset = hw_preset_map.get(encoder, hw_preset_map['libx264'])[preset['preset']]
        enc_quality = hw_quality_map.get(encoder, hw_quality_map['libx264'])[quality]

        # Filtre vidéo
        if preset['scale'] == '-2:-2':
            vf_filter = "format=yuv420p"
        else:
            vf_filter = f"scale={preset['scale']},format=yuv420p"

        # Construction de la commande FFmpeg
        cmd = [FFMPEG_CMD, '-i', input_path]

        if encoder == 'libx264':
            cmd += [
                '-vf', vf_filter,
                '-c:v', 'libx264',
                '-preset', enc_preset,
                '-crf', enc_quality,
                '-threads', '0',   # utiliser tous les cores CPU
            ]
        elif encoder == 'h264_nvenc':
            cmd += [
                '-vf', vf_filter,
                '-c:v', 'h264_nvenc',
                '-preset', enc_preset,
                '-rc', 'vbr',
                '-cq', enc_quality,
                '-b:v', '0',
            ]
        elif encoder == 'h264_qsv':
            cmd += [
                '-vf', vf_filter,
                '-c:v', 'h264_qsv',
                '-preset', enc_preset,
                '-global_quality', enc_quality,
            ]
        elif encoder == 'h264_amf':
            cmd += [
                '-vf', vf_filter,
                '-c:v', 'h264_amf',
                '-quality', enc_preset,
                '-qp_i', enc_quality, '-qp_p', enc_quality,
            ]

        cmd += [
            '-ac', '2',
            '-c:a', 'aac',
            '-b:a', '128k',
            '-pix_fmt', 'yuv420p',
            '-movflags', '+faststart',
            '-progress', 'pipe:2',
            '-y',
            temp_path
        ]

        print(f"[INFO] Encodeur utilisé : {encoder} | preset : {enc_preset} | qualité : {enc_quality}")
        
        # Obtenir la durée totale
        print(f"[DEBUG] Obtention des infos vidéo...")
        info = get_video_info(input_path)
        total_duration = 0
        if info and 'format' in info:
            total_duration = float(info['format'].get('duration', 0))
            print(f"[DEBUG] Durée totale: {total_duration}s")
        else:
            print(f"[WARNING] Impossible d'obtenir la durée de la vidéo")
        
        print(f"[DEBUG] Envoi du statut 'started'")
        await websocket.send_json({
            'status': 'started',
            'output_path': output_path,
            'total_duration': total_duration
        })
        
        # Lancer le processus FFmpeg
        print(f"[DEBUG] Lancement de FFmpeg...")
        print(f"[DEBUG] Commande complète:")
        print(f"[DEBUG]   {' '.join(cmd)}")
        print(f"[DEBUG] Fichier d'entrée: {input_path}")
        print(f"[DEBUG] Fichier de sortie: {output_path}")
        print(f"[DEBUG] Fichier existe: {os.path.exists(input_path)}")
        print(f"[DEBUG] Taille fichier: {os.path.getsize(input_path)} bytes")
        
        # Enregistrer le temps de début
        start_time = time.time()
        
        # CREATE_NO_WINDOW : empêche l'apparition de fenêtre DOS sous Windows
        no_window = subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0
        process = subprocess.Popen(
            cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            universal_newlines=True,
            bufsize=1,
            creationflags=no_window
        )
        
        # Stocker le processus dans le dictionnaire
        active_processes[video_index] = process
        print(f"[DEBUG] Processus FFmpeg lancé, PID: {process.pid}")
        
        # Récupérer la boucle asyncio courante pour envoyer les messages depuis un thread
        loop = asyncio.get_event_loop()
        
        # File de messages à envoyer au client
        progress_queue = asyncio.Queue()
        
        # Lire stderr dans un thread séparé et mettre les updates dans la queue
        def read_stderr():
            line_count = 0
            last_progress_sent = -1
            for line in process.stderr:
                line_count += 1
                if line_count <= 50 or line_count % 100 == 0:
                    print(f"[STDERR #{line_count}] {line.rstrip()}")
                
                time_match = re.search(r'out_time_ms=(\d+)', line)
                if time_match:
                    current_time_ms = int(time_match.group(1))
                    current_time_s = current_time_ms / 1000000
                    
                    if total_duration > 0:
                        progress = min(100, (current_time_s / total_duration) * 100)
                        if abs(progress - last_progress_sent) >= 0.5:
                            last_progress_sent = progress
                            asyncio.run_coroutine_threadsafe(
                                progress_queue.put({
                                    'status': 'progress',
                                    'progress': round(progress, 1),
                                    'current_time': round(current_time_s, 1),
                                    'total_duration': total_duration
                                }),
                                loop
                            )
            # Signaler la fin de la lecture
            asyncio.run_coroutine_threadsafe(progress_queue.put(None), loop)
        
        # Lire stdout dans un thread séparé
        def read_stdout():
            for line in process.stdout:
                print(f"[STDOUT] {line.rstrip()}")
        
        threading.Thread(target=read_stdout, daemon=True).start()
        stderr_thread = threading.Thread(target=read_stderr, daemon=True)
        stderr_thread.start()
        
        # Transmettre les messages de progression au client de façon non-bloquante
        print(f"[DEBUG] Lecture de la queue de progression...")
        while True:
            msg = await progress_queue.get()
            if msg is None:
                break  # fin du stderr
            try:
                await websocket.send_json(msg)
                print(f"[PROGRESS] {msg['progress']}% envoyé")
            except Exception as e:
                print(f"[ERROR] Envoi WebSocket: {e}")
                break
        
        # Attendre la fin du processus et du thread stderr
        stderr_thread.join(timeout=5)
        process.wait()
        
        print(f"[DEBUG] Processus terminé avec code: {process.returncode}")
        
        # Retirer le processus du dictionnaire
        if video_index in active_processes:
            del active_processes[video_index]
        
        if process.returncode == 0:
            print(f"[DEBUG] Compression réussie!")
            # Renommer le fichier temporaire vers le nom final
            if os.path.exists(temp_path):
                os.rename(temp_path, output_path)
                print(f"[DEBUG] Fichier temporaire renommé : {temp_path} -> {output_path}")
            if os.path.exists(output_path):
                # Calculer le temps de traitement
                elapsed_time = time.time() - start_time
                hours = int(elapsed_time // 3600)
                minutes = int((elapsed_time % 3600) // 60)
                seconds = int(elapsed_time % 60)
                
                if hours > 0:
                    time_str = f"{hours}h {minutes}m {seconds}s"
                elif minutes > 0:
                    time_str = f"{minutes}m {seconds}s"
                else:
                    time_str = f"{seconds}s"
                
                original_size = get_file_size_gb(input_path)
                compressed_size = get_file_size_gb(output_path)
                reduction = ((original_size - compressed_size) / original_size) * 100
                
                print(f"[DEBUG] Taille originale: {original_size:.2f} GB")
                print(f"[DEBUG] Taille compressée: {compressed_size:.2f} GB")
                print(f"[DEBUG] Réduction: {reduction:.1f}%")
                print(f"[DEBUG] Temps de traitement: {time_str}")
                
                await websocket.send_json({
                    'status': 'completed',
                    'output_path': output_path,
                    'original_size_gb': round(original_size, 2),
                    'compressed_size_gb': round(compressed_size, 2),
                    'reduction_percent': round(reduction, 1),
                    'processing_time': time_str
                })
            else:
                print(f"[ERROR] Le fichier de sortie n'existe pas: {output_path}")
                await websocket.send_json({
                    'status': 'error',
                    'message': 'Le fichier de sortie n\'a pas été créé'
                })
        else:
            error_msg = f'Erreur FFmpeg (code {process.returncode})'
            print(f"[ERROR] {error_msg}")
            # Supprimer le fichier temporaire partiel
            if os.path.exists(temp_path):
                os.remove(temp_path)
                print(f"[DEBUG] Fichier temporaire partiel supprimé : {temp_path}")
            await websocket.send_json({
                'status': 'error',
                'message': error_msg
            })
    
    except Exception as e:
        print(f"[ERROR] Exception lors de la compression: {str(e)}")
        import traceback
        print(f"[ERROR] Traceback: {traceback.format_exc()}")
        await websocket.send_json({
            'status': 'error',
            'message': f'Erreur: {str(e)}'
        })
    finally:
        # Nettoyer le processus du dictionnaire en cas d'erreur
        if video_index in active_processes:
            del active_processes[video_index]
        # Supprimer le fichier temporaire s'il existe encore (arrêt, exception, etc.)
        try:
            if 'temp_path' in locals() and os.path.exists(temp_path):
                os.remove(temp_path)
                print(f"[DEBUG] Fichier temporaire nettoyé dans finally : {temp_path}")
        except Exception:
            pass
        await websocket.close()

@app.post("/api/stop-compression")
async def stop_compression(request: Request):
    """Arrête une compression en cours"""
    try:
        data = await request.json()
        video_index = data.get('video_index')
        
        if video_index is None:
            raise HTTPException(status_code=400, detail="Index vidéo manquant")
        
        if video_index not in active_processes:
            return {
                'status': 'error',
                'message': 'Aucune compression active pour cette vidéo'
            }
        
        # Terminer le processus
        process = active_processes[video_index]
        process.terminate()
        
        # Attendre un peu pour la terminaison propre
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            # Si le processus ne se termine pas, le forcer
            process.kill()
            process.wait()
        
        # Retirer du dictionnaire
        del active_processes[video_index]
        
        return {
            'status': 'success',
            'message': 'Compression arrêtée avec succès'
        }
    
    except Exception as e:
        raise HTTPException(status_code=500, detail=str(e))

if __name__ == "__main__":
    import uvicorn
    import webbrowser
    import threading
    import sys

    def open_browser():
        import time
        time.sleep(2)  # Attendre que le serveur démarre
        webbrowser.open("http://127.0.0.1:8000")

    # Ne pas ouvrir le navigateur si lancé depuis Electron ou en mode dev
    if "--no-browser" not in sys.argv and "--dev" not in sys.argv:
        threading.Thread(target=open_browser, daemon=True).start()

    # En mode exe PyInstaller stdout/stderr sont None — fournir une config
    # de logging sans handler pour éviter le crash des formatters uvicorn
    if getattr(sys, 'frozen', False):
        log_config = {
            "version": 1,
            "disable_existing_loggers": True,
            "handlers": {},
            "loggers": {
                "uvicorn":        {"handlers": [], "level": "CRITICAL"},
                "uvicorn.error":  {"handlers": [], "level": "CRITICAL"},
                "uvicorn.access": {"handlers": [], "propagate": False},
            },
        }
    else:
        log_config = None

    uvicorn.run(app, host="127.0.0.1", port=8000, log_level="warning", log_config=log_config)
