//! Interface console : messages sobres (ASCII), confirmations, choix.

use std::io::{self, BufRead, Write};

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
    println!("{}", s.as_ref());
}
pub fn info(s: impl AsRef<str>) {
    println!("[..] {}", s.as_ref());
}
pub fn ok(s: impl AsRef<str>) {
    println!("[OK] {}", s.as_ref());
}
pub fn warn(s: impl AsRef<str>) {
    println!("[!!] {}", s.as_ref());
}
pub fn err(s: impl AsRef<str>) {
    eprintln!("[ERREUR] {}", s.as_ref());
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
    match read_line(&format!("{question} [o/N] ")) {
        Some(a) => matches!(a.to_lowercase().as_str(), "o" | "oui" | "y" | "yes"),
        None => false,
    }
}

/// Confirmation forte : l'utilisateur doit recopier un mot.
pub fn confirm_typed(question: &str, word: &str) -> bool {
    match read_line(&format!("{question}\nTapez « {word} » pour confirmer : ")) {
        Some(a) => a == word,
        None => false,
    }
}

/// Propose une liste numérotée ; renvoie l'index choisi (Entrée = 1, 0 = annuler).
pub fn choose(question: &str, options: &[String]) -> Option<usize> {
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
