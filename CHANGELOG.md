# Journal des changements

Format : une section par version publiée (la plus récente en haut). La section « Non publié »
regroupe ce qui partira dans la prochaine version. Voir « Publier une version » dans le README.

## Non publié

### Ajouté
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
