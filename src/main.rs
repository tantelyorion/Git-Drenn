//! GIT DRENN — Git Powered by AI.  « Vous décrivez le travail, l'IA s'occupe des opérations Git. »

mod actions;
mod agent;
mod config;
mod github;
mod gui;
mod gitengine;
mod planner;
mod rules;
mod security;
mod ui;
mod util;

use agent::Agent;
use config::Config;
use std::path::PathBuf;

const HELP: &str = "GIT DRENN 0.5.0 — Git piloté par un agent IA
Créateur : Mr Tantely Orion

Utilisation :
  drenn                      session interactive (conversation avec l'agent)
  drenn \"<instruction>\"      exécute une instruction puis quitte
  drenn doctor               vérifie l'installation (Git, curl, modèle IA, jeton)
  drenn projects             liste vos projets Git et leur état
  drenn history [n]          journal des dernières opérations
  drenn config               affiche la configuration
  drenn config init|path     crée la configuration / affiche son emplacement
  drenn config set <clé> <valeur>   modifie un réglage (ex. model_path, llama_bin, projects_dir)
  drenn config add-root <dossier>   autorise un dossier de travail supplémentaire
  drenn bench                mesure le modèle local (validité, justesse, temps) sur des demandes types
  drenn gui                  interface graphique locale dans le navigateur (--no-browser : ne l'ouvre pas)
  drenn about                la mascotte, la version et le créateur (Mr Tantely Orion)
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
  drenn \"publie mon projet Blog sur GitHub\"

GIT DRENN — créé par Mr Tantely Orion.";

fn main() {
    ui::setup_console();
    let code = run();
    std::process::exit(code);
}

fn run() -> i32 {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut dir: Option<PathBuf> = None;
    let (mut dry, mut no_ai, mut auto_medium, mut no_browser) = (false, false, false, false);
    let mut rest: Vec<String> = Vec::new();
    let mut it = args.drain(..);
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => {
                ui::say(HELP);
                return 0;
            }
            "-V" | "--version" => {
                ui::say(format!("GIT DRENN {} — créateur : {}", env!("CARGO_PKG_VERSION"), ui::CREATOR));
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
            "--no-browser" => no_browser = true,
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
        Some("config") => return cmd_config(&cfg, &rest[1..]),
        Some("about") => {
            ui::banner(env!("CARGO_PKG_VERSION"));
            return 0;
        }
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
        Some("bench") => cmd_bench(&agent),
        Some("gui") => match gui::serve(std::sync::Arc::new(agent), !no_browser) {
            Ok(()) => 0,
            Err(e) => {
                ui::err(e);
                1
            }
        },
        None => repl(&agent),
        Some(_) => {
            if agent.handle(&rest.join(" ")) { 0 } else { 1 }
        }
    }
}

fn repl(agent: &Agent) -> i32 {
    ui::banner(env!("CARGO_PKG_VERSION"));
    ui::say("\nTapez votre demande (« aide » pour des exemples, « quitter » pour sortir).");
    ui::say(format!("Dossier de travail : {}", agent.cwd.display()));
    loop {
        let Some(line) = ui::read_line("\ndrenn> ") else { return 0 };
        let l = line.trim();
        match l.to_lowercase().as_str() {
            "" => continue,
            "quitter" | "exit" | "quit" | "q" => return 0,
            "aide" | "help" | "?" => ui::say(HELP),
            "a propos" | "à propos" | "about" => ui::say(ui::about_text()),
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
    if agent.git.version < (2, 31) {
        ui::warn("Git < 2.31 : l'authentification GitHub par jeton est limitée ; mettez Git à jour (https://git-scm.com).");
    }
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

fn cmd_config(cfg: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
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
        Some("set") => match (args.get(1), args.get(2)) {
            (Some(k), Some(v)) => return config_set(cfg, k, v),
            _ => {
                ui::err("usage : drenn config set <clé> <valeur>");
                return 2;
            }
        },
        Some("add-root") => {
            let Some(d) = args.get(1) else {
                ui::err("usage : drenn config add-root <dossier>");
                return 2;
            };
            let Ok(c) = util::canonicalize_lenient(std::path::Path::new(d)) else {
                ui::err("dossier invalide");
                return 1;
            };
            if !c.is_dir() {
                ui::err(format!("{} n'est pas un dossier existant", c.display()));
                return 1;
            }
            let mut n = cfg.clone();
            if n.allowed_roots.is_empty() {
                ui::warn("Les dossiers autorisés par défaut (dossier courant + dossier personnel) sont remplacés par votre liste.");
            }
            let s = c.display().to_string();
            if !n.allowed_roots.contains(&s) {
                n.allowed_roots.push(s);
            }
            return save_cfg(&n);
        }
        _ => ui::say(serde_json::to_string_pretty(cfg).unwrap_or_default()),
    }
    0
}

fn save_cfg(c: &Config) -> i32 {
    match c.save() {
        Ok(p) => {
            ui::ok(format!("Configuration enregistrée : {}", p.display()));
            0
        }
        Err(e) => {
            ui::err(e);
            1
        }
    }
}

fn config_set(cfg: &Config, key: &str, val: &str) -> i32 {
    use serde_json::Value;
    let parse_num = |min: u64, max: u64| -> Result<Value, String> {
        let n: u64 = val.parse().map_err(|_| format!("« {val} » n'est pas un nombre"))?;
        if n < min || n > max {
            return Err(format!("valeur hors limites ({min}–{max})"));
        }
        Ok(Value::from(n))
    };
    let parse_bool = || -> Result<Value, String> {
        match val.to_lowercase().as_str() {
            "true" | "oui" | "1" | "yes" | "on" => Ok(Value::Bool(true)),
            "false" | "non" | "0" | "no" | "off" => Ok(Value::Bool(false)),
            _ => Err("attendu : oui ou non".into()),
        }
    };
    let new = match key {
        "llama_bin" | "model_path" | "projects_dir" => {
            if val.is_empty() || val == "none" {
                Ok(Value::Null)
            } else {
                let exists = std::path::Path::new(val).exists();
                if !exists {
                    ui::warn(format!("« {val} » n'existe pas (encore) : vérifiez le chemin avec `drenn doctor`."));
                }
                Ok(Value::String(val.to_string()))
            }
        }
        "ctx_size" => parse_num(256, 32768),
        "threads" => parse_num(1, 64),
        "max_tokens" => parse_num(32, 4096),
        "llama_timeout_secs" | "git_timeout_secs" | "network_timeout_secs" => parse_num(1, 7200),
        "max_file_size_mb" => parse_num(1, 100),
        "use_json_schema" | "auto_confirm_medium" => parse_bool(),
        "allow_hooks" => {
            let r = parse_bool();
            if matches!(r, Ok(Value::Bool(true))) {
                ui::warn("Les hooks Git des projets sont des scripts non vérifiés : ne l'activez que pour des projets de confiance.");
            }
            r
        }
        _ => Err("clé inconnue (voir `drenn config`) ; pour les dossiers autorisés : `drenn config add-root`".into()),
    };
    let new = match new {
        Ok(v) => v,
        Err(e) => {
            ui::err(e);
            return 2;
        }
    };
    let mut v = serde_json::to_value(cfg).unwrap_or(Value::Null);
    v[key] = new;
    match serde_json::from_value::<Config>(v) {
        Ok(c) => save_cfg(&c),
        Err(e) => {
            ui::err(format!("valeur refusée : {e}"));
            1
        }
    }
}

fn cmd_bench(agent: &Agent) -> i32 {
    let Some(lp) = planner::LlamaPlanner::from_cfg(&agent.cfg) else {
        ui::err("Modèle local non configuré : `drenn config set llama_bin <chemin>` et `drenn config set model_path <chemin>` (voir `drenn doctor`).");
        return 1;
    };
    let cases = [
        ("clone mastodon", "clone_repository"),
        ("enregistre mes modifications", "stage_changes"),
        ("crée une branche login", "create_branch"),
        ("publie mon projet Blog sur GitHub", "initialize_repository"),
        ("récupère les dernières modifications", "pull_changes"),
        ("quel est l'état du projet", "inspect_status"),
        ("annule mes dernières modifications", "discard_changes"),
        ("annule mon dernier commit", "undo_last_commit"),
        ("fusionne la branche dev", "merge_branch"),
        ("complète le gitignore", "update_gitignore"),
    ];
    let ctx = planner::PlanContext { cwd: agent.cwd.display().to_string(), in_repo: true, branch: Some("main".into()), dirty: 2, remotes: vec!["origin".into()] };
    ui::say(format!("Test du modèle sur {} demandes types (peut prendre quelques minutes)…", cases.len()));
    let (mut valid, mut correct, mut total_ms) = (0usize, 0usize, 0u128);
    for (instr, expected) in cases {
        let t = std::time::Instant::now();
        let r = lp.plan(instr, &ctx);
        let ms = t.elapsed().as_millis();
        total_ms += ms;
        match r {
            Ok(p) => {
                valid += 1;
                let first = p.steps.first().and_then(|s| serde_json::to_value(s).ok()).and_then(|v| v["action"].as_str().map(|a| a.to_string()));
                let good = first.as_deref() == Some(expected);
                if good {
                    correct += 1;
                }
                ui::say(format!("  {} {:<40} → {} ({} ms)", if good { "[OK]    " } else { "[ECART] " }, instr, first.unwrap_or_else(|| "(aucune étape)".into()), ms));
            }
            Err(e) => ui::say(format!("  [INVALIDE] {:<38} → {} ({} ms)", instr, e, ms)),
        }
    }
    let n = cases.len();
    ui::say(format!("\nPlans valides : {valid}/{n} · première action correcte : {correct}/{n} · temps moyen : {} ms", total_ms / n as u128));
    ui::say("Mémoire : relevez le pic de RAM de llama-cli dans le Gestionnaire des tâches pendant ce test.");
    if correct * 10 < n * 7 {
        ui::warn("Moins de 70 % de justesse : l'analyseur de règles prendra le relais, mais envisagez un modèle plus grand.");
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
