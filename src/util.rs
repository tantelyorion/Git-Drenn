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

// ---------------------------------------------------------------- plateformes

/// Nom court du système : windows, macos, linux, freebsd, openbsd, netbsd, dragonfly.
pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "freebsd") {
        "freebsd"
    } else if cfg!(target_os = "openbsd") {
        "openbsd"
    } else if cfg!(target_os = "netbsd") {
        "netbsd"
    } else if cfg!(target_os = "dragonfly") {
        "dragonfly"
    } else {
        "linux"
    }
}

/// Nom d'un exécutable selon le système (`llama-cli` → `llama-cli.exe` sous Windows).
pub fn exe_name(base: &str) -> String {
    if cfg!(windows) { format!("{base}.exe") } else { base.to_string() }
}

pub fn git_install_hint() -> &'static str {
    match os_name() {
        "windows" => "Installez Git pour Windows : https://git-scm.com/download/win",
        "macos" => "Installez Git : `xcode-select --install` ou `brew install git`",
        "freebsd" | "dragonfly" => "Installez Git : `pkg install git`",
        "openbsd" => "Installez Git : `pkg_add git`",
        "netbsd" => "Installez Git : `pkgin install git`",
        _ => "Installez Git avec votre gestionnaire de paquets (ex. `sudo apt install git`, `sudo dnf install git`, `sudo pacman -S git`)",
    }
}

pub fn curl_install_hint() -> &'static str {
    match os_name() {
        "windows" => "curl.exe est fourni avec Windows 10/11 : mettez Windows à jour",
        "macos" => "curl est fourni avec macOS ; sinon `brew install curl`",
        "freebsd" | "dragonfly" => "`pkg install curl`",
        "openbsd" => "`pkg_add curl`",
        "netbsd" => "`pkgin install curl`",
        _ => "installez curl avec votre gestionnaire de paquets (ex. `sudo apt install curl`)",
    }
}

/// Mémoire vive totale en Mo (meilleure estimation, sans dépendance).
pub fn total_ram_mb() -> Option<u64> {
    let t = Duration::from_secs(10);
    let sysctl = |key: &str| -> Option<u64> {
        let o = run_cmd(Command::new("sysctl").args(["-n", key]), None, t).ok()?;
        if !o.ok() {
            return None;
        }
        o.stdout.trim().parse::<u64>().ok().map(|b| b / 1024 / 1024)
    };
    match os_name() {
        "linux" => {
            let m = std::fs::read_to_string("/proc/meminfo").ok()?;
            let kb: u64 = m.lines().find(|l| l.starts_with("MemTotal:"))?.split_whitespace().nth(1)?.parse().ok()?;
            Some(kb / 1024)
        }
        "macos" => sysctl("hw.memsize"),
        "netbsd" => sysctl("hw.physmem64"),
        "freebsd" | "openbsd" | "dragonfly" => sysctl("hw.physmem"),
        "windows" => {
            let o = run_cmd(
                Command::new("powershell").args(["-NoProfile", "-Command", "(Get-CimInstance Win32_ComputerSystem).TotalPhysicalMemory"]),
                None,
                Duration::from_secs(20),
            )
            .ok()?;
            o.stdout.trim().parse::<u64>().ok().map(|b| b / 1024 / 1024)
        }
        _ => None,
    }
}

/// Cherche un exécutable dans le PATH puis dans des dossiers supplémentaires (recherche récursive limitée).
pub fn find_binary(base: &str, extra_dirs: &[PathBuf]) -> Option<PathBuf> {
    let name = exe_name(base);
    if let Some(paths) = std::env::var_os("PATH") {
        for d in std::env::split_paths(&paths) {
            let c = d.join(&name);
            if c.is_file() {
                return Some(c);
            }
        }
    }
    fn walk(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
        let rd = std::fs::read_dir(dir).ok()?;
        let mut subdirs = Vec::new();
        for e in rd.flatten() {
            let p = e.path();
            if p.is_file() && p.file_name().map(|n| n == name).unwrap_or(false) {
                return Some(p);
            }
            if depth > 0 && p.is_dir() {
                subdirs.push(p);
            }
        }
        subdirs.into_iter().find_map(|d| walk(&d, name, depth - 1))
    }
    extra_dirs.iter().find_map(|d| walk(d, &name, 3))
}

/// SHA-256 en flux (sans dépendance), pour vérifier les modèles téléchargés.
pub fn sha256_reader<R: Read>(mut r: R) -> std::io::Result<String> {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be,
        0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa,
        0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85,
        0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3,
        0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f,
        0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
    ];
    let mut h: [u32; 8] = [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    fn block(h: &mut [u32; 8], b: &[u8], k: &[u32; 64]) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b_, mut c, mut d, mut e, mut f, mut g, mut hh] = *h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh.wrapping_add(s1).wrapping_add(ch).wrapping_add(k[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b_) ^ (a & c) ^ (b_ & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b_;
            b_ = a;
            a = t1.wrapping_add(t2);
        }
        for (x, y) in h.iter_mut().zip([a, b_, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut buf = vec![0u8; 1 << 16];
    let mut pending: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    loop {
        let n = r.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        pending.extend_from_slice(&buf[..n]);
        let full = pending.len() / 64 * 64;
        for c in pending[..full].chunks(64) {
            block(&mut h, c, &K);
        }
        pending.drain(..full);
    }
    pending.push(0x80);
    while pending.len() % 64 != 56 {
        pending.push(0);
    }
    pending.extend_from_slice(&(total * 8).to_be_bytes());
    for c in pending.chunks(64) {
        block(&mut h, c, &K);
    }
    Ok(h.iter().map(|x| format!("{x:08x}")).collect())
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
    #[test]
    fn sha256_vecteurs_connus() {
        assert_eq!(sha256_reader(&b"abc"[..]).unwrap(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(sha256_reader(&b""[..]).unwrap(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        let long = vec![b'a'; 1_000_000];
        assert_eq!(sha256_reader(&long[..]).unwrap(), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
    }

    #[test]
    fn plateforme_connue() {
        assert!(["windows", "macos", "linux", "freebsd", "openbsd", "netbsd", "dragonfly"].contains(&os_name()));
        assert!(!git_install_hint().is_empty() && !curl_install_hint().is_empty());
        assert_eq!(exe_name("x").ends_with(".exe"), cfg!(windows));
    }

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
