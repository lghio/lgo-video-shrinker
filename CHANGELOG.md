# Changelog — LGO Video Shrinker

Toutes les modifications notables sont documentées ici, par ordre chronologique décroissant.

| Date | Résumé |
|---|---|
| 2026-11-15 | feat(tauri,updater,qa): implémentation complète de la v1 **Tauri v2** (`package.json`, `vite.config.js`, frontend `src/` avec Bootstrap 5.3 local et bannière de mise à jour automatique `src/js/updater.js`, backend Rust `src-tauri/` avec sidecars `ffmpeg`/`ffprobe`, détection hardware `h264_nvenc`/`qsv`/`amf`/`libx264`, support des chemins longs Windows `\\?\`, écriture atomique `.tmp.mp4`, suppression sécurisée vers la corbeille via `trash`, et workflow CI/CD `.github/workflows/release.yml`) ; validation TNR QA complète (`npm run build`, `cargo check`, et 15 tests unitaires et E2E Rust passés). |
| 2026-11-15 | feat(agents,skills,docs): création des 4 agents Copilot (`.github/agents/orchestrator.agent.md`, `developer.agent.md`, `product-manager.agent.md`, `qa.agent.md`), du skill `.github/skills/update-context/SKILL.md` et du prompt `.github/prompts/update-context.prompt.md`, de `.github/copilot-instructions.md`, `CONTEXT.md` et `CHANGELOG.md` ; rédaction de l'architecture cible Tauri v2 avec mise à jour automatique (`stack.md`) et du plan de migration détaillé (`PLAN/to-tauri.md`). |
