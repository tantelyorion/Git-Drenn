//! Résolution de conflits de fusion zone par zone (fonctions pures, travaillent sur les octets : fins de ligne et
//! encodages d'origine conservés). Les marqueurs reconnus sont ceux de Git (`<<<<<<<`, `|||||||`, `=======`, `>>>>>>>`).

use crate::util::{truncate, Res};

#[derive(Debug, Clone, PartialEq)]
pub struct Hunk {
    pub ours: Vec<u8>,
    pub base: Option<Vec<u8>>,
    pub theirs: Vec<u8>,
    pub ours_label: String,
    pub theirs_label: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Seg {
    Text(Vec<u8>),
    Hunk(Hunk),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Choice {
    /// Version de la branche courante.
    Ours,
    /// Version de la branche fusionnée.
    Theirs,
    /// Les deux : la courante puis la fusionnée.
    Both,
}

fn trim_eol(l: &[u8]) -> &[u8] {
    let mut e = l.len();
    while e > 0 && (l[e - 1] == b'\n' || l[e - 1] == b'\r') {
        e -= 1;
    }
    &l[..e]
}

/// Ligne de marqueur : 7 fois `ch`, suivie d'un espace ou de la fin de ligne.
fn marker(l: &[u8], ch: u8) -> bool {
    let t = trim_eol(l);
    t.len() >= 7 && t[..7].iter().all(|&c| c == ch) && (t.len() == 7 || t[7] == b' ')
}

fn label(l: &[u8]) -> String {
    let t = trim_eol(l);
    String::from_utf8_lossy(if t.len() > 8 { &t[8..] } else { &[] }).trim().to_string()
}

pub fn parse(data: &[u8]) -> Res<Vec<Seg>> {
    let mut segs: Vec<Seg> = Vec::new();
    let mut text: Vec<u8> = Vec::new();
    let mut cur: Option<Hunk> = None;
    let mut state = 0u8; // 0 texte, 1 « ours », 2 base (diff3), 3 « theirs »
    let mut i = 0;
    while i < data.len() {
        let end = data[i..].iter().position(|&b| b == b'\n').map(|p| i + p + 1).unwrap_or(data.len());
        let line = &data[i..end];
        i = end;
        match state {
            0 => {
                if marker(line, b'<') {
                    if !text.is_empty() {
                        segs.push(Seg::Text(std::mem::take(&mut text)));
                    }
                    cur = Some(Hunk { ours: vec![], base: None, theirs: vec![], ours_label: label(line), theirs_label: String::new() });
                    state = 1;
                } else {
                    text.extend_from_slice(line);
                }
            }
            1 | 2 | 3 => {
                let h = cur.as_mut().ok_or("conflit mal formé")?;
                if marker(line, b'<') {
                    return Err("conflit mal formé (zones imbriquées)".into());
                } else if state == 1 && marker(line, b'|') {
                    h.base = Some(vec![]);
                    state = 2;
                } else if state < 3 && trim_eol(line) == b"=======" {
                    state = 3;
                } else if state == 3 && marker(line, b'>') {
                    h.theirs_label = label(line);
                    segs.push(Seg::Hunk(cur.take().ok_or("conflit mal formé")?));
                    state = 0;
                } else {
                    match state {
                        1 => h.ours.extend_from_slice(line),
                        2 => h.base.get_or_insert_with(Vec::new).extend_from_slice(line),
                        _ => h.theirs.extend_from_slice(line),
                    }
                }
            }
            _ => unreachable!(),
        }
    }
    if state != 0 {
        return Err("conflit mal formé (marqueur de fin absent)".into());
    }
    if !text.is_empty() {
        segs.push(Seg::Text(text));
    }
    Ok(segs)
}

pub fn count(segs: &[Seg]) -> usize {
    segs.iter().filter(|s| matches!(s, Seg::Hunk(_))).count()
}

pub fn hunks(segs: &[Seg]) -> Vec<&Hunk> {
    segs.iter().filter_map(|s| if let Seg::Hunk(h) = s { Some(h) } else { None }).collect()
}

/// Reconstruit le fichier selon un choix par zone (dans l'ordre d'apparition).
pub fn render(segs: &[Seg], choices: &[Choice]) -> Res<Vec<u8>> {
    if choices.len() != count(segs) {
        return Err("nombre de choix différent du nombre de zones".into());
    }
    let mut out = Vec::new();
    let mut k = 0;
    for s in segs {
        match s {
            Seg::Text(t) => out.extend_from_slice(t),
            Seg::Hunk(h) => {
                match choices[k] {
                    Choice::Ours => out.extend_from_slice(&h.ours),
                    Choice::Theirs => out.extend_from_slice(&h.theirs),
                    Choice::Both => {
                        out.extend_from_slice(&h.ours);
                        if !h.ours.is_empty() && !h.ours.ends_with(b"\n") {
                            out.push(b'\n');
                        }
                        out.extend_from_slice(&h.theirs);
                    }
                }
                k += 1;
            }
        }
    }
    Ok(out)
}

/// Aperçu lisible et sûr d'un côté d'une zone (séquences d'échappement neutralisées, lignes tronquées).
pub fn preview(b: &[u8], max_lines: usize) -> String {
    if b.is_empty() {
        return "    (vide)".into();
    }
    let txt = String::from_utf8_lossy(b);
    let lines: Vec<&str> = txt.lines().collect();
    let mut out: Vec<String> = lines
        .iter()
        .take(max_lines)
        .map(|l| {
            let c: String = l.chars().map(|c| if c.is_control() && c != '\t' { ' ' } else { c }).collect();
            format!("    | {}", truncate(&c, 110))
        })
        .collect();
    if lines.len() > max_lines {
        out.push(format!("    | … (+{} ligne(s))", lines.len() - max_lines));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: &[u8] = b"debut\n<<<<<<< HEAD\nma ligne\n=======\nleur ligne\n>>>>>>> feature\nmilieu\n<<<<<<< HEAD\nA\n=======\nB\n>>>>>>> feature\nfin\n";

    #[test]
    fn analyse_deux_zones() {
        let s = parse(F).unwrap();
        assert_eq!(count(&s), 2);
        let h = hunks(&s);
        assert_eq!(h[0].ours, b"ma ligne\n");
        assert_eq!(h[0].theirs, b"leur ligne\n");
        assert_eq!(h[0].ours_label, "HEAD");
        assert_eq!(h[0].theirs_label, "feature");
    }

    #[test]
    fn choix_par_zone() {
        let s = parse(F).unwrap();
        let r = render(&s, &[Choice::Ours, Choice::Theirs]).unwrap();
        assert_eq!(r, b"debut\nma ligne\nmilieu\nB\nfin\n");
        let r = render(&s, &[Choice::Both, Choice::Ours]).unwrap();
        assert_eq!(r, b"debut\nma ligne\nleur ligne\nmilieu\nA\nfin\n");
        assert!(render(&s, &[Choice::Ours]).is_err());
    }

    #[test]
    fn fins_de_ligne_windows_conservees() {
        let f = b"a\r\n<<<<<<< HEAD\r\nx\r\n=======\r\ny\r\n>>>>>>> b\r\nz\r\n";
        let s = parse(f).unwrap();
        assert_eq!(render(&s, &[Choice::Theirs]).unwrap(), b"a\r\ny\r\nz\r\n");
        assert_eq!(render(&s, &[Choice::Both]).unwrap(), b"a\r\nx\r\ny\r\nz\r\n");
    }

    #[test]
    fn format_diff3_et_cas_limites() {
        let f = b"<<<<<<< HEAD\nA\n||||||| base\nO\n=======\nB\n>>>>>>> t\n";
        let s = parse(f).unwrap();
        assert_eq!(hunks(&s)[0].base.as_deref(), Some(&b"O\n"[..]));
        // « ======= » hors conflit (titre markdown) : simple texte.
        let md = b"Titre\n=======\ntexte\n";
        assert_eq!(count(&parse(md).unwrap()), 0);
        // vide d'un côté ; pas de retour à la ligne final
        let f = b"<<<<<<< HEAD\n=======\nB\n>>>>>>> t";
        let s = parse(f).unwrap();
        assert_eq!(render(&s, &[Choice::Ours]).unwrap(), b"");
        assert_eq!(render(&s, &[Choice::Theirs]).unwrap(), b"B\n");
    }

    #[test]
    fn fichiers_mal_formes_refuses() {
        assert!(parse(b"<<<<<<< HEAD\nA\n=======\nB\n").is_err());
        assert!(parse(b"<<<<<<< HEAD\n<<<<<<< x\n").is_err());
    }

    #[test]
    fn apercu_sans_echappement() {
        let p = preview(b"ok\x1b[31mrouge\nl2\nl3\n", 2);
        assert!(!p.contains('\u{1b}') && p.contains("+1 ligne"));
    }
}
