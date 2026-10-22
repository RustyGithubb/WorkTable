# WorkTable

Application d'organisation de travail en Rust (`eframe` / `egui` 0.37).
Onglets : Tableau de bord, Calendrier, Mail (Gmail), Messages (Google Chat), Prise de notes, Fichiers (Google Drive), Réglages.
Interface en français ou en anglais (Réglages › Général › Langue ; par défaut, la langue de Windows).

## Lancer
```
cargo run
```
Rust ≥ 1.85 (édition 2024). Tests : `cargo test`.

## Configurer Google (une seule fois, ~10 min)

WorkTable utilise l'API Gmail officielle. Google exige que chaque application ait
ses propres identifiants OAuth : il faut donc les créer dans ta console Google Cloud.

1. Va sur https://console.cloud.google.com et crée un projet (ex. « WorkTable »).
2. **API et services › Bibliothèque** : cherche **Gmail API** et clique sur **Activer**.
3. **Google Auth Platform** (ex-« Écran de consentement OAuth ») › **Commencer** :
   nom de l'app « WorkTable », ton adresse en support, audience **Externe**.
4. **Audience › Utilisateurs test** : ajoute ton adresse Gmail.
5. **Clients › Créer un client** : type **Application de bureau** › Créer.
   Copie le **Client ID** et le **Client secret**.
