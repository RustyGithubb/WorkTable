# Journal des changements

Format : une section par version publiée (la plus récente en haut). La section « Non publié »
regroupe ce qui partira dans la prochaine version. Voir « Publier une version » dans le README.

## Non publié

### Ajouté
- Raccourci global **Ctrl+Maj+K** pour quitter WorkTable (même cachée), réglable dans
  Réglages › Arrière-plan.
- Onglets personnalisés : clic droit sur le logo de la barre → galerie 3 × 3 (créer, renommer,
  émoji ou image, supprimer). Chaque onglet est une galerie de panneaux sur la grille du tableau de
  bord : « + Ajouter un panneau » (bloc-notes, tâches, rappels, planning, quadrillage, mails,
  messages épinglés, fichiers, liens), déplacer, redimensionner, retirer (« × »).
- Messages : bouton « 👥 Membres » dans l'en-tête des groupes et bouton « + Ajouter » dans la
  gestion des membres.

### Modifié
- Messages : ouvrir une conversation la relit toujours (nouveaux messages, réactions,
  modifications).

### Corrigé
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
