<p align="center"><img src="assets/mascot-128.png" alt="Mascotte GIT DRENN" width="128"></p>

# GIT DRENN 0.5.0 — Git Powered by AI

Vous décrivez le travail, l'agent prépare un plan, le moteur Rust l'exécute et **vérifie chaque résultat**.

**Créateur : Mr Tantely Orion.**

```
drenn "clone mastodon dans mon dossier Projets"
drenn "enregistre mes modifications"
drenn "publie mon projet Blog sur GitHub"
drenn                      # session interactive
drenn doctor | projects | history | bench | about
drenn config set model_path C:\\modeles\\qwen.gguf
drenn auth login
```
Options : `--dir <dossier>`, `--dry-run` (plan seul), `--no-ai`, `--auto-medium`.

## Compilation (Windows 10/11)
**Guide complet pas à pas : [docs/GUIDE_COMPILATION.md](docs/GUIDE_COMPILATION.md)** (3 méthodes, dont la compilation dans le cloud via GitHub Actions sans rien installer ; scripts `scripts\build-windows.ps1`, `install.ps1`, `setup-model.ps1`).

Résumé :
1. Installer Rust (https://rustup.rs) et Git pour Windows.
2. `cargo build --release` → `target\release\drenn.exe` (avec l'icône de la mascotte : `cargo build --release --features icon`, non testé sous Windows) (dépendances : serde, serde_json uniquement).
3. `curl.exe` (inclus dans Windows 10/11) sert aux appels à l'API GitHub.

## Modèle IA (facultatif)
**Guide détaillé : [docs/GUIDE_MODELE_IA.md](docs/GUIDE_MODELE_IA.md)** — en deux commandes, sur tous les systèmes : `drenn model engine` puis `drenn model install` (modèle choisi selon votre RAM, ex. Qwen2.5-1.5B-Instruct Q4_K_M pour 4 Go).

Sans modèle, un analyseur de règles FR/EN comprend les demandes courantes. Pour le modèle local :
1. Télécharger `llama-cli.exe` (https://github.com/ggml-org/llama.cpp/releases) et un modèle GGUF quantifié (ex. Qwen petit, Q4).
2. `drenn config set llama_bin <chemin\llama-cli.exe>` puis `drenn config set model_path <chemin\modele.gguf>`.
3. `drenn doctor` vérifie le tout ; `drenn bench` mesure validité, justesse et temps sur 10 demandes types (relevez le pic de RAM dans le Gestionnaire des tâches). Le modèle n'est chargé que pendant l'analyse d'une demande (aucune RAM occupée au repos) ; en cas de réponse invalide, l'agent se replie sur les règles.

## Architecture
`util` (processus avec délai, chemins) · `config` · `security` (périmètre, secrets, risques) · `actions` (22 fonctions prédéfinies, schéma strict) · `planner` (llama.cpp) · `rules` (repli) · `gitengine` (git sans shell) · `github` (API via curl) · `agent` (plan → validation → confirmation → exécution → vérification) · `ui`.

## Sécurité
- Le modèle ne produit que des actions prédéfinies ; paramètres inconnus ou invalides refusés. Aucune commande libre, aucun shell, arguments séparés, `--` avant les URL, `GIT_LITERAL_PATHSPECS`.
- Périmètre : dossier courant + dossier personnel (restreignable via `allowed_roots`) ; `.ssh`, `.aws`, `.gnupg`, `AppData`, `.git`… toujours interdits.
- Secrets : `.env`, clés, jetons détectés par nom et contenu ; exclus de l'index, commit refusé, push bloqué si l'historique à envoyer en contient.
- Confirmations : plan (dès « local »), puis par étape pour tout envoi / création de dépôt ; mot à recopier pour public (`PUBLIC`), force push (`FORCER`), abandon (`ABANDONNER`). Hors terminal interactif = refus.
- `discard_changes` crée une sauvegarde (`refs/drenn/backup/…`, récupérable par `git stash apply`) et ne touche jamais aux fichiers non suivis.
- Le contenu des dépôts n'est jamais envoyé au modèle ; hooks Git désactivés ; aucun script de projet exécuté.
- Jeton GitHub : variable `GITHUB_TOKEN` ou `drenn auth login` (fichier local) ; transmis par environnement/stdin, jamais en argument, masqué des sorties.

## Nouveautés de la v0.6.0
- **Moteur IA persistant** (`llama-server`, modèle gardé en mémoire, déchargé après inactivité) ou serveur local compatible OpenAI (Ollama, LM Studio).
- **`drenn model`** : choix du modèle selon votre RAM, téléchargement vérifié (SHA-256), détection de llama.cpp, sur Windows, macOS, Linux et BSD.
- **Mode cloud facultatif** (désactivé par défaut, consentement à chaque session, jamais le contenu des dépôts).
- **Messages de commit par IA locale** (à partir des noms de fichiers uniquement), **mode apprentissage** (`--teach`) et glossaire (« c'est quoi un commit ? »).
- **Conflits de fusion zone par zone** : « résous les conflits un par un ».
- Multiplateforme : `scripts/install.sh`, CI Windows/macOS/Linux/FreeBSD, [docs/INSTALL_PLATEFORMES.md](docs/INSTALL_PLATEFORMES.md).

## Nouveautés de la v0.5.0 — interface graphique
Boutons **Aide** et **À propos** (créateur : Mr Tantely Orion) dans l'interface ; `drenn about`, `drenn --help` et `drenn --version` indiquent aussi le créateur.
`drenn gui` ouvre une page dans votre navigateur : mascotte, conversation avec l'agent, boutons d'actions fréquentes et **fenêtres de confirmation** (oui/non, mot à recopier pour les actions critiques, choix d'un projet ou d'un dépôt).
- Sans dépendance supplémentaire (serveur HTTP en bibliothèque standard), lié à `127.0.0.1` uniquement.
- Sécurité : jeton aléatoire dans l'URL, contrôle des en-têtes `Host` (anti DNS-rebinding) et `Origin` (anti CSRF), CSP stricte, texte des dépôts affiché sans HTML, mots de confirmation vérifiés côté serveur, une seule opération à la fois.
- `--no-browser` pour ne pas ouvrir le navigateur ; bouton « Quitter » pour arrêter.
- Testé : serveur (accès refusés, confirmation, exécution) et rendu dans un vrai navigateur sous Linux ; non testé sous Windows.

## Nouveautés de la v0.4.1 — prêt pour Windows
- Guide complet de compilation/installation, scripts PowerShell, workflow GitHub Actions (tests + compilation sur Windows), runtime C lié statiquement.
- Durcissement : dossier vide dédié pour désactiver les hooks (plus fiable que `/dev/null` sous Windows), contrôle de la version de Git (≥ 2.23, 2.31 conseillé).

## Nouveautés de la v0.4 — conflits de fusion
- `merge_branch` accepte `keep_conflicts` (« fusionne x et laisse-moi gérer les conflits ») : la fusion reste en cours au lieu d'être annulée.
- **`inspect_conflicts`** : fichiers en conflit, nature (modifié des deux côtés, supprimé d'un côté…) et nombre de zones à arbitrer.
- **`resolve_conflicts`** : « résous les conflits en gardant ma version » (branche courante) ou « la version distante » (branche fusionnée). Confirmation, sauvegarde des fichiers dans `.git/drenn-backup/`, cas suppression/modification gérés, vérification qu'aucun conflit ne reste.
- **`continue_merge`** / **`abort_merge`** : « termine la fusion » (refusé s'il reste des conflits ou des secrets dans l'index) ; « annule la fusion » (retour exact à l'état d'avant).
- Limite : choix par fichier entier (pas d'arbitrage ligne par ligne).

## Nouveautés de la v0.3
- **`push_tag`** : « envoie le tag v1.0.0 sur GitHub » — confirmation, puis vérification côté distant ; une étiquette distante différente n'est jamais remplacée.
- **`stash_changes` / `restore_stash`** : « mets de côté mes modifications » (fichiers non suivis inclus, dossier vérifié propre) et « remets mes modifications mises de côté » (refusé si un travail est en cours ; en cas de conflit, retour à l'état propre, réserve conservée).
- Correctif : les pathspecs littéraux (`GIT_LITERAL_PATHSPECS`) empêchaient `git stash -u` de retirer les fichiers non suivis ; ils restent actifs pour toutes les autres commandes.

## Nouveautés de la v0.2
- **Mascotte** intégrée : `assets/` (png, ico), bannière en console, icône de `drenn.exe` (fonctionnalité `icon`).
- **`update_gitignore`** : « complète le gitignore » ajoute les secrets et les dossiers générés selon le type de projet (Node, PHP, Rust, Python…), signale les fichiers déjà suivis.
- **`undo_last_commit`** : annule le dernier commit en gardant les fichiers préparés ; refusé s'il est déjà sur le dépôt distant ; ancien commit conservé sous `refs/drenn/backup/`.
- **`merge_branch`** : fusion refusée si des modifications sont en attente ; en cas de conflit la fusion est annulée automatiquement et vérifiée.
- **`create_tag`** : étiquette de version annotée (locale).
- **Configuration simplifiée** : `drenn config set <clé> <valeur>` (valeurs validées), `drenn config add-root`.
- **`drenn bench`** : mesure du modèle local ; **`drenn about`**.

## Limites connues
- Non testé sur Windows ni avec un vrai modèle Qwen/llama.cpp (flags `--json-schema`/`-no-cnv` avec repli automatique) : mesures mémoire à faire (phase 6).
- Création de dépôt GitHub / `list_remote_repositories` non testés (jeton requis).
- Détection de secrets heuristique (noms + motifs connus), pas exhaustive. Conflits résolus fichier par fichier uniquement (version courante ou fusionnée) ; les étiquettes ne sont pas envoyées au dépôt distant ; annulation d'un commit déjà publié non prise en charge.
- Aucune version publiée automatiquement : les versions de GIT DRENN se créent uniquement sur demande.
