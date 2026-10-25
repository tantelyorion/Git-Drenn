//! GIT DRENN — Git Powered by AI.  « Vous décrivez le travail, l'IA s'occupe des opérations Git. »

mod actions;
mod agent;
mod config;
mod github;
mod gitengine;
mod planner;
mod rules;
mod security;
mod ui;
mod util;

use agent::Agent;
use config::Config;
use std::path::PathBuf;

const HELP: &str = "GIT DRENN 0.1 — Git piloté par un agent IA

Utilisation :
  drenn                      session interactive (conversation avec l'agent)
  drenn \"<instruction>\"      exécute une instruction puis quitte
  drenn doctor               vérifie l'installation (Git, curl, modèle IA, jeton)
  drenn projects             liste vos projets Git et leur état
  drenn history [n]          journal des dernières opérations
  drenn config [init|path]   affiche / crée la configuration
  drenn auth login|logout|status   jeton GitHub (login : lit le jeton sur l'entrée standard)

Options :
  --dir <dossier>    dossier de travail (défaut : dossier courant)
  --dry-run          affiche le plan sans rien exécuter
  --no-ai            n'utilise pas le modèle local (analyseur de règles seul)
  --auto-medium      exécute sans confirmation les opérations locales (jamais push/suppression)
  -h, --help / -V, --version

Exemples :
  drenn \"clone mastodon dans mon dossier Projets\"
  drenn \"enregistre mes modifications\"
  drenn \"publie mon projet Blog sur GitHub\"";

fn main() {
    ui::setup_console();
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir: Option<PathBuf> = None;
    let (mut dry, mut no_ai, mut auto_medium) = (false, false, false);
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.drain(..);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                ui::say(HELP);
                return 0;
            }
            "-V" | "--version" => {
                ui::say("GIT DRENN 0.1.0");
                return 0;
            }
            "--dir" => match it.next() {
                Some(d) => dir = Some(PathBuf::from(d)),
                None => {
                    ui::err("--dir attend un dossier");
                    return 2;
                }
            },
            "--dry-run" => dry = true,
            "--no-ai" => no_ai = true,
            "--auto-medium" => auto_medium = true,
            _ => rest.push(a),
        }
    }

    let mut cfg = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            ui::err(e);
            return 2;
        }
    };
    if auto_medium {
        cfg.auto_confirm_medium = true;
    }
    let cwd = dir.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match rest.first().map(|s| s.as_str()) {
        Some("config") => return cmd_config(&cfg, rest.get(1).map(|s| s.as_str())),
        Some("auth") => return cmd_auth(rest.get(1).map(|s| s.as_str())),
        Some("history") => {
            agent::show_history(rest.get(1).and_then(|n| n.parse().ok()).unwrap_or(10));
            return 0;
        }
        _ => {}
    }

    let token = github::load_token();
    let mut agent = match Agent::new(cfg.clone(), cwd, token) {
        Ok(a) => a,
        Err(e) => {
            ui::err(e);
            return 1;
        }
    };
    agent.dry_run = dry;
    agent.force_rules = no_ai;

    match rest.first().map(|s| s.as_str()) {
        Some("doctor") => cmd_doctor(&agent),
        Some("projects") => cmd_projects(&agent),
        None => repl(&agent),
        Some(_) => {
            if agent.handle(&rest.join(" ")) { 0 } else { 1 }
        }
    }
}

fn repl(agent: &Agent) -> i32 {
    ui::say("GIT DRENN 0.1 — décrivez ce que vous voulez faire (« aide » pour des exemples, « quitter » pour sortir).");
    ui::say(format!("Dossier de travail : {}", agent.cwd.display()));
    loop {
        let Some(line) = ui::read_line("\ndrenn> ") else { return 0 };
        let l = line.trim();
        match l.to_lowercase().as_str() {
            "" => continue,
            "quitter" | "exit" | "quit" | "q" => return 0,
            "aide" | "help" | "?" => ui::say(HELP),
            "historique" | "history" => agent::show_history(10),
            "projets" | "projects" => {
                cmd_projects(agent);
            }
            _ => {
                agent.handle(l);
            }
        }
    }
}

