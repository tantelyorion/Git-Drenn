# GIT DRENN — installation sur Windows, macOS, Linux et BSD

*Créateur : Mr Tantely Orion.*

GIT DRENN est un programme Rust unique, sans dépendance à l'exécution, à part **Git** (obligatoire) et **curl**
(GitHub, mode cloud, téléchargement du modèle). Les fichiers de configuration sont dans `~/.drenn` (`%USERPROFILE%\.drenn` sous Windows).

| Système | Prérequis | Installation |
|---|---|---|
| **Windows 10/11** | Git pour Windows ; curl.exe est fourni | binaire de la CI, ou `scripts\install.ps1` (voir GUIDE_COMPILATION.md) |
| **macOS** | `xcode-select --install` (Git) ; Rust via https://rustup.rs | `sh scripts/install.sh` |
| **Linux** | `git`, `curl`, Rust (rustup ou paquet) | `sh scripts/install.sh` |
| **FreeBSD** | `pkg install git curl rust` | `sh scripts/install.sh` |
| **OpenBSD** | `pkg_add git curl rust` | `sh scripts/install.sh` |
| **NetBSD** | `pkgin install git curl rust` | `sh scripts/install.sh` |

Pour que `drenn` soit trouvé, ajoutez `~/.local/bin` à votre PATH si le script vous le signale.

## Moteur IA (facultatif) sur chaque système

```
drenn model engine     # cherche llama-server / llama-cli ; sinon explique comment l'installer chez vous
drenn model install    # télécharge le modèle adapté à votre RAM, vérifie son empreinte SHA-256, configure
drenn doctor           # contrôle global
drenn bench            # mesure la justesse et le temps
```

llama.cpp : Windows (`winget` ou archive officielle), macOS/Linux (`brew install llama.cpp` ou archive), FreeBSD (paquet ou port
`misc/llama-cpp`), OpenBSD/NetBSD (compilation depuis les sources avec cmake). Les noms de paquets peuvent évoluer : `drenn model engine`
affiche les consignes à jour de votre système.
Alternative sur tous les systèmes : Ollama ou LM Studio déjà installé →
`drenn config set server_url http://127.0.0.1:11434` et `drenn config set server_model <nom du modèle>`.

## État des tests (honnêteté)

- **Linux** : testé (compilation, 70 tests, essais bout en bout avec un faux serveur IA et l'assistant de conflits).
- **Windows, macOS, BSD** : le code est écrit pour ces systèmes (chemins, ouverture du navigateur, détection de la RAM, noms d'exécutables,
  recherche de Git) mais **n'a pas pu être exécuté dans l'environnement de développement**. Le workflow
  `.github/workflows/multiplateforme.yml` lance les tests sur Windows, macOS et Linux (BSD en essai) : c'est la vérification à regarder en premier.
- Si le programme est arrêté brutalement (fermeture forcée), un processus `llama-server` peut rester en mémoire : fermez-le dans le gestionnaire de tâches.
