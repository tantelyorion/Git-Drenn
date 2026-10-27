//! Configuration de GIT DRENN (~/.drenn/config.json).

use crate::util::{drenn_dir, Res};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Chemin de `llama-cli` (llama.cpp ; `llama-cli.exe` sous Windows). Sans moteur local ni cloud : analyseur de règles.
    pub llama_bin: Option<String>,
    /// Chemin du modèle GGUF quantifié (ex. Qwen 0.8B Q4).
    pub model_path: Option<String>,
    pub ctx_size: u32,
    pub threads: u32,
    pub max_tokens: u32,
    /// Contraint la sortie du modèle par un schéma JSON (recommandé pour les petits modèles).
    pub use_json_schema: bool,
    pub llama_timeout_secs: u64,
    pub llama_extra_args: Vec<String>,
    /// Dossiers dans lesquels l'agent peut travailler. Vide = dossier courant + dossier personnel.
    pub allowed_roots: Vec<String>,
    /// Dossier par défaut des clones et recherche de projets.
    pub projects_dir: Option<String>,
    /// Exécute sans confirmation les opérations locales de niveau moyen (jamais push / suppression).
    pub auto_confirm_medium: bool,
    pub git_timeout_secs: u64,
    pub network_timeout_secs: u64,
    /// Autorise les hooks Git des projets (désactivés par défaut : ce sont des scripts non fiables).
    pub allow_hooks: bool,
    pub max_file_size_mb: u64,

    // ---- v0.6 : moteurs IA
    /// « auto » (serveur → processus → cloud si autorisé → règles), « server », « cli », « cloud » ou « rules ».
    pub backend: String,
    /// Chemin de `llama-server` (llama.cpp) : le modèle reste chargé entre deux demandes (déchargé après inactivité).
    pub llama_server_bin: Option<String>,
    pub llama_server_extra_args: Vec<String>,
    /// Serveur déjà lancé, compatible OpenAI, SUR CETTE MACHINE uniquement (ex. http://127.0.0.1:11434 pour Ollama).
    pub server_url: Option<String>,
    /// Nom du modèle à demander au serveur (obligatoire pour Ollama, ex. « qwen2.5:1.5b »).
    pub server_model: Option<String>,
    /// Secondes d'inactivité avant de décharger le modèle de la mémoire (0 = jamais).
    pub server_idle_secs: u64,
    /// Autorise l'envoi de VOS PHRASES (jamais le contenu des dépôts) à un service cloud. Désactivé par défaut.
    pub cloud_enabled: bool,
    /// « anthropic » ou « openai » (toute API compatible OpenAI via cloud_url).
    pub cloud_provider: String,
    pub cloud_url: Option<String>,
    pub cloud_model: Option<String>,
    /// Messages de commit rédigés par le modèle LOCAL à partir des noms de fichiers (jamais de leur contenu).
    pub ai_commit_messages: bool,
    /// Langue des messages de commit générés : « fr » ou « en ».
    pub commit_lang: String,
    /// Mode apprentissage : affiche la commande Git équivalente et une explication pour chaque étape.
    pub teach_mode: bool,
}

impl Default for Config {
    fn default() -> Self {
        let threads = std::thread::available_parallelism().map(|n| n.get() as u32).unwrap_or(2).min(4);
        Config {
            llama_bin: None,
            model_path: None,
            ctx_size: 2048,
            threads,
            max_tokens: 512,
            use_json_schema: true,
            llama_timeout_secs: 180,
            llama_extra_args: vec![],
            allowed_roots: vec![],
            projects_dir: None,
            auto_confirm_medium: false,
            git_timeout_secs: 60,
            network_timeout_secs: 600,
            allow_hooks: false,
            max_file_size_mb: 50,
            backend: "auto".into(),
            llama_server_bin: None,
            llama_server_extra_args: vec![],
            server_url: None,
            server_model: None,
            server_idle_secs: 300,
            cloud_enabled: false,
            cloud_provider: "anthropic".into(),
            cloud_url: None,
            cloud_model: None,
            ai_commit_messages: true,
            commit_lang: "fr".into(),
            teach_mode: false,
        }
    }
}

pub fn config_path() -> PathBuf {
    drenn_dir().join("config.json")
}

impl Config {
    pub fn load() -> Res<Config> {
        let p = config_path();
        if !p.exists() {
            return Ok(Config::default());
        }
        let txt = std::fs::read_to_string(&p).map_err(|e| format!("lecture de {} impossible : {e}", p.display()))?;
        serde_json::from_str(&txt).map_err(|e| format!("configuration invalide ({}) : {e}", p.display()))
    }

    pub fn save(&self) -> Res<PathBuf> {
        let p = config_path();
        if let Some(d) = p.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("création de {} impossible : {e}", d.display()))?;
        }
        let txt = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&p, txt).map_err(|e| format!("écriture impossible : {e}"))?;
        Ok(p)
    }
}
