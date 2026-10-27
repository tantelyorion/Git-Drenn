//! Mode apprentissage : pour chaque étape, la commande Git équivalente et une explication en langage simple.
//! Textes fixes (aucun modèle) : fiables, jamais d'invention. Plus un petit glossaire (« c'est quoi un commit ? »).

use crate::actions::Action;

pub fn norm(s: &str) -> String {
    let mut o = String::new();
    for c in s.to_lowercase().chars() {
        let r = match c {
            'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'î' | 'ï' => 'i',
            'ô' | 'ö' => 'o',
            'ù' | 'û' | 'ü' => 'u',
            'ç' => 'c',
            '\'' | '’' | '-' | '?' | '!' | '.' | ',' | ':' | ';' => ' ',
            c => c,
        };
        o.push(r);
    }
    o.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn or_here(p: &Option<String>) -> String {
    p.as_deref().filter(|s| !s.trim().is_empty()).map(|s| format!("(dans « {s} ») ")).unwrap_or_default()
}

/// (commande(s) Git équivalente(s), explication)
pub fn equivalent(a: &Action) -> Option<(String, String)> {
    use Action::*;
    let (cmd, why): (String, &str) = match a {
        InspectRepository { .. } => ("git status ; git log -1 ; git remote -v".into(), "Regarde où vous en êtes : branche, modifications, dernier enregistrement, dépôts distants."),
        CloneRepository { url, .. } => (format!("git clone -- {url}"), "Télécharge une copie complète d'un projet (avec tout son historique) sur votre ordinateur."),
        InitializeRepository { .. } => ("git init".into(), "Transforme un dossier ordinaire en projet Git : Git commence à suivre ses fichiers."),
        InspectStatus { .. } => ("git status".into(), "Liste les fichiers modifiés, ajoutés ou supprimés depuis le dernier enregistrement (commit)."),
        InspectDiff { staged, .. } => (if *staged { "git diff --cached".into() } else { "git diff".into() }, "Montre ligne par ligne ce qui a changé."),
        InspectHistory { limit, .. } => (format!("git log --oneline --decorate -n {}", limit.unwrap_or(10)), "Affiche les derniers enregistrements (commits) : le journal du projet."),
        ListBranches { .. } => ("git branch -vv".into(), "Liste les branches : des lignes de travail parallèles. La branche courante est marquée d'une étoile."),
        CreateBranch { name, switch_to, .. } => (if *switch_to { format!("git switch -c {name}") } else { format!("git branch {name}") }, "Crée une nouvelle ligne de travail pour essayer quelque chose sans toucher à la branche principale."),
        SwitchBranch { name, .. } => (format!("git switch {name}"), "Change de branche : vos fichiers prennent l'état de cette branche."),
        StageChanges { files, .. } => (
            if files.is_empty() { "git add -A  (sauf les fichiers sensibles, écartés par GIT DRENN)".into() } else { format!("git add -- {}", files.join(" ")) },
            "Prépare (« indexe ») les modifications à inclure dans le prochain commit. C'est la « boîte » que l'on remplit avant d'enregistrer.",
        ),
        CreateCommit { message, .. } => (
            format!("git commit -m \"{}\"", if message.trim().is_empty() { "<message généré>" } else { message.trim() }),
            "Enregistre définitivement les modifications préparées dans l'historique, avec un message qui dit ce qui a changé.",
        ),
        FetchChanges { .. } => ("git fetch <remote>".into(), "Télécharge les nouveautés du dépôt distant SANS toucher à vos fichiers."),
        PullChanges { .. } => ("git pull --ff-only".into(), "Récupère les nouveautés du dépôt distant et les ajoute à votre travail, seulement si cela ne risque rien (avance rapide)."),
        PushChanges { force, remote, branch, .. } => (
            format!("git push{} {} {}", if *force { " --force-with-lease" } else { "" }, remote.as_deref().unwrap_or("origin"), branch.as_deref().unwrap_or("<branche>")),
            "Envoie vos commits vers le dépôt distant (GitHub…) pour les sauvegarder et les partager.",
        ),
        InspectRemotes { .. } => ("git remote -v".into(), "Affiche les adresses des dépôts distants liés à ce projet."),
        ConfigureRemote { name, url, .. } => (format!("git remote add {} {url}", name.as_deref().unwrap_or("origin")), "Relie votre projet à un dépôt distant (son adresse porte un nom, souvent « origin »)."),
        DiscardChanges { .. } => ("git stash create + git restore (avec sauvegarde de secours)".into(), "ABANDONNE vos modifications non enregistrées. GIT DRENN garde une sauvegarde récupérable avant."),
        UpdateGitignore { .. } => ("(édition du fichier .gitignore)".into(), "Le fichier .gitignore liste ce que Git doit ignorer : secrets, dossiers générés, fichiers temporaires."),
        UndoLastCommit { .. } => ("git reset --soft HEAD~1".into(), "Annule le dernier commit mais garde toutes ses modifications prêtes à être ré-enregistrées."),
        MergeBranch { name, .. } => (format!("git merge {name}"), "Fusionne le travail d'une autre branche dans la branche courante. S'il y a des chevauchements : conflit à arbitrer."),
        InspectConflicts { .. } => ("git status ; git diff --name-only --diff-filter=U".into(), "Un conflit survient quand deux branches ont modifié la même zone d'un fichier : Git demande de choisir."),
        ResolveConflicts { strategy, .. } => (
            match strategy.as_str() {
                "ours" => "git checkout --ours -- <fichier> ; git add <fichier>".into(),
                "theirs" => "git checkout --theirs -- <fichier> ; git add <fichier>".into(),
                _ => "(édition des zones entre <<<<<<< ======= >>>>>>> puis git add <fichier>)".into(),
            },
            "Choisit quelle version garder dans chaque zone en conflit (la vôtre, l'autre, ou les deux), puis marque le fichier comme résolu.",
        ),
        AbortMerge { .. } => ("git merge --abort".into(), "Annule la fusion en cours : retour exact à l'état d'avant."),
        ContinueMerge { .. } => ("git commit --no-edit".into(), "Termine la fusion en créant le commit de fusion, une fois tous les conflits résolus."),
        CreateTag { name, .. } => (format!("git tag -a {name} -m \"…\""), "Pose une étiquette de version (ex. v1.0.0) sur un commit précis, pour le retrouver facilement."),
        PushTag { name, remote, .. } => (format!("git push {} {name}", remote.as_deref().unwrap_or("origin")), "Envoie une étiquette de version vers le dépôt distant."),
        StashChanges { .. } => ("git stash push -u".into(), "Met vos modifications « dans un tiroir » pour retrouver un dossier propre ; vous les récupérerez plus tard."),
        RestoreStash { .. } => ("git stash pop".into(), "Ressort du tiroir les dernières modifications mises de côté."),
        ListRemoteRepositories {} => ("(API GitHub : GET /user/repos)".into(), "Liste vos projets hébergés sur GitHub."),
        SearchRemoteRepository { query } => (format!("(API GitHub : recherche « {query} »)"), "Cherche des projets publics sur GitHub."),
        CreateRemoteRepository { private, .. } => (format!("(API GitHub : création d'un dépôt {})", if *private { "privé" } else { "PUBLIC" }), "Crée l'espace du projet sur GitHub (privé par défaut) ; ensuite on y envoie le code avec « push »."),
        InspectRemoteRepository { repository } => (format!("(API GitHub : {repository})"), "Affiche les informations d'un projet GitHub."),
        VerifyRemotePush { .. } => ("git ls-remote <remote> <branche>".into(), "Contrôle que le dépôt distant possède bien votre dernier commit : preuve que l'envoi a réussi."),
    };
    let cmd = format!("{}{}", or_here(&None), cmd);
    Some((cmd, why.to_string()))
}

const GLOSSARY: &[(&[&str], &str)] = &[
    (&["commit", "enregistrement"], "Un commit est un enregistrement de votre projet à un instant donné, avec un message qui dit ce qui a changé. L'ensemble des commits forme l'historique : on peut y revenir à tout moment."),
    (&["branche", "branch"], "Une branche est une ligne de travail parallèle. On crée une branche pour développer une idée sans risquer la version principale (souvent « main »), puis on la fusionne si le résultat est bon."),
    (&["fusion", "merge", "fusionner"], "Fusionner (merge) réunit le travail de deux branches. Si les deux ont modifié la même zone d'un fichier, Git ne sait pas choisir : c'est un conflit, que l'on arbitre."),
    (&["conflit", "conflict", "conflits"], "Un conflit apparaît quand deux branches modifient la même zone d'un fichier. Git marque la zone (<<<<<<<, =======, >>>>>>>) et attend que vous choisissiez : votre version, l'autre, ou les deux. GIT DRENN sait le faire zone par zone."),
    (&["stash", "mise de cote", "mettre de cote"], "Le stash est un tiroir : vous y mettez vos modifications en cours pour retrouver un dossier propre (par exemple pour changer de branche), puis vous les ressortez plus tard."),
    (&["remote", "depot distant", "origin"], "Un dépôt distant (remote) est une copie de votre projet hébergée ailleurs (GitHub…). Son adresse porte un nom ; par convention « origin » désigne celui d'où vient ou où va votre projet."),
    (&["push", "envoyer", "pousser"], "Push envoie vos commits vers le dépôt distant, pour les sauvegarder et les partager. Rien n'est envoyé tant que vous ne le demandez pas."),
    (&["pull", "recuperer", "tirer"], "Pull récupère les nouveautés du dépôt distant et les intègre à votre travail. C'est un fetch (télécharger) suivi d'une fusion."),
    (&["fetch"], "Fetch télécharge les nouveautés du dépôt distant sans toucher à vos fichiers : on regarde avant de décider d'intégrer."),
    (&["clone", "cloner"], "Cloner télécharge une copie complète d'un projet, historique compris, et le relie automatiquement à son dépôt d'origine."),
    (&["tag", "etiquette", "version"], "Une étiquette (tag) donne un nom stable à un commit, typiquement un numéro de version comme v1.0.0."),
    (&["index", "staging", "preparer", "stage"], "L'index (ou « staging ») est la zone de préparation : vous y placez les modifications à inclure dans le prochain commit. Cela permet de faire des commits propres et ciblés."),
    (&["gitignore"], "Le fichier .gitignore liste ce que Git ne doit pas suivre : mots de passe, clés, dossiers générés (node_modules, target…), fichiers temporaires."),
    (&["head"], "HEAD est le pointeur « vous êtes ici » : il désigne le commit (ou la branche) sur lequel vous travaillez actuellement."),
    (&["depot", "repository", "repo", "git"], "Un dépôt (repository) est un dossier de projet suivi par Git : il contient vos fichiers et tout l'historique, dans un dossier caché .git."),
    (&["rebase"], "Rebase rejoue vos commits au-dessus d'une autre base pour obtenir un historique linéaire. C'est puissant mais réécrit l'historique : GIT DRENN ne le propose pas pour rester sûr."),
];

/// Cherche une définition dans le glossaire (accents et casse ignorés).
pub fn glossary(term: &str) -> Option<&'static str> {
    let t = norm(term);
    if t.is_empty() {
        return None;
    }
    let words: Vec<&str> = t.split(' ').collect();
    for (keys, text) in GLOSSARY {
        for k in *keys {
            if t == *k || words.contains(k) || (k.contains(' ') && t.contains(k)) {
                return Some(text);
            }
        }
    }
    None
}

