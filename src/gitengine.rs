//! Moteur Git : exécute le vrai exécutable `git` (sans shell, arguments séparés, délais, sortie vérifiée).

use crate::config::Config;
use crate::util::{base64, redact, run_cmd, Out, Res};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Local,
    Net,
}

pub struct Git {
    bin: PathBuf,
    timeout: Duration,
    net_timeout: Duration,
    allow_hooks: bool,
    token: Option<String>,
    /// Dossier vide servant de « hooksPath » (plus fiable que /dev/null sous Windows).
    nohooks: String,
    pub version: (u32, u32),
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub x: char,
    pub y: char,
    pub path: String,
}

impl Entry {
    pub fn untracked(&self) -> bool {
        self.x == '?'
    }
    pub fn conflicted(&self) -> bool {
        self.x == 'U' || self.y == 'U' || (self.x == 'A' && self.y == 'A') || (self.x == 'D' && self.y == 'D')
    }
    pub fn staged(&self) -> bool {
        self.x != ' ' && self.x != '?' && self.x != '!'
    }
    pub fn unstaged(&self) -> bool {
        self.y != ' ' && self.y != '?' && self.y != '!'
    }
}

fn candidates() -> Vec<PathBuf> {
    let mut v = vec![PathBuf::from("git")];
    if cfg!(windows) {
        for p in [
            r"C:\Program Files\Git\cmd\git.exe",
            r"C:\Program Files (x86)\Git\cmd\git.exe",
        ] {
            v.push(PathBuf::from(p));
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            v.push(PathBuf::from(local).join("Programs").join("Git").join("cmd").join("git.exe"));
        }
    } else {
        // Environnements au PATH réduit (applications graphiques, services) : emplacements usuels de macOS, Linux et BSD.
        for p in ["/usr/bin/git", "/usr/local/bin/git", "/opt/homebrew/bin/git", "/usr/pkg/bin/git", "/opt/local/bin/git"] {
            v.push(PathBuf::from(p));
        }
    }
    v
}

impl Git {
    /// Localise Git (PATH puis emplacements usuels de Windows).
    pub fn detect(cfg: &Config, token: Option<String>) -> Res<Git> {
        for c in candidates() {
            let mut cmd = Command::new(&c);
            cmd.arg("--version");
            if let Ok(o) = run_cmd(&mut cmd, None, Duration::from_secs(10)) {
                if o.ok() && o.stdout.starts_with("git version") {
                    let version = parse_version(&o.stdout);
                    if version < (2, 23) {
                        return Err(format!(
                            "Git {}.{} est trop ancien : GIT DRENN nécessite Git 2.23 ou plus récent (idéalement 2.31+). Mettez Git à jour : https://git-scm.com",
                            version.0, version.1
                        ));
                    }
                    let hooks_dir = crate::util::drenn_dir().join("nohooks");
                    let nohooks = match std::fs::create_dir_all(&hooks_dir) {
                        Ok(_) => hooks_dir.to_string_lossy().replace('\\', "/"),
                        Err(_) => "/dev/null".to_string(),
                    };
                    return Ok(Git {
                        nohooks,
                        version,
                        bin: c,
                        timeout: Duration::from_secs(cfg.git_timeout_secs),
                        net_timeout: Duration::from_secs(cfg.network_timeout_secs),
                        allow_hooks: cfg.allow_hooks,
                        token,
                    });
                }
            }
        }
        Err(format!("Git est introuvable. {} puis relancez GIT DRENN.", crate::util::git_install_hint()))
    }

    pub fn version(&self) -> String {
        let mut cmd = Command::new(&self.bin);
        cmd.arg("--version");
        run_cmd(&mut cmd, None, Duration::from_secs(10)).map(|o| o.stdout.trim().to_string()).unwrap_or_default()
    }

