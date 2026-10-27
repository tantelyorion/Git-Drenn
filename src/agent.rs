//! Agent : instruction → plan → validation → confirmation → exécution contrôlée → vérification.

use crate::actions::{Action, Plan};
use crate::config::Config;
use crate::github::GitHub;
use crate::gitengine::{explain_error, Entry, Git, Kind};
use crate::conflicts::{self, Choice};
use crate::llm::{self, Llm};
use crate::planner::{self, PlanContext};
use crate::rules;
use crate::teach;
use crate::security::{self, Policy, Risk};
use crate::ui;
use crate::util::*;
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub struct Agent {
    pub cfg: Config,
    pub policy: Policy,
    pub git: Git,
    pub gh: GitHub,
    pub cwd: PathBuf,
    pub dry_run: bool,
    pub force_rules: bool,
    /// Réservé aux tests : répond « oui » aux confirmations (jamais activé par la CLI).
    pub test_yes: bool,
    /// Moteur IA choisi d'après la configuration (None = analyseur de règles seul).
    pub llm: Option<Arc<dyn Llm>>,
    /// Mode apprentissage de la session (en plus de `cfg.teach_mode`).
    pub teach: AtomicBool,
    /// Consentement donné pour cette session à l'envoi des phrases vers un service cloud.
    cloud_ok: AtomicBool,
    /// Réservé aux tests : choix prédéfinis pour les questions à choix multiples.
    pub test_choices: Mutex<VecDeque<usize>>,
}

/// Nettoie la réponse d'un modèle pour en faire un objet de commit : une ligne, 72 caractères au plus.
pub fn clean_commit_message(raw: &str) -> Option<String> {
    let strip = |t: &str| t.trim().trim_matches(|c| matches!(c, '"' | '`' | '\'' | '«' | '»' | '.')).trim().to_string();
    let first = raw.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut line = strip(first);
    for p in ["message :", "message:", "commit :", "commit:"] {
        if line.to_lowercase().starts_with(p) {
            line = strip(&line[p.len()..]);
        }
    }
    let line: String = line.chars().filter(|c| !c.is_control()).take(72).collect::<String>().trim().to_string();
    if line.chars().count() < 4 { None } else { Some(line) }
}

fn clean(s: &str) -> String {
    // Neutralise les séquences d'échappement d'un dépôt non fiable avant affichage.
    s.chars().filter(|c| !c.is_control() || *c == '\n' || *c == '\t').collect()
}

fn short(h: &str) -> &str {
    &h[..h.len().min(8)]
}

fn norm_rel(p: &str) -> String {
    p.replace('\\', "/").trim_start_matches("./").trim_end_matches('/').to_string()
}

const ALIASES: &[(&str, &str)] = &[
    ("mastodon", "mastodon/mastodon"),
    ("linux", "torvalds/linux"),
    ("git", "git/git"),
    ("rust", "rust-lang/rust"),
    ("react", "facebook/react"),
    ("vscode", "microsoft/vscode"),
    ("llama.cpp", "ggml-org/llama.cpp"),
    ("laravel", "laravel/laravel"),
    ("wordpress", "WordPress/WordPress"),
    ("django", "django/django"),
];

const SKIP_DIRS: &[&str] = &["node_modules", "target", "vendor", "appdata", "library", "windows", "program files", "$recycle.bin"];

impl Agent {
    pub fn new(cfg: Config, cwd: PathBuf, token: Option<String>) -> Res<Agent> {
        let git = Git::detect(&cfg, token.clone())?;
        let cwd = canonicalize_lenient(&cwd)?;
        let policy = Policy::new(&cfg, &cwd);
        let llm = llm::select(&cfg);
        let teach = AtomicBool::new(cfg.teach_mode);
        Ok(Agent {
            cfg,
            policy,
            git,
            gh: GitHub::new(token),
            cwd,
            dry_run: false,
            force_rules: false,
            test_yes: false,
            llm,
            teach,
            cloud_ok: AtomicBool::new(false),
            test_choices: Mutex::new(VecDeque::new()),
        })
    }

    /// Choix multiple (les tests injectent leurs réponses ; sinon console ou interface graphique).
    fn pick(&self, q: &str, opts: &[String]) -> Option<usize> {
        if self.test_yes {
            return self.test_choices.lock().ok().and_then(|mut c| c.pop_front());
        }
        ui::choose(q, opts)
    }

    /// Un moteur distant n'est utilisé qu'avec l'accord explicite de l'utilisateur, une fois par session.
    fn allow_llm(&self, l: &dyn Llm) -> bool {
        if l.is_local() || self.cloud_ok.load(Ordering::SeqCst) {
            return true;
        }
        ui::warn(format!("Le moteur configuré est : {}. Votre phrase (jamais le contenu de vos fichiers) serait envoyée sur Internet.", l.name()));
        if self.ask("Autoriser l'envoi pour cette session ?") {
            self.cloud_ok.store(true, Ordering::SeqCst);
            true
        } else {
            ui::info("Refusé : l'analyseur de règles local est utilisé.");
            false
        }
    }

    fn ask(&self, q: &str) -> bool {
        self.test_yes || ui::confirm(q)
    }
    fn ask_typed(&self, q: &str, word: &str) -> bool {
        self.test_yes || ui::confirm_typed(q, word)
    }

    // ------------------------------------------------------------ contexte et résolution

    pub fn context(&self) -> PlanContext {
        let in_repo = self.git.is_repo(&self.cwd);
        let (branch, dirty, remotes) = if in_repo {
            (
                self.git.branch(&self.cwd),
                self.git.status(&self.cwd).map(|s| s.len()).unwrap_or(0),
                self.git.remotes(&self.cwd).into_iter().map(|r| r.0).collect(),
            )
        } else {
            (None, 0, vec![])
        };
        PlanContext { cwd: self.cwd.display().to_string(), in_repo, branch, dirty, remotes }
    }

    fn abs(&self, s: &str) -> PathBuf {
        let p = Path::new(s.trim().trim_matches('"'));
        if p.is_absolute() { p.to_path_buf() } else { self.cwd.join(p) }
    }

    fn search_bases(&self) -> Vec<PathBuf> {
        let mut bases = vec![self.cwd.clone()];
        if let Some(p) = &self.cfg.projects_dir {
            bases.push(PathBuf::from(p));
        }
        if let Some(h) = home_dir() {
            for sub in ["Projects", "Projets", "Documents", "Desktop", "Bureau", "dev", "source/repos", ""] {
                bases.push(if sub.is_empty() { h.clone() } else { h.join(sub) });
            }
        }
        bases.extend(self.policy.roots().iter().cloned());
        let mut seen = BTreeSet::new();
        bases.into_iter().filter(|b| b.is_dir() && seen.insert(b.clone())).collect()
    }

