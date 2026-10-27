//! `drenn model` : choix, téléchargement vérifié et configuration du modèle IA ; détection du moteur llama.cpp.
//! Fonctionne sous Windows, macOS, Linux et BSD (téléchargement par `curl`, vérification SHA-256 intégrée).

use crate::config::Config;
use crate::ui;
use crate::util::*;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

pub struct Preset {
    pub id: &'static str,
    pub label: &'static str,
    pub file: &'static str,
    pub url: &'static str,
    pub size_mb: u64,
    pub min_ram_mb: u64,
}

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "qwen2.5-0.5b",
        label: "Qwen2.5 0.5B Instruct (très léger, moins fiable)",
        file: "qwen2.5-0.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf",
        size_mb: 400,
        min_ram_mb: 1500,
    },
    Preset {
        id: "qwen2.5-1.5b",
        label: "Qwen2.5 1.5B Instruct (recommandé pour 4 Go de RAM)",
        file: "qwen2.5-1.5b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct-GGUF/resolve/main/qwen2.5-1.5b-instruct-q4_k_m.gguf",
        size_mb: 1100,
        min_ram_mb: 3000,
    },
    Preset {
        id: "qwen2.5-3b",
        label: "Qwen2.5 3B Instruct (plus précis, 8 Go de RAM et plus)",
        file: "qwen2.5-3b-instruct-q4_k_m.gguf",
        url: "https://huggingface.co/Qwen/Qwen2.5-3B-Instruct-GGUF/resolve/main/qwen2.5-3b-instruct-q4_k_m.gguf",
        size_mb: 2000,
        min_ram_mb: 6500,
    },
];

pub fn recommended(ram_mb: Option<u64>) -> &'static Preset {
    let ram = ram_mb.unwrap_or(4096);
    PRESETS.iter().rev().find(|p| ram >= p.min_ram_mb).unwrap_or(&PRESETS[0])
}

pub fn models_dir() -> PathBuf {
    drenn_dir().join("models")
}

pub fn run(cfg: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        None | Some("list") => list(),
        Some("install") => install(cfg, args.get(1).map(|s| s.as_str())),
        Some("engine") => engine(cfg),
        _ => {
            ui::err("usage : drenn model [list | install [identifiant] | engine]");
            2
        }
    }
}

fn list() -> i32 {
    let ram = total_ram_mb();
    ui::say(format!("Système : {} · mémoire vive détectée : {}", os_name(), ram.map(|r| format!("{:.1} Go", r as f64 / 1024.0)).unwrap_or_else(|| "inconnue".into())));
    let rec = recommended(ram);
    for p in PRESETS {
        let present = if models_dir().join(p.file).is_file() { " [installé]" } else { "" };
        ui::say(format!("  {:<14} {}  ~{} Mo{}{}", p.id, p.label, p.size_mb, if p.id == rec.id { "  <= conseillé pour votre PC" } else { "" }, present));
    }
    ui::say("\nInstaller : drenn model install [identifiant]   ·   moteur llama.cpp : drenn model engine");
    ui::say("Autre modèle : téléchargez un .gguf « Instruct » puis `drenn config set model_path <fichier>` (voir docs/GUIDE_MODELE_IA.md).");
    0
}

/// (taille en octets, empreinte SHA-256 annoncée par Hugging Face) d'après les en-têtes HTTP.
fn head_info(url: &str) -> (Option<u64>, Option<String>) {
    let o = match run_cmd(Command::new("curl").args(["-sIL", "--proto", "=https", "--max-time", "30", url]), None, Duration::from_secs(40)) {
        Ok(o) if o.ok() => o,
        _ => return (None, None),
    };
    let (mut size, mut sha) = (None, None);
    for l in o.stdout.lines() {
        let l = l.trim().to_lowercase();
        if let Some(v) = l.strip_prefix("content-length:") {
            if let Ok(n) = v.trim().parse::<u64>() {
                size = Some(n);
            }
        }
        if let Some(v) = l.strip_prefix("x-linked-etag:") {
            let h = v.trim().trim_matches('"').to_string();
            if h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) {
                sha = Some(h);
            }
        }
    }
    (size, sha)
}

