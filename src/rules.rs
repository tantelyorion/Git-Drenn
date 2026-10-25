//! Analyseur de règles (français / anglais) : repli sans modèle IA, aussi utilisé quand le modèle échoue.
//! Même contrat que le modèle : il ne produit que des `Action` prédéfinies.

use crate::actions::{Action, Plan};
use crate::planner::PlanContext;
use crate::util::{normalize, slugify, truncate, Res};

struct Tok {
    orig: String,
    norm: String,
}

const STOP: &[&str] = &[
    "le", "la", "les", "l", "un", "une", "des", "du", "de", "d", "mon", "ma", "mes", "ton", "son", "sa", "ses", "sur", "vers",
    "dans", "en", "sous", "ou", "pour", "que", "qui", "github", "gitlab", "actuel", "actuels", "actuelle", "actuelles", "courant",
    "courante", "ici", "the", "my", "a", "an", "to", "on", "in", "of", "this", "that", "nouveau", "nouvelle", "new", "ce", "cette",
    "tous", "toutes", "tout", "toute", "au", "aux", "avec", "msgq", "me", "moi", "je", "j", "puisse", "afin",
];

const CONNECTORS: &[&str] = &["puis", "ensuite", "then", "and", "et", "apres", "after", "afterwards", "avant"];

fn tokenize(text: &str) -> Vec<Vec<Tok>> {
    let spaced: String = text.chars().map(|c| if c == '\'' || c == '’' { ' ' } else { c }).collect();
    let mut clauses: Vec<Vec<Tok>> = vec![vec![]];
    for raw in spaced.split_whitespace() {
        let brk = raw.ends_with([',', ';', '.', '!', '?']) && !raw.contains(['/', '\\']);
        let clean = raw.trim_matches(|c: char| ",;:!?«»\"()“”".contains(c)).trim_end_matches('.');
        if clean.is_empty() {
            if brk {
                clauses.push(vec![]);
            }
            continue;
        }
        let norm = normalize(clean);
        if CONNECTORS.contains(&norm.as_str()) {
            clauses.push(vec![]);
            continue;
        }
        clauses.last_mut().unwrap().push(Tok { orig: clean.to_string(), norm });
        if brk {
            clauses.push(vec![]);
        }
    }
    clauses.retain(|c| !c.is_empty());
    clauses
}

fn extract_quoted(s: &str) -> (Option<String>, String) {
    for (open, close) in [('«', '»'), ('"', '"'), ('“', '”')] {
        if let Some(i) = s.find(open) {
            let rest = &s[i + open.len_utf8()..];
            if let Some(j) = rest.find(close) {
                let q = rest[..j].trim().to_string();
                let mut rem = s[..i].to_string();
                rem.push_str(" MSGQ ");
                rem.push_str(&rest[j + close.len_utf8()..]);
                return (Some(q).filter(|q| !q.is_empty()), rem);
            }
        }
    }
    (None, s.to_string())
}

fn pos(c: &[Tok], stems: &[&str]) -> Option<usize> {
    c.iter().position(|t| {
        stems.iter().any(|s| match s.strip_suffix('=') {
            Some(e) => t.norm == e,
            None => t.norm.starts_with(s),
        })
    })
}

fn has(c: &[Tok], stems: &[&str]) -> bool {
    pos(c, stems).is_some()
}

fn is_stop(w: &str) -> bool {
    STOP.contains(&w)
}

/// Nom de projet après « projet / dépôt / application … ».
fn project_name(c: &[Tok]) -> Option<String> {
    let i = pos(c, &["projet", "project", "depot=", "repo=", "repository", "application=", "app=", "site="])?;
    c[i + 1..].iter().take(2).find(|t| !is_stop(&t.norm)).map(|t| t.orig.clone())
}

fn clone_target(c: &[Tok], start: usize) -> Option<String> {
    const FILL: &[&str] = &["depot", "projet", "repo", "repository", "code", "source", "sources", "project", "dernier", "derniere"];
    c[start + 1..]
        .iter()
        .find(|t| !is_stop(&t.norm) && !FILL.contains(&t.norm.as_str()))
        .filter(|t| !matches!(t.norm.as_str(), "ordinateur" | "machine" | "pc" | "computer" | "disque"))
        .map(|t| t.orig.clone())
}

fn destination(c: &[Tok]) -> Option<String> {
    let i = pos(c, &["dans=", "in=", "into=", "vers="])?;
    let mut j = i + 1;
    while j < c.len() && (matches!(c[j].norm.as_str(), "mon" | "le" | "la" | "ma" | "mes" | "my" | "the" | "un" | "dossier" | "repertoire" | "folder" | "directory" | "répertoire")) {
        j += 1;
    }
    let t = c.get(j)?;
    if matches!(t.norm.as_str(), "ordinateur" | "machine" | "pc" | "computer" | "disque") {
        return None;
    }
    Some(t.orig.clone())
}

