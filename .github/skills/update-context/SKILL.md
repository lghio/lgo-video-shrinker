---
name: update-context
description: "Use when: updating CONTEXT.md, refreshing project context, documenting session changes in CHANGELOG.md, running /update-context, or synchronizing project structure and technical conventions."
---

# Update Context — lgo-video-shrinker

Projet : `d:/python/lgo-video-shrinker`

Mets à jour le fichier `CONTEXT.md` (et `CHANGELOG.md`) à la racine du projet en suivant ces étapes :

## Étape 1 — Lister tous les fichiers du projet

Liste les fichiers du projet (hors `.git/`, `node_modules/`, `dist-frontend/`, `src-tauri/target/`) pour détecter les nouveaux fichiers, déplacements ou suppressions.

## Étape 2 — Mettre à jour la section "Project structure" de `CONTEXT.md`

- Ajoute les nouveaux fichiers ou dossiers avec leur description
- Supprime les fichiers qui n'existent plus
- Mets à jour les descriptions si le rôle d'un fichier a changé

## Étape 3 — Mettre à jour `CHANGELOG.md`

- Ajoute une ligne en tête du tableau avec la date d'aujourd'hui (`YYYY-MM-DD`) et un résumé concis des modifications effectuées pendant cette session (format Conventional Commits : `feat(...)`, `fix(...)`, `docs(...)`, `chore(...)`)
- Garde toutes les entrées précédentes intactes
- Conserve dans `CONTEXT.md` le renvoi vers `CHANGELOG.md` pour l'historique détaillé

## Étape 4 — Mettre à jour la section "Key implementation decisions" / "Conventions techniques"

- Documente toute nouvelle décision d'architecture, convention technique, endpoint/commande IPC ou règle métier établie pendant la session
- Ne modifie pas la description générale du projet sauf si le périmètre ou la stack a changé

## Règles

- Effectue les modifications directement dans `CONTEXT.md` et `CHANGELOG.md` sans demander de confirmation préalable
- Ne jamais inclure de secrets ou de contenus provenant d'un fichier `.env` ou de clés privées (`TAURI_SIGNING_PRIVATE_KEY`)
