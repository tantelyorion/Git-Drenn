# GIT DRENN 0.1 — Git Powered by AI

Vous décrivez le travail, l'agent prépare un plan, le moteur Rust l'exécute et **vérifie chaque résultat**.

```
drenn "clone mastodon dans mon dossier Projets"
drenn "enregistre mes modifications"
drenn "publie mon projet Blog sur GitHub"
drenn                      # session interactive
drenn doctor | projects | history | config | auth
```
Options : `--dir <dossier>`, `--dry-run` (plan seul), `--no-ai`, `--auto-medium`.

## Compilation (Windows 10/11)
1. Installer Rust (https://rustup.rs) et Git pour Windows.
2. `cargo build --release` → `target\release\drenn.exe` (dépendances : serde, serde_json uniquement).
3. `curl.exe` (inclus dans Windows 10/11) sert aux appels à l'API GitHub.

## Modèle IA (facultatif)
Sans modèle, un analyseur de règles FR/EN comprend les demandes courantes. Pour le modèle local :
1. Télécharger `llama-cli.exe` (https://github.com/ggml-org/llama.cpp/releases) et un modèle GGUF quantifié (ex. Qwen petit, Q4).
2. `drenn config init`, puis renseigner `llama_bin` et `model_path` dans `%USERPROFILE%\.drenn\config.json`.
3. `drenn doctor` vérifie le tout. Le modèle n'est chargé que pendant l'analyse d'une demande (aucune RAM occupée au repos) ; en cas de réponse invalide, l'agent se replie sur les règles.

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

## Limites connues de la v0.1
- Non testé sur Windows ni avec un vrai modèle Qwen/llama.cpp (flags `--json-schema`/`-no-cnv` avec repli automatique) : mesures mémoire à faire (phase 6).
- Création de dépôt GitHub / `list_remote_repositories` non testés (jeton requis).
- Détection de secrets heuristique (noms + motifs connus), pas exhaustive. Pas de résolution de conflits ni d'annulation de commit.
- Aucune version publiée automatiquement : les versions de GIT DRENN se créent uniquement sur demande.