    pub fn run(&self, cwd: &Path, args: &[&str], kind: Kind) -> Res<Out> {
        let mut cmd = Command::new(&self.bin);
        cmd.args(["-c", "core.quotepath=false", "-c", "protocol.ext.allow=never", "-c", "core.fsmonitor=false"]);
        if !self.allow_hooks {
            cmd.arg("-c").arg(format!("core.hooksPath={}", self.nohooks));
        }
        cmd.args(args).current_dir(cwd);
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .env("LANGUAGE", "C");
        // Pathspecs littéraux (noms de fichiers à caractères spéciaux). Exception : `git stash -u` nettoie les
        // fichiers non suivis avec ses propres pathspecs internes, qui cessent de fonctionner en mode littéral.
        // Les commandes stash de GIT DRENN n'acceptent jamais de pathspec issu de l'utilisateur.
        if args.first() != Some(&"stash") {
            cmd.env("GIT_LITERAL_PATHSPECS", "1");
        }
        if kind == Kind::Net {
            if let Some(t) = &self.token {
                // Jeton transmis par variables d'environnement (jamais en argument), limité à github.com.
                let b = base64(format!("x-access-token:{t}").as_bytes());
                cmd.env("GIT_CONFIG_COUNT", "1")
                    .env("GIT_CONFIG_KEY_0", "http.https://github.com/.extraheader")
                    .env("GIT_CONFIG_VALUE_0", format!("AUTHORIZATION: basic {b}"));
            }
        }
        let timeout = if kind == Kind::Net { self.net_timeout } else { self.timeout };
        let mut out = run_cmd(&mut cmd, None, timeout)?;
        out.stdout = redact(&out.stdout, self.token.as_deref());
        out.stderr = redact(&out.stderr, self.token.as_deref());
        Ok(out)
    }

    /// Sortie brute (non rognée) ; erreur si le code de retour n'est pas 0.
    pub fn out(&self, cwd: &Path, args: &[&str], kind: Kind) -> Res<String> {
        let o = self.run(cwd, args, kind)?;
        if o.ok() {
            Ok(o.stdout)
        } else {
            Err(o.err_text())
        }
    }

    pub fn line(&self, cwd: &Path, args: &[&str]) -> Res<String> {
        self.out(cwd, args, Kind::Local).map(|s| s.trim().to_string())
    }

    pub fn is_repo(&self, dir: &Path) -> bool {
        self.run(dir, &["rev-parse", "--is-inside-work-tree"], Kind::Local)
            .map(|o| o.ok() && o.stdout.trim() == "true")
            .unwrap_or(false)
    }

    pub fn toplevel(&self, dir: &Path) -> Res<PathBuf> {
        let s = self.line(dir, &["rev-parse", "--show-toplevel"]).map_err(|_| {
            format!("« {} » n'est pas un dépôt Git", dir.display())
        })?;
        crate::util::canonicalize_lenient(Path::new(&s))
    }

    pub fn branch(&self, repo: &Path) -> Option<String> {
        self.line(repo, &["branch", "--show-current"]).ok().filter(|s| !s.is_empty())
    }

    pub fn head(&self, repo: &Path) -> Option<String> {
        self.line(repo, &["rev-parse", "--verify", "-q", "HEAD"]).ok().filter(|s| !s.is_empty())
    }

    pub fn upstream(&self, repo: &Path) -> Option<String> {
        self.line(repo, &["rev-parse", "--abbrev-ref", "--symbolic-full-name", "@{u}"]).ok().filter(|s| !s.is_empty())
    }

    pub fn remotes(&self, repo: &Path) -> Vec<(String, String)> {
        let names = self.line(repo, &["remote"]).unwrap_or_default();
        names
            .lines()
            .filter(|n| !n.trim().is_empty())
            .map(|n| {
                let url = self.line(repo, &["remote", "get-url", n]).unwrap_or_default();
                (n.trim().to_string(), url)
            })
            .collect()
    }

    pub fn status(&self, repo: &Path) -> Res<Vec<Entry>> {
        let raw = self.out(repo, &["status", "--porcelain=v1", "-z", "--untracked-files=all"], Kind::Local)?;
        let mut entries = Vec::new();
        let mut it = raw.split('\0');
        while let Some(s) = it.next() {
            let cs: Vec<char> = s.chars().collect();
            if cs.len() < 4 {
                continue;
            }
            let (x, y) = (cs[0], cs[1]);
            let path: String = cs[3..].iter().collect();
            if x == 'R' || x == 'C' || y == 'R' || y == 'C' {
                it.next(); // ancien nom
            }
            entries.push(Entry { x, y, path });
        }
        if entries.len() > 20_000 {
            return Err(format!(
                "{} fichiers non suivis ou modifiés : ajoutez un fichier .gitignore (node_modules, vendor, builds…) avant de continuer",
                entries.len()
            ));
        }
        Ok(entries)
    }

