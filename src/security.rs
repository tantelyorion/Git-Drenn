//! Sécurité : périmètre de travail, validation des paramètres, détection de secrets, niveaux de risque.

use crate::config::Config;
use crate::util::{canonicalize_lenient, home_dir, Res};
use std::fs::File;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Risk {
    Low,
    Medium,
    High,
    Critical,
}

impl Risk {
    pub fn label(self) -> &'static str {
        match self {
            Risk::Low => "lecture seule",
            Risk::Medium => "modification locale",
            Risk::High => "élevé (action distante)",
            Risk::Critical => "critique (destructif ou public)",
        }
    }
}

/// Dossiers que l'agent n'approche jamais, même dans un dossier autorisé.
const DENIED_COMPONENTS: &[&str] = &[
    ".ssh", ".aws", ".gnupg", ".kube", ".docker", "appdata", ".drenn", ".git",
    // Trousseaux et gestionnaires de secrets (macOS, Linux, BSD).
    "keychains", "keyrings", ".password-store", ".azure", ".gcloud",
];

pub struct Policy {
    roots: Vec<PathBuf>,
}

impl Policy {
    pub fn new(cfg: &Config, cwd: &Path) -> Policy {
        let mut roots: Vec<PathBuf> = Vec::new();
        if cfg.allowed_roots.is_empty() {
            roots.push(cwd.to_path_buf());
            if let Some(h) = home_dir() {
                roots.push(h);
            }
        } else {
            roots.extend(cfg.allowed_roots.iter().map(PathBuf::from));
        }
        if let Some(p) = &cfg.projects_dir {
            roots.push(PathBuf::from(p));
        }
        let roots = roots
            .into_iter()
            .filter_map(|r| canonicalize_lenient(&r).ok())
            // Jamais la racine d'un disque comme périmètre.
            .filter(|r| r.components().filter(|c| matches!(c, Component::Normal(_))).count() >= 1)
            .collect();
        Policy { roots }
    }

    pub fn roots(&self) -> &[PathBuf] {
        &self.roots
    }

    /// Valide qu'un chemin est dans le périmètre autorisé et renvoie sa forme canonique.
    pub fn check_path(&self, p: &Path) -> Res<PathBuf> {
        let c = canonicalize_lenient(p)?;
        let parts: Vec<String> = c.components().filter_map(|k| if let Component::Normal(n) = k { Some(n.to_string_lossy().to_lowercase()) } else { None }).collect();
        for (i, n) in parts.iter().enumerate() {
            if DENIED_COMPONENTS.contains(&n.as_str()) {
                return Err(format!("accès refusé : le dossier « {n} » est protégé"));
            }
            // macOS : ~/Library contient courrier, messages, cookies et trousseaux (un projet nommé « Library » reste permis).
            if n == "library" && parts.get(i + 1).map(|x| ["application support", "mail", "messages", "cookies", "safari", "preferences", "containers", "group containers"].contains(&x.as_str())).unwrap_or(false) {
                return Err("accès refusé : ce dossier de la bibliothèque système est protégé".into());
            }
        }
        if self.roots.iter().any(|r| c.starts_with(r)) {
            Ok(c)
        } else {
            Err(format!(
                "accès refusé : {} est en dehors des dossiers autorisés (voir `allowed_roots` dans la configuration)",
                c.display()
            ))
        }
    }
}

// ---------------------------------------------------------------- validations

pub fn valid_remote_name(n: &str) -> Res<()> {
    let ok = !n.is_empty()
        && n.len() <= 60
        && !n.starts_with('-')
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if ok { Ok(()) } else { Err(format!("nom de remote invalide : « {n} »")) }
}

pub fn valid_repo_name(n: &str) -> Res<()> {
    let ok = !n.is_empty()
        && n.len() <= 100
        && n != "."
        && n != ".."
        && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
    if ok { Ok(()) } else { Err(format!("nom de dépôt invalide : « {n} » (lettres, chiffres, . _ - uniquement)")) }
}

pub fn valid_repo_slug(s: &str) -> Res<(String, String)> {
    let mut it = s.split('/');
    match (it.next(), it.next(), it.next()) {
        (Some(o), Some(n), None) if valid_repo_name(o).is_ok() && valid_repo_name(n).is_ok() => {
            Ok((o.to_string(), n.to_string()))
        }
        _ => Err(format!("dépôt invalide : « {s} » (attendu : propriétaire/nom)")),
    }
}

