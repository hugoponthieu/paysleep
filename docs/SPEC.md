# Paysleep : spécification MVP

## Objectif

Application web Rust mono-utilisateur pour déposer, classer, consulter et télécharger des bulletins de paie PDF. Elle est destinée à un déploiement privé dans Kubernetes.

## Parcours principal

Depuis une page unique, l'utilisateur renseigne l'entreprise, le mois et l'année, choisit un PDF puis l'envoie. Après succès, le bulletin apparaît dans une liste triée par année et mois décroissants et peut être téléchargé. L'interface affiche clairement les erreurs et l'état de réplication Proton.

## Règles métier

- Champs obligatoires : entreprise, mois de 1 à 12, année de 2000 à l'année courante + 1, fichier PDF.
- Taille maximale définie par `PAYSLEEP_MAX_UPLOAD_MB`, valeur par défaut : `20`.
- Vérifier l'extension `.pdf` et la signature `%PDF-` ; ne jamais faire confiance au nom fourni.
- Un seul bulletin par triplet entreprise normalisée + année + mois ; un doublon retourne HTTP 409.
- Identifiant UUID généré côté serveur. Chemin stocké : `{annee}/{entreprise}/{mois}/{nom-original}.pdf`, avec segment entreprise et nom de fichier assainis.
- Le fichier original n'est jamais transformé.
- L'enregistrement local est la réussite de référence. Un échec Proton ne supprime pas le fichier local et reste visible comme état `failed`.

## Périmètre

Inclus : upload, liste, téléchargement, filesystem local, réplication Proton optionnelle, health checks, métriques, logs, conteneur et manifests Kubernetes minimaux.

Exclus : authentification, multi-utilisateur, suppression, modification, aperçu PDF, OCR, Google Drive, S3, daemon séparé, base SQL et reprise automatique des réplications. Ne créer que des interfaces permettant d'ajouter un autre backend plus tard, sans implémentation vide.

## Architecture

Workspace Rust avec peu de composants :

- le crate `proton-drive` existant, conservé comme bibliothèque ;
- `paysleep-core` : modèles, validation, erreurs et trait asynchrone `Storage` ;
- `paysleep-app` : cas d'usage, implémentations filesystem/Proton, HTTP, templates et binaire.

Séparer par modules le domaine, les cas d'usage, HTTP, configuration et adaptateurs. Ne créer une abstraction que si elle sert au moins l'implémentation filesystem et Proton.

Le trait `Storage` expose au minimum `put`, `get`, `list` et `health`. Le filesystem est la source de vérité. Les métadonnées sont conservées dans un unique `index.json`, protégé dans le processus et remplacé atomiquement via fichier temporaire + renommage. Chaque entrée contient : id, entreprise, année, mois, nom original, nom stocké, taille, date d'ajout et état Proton (`disabled`, `synced` ou `failed` avec message court).

## HTTP et interface

Utiliser Actix Web, Tokio, Serde, Askama ou Tera, HTMX et `tracing`.

- `GET /` : formulaire, messages et liste.
- `POST /payslips` : upload multipart ; réponse HTML compatible HTMX.
- `GET /payslips/{id}/download` : PDF avec nom original sûr.
- `GET /health/live` : processus vivant.
- `GET /health/ready` : répertoire local accessible en lecture/écriture.
- `GET /metrics` : métriques Prometheus minimales sur requêtes et uploads.

La page doit fonctionner sans JavaScript pour le parcours principal, être utilisable au clavier et responsive. Prévoir états vide, envoi en cours, succès et erreur. Aucun élément interactif inactif.

Direction visuelle antislop : outil d'archivage privé, calme et fonctionnel ; thème clair, neutres avec un unique accent bleu marine, police système, aucune image ni animation décorative. Dials : `ENERGY 1 / RHYTHM 1 / MOTION 1`. La hiérarchie doit venir de la typographie, de l'espacement et des états réels.

## Configuration

Variables d'environnement :

- `PAYSLEEP_BIND` (`0.0.0.0:8080`)
- `PAYSLEEP_DATA_DIR` (`./data`)
- `PAYSLEEP_MAX_UPLOAD_MB` (`20`)
- `PAYSLEEP_PROTON_ENABLED` (`false`)
- `PAYSLEEP_PROTON_BIN` (`proton-drive`)
- `PAYSLEEP_PROTON_PATH` (`/my-files/Paysleep`)
- `RUST_LOG` (`info`)

Refuser le démarrage si Proton est activé avec une configuration invalide. Aucun secret ni contenu de bulletin dans les logs. Fournir `.env.example`.

## Livraison et acceptation

Fournir un README court, un Dockerfile multi-stage non-root et des manifests Kubernetes `Deployment`, `Service` ClusterIP et `PersistentVolumeClaim`, avec probes et montage du volume.

Tests requis : validation métier, doublon, écriture/lecture/listing filesystem, et parcours HTTP upload puis téléchargement. Utiliser des répertoires temporaires.

Le MVP est accepté si le parcours principal fonctionne après redémarrage, si une indisponibilité Proton n'entraîne aucune perte locale, et si les commandes suivantes réussissent :

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