fn cmd_projects(agent: &Agent) -> i32 {
    let repos = agent.scan_repos();
    if repos.is_empty() {
        ui::say("Aucun projet Git trouvé dans les dossiers usuels.");
        return 0;
    }
    ui::say(format!("{} projet(s) Git :", repos.len()));
    for r in repos {
        let branch = agent.git.branch(&r).unwrap_or_else(|| "-".into());
        let dirty = agent.git.status(&r).map(|s| s.len()).unwrap_or(0);
        let name = r.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        ui::say(format!("  {name:<28} [{branch}] {}  {}", if dirty == 0 { "propre".to_string() } else { format!("{dirty} modif.") }, r.display()));
    }
    0
}

fn cmd_doctor(agent: &Agent) -> i32 {
    ui::say("Vérification de l'installation :");
    ui::ok(format!("Git : {}", agent.git.version()));
    let curl = util::run_cmd(std::process::Command::new("curl").arg("--version"), None, std::time::Duration::from_secs(10));
    match curl {
        Ok(o) if o.ok() => ui::ok(format!("curl : {}", o.stdout.lines().next().unwrap_or(""))),
        _ => ui::warn("curl introuvable : les fonctions GitHub (API) seront indisponibles"),
    }
    match (&agent.cfg.llama_bin, &agent.cfg.model_path) {
        (Some(b), Some(m)) => {
            let bin_ok = std::path::Path::new(b).is_file();
            let model = std::fs::metadata(m).map(|x| x.len()).ok();
            if bin_ok { ui::ok(format!("llama.cpp : {b}")) } else { ui::warn(format!("llama-cli introuvable : {b}")) }
            match model {
                Some(sz) => ui::ok(format!("Modèle : {m} ({} Mo — compte environ autant en RAM pendant l'analyse)", sz / 1024 / 1024)),
                None => ui::warn(format!("modèle introuvable : {m}")),
            }
        }
        _ => ui::info("Modèle IA non configuré : l'analyseur de règles est utilisé (voir README, section « Modèle IA »)."),
    }
    if agent.gh.has_token() { ui::ok("Jeton GitHub : présent") } else { ui::info("Jeton GitHub : absent (`drenn auth login`)") }
    ui::say(format!("Configuration : {}", config::config_path().display()));
    ui::say(format!("Dossiers autorisés : {}", agent.policy.roots().iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(" ; ")));
    0
}

fn cmd_config(cfg: &Config, sub: Option<&str>) -> i32 {
    match sub {
        Some("path") => ui::say(config::config_path().display().to_string()),
        Some("init") => {
            if config::config_path().exists() {
                ui::warn("La configuration existe déjà : non modifiée.");
                return 0;
            }
            match cfg.save() {
                Ok(p) => ui::ok(format!("Configuration créée : {}", p.display())),
                Err(e) => {
                    ui::err(e);
                    return 1;
                }
            }
        }
        _ => ui::say(serde_json::to_string_pretty(cfg).unwrap_or_default()),
    }
    0
}

fn cmd_auth(sub: Option<&str>) -> i32 {
    let file = github::token_file();
    match sub {
        Some("login") => {
            ui::say("Collez un jeton GitHub (permission « repo ») puis Entrée. Il sera stocké dans :");
            ui::say(format!("  {}", file.display()));
            let Some(t) = ui::read_line("Jeton : ") else { return 1 };
            let t = t.trim().to_string();
            if t.len() < 20 || t.chars().any(|c| c.is_whitespace() || c.is_control()) {
                ui::err("jeton invalide");
                return 1;
            }
            if let Some(d) = file.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            if let Err(e) = std::fs::write(&file, &t) {
                ui::err(format!("écriture impossible : {e}"));
                return 1;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
            }
            match github::GitHub::new(Some(t)).user_login() {
                Ok(l) => ui::ok(format!("Connecté à GitHub en tant que {l}.")),
                Err(e) => ui::warn(format!("Jeton enregistré mais non vérifié : {e}")),
            }
            0
        }
        Some("logout") => {
            match std::fs::remove_file(&file) {
                Ok(_) => ui::ok("Jeton local supprimé. Révoquez-le aussi sur github.com/settings/tokens."),
                Err(_) => ui::info("Aucun jeton enregistré."),
            }
            0
        }
        Some("status") | None => {
            match github::load_token() {
                Some(t) => match github::GitHub::new(Some(t)).user_login() {
                    Ok(l) => ui::ok(format!("Connecté en tant que {l}.")),
                    Err(e) => ui::warn(format!("Jeton présent mais inutilisable : {e}")),
                },
                None => ui::info("Aucun jeton GitHub (`drenn auth login` ou variable GITHUB_TOKEN)."),
            }
            0
        }
        Some(_) => {
            ui::err("usage : drenn auth login|logout|status");
            2
        }
    }
}