    /// Parcourt les dossiers usuels (profondeur ≤ 3) ; `pred` décide si un dossier est retenu.
    fn walk(&self, pred: &dyn Fn(&Path, &str) -> bool, limit: usize) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = Vec::new();
        let mut visited = 0usize;
        let mut seen = BTreeSet::new();
        for base in self.search_bases() {
            let mut queue = vec![(base, 0u8)];
            while let Some((dir, depth)) = queue.pop() {
                if visited > 6000 || found.len() >= limit {
                    return found;
                }
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                for e in rd.flatten() {
                    visited += 1;
                    let Ok(ft) = e.file_type() else { continue };
                    if !ft.is_dir() || ft.is_symlink() {
                        continue;
                    }
                    let name = e.file_name().to_string_lossy().to_string();
                    let low = name.to_lowercase();
                    if name.starts_with('.') || SKIP_DIRS.contains(&low.as_str()) {
                        continue;
                    }
                    let p = e.path();
                    if pred(&p, &name) && seen.insert(p.clone()) {
                        found.push(p.clone());
                    }
                    if depth < 3 && !p.join(".git").exists() {
                        queue.push((p, depth + 1));
                    }
                }
            }
        }
        found
    }

    fn locate_projects(&self, name: &str) -> Vec<PathBuf> {
        let target = normalize(name);
        let found = self.walk(&|_, n| normalize(n) == target, 20);
        let with_git: Vec<PathBuf> = found.iter().filter(|p| p.join(".git").exists()).cloned().collect();
        if with_git.is_empty() { found } else { with_git }
    }

    pub fn scan_repos(&self) -> Vec<PathBuf> {
        self.walk(&|p, _| p.join(".git").exists(), 60)
    }

    fn resolve_dir(&self, p: &Option<String>) -> Res<PathBuf> {
        let Some(s) = p.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
            return self.policy.check_path(&self.cwd);
        };
        let cand = self.abs(s);
        if cand.is_dir() {
            return self.policy.check_path(&cand);
        }
        if pathlike(s) {
            return Err(format!("dossier introuvable : {s}"));
        }
        let found: Vec<PathBuf> = self.locate_projects(s).into_iter().filter(|f| self.policy.check_path(f).is_ok()).collect();
        match found.len() {
            0 => Err(format!("projet « {s} » introuvable dans les dossiers autorisés. Indiquez son chemin complet.")),
            1 => self.policy.check_path(&found[0]),
            _ => {
                let opts: Vec<String> = found.iter().map(|f| f.display().to_string()).collect();
                match ui::choose(&format!("Plusieurs projets « {s} » ont été trouvés :"), &opts) {
                    Some(i) if i < found.len() => self.policy.check_path(&found[i]),
                    _ => Err("choix du projet annulé ou invalide".into()),
                }
            }
        }
    }

    fn resolve_repo(&self, p: &Option<String>) -> Res<PathBuf> {
        let dir = self.resolve_dir(p)?;
        let root = self.git.toplevel(&dir)?;
        self.policy.check_path(&root)
    }

    fn remote_for(&self, repo: &Path, wanted: &Option<String>) -> Res<(String, String)> {
        let remotes = self.git.remotes(repo);
        if remotes.is_empty() {
            return Err("aucun dépôt distant n'est configuré (demandez par exemple « publie mon projet sur GitHub »)".into());
        }
        match wanted.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(w) => {
                security::valid_remote_name(w)?;
                remotes.into_iter().find(|r| r.0 == w).ok_or_else(|| format!("dépôt distant « {w} » inconnu"))
            }
            None => Ok(remotes.iter().find(|r| r.0 == "origin").cloned().unwrap_or_else(|| remotes[0].clone())),
        }
    }

    // ------------------------------------------------------------ plan

    fn make_plan(&self, instruction: &str) -> Res<(Plan, String)> {
        let ctx = self.context();
        if !self.force_rules {
            if let Some(l) = &self.llm {
                if self.allow_llm(l.as_ref()) {
                    ui::info(format!("Analyse de la demande par : {}…", l.name()));
                    match planner::plan_with(l.as_ref(), instruction, &ctx, self.cfg.max_tokens) {
                        Ok(p) if !p.is_empty() => return Ok((p, l.name())),
                        Ok(_) => ui::warn("réponse vide du modèle — repli sur l'analyseur de règles"),
                        Err(e) => ui::warn(format!("modèle IA indisponible ou invalide ({e}) — repli sur l'analyseur de règles")),
                    }
                }
            }
        }
        rules::plan(instruction, &ctx).map(|p| (p, "analyseur de règles".to_string()))
    }

    /// Validation statique des paramètres, AVANT toute confirmation ou exécution.
    pub fn validate(&self, a: &Action) -> Res<()> {
        use Action::*;
        let rels = |files: &Vec<String>| -> Res<()> { files.iter().try_for_each(|f| security::valid_rel_path(&norm_rel(f))) };
        match a {
            CloneRepository { url, destination } => {
                if security::looks_like_url(url) {
                    security::valid_repo_url(url)?;
                } else if url.trim().is_empty() || url.starts_with('-') || url.len() > 200 {
                    return Err("dépôt à cloner invalide".into());
                }
                if let Some(d) = destination {
                    if d.contains('\0') {
                        return Err("destination invalide".into());
                    }
                }
                Ok(())
            }
            UpdateGitignore { patterns, .. } => {
                if patterns.len() > 50 {
                    return Err("trop de règles .gitignore à la fois (50 maximum)".into());
                }
                for pat in patterns {
                    let t = pat.trim();
                    if t.is_empty() || t.len() > 200 || t.chars().any(|c| c.is_control()) || matches!(t, "*" | "**" | "/" | "!*" | ".") {
                        return Err(format!("règle .gitignore refusée : « {pat} »"));
                    }
                }
                Ok(())
            }
            PushTag { name, remote, .. } => {
                if let Some(r) = remote {
                    security::valid_remote_name(r)?;
                }
                if name.is_empty() || name.starts_with('-') || name.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    return Err(format!("nom d'étiquette invalide : « {name} »"));
                }
                Ok(())
            }
            StashChanges { message: Some(m), .. } => security::valid_commit_message(m),
            ResolveConflicts { files, strategy, .. } => {
                if !["ours", "theirs", "both", "ask"].contains(&strategy.as_str()) {
                    return Err("stratégie invalide : « ours » (branche courante), « theirs » (branche fusionnée), « both » (les deux) ou « ask » (zone par zone)".into());
                }
                rels(files)
            }
            CreateBranch { name, .. } | SwitchBranch { name, .. } | MergeBranch { name, .. } | CreateTag { name, .. } => {
                if name.is_empty() || name.starts_with('-') || name.len() > 100 || name.chars().any(|c| c.is_control() || c.is_whitespace()) {
                    Err(format!("nom de branche invalide : « {name} »"))
                } else {
                    Ok(())
                }
            }
            StageChanges { files, .. } | DiscardChanges { files, .. } => rels(files),
            CreateCommit { message, .. } => if message.trim().is_empty() { Ok(()) } else { security::valid_commit_message(message) },
            // (message d'étiquette validé à l'exécution)
            ConfigureRemote { name, url, .. } => {
                if let Some(n) = name {
                    security::valid_remote_name(n)?;
                }
                security::valid_repo_url(url)
            }
            PushChanges { remote, branch, .. } | VerifyRemotePush { remote, branch, .. } => {
                if let Some(r) = remote {
                    security::valid_remote_name(r)?;
                }
                if let Some(b) = branch {
                    if b.starts_with('-') || b.chars().any(|c| c.is_control() || c.is_whitespace()) {
                        return Err(format!("nom de branche invalide : « {b} »"));
                    }
                }
                Ok(())
            }
            FetchChanges { remote: Some(r), .. } => security::valid_remote_name(r),
            CreateRemoteRepository { name, .. } => if name.is_empty() { Ok(()) } else { security::valid_repo_name(name) },
            InspectRemoteRepository { repository } => security::valid_repo_slug(repository).map(|_| ()),
            SearchRemoteRepository { query } => if query.trim().is_empty() || query.len() > 100 { Err("recherche invalide".into()) } else { Ok(()) },
            _ => Ok(()),
        }
    }

    /// Point d'entrée : traite une instruction de bout en bout. Renvoie vrai si tout s'est bien passé.
    pub fn handle(&self, instruction: &str) -> bool {
        if let Some(ok) = self.teach_command(instruction) {
            return ok;
        }
        let teaching = self.teach.load(Ordering::SeqCst);
        let (plan, source) = match self.make_plan(instruction) {
            Ok(x) => x,
            Err(e) => {
                ui::err(e);
                return false;
            }
        };
        if let Some(q) = &plan.question {
            ui::say(format!("\n{q}"));
            return false;
        }
        ui::say(format!("\nObjectif : {}   (plan établi par : {source})", plan.goal));
        for n in &plan.notes {
            ui::warn(n);
        }
        for (i, s) in plan.steps.iter().enumerate() {
            let tag = match s.risk() {
                Risk::Low => "lecture",
                Risk::Medium => "local  ",
                Risk::High => "ÉLEVÉ  ",
                Risk::Critical => "CRITIQUE",
            };
            let net = if s.needs_network() { " (Internet)" } else { "" };
            ui::say(format!("  {}. [{tag}] {}{net}", i + 1, s.describe()));
            if teaching {
                if let Some((cmd, why)) = teach::equivalent(s) {
                    ui::say(format!("       Git : {cmd}\n       {why}"));
                }
            }
        }
        for (i, s) in plan.steps.iter().enumerate() {
            if let Err(e) = self.validate(s) {
                ui::err(format!("étape {} refusée : {e}", i + 1));
                return false;
            }
        }
        let risk = plan.max_risk();
        ui::say(format!("Risque maximal : {}", risk.label()));
        if self.dry_run {
            ui::info("Simulation (--dry-run) : rien n'a été exécuté.");
            return true;
        }
        let go = match risk {
            Risk::Low => true,
            Risk::Medium if self.cfg.auto_confirm_medium => true,
            _ => self.ask("Exécuter ce plan ?"),
        };
        if !go {
            ui::say("Plan annulé. Aucune modification n'a été effectuée.");
            return false;
        }
        let total = plan.steps.len();
        let mut results: Vec<(String, bool, String)> = Vec::new();
        let mut all_ok = true;
        for (i, s) in plan.steps.iter().enumerate() {
            ui::say(format!("\n> Étape {}/{} : {}", i + 1, total, s.describe()));
            match self.exec(s) {
                Ok(msg) => {
                    ui::ok(clean(&msg));
                    results.push((s.describe(), true, clean(&msg)));
                }
                Err(e) => {
                    ui::err(clean(&e));
                    if let Some(h) = explain_error(&e) {
                        ui::say(format!("     Cause probable : {h}"));
                    }
                    results.push((s.describe(), false, clean(&e)));
                    if i + 1 < total {
                        ui::warn(format!("Plan interrompu : {} étape(s) non exécutée(s).", total - i - 1));
                    }
                    all_ok = false;
                    break;
                }
            }
        }
        if all_ok {
            ui::say(format!("\nTerminé : {total}/{total} étape(s) exécutée(s) et vérifiée(s)."));
        }
        self.log_history(instruction, &plan.goal, &results);
        all_ok
    }

    /// « mode apprentissage », « c'est quoi un commit ? », « explique le stash » : réponses pédagogiques sans toucher au dépôt.
    fn teach_command(&self, instruction: &str) -> Option<bool> {
        let n = teach::norm(instruction);
        if n.contains("mode apprentissage") || n.contains("mode pedagogique") || n.contains("mode explication") || n.contains("mode teach") {
            let off = ["desactiv", "arret", "stop", "coupe", "retire", "sans ", " off"].iter().any(|k| n.contains(k));
            self.teach.store(!off, Ordering::SeqCst);
            if off {
                ui::ok("Mode apprentissage désactivé.");
            } else {
                ui::ok("Mode apprentissage activé : chaque étape sera accompagnée de la commande Git équivalente et d'une explication.");
            }
            return Some(true);
        }
        let (term, pure) = teach::definition_request(instruction)?;
        if let Some(t) = teach::glossary(&term) {
            ui::say(format!("\n{t}"));
            return Some(true);
        }
        if !pure {
            return None; // « explique l'état du projet » : demande d'action ordinaire
        }
        let Some(l) = &self.llm else {
            ui::say(format!("Je n'ai pas de définition pour « {term} » dans mon glossaire (commit, branche, fusion, conflit, stash, remote, push, pull, fetch, clone, tag, index, gitignore, HEAD, dépôt…).\nInstallez un modèle IA (`drenn model install`) pour des réponses libres."));
            return Some(false);
        };
        if self.force_rules || !self.allow_llm(l.as_ref()) {
            ui::say(format!("Je n'ai pas de définition pour « {term} » dans mon glossaire."));
            return Some(false);
        }
        const TEACHER: &str = "Tu es un professeur de Git patient. Réponds en français, en 6 phrases maximum, avec des mots simples et un exemple concret. Si une commande peut détruire du travail, préviens-en clairement.";
        match l.chat(TEACHER, &format!("Question : {term}"), None, 350) {
            Ok(t) if !t.trim().is_empty() => {
                ui::say(format!("\n{}", clean(t.trim())));
                Some(true)
            }
            Ok(_) => Some(false),
            Err(e) => {
                ui::err(format!("explication impossible : {e}"));
                Some(false)
            }
        }
    }

    fn log_history(&self, instruction: &str, goal: &str, results: &[(String, bool, String)]) {
        let dir = drenn_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let file = dir.join("history.jsonl");
        if std::fs::metadata(&file).map(|m| m.len() > 1_000_000).unwrap_or(false) {
            let _ = std::fs::rename(&file, dir.join("history.old.jsonl"));
        }
        let steps: Vec<serde_json::Value> = results
            .iter()
            .map(|(d, ok, m)| serde_json::json!({"step": d, "ok": ok, "result": truncate(m, 400)}))
            .collect();
        let rec = serde_json::json!({"ts": now_secs(), "instruction": truncate(instruction, 300), "goal": goal, "steps": steps});
        use std::io::Write;
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
            let _ = writeln!(f, "{rec}");
        }
    }

    // ------------------------------------------------------------ exécution + vérification

    pub fn exec(&self, a: &Action) -> Res<String> {
        self.validate(a)?;
        use Action::*;
        match a {
            InspectRepository { path } => self.inspect_repository(path),
            CloneRepository { url, destination } => self.clone_repo(url, destination),
            InitializeRepository { path } => self.init_repo(path),
            InspectStatus { path } => self.inspect_status(path),
            InspectDiff { path, staged } => self.inspect_diff(path, *staged),
            InspectHistory { path, limit } => {
                let repo = self.resolve_repo(path)?;
                if self.git.head(&repo).is_none() {
                    return Ok("Aucun commit pour l'instant.".into());
                }
                let n = limit.unwrap_or(10).clamp(1, 100).to_string();
                let o = self.git.out(&repo, &["log", "--oneline", "--decorate", "--no-color", "-n", &n], Kind::Local)?;
                Ok(format!("Historique :\n{}", indent(&o)))
            }
            ListBranches { path } => {
                let repo = self.resolve_repo(path)?;
                let o = self.git.out(&repo, &["branch", "--list", "-vv", "--no-color"], Kind::Local)?;
                if o.trim().is_empty() {
                    return Ok(format!("Branche courante : {} (aucun commit pour l'instant).", self.git.branch(&repo).unwrap_or_else(|| "?".into())));
                }
                Ok(format!("Branches :\n{}", indent(&o)))
            }
            CreateBranch { path, name, switch_to } => self.create_branch(path, name, *switch_to),
            SwitchBranch { path, name } => self.switch_branch(path, name),
            StageChanges { path, files } => self.stage(path, files),
            CreateCommit { path, message } => self.commit(path, message),
            FetchChanges { path, remote } => {
                let repo = self.resolve_repo(path)?;
                let (r, _) = self.remote_for(&repo, remote)?;
                self.git.out(&repo, &["fetch", &r], Kind::Net)?;
                Ok(format!("Nouveautés de « {r} » téléchargées (votre travail local n'a pas été modifié)."))
            }
            PullChanges { path } => self.pull(path),
            PushChanges { path, remote, branch, force } => self.push(path, remote, branch, *force),
            InspectRemotes { path } => {
                let repo = self.resolve_repo(path)?;
                let r = self.git.remotes(&repo);
                if r.is_empty() {
                    return Ok("Aucun dépôt distant configuré.".into());
                }
                Ok(format!("Dépôts distants :\n{}", r.iter().map(|(n, u)| format!("  {n} → {u}")).collect::<Vec<_>>().join("\n")))
            }
            ConfigureRemote { path, name, url } => self.configure_remote(path, name, url),
            DiscardChanges { path, files } => self.discard(path, files),
            UpdateGitignore { path, patterns } => self.update_gitignore(path, patterns),
            UndoLastCommit { path } => self.undo_last_commit(path),
            MergeBranch { path, name, keep_conflicts } => self.merge_branch(path, name, *keep_conflicts),
            InspectConflicts { path } => self.inspect_conflicts(path),
            ResolveConflicts { path, files, strategy } => self.resolve_conflicts(path, files, strategy),
            AbortMerge { path } => self.abort_merge(path),
            ContinueMerge { path } => self.continue_merge(path),
            CreateTag { path, name, message } => self.create_tag(path, name, message),
            PushTag { path, name, remote } => self.push_tag(path, name, remote),
            StashChanges { path, message } => self.stash(path, message),
            RestoreStash { path } => self.restore_stash(path),
            ListRemoteRepositories {} => {
                let repos = self.gh.list_repos()?;
                Ok(format!("Vos dépôts GitHub (30 derniers modifiés) :\n{}", repos.iter().map(fmt_repo).collect::<Vec<_>>().join("\n")))
            }
            SearchRemoteRepository { query } => {
                let repos = self.gh.search_repos(query)?;
                if repos.is_empty() {
                    return Ok(format!("Aucun dépôt trouvé pour « {query} »."));
                }
                Ok(format!("Résultats pour « {query} » :\n{}", repos.iter().map(fmt_repo).collect::<Vec<_>>().join("\n")))
            }
            CreateRemoteRepository { name, private, description, path } => self.create_remote(name, *private, description, path),
            InspectRemoteRepository { repository } => match self.gh.repo(repository)? {
                Some(r) => Ok(format!(
                    "{}\n  {}\n  Branche par défaut : {} · {} étoile(s) · mis à jour le {}\n  Cloner : {}",
                    fmt_repo(&r), r.html_url, r.default_branch, r.stars, r.updated_at, r.clone_url
                )),
                None => Err(format!("le dépôt « {repository} » n'existe pas ou n'est pas accessible")),
            },
            VerifyRemotePush { path, remote, branch } => {
                let repo = self.resolve_repo(path)?;
                let (r, url) = self.remote_for(&repo, remote)?;
                let b = self.target_branch(&repo, branch)?;
                self.verify_push(&repo, &r, &url, &b)
            }
        }
    }

    fn target_branch(&self, repo: &Path, branch: &Option<String>) -> Res<String> {
        let b = match branch.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(b) => b.to_string(),
            None => self.git.branch(repo).ok_or("aucune branche courante (HEAD détaché ou aucun commit)")?,
        };
        if b.starts_with('-') || !self.git.run(repo, &["check-ref-format", "--branch", &b], Kind::Local)?.ok() {
            return Err(format!("nom de branche invalide : « {b} »"));
        }
        Ok(b)
    }

    fn local_hash(&self, repo: &Path, branch: &str) -> Res<String> {
        self.git
            .line(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{branch}")])
            .map_err(|_| format!("la branche locale « {branch} » n'existe pas ou n'a aucun commit"))
    }

    fn verify_push(&self, repo: &Path, remote: &str, url: &str, branch: &str) -> Res<String> {
        let local = self.local_hash(repo, branch)?;
        match self.git.remote_branch_hash(repo, remote, branch)? {
            Some(h) if h == local => Ok(format!("Vérifié : « {remote} » ({url}) contient la branche « {branch} » au commit {}.", short(&local))),
            Some(h) => Err(format!("« {remote} » est au commit {} alors que votre branche est au commit {} : les dépôts ne sont pas synchronisés", short(&h), short(&local))),
            None => Err(format!("la branche « {branch} » n'existe pas sur « {remote} »")),
        }
    }

    // ---- lecture

    fn inspect_repository(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let mut s = format!("Dépôt : {}\n", repo.display());
        s += &format!("  Branche : {}\n", self.git.branch(&repo).unwrap_or_else(|| "(aucune)".into()));
        match self.git.head(&repo) {
            Some(h) => {
                let subj = self.git.line(&repo, &["log", "-1", "--format=%s"]).unwrap_or_default();
                s += &format!("  Dernier commit : {} — {}\n", short(&h), subj);
            }
            None => s += "  Aucun commit.\n",
        }
        let remotes = self.git.remotes(&repo);
        s += &if remotes.is_empty() { "  Distant : aucun\n".to_string() } else { format!("  Distant : {}\n", remotes.iter().map(|r| format!("{} ({})", r.0, r.1)).collect::<Vec<_>>().join(", ")) };
        s += &summarize_status(&self.git.status(&repo)?);
        s += &present_project(&repo);
        Ok(s.trim_end().to_string())
    }

    fn inspect_status(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let branch = self.git.branch(&repo).unwrap_or_else(|| "(aucune)".into());
        let up = self.git.upstream(&repo).map(|u| format!(" → {u}")).unwrap_or_default();
        Ok(format!("Branche : {branch}{up}\n{}", summarize_status(&self.git.status(&repo)?).trim_end()))
    }

    fn inspect_diff(&self, path: &Option<String>, staged: bool) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let mut args = vec!["diff", "--no-color", "--stat"];
        if staged {
            args.push("--cached");
        }
        let stat = self.git.out(&repo, &args, Kind::Local)?;
        let entries = self.git.status(&repo)?;
        let untracked: Vec<&Entry> = entries.iter().filter(|e| e.untracked()).collect();
        let mut s = String::new();
        if stat.trim().is_empty() {
            s += if staged { "Aucune modification indexée.\n" } else { "Aucune modification de fichier suivi.\n" };
        } else {
            s += &format!("Modifications :\n{}\n", indent(&stat));
        }
        if !untracked.is_empty() && !staged {
            s += &format!("Nouveaux fichiers non suivis ({}) : {}\n", untracked.len(), list_names(untracked.iter().map(|e| e.path.as_str()), 10));
        }
        Ok(s.trim_end().to_string())
    }

    // ---- clonage

    fn resolve_repository(&self, q: &str) -> Res<(String, String)> {
        let q = q.trim();
        let name_of = |u: &str| -> String {
            u.trim_end_matches('/').rsplit(['/', ':']).next().unwrap_or("depot").trim_end_matches(".git").to_string()
        };
        if security::looks_like_url(q) {
            security::valid_repo_url(q)?;
            return Ok((q.to_string(), name_of(q)));
        }
        if let Ok((o, n)) = security::valid_repo_slug(q) {
            return Ok((format!("https://github.com/{o}/{n}.git"), n));
        }
        let key = normalize(q);
        if let Some((_, slug)) = ALIASES.iter().find(|(a, _)| *a == key) {
            ui::info(format!("Dépôt officiel retenu pour « {q} » : github.com/{slug}"));
            let n = slug.split('/').nth(1).unwrap_or(q).to_string();
            return Ok((format!("https://github.com/{slug}.git"), n));
        }
        ui::info(format!("Recherche de « {q} » sur GitHub…"));
        let repos = self.gh.search_repos(q)?;
        if repos.is_empty() {
            return Err(format!("aucun dépôt trouvé pour « {q} » : précisez propriétaire/nom ou l'URL"));
        }
        let opts: Vec<String> = repos.iter().map(fmt_repo).collect();
        let idx = if self.test_yes { Some(0) } else { ui::choose("Plusieurs dépôts correspondent. Lequel souhaitez-vous cloner ?", &opts) };
        match idx {
            Some(i) if i < repos.len() => {
                let r = &repos[i];
                let n = r.full_name.split('/').nth(1).unwrap_or(q).to_string();
                Ok((r.clone_url.clone(), n))
            }
            _ => Err("choix du dépôt annulé".into()),
        }
    }

    fn folder_hint(&self, h: &str) -> Option<PathBuf> {
        let mut cands = vec![self.abs(h)];
        if !pathlike(h) {
            if let Some(p) = &self.cfg.projects_dir {
                cands.push(PathBuf::from(p).join(h));
            }
            if let Some(home) = home_dir() {
                cands.push(home.join(h));
                cands.push(home.join("Documents").join(h));
                cands.push(home.join("Desktop").join(h));
            }
        }
        cands.into_iter().find(|c| c.is_dir() && self.policy.check_path(c).is_ok())
    }

    fn clone_repo(&self, url: &str, destination: &Option<String>) -> Res<String> {
        let (url, name) = self.resolve_repository(url)?;
        security::valid_repo_url(&url)?;
        security::valid_repo_name(&name)?;
        let dest = match destination.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(h) => match self.folder_hint(h) {
                Some(dir) => dir.join(&name),
                None => self.abs(h),
            },
            None => match &self.cfg.projects_dir {
                Some(p) => PathBuf::from(p).join(&name),
                None => self.cwd.join(&name),
            },
        };
        let dest = self.policy.check_path(&dest)?;
        if dest.exists() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(true) {
            return Err(format!("{} existe déjà et n'est pas vide", dest.display()));
        }
        let parent = dest.parent().ok_or("destination invalide")?.to_path_buf();
        std::fs::create_dir_all(&parent).map_err(|e| format!("création du dossier impossible : {e}"))?;
        ui::info("Clonage en cours (peut durer plusieurs minutes selon la taille du projet)…");
        let dest_s = dest.to_string_lossy().to_string();
        self.git.out(&parent, &["clone", "--", &url, &dest_s], Kind::Net)?;
        // Vérification : dépôt valide, contenu présent.
        if !dest.join(".git").exists() || !self.git.is_repo(&dest) {
            return Err("le clonage a signalé un succès mais le dépôt local est introuvable".into());
        }
        let mut s = format!("Cloné dans {}\n", dest.display());
        match self.git.head(&dest) {
            Some(h) => s += &format!("  Branche {} — dernier commit {}\n", self.git.branch(&dest).unwrap_or_default(), short(&h)),
            None => s += "  Dépôt vide (aucun commit).\n",
        }
        s += &present_project(&dest);
        s += "  Aucun script du projet n'a été exécuté.";
        Ok(s)
    }

    // ---- initialisation, branches

    fn init_repo(&self, path: &Option<String>) -> Res<String> {
        let dir = self.resolve_dir(path)?;
        if !dir.exists() {
            std::fs::create_dir_all(&dir).map_err(|e| format!("création du dossier impossible : {e}"))?;
        }
        if self.git.is_repo(&dir) {
            let root = self.git.toplevel(&dir)?;
            return Ok(format!("Déjà sous Git (racine : {}) — rien à initialiser.", root.display()));
        }
        let o = self.git.run(&dir, &["init", "-b", "main"], Kind::Local)?;
        if !o.ok() {
            self.git.out(&dir, &["init"], Kind::Local)?;
        }
        if !dir.join(".git").exists() || !self.git.is_repo(&dir) {
            return Err("l'initialisation a échoué : dépôt introuvable".into());
        }
        Ok(format!("Dépôt Git initialisé dans {}", dir.display()))
    }

    fn check_branch_name(&self, repo: &Path, name: &str) -> Res<()> {
        if name.starts_with('-') || !self.git.run(repo, &["check-ref-format", "--branch", name], Kind::Local)?.ok() {
            return Err(format!("nom de branche invalide pour Git : « {name} »"));
        }
        Ok(())
    }

    fn branch_exists(&self, repo: &Path, name: &str) -> bool {
        self.git.run(repo, &["rev-parse", "--verify", "-q", &format!("refs/heads/{name}")], Kind::Local).map(|o| o.ok()).unwrap_or(false)
    }

    fn create_branch(&self, path: &Option<String>, name: &str, switch_to: bool) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        self.check_branch_name(&repo, name)?;
        if self.branch_exists(&repo, name) {
            return Err(format!("la branche « {name} » existe déjà"));
        }
        let has_commit = self.git.head(&repo).is_some();
        if switch_to {
            let o = self.git.run(&repo, &["switch", "-c", name], Kind::Local)?;
            if !o.ok() {
                self.git.out(&repo, &["checkout", "-b", name], Kind::Local)?;
            }
            if self.git.branch(&repo).as_deref() != Some(name) {
                return Err("la branche a été créée mais n'est pas active".into());
            }
            Ok(format!("Branche « {name} » créée et active."))
        } else {
            if !has_commit {
                return Err("impossible de créer une branche sans commit".into());
            }
            self.git.out(&repo, &["branch", name], Kind::Local)?;
            if !self.branch_exists(&repo, name) {
                return Err("la branche n'a pas été créée".into());
            }
            Ok(format!("Branche « {name} » créée (branche active inchangée)."))
        }
    }

    fn switch_branch(&self, path: &Option<String>, name: &str) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        self.check_branch_name(&repo, name)?;
        if !self.branch_exists(&repo, name) {
            let list = self.git.line(&repo, &["branch", "--format=%(refname:short)"]).unwrap_or_default();
            return Err(format!("la branche « {name} » n'existe pas. Branches : {}", list.replace('\n', ", ")));
        }
        self.git.out(&repo, &["switch", name], Kind::Local)?;
        if self.git.branch(&repo).as_deref() != Some(name) {
            return Err("le changement de branche n'a pas été vérifié".into());
        }
        Ok(format!("Vous êtes sur la branche « {name} »."))
    }

    // ---- indexation et commit

    fn stage(&self, path: &Option<String>, files: &[String]) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let entries = self.git.status(&repo)?;
        let wanted: Vec<&Entry> = if files.is_empty() {
            entries.iter().filter(|e| e.untracked() || e.unstaged()).collect()
        } else {
            let fl: Vec<String> = files.iter().map(|f| norm_rel(f)).collect();
            entries.iter().filter(|e| fl.iter().any(|f| *f == e.path || e.path.starts_with(&format!("{f}/")))).collect()
        };
        if wanted.is_empty() {
            let n = entries.iter().filter(|e| e.staged()).count();
            return Ok(format!("Rien de nouveau à préparer ({n} fichier(s) déjà préparé(s))."));
        }
        let mut ok_paths: Vec<String> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        let mut scanned = 0;
        for e in wanted {
            if e.conflicted() {
                skipped.push((e.path.clone(), "conflit non résolu".into()));
                continue;
            }
            let abs = repo.join(&e.path);
            if abs.exists() {
                scanned += 1;
                if let Some(r) = security::check_file(&abs, &e.path, self.cfg.max_file_size_mb, scanned <= 3000) {
                    skipped.push((e.path.clone(), r));
                    continue;
                }
            }
            ok_paths.push(e.path.clone());
        }
        for chunk in ok_paths.chunks(100) {
            let mut args: Vec<&str> = vec!["add", "-A", "--"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            self.git.out(&repo, &args, Kind::Local)?;
        }
        // Vérification : chaque fichier retenu est bien indexé.
        let after = self.git.status(&repo)?;
        let missing: Vec<&String> = ok_paths.iter().filter(|p| after.iter().any(|e| &e.path == *p && (e.untracked() || e.unstaged() && !e.staged()))).collect();
        if !missing.is_empty() {
            return Err(format!("{} fichier(s) n'ont pas pu être préparés, par exemple : {}", missing.len(), missing[0]));
        }
        let mut s = format!("{} fichier(s) préparé(s) : {}", ok_paths.len(), list_names(ok_paths.iter().map(|s| s.as_str()), 8));
        if !skipped.is_empty() {
            s += &format!("\n  {} fichier(s) EXCLU(S) par sécurité :", skipped.len());
            for (p, r) in skipped.iter().take(10) {
                s += &format!("\n    - {p} ({r})");
            }
            s += "\n  Dites « complète le gitignore » pour qu'ils ne soient plus proposés.";
        }
        Ok(s)
    }

    fn staged_names(&self, repo: &Path) -> Res<Vec<String>> {
        let raw = self.git.out(repo, &["diff", "--cached", "--name-only", "-z"], Kind::Local)?;
        Ok(raw.split('\0').filter(|s| !s.is_empty()).map(|s| s.to_string()).collect())
    }

    fn commit(&self, path: &Option<String>, message: &str) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let staged = self.staged_names(&repo)?;
        if staged.is_empty() {
            return Ok("Rien à enregistrer : aucune modification préparée.".into());
        }
        let mut found = Vec::new();
        for (i, f) in staged.iter().enumerate() {
            if let Some(r) = security::check_file(&repo.join(f), f, self.cfg.max_file_size_mb, i < 3000) {
                found.push(format!("{f} ({r})"));
            }
        }
        if !found.is_empty() {
            return Err(format!(
                "enregistrement refusé : fichiers sensibles dans l'index :\n    - {}\n  Retirez-les (git restore --staged <fichier>) et ajoutez-les à .gitignore.",
                found.join("\n    - ")
            ));
        }
        for key in ["user.name", "user.email"] {
            if self.git.line(&repo, &["config", key]).unwrap_or_default().is_empty() {
                return Err("please tell me who you are : identité Git absente".into());
            }
        }
        let before = self.git.head(&repo);
        let msg = if message.trim().is_empty() {
            let names: Vec<&str> = staged.iter().map(|s| s.rsplit('/').next().unwrap_or(s)).collect();
            let head = names.iter().take(3).cloned().collect::<Vec<_>>().join(", ");
            let extra = if names.len() > 3 { format!(" (+{} autres)", names.len() - 3) } else { String::new() };
            let fallback = format!("{} : {head}{extra}", if before.is_none() { "Premier commit" } else { "Mise à jour" });
            self.ai_commit_message(&repo, staged.len()).unwrap_or(fallback)
        } else {
            message.trim().to_string()
        };
        security::valid_commit_message(&msg)?;
        self.git.out(&repo, &["commit", "-m", &msg], Kind::Local)?;
        let after = self.git.head(&repo).ok_or("le commit n'a pas été créé")?;
        if Some(&after) == before.as_ref() {
            return Err("le commit n'a pas été créé (HEAD inchangé)".into());
        }
        let subj = self.git.line(&repo, &["log", "-1", "--format=%s"]).unwrap_or_default();
        if subj != msg.lines().next().unwrap_or("").trim() {
            return Err("le commit créé ne correspond pas au message attendu".into());
        }
        Ok(format!("Commit {} créé : « {subj} » ({} fichier(s)).", short(&after), staged.len()))
    }

    /// Message de commit rédigé par un modèle LOCAL à partir de la nature du changement et du NOM des fichiers
    /// (jamais de leur contenu, jamais vers un service cloud). Toute anomalie : message standard.
    fn ai_commit_message(&self, repo: &Path, total: usize) -> Option<String> {
        if !self.cfg.ai_commit_messages || self.force_rules {
            return None;
        }
        let l = self.llm.as_ref().filter(|l| l.is_local())?;
        let ns = self.git.out(repo, &["diff", "--cached", "--name-status", "--no-color"], Kind::Local).ok()?;
        let safe = |n: &str| -> String {
            let t: String = n.chars().map(|c| if c.is_alphanumeric() || "._-/ ".contains(c) { c } else { '_' }).collect();
            truncate(t.trim(), 60)
        };
        let mut lines = Vec::new();
        for l in ns.lines().take(40) {
            let mut it = l.split('\t');
            let st = match it.next().and_then(|s| s.chars().next()) {
                Some('A') => "ajouté",
                Some('M') => "modifié",
                Some('D') => "supprimé",
                Some('R') => "renommé",
                _ => "changé",
            };
            if let Some(name) = it.last() {
                lines.push(format!("- {st} : {}", safe(name)));
            }
        }
        if lines.is_empty() {
            return None;
        }
        let sys = if self.cfg.commit_lang == "en" {
            "You write Git commit messages. Reply ONLY with the message: a single line, 72 characters max, in English, style \"type: description\" (feat, fix, docs, refactor, chore, test), no quotes, no final period. You only receive the list of changed files (kind of change and name), never their content."
        } else {
            "Tu écris des messages de commit Git. Réponds UNIQUEMENT par le message : une seule ligne, 72 caractères maximum, en français, au style « type: description » (feat, fix, docs, refactor, chore, test), sans guillemets ni point final. Tu reçois seulement la liste des fichiers modifiés (nature du changement et nom), jamais leur contenu."
        };
        let user = format!("Fichiers ({total} au total) :\n{}", lines.join("\n"));
        let raw = l.chat(sys, &user, None, 60).ok()?;
        let m = clean_commit_message(&raw)?;
        security::valid_commit_message(&m).ok()?;
        ui::info(format!("Message proposé par l'IA : « {m} »"));
        Some(m)
    }

    // ---- distant

    fn pull(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        if self.git.upstream(&repo).is_none() {
            return Err("la branche courante n'est liée à aucune branche distante : rien à récupérer (envoyez-la d'abord ou configurez un dépôt distant)".into());
        }
        if self.git.status(&repo)?.iter().any(|e| e.conflicted()) {
            return Err("des conflits non résolus existent : résolvez-les avant de mettre à jour".into());
        }
        let before = self.git.head(&repo);
        self.git.out(&repo, &["pull", "--ff-only"], Kind::Net)?;
        let after = self.git.head(&repo);
        if before == after {
            return Ok("Déjà à jour.".into());
        }
        let n = match &before {
            Some(b) => self.git.line(&repo, &["rev-list", "--count", &format!("{b}..HEAD")]).unwrap_or_default(),
            None => "?".into(),
        };
        Ok(format!("Mis à jour : {n} nouveau(x) commit(s), maintenant au commit {}.", short(&after.unwrap_or_default())))
    }

    fn push(&self, path: &Option<String>, remote: &Option<String>, branch: &Option<String>, force: bool) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let (r, url) = self.remote_for(&repo, remote)?;
        let b = self.target_branch(&repo, branch)?;
        self.local_hash(&repo, &b)?;
        // Contrôle des fichiers qui partiraient réellement (commits absents du distant).
        let refname = format!("refs/heads/{b}");
        let remotes_arg = format!("--remotes={r}");
        let count = self.git.line(&repo, &["rev-list", "--count", &refname, "--not", &remotes_arg]).unwrap_or_else(|_| "?".into());
        let names = self.git.out(&repo, &["log", "--name-only", "--pretty=format:", &refname, "--not", &remotes_arg], Kind::Local)?;
        let files: BTreeSet<&str> = names.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
        let bad: Vec<String> = files
            .iter()
            .filter_map(|f| security::sensitive_name(f).map(|r| format!("{f} ({r})")))
            .collect();
        if !bad.is_empty() {
            return Err(format!(
                "envoi bloqué : des fichiers sensibles figurent dans les commits à envoyer :\n    - {}\n  Retirez-les de l'historique avant publication (GIT DRENN ne les enverra jamais).",
                bad.join("\n    - ")
            ));
        }
        ui::say(format!("  Destination : {r} → {url}\n  Branche : {b} · {count} commit(s) · {} fichier(s) concerné(s)", files.len()));
        if url.contains("github.com") && !self.gh.has_token() {
            ui::info("Aucun jeton GitHub configuré : Git utilisera son gestionnaire d'identifiants.");
        }
        if !self.ask("Confirmer l'envoi vers le dépôt distant ?") {
            return Err("envoi annulé par l'utilisateur".into());
        }
        let refspec = format!("{refname}:{refname}");
        let mut args: Vec<&str> = vec!["push"];
        if force {
            if !self.ask_typed("ENVOI FORCÉ : l'historique distant sera remplacé.", "FORCER") {
                return Err("envoi forcé annulé".into());
            }
            args.push("--force-with-lease");
        }
        if self.git.upstream(&repo).is_none() {
            args.push("-u");
        }
        args.extend([r.as_str(), refspec.as_str()]);
        self.git.out(&repo, &args, Kind::Net)?;
        self.verify_push(&repo, &r, &url, &b)
    }

    fn configure_remote(&self, path: &Option<String>, name: &Option<String>, url: &str) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let n = name.as_deref().unwrap_or("origin");
        security::valid_remote_name(n)?;
        security::valid_repo_url(url)?;
        let existing = self.git.remotes(&repo).into_iter().find(|r| r.0 == n);
        match existing {
            Some((_, u)) if u == url => return Ok(format!("Le dépôt distant « {n} » pointe déjà vers {url}.")),
            Some((_, old)) => {
                self.git.out(&repo, &["remote", "set-url", n, url], Kind::Local)?;
                if self.git.line(&repo, &["remote", "get-url", n])? != url {
                    return Err("la modification du dépôt distant n'a pas été vérifiée".into());
                }
                return Ok(format!("Dépôt distant « {n} » modifié : {old} → {url}"));
            }
            None => {
                self.git.out(&repo, &["remote", "add", n, url], Kind::Local)?;
            }
        }
        if self.git.line(&repo, &["remote", "get-url", n])? != url {
            return Err("l'ajout du dépôt distant n'a pas été vérifié".into());
        }
        Ok(format!("Dépôt distant « {n} » configuré → {url}"))
    }

    fn create_remote(&self, name: &str, private: bool, description: &Option<String>, path: &Option<String>) -> Res<String> {
        let repo = if path.is_some() { Some(self.resolve_repo(path)?) } else { self.resolve_repo(&None).ok() };
        if let Some(r) = &repo {
            if let Some((n, u)) = self.git.remotes(r).into_iter().find(|x| x.0 == "origin") {
                return Ok(format!("Un dépôt distant « {n} » existe déjà ({u}) : création ignorée."));
            }
        }
        let name = if name.trim().is_empty() {
            let base = repo.as_ref().and_then(|r| r.file_name()).map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
            let n: String = base.chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect();
            security::valid_repo_name(&n).map_err(|_| "précisez le nom du dépôt à créer".to_string())?;
            n
        } else {
            name.trim().to_string()
        };
        if !self.gh.has_token() {
            return Err("jeton GitHub requis pour créer un dépôt : lancez `drenn auth login` ou définissez GITHUB_TOKEN".into());
        }
        let login = self.gh.user_login()?;
        let slug = format!("{login}/{name}");
        let info = match self.gh.repo(&slug)? {
            Some(existing) => {
                ui::warn(format!("Le dépôt GitHub « {slug} » existe déjà ({}).", if existing.private { "privé" } else { "PUBLIC" }));
                if !self.ask("Utiliser ce dépôt existant comme dépôt distant « origin » ?") {
                    return Err("un dépôt de ce nom existe déjà : choisissez un autre nom".into());
                }
                existing
            }
            None => {
                ui::say(format!("  Création du dépôt GitHub « {slug} » — visibilité : {}", if private { "PRIVÉE" } else { "PUBLIQUE (visible par tout le monde)" }));
                if !self.ask("Confirmer la création du dépôt distant ?") {
                    return Err("création annulée par l'utilisateur".into());
                }
                if !private && !self.ask_typed("PUBLICATION PUBLIQUE : le code envoyé sera visible par tous.", "PUBLIC") {
                    return Err("création publique annulée".into());
                }
                let created = self.gh.create_repo(&name, private, description.as_deref())?;
                let check = self.gh.repo(&created.full_name)?.ok_or("le dépôt créé est introuvable côté GitHub")?;
                if check.private != private {
                    return Err("la visibilité du dépôt créé ne correspond pas à la demande — vérifiez-le sur GitHub".into());
                }
                created
            }
        };
        let mut s = format!("Dépôt GitHub prêt : {} ({})", info.full_name, if info.private { "privé" } else { "public" });
        if let Some(r) = repo {
            self.git.out(&r, &["remote", "add", "origin", &info.clone_url], Kind::Local)?;
            if self.git.line(&r, &["remote", "get-url", "origin"])? != info.clone_url {
                return Err("le remote origin n'a pas été configuré correctement".into());
            }
            s += "\n  Remote « origin » configuré.";
        }
        Ok(s)
    }

    // ---- v0.2 : .gitignore, annulation de commit, fusion, étiquettes

    fn update_gitignore(&self, path: &Option<String>, patterns: &[String]) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let wanted: Vec<String> = if patterns.is_empty() { default_ignore_rules(&repo) } else { patterns.iter().map(|p| p.trim().to_string()).collect() };
        let file = repo.join(".gitignore");
        let existing = std::fs::read_to_string(&file).unwrap_or_default();
        let have: BTreeSet<&str> = existing.lines().map(str::trim).collect();
        let missing: Vec<&String> = wanted.iter().filter(|w| !have.contains(w.as_str())).collect();
        if missing.is_empty() {
            return Ok("Le .gitignore contient déjà toutes ces règles : rien à ajouter.".into());
        }
        let mut content = existing.clone();
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str("# Ajouté par GIT DRENN\n");
        for m in &missing {
            content.push_str(m);
            content.push('\n');
        }
        std::fs::write(&file, &content).map_err(|e| format!("écriture du .gitignore impossible : {e}"))?;
        // Vérification : relecture, et fichiers déjà suivis que la règle ne peut pas retirer.
        let back = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
        if missing.iter().any(|m| !back.lines().any(|l| l.trim() == m.as_str())) {
            return Err("le .gitignore n'a pas été écrit correctement".into());
        }
        let tracked = self.git.out(&repo, &["ls-files", "-i", "-c", "--exclude-standard", "-z"], Kind::Local).unwrap_or_default();
        let tracked: Vec<&str> = tracked.split('\0').filter(|s| !s.is_empty()).collect();
        let mut s = format!(".gitignore mis à jour : {} règle(s) ajoutée(s) ({}).", missing.len(), list_names(missing.iter().map(|m| m.as_str()), 8));
        if !tracked.is_empty() {
            s += &format!(
                "\n  Attention : {} fichier(s) déjà suivi(s) par Git restent suivis ({}). Pour les retirer sans les supprimer : git rm --cached <fichier>. S'ils contiennent des secrets, changez-les.",
                tracked.len(), list_names(tracked.iter().cloned(), 5)
            );
        }
        Ok(s)
    }

    fn undo_last_commit(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let head = self.git.head(&repo).ok_or("aucun commit à annuler")?;
        let parent = self
            .git
            .line(&repo, &["rev-parse", "--verify", "-q", "HEAD~1"])
            .map_err(|_| "c'est le tout premier commit : il ne peut pas être annulé de cette façon".to_string())?;
        if self.git.run(&repo, &["rev-parse", "--verify", "-q", "HEAD^2"], Kind::Local)?.ok() {
            return Err("le dernier commit est une fusion : annulation automatique refusée".into());
        }
        let pushed = self.git.line(&repo, &["branch", "-r", "--contains", &head]).unwrap_or_default();
        if !pushed.is_empty() {
            return Err(format!(
                "ce commit est déjà présent sur le dépôt distant ({}) : l'annuler obligerait à réécrire l'historique partagé. GIT DRENN ne le fait pas automatiquement.",
                pushed.lines().next().unwrap_or("").trim()
            ));
        }
        let subj = self.git.line(&repo, &["log", "-1", "--format=%s"]).unwrap_or_default();
        // Filet de sécurité : l'ancien commit reste référencé.
        let backup = format!("refs/drenn/backup/commit-{}", now_secs());
        self.git.out(&repo, &["update-ref", &backup, &head], Kind::Local)?;
        self.git.out(&repo, &["reset", "--soft", "HEAD~1"], Kind::Local)?;
        if self.git.head(&repo).as_deref() != Some(parent.as_str()) {
            return Err("l'annulation n'a pas été vérifiée : HEAD n'est pas revenu au commit précédent".into());
        }
        let n = self.staged_names(&repo)?.len();
        Ok(format!(
            "Commit {} « {} » annulé. Ses {n} fichier(s) restent préparés : rien n'est perdu.\n  Pour le retrouver tel quel : git reset --soft {}",
            short(&head), subj, short(&head)
        ))
    }

    fn merge_branch(&self, path: &Option<String>, name: &str, keep_conflicts: bool) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        self.check_branch_name(&repo, name)?;
        if !self.branch_exists(&repo, name) {
            return Err(format!("la branche « {name} » n'existe pas"));
        }
        let current = self.git.branch(&repo).ok_or("aucune branche courante : fusion impossible")?;
        if current == name {
            return Err("on ne peut pas fusionner une branche dans elle-même".into());
        }
        let entries = self.git.status(&repo)?;
        if entries.iter().any(|e| !e.untracked()) {
            return Err("des modifications non enregistrées existent : enregistrez-les (ou abandonnez-les) avant de fusionner".into());
        }
        let before = self.git.head(&repo).ok_or("aucun commit sur la branche courante")?;
        let o = self.git.run(&repo, &["merge", "--no-edit", name], Kind::Local)?;
        if !o.ok() {
            let conflicts: Vec<String> = self.git.status(&repo)?.into_iter().filter(|e| e.conflicted()).map(|e| e.path).collect();
            if !conflicts.is_empty() && keep_conflicts {
                return Ok(format!(
                    "Fusion de « {name} » en conflit sur {} fichier(s) : {}.\n  La fusion est EN COURS. Dites « montre les conflits », puis « résous les conflits en gardant ma version » (ou « la version distante »), puis « termine la fusion » — ou « annule la fusion ».",
                    conflicts.len(), list_names(conflicts.iter().map(|s| s.as_str()), 8)
                ));
            }
            if !conflicts.is_empty() {
                let _ = self.git.run(&repo, &["merge", "--abort"], Kind::Local);
                let restored = self.git.head(&repo).as_deref() == Some(before.as_str()) && self.git.status(&repo)?.iter().all(|e| !e.conflicted());
                return Err(format!(
                    "conflit de fusion dans : {}. La fusion a été annulée automatiquement{} — résolvez ces fichiers à la main ou fusionnez dans l'autre sens.",
                    list_names(conflicts.iter().map(|s| s.as_str()), 8),
                    if restored { " et votre branche est intacte" } else { " MAIS l'état n'a pas pu être vérifié : vérifiez avec « état du projet »" }
                ));
            }
            return Err(o.err_text());
        }
        if !self.git.run(&repo, &["merge-base", "--is-ancestor", name, "HEAD"], Kind::Local)?.ok() {
            return Err("la fusion n'a pas été vérifiée : la branche n'est pas incluse dans HEAD".into());
        }
        let after = self.git.head(&repo).unwrap_or_default();
        Ok(if after == before {
            format!("« {name} » était déjà intégrée dans « {current} » : rien à fusionner.")
        } else {
            format!("« {name} » fusionnée dans « {current} » (nouveau commit {}).", short(&after))
        })
    }

    fn create_tag(&self, path: &Option<String>, name: &str, message: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let tag_ref = format!("refs/tags/{name}");
        if !self.git.run(&repo, &["check-ref-format", &tag_ref], Kind::Local)?.ok() {
            return Err(format!("nom d'étiquette invalide : « {name} »"));
        }
        let head = self.git.head(&repo).ok_or("aucun commit à étiqueter")?;
        if self.git.run(&repo, &["rev-parse", "--verify", "-q", &tag_ref], Kind::Local)?.ok() {
            return Err(format!("l'étiquette « {name} » existe déjà"));
        }
        let msg = match message.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => {
                security::valid_commit_message(m)?;
                m.to_string()
            }
            None => format!("Version {name}"),
        };
        self.git.out(&repo, &["tag", "-a", name, "-m", &msg], Kind::Local)?;
        let tagged = self.git.line(&repo, &["rev-parse", &format!("{tag_ref}^{{commit}}")])?;
        if tagged != head {
            return Err("l'étiquette ne pointe pas sur le dernier commit".into());
        }
        Ok(format!("Étiquette « {name} » créée sur le commit {} (locale : elle n'est pas envoyée au dépôt distant).", short(&head)))
    }

    fn push_tag(&self, path: &Option<String>, name: &str, remote: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let (r, url) = self.remote_for(&repo, remote)?;
        let tag_ref = format!("refs/tags/{name}");
        let local = self
            .git
            .line(&repo, &["rev-parse", "--verify", "-q", &tag_ref])
            .map_err(|_| format!("l'étiquette « {name} » n'existe pas localement (créez-la d'abord)"))?;
        let remote_hash = |g: &Git| -> Res<Option<String>> {
            let o = g.out(&repo, &["ls-remote", "--tags", &r, &tag_ref], Kind::Net)?;
            Ok(o.lines().next().and_then(|l| l.split_whitespace().next()).map(|s| s.to_string()))
        };
        if let Some(h) = remote_hash(&self.git)? {
            return if h == local {
                Ok(format!("L'étiquette « {name} » est déjà sur « {r} » : rien à envoyer."))
            } else {
                Err(format!("« {r} » possède déjà une étiquette « {name} » différente : GIT DRENN ne la remplace jamais."))
            };
        }
        ui::say(format!("  Destination : {r} → {url}\n  Étiquette : {name}"));
        if !self.ask("Confirmer l'envoi de l'étiquette ?") {
            return Err("envoi annulé par l'utilisateur".into());
        }
        let spec = format!("{tag_ref}:{tag_ref}");
        self.git.out(&repo, &["push", &r, &spec], Kind::Net)?;
        match remote_hash(&self.git)? {
            Some(h) if h == local => Ok(format!("Vérifié : l'étiquette « {name} » est présente sur « {r} ».")),
            _ => Err("l'envoi n'a pas pu être vérifié : l'étiquette est absente ou différente côté distant".into()),
        }
    }

    fn stash_count(&self, repo: &Path) -> usize {
        self.git.line(repo, &["stash", "list"]).map(|s| s.lines().count()).unwrap_or(0)
    }

    fn stash(&self, path: &Option<String>, message: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let entries = self.git.status(&repo)?;
        if entries.is_empty() {
            return Ok("Aucune modification à mettre de côté.".into());
        }
        if self.git.head(&repo).is_none() {
            return Err("aucun commit : impossible de mettre de côté (enregistrez d'abord un premier commit)".into());
        }
        let before = self.stash_count(&repo);
        let msg = message.as_deref().map(str::trim).filter(|m| !m.is_empty()).map(|m| m.to_string()).unwrap_or_else(|| format!("GIT DRENN {}", now_secs()));
        self.git.out(&repo, &["stash", "push", "-u", "-m", &msg], Kind::Local)?;
        let left = self.git.status(&repo)?;
        if self.stash_count(&repo) != before + 1 || !left.is_empty() {
            return Err("la mise de côté n'a pas été vérifiée : le dossier n'est pas propre ou la réserve n'a pas été créée".into());
        }
        Ok(format!("{} fichier(s) mis de côté (« {msg} »). Dossier propre ; dites « remets mes modifications mises de côté » pour les retrouver.", entries.len()))
    }

    fn restore_stash(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        if self.stash_count(&repo) == 0 {
            return Err("aucune modification mise de côté".into());
        }
        if self.git.status(&repo)?.iter().any(|e| !e.untracked()) {
            return Err("des modifications en cours existent : enregistrez-les ou mettez-les de côté avant de remettre les anciennes".into());
        }
        let before = self.stash_count(&repo);
        let o = self.git.run(&repo, &["stash", "pop"], Kind::Local)?;
        if !o.ok() {
            if self.git.status(&repo)?.iter().any(|e| e.conflicted()) {
                // L'arbre suivi était propre avant : revenir à HEAD ne perd rien ; la réserve est conservée par Git.
                let _ = self.git.run(&repo, &["reset", "--hard", "HEAD"], Kind::Local);
                return Err("conflit en remettant les modifications : opération annulée, vos modifications restent en sécurité dans la réserve (git stash list)".into());
            }
            return Err(o.err_text());
        }
        if self.stash_count(&repo) + 1 != before {
            return Err("la remise en place n'a pas été vérifiée".into());
        }
        Ok(format!("Modifications remises en place ({} fichier(s) concerné(s)).", self.git.status(&repo)?.len()))
    }

    // ---- v0.3 : conflits de fusion

    fn merging(&self, repo: &Path) -> bool {
        self.git.run(repo, &["rev-parse", "-q", "--verify", "MERGE_HEAD"], Kind::Local).map(|o| o.ok()).unwrap_or(false)
    }

    fn inspect_conflicts(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let conflicts: Vec<Entry> = self.git.status(&repo)?.into_iter().filter(|e| e.conflicted()).collect();
        if conflicts.is_empty() {
            return Ok(if self.merging(&repo) {
                "Aucun conflit restant : la fusion est en cours, dites « termine la fusion ».".into()
            } else {
                "Aucun conflit.".into()
            });
        }
        let mut s = format!("{} fichier(s) en conflit{} :\n", conflicts.len(), if self.merging(&repo) { " (fusion en cours)" } else { "" });
        for e in &conflicts {
            let what = match (e.x, e.y) {
                ('U', 'U') => "modifié des deux côtés".to_string(),
                ('A', 'A') => "ajouté des deux côtés".to_string(),
                ('D', 'U') => "supprimé sur la branche courante, modifié sur l'autre".to_string(),
                ('U', 'D') => "modifié sur la branche courante, supprimé sur l'autre".to_string(),
                ('A', 'U') => "ajouté sur la branche courante seulement".to_string(),
                ('U', 'A') => "ajouté sur l'autre branche seulement".to_string(),
                ('D', 'D') => "supprimé des deux côtés".to_string(),
                _ => "conflit".to_string(),
            };
            let hunks = std::fs::read_to_string(repo.join(&e.path)).map(|t| t.lines().filter(|l| l.starts_with("<<<<<<<")).count()).unwrap_or(0);
            s += &format!("  - {} : {what}{}\n", e.path, if hunks > 0 { format!(" ({hunks} zone(s) à arbitrer)") } else { String::new() });
        }
        s += "  Choisissez : « résous les conflits un par un » (zone par zone, avec vous), « … en gardant ma version » (branche courante), « … la version distante » (branche fusionnée), « … en gardant les deux », ou « annule la fusion ».";
        Ok(s)
    }

    fn resolve_conflicts(&self, path: &Option<String>, files: &[String], strategy: &str) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        let fl: Vec<String> = files.iter().map(|f| norm_rel(f)).collect();
        let targets: Vec<Entry> = self
            .git
            .status(&repo)?
            .into_iter()
            .filter(|e| e.conflicted() && (fl.is_empty() || fl.iter().any(|f| *f == e.path)))
            .collect();
        if targets.is_empty() {
            return Err("aucun fichier en conflit correspondant".into());
        }
        if strategy == "ask" || strategy == "both" {
            return self.resolve_hunks(&repo, &targets, strategy);
        }
        let side = if strategy == "ours" { "ours" } else { "theirs" };
        let label = if side == "ours" { "de la branche COURANTE" } else { "de la branche FUSIONNÉE" };
        ui::say(format!("  {} fichier(s) : {}\n  Pour chacun, la version {label} sera gardée ; l'autre sera écartée.", targets.len(), list_names(targets.iter().map(|e| e.path.as_str()), 8)));
        if !self.ask("Confirmer cette résolution ?") {
            return Err("résolution annulée par l'utilisateur".into());
        }
        // Sauvegarde des fichiers (avec marqueurs de conflit) dans .git, hors de tout commit.
        let bdir = repo.join(".git").join("drenn-backup").join(now_secs().to_string());
        std::fs::create_dir_all(&bdir).map_err(|e| format!("sauvegarde impossible, résolution annulée : {e}"))?;
        for e in &targets {
            let src = repo.join(&e.path);
            if src.is_file() {
                let flat = e.path.replace(['/', '\\'], "__");
                std::fs::copy(&src, bdir.join(flat)).map_err(|er| format!("sauvegarde impossible, résolution annulée : {er}"))?;
            }
        }
        for e in &targets {
            let side_exists = match (e.x, e.y, side) {
                ('U', 'U', _) | ('A', 'A', _) => true,
                ('D', 'U', "ours") | ('U', 'A', "ours") | ('D', 'D', _) => false,
                ('U', 'D', "theirs") | ('A', 'U', "theirs") => false,
                _ => true,
            };
            if side_exists {
                let flag = format!("--{side}");
                self.git.out(&repo, &["checkout", &flag, "--", &e.path], Kind::Local)?;
                self.git.out(&repo, &["add", "--", &e.path], Kind::Local)?;
            } else {
                self.git.out(&repo, &["rm", "-q", "--", &e.path], Kind::Local)?;
            }
        }
        let still: Vec<String> = self.git.status(&repo)?.into_iter().filter(|e| e.conflicted() && targets.iter().any(|t| t.path == e.path)).map(|e| e.path).collect();
        if !still.is_empty() {
            return Err(format!("{} fichier(s) restent en conflit : {}", still.len(), list_names(still.iter().map(|s| s.as_str()), 5)));
        }
        let left = self.git.status(&repo)?.iter().filter(|e| e.conflicted()).count();
        Ok(format!(
            "{} conflit(s) résolu(s) en gardant la version {label}. Sauvegarde des fichiers d'origine : {}\n  {}",
            targets.len(),
            bdir.display(),
            if left > 0 { format!("Il reste {left} fichier(s) en conflit.") } else { "Plus aucun conflit : dites « termine la fusion ».".to_string() }
        ))
    }

    /// Résolution zone par zone (« ask », avec l'utilisateur) ou en gardant les deux versions (« both »).
    /// Le contenu des zones n'est montré qu'à l'utilisateur : il n'est jamais envoyé à un modèle.
    fn resolve_hunks(&self, repo: &Path, targets: &[Entry], strategy: &str) -> Res<String> {
        if strategy == "both" && !self.ask(&format!("Garder les DEUX versions (la vôtre puis l'autre) dans chaque zone en conflit de {} fichier(s) ?", targets.len())) {
            return Err("résolution annulée par l'utilisateur".into());
        }
        let bdir = repo.join(".git").join("drenn-backup").join(now_secs().to_string());
        let mut done: Vec<(String, usize)> = Vec::new();
        let mut skipped: Vec<String> = Vec::new();
        let mut cancelled = false;
        let label = |l: &str| if l.is_empty() { String::new() } else { format!(" : {}", clean(&truncate(l, 40))) };
        'files: for e in targets {
            if !matches!((e.x, e.y), ('U', 'U') | ('A', 'A')) {
                skipped.push(format!("{} (supprimé d'un côté : utilisez « garder ma version » ou « la version distante »)", e.path));
                continue;
            }
            let abs = repo.join(&e.path);
            match std::fs::symlink_metadata(&abs) {
                Ok(m) if m.is_file() && m.len() <= 5_000_000 => {}
                _ => {
                    skipped.push(format!("{} (fichier non ordinaire, introuvable ou trop gros)", e.path));
                    continue;
                }
            }
            let data = std::fs::read(&abs).map_err(|er| format!("lecture de {} impossible : {er}", e.path))?;
            if data.contains(&0) {
                skipped.push(format!("{} (fichier binaire : choisissez une version entière)", e.path));
                continue;
            }
            let segs = match conflicts::parse(&data) {
                Ok(s) => s,
                Err(er) => {
                    skipped.push(format!("{} ({er})", e.path));
                    continue;
                }
            };
            let n = conflicts::count(&segs);
            let choices: Vec<Choice> = if n == 0 {
                if !self.ask(&format!("« {} » ne contient plus de marques de conflit. Le considérer comme résolu ?", clean(&e.path))) {
                    skipped.push(format!("{} (non validé)", e.path));
                    continue;
                }
                vec![]
            } else if strategy == "both" {
                vec![Choice::Both; n]
            } else {
                let mut ch = Vec::new();
                for (i, h) in conflicts::hunks(&segs).iter().enumerate() {
                    let q = format!(
                        "{} — zone {}/{}\n-- MA VERSION (branche courante{}) --\n{}\n-- VERSION FUSIONNÉE{} --\n{}\nQue garder pour cette zone ?",
                        clean(&e.path),
                        i + 1,
                        n,
                        label(&h.ours_label),
                        clean(&conflicts::preview(&h.ours, 15)),
                        label(&h.theirs_label),
                        clean(&conflicts::preview(&h.theirs, 15)),
                    );
                    let opts = vec![
                        "Garder MA version (branche courante)".to_string(),
                        "Garder la version FUSIONNÉE".to_string(),
                        "Garder les DEUX (la mienne puis l'autre)".to_string(),
                        "Passer ce fichier (le laisser en conflit)".to_string(),
                    ];
                    match self.pick(&q, &opts) {
                        Some(0) => ch.push(Choice::Ours),
                        Some(1) => ch.push(Choice::Theirs),
                        Some(2) => ch.push(Choice::Both),
                        Some(_) => {
                            skipped.push(format!("{} (laissé en conflit à votre demande)", e.path));
                            continue 'files;
                        }
                        None => {
                            cancelled = true;
                            skipped.push(format!("{} (annulé)", e.path));
                            break 'files;
                        }
                    }
                }
                ch
            };
            let out = conflicts::render(&segs, &choices)?;
            std::fs::create_dir_all(&bdir).map_err(|er| format!("sauvegarde impossible, résolution annulée : {er}"))?;
            std::fs::write(bdir.join(e.path.replace(['/', '\\'], "__")), &data).map_err(|er| format!("sauvegarde impossible, résolution annulée : {er}"))?;
            std::fs::write(&abs, &out).map_err(|er| format!("écriture de {} impossible : {er}", e.path))?;
            self.git.out(repo, &["add", "--", &e.path], Kind::Local)?;
            done.push((e.path.clone(), n));
        }
        let still: Vec<String> = self.git.status(repo)?.into_iter().filter(|e| e.conflicted() && done.iter().any(|d| d.0 == e.path)).map(|e| e.path).collect();
        if !still.is_empty() {
            return Err(format!("{} fichier(s) restent en conflit après résolution : {}", still.len(), list_names(still.iter().map(|s| s.as_str()), 5)));
        }
        if done.is_empty() {
            return Err(if cancelled { "résolution annulée : rien n'a été modifié".to_string() } else { format!("aucun fichier résolu :\n    - {}", skipped.join("\n    - ")) });
        }
        let left = self.git.status(repo)?.iter().filter(|e| e.conflicted()).count();
        let mut msg = format!(
            "{} fichier(s) résolu(s) ({} zone(s)) : {}. Fichiers d'origine sauvegardés dans {}",
            done.len(),
            done.iter().map(|d| d.1).sum::<usize>(),
            list_names(done.iter().map(|d| d.0.as_str()), 6),
            bdir.display()
        );
        if !skipped.is_empty() {
            msg += &format!("\n  Non traité(s) :\n    - {}", skipped.join("\n    - "));
        }
        msg += &if left > 0 { format!("\n  Il reste {left} fichier(s) en conflit.") } else { "\n  Plus aucun conflit : dites « termine la fusion ».".to_string() };
        Ok(msg)
    }

    fn abort_merge(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        if !self.merging(&repo) {
            return Err("aucune fusion en cours".into());
        }
        self.git.out(&repo, &["merge", "--abort"], Kind::Local)?;
        if self.merging(&repo) || self.git.status(&repo)?.iter().any(|e| e.conflicted()) {
            return Err("l'annulation n'a pas été vérifiée : la fusion semble encore en cours".into());
        }
        Ok("Fusion annulée : votre branche est revenue exactement à son état d'avant.".into())
    }

    fn continue_merge(&self, path: &Option<String>) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        if !self.merging(&repo) {
            return Err("aucune fusion en cours".into());
        }
        let left: Vec<String> = self.git.status(&repo)?.into_iter().filter(|e| e.conflicted()).map(|e| e.path).collect();
        if !left.is_empty() {
            return Err(format!("{} fichier(s) sont encore en conflit : {}", left.len(), list_names(left.iter().map(|s| s.as_str()), 5)));
        }
        let staged = self.staged_names(&repo)?;
        let found: Vec<String> = staged
            .iter()
            .enumerate()
            .filter_map(|(i, f)| security::check_file(&repo.join(f), f, self.cfg.max_file_size_mb, i < 3000).map(|r| format!("{f} ({r})")))
            .collect();
        if !found.is_empty() {
            return Err(format!("fusion non terminée : fichiers sensibles dans l'index :\n    - {}", found.join("\n    - ")));
        }
        let before = self.git.head(&repo);
        self.git.out(&repo, &["commit", "--no-edit"], Kind::Local)?;
        let after = self.git.head(&repo);
        if after == before || self.merging(&repo) {
            return Err("le commit de fusion n'a pas été créé".into());
        }
        Ok(format!("Fusion terminée : commit {}.", short(&after.unwrap_or_default())))
    }

    // ---- abandon de modifications (destructif, avec sauvegarde de secours)

    fn discard(&self, path: &Option<String>, files: &[String]) -> Res<String> {
        let repo = self.resolve_repo(path)?;
        if self.git.head(&repo).is_none() {
            return Err("aucun commit : il n'y a pas d'état précédent vers lequel revenir".into());
        }
        let entries = self.git.status(&repo)?;
        let fl: Vec<String> = files.iter().map(|f| norm_rel(f)).collect();
        let targets: Vec<&Entry> = entries
            .iter()
            .filter(|e| !e.untracked() && !e.conflicted() && (fl.is_empty() || fl.iter().any(|f| *f == e.path || e.path.starts_with(&format!("{f}/")))))
            .collect();
        let untracked = entries.iter().filter(|e| e.untracked()).count();
        if targets.is_empty() {
            return Ok(format!("Aucune modification suivie à abandonner ({untracked} nouveau(x) fichier(s) non suivi(s) conservé(s))."));
        }
        ui::say(format!("  Modifications qui seront ABANDONNÉES ({}) : {}", targets.len(), list_names(targets.iter().map(|e| e.path.as_str()), 10)));
        ui::say(format!("  {untracked} fichier(s) non suivi(s) ne seront PAS touchés."));
        if !self.ask_typed("Cette action remplace vos modifications par la dernière version enregistrée.", "ABANDONNER") {
            return Err("abandon annulé par l'utilisateur".into());
        }
        // Sauvegarde de secours : objet stash référencé, récupérable avec `git stash apply <sha>`.
        let sha = self
            .git
            .line(&repo, &["stash", "create"])
            .map_err(|e| format!("sauvegarde de secours impossible, abandon annulé : {e}"))?;
        let backup = if sha.is_empty() {
            None
        } else {
            let refname = format!("refs/drenn/backup/{}", now_secs());
            self.git.out(&repo, &["update-ref", &refname, &sha], Kind::Local)?;
            Some(sha)
        };
        let (added, others): (Vec<&Entry>, Vec<&Entry>) = targets.iter().partition(|e| e.x == 'A');
        for chunk in added.chunks(100) {
            let mut args: Vec<&str> = vec!["restore", "--staged", "--"];
            args.extend(chunk.iter().map(|e| e.path.as_str()));
            self.git.out(&repo, &args, Kind::Local)?;
        }
        for chunk in others.chunks(100) {
            let mut args: Vec<&str> = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
            args.extend(chunk.iter().map(|e| e.path.as_str()));
            self.git.out(&repo, &args, Kind::Local)?;
        }
        let after = self.git.status(&repo)?;
        let left: Vec<&Entry> = others.iter().filter(|t| after.iter().any(|e| e.path == t.path && !e.untracked())).cloned().collect();
        if !left.is_empty() {
            return Err(format!("{} fichier(s) n'ont pas pu être restaurés, par exemple : {}", left.len(), left[0].path));
        }
        let mut s = format!("{} fichier(s) restauré(s) à leur dernière version enregistrée ; {untracked} nouveau(x) fichier(s) conservé(s).", others.len());
        if !added.is_empty() {
            s += &format!(" {} fichier(s) ajouté(s) retiré(s) de l'index (conservés sur le disque).", added.len());
        }
        if let Some(b) = backup {
            s += &format!("\n  Sauvegarde de secours : `git stash apply {}` permet de tout récupérer.", short(&b));
        }
        Ok(s)
    }
}

