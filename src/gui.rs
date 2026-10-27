//! Interface graphique locale : petit serveur HTTP (bibliothèque standard uniquement) lié à 127.0.0.1.
//!
//! Sécurité : jeton aléatoire obligatoire dans l'URL, contrôle des en-têtes Host (anti DNS-rebinding) et Origin (anti CSRF),
//! contenu servi avec une politique CSP stricte, tout texte issu des dépôts inséré via `textContent` (jamais en HTML).
//! Les confirmations de l'agent s'affichent dans des fenêtres ; les mots à recopier (PUBLIC, FORCER, ABANDONNER) sont
//! vérifiés côté serveur.

use crate::agent::Agent;
use crate::ui::{self, Io};
use crate::util::{now_secs, truncate, Res};
use serde_json::{json, Value};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

const PAGE: &str = include_str!("gui.html");
const MASCOT: &[u8] = include_bytes!("../assets/mascot-128.png");

#[derive(Clone)]
struct Prompt {
    id: u64,
    kind: String,
    text: String,
    word: String,
    options: Vec<String>,
}

#[derive(Default)]
struct State {
    events: Vec<(String, String)>,
    prompt: Option<Prompt>,
    answer: Option<(u64, String)>,
    running: bool,
    next_id: u64,
}

struct Shared {
    st: Mutex<State>,
    cv: Condvar,
}

impl Shared {
    fn push(&self, kind: &str, text: &str) {
        let mut st = self.st.lock().unwrap();
        st.events.push((kind.to_string(), truncate(text, 8000)));
    }

    fn ask(&self, kind: &str, text: &str, word: &str, options: &[String]) -> Option<String> {
        let mut st = self.st.lock().unwrap();
        st.next_id += 1;
        let id = st.next_id;
        st.prompt = Some(Prompt { id, kind: kind.into(), text: text.into(), word: word.into(), options: options.to_vec() });
        st.answer = None;
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            if let Some((aid, a)) = st.answer.take() {
                if aid == id {
                    st.prompt = None;
                    return Some(a);
                }
            }
            let now = Instant::now();
            if now >= deadline {
                st.prompt = None;
                return None;
            }
            st = self.cv.wait_timeout(st, deadline - now).unwrap().0;
        }
    }
}

struct GuiIo(Arc<Shared>);

impl Io for GuiIo {
    fn out(&self, kind: &str, text: &str) {
        self.0.push(kind, text);
    }
    fn confirm(&self, q: &str) -> bool {
        self.0.ask("confirm", q, "", &[]).map(|a| a == "yes").unwrap_or(false)
    }
    fn typed(&self, q: &str, word: &str) -> bool {
        self.0.ask("typed", q, word, &[]).map(|a| a == word).unwrap_or(false)
    }
    fn choose(&self, q: &str, opts: &[String]) -> Option<usize> {
        self.0.ask("choose", q, "", opts).and_then(|a| a.parse::<usize>().ok()).filter(|i| *i < opts.len())
    }
}

struct Ctx {
    agent: Arc<Agent>,
    shared: Arc<Shared>,
    token: String,
    port: u16,
}

fn random_token() -> String {
    let mut s = String::new();
    for i in 0..4usize {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0));
        h.write_usize(i);
        h.write_u32(std::process::id());
        s.push_str(&format!("{:016x}", h.finish()));
    }
    s
}

/// Démarre le serveur ; renvoie (port, jeton). Le serveur tourne dans des threads d'arrière-plan.
pub fn start(agent: Arc<Agent>) -> Res<(u16, String)> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("impossible d'ouvrir le port local : {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let shared = Arc::new(Shared { st: Mutex::new(State::default()), cv: Condvar::new() });
    if !ui::set_handler(Box::new(GuiIo(shared.clone()))) {
        return Err("le canal d'interaction est déjà configuré".into());
    }
    let token = random_token();
    let ctx = Arc::new(Ctx { agent, shared, token: token.clone(), port });
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let ctx = ctx.clone();
            std::thread::spawn(move || {
                let _ = handle_conn(conn, &ctx);
            });
        }
    });
    Ok((port, token))
}

