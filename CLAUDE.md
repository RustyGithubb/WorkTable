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
- Vérifie avec `cargo check`, jamais `cargo build --release`.

## Agents (`.claude/agents/`, versionnés : présents sur chaque poste après `git pull`)
Utilise-les systématiquement pendant le dev :
- `explorateur` : avant de lire du code, pour toute question « où est… / qui appelle… ».
- `planificateur` : avant toute fonctionnalité sur plus d'un fichier ; son plan est celui que je valide.
- `build-fixer` : dès que `cargo check` échoue.
- `relecteur` : toujours, avant de me donner les commandes de commit.
- `changelog` : quand je prépare une version (puis vérifie `cargo fmt --check`, exigé par la CI).
- `decoupeur` : uniquement si je le demande.

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
