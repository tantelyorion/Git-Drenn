//! Interface console : messages sobres (ASCII), confirmations, choix.

use std::io::{self, BufRead, Write};
use std::sync::OnceLock;

/// Canal d'interaction : console par défaut, remplaçable (interface graphique locale).
pub trait Io: Send + Sync {
    fn out(&self, kind: &str, text: &str);
    fn confirm(&self, q: &str) -> bool;
    fn typed(&self, q: &str, word: &str) -> bool;
    fn choose(&self, q: &str, opts: &[String]) -> Option<usize>;
}

static HANDLER: OnceLock<Box<dyn Io>> = OnceLock::new();

pub fn set_handler(h: Box<dyn Io>) -> bool {
    HANDLER.set(h).is_ok()
}

pub fn setup_console() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleOutputCP(id: u32) -> i32;
            fn SetConsoleCP(id: u32) -> i32;
        }
        unsafe {
            SetConsoleOutputCP(65001);
            SetConsoleCP(65001);
        }
    }
}

pub fn say(s: impl AsRef<str>) {
    match HANDLER.get() {
        Some(h) => h.out("say", s.as_ref()),
        None => println!("{}", s.as_ref()),
    }
}
pub fn info(s: impl AsRef<str>) {
    match HANDLER.get() {
        Some(h) => h.out("info", s.as_ref()),
        None => println!("[..] {}", s.as_ref()),
    }
}
pub fn ok(s: impl AsRef<str>) {
    match HANDLER.get() {
        Some(h) => h.out("ok", s.as_ref()),
        None => println!("[OK] {}", s.as_ref()),
    }
}
pub fn warn(s: impl AsRef<str>) {
    match HANDLER.get() {
        Some(h) => h.out("warn", s.as_ref()),
        None => println!("[!!] {}", s.as_ref()),
    }
}
pub fn err(s: impl AsRef<str>) {
    match HANDLER.get() {
        Some(h) => h.out("err", s.as_ref()),
        None => eprintln!("[ERREUR] {}", s.as_ref()),
    }
}

pub fn read_line(prompt: &str) -> Option<String> {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut l = String::new();
    match io::stdin().lock().read_line(&mut l) {
        Ok(0) | Err(_) => None,
        Ok(_) => Some(l.trim().to_string()),
    }
}

/// Réponse « oui » seulement sur o / oui / y / yes. Fin de flux (mode non interactif) = non.
pub fn confirm(question: &str) -> bool {
    if let Some(h) = HANDLER.get() {
        return h.confirm(question);
    }
    match read_line(&format!("{question} [o/N] ")) {
        Some(a) => matches!(a.to_lowercase().as_str(), "o" | "oui" | "y" | "yes"),
        None => false,
    }
}

/// Confirmation forte : l'utilisateur doit recopier un mot.
pub fn confirm_typed(question: &str, word: &str) -> bool {
    if let Some(h) = HANDLER.get() {
        return h.typed(question, word);
    }
    match read_line(&format!("{question}\nTapez « {word} » pour confirmer : ")) {
        Some(a) => a == word,
        None => false,
    }
}

/// Propose une liste numérotée ; renvoie l'index choisi (Entrée = 1, 0 = annuler).
pub fn choose(question: &str, options: &[String]) -> Option<usize> {
    if let Some(h) = HANDLER.get() {
        return h.choose(question, options);
    }
    println!("{question}");
    for (i, o) in options.iter().enumerate() {
        println!("  {}. {}", i + 1, o);
    }
    let a = read_line(&format!("Votre choix (1-{}, 0 = annuler) : ", options.len()))?;
    if a.is_empty() {
        return Some(0);
    }
    match a.parse::<usize>() {
        Ok(n) if n >= 1 && n <= options.len() => Some(n - 1),
        _ => None,
    }
}

/// Créateur du projet (affiché dans l'aide, « à propos » et l'interface graphique).
pub const CREATOR: &str = "Mr Tantely Orion";

/// Texte « À propos » commun à la console et à l'interface graphique.
pub fn about_text() -> String {
    format!(
        "GIT DRENN {} - Git Powered by AI\nCréateur : {}\nLicence : MIT\n\nVous décrivez ce que vous voulez faire en langage humain ;\nl'agent IA prépare un plan, le moteur le vérifie, vous confirmez, puis il l'exécute.",
        env!("CARGO_PKG_VERSION"),
        CREATOR
    )
}

/// Mascotte de GIT DRENN en ASCII (compatible avec toutes les consoles Windows).
pub fn banner(version: &str) {
    println!();
    println!("     .-----.");
    println!("    / (o o) \\    GIT DRENN {version} - Git Powered by AI");
    println!("   |   \\_/   |   Decrivez ce que vous voulez faire,");
    println!("    \\_______/    je m'occupe des operations Git.");
    println!("     /|[G]|\\");
    println!("   Créateur : {CREATOR}");
}