/// Valide l'URL d'un dépôt distant : https ou ssh uniquement, sans identifiants intégrés.
pub fn valid_repo_url(url: &str) -> Res<()> {
    let u = url.trim();
    if u.is_empty() || u.len() > 500 {
        return Err("URL de dépôt vide ou trop longue".into());
    }
    if u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("URL invalide : caractères interdits".into());
    }
    if u.starts_with('-') {
        return Err("URL invalide".into());
    }
    let lower = u.to_lowercase();
    if lower.contains("ext::") || lower.starts_with("file:") || lower.starts_with("fd::") {
        return Err("protocole refusé pour des raisons de sécurité".into());
    }
    if let Some(rest) = lower.strip_prefix("https://").or_else(|| lower.strip_prefix("ssh://")) {
        let authority = rest.split('/').next().unwrap_or("");
        if lower.starts_with("https://") && authority.contains('@') {
            return Err("n'intégrez jamais d'identifiants dans l'URL (utilisez l'authentification de GIT DRENN)".into());
        }
        if authority.is_empty() {
            return Err("URL invalide : hôte manquant".into());
        }
        return Ok(());
    }
    // forme scp : git@hote:chemin
    if let Some(rest) = u.strip_prefix("git@") {
        if rest.contains(':') && !rest.starts_with(':') {
            return Ok(());
        }
    }
    Err("seules les URL https://, ssh:// ou git@hôte:chemin sont acceptées".into())
}

pub fn looks_like_url(s: &str) -> bool {
    let l = s.trim().to_lowercase();
    l.starts_with("https://") || l.starts_with("ssh://") || l.starts_with("git@") || l.starts_with("http://")
}

pub fn valid_commit_message(m: &str) -> Res<()> {
    if m.trim().is_empty() {
        return Err("message de commit vide".into());
    }
    if m.len() > 5000 || m.contains('\0') {
        return Err("message de commit invalide".into());
    }
    Ok(())
}

/// Chemin relatif interne à un dépôt (pas d'absolu, pas de `..`).
pub fn valid_rel_path(p: &str) -> Res<()> {
    let path = Path::new(p);
    if p.is_empty() || p.contains('\0') || path.is_absolute() {
        return Err(format!("chemin relatif attendu : « {p} »"));
    }
    if path.components().any(|c| matches!(c, Component::ParentDir | Component::Prefix(_) | Component::RootDir)) {
        return Err(format!("chemin refusé (sortie du projet) : « {p} »"));
    }
    if path.components().any(|c| c.as_os_str() == ".git") {
        return Err("le dossier .git ne peut pas être ciblé".into());
    }
    Ok(())
}

// ---------------------------------------------------------------- secrets

/// Détection par nom de fichier (chemin relatif au dépôt, séparateurs / ou \).
pub fn sensitive_name(rel: &str) -> Option<&'static str> {
    let norm = rel.replace('\\', "/").to_lowercase();
    let name = norm.rsplit('/').next().unwrap_or(&norm);
    if norm.split('/').any(|c| c == ".ssh" || c == ".aws" || c == ".gnupg") {
        return Some("dossier de secrets (.ssh / .aws / .gnupg)");
    }
    if name == ".env" || (name.starts_with(".env.") && !is_template(name)) {
        return Some("fichier d'environnement (.env)");
    }
    if name.ends_with(".env") && !is_template(name) && name != "env" {
        return Some("fichier d'environnement");
    }
    for ext in [".pem", ".key", ".p12", ".pfx", ".jks", ".keystore", ".kdbx", ".ppk"] {
        if name.ends_with(ext) {
            return Some("clé ou certificat privé");
        }
    }
    for pre in ["id_rsa", "id_dsa", "id_ecdsa", "id_ed25519"] {
        if name.starts_with(pre) && !name.ends_with(".pub") {
            return Some("clé SSH privée");
        }
    }
    const EXACT: &[&str] = &[
        ".npmrc", ".pypirc", ".netrc", ".git-credentials", ".htpasswd", "credentials", "credentials.json",
        "secrets.json", "secrets.yml", "secrets.yaml", "service-account.json", "terraform.tfstate", "wp-config.php",
    ];
    if EXACT.contains(&name) {
        return Some("fichier de identifiants / secrets");
    }
    None
}

fn is_template(name: &str) -> bool {
    ["example", "sample", "template", "dist", "default"].iter().any(|s| name.ends_with(s))
}

fn has_token(text: &str, prefix: &str, min: usize, ok: fn(char) -> bool) -> bool {
    let mut from = 0;
    while let Some(i) = text[from..].find(prefix) {
        let st = from + i + prefix.len();
        let n = text[st..].chars().take_while(|c| ok(*c)).count();
        if n >= min {
            return true;
        }
        from = st;
    }
    false
}

fn alnum(c: char) -> bool {
    c.is_ascii_alphanumeric()
}
fn alnum_us(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}
fn upper_digit(c: char) -> bool {
    c.is_ascii_uppercase() || c.is_ascii_digit()
}

