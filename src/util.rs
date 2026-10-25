//! Utilitaires communs : exécution de processus avec délai, chemins, JSON, texte.

use serde_json::Value;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub type Res<T> = Result<T, String>;

#[derive(Debug, Clone)]
pub struct Out {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl Out {
    pub fn ok(&self) -> bool {
        self.code == 0
    }
    /// Texte d'erreur le plus parlant (stderr, sinon stdout).
    pub fn err_text(&self) -> String {
        let t = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        t.trim().to_string()
    }
}

/// Exécute une commande (sans shell), avec délai maximal. Le processus est tué au-delà.
pub fn run_cmd(cmd: &mut Command, input: Option<&[u8]>, timeout: Duration) -> Res<Out> {
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().map_err(|e| format!("lancement impossible : {e}"))?;
    if let Some(data) = input {
        if let Some(mut si) = child.stdin.take() {
            let _ = si.write_all(data);
        }
    }
    let mut so = child.stdout.take().ok_or("stdout indisponible")?;
    let mut se = child.stderr.take().ok_or("stderr indisponible")?;
    let h1 = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = so.read_to_end(&mut b);
        b
    });
    let h2 = thread::spawn(move || {
        let mut b = Vec::new();
        let _ = se.read_to_end(&mut b);
        b
    });
    let start = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break st,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(format!("délai dépassé ({} s) — opération interrompue", timeout.as_secs()));
                }
                thread::sleep(Duration::from_millis(15));
            }
            Err(e) => return Err(format!("attente du processus impossible : {e}")),
        }
    };
    let so = h1.join().unwrap_or_default();
    let se = h2.join().unwrap_or_default();
    Ok(Out {
        code: status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&so).into_owned(),
        stderr: String::from_utf8_lossy(&se).into_owned(),
    })
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

pub fn drenn_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("DRENN_HOME") {
        return PathBuf::from(d);
    }
    home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".drenn")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Retire le préfixe `\\?\` produit par canonicalize() sous Windows.
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        if !rest.starts_with("UNC\\") {
            return PathBuf::from(rest);
        }
    }
    p
}

/// Canonicalise un chemin même s'il n'existe pas encore (ancêtre existant + reste).
pub fn canonicalize_lenient(p: &Path) -> Res<PathBuf> {
    if let Ok(c) = p.canonicalize() {
        return Ok(strip_verbatim(c));
    }
    let mut rest: Vec<OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    while !cur.exists() {
        let name = cur.file_name().map(|s| s.to_os_string());
        let parent = cur.parent().map(|x| x.to_path_buf());
        match (name, parent) {
            (Some(n), Some(par)) => {
                rest.push(n);
                cur = par;
            }
            _ => return Err(format!("chemin invalide : {}", p.display())),
        }
    }
    let mut base = strip_verbatim(cur.canonicalize().map_err(|e| format!("chemin invalide : {e}"))?);
    for n in rest.into_iter().rev() {
        base.push(n);
    }
    Ok(base)
}

pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let t: String = s.chars().take(max).collect();
    format!("{t}… [tronqué]")
}

pub fn redact(s: &str, secret: Option<&str>) -> String {
    match secret {
        Some(t) if t.len() >= 6 => s.replace(t, "***"),
        _ => s.to_string(),
    }
}

/// Minuscules + suppression des accents courants du français.
pub fn normalize(s: &str) -> String {
    s.chars()
        .map(|c| {
            let c = c.to_lowercase().next().unwrap_or(c);
            match c {
                'à' | 'â' | 'ä' => 'a',
                'é' | 'è' | 'ê' | 'ë' => 'e',
                'î' | 'ï' => 'i',
                'ô' | 'ö' => 'o',
                'ù' | 'û' | 'ü' => 'u',
                'ç' => 'c',
                '’' | '‘' => '\'',
                _ => c,
            }
        })
        .collect()
}

pub fn slugify(words: &[&str]) -> String {
    let joined = words.join("-");
    let mut out = String::new();
    let mut last_dash = false;
    for c in normalize(&joined).chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

/// Extrait le premier objet JSON équilibré d'un texte (sortie brute d'un modèle).
pub fn extract_json(text: &str) -> Option<Value> {
    for (s, _) in text.match_indices('{') {
        if let Some(len) = balanced_len(&text[s..]) {
            if let Ok(v) = serde_json::from_str::<Value>(&text[s..s + len]) {
                if v.is_object() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn balanced_len(t: &str) -> Option<usize> {
    let (mut depth, mut in_str, mut esc) = (0i32, false, false);
    for (i, c) in t.char_indices() {
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
        } else {
            match c {
                '"' => in_str = true,
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(i + 1);
                    }
                }
                _ => {}
            }
        }
    }
    None
}

pub fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for ch in input.chunks(3) {
        let b = [ch[0], *ch.get(1).unwrap_or(&0), *ch.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_ok() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"x-access-token:abc"), "eC1hY2Nlc3MtdG9rZW46YWJj");
    }

    #[test]
    fn json_extraction() {
        let t = "bla {\"a\": {\"b\": \"}\"}, \"c\": 1} fin {\"x\":2}";
        let v = extract_json(t).unwrap();
        assert_eq!(v["c"], 1);
        assert!(extract_json("pas de json").is_none());
    }

    #[test]
    fn normalisation() {
        assert_eq!(normalize("Récupère à l'État"), "recupere a l'etat");
        assert_eq!(slugify(&["nouveau", "système", "connexion"]), "nouveau-systeme-connexion");
    }

    #[test]
    fn redaction() {
        assert_eq!(redact("token=abcdef123", Some("abcdef123")), "token=***");
    }
}