fn install(cfg: &Config, id: Option<&str>) -> i32 {
    let ram = total_ram_mb();
    let preset = match id {
        None => recommended(ram),
        Some(i) => match PRESETS.iter().find(|p| p.id == i) {
            Some(p) => p,
            None => {
                ui::err(format!("modèle inconnu « {i} » : voyez `drenn model list`"));
                return 2;
            }
        },
    };
    if let Some(r) = ram {
        if r < preset.min_ram_mb {
            ui::warn(format!("Ce modèle demande environ {:.1} Go de RAM ; votre PC en a {:.1} Go. Un modèle plus petit sera plus sûr.", preset.min_ram_mb as f64 / 1024.0, r as f64 / 1024.0));
        }
    }
    let dir = models_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        ui::err(format!("dossier {} inaccessible : {e}", dir.display()));
        return 1;
    }
    let dest = dir.join(preset.file);
    if dest.is_file() {
        ui::ok(format!("Déjà présent : {}", dest.display()));
        return finish(cfg, &dest);
    }
    ui::say(format!("Modèle : {}\nSource : {}", preset.label, preset.url));
    let (size, sha) = head_info(preset.url);
    match size {
        Some(n) => ui::say(format!("Taille à télécharger : {} Mo", n / 1024 / 1024)),
        None => ui::warn(format!("Taille non obtenue (Internet ? curl : {}) — environ {} Mo attendus.", curl_install_hint(), preset.size_mb)),
    }
    if !ui::confirm("Télécharger maintenant ?") {
        ui::say("Annulé.");
        return 1;
    }
    let part = dir.join(format!("{}.part", preset.file));
    // -C - : reprend un téléchargement interrompu. La progression s'affiche directement.
    let st = Command::new("curl").args(["-L", "--fail", "--proto", "=https", "--retry", "3", "-C", "-", "-o"]).arg(&part).arg(preset.url).status();
    match st {
        Ok(s) if s.success() => {}
        Ok(s) => {
            ui::err(format!("téléchargement interrompu (code {:?}) : relancez la même commande, il reprendra là où il s'est arrêté", s.code()));
            return 1;
        }
        Err(e) => {
            ui::err(format!("curl introuvable ({e}) : {}", curl_install_hint()));
            return 1;
        }
    }
    let hash = match std::fs::File::open(&part).and_then(|f| sha256_reader(std::io::BufReader::new(f))) {
        Ok(h) => h,
        Err(e) => {
            ui::err(format!("lecture du fichier téléchargé impossible : {e}"));
            return 1;
        }
    };
    match sha {
        Some(expected) if expected == hash => ui::ok("Intégrité vérifiée : l'empreinte SHA-256 correspond à celle publiée par Hugging Face."),
        Some(expected) => {
            ui::err(format!("EMPREINTE DIFFÉRENTE (attendue {expected}, obtenue {hash}) : fichier supprimé, rien n'est installé."));
            let _ = std::fs::remove_file(&part);
            return 1;
        }
        None => ui::warn(format!("Empreinte SHA-256 : {hash} (aucune empreinte publiée obtenue : comparez-la à celle de la page du modèle).")),
    }
    if let Err(e) = std::fs::rename(&part, &dest) {
        ui::err(format!("installation du fichier impossible : {e}"));
        return 1;
    }
    ui::ok(format!("Modèle installé : {}", dest.display()));
    finish(cfg, &dest)
}

fn finish(cfg: &Config, dest: &std::path::Path) -> i32 {
    let mut c = cfg.clone();
    c.model_path = Some(dest.display().to_string());
    if let Err(e) = c.save() {
        ui::err(e);
        return 1;
    }
    ui::ok("Configuration mise à jour (model_path).");
    if c.llama_bin.is_none() && c.llama_server_bin.is_none() && c.server_url.is_none() {
        ui::say("Il manque le moteur : lancez `drenn model engine`.");
    } else {
        ui::say("Vérifiez avec `drenn doctor`, puis `drenn bench`.");
    }
    0
}

