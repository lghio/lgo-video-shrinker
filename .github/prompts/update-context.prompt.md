---
description: "Met à jour CONTEXT.md avec la description des fichiers actuels et un résumé des derniers changements de la session."
---

Mets à jour le fichier `CONTEXT.md` à la racine du projet en suivant ces étapes :

1. **Liste tous les fichiers du projet** (hors `.venv/`, `venv/`, `.git/`, `__pycache__/`, `node_modules/`, `build/`, `dist/`, `electron/node_modules/`, `electron/dist/`, `src-tauri/target/`) pour détecter les nouveaux fichiers ou suppressions.

2. **Met à jour la section "Project structure"** :
   - Ajoute les nouveaux fichiers avec leur description
   - Supprime les fichiers qui n'existent plus
   - Mets à jour les descriptions si le rôle d'un fichier a changé

3. **Met à jour `CHANGELOG.md`** (section "Derniers changements" externalisée depuis `CONTEXT.md`) :
   - Ajoute une ligne en tête du tableau avec la date d'aujourd'hui et un résumé concis des modifications effectuées pendant cette session
   - Garde toutes les entrées précédentes
   - Ne touche pas à la section correspondante dans `CONTEXT.md` (elle contient juste un lien vers `CHANGELOG.md`)

4. **Met à jour la section "Key implementation decisions"** si de nouvelles conventions ont été établies pendant la session.

5. **Ne modifie pas** la section de présentation du projet sauf si le périmètre ou la stack du projet a changé.

Effectue les modifications directement dans `CONTEXT.md` et `CHANGELOG.md` sans demander de confirmation.
