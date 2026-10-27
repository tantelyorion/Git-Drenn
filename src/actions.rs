//! Fonctions prédéfinies de l'agent : le modèle ne peut demander QUE ces actions, validées par un schéma strict.

use crate::security::Risk;
use crate::util::Res;
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    InspectRepository { #[serde(default)] path: Option<String> },
    CloneRepository { url: String, #[serde(default)] destination: Option<String> },
    InitializeRepository { #[serde(default)] path: Option<String> },
    InspectStatus { #[serde(default)] path: Option<String> },
    InspectDiff { #[serde(default)] path: Option<String>, #[serde(default)] staged: bool },
    InspectHistory { #[serde(default)] path: Option<String>, #[serde(default)] limit: Option<u32> },
    ListBranches { #[serde(default)] path: Option<String> },
    CreateBranch { #[serde(default)] path: Option<String>, name: String, #[serde(default = "yes", rename = "switch")] switch_to: bool },
    SwitchBranch { #[serde(default)] path: Option<String>, name: String },
    StageChanges { #[serde(default)] path: Option<String>, #[serde(default)] files: Vec<String> },
    CreateCommit { #[serde(default)] path: Option<String>, #[serde(default)] message: String },
    FetchChanges { #[serde(default)] path: Option<String>, #[serde(default)] remote: Option<String> },
    PullChanges { #[serde(default)] path: Option<String> },
    PushChanges {
        #[serde(default)] path: Option<String>,
        #[serde(default)] remote: Option<String>,
        #[serde(default)] branch: Option<String>,
        #[serde(default)] force: bool,
    },
    InspectRemotes { #[serde(default)] path: Option<String> },
    ConfigureRemote { #[serde(default)] path: Option<String>, #[serde(default)] name: Option<String>, url: String },
    DiscardChanges { #[serde(default)] path: Option<String>, #[serde(default)] files: Vec<String> },
    UpdateGitignore { #[serde(default)] path: Option<String>, #[serde(default)] patterns: Vec<String> },
    UndoLastCommit { #[serde(default)] path: Option<String> },
    MergeBranch { #[serde(default)] path: Option<String>, name: String, #[serde(default)] keep_conflicts: bool },
    InspectConflicts { #[serde(default)] path: Option<String> },
    ResolveConflicts { #[serde(default)] path: Option<String>, #[serde(default)] files: Vec<String>, strategy: String },
    AbortMerge { #[serde(default)] path: Option<String> },
    ContinueMerge { #[serde(default)] path: Option<String> },
    CreateTag { #[serde(default)] path: Option<String>, name: String, #[serde(default)] message: Option<String> },
    PushTag { #[serde(default)] path: Option<String>, name: String, #[serde(default)] remote: Option<String> },
    StashChanges { #[serde(default)] path: Option<String>, #[serde(default)] message: Option<String> },
    RestoreStash { #[serde(default)] path: Option<String> },
    ListRemoteRepositories {},
    SearchRemoteRepository { query: String },
    CreateRemoteRepository {
        #[serde(default)] name: String,
        #[serde(default = "yes")] private: bool,
        #[serde(default)] description: Option<String>,
        #[serde(default)] path: Option<String>,
    },
    InspectRemoteRepository { repository: String },
    VerifyRemotePush {
        #[serde(default)] path: Option<String>,
        #[serde(default)] remote: Option<String>,
        #[serde(default)] branch: Option<String>,
    },
}

pub const ACTION_NAMES: &[&str] = &[
    "inspect_repository", "clone_repository", "initialize_repository", "inspect_status", "inspect_diff",
    "inspect_history", "list_branches", "create_branch", "switch_branch", "stage_changes", "create_commit",
    "fetch_changes", "pull_changes", "push_changes", "inspect_remotes", "configure_remote", "discard_changes",
    "update_gitignore", "undo_last_commit", "merge_branch", "create_tag", "push_tag", "stash_changes", "restore_stash",
    "inspect_conflicts", "resolve_conflicts", "abort_merge", "continue_merge",
    "list_remote_repositories", "search_remote_repository", "create_remote_repository",
    "inspect_remote_repository", "verify_remote_push",
];

fn p(path: &Option<String>) -> String {
    match path.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => format!("« {s} »"),
        None => "ce dossier".into(),
    }
}

impl Action {
    pub fn risk(&self) -> Risk {
        use Action::*;
        match self {
            InspectRepository { .. } | InspectStatus { .. } | InspectDiff { .. } | InspectHistory { .. }
            | ListBranches { .. } | InspectRemotes { .. } | ListRemoteRepositories {} | SearchRemoteRepository { .. }
            | InspectRemoteRepository { .. } | VerifyRemotePush { .. } | FetchChanges { .. } | InspectConflicts { .. } => Risk::Low,
            CloneRepository { .. } | InitializeRepository { .. } | CreateBranch { .. } | SwitchBranch { .. }
            | StageChanges { .. } | CreateCommit { .. } | PullChanges { .. } | ConfigureRemote { .. }
            | UpdateGitignore { .. } | UndoLastCommit { .. } | MergeBranch { .. } | CreateTag { .. }
            | StashChanges { .. } | RestoreStash { .. } | AbortMerge { .. } | ContinueMerge { .. } => Risk::Medium,
            PushTag { .. } | ResolveConflicts { .. } => Risk::High,
            PushChanges { force, .. } => if *force { Risk::Critical } else { Risk::High },
            CreateRemoteRepository { private, .. } => if *private { Risk::High } else { Risk::Critical },
            DiscardChanges { .. } => Risk::Critical,
        }
    }

    pub fn needs_network(&self) -> bool {
        use Action::*;
        matches!(self, CloneRepository { .. } | FetchChanges { .. } | PullChanges { .. } | PushChanges { .. } | PushTag { .. }
            | ListRemoteRepositories {} | SearchRemoteRepository { .. } | CreateRemoteRepository { .. }
            | InspectRemoteRepository { .. } | VerifyRemotePush { .. })
    }

    pub fn describe(&self) -> String {
        use Action::*;
        match self {
            InspectRepository { path } => format!("Examiner le dépôt ({})", p(path)),
            CloneRepository { url, destination } => match destination {
                Some(d) if !d.trim().is_empty() => format!("Cloner « {url} » dans « {d} »"),
                _ => format!("Cloner « {url} »"),
            },
            InitializeRepository { path } => format!("Initialiser Git dans {} (si nécessaire)", p(path)),
            InspectStatus { path } => format!("Examiner l'état de {}", p(path)),
            InspectDiff { path, staged } => format!("Examiner les modifications {}de {}", if *staged { "indexées " } else { "" }, p(path)),
            InspectHistory { path, limit } => format!("Consulter l'historique de {} ({} derniers)", p(path), limit.unwrap_or(10)),
            ListBranches { path } => format!("Lister les branches de {}", p(path)),
            CreateBranch { name, switch_to, path } => format!("Créer la branche « {name} »{} dans {}", if *switch_to { " et s'y placer" } else { "" }, p(path)),
            SwitchBranch { name, path } => format!("Passer sur la branche « {name} » dans {}", p(path)),
            StageChanges { files, path } => if files.is_empty() {
                format!("Préparer toutes les modifications de {} (fichiers sensibles exclus)", p(path))
            } else {
                format!("Préparer {} fichier(s) dans {}", files.len(), p(path))
            },
            CreateCommit { message, path } => if message.trim().is_empty() {
                format!("Enregistrer les modifications préparées de {} (message généré)", p(path))
            } else {
                format!("Enregistrer les modifications de {} — « {} »", p(path), crate::util::truncate(message, 60))
            },
            FetchChanges { path, .. } => format!("Télécharger les nouveautés distantes de {}", p(path)),
            PullChanges { path } => format!("Mettre à jour {} (avance rapide uniquement, sans écraser votre travail)", p(path)),
            PushChanges { force, remote, branch, path } => format!(
                "Envoyer {}{} vers {}{}",
                branch.as_deref().map(|b| format!("la branche « {b} » de ")).unwrap_or_default(),
                p(path),
                remote.as_deref().unwrap_or("origin"),
                if *force { " — ENVOI FORCÉ" } else { "" }
            ),
            InspectRemotes { path } => format!("Consulter les dépôts distants de {}", p(path)),
            ConfigureRemote { name, url, .. } => format!("Configurer le dépôt distant « {} » → {url}", name.as_deref().unwrap_or("origin")),
            DiscardChanges { files, path } => format!(
                "ABANDONNER les modifications {} de {} (sauvegarde de secours créée ; fichiers non suivis conservés)",
                if files.is_empty() { "suivies".to_string() } else { format!("de {} fichier(s)", files.len()) },
                p(path)
            ),
            UpdateGitignore { patterns, path } => if patterns.is_empty() {
                format!("Compléter le .gitignore de {} (secrets et dossiers générés)", p(path))
            } else {
                format!("Ajouter {} règle(s) au .gitignore de {}", patterns.len(), p(path))
            },
            UndoLastCommit { path } => format!("Annuler le dernier commit de {} en gardant toutes ses modifications", p(path)),
            MergeBranch { name, path, keep_conflicts } => format!(
                "Fusionner la branche « {name} » dans la branche courante de {}{}",
                p(path),
                if *keep_conflicts { " (conflits conservés pour résolution guidée)" } else { "" }
            ),
            InspectConflicts { path } => format!("Lister les fichiers en conflit de {}", p(path)),
            ResolveConflicts { files, strategy, path } => format!(
                "Résoudre les conflits de {} ({}) {}",
                p(path),
                if files.is_empty() { "tous les fichiers".to_string() } else { format!("{} fichier(s)", files.len()) },
                match strategy.as_str() {
                    "ours" => "en gardant la version de la branche COURANTE",
                    "both" => "en gardant les DEUX versions de chaque zone",
                    "ask" => "zone par zone, avec votre choix à chaque fois",
                    _ => "en gardant la version de la branche FUSIONNÉE",
                }
            ),
            AbortMerge { path } => format!("Annuler la fusion en cours dans {} (retour à l'état d'avant)", p(path)),
            ContinueMerge { path } => format!("Terminer la fusion en cours dans {} (création du commit de fusion)", p(path)),
            CreateTag { name, path, .. } => format!("Créer l'étiquette de version « {name} » sur le dernier commit de {}", p(path)),
            PushTag { name, remote, path } => format!("Envoyer l'étiquette « {name} » de {} vers {}", p(path), remote.as_deref().unwrap_or("origin")),
            StashChanges { path, .. } => format!("Mettre de côté les modifications en cours de {} (récupérables)", p(path)),
            RestoreStash { path } => format!("Remettre les dernières modifications mises de côté dans {}", p(path)),
            ListRemoteRepositories {} => "Lister vos dépôts GitHub".into(),
            SearchRemoteRepository { query } => format!("Rechercher « {query} » sur GitHub"),
            CreateRemoteRepository { name, private, .. } => format!(
                "Créer le dépôt GitHub {} « {} » (ignoré si un remote origin existe déjà)",
                if *private { "PRIVÉ" } else { "PUBLIC" },
                if name.is_empty() { "<nom du projet>" } else { name }
            ),
            InspectRemoteRepository { repository } => format!("Consulter le dépôt GitHub « {repository} »"),
            VerifyRemotePush { path, .. } => format!("Vérifier que le dépôt distant contient bien les modifications de {}", p(path)),
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct Plan {
    pub goal: String,
    pub steps: Vec<Action>,
    pub question: Option<String>,
    pub notes: Vec<String>,
}

impl Plan {
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty() && self.question.is_none()
    }

    pub fn max_risk(&self) -> Risk {
        self.steps.iter().map(|s| s.risk()).max().unwrap_or(Risk::Low)
    }

    /// Valide strictement un plan produit par un modèle (JSON → actions connues, paramètres conformes).
    pub fn from_value(v: &Value) -> Res<Plan> {
        let goal = v.get("goal").and_then(|g| g.as_str()).unwrap_or("").trim().to_string();
        let question = v
            .get("question")
            .and_then(|q| q.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut steps = Vec::new();
        if let Some(arr) = v.get("steps").and_then(|s| s.as_array()) {
            if arr.len() > 12 {
                return Err("plan trop long (12 étapes maximum)".into());
            }
            for (i, s) in arr.iter().enumerate() {
                let cleaned = match s {
                    Value::Object(m) => Value::Object(m.iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.clone(), v.clone())).collect()),
                    other => other.clone(),
                };
                let a: Action = serde_json::from_value(cleaned).map_err(|e| format!("étape {} invalide : {e}", i + 1))?;
                steps.push(a);
            }
        }
        Ok(Plan { goal, steps, question, notes: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plan_valide() {
        let v = json!({"goal":"x","steps":[{"action":"inspect_status","path":null},{"action":"create_branch","name":"dev"}]});
        let p = Plan::from_value(&v).unwrap();
        assert_eq!(p.steps.len(), 2);
        assert_eq!(p.steps[1], Action::CreateBranch { path: None, name: "dev".into(), switch_to: true });
    }

    #[test]
    fn action_inconnue_ou_param_inconnu_refuses() {
        assert!(Plan::from_value(&json!({"steps":[{"action":"rm_rf","path":"/"}]})).is_err());
        assert!(Plan::from_value(&json!({"steps":[{"action":"inspect_status","command":"rm -rf /"}]})).is_err());
        assert!(Plan::from_value(&json!({"steps":[{"action":"create_branch"}]})).is_err());
    }

    #[test]
    fn risques() {
        let push = Action::PushChanges { path: None, remote: None, branch: None, force: false };
        assert_eq!(push.risk(), Risk::High);
        let force = Action::PushChanges { path: None, remote: None, branch: None, force: true };
        assert_eq!(force.risk(), Risk::Critical);
        assert_eq!(Action::InspectStatus { path: None }.risk(), Risk::Low);
        assert_eq!(Action::DiscardChanges { path: None, files: vec![] }.risk(), Risk::Critical);
    }
}
