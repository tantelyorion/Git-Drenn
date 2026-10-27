# GIT DRENN — Guide du moteur IA

*GIT DRENN est créé par **Mr Tantely Orion**.*

> **Honnêteté d'abord.** Le code est testé avec de faux moteurs (un faux `llama-server` lancé, utilisé puis arrêté ; de faux serveurs HTTP
> pour l'API locale et le cloud). Il n'a **pas encore été essayé avec un vrai modèle Qwen** ni avec un vrai service cloud (pas de téléchargement
> possible dans l'environnement de développement). `drenn doctor` puis `drenn bench` vous diront si tout fonctionne chez vous.

## 1. Principe (inchangé : sécurité d'abord)

```
Votre phrase ──► Moteur IA ──► Plan JSON ──► Validation stricte ──► Confirmation ──► Exécution Git ──► Vérification
```

Le moteur ne fait que **traduire** votre phrase en actions prédéfinies. Il n'exécute rien. Il ne voit **jamais** le contenu de vos fichiers,
ni vos conflits de fusion. Sans moteur, l'analyseur de règles (français/anglais) couvre les demandes courantes ; en cas d'échec du moteur,
l'agent retombe sur les règles.

## 2. Les trois moteurs (`drenn config set backend …`)

| `backend` | Fonctionnement | Quand l'utiliser |
|---|---|---|
| `server` | **llama-server** lancé à la première demande et **gardé en mémoire** (déchargé après `server_idle_secs`, 300 s par défaut). Réponses rapides. Peut aussi être un serveur local existant (`server_url`, ex. Ollama). Utilise le gabarit de chat du modèle : tout modèle « Instruct » convient. | Recommandé. |
| `cli` | `llama-cli` relancé à chaque demande : rien en mémoire au repos, mais lent (rechargement). Gabarit ChatML (famille Qwen). | PC très limité. |
| `cloud` | API distante (Anthropic ou compatible OpenAI). **Désactivé par défaut**, voir §5. | PC trop faible pour un modèle local. |
| `rules` | Aucun modèle. | Mode minimal. |
| `auto` (défaut) | serveur → cli → cloud (seulement si activé) → règles. | |

## 3. Installation en deux commandes (tous systèmes)

```
drenn model engine     # trouve llama-server / llama-cli ; sinon donne la procédure de VOTRE système
drenn model install    # choisit le modèle selon votre RAM, télécharge, vérifie le SHA-256, configure
```

| Votre PC | Modèle (Q4_K_M) | Fichier | RAM pendant l'analyse |
|---|---|---|---|
| 4 Go | `qwen2.5-1.5b` | ≈ 1,1 Go | ≈ 1,5–2 Go |
| 8 Go et plus | `qwen2.5-3b` | ≈ 2 Go | ≈ 2,5–3 Go |
| très faible | `qwen2.5-0.5b` | ≈ 0,4 Go | < 1 Go (moins fiable) |

`drenn model list` montre la liste et le modèle conseillé. Le téléchargement reprend s'il est interrompu ; si l'empreinte publiée par
Hugging Face ne correspond pas, le fichier est supprimé. Sous Windows, `scripts\setup-model.ps1` fait aussi le travail pas à pas.

**Autre modèle** : téléchargez un `.gguf` « Instruct », puis `drenn config set model_path <fichier>`, `drenn bench` pour comparer.
Évitez les modèles qui « réfléchissent à voix haute » : ils gênent la sortie JSON.

### Utiliser Ollama ou LM Studio (déjà installé)

```
drenn config set server_url  http://127.0.0.1:11434
drenn config set server_model qwen2.5:1.5b
```
Seules les adresses de **cette machine** sont acceptées : vos phrases ne circulent jamais sur le réseau local.

## 4. Fonctions IA ajoutées

- **Messages de commit par IA** : « enregistre mes modifications » sans message → le modèle **local** rédige une ligne (`feat: …`) à partir de la
  nature du changement et du **nom** des fichiers, jamais de leur contenu, jamais via le cloud. Réglages : `ai_commit_messages`, `commit_lang` (fr/en).
  Si le modèle répond mal, le message standard est utilisé.
- **Mode apprentissage** (`--teach`, `drenn config set teach_mode oui`, ou « active le mode apprentissage ») : chaque étape affiche la commande Git
  équivalente et une explication. Textes fixes : aucune invention possible.
- **Explications** : « c'est quoi un commit ? », « explique le stash » → glossaire intégré (conflit, branche, fusion, remote…) ; hors glossaire, le modèle
  répond en texte simple (aucune action possible).
- **Conflits zone par zone** : « résous les conflits un par un » → pour chaque zone, vous voyez les deux versions et choisissez : la vôtre, l'autre, les deux,
  ou passer le fichier. Les fichiers d'origine sont sauvegardés dans `.git/drenn-backup/`. « … en gardant les deux » traite toutes les zones d'un coup.
  Fins de ligne et encodage conservés ; fichiers binaires ou mal formés ignorés avec explication.

## 5. Mode cloud (facultatif, à vos risques)

1. `drenn auth cloud` (clé API, stockée dans `~/.drenn/cloud_key`, droits restreints sous Unix) — ou variable `DRENN_CLOUD_KEY` / `ANTHROPIC_API_KEY` / `OPENAI_API_KEY`.
2. `drenn config set cloud_provider anthropic` (ou `openai`), éventuellement `cloud_model`, `cloud_url` (https obligatoire).
3. `drenn config set cloud_enabled oui` puis `drenn config set backend cloud` (ou laissez `auto` : le cloud n'est utilisé que si aucun moteur local n'existe).

Garanties : **une confirmation à chaque session** avant le premier envoi ; seule votre **phrase** et un contexte minimal (nom de branche assaini, nombre de fichiers modifiés)
partent ; jamais le contenu des dépôts, ni les noms de fichiers (pas de message de commit par le cloud), ni les conflits ; la clé n'apparaît dans aucun message d'erreur.
Le modèle par défaut (`claude-haiku-5-5` pour Anthropic, `gpt-4o-mini` pour OpenAI) n'a pas été essayé ici : changez `cloud_model` si besoin.

## 6. Réglages (`drenn config`)

`backend`, `llama_server_bin`, `llama_bin`, `model_path`, `server_url`, `server_model`, `server_idle_secs`, `llama_server_extra_args`, `threads`, `ctx_size`,
`max_tokens`, `llama_timeout_secs`, `use_json_schema`, `cloud_*`, `ai_commit_messages`, `commit_lang`, `teach_mode`.

## 7. Dépannage

- **`drenn doctor`** affiche le système, la RAM, le moteur actif et ce qui manque.
- **« llama-server s'est arrêté au démarrage »** : mémoire insuffisante → modèle plus petit (`qwen2.5-0.5b`), ou `backend cli`.
- **Trop lent / délai dépassé** : augmentez `llama_timeout_secs` ; vérifiez `threads`.
- **Mauvais plans** : `drenn bench` ; essayez le 3B ; reformulez. Pour tester le reste sans IA : `drenn --no-ai "…"`.
- **Arrêt brutal de GIT DRENN** : fermez un éventuel `llama-server` resté en mémoire (gestionnaire de tâches / `pkill llama-server`).

## 8. Idées pour la suite

Signature des binaires (macOS/Windows), mises à jour automatiques vérifiées, rebase assisté, résolution de conflits avec suggestion du modèle (à n'activer qu'en local),
interface graphique avec aperçu coloré des différences.
