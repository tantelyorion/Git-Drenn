# GIT DRENN — Guide du moteur IA (LLM)

*GIT DRENN est créé par **Mr Tantely Orion**.*

> **Honnêteté d'abord.** Le code qui appelle le modèle (`src/planner.rs`) est testé avec des faux modèles et des cas
> d'erreur, mais **pas encore avec un vrai modèle Qwen + llama.cpp** (pas de téléchargement possible dans l'environnement
> de développement). Les procédures ci-dessous sont les procédures standard ; `drenn doctor` et `drenn bench` vous diront
> si tout fonctionne chez vous.

## 1. Comment ça marche

```
Votre phrase ──► Modèle IA (LLM local) ──► Plan JSON ──► Validation stricte ──► Confirmation ──► Exécution Git ──► Vérification
```

- Le modèle **ne fait que traduire** votre phrase en un plan composé d'actions prédéfinies (`stage_changes`, `create_commit`, `push_changes`…).
- Il n'exécute **jamais** de commande : c'est le moteur Rust qui valide, demande confirmation et exécute.
- Il ne voit **jamais** le contenu de vos fichiers ni vos messages de commit.
- Sans modèle, un **analyseur de règles** (français/anglais) couvre déjà les demandes courantes. Le modèle sert aux phrases
  plus libres ou mal formulées. Si le modèle se trompe ou répond mal, l'agent retombe sur les règles.

## 2. Quel modèle choisir ?

Il faut un **petit modèle « Instruct »**, au format **GGUF**, quantifié **Q4_K_M**. Le gabarit de `planner.rs` est du ChatML : la famille **Qwen** convient directement.

| Votre PC | Modèle conseillé | Taille du fichier | RAM utilisée pendant l'analyse |
|---|---|---|---|
| 4 Go de RAM | **Qwen2.5-1.5B-Instruct** Q4_K_M | ≈ 1,1 Go | ≈ 1,5 à 2 Go |
| 8 Go de RAM ou plus | **Qwen2.5-3B-Instruct** Q4_K_M | ≈ 2 Go | ≈ 2,5 à 3 Go |
| PC très faible | Qwen2.5-0.5B-Instruct Q4_K_M | ≈ 0,4 Go | < 1 Go (moins fiable) |

Des modèles plus récents (Qwen3, etc.) existent peut-être : si vous en essayez un, lancez `drenn bench` pour comparer
(un modèle « raisonneur » qui écrit d'abord ses pensées peut gêner la sortie JSON).
Un modèle plus gros n'est pas forcément meilleur ici : la sortie est **contrainte par un schéma JSON**, donc un petit modèle suffit.

## 3. Installation (méthode guidée, Windows)

1. Installez GIT DRENN (`scripts\install.ps1`).
2. Lancez :
   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\setup-model.ps1
   ```
   Le script cherche la dernière version de **llama.cpp** (Windows CPU), propose les modèles Qwen ci-dessus selon votre RAM,
   demande votre accord avant chaque téléchargement, affiche l'empreinte SHA-256 puis configure GIT DRENN.
3. Vérifiez : `drenn doctor` puis `drenn bench`.

## 4. Installation manuelle

1. Téléchargez `llama-cli.exe` : https://github.com/ggml-org/llama.cpp/releases (archive Windows **CPU x64**) et dézippez-la.
2. Téléchargez un fichier `.gguf` (page Hugging Face du modèle, onglet *Files*).
3. Configurez :
   ```powershell
   drenn config set llama_bin  "C:\chemin\llama-cli.exe"
   drenn config set model_path "C:\chemin\qwen2.5-1.5b-instruct-q4_k_m.gguf"
   drenn doctor
   ```

## 5. Réglages utiles (`drenn config`)

| Clé | Rôle | Conseil |
|---|---|---|
| `threads` | cœurs utilisés | nombre de cœurs physiques (max 4 par défaut) |
| `ctx_size` | contexte | 2048 suffit |
| `max_tokens` | longueur max de la réponse | 512 |
| `llama_timeout_secs` | délai maximal | augmentez si le PC est lent |
| `use_json_schema` | force le JSON valide | laissez à `true` |

## 6. Dépannage

- **« llama.cpp a échoué »** : lancez `drenn doctor` ; vérifiez les chemins ; essayez `drenn --no-ai "…"` pour confirmer que le reste fonctionne.
- **Très lent** : prenez un modèle plus petit ou augmentez `llama_timeout_secs`. Le modèle est rechargé à chaque demande (voir §7).
- **Mauvais plans** : `drenn bench` ; essayez le modèle 3B ; reformulez plus précisément.

## 7. Améliorations possibles (prochaines étapes)

1. **Serveur llama.cpp persistant** (`llama-server`, API locale compatible OpenAI) : le modèle resterait chargé, les réponses passeraient de plusieurs secondes à moins d'une seconde, au prix de la RAM occupée en permanence.
2. **Mode « cloud » facultatif** (API d'un fournisseur de LLM) pour les PC trop faibles : à n'activer que si vous acceptez d'envoyer vos phrases (jamais le contenu des dépôts) à un service externe.
3. **Messages de commit générés par l'IA** à partir d'un résumé *local* des fichiers modifiés (noms et statistiques seulement, jamais le contenu).
4. **Explications pédagogiques** : « que va faire cette commande Git ? » pour apprendre Git en l'utilisant.
5. Conflits de fusion **ligne par ligne** (aujourd'hui : fichier entier).