fn engine(cfg: &Config) -> i32 {
    let mut extra: Vec<PathBuf> = vec![models_dir().join("llama.cpp")];
    for d in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/pkg/bin", "/usr/bin"] {
        extra.push(PathBuf::from(d));
    }
    if let Some(h) = home_dir() {
        extra.push(h.join(".local").join("bin"));
        extra.push(h.join("llama.cpp").join("build").join("bin"));
    }
    let server = find_binary("llama-server", &extra);
    let cli = find_binary("llama-cli", &extra);
    let mut c = cfg.clone();
    let mut changed = false;
    if let Some(s) = &server {
        ui::ok(format!("llama-server trouvé : {}", s.display()));
        if c.llama_server_bin.is_none() {
            c.llama_server_bin = Some(s.display().to_string());
            changed = true;
        }
    }
    if let Some(s) = &cli {
        ui::ok(format!("llama-cli trouvé : {}", s.display()));
        if c.llama_bin.is_none() {
            c.llama_bin = Some(s.display().to_string());
            changed = true;
        }
    }
    if server.is_none() && cli.is_none() {
        ui::warn("llama.cpp (llama-server / llama-cli) est introuvable sur ce PC.");
        ui::say(engine_help());
        return 1;
    }
    if server.is_none() {
        ui::info("llama-server absent : GIT DRENN utilisera llama-cli (modèle rechargé à chaque demande, plus lent).");
    }
    if changed {
        if let Err(e) = c.save() {
            ui::err(e);
            return 1;
        }
        ui::ok("Configuration mise à jour.");
    }
    ui::say("Ensuite : drenn model install   puis   drenn doctor");
    0
}

pub fn engine_help() -> String {
    let how = match os_name() {
        "windows" => "  • winget install llama.cpp   (ou téléchargez l'archive « win-cpu-x64 » sur https://github.com/ggml-org/llama.cpp/releases,\n    ou lancez scripts\\setup-model.ps1 qui s'en charge)",
        "macos" => "  • brew install llama.cpp\n  • ou l'archive « macos-arm64 » / « macos-x64 » : https://github.com/ggml-org/llama.cpp/releases",
        "freebsd" | "dragonfly" => "  • pkg install llama-cpp   (sinon : cd /usr/ports/misc/llama-cpp && make install)\n  • ou compilation depuis les sources (ci-dessous)",
        "openbsd" | "netbsd" => "  • compilation depuis les sources (ci-dessous) : pas d'archive officielle pour ce système",
        _ => "  • brew install llama.cpp   (Homebrew sous Linux)  ·  ou le paquet de votre distribution\n  • ou l'archive « ubuntu-x64 » : https://github.com/ggml-org/llama.cpp/releases",
    };
    format!(
        "Installer llama.cpp :\n{how}\n  • Compilation depuis les sources (tous systèmes, cmake + compilateur C++) :\n      git clone https://github.com/ggml-org/llama.cpp && cd llama.cpp\n      cmake -B build && cmake --build build --config Release -j\n    puis `drenn model engine` (cherche aussi dans ~/llama.cpp/build/bin).\n  • Alternative : Ollama ou LM Studio déjà installé → `drenn config set server_url http://127.0.0.1:11434` et `drenn config set server_model <nom>`.\n  Les noms de paquets peuvent évoluer : vérifiez sur la page du projet si une commande échoue."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modele_conseille_selon_la_ram() {
        assert_eq!(recommended(Some(2048)).id, "qwen2.5-0.5b");
        assert_eq!(recommended(Some(4096)).id, "qwen2.5-1.5b");
        assert_eq!(recommended(Some(16384)).id, "qwen2.5-3b");
        assert_eq!(recommended(None).id, "qwen2.5-1.5b");
    }

    #[test]
    fn identifiants_uniques_et_https() {
        for p in PRESETS {
            assert!(p.url.starts_with("https://") && p.url.ends_with(p.file) && p.file.ends_with(".gguf"));
            assert_eq!(PRESETS.iter().filter(|q| q.id == p.id).count(), 1);
        }
    }
}