/// `drenn gui` : démarre, ouvre le navigateur et attend.
pub fn serve(agent: Arc<Agent>, open_browser: bool) -> Res<()> {
    let (port, token) = start(agent)?;
    let url = format!("http://127.0.0.1:{port}/?t={token}");
    println!("GIT DRENN — interface locale : {url}");
    println!("(accessible uniquement depuis cet ordinateur ; Ctrl+C ou le bouton « Quitter » pour arrêter)");
    if open_browser {
        open_url(&url);
    }
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

fn open_url(url: &str) {
    use std::process::{Command, Stdio};
    // Windows : start ; macOS : open ; Linux et BSD : xdg-open puis solutions de repli.
    let candidates: Vec<(&str, Vec<&str>)> = if cfg!(windows) {
        vec![("cmd", vec!["/C", "start", "", url])]
    } else if cfg!(target_os = "macos") {
        vec![("open", vec![url])]
    } else {
        vec![("xdg-open", vec![url]), ("gio", vec!["open", url]), ("sensible-browser", vec![url]), ("x-www-browser", vec![url])]
    };
    for (prog, args) in candidates {
        if Command::new(prog).args(&args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().is_ok() {
            return;
        }
    }
    // Aucun ouvreur : l'adresse a déjà été affichée, il suffit de la copier dans un navigateur.
}

struct Req {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    host: String,
    origin: Option<String>,
    ctype: String,
    body: Vec<u8>,
}

fn read_request(s: &mut TcpStream) -> Result<Req, &'static str> {
    s.set_read_timeout(Some(Duration::from_secs(10))).map_err(|_| "timeout")?;
    let mut buf: Vec<u8> = Vec::new();
    let mut tmp = [0u8; 2048];
    let head_end = loop {
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        if buf.len() > 16 * 1024 {
            return Err("en-têtes trop grands");
        }
        let n = s.read(&mut tmp).map_err(|_| "lecture")?;
        if n == 0 {
            return Err("connexion fermée");
        }
        buf.extend_from_slice(&tmp[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let first = lines.next().ok_or("requête vide")?;
    let mut parts = first.split_whitespace();
    let method = parts.next().ok_or("méthode")?.to_string();
    let target = parts.next().ok_or("cible")?.to_string();
    let (path, q) = target.split_once('?').unwrap_or((&target, ""));
    let query = q
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (k.to_string(), v.to_string())
        })
        .collect();
    let (mut host, mut origin, mut ctype, mut len) = (String::new(), None, String::new(), 0usize);
    for l in lines {
        if let Some((k, v)) = l.split_once(':') {
            let v = v.trim();
            match k.trim().to_lowercase().as_str() {
                "host" => host = v.to_lowercase(),
                "origin" => origin = Some(v.to_lowercase()),
                "content-type" => ctype = v.to_lowercase(),
                "content-length" => len = v.parse().unwrap_or(0),
                _ => {}
            }
        }
    }
    if len > 64 * 1024 {
        return Err("corps trop grand");
    }
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = s.read(&mut tmp).map_err(|_| "lecture")?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&tmp[..n]);
    }
    body.truncate(len);
    Ok(Req { method, path: path.to_string(), query, host, origin, ctype, body })
}