    /// Vrai si le dépôt distant `remote` possède la branche, avec son hash.
    pub fn remote_branch_hash(&self, repo: &Path, remote: &str, branch: &str) -> Res<Option<String>> {
        let refname = format!("refs/heads/{branch}");
        let o = self.out(repo, &["ls-remote", "--heads", remote, &refname], Kind::Net)?;
        Ok(o.lines().next().and_then(|l| l.split_whitespace().next()).map(|s| s.to_string()))
    }
}

/// Extrait (majeur, mineur) de « git version 2.43.0.windows.1 ».
pub fn parse_version(s: &str) -> (u32, u32) {
    let v = s.trim().trim_start_matches("git version").trim();
    let mut it = v.split(|c: char| !c.is_ascii_digit()).filter(|x| !x.is_empty()).map(|x| x.parse::<u32>().unwrap_or(0));
    (it.next().unwrap_or(0), it.next().unwrap_or(0))
}

/// Traduit les erreurs Git les plus courantes en cause probable + piste de correction.
pub fn explain_error(msg: &str) -> Option<String> {
    let m = msg.to_lowercase();
    let table: &[(&[&str], &str)] = &[
        (&["could not resolve host", "unable to access", "network is unreachable", "timed out"],
         "Connexion Internet indisponible ou service injoignable. Les opérations locales restent utilisables ; réessayez les opérations distantes une fois la connexion rétablie."),
        (&["authentication failed", "could not read username", "invalid username or password", "terminal prompts disabled"],
         "Authentification refusée. Configurez un jeton GitHub (`drenn auth login`) ou connectez-vous via le gestionnaire d'identifiants de Git."),
        (&["permission denied (publickey)"],
         "Clé SSH refusée. Utilisez une URL https:// ou configurez votre clé SSH auprès de GitHub."),
        (&["repository not found", "not found"],
         "Dépôt introuvable ou accès non autorisé : vérifiez le nom, l'orthographe et vos droits."),
        (&["non-fast-forward", "fetch first", "[rejected]", "failed to push some refs"],
         "Le dépôt distant contient des modifications que vous n'avez pas : récupérez-les d'abord (« récupère les dernières modifications »), puis renvoyez."),
        (&["not possible to fast-forward", "diverging branches"],
         "Les historiques local et distant ont divergé : une fusion manuelle est nécessaire. GIT DRENN ne fusionne pas automatiquement pour ne pas écraser votre travail."),
        (&["would be overwritten"],
         "Des modifications locales seraient écrasées. Enregistrez-les (commit) ou mettez-les de côté avant de continuer."),
        (&["please tell me who you are", "unable to auto-detect email", "empty ident"],
         "Identité Git absente. Exécutez : git config --global user.name \"Votre nom\" puis git config --global user.email \"vous@exemple.com\"."),
        (&["does not match any", "src refspec"],
         "Aucun commit à envoyer : enregistrez d'abord vos modifications."),
        (&["already exists and is not an empty directory"],
         "Le dossier de destination existe déjà et n'est pas vide. Choisissez un autre dossier."),
        (&["not a git repository"],
         "Ce dossier n'est pas un dépôt Git. Demandez d'abord son initialisation."),
        (&["conflict", "unmerged"],
         "Des conflits existent. Résolvez-les dans les fichiers concernés, puis enregistrez."),
        (&["unrelated histories"],
         "Les deux dépôts n'ont pas d'historique commun (souvent un dépôt distant déjà initialisé avec un README)."),
        (&["file exceeds github", "this exceeds github's file size limit"],
         "Un fichier dépasse la limite de 100 Mo de GitHub : excluez-le (.gitignore) ou utilisez Git LFS."),
    ];
    for (keys, text) in table {
        if keys.iter().any(|k| m.contains(k)) {
            return Some((*text).to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::parse_version;
    #[test]
    fn versions() {
        assert_eq!(parse_version("git version 2.43.0.windows.1\n"), (2, 43));
        assert_eq!(parse_version("git version 2.20.1"), (2, 20));
        assert_eq!(parse_version("n'importe quoi"), (0, 0));
    }
}
