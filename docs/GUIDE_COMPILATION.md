# GIT DRENN — Guide complet de compilation et d'installation (Windows 10 / 11)

> **Honnêteté d'abord.** Ce guide a été rédigé et vérifié côté code sous Linux (compilation, 40 tests, scénarios réels avec Git).
> La compilation sous Windows, les scripts PowerShell (`scripts\*.ps1`), le workflow GitHub Actions et le moteur IA réel
> (llama.cpp + modèle) **n'ont pas pu être exécutés** dans l'environnement où le projet a été écrit. Les étapes ci-dessous sont
> les procédures standard ; la section « Dépannage » (§10) couvre les accrocs probables. Notez ce qui diffère chez vous.

---

## 0. Quelle méthode choisir ?

| Situation | Méthode | Installation sur votre PC |
|---|---|---|
| PC modeste (4 Go de RAM), ou vous voulez éviter 5 Go d'outils | **A. GitHub Actions** (compilation dans le cloud) | Aucune (navigateur seulement) |
| Vous compilez régulièrement | **B. Compilation locale (MSVC)** | Git + Rust + Build Tools C++ (≈ 5 à 8 Go) |
| Vous préférez des outils plus légers | **C. Compilation locale (GNU / MSYS2)** — avancé | Git + Rust + MSYS2 |

Dans tous les cas, **le programme obtenu (`drenn.exe`, environ 1 Mo) est autonome** et tourne sur un PC de 4 Go de RAM : seule la
*compilation* est gourmande, pas l'utilisation.

---

## 1. Prérequis communs

