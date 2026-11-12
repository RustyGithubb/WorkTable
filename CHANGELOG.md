# Journal des changements

Format : une section par version publiée (la plus récente en haut). La section « Non publié »
regroupe ce qui partira dans la prochaine version. Voir « Publier une version » dans le README.

## Non publié

### Ajouté
- Réglages › Compte NAS : synchronisation des données avec le serveur NAS (Mac mini) pour les
  retrouver sur chaque poste. Premier login avec le code d'invitation de la console du NAS (code de
  récupération affiché une fois), puis identifiant + mot de passe. Notes, tâches, Blueprints,
  réglages, contacts, onglets perso et caches (messages, contacts, plannings) sont chiffrés sur le
  poste avant l'envoi. Les changements des autres postes s'appliquent au démarrage (bouton
  « Redémarrer et appliquer ») ; un même fichier JSON modifié sur deux postes est fusionné, sinon
  la version d'ici est gardée en copie « conflit ». Postes connectés et « Révoquer ».

### Corrigé
- Tâches, rappels et bases de surlignage : identifiants uniques (date + hasard) au lieu de 1, 2,
  3…, pour que deux postes synchronisés ne créent pas le même.

## 0.4.51

### Ajouté
- Messages et Mail : clic droit sur une image jointe › « Télécharger l'image… » (fenêtre
  « Enregistrer sous », dossier Téléchargements par défaut). Dans Mail : pièces jointes images et
  image de la fenêtre d'aperçu.
- Mail › Annuaire : clic droit sur un contact › « Retirer de l'annuaire ».

### Corrigé
- Messages : Maj+Entrée revient à la ligne (il envoyait le message).
- Mail : Maj+Entrée revient à la ligne dans le paragraphe (Entrée crée toujours un nouveau
  paragraphe).

## 0.4.4

### Ajouté
- Messages : recherche des personnes dans l'annuaire de l'école. Dans « Rechercher une personne »
  et « Nouvelle conversation », un nom suffit : les personnes trouvées (avec qui aucune conversation
  n'existe encore) s'affichent, un clic ou « Ouvrir » démarre la conversation. Plus besoin de taper
  l'adresse complète.

## 0.4.3

### Ajouté
- Blueprint, action « Ranger un mail dans un groupe » : nouvelle option « Retirer de la boîte de
  réception ».
- Mail : liste des adresses en spam (Réglages › Adresses en spam, bouton Retirer). « Spam » ajoute
  l'adresse de l'expéditeur, « Pas un spam » la retire. Les futurs mails de ces adresses partent
  directement dans les spams de Gmail, sans notification.
- Recherche (Ctrl+Espace) : photo de profil Google Chat des personnes. Les photos sont gardées en copie
  locale chiffrée et ne sont téléchargées qu'une seule fois.
- Recherche : pastille du type de fichier Drive (Docs, Sheets, Slides, PDF, dossier…), comme dans
  l'onglet Fichiers.

### Modifié
- Blueprint, event « Mail reçu » : les mots-clés sont aussi cherchés dans le nom de l'expéditeur, son
  adresse et sa fiche contact. « @artfx » ne correspond qu'au domaine de l'adresse de l'expéditeur.
- Colonne de gauche de même largeur (260 px) dans tous les onglets (Mail, Calendrier, Notes, Fichiers,
  Plannings, Blueprint, Messages).

### Corrigé
- Notifications : le bandeau apparaît bien quand l'app est cachée en arrière-plan (il s'ouvrait hors de
  l'écran).
- Recherche : une même personne n'apparaît plus deux fois (fiche contact et annuaire mail).
- Recherche : la liste ne remonte plus toute seule quand on la fait défiler ; elle ne revient en haut
  qu'à la saisie de texte.

## 0.4.2

### Ajouté
- Recherche (Ctrl+Espace) : cours HyperPlanning (matière, enseignant, salle, groupe). Un clic ouvre
  le planning au jour de la séance, séance sélectionnée.
- Recherche : mot-clé de catégorie en tête (`hpp`, `cal`, `msg`, `mail`, `drive`, `sheet`, `doc`,
  `note` et leurs variantes) pour ne chercher que dans cette catégorie.
- Recherche : pastilles. Un mot reconnu (catégorie, groupe / salle / enseignant HPP, action comme
  « nouvelle note », « rdv », « réglages ») devient une pastille ; un clic sur un élément HPP proposé
  en crée une. Plusieurs pastilles HPP : « Ouvrir ensemble » (plannings combinés) et cours communs.
- Recherche : avec une pastille de groupe, les cours et éléments rattachés passent en premier ; un
  clic sur un cours liste toutes ses séances (à venir puis passées).
- Messages : les demandes de chat reçues (conversation d'une personne qui ne t'a jamais écrit)
  apparaissent dans « Demandes de chat », sous le nom de leur auteur, avec un bandeau pour y
  répondre dans Google Chat (Google ne permet pas d'accepter depuis une autre application) et un
  bouton « Actualiser ».

### Modifié
- L'onglet Plannings s'appelle **HyperPlanning** (HPP en abrégé).
- HyperPlanning : ouvert depuis la recherche, un planning s'affiche sur sa prochaine séance et
  l'élément est montré dans le répertoire (type, catégorie dépliée, ligne surlignée).

### Corrigé
- HyperPlanning : la grille ne saute plus quand on change de planning dans le répertoire.

## 0.4.1

### Ajouté
- Plannings : plusieurs plannings affichés ensemble (Ctrl+clic dans la liste pour en ajouter ou en
  retirer). Chaque planning a sa couleur, rappelée par une pastille en haut (clic pour le retirer).
  Un planning de salle s'affiche toujours seul.

### Corrigé
- CI : mise en forme du code (`cargo fmt`).

## 0.4.0

### Ajouté
- Onglet **Plannings** : emplois du temps HyperPlanning de l'école (enseignants, salles,
  promotions, TD / options, regroupements), lus dans une copie locale synchronisée. Même forme et
  mêmes raccourcis que le Calendrier : vues Jour, 3 jours, Semaine, Mois, vue de l'année scolaire,
  colonne de gauche masquable (S).
- Plannings : favoris, choix du type de planning dépliable, bandeau de l'année scolaire ; clic sur
  une séance = détail, clic sur un nom du détail = son planning.
- Plannings : salles libres sur un créneau (capacité minimale, groupes de salles).
- Plannings : demande de réservation d'une salle (cliquer-glisser sur la grille ou « Demander »
  depuis les salles libres), envoyée à l'espace web HyperPlanning.net et validée par un
  administrateur.