/// Règles .gitignore proposées selon le type de projet détecté (toujours : secrets et fichiers système).
fn default_ignore_rules(repo: &Path) -> Vec<String> {
    let mut r: Vec<&str> = vec![".env", ".env.*", "!.env.example", "*.pem", "*.key", "id_rsa", "id_ed25519", ".DS_Store", "Thumbs.db"];
    let has = |f: &str| repo.join(f).exists();
    if has("package.json") {
        r.extend(["node_modules/", "dist/", "npm-debug.log*"]);
    }
    if has("composer.json") || has("index.php") {
        r.extend(["vendor/", "*.log"]);
    }
    if has("Cargo.toml") {
        r.push("target/");
    }
    if has("requirements.txt") || has("pyproject.toml") {
        r.extend(["__pycache__/", "*.pyc", ".venv/"]);
    }
    if has("pom.xml") {
        r.push("target/");
    }
    if has("go.mod") {
        r.push("bin/");
    }
    let mut seen = BTreeSet::new();
    r.into_iter().filter(|x| seen.insert(*x)).map(|s| s.to_string()).collect()
}

// ---------------------------------------------------------------- présentation

fn pathlike(s: &str) -> bool {
    s.contains('/') || s.contains('\\') || s.contains(':')
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

fn list_names<'a>(it: impl Iterator<Item = &'a str>, max: usize) -> String {
    let v: Vec<&str> = it.collect();
    let shown = v.iter().take(max).cloned().collect::<Vec<_>>().join(", ");
    if v.len() > max { format!("{shown} … (+{})", v.len() - max) } else { shown }
}