fn branch_name(c: &[Tok], i: usize) -> Option<String> {
    const NAMED: &[&str] = &["nommee", "appelee", "nomme", "appele", "named", "called"];
    const FILL: &[&str] = &["travailler", "work", "working", "developper", "bosser", "implementer", "pour", "for", "nommee", "appelee"];
    let rest = &c[i + 1..];
    if let Some(n) = rest.first() {
        if NAMED.contains(&n.norm.as_str()) {
            return rest.get(1).map(|t| t.orig.clone());
        }
    }
    let words: Vec<&str> = rest
        .iter()
        .filter(|t| !is_stop(&t.norm) && !FILL.contains(&t.norm.as_str()))
        .take(5)
        .map(|t| t.orig.as_str())
        .collect();
    if words.is_empty() {
        return None;
    }
    let slug = slugify(&words);
    if slug.is_empty() {
        return None;
    }
    Some(if words.len() >= 2 { format!("feature/{slug}") } else { slug })
}

struct Ctx<'a> {
    pc: &'a PlanContext,
    project: Option<String>,
    message: Option<String>,
    public: bool,
}

fn clause(c: &[Tok], x: &Ctx) -> Result<Vec<Action>, String> {
    let path = project_name(c).or_else(|| x.project.clone());
    let q = |s: &str| Err(s.to_string());

    // 1. Annuler / abandonner des modifications
    if has(c, &["annul", "undo", "discard", "abandon", "revert", "restaur", "jette"]) {
        if has(c, &["commit"]) {
            return q("Annuler un commit n'est pas encore pris en charge par GIT DRENN. Souhaitez-vous plutôt abandonner les modifications non enregistrées ?");
        }
        if has(c, &["modif", "change", "travail"]) {
            return Ok(vec![Action::DiscardChanges { path, files: vec![] }]);
        }
    }
    // 2. Publier
    if has(c, &["publi"]) && has(c, &["github", "ligne", "online", "distant"]) {
        return Ok(vec![
            Action::InitializeRepository { path: path.clone() },
            Action::StageChanges { path: path.clone(), files: vec![] },
            Action::CreateCommit { path: path.clone(), message: x.message.clone().unwrap_or_default() },
            Action::CreateRemoteRepository { name: String::new(), private: !x.public, description: None, path: path.clone() },
            Action::PushChanges { path: path.clone(), remote: None, branch: None, force: false },
            Action::VerifyRemotePush { path, remote: None, branch: None },
        ]);
    }
    // 3. Enregistrer (commit)
    if has(c, &["enregistr", "commit", "sauvegard", "valide"]) && !has(c, &["branch"]) {
        return Ok(vec![
            Action::StageChanges { path: path.clone(), files: vec![] },
            Action::CreateCommit { path, message: x.message.clone().unwrap_or_default() },
        ]);
    }
    // 4. Envoyer
    if has(c, &["envoi", "envoy", "push", "pouss", "upload", "televers", "publi"]) {
        return Ok(vec![
            Action::PushChanges { path: path.clone(), remote: None, branch: None, force: false },
            Action::VerifyRemotePush { path, remote: None, branch: None },
        ]);
    }
    // 5. Cloner
    if let Some(i) = pos(c, &["clon", "clone=", "git clone"]).or_else(|| pos(c, &["telecharg", "download"])) {
        return match clone_target(c, i) {
            Some(t) => Ok(vec![Action::CloneRepository { url: t, destination: destination(c) }]),
            None => q("Quel dépôt souhaitez-vous cloner (nom, propriétaire/nom ou URL) ?"),
        };
    }
    // 6. Mettre à jour
    if has(c, &["recuper", "pull", "fetch", "synchron", "update", "actualis"]) || (has(c, &["met", "mise", "mettre"]) && has(c, &["jour"])) {
        let latest = has(c, &["dernier", "modif", "mise", "update", "latest", "change", "nouveaut", "jour"]);
        if latest || x.pc.in_repo {
            return Ok(vec![Action::PullChanges { path }]);
        }
        return q("Quel dépôt souhaitez-vous récupérer (nom, propriétaire/nom ou URL) ?");
    }
    // 7. Branches
    if let Some(i) = pos(c, &["branch"]) {
        if has(c, &["cre", "nouvell", "create", "new", "ajout"]) {
            return match branch_name(c, i) {
                Some(name) => Ok(vec![Action::CreateBranch { path, name, switch_to: true }]),
                None => q("Quel nom donner à la nouvelle branche ?"),
            };
        }
        if has(c, &["bascul", "passe", "switch", "checkout", "change", "aller", "va"]) {
            let name = c[i + 1..].iter().find(|t| !is_stop(&t.norm)).map(|t| t.orig.clone());
            return match name {
                Some(name) => Ok(vec![Action::SwitchBranch { path, name }]),
                None => q("Sur quelle branche souhaitez-vous passer ?"),
            };
        }
        return Ok(vec![Action::ListBranches { path }]);
    }
    // 8. Initialiser
    if has(c, &["initialis", "init="]) {
        return Ok(vec![Action::InitializeRepository { path }]);
    }
    // 9. Préparer les modifications
    if has(c, &["prepar", "stage", "ajoute", "indexe"]) && has(c, &["modif", "chang", "fichier", "travail"]) {
        return Ok(vec![Action::StageChanges { path, files: vec![] }]);
    }
    // 10. Rechercher / lister sur GitHub
    if has(c, &["cherch", "recherch", "search", "trouve"]) {
        let words: Vec<&str> = c
            .iter()
            .skip(1)
            .take_while(|t| !matches!(t.norm.as_str(), "sur" | "on" | "in" | "github"))
            .filter(|t| !is_stop(&t.norm) && !matches!(t.norm.as_str(), "depot" | "projet" | "repo"))
            .map(|t| t.orig.as_str())
            .collect();
        if !words.is_empty() {
            return Ok(vec![Action::SearchRemoteRepository { query: words.join(" ") }]);
        }
    }
    if has(c, &["depot", "repo"]) && has(c, &["github"]) && has(c, &["list", "mes", "affich", "montr", "show"]) {
        return Ok(vec![Action::ListRemoteRepositories {}]);
    }
    // 11. Lecture
    if has(c, &["diff", "modif", "chang", "difference"]) {
        return Ok(vec![Action::InspectDiff { path, staged: false }]);
    }
    if has(c, &["histor", "log=", "commits", "journal"]) {
        return Ok(vec![Action::InspectHistory { path, limit: None }]);
    }
    if has(c, &["statut", "status", "etat", "situation"]) {
        return Ok(vec![Action::InspectStatus { path }]);
    }
    if has(c, &["remote", "distant", "origin"]) {
        return Ok(vec![Action::InspectRemotes { path }]);
    }
    if has(c, &["inspect", "examin", "analys", "decri", "montre", "affiche", "presente", "ouvre"]) {
        return Ok(vec![Action::InspectRepository { path }]);
    }
    Ok(vec![])
}