- Réglages › HyperPlanning : identifiants, mot de passe protégé par Windows (DPAPI).

## 0.3.55

### Ajouté
- Raccourci global **Ctrl+Maj+K** pour quitter WorkTable (même cachée), réglable dans
  Réglages › Arrière-plan.
- Onglets personnalisés : clic droit sur le logo de la barre → galerie 3 × 3 (créer, renommer,
  émoji ou image, supprimer). Chaque onglet est une galerie de panneaux sur la grille du tableau de
  bord : « + Ajouter un panneau » (bloc-notes, tâches, rappels, planning, quadrillage, mails,
  messages épinglés, fichiers, liens), déplacer, redimensionner, retirer (« × »). Un fichier par
  onglet (`data\custom_tabs`), à partager : un fichier déposé dans ce dossier est importé au
  lancement.
- Bases de jours mis en évidence : chaque quadrillage d'un onglet personnalisé a sa propre base ;
  dans le Calendrier, « Mettre en évidence » permet de choisir les bases (et d'en créer).
- Messages : bouton « 👥 Membres » dans l'en-tête des groupes et bouton « + Ajouter » dans la
  gestion des membres.

### Modifié
- Messages : ouvrir une conversation la relit toujours (nouveaux messages, réactions,
  modifications).

### Corrigé
- Réglages : la saisie d'un nouveau raccourci prend bien la combinaison tapée sous Windows.
- Messages : plus aucune notification (bandeau, son, non-lu) pour mes propres messages, même
  envoyés depuis le téléphone ou le web.

## 0.3.5

### Ajouté
- Mail : lien derrière un mot ou une phrase (🔗 ou Ctrl+K dans l'éditeur).

### Corrigé
- Mail : envoi impossible quand un destinataire avait été choisi dans les suggestions
  (« Nom <adresse>, » : la virgule finale et certains noms étaient refusés).

## 0.3.4

### Ajouté
- Mail : brouillons comme dans Gmail (enregistrement automatique sur l'ordinateur et dans Gmail,
  barre des brouillons pour les reprendre) : un mail en cours d'écriture n'est plus perdu en
  ouvrant autre chose.
- Mail : la rédaction s'ouvre dans une fenêtre au style de la recherche et de la fiche contact
  (fond flouté, carte centrée, Ctrl+Entrée pour envoyer, Échap pour fermer en gardant le brouillon).
- Mail : clic sur un groupe = seulement ses mails, recherchés dans toute la messagerie.
- Messages : les GIF (choisis dans Chat ou envoyés en fichier) s'affichent et s'animent, aussi en
  grand dans la visionneuse.
- Messages : envoi de GIF façon Discord (bouton « GIF » : tendances, recherche GIPHY, favoris).

## 0.3.3

### Ajouté
- Arrière-plan (Windows) : fermer la fenêtre laisse l'app tourner ; raccourci global réglable
  pour la rouvrir ou la cacher (Ctrl+Maj+Espace par défaut), avec une animation élastique ; une
  seule instance (relancer l'app la rouvre) ; bouton « Quitter WorkTable » en haut des Réglages.
- Correcteur d'orthographe (français, anglais) dans tous les champs de saisie : vague rouge sous
  les mots inconnus, corrections au clic droit, dictionnaire personnel.

### Modifié
- Mise à jour : la nouvelle version attend la fermeture de l'ancienne avant de démarrer.

## 0.3.2

### Ajouté
- Tableau de bord : panneau « Quadrillage » (un carré par jour, année scolaire française par
  défaut, dates réglables) ; survol = date, place et événements mis en évidence ; clic sur un
  jour = ce jour dans le calendrier, sans changer sa vue.
- Calendrier : « Mettre en évidence » un événement (son jour est coloré dans le quadrillage).
- Vacances : déclarées depuis le calendrier (clic droit sur un jour ou sa semaine) ou dans
  Réglages › Quadrillage ; affichées comme les week-ends.
- Tableau de bord : panneaux déplaçables (effet aimant) et redimensionnables, rangées toujours
  remplies à 100 % ; disposition par défaut dans les Réglages.
- Réglages : nombre de fichiers du panneau « Fichiers reçus ».
- Messages : création de groupe façon palette (`Ctrl+Espace`) ; renommer, gérer les membres et
  supprimer un groupe.
- Blueprint : mots-clés des events en pilules, recherche du **mot entier** (sans tenir compte des
  majuscules ni des accents).
- Blueprint : annuler / rétablir (`Ctrl+Z`, `Ctrl+Y`), copier / couper / coller des nodes, aussi
  d'un graph à l'autre.
- Blueprint : variables gardées d'une exécution à l'autre (définir, lire, ajouter), « Une fois par
  période », « Attendre », « Calcul », « Comparer des nombres », « Maintenant ».
- Blueprint : rattrapage au lancement des mails non lus reçus app fermée (moins de 24 h) ; journal
  des exécutions réelles et attentes en cours conservés d'un lancement à l'autre ; contenu complet
  des mails (plus seulement l'extrait).
- Blueprint : export et import de graphs (`.json`).
- Prise de notes : note « Guide de WorkTable (README) », le README du projet en lecture seule.
- CI : mise en forme, clippy (avertissements = erreurs), tests et version minimale de Rust à chaque
  push et pull request.

### Modifié
- Boutons « + Nouveau » uniformisés (mail, message, calendrier, notes, Blueprint).
- Blueprint : les champs inutiles selon le choix sont masqués (« Minutes avant », « Dans » /
  « Heure », « Valeur ») ; noms des nodes et largeurs uniformisés.
- Les images des notes sont lues et décodées en arrière-plan (plus de blocage à l'ouverture).
- Les réglages sont enregistrés une fois l'édition finie, plus à chaque image pendant le glisser
  d'un curseur.
- Les versions ne sont plus construites que pour Windows x64.
- Version minimale de Rust : 1.95 (`rust-version`).
- Dépendances : une seule version de html5ever (scraper 0.29) et de la pile de hachage (sha2 0.10).

### Sécurité
- Mise à jour : l'empreinte SHA-256 est désormais obligatoire ; sans elle, rien n'est installé.
- `.gitignore` : tout le dossier `data/` (données personnelles en développement) est ignoré.

## 0.3.1
- Onglet Blueprint : graphs d'automatisation (events mail, message, rendez-vous, rappel ; actions
  notification, tâche, note, rappel, rendez-vous, épingler, favoris, corbeille, groupe, catégorie),
  simulation animée, exécution réelle des graphs actifs, type Objet et « Break data ».
- Heure du titre cliquable dans tous les onglets.

## 0.1.1 et versions précédentes
- Premières versions publiées : tableau de bord, calendrier, mail, messages, notes, fichiers ;
  installeur Windows et mise à jour depuis GitHub.
