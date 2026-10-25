//! Intégration GitHub via l'API REST. Utilise `curl` (fourni avec Windows 10/11) : aucune dépendance TLS dans l'exécutable.
//! Le jeton est transmis à curl par l'entrée standard (jamais en argument de ligne de commande).

use crate::util::{drenn_dir, run_cmd, Res};
use serde_json::{json, Value};
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct RepoInfo {
    pub full_name: String,
    pub description: String,
    pub clone_url: String,
    pub private: bool,
    pub default_branch: String,
    pub stars: u64,
    pub updated_at: String,
    pub html_url: String,
}

impl RepoInfo {
    fn from_value(v: &Value) -> Option<RepoInfo> {
        Some(RepoInfo {
            full_name: v.get("full_name")?.as_str()?.to_string(),
            description: v.get("description").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            clone_url: v.get("clone_url").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            private: v.get("private").and_then(|d| d.as_bool()).unwrap_or(false),
            default_branch: v.get("default_branch").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            stars: v.get("stargazers_count").and_then(|d| d.as_u64()).unwrap_or(0),
            updated_at: v.get("updated_at").and_then(|d| d.as_str()).unwrap_or("").to_string(),
            html_url: v.get("html_url").and_then(|d| d.as_str()).unwrap_or("").to_string(),
        })
    }
}

pub struct GitHub {
    token: Option<String>,
}

pub fn token_file() -> std::path::PathBuf {
    drenn_dir().join("github_token")
}

pub fn load_token() -> Option<String> {
    for var in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(t) = std::env::var(var) {
            let t = t.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    std::fs::read_to_string(token_file()).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

fn curl_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r").replace('\t', "\\t")
}

impl GitHub {
    pub fn new(token: Option<String>) -> GitHub {
        GitHub { token }
    }
    pub fn has_token(&self) -> bool {
        self.token.is_some()
    }

    fn request(&self, method: &str, path: &str, body: Option<&Value>, auth: bool) -> Res<(u16, Value)> {
        if !path.starts_with('/') {
            return Err("chemin d'API invalide".into());
        }
        let mut cfg = String::new();
        cfg.push_str(&format!("url = \"{}\"\n", curl_escape(&format!("https://api.github.com{path}"))));
        cfg.push_str(&format!("request = \"{}\"\n", method));
        cfg.push_str("header = \"Accept: application/vnd.github+json\"\n");
        cfg.push_str("header = \"X-GitHub-Api-Version: 2022-11-28\"\n");
        cfg.push_str("header = \"User-Agent: git-drenn\"\n");
        if auth {
            let t = self.token.as_ref().ok_or("jeton GitHub requis : lancez `drenn auth login` ou définissez GITHUB_TOKEN")?;
            cfg.push_str(&format!("header = \"Authorization: Bearer {}\"\n", curl_escape(t)));
        }
        if let Some(b) = body {
            cfg.push_str("header = \"Content-Type: application/json\"\n");
            cfg.push_str(&format!("data = \"{}\"\n", curl_escape(&b.to_string())));
        }
        cfg.push_str("write-out = \"\\n%{http_code}\"\n");
        let mut cmd = Command::new("curl");
        cmd.args(["-sS", "--proto", "=https", "--max-time", "30", "-K", "-"]);
        let o = run_cmd(&mut cmd, Some(cfg.as_bytes()), Duration::from_secs(40))
            .map_err(|e| format!("curl indisponible ou délai dépassé : {e}"))?;
        if !o.ok() {
            let e = o.err_text();
            let e = crate::util::redact(&e, self.token.as_deref());
            return Err(format!("requête GitHub impossible (connexion Internet ?) : {e}"));
        }
        let (body_txt, code) = o.stdout.rsplit_once('\n').ok_or("réponse GitHub illisible")?;
        let code: u16 = code.trim().parse().map_err(|_| "réponse GitHub illisible")?;
        let val: Value = serde_json::from_str(body_txt).unwrap_or(Value::Null);
        Ok((code, val))
    }

    fn api_error(code: u16, v: &Value) -> String {
        let msg = v.get("message").and_then(|m| m.as_str()).unwrap_or("");
        match code {
            401 => "jeton GitHub invalide ou expiré (`drenn auth login`)".into(),
            403 => format!("accès refusé ou limite de requêtes atteinte ({msg})"),
            404 => "ressource introuvable (ou droits insuffisants)".into(),
            422 => format!("requête refusée par GitHub : {msg}"),
            _ => format!("erreur GitHub {code} : {msg}"),
        }
    }

    pub fn user_login(&self) -> Res<String> {
        let (c, v) = self.request("GET", "/user", None, true)?;
        if c != 200 {
            return Err(Self::api_error(c, &v));
        }
        v.get("login").and_then(|l| l.as_str()).map(|s| s.to_string()).ok_or_else(|| "identifiant GitHub introuvable".into())
    }

    pub fn search_repos(&self, query: &str) -> Res<Vec<RepoInfo>> {
        let q: String = query
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.' || *c == ' ')
            .collect();
        let q = q.trim().replace(' ', "+");
        if q.is_empty() {
            return Err("recherche vide".into());
        }
        let (c, v) = self.request("GET", &format!("/search/repositories?q={q}&sort=stars&per_page=5"), None, self.has_token())?;
        if c != 200 {
            return Err(Self::api_error(c, &v));
        }
        Ok(v.get("items").and_then(|i| i.as_array()).map(|a| a.iter().filter_map(RepoInfo::from_value).collect()).unwrap_or_default())
    }

    pub fn repo(&self, slug: &str) -> Res<Option<RepoInfo>> {
        crate::security::valid_repo_slug(slug)?;
        let (c, v) = self.request("GET", &format!("/repos/{slug}"), None, self.has_token())?;
        match c {
            200 => Ok(RepoInfo::from_value(&v)),
            404 => Ok(None),
            _ => Err(Self::api_error(c, &v)),
        }
    }

    pub fn list_repos(&self) -> Res<Vec<RepoInfo>> {
        let (c, v) = self.request("GET", "/user/repos?per_page=30&sort=updated", None, true)?;
        if c != 200 {
            return Err(Self::api_error(c, &v));
        }
        Ok(v.as_array().map(|a| a.iter().filter_map(RepoInfo::from_value).collect()).unwrap_or_default())
    }

    pub fn create_repo(&self, name: &str, private: bool, description: Option<&str>) -> Res<RepoInfo> {
        crate::security::valid_repo_name(name)?;
        let mut body = json!({"name": name, "private": private, "auto_init": false});
        if let Some(d) = description {
            body["description"] = json!(d);
        }
        let (c, v) = self.request("POST", "/user/repos", Some(&body), true)?;
        if c != 201 {
            return Err(Self::api_error(c, &v));
        }
        RepoInfo::from_value(&v).ok_or_else(|| "réponse GitHub inattendue".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn echappement_curl() {
        assert_eq!(curl_escape("a\"b\\c\n"), "a\\\"b\\\\c\\n");
    }
}
