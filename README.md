# Paysleep

Application web mono-utilisateur pour archiver des bulletins de paie PDF sur un volume local et, en option, les répliquer vers Proton Drive et Google Drive.

## Démarrage

```sh
cp .env.example .env
set -a; . ./.env; set +a
cargo run -p paysleep-app
```

Ouvrir `http://localhost:8080`. Les PDF sont classés dans `PAYSLEEP_DATA_DIR/files/<année>/<entreprise>/<mois>/<nom-original>.pdf`, par exemple `files/2026/acme-france/04/avril.pdf`. Les métadonnées restent dans `PAYSLEEP_DATA_DIR/index.json`, remplacé atomiquement. Le stockage local reste la source de vérité même si Proton échoue.

## Configuration Proton Drive

Paysleep utilise le [CLI officiel Proton Drive](https://proton.me/download/drive/cli/index.html). Proton ne propose pas de fichier de credentials ni de connexion automatisée avec un mot de passe : `auth login` affiche une URL, la connexion se fait dans le navigateur et la session est enregistrée par le CLI. Ne placez donc jamais votre identifiant ou mot de passe Proton dans `.env` ou dans un Secret Kubernetes.

Pour une installation locale :

```sh
mkdir -p ./data/proton-drive
PROTON_DRIVE_CACHE_DIR="$(pwd)/data/proton-drive" \
PROTON_DRIVE_CREDENTIALS_STORE=keychain \
proton-drive auth login
```

L'URL affichée peut être ouverte sur un autre appareil. Utilisez ensuite exactement le même répertoire et le même coffre dans Paysleep :

```env
PAYSLEEP_PROTON_ENABLED=true
PAYSLEEP_PROTON_BIN=/chemin/absolu/vers/proton-drive
PAYSLEEP_PROTON_PATH=/my-files/fiche_de_paie
PAYSLEEP_PROTON_STATE_DIR=/chemin/absolu/vers/data/proton-drive
PAYSLEEP_PROTON_CREDENTIALS_STORE=keychain
```

Le dossier distant donné par `PAYSLEEP_PROTON_PATH` doit déjà exister. Au démarrage, Paysleep vérifie le binaire, la session et l'accès à ce dossier.

`PAYSLEEP_PROTON_CREDENTIALS_STORE` accepte les valeurs prises en charge par le CLI :

- `keychain` : coffre du système, conseillé sur une machine avec GNOME Keyring, KWallet ou le trousseau macOS ;
- `pass` : entrée GPG `ch.proton.drive/drive-sdk-cli/auth-session`, adaptée à un serveur où `pass` et GPG sont déjà configurés ;
- `unsafe_file` : session en clair dans `PAYSLEEP_PROTON_STATE_DIR`, uniquement pour les tests, comme l'indique le CLI officiel.

La crate Rust expose la même configuration sans demander de secret :

```rust
use proton_drive::{Client, CredentialsStore};

let client = Client::new("/usr/local/bin/proton-drive")?
    .with_cache_dir("/var/lib/paysleep/proton-drive")
    .with_credentials_store(CredentialsStore::Pass);
client.login_interactive()?; // à exécuter une seule fois
# Ok::<(), proton_drive::Error>(())
```

### Proton dans Kubernetes

L'image distroless contient le CLI officiel, mais pas de keychain, `pass` ou GPG. Kubernetes utilise donc le coffre `unsafe_file` du CLI et conserve la session dans le même PVC que les données Paysleep. Aucun identifiant ni mot de passe Proton n'est placé dans le manifeste.

Le conteneur reçoit à la fois les variables Paysleep et les variables natives du CLI :

```env
PAYSLEEP_PROTON_ENABLED=false
PAYSLEEP_PROTON_BIN=/usr/local/bin/proton-drive
PAYSLEEP_PROTON_PATH=/my-files/fiche_de_paie
PAYSLEEP_PROTON_STATE_DIR=/var/lib/paysleep/proton-drive
PAYSLEEP_PROTON_CREDENTIALS_STORE=unsafe_file
PROTON_DRIVE_CACHE_DIR=/var/lib/paysleep/proton-drive
PROTON_DRIVE_CREDENTIALS_STORE=unsafe_file
```

La première connexion reste interactive et doit être faite une seule fois :

1. Déployez Paysleep avec `PAYSLEEP_PROTON_ENABLED=false`.
2. Lancez directement le CLI embarqué dans le pod :

   ```sh
   kubectl --namespace '<NAMESPACE>' exec -it deploy/paysleep -- \
     /usr/local/bin/proton-drive auth login
   ```

3. Ouvrez l'URL affichée et terminez la connexion dans le navigateur.
4. Vérifiez l'accès au dossier :

   ```sh
   kubectl --namespace '<NAMESPACE>' exec deploy/paysleep -- \
     /usr/local/bin/proton-drive filesystem list -j /my-files/fiche_de_paie
   ```

5. Passez `PAYSLEEP_PROTON_ENABLED=true` et redémarrez le Deployment.

Le fichier de session est en clair dans le PVC, avec les permissions du processus `10001`. Le volume doit donc être privé, chiffré au repos par la couche de stockage et inaccessible aux autres workloads. Proton peut renouveler la session : ne montez pas ce dossier en lecture seule.

Avec Docker Compose, initialisez le même volume avant le premier démarrage :

```sh
docker compose run --rm \
  --entrypoint /usr/local/bin/proton-drive \
  paysleep auth login
docker compose up -d
```

## Configuration Google Drive

Pour un compte Google personnel, Paysleep utilise OAuth et le quota de votre propre Drive. Google Workspace et les comptes de service ne sont pas nécessaires.

1. Ouvrez la [console Google Cloud](https://console.cloud.google.com/) et créez ou sélectionnez un projet.
2. Dans **API et services > Bibliothèque**, activez **Google Drive API**.
3. Dans **Google Auth Platform**, configurez l'écran de consentement avec une audience **Externe** et ajoutez votre adresse Google comme utilisateur de test.
4. Dans **Clients**, créez un client OAuth de type **Application de bureau**. Le type **Application Web** ne fonctionne pas pour ce flux local.
5. Téléchargez le JSON et enregistrez-le hors du dépôt sous `client-secret.json`.

Le JSON doit contenir une clé `installed`. Les valeurs ci-dessous sont volontairement masquées :

```json
{
  "installed": {
    "client_id": "<REDACTED>.apps.googleusercontent.com",
    "project_id": "<REDACTED>",
    "auth_uri": "https://accounts.google.com/o/oauth2/auth",
    "token_uri": "https://oauth2.googleapis.com/token",
    "client_secret": "<REDACTED>",
    "redirect_uris": ["http://localhost"]
  }
}
```

Configurez ensuite :

```env
PAYSLEEP_GOOGLE_ENABLED=true
PAYSLEEP_GOOGLE_CLIENT_SECRET_FILE=/chemin/absolu/vers/client-secret.json
PAYSLEEP_GOOGLE_TOKEN_CACHE_FILE=/chemin/absolu/vers/data/google-oauth-token.json
PAYSLEEP_GOOGLE_FOLDER_ID=
```

Au premier démarrage, Paysleep affiche une URL Google et attend la redirection locale. Ouvrez cette URL, sélectionnez votre compte et acceptez l'accès. Le refresh token est ensuite conservé dans `PAYSLEEP_GOOGLE_TOKEN_CACHE_FILE` et renouvelé automatiquement.

Avec `PAYSLEEP_GOOGLE_FOLDER_ID` vide, Paysleep recherche ou crée automatiquement un dossier `Paysleep` à la racine de votre Drive. Lorsqu'un identifiant est fourni, Paysleep utilise le scope `drive` afin d'accéder à ce dossier même s'il a été créé manuellement. Il y crée de vrais sous-dossiers selon la même structure que le stockage local : `<année>/<entreprise>/<mois>/<nom-original>.pdf`. Cette permission donne techniquement accès à l'ensemble du Drive du compte autorisé ; utilisez un compte dédié si vous souhaitez limiter l'exposition.

Le client secret et le cache de tokens contiennent des secrets. Ils sont ignorés par le `.gitignore` du dépôt et ne doivent jamais être copiés dans une image Docker. Si l'application OAuth reste en statut **Test**, Google peut expirer le refresh token après sept jours ; passez-la en **Production** pour un service durable.

### OAuth Google dans Kubernetes

Effectuez d'abord la connexion OAuth localement, puis copiez le cache de tokens vers le PVC. Créez le Secret du client OAuth avec des valeurs remplacées ici par des placeholders :

```sh
kubectl --namespace '<NAMESPACE>' create secret generic paysleep-google \
  --from-file=client-secret.json=./client-secret.json
```

Déployez une première fois avec `PAYSLEEP_GOOGLE_ENABLED=false`, puis copiez le cache vers le PVC :

```sh
kubectl --namespace '<NAMESPACE>' exec -i deploy/paysleep -- \
  sh -c 'umask 077; cat > /var/lib/paysleep/google-oauth-token.json' \
  < ./data/google-oauth-token.json
```

Activez ensuite `PAYSLEEP_GOOGLE_ENABLED=true` et redémarrez le Deployment. Le manifeste monte le client secret en lecture seule et conserve le cache de tokens modifiable dans le PVC.

Pour révoquer l'accès, supprimez l'autorisation Paysleep dans les paramètres de sécurité de votre compte Google et effacez `google-oauth-token.json`.

## Vérification

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Conteneur et Kubernetes

```sh
docker build -t paysleep:latest .
kubectl apply -f k8s/paysleep.yaml
```

Avec Docker Compose :

```sh
docker compose up --build -d
```

Le service écoute sur le port `8080`. Définissez `PAYSLEEP_PORT` pour changer le port publié sur l'hôte. Les PDF, les tokens OAuth et l'état Proton restent dans `./data`.

Compose exécute le service avec `PAYSLEEP_UID` et `PAYSLEEP_GID`, réglés par défaut à `1000:1000`, afin que le bind mount `./data` reste accessible sans `chown`. Adaptez ces deux valeurs dans `.env` si votre utilisateur local possède d'autres identifiants.

Le conteneur s'exécute sans privilèges avec l'UID 10001. Le manifeste crée un `Deployment`, un `Service` ClusterIP et un `PersistentVolumeClaim`, avec probes HTTP et montage persistant.

## Workspace

- `paysleep-core` : modèle métier, validation et génération des clés de stockage.
- `paysleep-application` : cas d'usage et ports de persistance.
- `paysleep-storage-filesystem` : stockage local des PDF et index JSON.
- `paysleep-storage-google-drive` : adaptateur OAuth Google Drive.
- `paysleep-storage-proton-drive` : adaptateur Paysleep pour Proton Drive.
- `paysleep-web` : routes Actix, rendu Tera/HTMX et métriques.
- `paysleep-app` : configuration Clap et assemblage du serveur.
- `proton-drive` : wrapper générique du CLI officiel Proton Drive.