fn fmt_repo(r: &crate::github::RepoInfo) -> String {
    let d = if r.description.is_empty() { String::new() } else { format!(" — {}", truncate(&r.description, 80)) };
    format!("  {}{} ({} étoile(s)){}", r.full_name, if r.private { " [privé]" } else { "" }, r.stars, d)
}

pub fn summarize_status(entries: &[Entry]) -> String {
    if entries.is_empty() {
        return "  Arbre de travail propre : aucune modification.\n".into();
    }
    let staged: Vec<&Entry> = entries.iter().filter(|e| e.staged() && !e.conflicted()).collect();
    let modified: Vec<&Entry> = entries.iter().filter(|e| e.unstaged() && !e.conflicted()).collect();
    let untracked: Vec<&Entry> = entries.iter().filter(|e| e.untracked()).collect();
    let conflicts: Vec<&Entry> = entries.iter().filter(|e| e.conflicted()).collect();
    let mut s = String::new();
    let mut line = |label: &str, v: &Vec<&Entry>| {
        if !v.is_empty() {
            s += &format!("  {label} ({}) : {}\n", v.len(), list_names(v.iter().map(|e| e.path.as_str()), 8));
        }
    };
    line("Préparés pour le commit", &staged);
    line("Modifiés, non préparés", &modified);
    line("Nouveaux, non suivis", &untracked);
    line("EN CONFLIT", &conflicts);
    s
}

