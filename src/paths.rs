//! Emplacement de toutes les données de l'application : un seul dossier `data\` transportable
//! (clé USB, dépôt git), à côté du projet — ou de l'exécutable s'il est copié ailleurs.
//!
//! ```text
//! data\
//!   config\   réglages, tâches, contacts, liens, épinglés, icônes Chat…
//!   data\     notes (Markdown) et leurs pièces jointes
//!   cache\    émojis téléchargés (recréés au besoin)
//!   secrets\  identifiants Google (client OAuth, jeton de connexion) — ignoré par git
//! ```
//!
//! Au premier lancement, les données de l'ancien emplacement (`%APPDATA%\WorkTable\WorkTable`)
//! et le jeton du Gestionnaire d'identification Windows y sont copiés (rien n'est supprimé).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Dossier racine des données.
pub fn root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        if let Some(p) = std::env::var_os("WORKTABLE_DATA").filter(|p| !p.is_empty()) {
            return PathBuf::from(p);
        }
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        // `cargo run` : exécutable dans target\release (ou debug) → dossier du projet.
        let base = match (exe_dir.file_name().and_then(|n| n.to_str()), exe_dir.parent()) {
            (Some("release" | "debug"), Some(t)) if t.file_name().is_some_and(|n| n == "target") => t.parent().map(Path::to_path_buf).unwrap_or(exe_dir.clone()),
            _ => exe_dir,
        };
        base.join("data")
    })
}

pub fn config() -> PathBuf {
    root().join("config")
}

pub fn data() -> PathBuf {
    root().join("data")
}

pub fn cache() -> PathBuf {
    root().join("cache")
}

pub fn secrets() -> PathBuf {
    root().join("secrets")
}

/// Copie récursive sans écraser les fichiers déjà présents.
fn copy_tree(from: &Path, to: &Path) -> usize {
    let Ok(rd) = std::fs::read_dir(from) else { return 0 };
    let _ = std::fs::create_dir_all(to);
    let mut n = 0;
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            n += copy_tree(&src, &dst);
        } else if !dst.exists() && std::fs::copy(&src, &dst).is_ok() {
            n += 1;
        }
    }
    n
}

/// Rassemble une fois les données de l'ancien emplacement dans le dossier `data\`.
pub fn migrate() {
    let root = root();
    let marker = root.join(".migrated");
    if marker.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(root);
    let mut copied = 0;
    if let Some(old) = directories::ProjectDirs::from("fr", "WorkTable", "WorkTable") {
        copied += copy_tree(old.config_dir(), &config());
        copied += copy_tree(old.data_dir(), &data());
        copied += copy_tree(old.cache_dir(), &cache());
        // Identifiants Google : dans secrets\.
        let google = config().join("google.json");
        if google.exists() {
            let _ = std::fs::create_dir_all(secrets());
            if !secrets().join("google.json").exists() {
                let _ = std::fs::rename(&google, secrets().join("google.json"));
            }
        }
    }
    // Jeton de connexion Google (Gestionnaire d'identification Windows) → secrets\.
    if let Some(t) = crate::google::auth::keyring_refresh_token() {
        let _ = std::fs::create_dir_all(secrets());
        let p = secrets().join("google_refresh_token");
        if !p.exists() {
            let _ = std::fs::write(p, t);
            copied += 1;
        }
    }
    let readme = "Données de WorkTable (transportables avec le projet).\n\
                  config : réglages, tâches, contacts, liens… — data : notes — cache : émojis\n\
                  secrets : identifiants Google — à ne pas publier (ignoré par git).\n";
    let _ = std::fs::write(root.join("LISEZMOI.txt"), readme);
    let _ = std::fs::write(marker, format!("{copied} fichier(s) repris de l'ancien emplacement\n"));
}