/// Analyse du contenu d'un texte à la recherche de secrets évidents.
pub fn scan_text(text: &str) -> Option<&'static str> {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY-----") {
        return Some("clé privée dans le contenu");
    }
    for p in ["ghp_", "gho_", "ghu_", "ghs_", "ghr_"] {
        if has_token(text, p, 36, alnum) {
            return Some("jeton GitHub dans le contenu");
        }
    }
    if has_token(text, "github_pat_", 22, alnum_us) {
        return Some("jeton GitHub dans le contenu");
    }
    if has_token(text, "AKIA", 16, upper_digit) {
        return Some("clé d'accès AWS dans le contenu");
    }
    if has_token(text, "sk_live_", 20, alnum) {
        return Some("clé secrète Stripe dans le contenu");
    }
    if has_token(text, "xoxb-", 20, |c| c.is_ascii_alphanumeric() || c == '-') {
        return Some("jeton Slack dans le contenu");
    }
    None
}

/// Analyse un fichier : nom, taille, puis contenu (texte, ≤ 512 Ko).
pub fn check_file(abs: &Path, rel: &str, max_size_mb: u64, scan_content: bool) -> Option<String> {
    if let Some(r) = sensitive_name(rel) {
        return Some(r.to_string());
    }
    let meta = std::fs::metadata(abs).ok()?;
    if !meta.is_file() {
        return None;
    }
    if meta.len() > max_size_mb * 1024 * 1024 {
        return Some(format!("fichier volumineux ({} Mo > {} Mo)", meta.len() / 1024 / 1024, max_size_mb));
    }
    if scan_content && meta.len() <= 512 * 1024 {
        let mut buf = Vec::new();
        File::open(abs).ok()?.take(512 * 1024).read_to_end(&mut buf).ok()?;
        if buf.iter().take(4096).any(|b| *b == 0) {
            return None; // binaire
        }
        return scan_text(&String::from_utf8_lossy(&buf)).map(|s| s.to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noms_sensibles() {
        assert!(sensitive_name(".env").is_some());
        assert!(sensitive_name("app/.env.production").is_some());
        assert!(sensitive_name(".env.example").is_none());
        assert!(sensitive_name("keys/server.pem").is_some());
        assert!(sensitive_name("id_rsa").is_some());
        assert!(sensitive_name("id_rsa.pub").is_none());
        assert!(sensitive_name("src\\main.rs").is_none());
        assert!(sensitive_name("home/.ssh/config").is_some());
    }

    #[test]
    fn contenus_sensibles() {
        let tok = format!("token = {}{}", "ghp_", "a".repeat(36));
        assert!(scan_text(&tok).is_some());
        assert!(scan_text("ghp_court").is_none());
        let k = format!("-----BEGIN RSA {}-----", "PRIVATE KEY");
        assert!(scan_text(&k).is_some());
        assert!(scan_text("fn main() {}").is_none());
    }

    #[test]
    fn urls() {
        assert!(valid_repo_url("https://github.com/mastodon/mastodon.git").is_ok());
        assert!(valid_repo_url("git@github.com:a/b.git").is_ok());
        assert!(valid_repo_url("https://user:tok@github.com/a/b").is_err());
        assert!(valid_repo_url("ext::sh -c touch /tmp/x").is_err());
        assert!(valid_repo_url("--upload-pack=evil").is_err());
        assert!(valid_repo_url("file:///etc").is_err());
        assert!(valid_repo_url("https://github.com/a b").is_err());
    }

    #[test]
    fn chemins_relatifs() {
        assert!(valid_rel_path("src/main.rs").is_ok());
        assert!(valid_rel_path("../x").is_err());
        assert!(valid_rel_path("/etc/passwd").is_err());
        assert!(valid_rel_path(".git/config").is_err());
    }

    #[test]
    fn noms() {
        assert!(valid_repo_slug("mastodon/mastodon").is_ok());
        assert!(valid_repo_slug("a/b/c").is_err());
        assert!(valid_repo_name("mon projet").is_err());
        assert!(valid_remote_name("-x").is_err());
    }

    #[test]
    fn perimetre() {
        let cwd = std::env::temp_dir();
        let cfg = Config { allowed_roots: vec![cwd.to_string_lossy().to_string()], ..Config::default() };
        let p = Policy::new(&cfg, &cwd);
        assert!(p.check_path(&cwd.join("drenn_ok_x")).is_ok());
        assert!(p.check_path(&cwd.join(".ssh").join("k")).is_err());
        assert!(p.check_path(Path::new("/etc")).is_err());
    }
}