fn present_project(dir: &Path) -> String {
    let Ok(rd) = std::fs::read_dir(dir) else { return String::new() };
    let mut names: Vec<String> = rd
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if n == ".git" {
                return None;
            }
            Some(if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { format!("{n}/") } else { n })
        })
        .collect();
    names.sort();
    let has = |f: &str| names.iter().any(|n| n.eq_ignore_ascii_case(f));
    let mut kinds = Vec::new();
    for (f, k) in [("Cargo.toml", "Rust"), ("package.json", "Node.js"), ("composer.json", "PHP (Composer)"), ("requirements.txt", "Python"), ("pyproject.toml", "Python"), ("go.mod", "Go"), ("pom.xml", "Java (Maven)"), ("Gemfile", "Ruby"), ("index.php", "PHP")] {
        if has(f) && !kinds.contains(&k) {
            kinds.push(k);
        }
    }
    let mut s = String::new();
    if !kinds.is_empty() {
        s += &format!("  Type de projet détecté : {}\n", kinds.join(", "));
    }
    if names.iter().any(|n| n.to_lowercase().starts_with("readme")) {
        s += "  Documentation : README présent\n";
    }
    s += &format!("  Contenu : {}\n", list_names(names.iter().map(|s| s.as_str()), 15));
    s
}