fn respond(s: &mut TcpStream, code: u16, ctype: &str, body: &[u8]) {
    let reason = match code {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {code} {reason}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nX-Frame-Options: DENY\r\nContent-Security-Policy: default-src 'none'; img-src 'self'; style-src 'unsafe-inline'; script-src 'unsafe-inline'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = s.write_all(head.as_bytes());
    let _ = s.write_all(body);
}

fn json_resp(s: &mut TcpStream, code: u16, v: Value) {
    respond(s, code, "application/json; charset=utf-8", v.to_string().as_bytes());
}

fn handle_conn(mut s: TcpStream, ctx: &Ctx) -> Result<(), ()> {
    let req = match read_request(&mut s) {
        Ok(r) => r,
        Err(e) => {
            respond(&mut s, 400, "text/plain; charset=utf-8", e.as_bytes());
            return Err(());
        }
    };
    let ok_hosts = [format!("127.0.0.1:{}", ctx.port), format!("localhost:{}", ctx.port)];
    if !ok_hosts.contains(&req.host) {
        respond(&mut s, 403, "text/plain; charset=utf-8", "Hôte refusé".as_bytes());
        return Err(());
    }
    if let Some(o) = &req.origin {
        if !ok_hosts.iter().any(|h| *o == format!("http://{h}")) {
            respond(&mut s, 403, "text/plain; charset=utf-8", "Origine refusée".as_bytes());
            return Err(());
        }
    }
    let tok = req.query.iter().find(|(k, _)| k == "t").map(|(_, v)| v.as_str()).unwrap_or("");
    if tok != ctx.token {
        respond(&mut s, 403, "text/plain; charset=utf-8", "Accès refusé : utilisez l'adresse affichée par `drenn gui`.".as_bytes());
        return Err(());
    }
    if req.method == "POST" && !req.ctype.starts_with("application/json") {
        respond(&mut s, 400, "text/plain; charset=utf-8", b"JSON attendu");
        return Err(());
    }
    let body: Value = if req.method == "POST" { serde_json::from_slice(&req.body).unwrap_or(Value::Null) } else { Value::Null };

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/") => respond(&mut s, 200, "text/html; charset=utf-8", PAGE.as_bytes()),
        ("GET", "/mascot.png") => respond(&mut s, 200, "image/png", MASCOT),
        ("GET", "/api/info") => json_resp(&mut s, 200, json!({"version": env!("CARGO_PKG_VERSION"), "cwd": ctx.agent.cwd.display().to_string(), "creator": crate::ui::CREATOR, "about": crate::ui::about_text()})),
        ("GET", "/api/events") => {
            let since: usize = req.query.iter().find(|(k, _)| k == "since").and_then(|(_, v)| v.parse().ok()).unwrap_or(0);
            let st = ctx.shared.st.lock().unwrap();
            let since = since.min(st.events.len());
            let events: Vec<Value> = st.events[since..].iter().map(|(k, t)| json!([k, t])).collect();
            let prompt = st.prompt.as_ref().map(|p| json!({"id": p.id, "kind": p.kind, "text": p.text, "word": p.word, "options": p.options}));
            json_resp(&mut s, 200, json!({"events": events, "next": st.events.len(), "prompt": prompt, "running": st.running}));
        }
        ("POST", "/api/run") => {
            let instr = body.get("instruction").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
            if instr.is_empty() || instr.chars().count() > 1000 || instr.chars().any(|c| c.is_control()) {
                return Ok(json_resp(&mut s, 400, json!({"error": "instruction vide ou invalide"})));
            }
            {
                let mut st = ctx.shared.st.lock().unwrap();
                if st.running {
                    drop(st);
                    return Ok(json_resp(&mut s, 409, json!({"error": "une opération est déjà en cours"})));
                }
                st.running = true;
                st.events.push(("user".into(), instr.clone()));
            }
            let (agent, shared) = (ctx.agent.clone(), ctx.shared.clone());
            std::thread::spawn(move || {
                let ok = agent.handle(&instr);
                let mut st = shared.st.lock().unwrap();
                st.events.push(("done".into(), if ok { "Terminé.".into() } else { "Terminé (sans exécution complète).".into() }));
                st.running = false;
            });
            json_resp(&mut s, 200, json!({"started": true}));
        }
        ("POST", "/api/answer") => {
            let id = body.get("id").and_then(|v| v.as_u64()).unwrap_or(0);
            let a = body.get("answer").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let mut st = ctx.shared.st.lock().unwrap();
            let valid = st.prompt.as_ref().map(|p| p.id == id).unwrap_or(false);
            if valid {
                st.answer = Some((id, a));
                ctx.shared.cv.notify_all();
            }
            drop(st);
            json_resp(&mut s, 200, json!({"accepted": valid}));
        }
        ("POST", "/api/quit") => {
            json_resp(&mut s, 200, json!({"bye": true}));
            let _ = s.flush();
            crate::llm::shutdown_all();
            std::process::exit(0);
        }
        _ => respond(&mut s, 404, "text/plain; charset=utf-8", b"Introuvable"),
    }
    let _ = now_secs();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn http(port: u16, method: &str, target: &str, host: Option<&str>, body: Option<&str>) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let host = host.map(|h| h.to_string()).unwrap_or(format!("127.0.0.1:{port}"));
        let mut req = format!("{method} {target} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
        if let Some(b) = body {
            req += &format!("Content-Type: application/json\r\nContent-Length: {}\r\n", b.len());
        }
        req += "\r\n";
        if let Some(b) = body {
            req += b;
        }
        s.write_all(req.as_bytes()).unwrap();
        let mut out = Vec::new();
        s.read_to_end(&mut out).unwrap();
        let text = String::from_utf8_lossy(&out).to_string();
        let code = text.split_whitespace().nth(1).and_then(|c| c.parse().ok()).unwrap_or(0);
        let body = text.split_once("\r\n\r\n").map(|x| x.1.to_string()).unwrap_or_default();
        (code, body)
    }

    #[test]
    fn serveur_complet() {
        let base = std::env::temp_dir().join(format!("drenn_gui_{}", now_secs()));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("proj");
        std::fs::create_dir_all(&proj).unwrap();
        let cfg = Config { allowed_roots: vec![base.to_string_lossy().to_string()], ..Config::default() };
        let agent = Arc::new(Agent::new(cfg, base.clone(), None).unwrap());
        for a in [vec!["init", "-q", "-b", "main"], vec!["config", "user.name", "T"], vec!["config", "user.email", "t@t.t"]] {
            agent.git.out(&proj, &a, crate::gitengine::Kind::Local).unwrap();
        }
        std::fs::write(proj.join("a.txt"), "bonjour").unwrap();
        std::fs::write(proj.join(".env"), "S=1").unwrap();
        let (port, tok) = start(agent.clone()).unwrap();

        // Sécurité : sans jeton, mauvais jeton, mauvais Host.
        assert_eq!(http(port, "GET", "/", None, None).0, 403);
        assert_eq!(http(port, "GET", "/?t=faux", None, None).0, 403);
        assert_eq!(http(port, "GET", &format!("/?t={tok}"), Some("evil.example:80"), None).0, 403);
        // Page et mascotte.
        let (c, page) = http(port, "GET", &format!("/?t={tok}"), None, None);
        assert_eq!(c, 200);
        assert!(page.contains("GIT DRENN"));
        assert_eq!(http(port, "GET", &format!("/mascot.png?t={tok}"), None, None).0, 200);
        // Instruction invalide.
        assert_eq!(http(port, "POST", &format!("/api/run?t={tok}"), None, Some("{\"instruction\":\"\"}")).0, 400);

        // Instruction réelle avec confirmation du plan dans la « fenêtre ».
        let (c, _) = http(port, "POST", &format!("/api/run?t={tok}"), None, Some("{\"instruction\":\"enregistre les modifications du projet proj\"}"));
        assert_eq!(c, 200);
        let mut answered = false;
        let mut done = false;
        for _ in 0..200 {
            let (_, b) = http(port, "GET", &format!("/api/events?since=0&t={tok}"), None, None);
            let v: Value = serde_json::from_str(&b).unwrap();
            if let Some(p) = v["prompt"].as_object() {
                if !answered {
                    assert_eq!(p["kind"], "confirm");
                    let body = format!("{{\"id\":{},\"answer\":\"yes\"}}", p["id"]);
                    let (_, r) = http(port, "POST", &format!("/api/answer?t={tok}"), None, Some(&body));
                    assert!(r.contains("true"));
                    answered = true;
                }
            }
            if v["events"].as_array().unwrap().iter().any(|e| e[0] == "done") {
                done = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(answered && done, "confirmation ou fin d'opération manquante");
        let files = agent.git.line(&proj, &["ls-files"]).unwrap();
        assert_eq!(files.trim(), "a.txt", "le .env ne doit pas être enregistré");
    }
}
