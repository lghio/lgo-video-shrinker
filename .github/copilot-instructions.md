# Instructions Copilot — lgo-video-shrinker

## Règle de démarrage obligatoire

**Au début de chaque session de chat**, lis TOUJOURS le fichier `CONTEXT.md` à la racine du projet AVANT de faire quoi que ce soit d'autre. Ce fichier contient :
- La description à jour de tous les fichiers du projet
- Le lien vers le résumé des derniers changements effectués (`CHANGELOG.md`)
- Les conventions et choix techniques du projet

Cela évite de devoir relire tout le code à chaque session.

## Comportement attendu

- Ne jamais explorer tout le codebase si `CONTEXT.md` répond déjà à la question
- Après chaque session de modifications importantes, proposer à l'utilisateur de lancer `/update-context` (ou le skill `update-context`) pour mettre `CONTEXT.md` et `CHANGELOG.md` à jour
- Maintenir `CONTEXT.md` comme source de vérité du projet

## Projet

**LGO Video Shrinker** est une application desktop Windows permettant de scanner récursivement des répertoires de vidéos `.mp4` volumineuses, de les compresser avec FFmpeg (accélération matérielle NVENC / QuickSync / AMF ou CPU `libx264`) avec suivi de progression en temps réel, et d'envoyer les fichiers originaux dans la corbeille Windows en toute sécurité.

## Sécurité

- Ne jamais lire, afficher, copier ou transmettre le contenu d'un fichier `.env` ou de clés privées de signature (ex. `TAURI_SIGNING_PRIVATE_KEY`, `*.key`).
- Ne jamais supprimer définitivement des fichiers vidéo utilisateur hors de la corbeille Windows (`send2trash` / `trash`).