pub fn show_history(n: usize) {
    let file = drenn_dir().join("history.jsonl");
    let Ok(txt) = std::fs::read_to_string(&file) else {
        ui::say("Aucun historique pour l'instant.");
        return;
    };
    let lines: Vec<&str> = txt.lines().collect();
    for l in lines.iter().rev().take(n).rev() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(l) {
            ui::say(format!("\n[{}] {}", v["ts"], v["goal"].as_str().unwrap_or("")));
            if let Some(steps) = v["steps"].as_array() {
                for s in steps {
                    ui::say(format!("  {} {}", if s["ok"].as_bool().unwrap_or(false) { "[OK]   " } else { "[ECHEC]" }, s["step"].as_str().unwrap_or("")));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(name: &str) -> (Agent, PathBuf) {
        let base = std::env::temp_dir().join(format!("drenn_t_{name}_{}", now_secs()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("proj")).unwrap();
        let cfg = Config { allowed_roots: vec![base.to_string_lossy().to_string()], ..Config::default() };
        let mut a = Agent::new(cfg, base.clone(), None).unwrap();
        a.test_yes = true;
        let g = |args: &[&str]| a.git.out(&base.join("proj"), args, Kind::Local);
        let _ = g(&["init", "-q"]);
        let _ = g(&["config", "user.name", "T"]);
        let _ = g(&["config", "user.email", "t@t.t"]);
        (a, base)
    }

    fn act(json: serde_json::Value) -> Action {
        serde_json::from_value(json).unwrap()
    }

    #[test]
    fn commit_exclut_et_bloque_les_secrets() {
        let (a, base) = setup("secrets");
        let p = base.join("proj");
        std::fs::write(p.join("a.txt"), "ok").unwrap();
        std::fs::write(p.join(".env"), "X=1").unwrap();
        let tok = format!("t = {}{}", "ghp_", "b".repeat(36));
        std::fs::write(p.join("conf.txt"), tok).unwrap();
        let r = a.exec(&act(serde_json::json!({"action":"stage_changes","path":"proj"}))).unwrap();
        assert!(r.contains("EXCLU"), "{r}");
        let r = a.exec(&act(serde_json::json!({"action":"create_commit","path":"proj","message":"init"}))).unwrap();
        assert!(r.contains("créé"), "{r}");
        let files = a.git.line(&p, &["ls-files"]).unwrap();
        assert_eq!(files.trim(), "a.txt");
        // Un secret forcé dans l'index est refusé au commit.
        a.git.out(&p, &["add", "-f", ".env"], Kind::Local).unwrap();
        let e = a.exec(&act(serde_json::json!({"action":"create_commit","path":"proj","message":"x"}))).unwrap_err();
        assert!(e.contains("sensibles"), "{e}");
    }

    #[test]
    fn push_bloque_sur_historique_sensible_puis_verifie() {
        let (a, base) = setup("push");
        let p = base.join("proj");
        let bare = base.join("r.git");
        a.git.out(&base, &["init", "-q", "--bare", bare.to_str().unwrap()], Kind::Local).unwrap();
        a.git.out(&p, &["remote", "add", "origin", bare.to_str().unwrap()], Kind::Local).unwrap();
        std::fs::write(p.join(".env"), "S=1").unwrap();
        a.git.out(&p, &["add", "-f", "."], Kind::Local).unwrap();
        a.git.out(&p, &["commit", "-qm", "oups"], Kind::Local).unwrap();
        let push = act(serde_json::json!({"action":"push_changes","path":"proj"}));
        assert!(a.exec(&push).unwrap_err().contains("bloqué"));
        // Historique propre : envoi + vérification.
        let (a, base) = setup("push2");
        let p = base.join("proj");
        let bare = base.join("r.git");
        a.git.out(&base, &["init", "-q", "--bare", bare.to_str().unwrap()], Kind::Local).unwrap();
        a.git.out(&p, &["remote", "add", "origin", bare.to_str().unwrap()], Kind::Local).unwrap();
        std::fs::write(p.join("a.txt"), "ok").unwrap();
        a.exec(&act(serde_json::json!({"action":"stage_changes","path":"proj"}))).unwrap();
        a.exec(&act(serde_json::json!({"action":"create_commit","path":"proj"}))).unwrap();
        let r = a.exec(&push).unwrap();
        assert!(r.contains("Vérifié"), "{r}");
    }

    #[test]
    fn perimetre_et_parametres_refuses() {
        let (a, _b) = setup("peri");
        assert!(a.exec(&act(serde_json::json!({"action":"inspect_status","path":"/etc"}))).is_err());
        assert!(a.exec(&act(serde_json::json!({"action":"create_branch","name":"--evil"}))).is_err());
        assert!(a.exec(&act(serde_json::json!({"action":"clone_repository","url":"ext::sh -c id"}))).is_err());
        assert!(a.exec(&act(serde_json::json!({"action":"stage_changes","path":"proj","files":["../../x"]}))).is_err());
    }

    #[test]
    fn abandon_conserve_les_fichiers_non_suivis() {
        let (a, base) = setup("discard");
        let p = base.join("proj");
        std::fs::write(p.join("a.txt"), "v1").unwrap();
        a.exec(&act(serde_json::json!({"action":"stage_changes","path":"proj"}))).unwrap();
        a.exec(&act(serde_json::json!({"action":"create_commit","path":"proj","message":"v1"}))).unwrap();
        std::fs::write(p.join("a.txt"), "v2").unwrap();
        std::fs::write(p.join("n.txt"), "nouveau").unwrap();
        let r = a.exec(&act(serde_json::json!({"action":"discard_changes","path":"proj"}))).unwrap();
        assert!(r.contains("stash apply"), "{r}");
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "v1");
        assert!(p.join("n.txt").exists());
    }
}

#[cfg(test)]
mod tests_v02 {
    use super::*;
    use serde_json::json;

    fn setup(name: &str) -> (Agent, PathBuf, PathBuf) {
        let base = std::env::temp_dir().join(format!("drenn_v2_{name}_{}", now_secs()));
        let _ = std::fs::remove_dir_all(&base);
        let p = base.join("proj");
        std::fs::create_dir_all(&p).unwrap();
        let cfg = Config { allowed_roots: vec![base.to_string_lossy().to_string()], ..Config::default() };
        let mut a = Agent::new(cfg, base.clone(), None).unwrap();
        a.test_yes = true;
        for args in [vec!["init", "-q", "-b", "main"], vec!["config", "user.name", "T"], vec!["config", "user.email", "t@t.t"]] {
            a.git.out(&p, &args, Kind::Local).unwrap();
        }
        (a, base, p)
    }
    fn go(a: &Agent, v: serde_json::Value) -> Res<String> {
        a.exec(&serde_json::from_value::<Action>(v).unwrap())
    }
    fn commit_file(a: &Agent, p: &Path, f: &str, c: &str, msg: &str) {
        std::fs::write(p.join(f), c).unwrap();
        go(a, json!({"action":"stage_changes","path":"proj"})).unwrap();
        go(a, json!({"action":"create_commit","path":"proj","message":msg})).unwrap();
    }

    #[test]
    fn gitignore_defaut_et_idempotent() {
        let (a, _b, p) = setup("ign");
        std::fs::write(p.join("package.json"), "{}").unwrap();
        let r = go(&a, json!({"action":"update_gitignore","path":"proj"})).unwrap();
        assert!(r.contains("mis à jour"), "{r}");
        let g = std::fs::read_to_string(p.join(".gitignore")).unwrap();
        assert!(g.contains(".env") && g.contains("node_modules/"));
        assert!(go(&a, json!({"action":"update_gitignore","path":"proj"})).unwrap().contains("rien à ajouter"));
        assert!(go(&a, json!({"action":"update_gitignore","path":"proj","patterns":["*"]})).is_err());
        // les secrets ignorés ne sont plus proposés au stage
        std::fs::write(p.join(".env"), "A=1").unwrap();
        let r = go(&a, json!({"action":"stage_changes","path":"proj"})).unwrap();
        assert!(!r.contains("EXCLU"), "{r}");
    }

    #[test]
    fn annulation_du_dernier_commit() {
        let (a, base, p) = setup("undo");
        commit_file(&a, &p, "a.txt", "1", "un");
        commit_file(&a, &p, "b.txt", "2", "deux");
        let first = a.git.line(&p, &["rev-parse", "HEAD~1"]).unwrap();
        let r = go(&a, json!({"action":"undo_last_commit","path":"proj"})).unwrap();
        assert!(r.contains("annulé"), "{r}");
        assert_eq!(a.git.head(&p).unwrap(), first);
        assert!(p.join("b.txt").exists());
        assert_eq!(a.staged_names(&p).unwrap(), vec!["b.txt".to_string()]);
        // Un commit déjà envoyé n'est jamais annulé.
        go(&a, json!({"action":"create_commit","path":"proj","message":"deux"})).unwrap();
        let bare = base.join("r.git");
        a.git.out(&base, &["init", "-q", "--bare", bare.to_str().unwrap()], Kind::Local).unwrap();
        a.git.out(&p, &["remote", "add", "origin", bare.to_str().unwrap()], Kind::Local).unwrap();
        go(&a, json!({"action":"push_changes","path":"proj"})).unwrap();
        let e = go(&a, json!({"action":"undo_last_commit","path":"proj"})).unwrap_err();
        assert!(e.contains("déjà présent"), "{e}");
    }

    #[test]
    fn fusion_et_conflit_annule_proprement() {
        let (a, _b, p) = setup("merge");
        commit_file(&a, &p, "a.txt", "base\n", "base");
        go(&a, json!({"action":"create_branch","path":"proj","name":"dev"})).unwrap();
        commit_file(&a, &p, "d.txt", "dev\n", "dev");
        go(&a, json!({"action":"switch_branch","path":"proj","name":"main"})).unwrap();
        let r = go(&a, json!({"action":"merge_branch","path":"proj","name":"dev"})).unwrap();
        assert!(r.contains("fusionnée"), "{r}");
        assert!(p.join("d.txt").exists());
        // conflit
        go(&a, json!({"action":"create_branch","path":"proj","name":"x"})).unwrap();
        commit_file(&a, &p, "a.txt", "X\n", "x");
        go(&a, json!({"action":"switch_branch","path":"proj","name":"main"})).unwrap();
        commit_file(&a, &p, "a.txt", "MAIN\n", "main");
        let head = a.git.head(&p).unwrap();
        let e = go(&a, json!({"action":"merge_branch","path":"proj","name":"x"})).unwrap_err();
        assert!(e.contains("conflit") && e.contains("intacte"), "{e}");
        assert_eq!(a.git.head(&p).unwrap(), head);
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "MAIN\n");
        // modifications non enregistrées : refus
        std::fs::write(p.join("a.txt"), "sale").unwrap();
        assert!(go(&a, json!({"action":"merge_branch","path":"proj","name":"dev"})).unwrap_err().contains("non enregistrées"));
    }

    fn conflict_repo(name: &str) -> (Agent, PathBuf) {
        let (a, _b, p) = setup(name);
        commit_file(&a, &p, "a.txt", "base\n", "base");
        go(&a, json!({"action":"create_branch","path":"proj","name":"x"})).unwrap();
        commit_file(&a, &p, "a.txt", "X\n", "x");
        go(&a, json!({"action":"switch_branch","path":"proj","name":"main"})).unwrap();
        commit_file(&a, &p, "a.txt", "MAIN\n", "main");
        (a, p)
    }

    #[test]
    fn conflits_garder_ma_version_puis_terminer() {
        let (a, p) = conflict_repo("cf1");
        let r = go(&a, json!({"action":"merge_branch","path":"proj","name":"x","keep_conflicts":true})).unwrap();
        assert!(r.contains("EN COURS"), "{r}");
        let r = go(&a, json!({"action":"inspect_conflicts","path":"proj"})).unwrap();
        assert!(r.contains("a.txt") && r.contains("zone"), "{r}");
        assert!(go(&a, json!({"action":"continue_merge","path":"proj"})).unwrap_err().contains("conflit"));
        let r = go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"ours"})).unwrap();
        assert!(r.contains("Sauvegarde"), "{r}");
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "MAIN\n");
        assert!(p.join(".git").join("drenn-backup").is_dir());
        let r = go(&a, json!({"action":"continue_merge","path":"proj"})).unwrap();
        assert!(r.contains("terminée"), "{r}");
        assert!(!a.merging(&p));
        assert!(a.git.line(&p, &["rev-parse", "--verify", "-q", "HEAD^2"]).is_ok());
    }

    #[test]
    fn conflits_version_fusionnee_et_annulation() {
        let (a, p) = conflict_repo("cf2");
        go(&a, json!({"action":"merge_branch","path":"proj","name":"x","keep_conflicts":true})).unwrap();
        go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"theirs"})).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "X\n");
        let head = a.git.head(&p).unwrap();
        go(&a, json!({"action":"abort_merge","path":"proj"})).unwrap();
        assert_eq!(a.git.head(&p).unwrap(), head);
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "MAIN\n");
        assert!(go(&a, json!({"action":"abort_merge","path":"proj"})).is_err());
        assert!(go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"nimporte"})).is_err());
    }

    #[test]
    fn conflit_suppression_contre_modification() {
        let (a, _b, p) = setup("cf3");
        commit_file(&a, &p, "a.txt", "base\n", "base");
        commit_file(&a, &p, "b.txt", "b\n", "b");
        go(&a, json!({"action":"create_branch","path":"proj","name":"x"})).unwrap();
        a.git.out(&p, &["rm", "-q", "a.txt"], Kind::Local).unwrap();
        go(&a, json!({"action":"create_commit","path":"proj","message":"suppr"})).unwrap();
        go(&a, json!({"action":"switch_branch","path":"proj","name":"main"})).unwrap();
        commit_file(&a, &p, "a.txt", "modifie\n", "modif");
        go(&a, json!({"action":"merge_branch","path":"proj","name":"x","keep_conflicts":true})).unwrap();
        assert!(go(&a, json!({"action":"inspect_conflicts","path":"proj"})).unwrap().contains("supprimé"));
        go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"theirs"})).unwrap();
        assert!(!p.join("a.txt").exists());
        go(&a, json!({"action":"continue_merge","path":"proj"})).unwrap();
    }

    #[test]
    fn etiquette_distante() {
        let (a, base, p) = setup("tagpush");
        commit_file(&a, &p, "a.txt", "1", "un");
        let bare = base.join("r.git");
        a.git.out(&base, &["init", "-q", "--bare", bare.to_str().unwrap()], Kind::Local).unwrap();
        a.git.out(&p, &["remote", "add", "origin", bare.to_str().unwrap()], Kind::Local).unwrap();
        assert!(go(&a, json!({"action":"push_tag","path":"proj","name":"v1"})).unwrap_err().contains("n'existe pas"));
        go(&a, json!({"action":"create_tag","path":"proj","name":"v1"})).unwrap();
        let r = go(&a, json!({"action":"push_tag","path":"proj","name":"v1"})).unwrap();
        assert!(r.contains("Vérifié"), "{r}");
        assert!(go(&a, json!({"action":"push_tag","path":"proj","name":"v1"})).unwrap().contains("déjà"));
    }

    #[test]
    fn mise_de_cote_et_retour() {
        let (a, _b, p) = setup("stash");
        commit_file(&a, &p, "a.txt", "v1", "un");
        assert!(go(&a, json!({"action":"stash_changes","path":"proj"})).unwrap().contains("Aucune"));
        std::fs::write(p.join("a.txt"), "v2").unwrap();
        std::fs::write(p.join("n.txt"), "neuf").unwrap();
        let r = go(&a, json!({"action":"stash_changes","path":"proj","message":"pause"})).unwrap();
        assert!(r.contains("mis(es) de côté") || r.contains("mis de côté"), "{r}");
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "v1");
        assert!(!p.join("n.txt").exists());
        // un travail en cours bloque la remise en place
        std::fs::write(p.join("a.txt"), "autre").unwrap();
        assert!(go(&a, json!({"action":"restore_stash","path":"proj"})).unwrap_err().contains("en cours"));
        go(&a, json!({"action":"discard_changes","path":"proj"})).unwrap();
        go(&a, json!({"action":"restore_stash","path":"proj"})).unwrap();
        assert_eq!(std::fs::read_to_string(p.join("a.txt")).unwrap(), "v2");
        assert!(p.join("n.txt").exists());
        assert!(go(&a, json!({"action":"restore_stash","path":"proj"})).is_err());
    }

    #[test]
    fn etiquette() {
        let (a, _b, p) = setup("tag");
        commit_file(&a, &p, "a.txt", "1", "un");
        let r = go(&a, json!({"action":"create_tag","path":"proj","name":"v1.0.0","message":"Première version"})).unwrap();
        assert!(r.contains("v1.0.0"), "{r}");
        assert!(go(&a, json!({"action":"create_tag","path":"proj","name":"v1.0.0"})).unwrap_err().contains("existe déjà"));
        assert!(go(&a, json!({"action":"create_tag","path":"proj","name":"a b"})).is_err());
        assert!(go(&a, json!({"action":"create_tag","path":"proj","name":"-x"})).is_err());
    }

    // ---- v0.6 : conflits zone par zone, commit IA, apprentissage

    struct Mock(&'static str, bool);
    impl Llm for Mock {
        fn name(&self) -> String {
            "faux moteur".into()
        }
        fn is_local(&self) -> bool {
            self.1
        }
        fn chat(&self, _s: &str, user: &str, _sch: Option<&serde_json::Value>, _m: u32) -> Res<String> {
            assert!(!user.contains("contenu secret"), "le contenu des fichiers ne doit jamais être transmis");
            Ok(self.0.to_string())
        }
    }

    /// Deux zones de conflit distinctes (lignes 2 et 11 d'un fichier de 12 lignes).
    fn two_hunk_repo(name: &str) -> (Agent, PathBuf) {
        let (a, _b, p) = setup(name);
        let base: Vec<String> = (1..=12).map(|i| format!("l{i}")).collect();
        let write = |f: &dyn Fn(usize, &str) -> String| base.iter().enumerate().map(|(i, l)| f(i, l)).collect::<Vec<_>>().join("\n") + "\n";
        commit_file(&a, &p, "a.txt", &base.join("\n"), "base");
        go(&a, json!({"action":"create_branch","path":"proj","name":"x"})).unwrap();
        commit_file(&a, &p, "a.txt", &write(&|i, l| if i == 1 || i == 10 { format!("X-{l}") } else { l.to_string() }), "x");
        go(&a, json!({"action":"switch_branch","path":"proj","name":"main"})).unwrap();
        commit_file(&a, &p, "a.txt", &write(&|i, l| if i == 1 || i == 10 { format!("M-{l}") } else { l.to_string() }), "main");
        go(&a, json!({"action":"merge_branch","path":"proj","name":"x","keep_conflicts":true})).unwrap();
        (a, p)
    }

    #[test]
    fn conflits_zone_par_zone() {
        let (a, p) = two_hunk_repo("hk1");
        let r = go(&a, json!({"action":"inspect_conflicts","path":"proj"})).unwrap();
        assert!(r.contains("2 zone"), "{r}");
        a.test_choices.lock().unwrap().extend([0, 1]); // zone 1 : ma version ; zone 2 : version fusionnée
        let r = go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"ask"})).unwrap();
        assert!(r.contains("2 zone") && r.contains("termine la fusion"), "{r}");
        let txt = std::fs::read_to_string(p.join("a.txt")).unwrap();
        assert!(txt.contains("M-l2") && txt.contains("X-l11") && !txt.contains("X-l2") && !txt.contains("<<<<"), "{txt}");
        assert!(p.join(".git").join("drenn-backup").is_dir());
        assert!(go(&a, json!({"action":"continue_merge","path":"proj"})).unwrap().contains("terminée"));
    }

    #[test]
    fn conflits_zone_par_zone_annulation_et_les_deux() {
        let (a, p) = two_hunk_repo("hk2");
        // aucune réponse (annulation) : rien n'est modifié
        let e = go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"ask"})).unwrap_err();
        assert!(e.contains("annulée"), "{e}");
        assert!(std::fs::read_to_string(p.join("a.txt")).unwrap().contains("<<<<<<<"));
        assert!(go(&a, json!({"action":"continue_merge","path":"proj"})).is_err());
        // « passer ce fichier » : laissé en conflit
        a.test_choices.lock().unwrap().push_back(3);
        assert!(go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"ask"})).is_err());
        // les deux versions
        let r = go(&a, json!({"action":"resolve_conflicts","path":"proj","strategy":"both"})).unwrap();
        assert!(r.contains("2 zone"), "{r}");
        let txt = std::fs::read_to_string(p.join("a.txt")).unwrap();
        assert!(txt.contains("M-l2\nX-l2\n") && txt.contains("M-l11\nX-l11\n"), "{txt}");
    }

    #[test]
    fn message_de_commit_par_ia_locale() {
        let (mut a, _b, p) = setup("aimsg");
        std::fs::write(p.join("a.txt"), "contenu secret").unwrap();
        a.llm = Some(Arc::new(Mock("\"feat: ajoute le fichier a.\"\nexplication inutile", true)));
        go(&a, json!({"action":"stage_changes","path":"proj"})).unwrap();
        let r = go(&a, json!({"action":"create_commit","path":"proj"})).unwrap();
        assert!(r.contains("« feat: ajoute le fichier a »"), "{r}");
    }

    #[test]
    fn message_de_commit_jamais_par_le_cloud() {
        let (mut a, _b, p) = setup("aimsg2");
        std::fs::write(p.join("a.txt"), "x").unwrap();
        a.llm = Some(Arc::new(Mock("feat: ne doit pas servir", false)));
        go(&a, json!({"action":"stage_changes","path":"proj"})).unwrap();
        let r = go(&a, json!({"action":"create_commit","path":"proj"})).unwrap();
        assert!(r.contains("Premier commit"), "{r}");
    }

    #[test]
    fn nettoyage_des_messages_de_modele() {
        assert_eq!(clean_commit_message("Message : « fix: corrige »\nautre").as_deref(), Some("fix: corrige"));
        assert_eq!(clean_commit_message("\n\n`chore: x y`.").as_deref(), Some("chore: x y"));
        assert_eq!(clean_commit_message(&"a".repeat(200)).unwrap().chars().count(), 72);
        assert!(clean_commit_message("   ").is_none() && clean_commit_message("ok").is_none());
    }

    #[test]
    fn apprentissage_et_glossaire() {
        let (a, _b, _p) = setup("teach");
        assert!(a.handle("c'est quoi un commit ?"));
        assert!(!a.teach.load(Ordering::SeqCst));
        assert!(a.handle("active le mode apprentissage"));
        assert!(a.teach.load(Ordering::SeqCst));
        assert!(a.handle("désactive le mode apprentissage"));
        assert!(!a.teach.load(Ordering::SeqCst));
        // question sans réponse dans le glossaire et sans modèle : échec propre
        assert!(!a.handle("c'est quoi le zorglub ?"));
    }

    #[test]
    fn consentement_cloud() {
        let (a, _b, _p) = setup("cloud");
        let cloud = Mock("{}", false);
        assert!(a.allow_llm(&cloud)); // test_yes simule « oui »
        assert!(a.cloud_ok.load(Ordering::SeqCst));
        assert!(a.allow_llm(&Mock("{}", true))); // un moteur local ne demande jamais d'accord
    }
}