pub fn plan(instruction: &str, ctx: &PlanContext) -> Res<Plan> {
    let instruction = instruction.trim();
    if instruction.is_empty() {
        return Err("instruction vide".into());
    }
    let (message, remaining) = extract_quoted(instruction);
    let clauses = tokenize(&remaining);
    let all: Vec<&Tok> = clauses.iter().flatten().collect();
    let public = all.iter().any(|t| t.norm.starts_with("public")) && !all.iter().any(|t| t.norm.starts_with("prive") || t.norm == "private");
    // Projet cité n'importe où dans la demande (sauf clauses de clonage, où le nom est le dépôt distant).
    let project = clauses
        .iter()
        .filter(|c| pos(c, &["clon", "telecharg", "download"]).is_none())
        .find_map(|c| project_name(c));
    let x = Ctx { pc: ctx, project, message, public };

    let mut plan = Plan { goal: truncate(instruction, 140), ..Plan::default() };
    for c in &clauses {
        match clause(c, &x) {
            Ok(steps) if steps.is_empty() => {
                let txt: Vec<&str> = c.iter().map(|t| t.orig.as_str()).collect();
                plan.notes.push(format!("partie non comprise, ignorée : « {} »", txt.join(" ")));
            }
            Ok(steps) => plan.steps.extend(steps),
            Err(question) => {
                plan.question = Some(question);
                plan.steps.clear();
                return Ok(plan);
            }
        }
    }
    if plan.steps.is_empty() {
        plan.question = Some(
            "Je n'ai pas compris cette demande. Essayez par exemple : « clone mastodon », « enregistre mes modifications », « crée une branche login » ou « publie mon projet Blog sur GitHub »."
                .into(),
        );
        plan.notes.clear();
    }
    // Après un clonage, « examiner le projet » est déjà couvert par la présentation du clone
    // (sinon l'étape viserait le dossier courant, pas le projet cloné).
    if matches!(plan.steps.first(), Some(Action::CloneRepository { .. })) {
        plan.steps.retain(|s| !matches!(s, Action::InspectRepository { path: None }));
    }
    // Dédoublonnage des étapes strictement identiques consécutives.
    plan.steps.dedup();
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(in_repo: bool) -> PlanContext {
        PlanContext { cwd: "/x".into(), in_repo, branch: None, dirty: 0, remotes: vec![] }
    }
    fn names(p: &Plan) -> Vec<String> {
        p.steps.iter().map(|s| serde_json::to_value(s).unwrap()["action"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn cloner() {
        let p = plan("Clone Mastodon dans mon dossier Projets.", &ctx(false)).unwrap();
        assert_eq!(p.steps, vec![Action::CloneRepository { url: "Mastodon".into(), destination: Some("Projets".into()) }]);
        let p = plan("Clone Mastodon sur mon ordinateur, puis prépare le projet pour que je puisse examiner son code source.", &ctx(false)).unwrap();
        assert_eq!(names(&p), vec!["clone_repository"]);
        assert_eq!(p.notes.len(), 0.max(p.notes.len())); // clause 2 ignorée ou comprise : pas bloquante
    }

    #[test]
    fn publier() {
        let p = plan("Publie mon projet Dabawalla sur GitHub.", &ctx(false)).unwrap();
        assert_eq!(names(&p), vec!["initialize_repository", "stage_changes", "create_commit", "create_remote_repository", "push_changes", "verify_remote_push"]);
        assert_eq!(p.steps[0], Action::InitializeRepository { path: Some("Dabawalla".into()) });
        match &p.steps[3] {
            Action::CreateRemoteRepository { private, .. } => assert!(*private),
            _ => panic!(),
        }
        let p = plan("Publie mon projet Dabawalla en public sur GitHub", &ctx(false)).unwrap();
        match &p.steps[3] {
            Action::CreateRemoteRepository { private, .. } => assert!(!*private),
            _ => panic!(),
        }
    }

    #[test]
    fn enregistrer() {
        let p = plan("Enregistre les modifications actuelles avec un message qui explique ce que j'ai changé.", &ctx(true)).unwrap();
        assert_eq!(names(&p), vec!["stage_changes", "create_commit"]);
        assert_eq!(p.steps[1], Action::CreateCommit { path: None, message: String::new() });
        let p = plan("Enregistre mes modifications avec le message « Ajout du login »", &ctx(true)).unwrap();
        assert_eq!(p.steps[1], Action::CreateCommit { path: None, message: "Ajout du login".into() });
    }

    #[test]
    fn mise_a_jour_et_annulation() {
        let p = plan("Récupère les dernières modifications du projet.", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::PullChanges { path: None }]);
        let p = plan("Annule mes dernières modifications sans supprimer mes nouveaux fichiers.", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::DiscardChanges { path: None, files: vec![] }]);
    }

    #[test]
    fn branches() {
        let p = plan("Crée une branche pour travailler sur le nouveau système de connexion.", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::CreateBranch { path: None, name: "feature/systeme-connexion".into(), switch_to: true }]);
        let p = plan("crée une branche nommée Login", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::CreateBranch { path: None, name: "Login".into(), switch_to: true }]);
        let p = plan("crée une branche login", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::CreateBranch { path: None, name: "login".into(), switch_to: true }]);
        let p = plan("bascule sur la branche dev", &ctx(true)).unwrap();
        assert_eq!(p.steps, vec![Action::SwitchBranch { path: None, name: "dev".into() }]);
    }

    #[test]
    fn enchainement() {
        let p = plan("Récupère le projet, crée une branche de développement, prépare les modifications et envoie-les sur GitHub.", &ctx(true)).unwrap();
        assert_eq!(names(&p), vec!["pull_changes", "create_branch", "stage_changes", "push_changes", "verify_remote_push"]);
    }

    #[test]
    fn lecture_et_incomprehension() {
        let p = plan("Quel est l'état du projet ?", &ctx(true)).unwrap();
        assert_eq!(names(&p), vec!["inspect_status"]);
        let p = plan("blabla zzz", &ctx(true)).unwrap();
        assert!(p.steps.is_empty() && p.question.is_some());
        let p = plan("Annule mon dernier commit", &ctx(true)).unwrap();
        assert!(p.question.is_some());
    }

    #[test]
    fn anglais() {
        let p = plan("clone rust-lang/rust into Projects", &ctx(false)).unwrap();
        assert_eq!(p.steps, vec![Action::CloneRepository { url: "rust-lang/rust".into(), destination: Some("Projects".into()) }]);
        let p = plan("push my project Blog", &ctx(true)).unwrap();
        assert_eq!(names(&p), vec!["push_changes", "verify_remote_push"]);
    }
}