6. Dans WorkTable (onglet Mail ou Réglages) : colle les deux valeurs › **Connecter Gmail**.
   Le navigateur s'ouvre : choisis ton compte. Google affiche « Google n'a pas validé
   cette application » (normal, c'est ton app) › **Continuer** › **Autoriser**.

Ensuite la connexion est automatique à chaque lancement.

7. **Agenda** : dans **API et services › Bibliothèque**, active aussi **Google Calendar API**.
   WorkTable demande désormais aussi l'accès à Google Agenda et à la lecture des réglages Gmail
   (signatures) : si tu étais déjà connecté, l'onglet Calendrier affiche « Autoriser l'accès à
   Google Agenda » → reconnecte-toi une fois pour accorder ces autorisations.

8. **Google Chat (onglet Messages)** — compte Google Workspace requis (ex. @artfx.fr) :
   - **Bibliothèque** : active **Google Chat API** et **People API** (noms des membres).
   - **Google Chat API › Configuration** : renseigne un nom d'application (ex. « WorkTable »),
     une URL d'avatar et une description, décoche les fonctionnalités interactives, enregistre.
     (Sans cette étape Google répond « Chat app not found ».)
   - Reconnecte-toi une fois : l'onglet Messages affiche « Autoriser l'accès à Google Chat ».

9. **Google Drive (onglet Fichiers)** : active **Google Drive API** dans la Bibliothèque, puis
   reconnecte-toi une fois (« Autoriser l'accès à Google Drive »). Accès en lecture seule.

**Limite du mode test :** tant que l'app est en statut « Test » dans Google Cloud, Google
invalide l'autorisation au bout de 7 jours : il suffit de cliquer à nouveau sur
« Connecter Gmail ». Pour éviter ça, passe l'app en **Production** (Audience › Publier) ;
pour un usage personnel, aucune validation Google n'est nécessaire, l'avertissement
« app non validée » restera simplement affiché à la connexion.

### Où sont stockées les données
L'emplacement dépend de la façon dont l'app est lancée (Réglages › Données l'affiche, avec
« Ouvrir le dossier » et « Importer… ») :

| Mode | Quand | Données |
|---|---|---|
| **Installée** | lancée depuis l'installeur | `%APPDATA%\WorkTable\` (+ cache dans `%LOCALAPPDATA%\WorkTable\cache\`) |
| **Portable** | un dossier `data\` (ou un fichier vide `portable`) à côté de l'exe | ce dossier `data\` |
| **Développement** | `cargo run` | `data\` du projet |
| Forcé | variable `WORKTABLE_DATA=<dossier>` | ce dossier |

Ce dossier et ses sous-dossiers (`config\`, `data\notes\`, `secrets\`, cache) sont créés dès le
premier lancement — et dès l'installation pour une installation par utilisateur —, puis recréés si
l'un d'eux a été supprimé. Dans ce dossier :
- `config\` : réglages (`prefs.json`), tâches, contacts, mails épinglés, liens et logos, icônes
  et annuaire Chat, `crash.log` ;
- `data\notes\` : notes Markdown et leurs pièces jointes ;
- `cache\` : émojis téléchargés (recréés automatiquement si absents) et `chat.wtc`, copie chiffrée
  des conversations Google Chat récentes (voir Messages) ;
- `secrets\` : identifiants Google (`google.json`) et jeton de connexion — **ignoré par git**,
  à ne jamais publier.

Les fichiers sont enregistrés de façon atomique (fichier temporaire puis renommage) : une coupure
ne laisse jamais un réglage ou une note à moitié écrit. Aucun mail n'est stocké sur le disque.
Pour passer d'une version portable / de développement à la version installée : Réglages ›
Données › **Importer…** et choisir l'ancien dossier `data`.

## Installeur Windows
```
powershell -ExecutionPolicy Bypass -File installer\build.ps1
```
Prérequis : Rust et [Inno Setup 7](https://jrsoftware.org/isdl.php) (ou 6). Le script compile la version
finale, lit la version dans `Cargo.toml` et produit `installer\Output\WorkTable-<version>-setup.exe`.
- Installation par utilisateur (sans droits administrateur) dans `%LOCALAPPDATA%\Programs\WorkTable`,
  ou pour tous les utilisateurs ; raccourcis menu Démarrer / Bureau, lancement à l'ouverture de
  session en option. Icône, nom et version visibles dans « Applications installées ».
- Les données restent dans le profil de l'utilisateur : une mise à jour (nouvel installeur) les
  conserve ; à la désinstallation, l'utilisateur choisit de les garder ou de les supprimer.
- Pour livrer les identifiants OAuth Google avec l'app, placer `google.json` dans `installer\`
  avant de construire (ignoré par git) : il est installé à côté de l'exe et sert par défaut.

## Fichiers (Google Drive)
- Navigation : Mon Drive, Partagés avec moi, Récents, Suivis, Favoris, Drives partagés (colonne masquable « ).
- **Favoris** (propres à WorkTable, distincts des « Suivis » de Drive) : clic droit sur un dossier ou un
  fichier › « Ajouter aux favoris » (ou bouton du panneau de détail). La catégorie **Favoris** de la
  navigation liste « Tous les favoris » (vue liste / grille) puis chaque favori : clic sur un dossier =
  l'ouvrir, sur un fichier = son détail, double-clic = l'ouvrir, clic droit = retirer. Étoile de
  couleur sur les favoris. Sauvegarde : `data\config\drive_favorites.json`.
- Fil d'Ariane cliquable, retour (‹ ou Retour arrière), tri Nom / Modifié, vue Liste / Grille
  (miniatures), « Charger plus » par pages de 100.
- Recherche Drive (nom et contenu des fichiers) ; les fichiers Drive apparaissent aussi dans la
  recherche globale Ctrl+Espace.
- Double-clic : dossier = l'ouvrir ; document Google = navigateur ; autre fichier = téléchargé puis
  ouvert avec l'application Windows par défaut (navigateur en dernier recours). Clic droit et panneau
  de détail : ouvrir dans le navigateur, ouvrir un Docs / Sheets / Slides avec une application du PC
  (export docx / xlsx / pptx), télécharger, copier le lien, afficher l'emplacement.
- Le panneau de détail s'ouvre et se referme avec l'animation des colonnes de Google Chat.
- **Fiche « Auteur »** dans le panneau de détail (ouvert au clic sur un fichier) : nom et adresse du
  propriétaire (Drive partagé : dernière personne à l'avoir modifié), et ce que WorkTable sait déjà de
  lui : annuaire (mails échangés), ses 3 derniers mails, la conversation Google Chat avec lui,
  les rendez-vous en commun — chaque ligne s'ouvre dans son onglet. Boutons : écrire un mail, ses
  mails, ouvrir / écrire sur Chat, « Ses fichiers » (fichiers Drive dont il est propriétaire),
  « Fiche contact ».
- Comme pour Gmail : dossiers gardés en mémoire (relus au plus toutes les 2 minutes), champs limités,
  requêtes via le même limiteur et les mêmes reprises.

## Fiches contact
Annuaire caché, sans onglet : chaque personne croisée dans l'app (mails, Google Chat, invités des
rendez-vous, auteurs des fichiers Drive) a une fiche qui rassemble ce qui la relie à toi.
- **Ouverture** : clic droit sur une trace de la personne › « 👤 Fiche contact » — un mail (liste,
  expéditeur, À / Cc, tableau de bord), un message ou une conversation Google Chat (membres d'un
  groupe), un fichier Drive (propriétaire, dernière modification), un rendez-vous (invités, dans la
  grille, le panneau ou le planning du tableau de bord), un contact de l'annuaire, un fichier reçu,
  un message épinglé. Aussi depuis Ctrl+Espace (en premier) et la fiche « Auteur » du Drive.
- **Overlay** (même style que Ctrl+Espace) : conversation privée, groupes en commun, mails (reçus,
  envoyés, en copie), fichiers, rendez-vous (prochains d'abord) — clic ou Entrée pour s'y rendre.
  Boutons : écrire, Chat, annuaire, et **« Chercher ses mails »** : seule requête à Gmail, à la
  demande, pour relier tous les mails échangés avec cette personne.
- **Sans requête supplémentaire** : les fiches se remplissent avec ce que les onglets ont déjà
  chargé ; les conversations viennent de la copie locale de Google Chat.
- **Stockage** : `cache\people.wtc`, chiffré (AES-256-GCM) avec la clé de la copie des
  conversations (protégée par Windows), lié au compte, effacé à la déconnexion ou par Réglages ›
  Données › Vider. 40 mails, 40 fichiers et 30 rendez-vous gardés par personne.

## Recherche globale (Ctrl+Espace)
Barre de recherche au centre, par-dessus l'app, sur un fond flouté. Elle trouve :
- les **mails** (déjà chargés, plus une recherche Gmail après une courte pause de frappe) ;
- les **fiches contact** (proposées en premier) : Entrée ouvre la fiche de la personne ;
- les **événements** du calendrier et les **dates** (« demain », « vendredi », « 25/10 », « 3 novembre ») ;
- les fichiers **Google Drive** (nom et contenu) ;
- les conversations et messages **Google Chat**, les **tâches** (archives comprises), les **fichiers partagés** ;
- les **notes** (titre et texte ; seul le titre des notes protégées) ;
- des **actions** : nouveau message, nouvel événement, nouvelle note, aller à un onglet.
↑ / ↓ pour choisir, Entrée ou clic pour s'y rendre, Échap pour fermer. Sans texte : prochains
événements et actions rapides.

## Prise de notes
Colonne de gauche : **＋ Nouvelle note**, recherche, **Favoris** (étoile au survol ou clic droit)
et toutes les **Notes** (les plus récentes en haut). Clic droit : favori, bloquer l'éditeur,
mot de passe, supprimer.

À droite, une page sans bord façon Notion / Obsidian :
- **Blocs** : texte, titres 1-3, listes à puces / numérotées, cases à cocher, citation, code,
  séparateur, image, fichier joint. Menu **« / »** pour insérer un bloc (↑ ↓ Entrée), et
  raccourcis Markdown en début de ligne : `# `, `## `, `### `, `- `, `1. `, `[] `, `> `, ` ``` `, `---`.
- **Mise en forme** (barre d'outils ou clavier) : gras Ctrl+B, italique Ctrl+I, souligné Ctrl+U,
  barré Ctrl+Maj+X, code Ctrl+E, surligné Ctrl+Maj+H, lien Ctrl+K (Ctrl+clic pour l'ouvrir).
- Entrée : nouveau bloc (les listes continuent, Entrée sur une puce vide en sort) ;
  Retour arrière en début de bloc : redevient du texte puis fusionne ; Alt+↑ / Alt+↓ : déplacer le bloc.
  Coller du Markdown le découpe en blocs.
- **Images** : bouton 🖼, commande « /image » ou glisser-déposer ; les autres fichiers déposés
  deviennent des pièces jointes 📎 (copiées avec la note, ouvertes avec l'application par défaut).
- **Rattacher** : un événement du calendrier, une date, un mail, un message Google Chat, une
  tâche ou un fichier Google Drive. Les pastilles sous le titre y mènent en un clic.
  Dans le texte, tape **/mail**, **/message**, **/événement**, **/tâche** ou **/drive** : la mention
  s'insère à l'endroit du curseur (clic dessus en lecture, Ctrl+clic en édition, pour l'ouvrir).
- **Édition / Lecture** : bloque ou débloque l'éditeur (mémorisé par note). En lecture, les
  liens s'ouvrent d'un clic et les cases restent cochables.
- **Mot de passe** (🔒) : texte, liens, images et pièces jointes sont chiffrés (AES-256-GCM, clé
  dérivée par Argon2id). Sans le mot de passe la note s'affiche floutée ; elle se reverrouille
  quand on en ouvre une autre. Un mot de passe oublié ne peut pas être récupéré.

**Stockage local uniquement** : `data\data\notes\` (menu ⋯ → « Ouvrir
le dossier des notes »).
- `<id>.md` : une note en Markdown (lisible par Obsidian), en-tête `worktable:` (titre, dates,
  favori, lecture seule, liens) ;
- `attachments/<id>/` : ses images et fichiers joints ;
- `<id>.locked.json` : une note protégée (seul le titre est en clair).
Enregistrement automatique une seconde après la dernière frappe.

## Tableau de bord
- **Ne pas déranger** : pilule à gauche de l'heure (visible seulement dans le tableau de bord), dont la
  pastille porte une tête de raton laveur : normale (yeux ouverts), puis masquée (loup sur les yeux
  fermés). Un clic fait couler la pastille d'un bout à l'autre comme un liquide (ease-in-out : elle
  s'étire en chemin puis se reforme en goutte) : plus
  aucun bandeau ni son de notification jusqu'au clic suivant (le bandeau en cours disparaît ; les
  notifications reçues entre-temps ne sont pas rejouées). Chaque bascule fait vibrer les panneaux en
  vague partant du bouton. Mémorisé entre deux lancements.
- Titre : une phrase d'accueil tirée au hasard à chaque lancement (une douzaine).
- **Tâches** : l'onglet « Archives » à côté du titre affiche les tâches archivées (restaurer / supprimer).
- **Liens Sites Important** (premier panneau) : galerie 3 × 3 de logos cliquables qui ouvrent un
  site. Clic droit › Modifier : nom, adresse, logo en émoji (ou « :nom: », sélecteur) ou image
  (recadrée et redimensionnée en 256 × 256). Glisser une case sur une autre pour les échanger ;
  case vide « + » pour ajouter. Enregistré dans `data\config\links.json`
  (images dans `data\config\links\`).
- **Planning du jour** : événements des agendas affichés dans le Calendrier (journée entière en tête),
  ligne « maintenant », événement en cours surligné, « Dans 25 min » pour le suivant ; clic = ouvre
  l'événement dans le Calendrier.
- **Mails** : mails épinglés puis les 5 derniers non lus de la boîte de réception ; clic = ouvre le mail ; clic droit sur un mail épinglé › Désépingler.
- **Messages épinglés** (à côté des Mails) : messages Google Chat épinglés depuis l'onglet Messages ;
  clic = ouvre la conversation, clic droit = désépingler. Sauvegarde : `data\config\chat_pinned.json`.
- **Rappels** : tâches « À faire » depuis les jours précédents et « En cours » depuis 3 jours ou plus,
  avec « Commencer » / « Archiver ».
  - Bouton **+** en bas à droite : menu d'actions. **⏰ Rappel minuté** : une tâche (existante, ou
    nouvelle, ajoutée à « À faire ») rappelée **dans** un délai (5 min… 1 jour, ou heures / minutes)
    ou **le** jour et à l'heure choisis (raccourcis : ce soir, demain, lundi).
  - Les rappels s'affichent en tête du panneau avec un compte à rebours ; échus : en rouge, avec
    « +10 min », « ✓ Fait » (archive la tâche) et « ✕ ». Réveil et heure sur la carte Kanban.
  - À l'échéance : notification avec un logo réveil dédié (même pendant « Ne pas déranger » ;
    « Rappel en retard » si l'app était fermée), clic = la tâche dans le tableau de bord.
  - Calendrier : pastille ⏰ et barre pointillée à l'heure du rappel (vue Mois : pastille du jour) ;
    clic = la tâche, mise en évidence dans le panneau Rappels. Sauvegarde avec les tâches.
- **Tâches** (Kanban) : À faire / En cours / Archivé. Ajout avec Entrée, glisser-déposer d'une colonne à
  l'autre, boutons ◀ ▶ ✓ ✕, double-clic pour renommer, clic droit › Renommer / Effacer la tâche. Le bloc « Archivé » est vidé chaque jour : ses
  tâches passent dans **Archives** (repliées sous le Kanban, par jour, avec « Restaurer »).
  Sauvegarde : `data\config\tasks.json`.
- **Fichiers reçus** : les 6 derniers fichiers partagés par mail, uniquement PDF (pièces jointes) et
  documents Google Docs / Sheets / Forms (liens, ex. notifications de partage) ; clic = ouvre le PDF
  avec l'application par défaut ou le document dans le navigateur, clic droit = voir le mail.

## Mail
- Réception : 40 derniers messages du dossier, relève automatique toutes les 60 s + bouton Actualiser.
- Économie de quota : chaque mail n'est téléchargé qu'une fois (cache en mémoire), la relève
  utilise l'API `history` (1 requête si rien n'a changé), 4 requêtes simultanées maximum,
  et nouvelle tentative automatique (1 s, 2 s, 4 s, 8 s) si Gmail répond « trop de requêtes ».
- Lecture : texte du mail avec ses images à leur emplacement d'origine (images intégrées,
  hébergées ou encodées ; les pixels de suivi 1×1 sont ignorés). Ouvrir un mail le marque comme lu.
- Corbeille : bouton « 🗑 Corbeille » dans la lecture, touche Suppr, ou clic droit sur un mail de la
  liste — toute la conversation quitte la boîte de réception ; depuis le dossier Corbeille, « Restaurer » le remet dans la boîte de réception.
- Liste : l'épingle, l'étoile et le nom du groupe coloré n'apparaissent qu'au survol d'un mail.
- Mise en forme affichée :
  - HTML (ce que produit l'éditeur de Gmail) : gras `<b>`, italique `<i>`, souligné `<u>`,
    barré `<strike>`, tailles `<font size>` / `font-size`, titres `<h1>`…, alignement `text-align`,
    listes `<ul>/<ol>`, citations `<blockquote>`, liens cliquables `<a href>`, code `<code>/<pre>` ;
  - Markdown écrit dans le texte : `**gras**`, `__gras__`, `*italique*`, `_italique_`,
    `~~barré~~`, `` `code` ``, `# titre`, `> citation`, `- puce`.
- Rédaction (Nouveau message / Répondre / Transférer) :
  - taille (Petit, Normal, Grand, Titre) et alignement (gauche, centre, droite, justifié) par paragraphe ;
  - gras : sélectionner du texte puis « B » (noté `**texte**`, affiché en gras) ;
  - « Image » insère une image à l'emplacement du curseur ; « Pièce jointe » ajoute des fichiers ;
  - glisser-déposer : les images vont dans le texte, les autres fichiers en pièces jointes ;
  - Entrée = nouveau paragraphe, Retour arrière en début de paragraphe = fusion / retrait de l'image ;
  - envoi en HTML + texte, 25 Mo max (les pièces jointes du mail d'origine ne sont pas re-transférées).
- Pièces jointes : pastilles sous l'en-tête du mail. Clic →
  1. images et fichiers texte : aperçu directement dans l'app ;
  2. autres fichiers (PDF, Word…) : ouverts avec l'application par défaut de Windows
     (copie temporaire dans `%TEMP%\WorkTable`) ;
  3. en dernier recours seulement (téléchargement refusé, aucune application, ou fichier
     exécutable bloqué par sécurité) : pastille « Voir dans Gmail » qui ouvre le message dans le navigateur.
- Annuaire (Mail › Annuaire) :
  - chaque expéditeur des mails reçus (et chaque destinataire de tes envois) est enregistré
    dans `data\config\contacts.json` ;
  - recherche par nom ou adresse (sans tenir compte des accents) ; étoile = favori,
    les favoris sont mis en avant en tête de liste (fond éclairé, liseré, nom en gras) ;
  - fiche contact : Écrire, Voir ses mails (recherche Gmail `from:`), favori, retirer ;
  - clic sur l'expéditeur d'un mail ouvert : voir tous ses mails, écrire, favori, ouvrir dans l'annuaire ;
  - champs À / Cc : suggestions pendant la frappe (favoris d'abord), ↑ ↓ pour choisir,
    Tab / Entrée / clic pour ajouter, Échap pour masquer.
- Colonne de gauche : la section Catégories se replie / se déplie (clic sur son titre).
- « ‹ Masquer la liste » (en haut du panneau de lecture) cache la liste des mails pour agrandir
  la lecture ; « › Afficher la liste » la fait revenir. L'état est mémorisé.
- Annuaire : cliquer sur un contact affiche ses derniers mails sous les statistiques (clic = ouvrir).
- Images reçues : taille imposée par l'expéditeur (`width`/`height`, `%`, `max-width`),
  proportions conservées, alignement (`text-align`, `align`, `<center>`, `margin:auto`, `float`),
  jamais plus large que le panneau ; image cliquable si elle est dans un lien.
- Liens de téléchargement (fichiers .pdf/.zip/.docx…, Google Drive, WeTransfer, Dropbox, OneDrive,
  texte « Télécharger »…) : soulignés en bleu. Les adresses web écrites en texte deviennent cliquables.
- Pastilles « documents » en haut du mail : une par pièce jointe et par lien de téléchargement ;
  un clic fait défiler jusqu'au document (pièces jointes regroupées en bas du mail) et le fait clignoter.
- Groupes : les mails reçus via une adresse intermédiaire (ex. `theotter@artfx.fr`, liste de
  diffusion, Google Groups via `List-Id`) peuvent être colorés : liseré + teinte + nom du groupe
  dans la liste. Section « Groupes » (colonne de gauche, adresses détectées à un clic),
  « Reçu via … › Colorer » dans le mail, ou Réglages › Couleurs des groupes.
  Palette de 10 couleurs + couleur libre ; le changement s'applique immédiatement à tous les mails.
- Spam : bouton « Spam » dans le mail ouvert (ou clic droit sur un mail de la liste ›
  « Signaler comme spam »). Dans le dossier Spam : « Pas un spam » le remet en boîte de réception.
- Signature : reprise des réglages Gmail (« Envoyer en tant que »), ou à défaut du bloc
  `gmail_signature` de tes derniers mails envoyés (ex. signature ARTFX). Insérée exactement comme
  Gmail (séparateur « -- », au-dessus du message cité dans une réponse), aperçu fidèle dans l'éditeur
  (tailles, gras, couleurs, liens, logos). Menu « Signature » : choisir ou « Aucune signature ».
- Catégories = libellés Gmail. « + » pour en créer, « Ranger dans… » pour y déplacer un mail
  (il quitte la boîte de réception, comme un dossier).
- Favoris = étoile Gmail. Les favoris restent listés en bas de la colonne de gauche
  tant qu'ils ne sont pas retirés des favoris. Tout est synchronisé avec Gmail.

## Calendrier (inspiré de Notion Calendar, branché sur Google Agenda)
- Colonne de gauche : « Nouvel événement », mini-calendrier (clic = aller au jour), agendas
  Google cochables / masquables (clic droit : « Afficher uniquement cet agenda »). Les agendas
  masqués dans Google Agenda le sont aussi au premier affichage. Choix mémorisés.
- Vues : Jour, 3 jours (ou 1 à 7 jours au clavier), Semaine, Mois. Grille heure par heure,
  ligne rouge « maintenant », week-ends assombris, bandeau « Journée » pour les événements
  sans horaire, événements simultanés côte à côte.
- Créer : glisser sur une plage libre (pas de 15 min), clic simple (1 h), double-clic dans le
  bandeau Journée (journée entière) ou dans la vue Mois.
- Clic sur un événement : panneau de droite **en lecture** (titre, date et heures, agenda, lieu,
  Google Meet, invités — clic = fiche contact —, description). Rien n'est modifiable avant le
  bouton **« ✏ Modifier »** (ou E) ; Annuler / Échap revient à l'affichage, l'enregistrement aussi.
- Modifier à la souris (seulement pendant la modification de cet événement) : le glisser (autre
  heure / autre jour), tirer son bord bas (durée).
- Le panneau s'ouvre et se referme avec l'animation des colonnes de Google Chat (rebond élastique).
- Panneau en modification : titre, début / fin, journée entière, durées rapides, agenda, couleur
  (11 couleurs Google), invités avec suggestions de l'annuaire, lieu, Google Meet (ajout / rejoindre),
  description, enregistrer (Ctrl+Entrée), supprimer (avec confirmation), ouvrir dans Google Agenda.
  Les invités reçoivent les invitations / mises à jour de Google.
- Raccourcis : T aujourd'hui · J / K (ou → / ←) suivant / précédent · D / W / M jour / semaine / mois ·
  1 à 7 nombre de jours · C nouvel événement · Échap fermer.
- Tableau de bord : la carte « Aujourd'hui » liste les événements du jour.

## Messages (Google Chat)
- Deux blocs masquables (rebond élastique) : **Groupes** (espaces, discussions de groupe,
  applications) et **Personnes**, grand annuaire alphabétique de tous tes messages privés
  (photo, nom, adresse, recherche par nom ou adresse) avec **Récents** en tête.
- Photos de profil (API People) pour les personnes et les auteurs des messages ; discussions de
  groupe : photos de deux membres ; espaces : image au choix (clic droit › « Choisir une image… »,
  copiée dans `chat_icons`), sinon initiales.
- **Rendre muet** une conversation (clic droit ou cloche dans l'en-tête) : nom estompé, cloche
  barrée, plus comptée dans les non lus ni dans la pastille de la barre.
- Trait de séparation entre les sections.
- **Conversations hors ligne** : copie chiffrée gardée sur l'ordinateur (`cache\chat.wtc`,
  AES-256-GCM ; clé dans `secrets\chat_cache.key`, protégée par Windows — DPAPI : lisible
  seulement par ta session). Elle contient le nom de **toutes** les conversations et de tous les
  groupes (membres, personnes, sections compris) et les 200 derniers messages des 50 conversations
  les plus récentes, mises à jour automatiquement en arrière-plan. Relue dès le lancement dans un
  fil à part, avant même la connexion à Google : la liste, les noms et les messages s'affichent
  aussitôt, et ouvrir une conversation ne demande plus que ses nouveaux messages. Liée au compte
  Google (effacée à la déconnexion ou si un autre compte se connecte) ; Réglages › Données › « Vider ».
- **Épingler** un message (📌 dans la barre d'actions au survol) : il apparaît dans le panneau
  « Messages épinglés » du tableau de bord ; « · épinglé » s'affiche à côté de l'heure.
- **Noms des conversations vérifiés en arrière-plan** : toutes les conversations privées et les
  groupes sans titre (pas seulement les plus récents) voient leurs membres et leurs personnes
  relus par lots toutes les 20 s, jusqu'à afficher un vrai nom au lieu de « Message privé » /
  « Utilisateur ». Échec (erreur, quota Google) : nouvel essai après 1 min, 5 min, 30 min, puis
  toutes les 3 h. Petit indicateur à côté du titre « Personnes » pendant l'identification.
- Recherche de personne : la conversation existante s'affiche ; sinon les contacts de l'annuaire
  (ou l'adresse tapée) proposent « Écrire › », qui crée la conversation privée. Entrée ouvre le
  premier résultat.
- Glisser-déposer un groupe d'une section à l'autre (synchronisé avec Google Chat).
- Créer un groupe (bouton + du bloc Groupes) : espace nommé ou discussion de groupe, membres
  choisis dans l'annuaire.
- Émojis : catalogue Unicode par catégories, recherche (anglais + mots français courants),
  récents ; codes « :nom: » remplacés pendant la frappe avec suggestions (Tab pour insérer) ;
  « ＋ » au survol d'un message pour réagir avec n'importe quel émoji.
- Raccourcis : Ctrl+B gras, Ctrl+I italique, Ctrl+Maj+X barré, Ctrl+Maj+C code, Ctrl+K lien ;
  le Markdown (`**gras**`, `~~barré~~`, `[texte](url)`) est converti en syntaxe Google Chat.
- Tes **sections Google Chat** (dossiers personnels et sections par défaut) sont reprises avec
  leurs noms et leur ordre ; les conversations non rangées vont dans Messages privés / Espaces.
  **Favoris** locaux (clic droit ou étoile) au-dessus. Recherche, aperçu du dernier message,
  pastille terracotta des non lus.
- Discussion : messages groupés par expéditeur et par jour, mise en forme Google Chat
  (`*gras*`, `_italique_`, `~barré~`, `` `code` ``, blocs ```` ``` ````, listes, liens `<url|texte>`,
  mentions `@Nom`), messages cités, « (modifié) ».
- Survol d'un message : réactions rapides 👍 ❤️ 😂 🎉 👀 (clic sur une réaction = l'ajouter / la retirer),
  répondre dans le fil, modifier / supprimer ses messages, copier le texte.
- Fils (espaces à fils) : « N réponses · dernière à hh:mm » ouvre le fil dans un panneau à droite.
- Pièces jointes : images affichées dans la discussion ; fichiers téléchargés puis ouverts avec
  l'application Windows par défaut ; fichiers Drive ouverts dans le navigateur ; « Ouvrir dans
  Google Chat » en dernier recours. Envoi : bouton 📎 ou glisser-déposer sur la fenêtre.
- Rédaction : Entrée envoie, Maj+Entrée va à la ligne, boutons B / I / S / code (entourent la
  sélection), émojis.
- Nouvelle conversation : autocomplétion depuis l'annuaire des mails ; retrouve ou crée le message privé.
- Liens avec l'onglet Mail : « Message Google Chat » sur une fiche de l'annuaire et dans le menu de
  l'expéditeur d'un mail ; « Écrire un mail » depuis un message privé.
- Colonne des conversations masquable (« ) avec rebond élastique ; Alt+↑ / Alt+↓ : conversation
  précédente / suivante ; Échap ferme le fil.
- Économie de quota (comme Gmail) : la liste des espaces (1 requête toutes les 8 s onglet ouvert,
  45 s sinon) sert de signal ; seuls les messages postérieurs au dernier connu sont relus ;
  la fenêtre récente est rafraîchie au plus une fois par minute ; noms des membres en cache
  (`chat_people.json`).

## Barre de navigation
- Logo en haut : monochrome (équerres, cadre arrondi, grille 3 × 3), dessiné dans la couleur du texte
  du thème. Icône de la fenêtre et de la barre des tâches (logo sombre sur fond clair) : `assets/logo.png` (256 × 256),
  version HD `assets/logo-hd.png` (1024 × 1024).
  Un clic sur le logo le fait tourner ; chaque clic accélère et le grossit (élastique), puis il
  freine doucement et revient à sa place.
  Pendant qu'il tourne, les panneaux de l'onglet tremblent, chacun à son rythme, d'autant plus fort
  que la roue va vite (amplitude et fréquence liées à sa vitesse), puis se calment avec elle.
  Icône du fichier .exe : `assets/logo.ico` (16 à 256 px), intégrée à la compilation par `build.rs`.
- Pastilles terracotta : nombre de non lus pour Mail et Messages.
- Aperçu au survol : rester 0,3 s sur l'icône Mail ou Messages quand des notifications attendent
  ouvre un petit panneau à droite (expéditeur/conversation, objet ou dernier message, heure),
  qui jaillit de l'icône comme une goutte avec un rebond élastique ;
  un clic mène au mail ou à la conversation. Il reste ouvert tant que la souris est dessus ou
  à proximité, et se ferme en s'éloignant, avec Échap ou en cliquant sur un autre onglet.
- Loupe façon Dock macOS : l'icône survolée grossit (et ses voisines un peu), en douceur.
- Animation au clic propre à chaque icône : tuiles qui s'écartent (tableau de bord), saut et page
  qui se tourne (calendrier), enveloppe qui s'ouvre (mail), lignes réécrites (notes),
  engrenage qui tourne (réglages).

## Animations
Toutes les sélections (onglets de la barre, dossiers, mails, favoris, annuaire, alignement,
sélecteurs) utilisent une forme « liquide » qui s'écoule d'un élément à l'autre.
Réglages › Animations : vitesse (durée, 100–1500 ms), opacité, courbe de mouvement
(Linéaire, Ease in, Ease out, **Ease in-out** par défaut, Élastique) avec aperçu en direct.
Préférences enregistrées dans `data\config\prefs.json`.

## Émojis
Affichés en couleur, style 3D proche d'iOS : jeu **Noto Emoji 3D** de Google (Chat, Mail,
réactions, sélecteur). Chaque image est téléchargée une fois puis gardée dans
`data\cache\emoji` ; hors ligne, le dessin monochrome sert de repli.
Un message composé uniquement d'émojis s'affiche en grand.

## Notifications
Un seul système : un bandeau animé en haut au centre de l'écran, par-dessus toutes les fenêtres,
façon « succès débloqué » Xbox One (pastille aux couleurs de la sélection de la barre de gauche,
capsule aux couleurs des panneaux, titre en gras, barre de durée : 10 s pour un mail, 5 s pour le
reste). Un clic mène au bon endroit.
- ✉️ nouveau mail reçu dans la boîte de réception (expéditeur, objet, et le début du mail sur
  3 lignes dans un bandeau plus haut) ;
- 💬 nouveau message Google Chat (sauf conversations muettes ou déjà ouvertes à l'écran) ;
- 📄 document partagé avec moi sur Google Drive (vérifié toutes les 5 minutes) ;
- 📅 rendez-vous qui commence dans 10 minutes (délai réglable).

Les bandeaux passent l'un après l'autre ; une petite pastille sur le pictogramme indique combien
de notifications du même type attendent. Un petit son les accompagne (carillon pour un mail ou un
rendez-vous, « blip » pour un message ou un document). Chaque type, et le son, s'active ou se coupe
dans Réglages › Notifications. Il n'y a plus de notifications Windows.

Sous Windows, l'app s'affiche avec DirectX 12 (mode « DirectComposition ») pour que le bandeau soit
vraiment transparent, avec son ombre portée. Sans DirectX 12, elle passe sur OpenGL et le bandeau
est découpé à sa forme (sans ombre). `WORKTABLE_RENDERER=glow` ou `=wgpu` force un moteur.

## Apparence commune
- **Barre de titre intégrée** : plus de barre Windows ; on déplace la fenêtre par la bande du haut
  (double-clic : agrandir / restaurer), boutons réduire, agrandir et fermer en haut à droite,
  redimensionnement par les bords.
- **Heure** en haut à droite, sur la ligne du titre de l'onglet et dans sa police, dans tous les
  onglets. Un clic la fait disparaître avec un effet élastique, un clic au même endroit la fait
  revenir ; masquée, elle réapparaît en transparence quand la souris s'approche (aussi réglable
  dans Réglages › Apparence).
- **Titres d'onglets** identiques partout : sur-titre en capitales + grand titre (police « Titres »),
  avec une info à côté (non lus, semaine, nombre de notes). Le contenu de chaque onglet commence
  à la même distance de la barre verticale.
- **Émojis** : partout dans l'app (titres, listes, mails, notes, agenda, champs de saisie), les
  émojis s'affichent avec les mêmes images couleur que dans Google Chat. Le sélecteur 🙂 et les
  codes « :nom: » (Tab pour la première suggestion) marchent dans Messages, les notes et le
  composeur de mail ; les codes marchent aussi dans les tâches.
- **Thèmes** (Réglages › Apparence) : Monochrome, GitHub Dark, Tokyo Night, Dracula, Unreal Engine 5.8, Wii (clair), Windows XP (clair), GTA VI,
  Discord, Noire, PS2, Game Boy. Appliqué immédiatement ; l'accent du thème remplace le terracotta.
- **Polices** (Réglages › Polices) : une police et une taille par groupe — Titres, Interface,
  Texte (mails, notes, messages), Code. Choix : Inter, Ubuntu, IBM Plex Sans, Lato, IBM Plex Serif,
  Hack, IBM Plex Mono. Appliqué immédiatement dans tous les onglets.
- **Images en grand** : un clic sur une image l'ouvre dans la même visionneuse partout — Chat,
  Mail (images du corps sans lien, aperçu des pièces jointes), Notes, Drive (aperçu du panneau
  Détails, rechargé en haute définition). Molette ou +/− pour zoomer, glisser pour déplacer,
  double-clic 100 % / 200 %, 0 pour réinitialiser, Échap ou clic à côté pour fermer ; « Ouvrir »
  lance l'application par défaut (ou Drive dans le navigateur). Notes protégées : pas d'« Ouvrir ».

## Licences
- Police Inter : SIL Open Font License 1.1 — `assets/fonts/LICENSE-Inter.txt`.
- Polices IBM Plex Sans / Serif / Mono : SIL OFL 1.1 — `assets/fonts/LICENSE-IBMPlex.txt`.
- Police Lato : SIL OFL 1.1 — `assets/fonts/LICENSE-Lato.txt`.
- Émojis Noto Emoji (3D et 2D), © Google LLC : SIL Open Font License 1.1 — https://github.com/googlefonts/noto-emoji

## Structure
- `src/app.rs` — état global, mise en page plein écran, routage des événements
- `src/sidebar.rs` — barre de navigation en pilule + bouton Réglages (flottants)
- `src/theme.rs` — DA monochrome
- `src/widgets.rs` — cartes, panneaux, champs, états vides
- `src/i18n.rs` — langue de l'interface : chaque texte est écrit dans les deux langues là où il
  est affiché, `t!("Réglages", "Settings")` (texte fixe) ou `tf!("{n} non lus", "{n} unread")` (texte formaté)
- `src/google/` — OAuth (`auth.rs`), API Gmail (`gmail.rs`), API Google Chat + People (`chat.rs`), rédaction MIME (`compose.rs`), compte/session (`mod.rs`)
- `src/notes_store.rs` — notes locales : Markdown, pièces jointes, chiffrement
- `src/tabs/notes/` — éditeur par blocs (`editor.rs`), Markdown en ligne (`inline.rs`), liens (`links.rs`)
- `src/tabs/` — un fichier par onglet ; `mail.rs` (état/actions) + `mail/view.rs` (affichage)
