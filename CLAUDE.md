# WorkTable

App desktop Rust (egui) d'organisation de travail : tableau de bord, calendrier, mail, messages, notes, fichiers, Blueprint. DA monochrome.

## Règles
- Réponds en français, concis, sans récapitulatif.
- Fais UNIQUEMENT ce qui est demandé : aucune fonctionnalité, refacto ou amélioration non demandée.
- Fonctionnalité sur plus d'un fichier : plan court d'abord, j'attends ta validation.
- Pas d'intégration d'IA payante (API Claude).
- Git : ne jamais commit/branch/tag/push. Donne-moi les commandes à lancer moi-même (je travaille sur plusieurs PC).

## Économie de tokens
- Ne lis pas README.md (50 Ko), CHANGELOG.md, Cargo.lock, assets/, installer/, data/, target/.
- Beaucoup de fichiers dépassent 40 Ko : utilise Grep pour trouver la fonction, puis Read avec offset/limit. Ne lis jamais un gros fichier en entier.
- Agents : `explorateur` (où est le code), `build-fixer` (erreurs de compilation), `planificateur` (avant une fonctionnalité multi-fichiers), `relecteur` (avant de me donner les commandes de commit), `changelog` (préparer une version), `decoupeur` (uniquement si je le demande).
- Vérifie avec `cargo check`, jamais `cargo build --release`.

## Carte du code (src/)
- main.rs, app.rs : démarrage et boucle principale
- sidebar.rs, titlebar.rs, theme.rs, widgets.rs : interface commune
- tabs/ : un onglet par fichier + sous-dossier
  - dashboard.rs, dash_layout.rs : tableau de bord
  - calendar/ (grid, month, panel, side)
  - mail/ (view, composer, contacts, drafts…) — mail/view.rs = 85 Ko
  - messages/ (view, emoji, gif…) — messages/view.rs = 100 Ko
  - notes/ (editor, links, inline)
  - files/, timeline.rs, settings.rs, custom.rs
  - blueprint/ : graphs de nodes façon UE5 (editor, nodes, canvas, runtime, sim, store, library). Intégration étape par étape avec validation.
- google/ : auth, gmail, calendar, drive, chat, compose, html
- notes_store.rs, tasks.rs, people.rs, prefs.rs, paths.rs : données
- search.rs, notify.rs, spell.rs, update.rs, background.rs : services
