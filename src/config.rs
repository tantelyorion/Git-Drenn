//! Configuration de GIT DRENN (~/.drenn/config.json).

use crate::util::{drenn_dir, Res};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Chemin de `llama-cli` (llama.cpp). Absent = analyseur de règles uniquement.
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
