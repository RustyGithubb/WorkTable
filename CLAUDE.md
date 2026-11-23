# WorkTable

App desktop Rust (egui) d'organisation de travail : tableau de bord, calendrier, mail, messages, notes, fichiers, Blueprint. DA monochrome.

## Règles
- Réponds en français, concis, sans récapitulatif.
- Fais UNIQUEMENT ce qui est demandé : aucune fonctionnalité, refacto ou amélioration non demandée.
- Fonctionnalité sur plus d'un fichier : plan court d'abord, j'attends ta validation.
- Pas d'intégration d'IA payante (API Claude).
- Git : ne jamais commit/branch/tag/push. Donne-moi les commandes à lancer moi-même (je travaille sur plusieurs PC).

## DA — panneaux superposés (obligatoire)
Tout panneau ouvert par-dessus l'app (recherche Ctrl+Espace, panneau Compte, fiche contact, demande de réservation…) suit ces règles. Modèles : `search.rs` (`Palette`), `account_panel.rs`.
- Fond : `search::Backdrop` (capture floutée + voile sombre, fondu 0,14 s). `bg.start(ctx)` à l'ouverture, `bg.clear()` à la fermeture.
- Boîte : `search::overlay_frame()` dans une `egui::Area` à `Order::Tooltip`, contenu en `ui.multiply_opacity(k)` (fondu du fond).
- Premier plan : après la boîte, à chaque image, `ctx.move_to_top(LayerId::new(Order::Tooltip, id_de_la_boîte))`. Sinon un clic sur le fond le fait passer devant et la boîte disparaît (y compris à la réouverture).
- Fermeture : Échap ou clic sur le fond (un clic sur le bouton qui l'a ouvert compte comme un clic sur le fond).
- Largeur et position : 640 px (`min(largeur − 40)`), centrée ; haut à 16 % de la hauteur de la fenêtre, glissement de 12 px pendant le fondu.
- Étirement en hauteur : même écart sous le panneau qu'au-dessus (16 % de la hauteur), quelle que soit la taille de la fenêtre. Le contenu défile dans une `ScrollArea` de hauteur imposée : `min_scrolled_height(h).max_height(h).auto_shrink([false, false])`, avec `h` = bas visé − marge du cadre (14 px) − `ui.cursor().top()`. Sans `min_scrolled_height`, egui tasse la zone et le panneau s'arrête n'importe où.
- En-tête fixe (hors de la zone qui défile) ; sections = titre en gras + filets `theme::BORDER()` (8 px avant et après), pas de `widgets::card` dans un panneau, Réglages compris.
- Réglages : panneau superposé (`tabs::settings::SettingsPanel`), ouvert/fermé par la pilule ⚙️ seulement, jamais un onglet ; sections via `settings::group`.
- Image ronde dans une pastille (ex. pastille Compte) : l'image remplit toute la pastille (pas de marge intérieure), liseré `BORDER`, `TEXT_MUTED` au survol, `TEXT` et plus épais quand son panneau est ouvert.
- Nom de l'utilisateur affiché : celui de sa fiche (prénom et nom Google), jamais « Vous ».

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
- sidebar.rs (pastille Compte incluse), titlebar.rs, theme.rs, widgets.rs : interface commune
- account_panel.rs : panneau Compte (overlay façon Ctrl+Espace : image de profil, compte NAS, compte Google, HyperPlanning)
- avatar.rs : image du compte (photo Google, initiale ou image importée ; `config\avatar.json` et `config\avatar.png`, synchronisés)
- tabs/ : un onglet par fichier + sous-dossier
  - dashboard.rs, dash_layout.rs : tableau de bord
  - calendar/ (grid, month, panel, side)
  - mail/ (view, composer, contacts, drafts…) — mail/view.rs = 85 Ko
  - messages/ (view, emoji, gif…) — messages/view.rs = 100 Ko
  - notes/ (editor, links, inline)
  - files/, timeline.rs, custom.rs
  - settings.rs : panneau Réglages (overlay façon Ctrl+Espace, pilule ⚙️), sections `group`
  - blueprint/ : graphs de nodes façon UE5 (editor, nodes, canvas, runtime, sim, store, library). Intégration étape par étape avec validation.
- google/ : auth, gmail, calendar, drive, chat, compose, html ; account_sync.rs (compte Google rattaché au compte NAS : identifiants OAuth et jeton copiés chiffrés dans `config\google_account.bin`, repris sur un autre poste)
- notes_store.rs, tasks.rs, people.rs, prefs.rs : données ; paths.rs : un dossier `profiles\<identifiant>\` par compte (config, data, secrets, sync ; `_local` sans compte ; `profiles.json` = compte ouvert)
- sync/ : synchronisation avec le NAS (crate `nas-sync-client` du dépôt NAS) — mod.rs (compte, coffre, fil, `startup()` appelé dans main avant le chargement), engine.rs (fichiers synchronisés, réception mise de côté, application et fusion au démarrage, envoi), merge.rs (fusion JSON à trois voies), ui.rs (partie Compte NAS du panneau Compte). `paths::write_atomic` appelle `sync::poke()`
- search.rs, notify.rs, spell.rs, update.rs, background.rs : services