- Windows 10 (version 1803 ou plus récente) ou Windows 11. `curl.exe` y est fourni (utilisé pour l'API GitHub).
- **Git pour Windows 2.31 ou plus** (2.23 minimum ; les versions récentes sont recommandées). Vérifier : `git --version`.
- Une identité Git (une seule fois) :
  ```powershell
  git config --global user.name  "Votre Nom"
  git config --global user.email "vous@exemple.com"
  git config --global core.longpaths true
  ```
- Espace disque : 1 Go libre pour le code + `target\` ; 5 à 8 Go pour les outils de la méthode B.
- Travaillez dans un dossier **hors OneDrive** (ex. `C:\dev\git-drenn`) : la synchronisation verrouille des fichiers pendant la compilation.

---

## 2. Méthode A — Compiler sans rien installer (GitHub Actions)

Principe : GitHub compile le projet sur une machine Windows et vous remet `drenn.exe`.

1. Créez un compte sur https://github.com puis un dépôt **privé** vide, par exemple `git-drenn`.
2. Décompressez `git-drenn-v0.4.1.zip`. Le dossier `git-drenn` contient `.github\workflows\windows.yml` (**attention : ce dossier est caché
   dans l'Explorateur — activez « Éléments masqués »**).
3. Envoyez le code. Le plus simple, sans ligne de commande Git : sur la page du dépôt, **Add file → Upload files**, et glissez le contenu du dossier
   (dossiers cachés inclus : `.github`, `.cargo`). Sinon, depuis PowerShell dans le dossier :
   ```powershell
   git init -b main
   git add .
   git commit -m "GIT DRENN 0.4.1"
   git remote add origin https://github.com/VOTRE_COMPTE/git-drenn.git
   git push -u origin main
   ```
4. Onglet **Actions** du dépôt : le workflow « Windows » démarre seul et exécute les **tests sur Windows**. C'est la première vraie validation.
5. Pour obtenir l'exécutable : **Actions → Windows → Run workflow** (bouton à droite). À la fin (≈ 5 à 10 min), ouvrez l'exécution et téléchargez
   l'artefact **git-drenn-windows** (zip contenant `drenn.exe`). Il s'affiche aussi l'empreinte SHA-256.
6. Passez au §6 (installation).

Aucune *release* n'est créée automatiquement : une version ne se publie que sur votre demande explicite.

> Quand GIT DRENN sera installé, vous pourrez lui demander : « publie mon projet git-drenn sur GitHub » — c'est un bon test grandeur nature.

---

## 3. Méthode B — Compilation locale (MSVC)

### 3.1 Installer les outils (PowerShell, une fois)

```powershell
winget install Git.Git
winget install Rustlang.Rustup
winget install Microsoft.VisualStudio.2022.BuildTools --override "--wait --passive --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```
`winget` est fourni avec Windows 11 et les Windows 10 à jour (sinon, installez « App Installer » depuis le Microsoft Store, ou téléchargez
chaque outil depuis son site : https://git-scm.com, https://rustup.rs, https://visualstudio.microsoft.com/visual-cpp-build-tools/ — cochez
**« Développement Desktop en C++ »**).

Fermez puis **rouvrez PowerShell**, puis vérifiez :
```powershell
git --version
rustc --version      # Rust 1.75 ou plus récent
cargo --version
rustup default stable    # si rustup demande de choisir une chaîne : stable-x86_64-pc-windows-msvc
```

### 3.2 Compiler — avec le script

```powershell
cd C:\dev\git-drenn
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
```
Le script vérifie les prérequis, lance les tests, compile (avec l'icône de la mascotte, sinon sans), prépare le dossier `dist\` et un zip
`git-drenn-windows.zip`, et affiche l'empreinte SHA-256. Options : `-SansIcone`, `-SansTests`.

### 3.3 Compiler — à la main

```powershell
cargo test --release                    # 40 tests
cargo build --release                   # sans icône
cargo build --release --features icon   # avec l'icône (nécessite le Windows SDK, inclus dans « includeRecommended »)
```
Résultat : `target\release\drenn.exe`. Le fichier `.cargo\config.toml` lie le runtime C statiquement : **le .exe fonctionne sur un
Windows vierge**, sans « Visual C++ Redistributable ».

Compilation sur 4 Go de RAM : fermez navigateur et autres applications ; la première compilation prend quelques minutes. Si elle échoue par manque de
mémoire, ajoutez `$env:CARGO_BUILD_JOBS=1` avant `cargo build`, ou utilisez la méthode A.

---

## 4. Méthode C — GNU / MSYS2 (avancé)

1. Installez MSYS2 (https://www.msys2.org) puis, dans « MSYS2 MINGW64 » : `pacman -S --needed mingw-w64-x86_64-toolchain`.
2. `rustup toolchain install stable-x86_64-pc-windows-gnu` puis, dans le dossier du projet :
   `rustup override set stable-x86_64-pc-windows-gnu`.
3. Ajoutez `C:\msys64\mingw64\bin` au PATH, puis `cargo build --release` (l'icône utilise `windres`, fourni par la chaîne MinGW).

---

## 5. Compiler sous Linux / macOS (développement et tests)

```bash
sudo apt install cargo git curl      # Debian/Ubuntu ; ou rustup
cargo test && cargo build --release  # binaire : target/release/drenn
```
Ce chemin est celui qui a été effectivement testé (40 tests, scénarios bout en bout).

---

## 6. Installation

```powershell
powershell -ExecutionPolicy Bypass -File dist\install.ps1
```
Le script copie `drenn.exe` dans `%LOCALAPPDATA%\Programs\GitDrenn`, propose (avec votre accord) de l'ajouter au PATH de votre compte,
puis lance `drenn doctor`. Fermez et rouvrez PowerShell.

Manuellement : copiez `drenn.exe` où vous voulez et lancez `.\drenn.exe doctor`.

`drenn doctor` doit afficher : version de Git, curl, l'état du modèle IA (non configuré = normal à ce stade), le jeton GitHub, la configuration
(`%USERPROFILE%\.drenn\config.json`) et les dossiers autorisés.

> **Windows Defender / SmartScreen.** `drenn.exe` n'est pas signé numériquement : Windows peut afficher « Windows a protégé votre PC ».
> Cliquez **Informations complémentaires → Exécuter quand même**, après avoir comparé l'empreinte SHA-256 (`Get-FileHash drenn.exe`) avec celle
> affichée à la compilation. Ne désactivez jamais l'antivirus globalement.

---

## 7. Premier test pas à pas (5 minutes, sans réseau)

```powershell
mkdir C:\dev\drenn-test\Blog ; cd C:\dev\drenn-test
"<?php echo 'bonjour';" | Out-File Blog\index.php -Encoding utf8
"DB_PASS=secret"      | Out-File Blog\.env -Encoding utf8

drenn --dry-run "publie mon projet Blog sur GitHub"     # affiche le plan, n'exécute rien
drenn "initialise le projet Blog"
drenn "enregistre les modifications du projet Blog"      # le .env doit être EXCLU
drenn "complète le gitignore du projet Blog"
drenn "crée une branche nommée essai dans le projet Blog"
drenn "quel est l'état du projet Blog"
drenn history
```
Résultat attendu : le commit ne contient **que** `index.php` ; `.env` est signalé comme exclu ; chaque étape se termine par `[OK]` après vérification.
Si une confirmation est demandée, répondez `o`. Sans terminal interactif, toute confirmation est refusée (par sécurité).

---

### Interface graphique (facultatif)

```powershell
drenn gui
```
Une page s'ouvre dans votre navigateur (adresse du type `http://127.0.0.1:PORT/?t=JETON`, valable uniquement sur votre PC et pour cette session).
Les confirmations apparaissent dans des fenêtres. Si le navigateur ne s'ouvre pas : `drenn gui --no-browser` et copiez l'adresse affichée.
Si le pare-feu Windows pose une question, l'accès local suffit : refusez les réseaux publics et privés.

---

## 8. Jeton GitHub (publication, création de dépôt)

1. GitHub → **Settings → Developer settings → Personal access tokens → Tokens (classic) → Generate new token**.
2. Cochez la portée **`repo`** (le jeton « classique » est le plus simple pour *créer* des dépôts), durée d'expiration courte (30 à 90 jours).
3. Dans PowerShell : `drenn auth login`, collez le jeton. Il est stocké dans `%USERPROFILE%\.drenn\github_token` (lisible par votre compte).
   Alternative : variable d'environnement `GITHUB_TOKEN` (`setx GITHUB_TOKEN "ghp_..."`).
4. `drenn auth status` doit afficher votre identifiant GitHub. Révocation : `drenn auth logout` **et** suppression du jeton sur github.com/settings/tokens.

Test réseau recommandé : `drenn "cherche llama.cpp sur github"`, puis `drenn "clone ggml-org/llama.cpp"` dans un dossier de test.

---

## 9. Moteur IA local (facultatif mais c'est le but du projet)

Sans modèle, GIT DRENN fonctionne déjà avec son analyseur de règles (français/anglais). Pour le moteur IA :

1. **llama.cpp** : page *Releases* de https://github.com/ggml-org/llama.cpp → archive Windows **CPU x64** (le nom contient `win`, `cpu`, `x64`).
   Décompressez ; vous cherchez `llama-cli.exe`.
2. **Modèle** : un fichier **GGUF quantifié** (Q4 recommandé) d'un petit modèle de la famille Qwen. *La disponibilité exacte d'un « Qwen 0.8B »,
   sa licence et son format restent à vérifier (cahier des charges §4.2)* ; prenez le plus petit modèle instruct disponible (< 1 Go).
3. Installation guidée (demande votre accord avant tout téléchargement, vérifie l'espace disque, affiche le SHA-256) :
   ```powershell
   powershell -ExecutionPolicy Bypass -File dist\setup-model.ps1
   ```
   Ou à la main :
   ```powershell
   drenn config set llama_bin  "C:\outils\llama\llama-cli.exe"
   drenn config set model_path "C:\modeles\qwen-q4.gguf"
   drenn doctor
   drenn bench
   ```
4. **Réglages pour 4 Go de RAM** : `drenn config set ctx_size 1024` (ou 2048), `drenn config set threads 2` (ou le nombre de cœurs physiques),
   `drenn config set max_tokens 384`. Fermez les gros programmes pendant l'analyse. Le modèle n'est chargé que le temps d'une demande.
5. `drenn bench` affiche : plans valides, première action correcte, temps moyen. En dessous de 70 % de justesse, GIT DRENN utilise l'analyseur de règles
   en secours ; envisagez alors un modèle plus grand ou un contexte plus large.
6. Si llama.cpp refuse des options (`--json-schema`, `-no-cnv`), GIT DRENN réessaie seul avec moins d'options. Sinon : mettez llama.cpp à jour,
   ou ajustez `llama_extra_args` dans `config.json`.

### Relevé de mesures (phase 6 du cahier des charges)

À remplir sur chaque machine cible : Gestionnaire des tâches → onglet Détails → colonne « Mémoire (jeu de travail privé) ».

| Machine | Windows | RAM | Modèle (taille) | ctx | Pic RAM llama-cli | Temps moyen `bench` | Justesse |
|---|---|---|---|---|---|---|---|
| | | | | | | | |

`drenn.exe` seul (au repos entre deux demandes) doit rester de l'ordre de quelques Mo : à confirmer par mesure.

---

## 10. Dépannage

| Symptôme | Cause probable | Solution |
|---|---|---|
| `cargo` / `rustc` « n'est pas reconnu » | PATH non rechargé | Rouvrez PowerShell ; sinon ajoutez `%USERPROFILE%\.cargo\bin` au PATH |
| `linker 'link.exe' not found` | Build Tools C++ absents | Installez la charge de travail « Développement Desktop en C++ » (§3.1) |
| `rc.exe` / `LNK1181` avec `--features icon` | Windows SDK absent | Compilez sans icône (`-SansIcone`) ou installez le SDK (Visual Studio Installer → Modifier) |
| Erreur de mémoire pendant la compilation | 4 Go de RAM | Fermez les applications, `CARGO_BUILD_JOBS=1`, ou méthode A |
| `Git est introuvable` | Git absent du PATH | `winget install Git.Git` puis rouvrez PowerShell |
| `Git x.y est trop ancien` | Git < 2.23 | Mettez Git à jour |
| Accents illisibles (`Ã©`) | Console non UTF-8 | Utilisez **Windows Terminal** ; sinon `chcp 65001` |
| `accès refusé … en dehors des dossiers autorisés` | Périmètre de sécurité | `drenn config add-root D:\Projets` |
| `Authentication failed` / `could not read Username` | Pas de jeton ou expiré | `drenn auth login` (§8) |
| `requête GitHub impossible` | Pas d'Internet, proxy ou curl absent | Vérifiez la connexion ; proxy : `setx HTTPS_PROXY http://hote:port` et `git config --global http.proxy http://hote:port` |
| `Filename too long` | Chemins > 260 caractères | `git config --global core.longpaths true` |
| `délai dépassé (180 s)` pendant l'analyse IA | Modèle trop lourd / CPU lent | `drenn config set llama_timeout_secs 400`, modèle plus petit, `ctx_size` plus bas |
| `modèle IA indisponible ou invalide … repli sur les règles` | Sortie non JSON / options refusées | Normal et sans danger (repli automatique) ; lancez `drenn bench` pour diagnostiquer |
| Des tests échouent sous Windows | Particularité Windows non vue | `cargo test -- --nocapture`, notez le nom du test et le message ; compilez avec `-SansTests` en attendant |
| Fichiers verrouillés pendant `cargo build` | Antivirus / OneDrive | Compilez hors OneDrive ; excluez `target\` de l'analyse en temps réel si votre politique le permet |
| `.github` / `.cargo` « introuvables » | Dossiers cachés | Explorateur → Affichage → Éléments masqués |

---

## 11. Mise à jour et versions

- Recompiler après modification : `cargo build --release` (ou le script). `drenn --version` affiche la version.
- **Règle du projet :** une nouvelle version de GIT DRENN n'est créée (tag, release) que **sur demande explicite**. Le workflow GitHub Actions
  ne publie aucune release ; il produit seulement des artefacts de test.
- Désinstallation : supprimez `%LOCALAPPDATA%\Programs\GitDrenn`, l'entrée du PATH, et si vous le souhaitez `%USERPROFILE%\.drenn`
  (configuration, journal `history.jsonl`, jeton, modèles).

---

## 12. Aide-mémoire

```powershell
drenn doctor | projects | history | bench | about
drenn config                      # voir la configuration
drenn config set <clé> <valeur>   # ex. model_path, llama_bin, projects_dir, ctx_size, threads
drenn config add-root <dossier>   # autoriser un dossier de travail
drenn auth login | status | logout
drenn --dry-run "…"               # voir le plan sans rien exécuter
drenn --no-ai "…"                 # forcer l'analyseur de règles
```
