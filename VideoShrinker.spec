# -*- mode: python ; coding: utf-8 -*-
# Fichier de configuration PyInstaller pour Video Shrinker
# Généré manuellement — lancer avec : pyinstaller VideoShrinker.spec

import sys
from pathlib import Path

block_cipher = None

a = Analysis(
    ['app.py'],
    pathex=[],
    binaries=[],
    datas=[
        # Inclure les templates Jinja2
        ('templates', 'templates'),
        # Inclure les fichiers statiques (CSS, JS)
        ('static', 'static'),
    ],
    hiddenimports=[
        # FastAPI / Starlette
        'uvicorn',
        'uvicorn.logging',
        'uvicorn.loops',
        'uvicorn.loops.auto',
        'uvicorn.loops.asyncio',
        'uvicorn.protocols',
        'uvicorn.protocols.http',
        'uvicorn.protocols.http.auto',
        'uvicorn.protocols.http.h11_impl',
        'uvicorn.protocols.http.httptools_impl',
        'uvicorn.protocols.websockets',
        'uvicorn.protocols.websockets.auto',
        'uvicorn.protocols.websockets.websockets_impl',
        'uvicorn.protocols.websockets.wsproto_impl',
        'uvicorn.lifespan',
        'uvicorn.lifespan.on',
        'fastapi',
        'fastapi.responses',
        'fastapi.staticfiles',
        'fastapi.templating',
        'starlette',
        'starlette.routing',
        'starlette.responses',
        'starlette.staticfiles',
        'starlette.templating',
        'starlette.middleware',
        'starlette.middleware.cors',
        'starlette.websockets',
        'jinja2',
        'anyio',
        'anyio._backends._asyncio',
        'anyio._backends._trio',
        'h11',
        'httptools',
        'websockets',
        'wsproto',
        'click',
        'multipart',
        'send2trash',
        'imageio_ffmpeg',
        'tkinter',
    ],
    hookspath=[],
    hooksconfig={},
    runtime_hooks=[],
    excludes=[
        'matplotlib',
        'numpy',
        'pandas',
        'PIL',
        'scipy',
    ],
    win_no_prefer_redirects=False,
    win_private_assemblies=False,
    cipher=block_cipher,
    noarchive=False,
)

pyz = PYZ(a.pure, a.zipped_data, cipher=block_cipher)

exe = EXE(
    pyz,
    a.scripts,
    a.binaries,
    a.zipfiles,
    a.datas,
    [],
    name='VideoShrinker',
    debug=False,
    bootloader_ignore_signals=False,
    strip=False,
    upx=True,           # Compresser l'exécutable (nécessite UPX installé, optionnel)
    upx_exclude=[],
    runtime_tmpdir=None,
    console=True,       # Fenêtre console visible pour les logs (fermer = quitter l'app)
    disable_windowed_traceback=False,
    target_arch=None,
    codesign_identity=None,
    entitlements_file=None,
    # icon='static/icon.ico',  # Décommenter si tu ajoutes une icône .ico
)