/// Si la phrase est une question de définition, renvoie le terme demandé ; second booléen : question « pure ».
pub fn definition_request(instr: &str) -> Option<(String, bool)> {
    let n = norm(instr);
    for (p, pure) in [("c est quoi ", true), ("qu est ce que ", true), ("qu est ce qu un ", true), ("qu est ce qu une ", true), ("que veut dire ", true), ("what is ", true), ("explique moi ", false), ("explique ", false), ("explain ", false)] {
        if let Some(rest) = n.strip_prefix(p) {
            let rest = rest.trim_start_matches("un ").trim_start_matches("une ").trim_start_matches("le ").trim_start_matches("la ").trim_start_matches("l ").trim_start_matches("a ");
            return Some((rest.to_string(), pure));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glossaire_et_questions() {
        assert!(glossary("Commit").unwrap().contains("enregistrement"));
        assert!(glossary("une mise de côté").is_some());
        assert!(glossary("zzz").is_none());
        let (t, pure) = definition_request("C'est quoi un commit ?").unwrap();
        assert_eq!((t.as_str(), pure), ("commit", true));
        let (t, pure) = definition_request("Explique le stash").unwrap();
        assert_eq!((t.as_str(), pure), ("stash", false));
        assert!(definition_request("enregistre mes modifications").is_none());
    }

    #[test]
    fn chaque_action_a_une_explication() {
        let a: Action = serde_json::from_value(serde_json::json!({"action": "stage_changes"})).unwrap();
        let (c, w) = equivalent(&a).unwrap();
        assert!(c.contains("git add") && !w.is_empty());
        for n in crate::actions::ACTION_NAMES {
            // Champs obligatoires seulement (les actions refusent les champs inconnus).
            let v = match *n {
                "clone_repository" => serde_json::json!({"action": n, "url": "https://github.com/a/b"}),
                "create_branch" | "switch_branch" | "merge_branch" | "create_tag" | "push_tag" => serde_json::json!({"action": n, "name": "x"}),
                "configure_remote" => serde_json::json!({"action": n, "url": "https://github.com/a/b"}),
                "resolve_conflicts" => serde_json::json!({"action": n, "strategy": "ask"}),
                "search_remote_repository" => serde_json::json!({"action": n, "query": "q"}),
                "inspect_remote_repository" => serde_json::json!({"action": n, "repository": "a/b"}),
                _ => serde_json::json!({"action": n}),
            };
            let act: Action = serde_json::from_value(v).unwrap_or_else(|e| panic!("{n} : {e}"));
            assert!(equivalent(&act).is_some(), "{n}");
        }
    }
}
