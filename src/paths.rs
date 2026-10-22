//! Emplacement des données de l'application, selon la façon dont elle est lancée.
//!
//! - **Installée** (cas normal, via l'installeur) : les données de l'utilisateur dans son profil
//!   Windows, jamais dans le dossier du programme (en lecture seule sous `Program Files`) :
//!   ```text
//!   %APPDATA%\WorkTable\        (suit l'utilisateur d'un poste à l'autre en entreprise)
//!     config\   réglages, tâches, contacts, liens, épinglés, icônes Chat…
//!     data\     notes (Markdown) et leurs pièces jointes
//!     secrets\  identifiants Google (client OAuth, jeton de connexion)
//!   %LOCALAPPDATA%\WorkTable\cache\   émojis téléchargés (recréés au besoin, propres au poste)
//!   ```
//! - **Portable** (clé USB, dépôt git) : un dossier `data\` à côté de l'exécutable, ou un fichier
//!   `portable` (vide) à côté de lui. Tout (cache compris) reste alors dans `data\`.
//! - **Développement** (`cargo run`) : le dossier `data\` du projet.
//! - `WORKTABLE_DATA=<dossier>` force un dossier (tout dedans, comme en portable).
//!
//! Au premier lancement, les données de l'ancien emplacement (`%APPDATA%\WorkTable\WorkTable`) et
//! le jeton du Gestionnaire d'identification Windows sont copiés (rien n'est supprimé).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

const APP: &str = "WorkTable";

/// Comment l'application a été lancée (voir l'en-tête du module).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Forced,
    Development,
    Portable,
    Installed,
}

struct Layout {
    mode: Mode,
    root: PathBuf,
    cache: PathBuf,
}

fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        let one = |mode: Mode, root: PathBuf| Layout { mode, cache: root.join("cache"), root };
        if let Some(p) = std::env::var_os("WORKTABLE_DATA").filter(|p| !p.is_empty()) {
            return one(Mode::Forced, PathBuf::from(p));
        }
        let exe_dir = std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."));
        // `cargo run` : exécutable dans target\release (ou debug, dist) → dossier du projet.
        if let (Some("release" | "debug" | "dist"), Some(target)) = (exe_dir.file_name().and_then(|n| n.to_str()), exe_dir.parent())
            && target.file_name().is_some_and(|n| n == "target")
            && let Some(project) = target.parent()
        {
            return one(Mode::Development, project.join("data"));
        }
        // Version portable : `data\` (ou le fichier `portable`) à côté de l'exécutable.
        if exe_dir.join("data").is_dir() || exe_dir.join("portable").exists() {
            return one(Mode::Portable, exe_dir.join("data"));
        }
        // Version installée : profil de l'utilisateur.
        match directories::BaseDirs::new() {
            Some(b) => Layout { mode: Mode::Installed, root: b.data_dir().join(APP), cache: b.data_local_dir().join(APP).join("cache") },
            None => one(Mode::Portable, exe_dir.join("data")),
        }
    })
}

pub fn mode() -> Mode {
    layout().mode
}

/// Dossier racine des données de l'utilisateur.
pub fn root() -> &'static Path {
    &layout().root
}

pub fn config() -> PathBuf {
    root().join("config")
}

pub fn data() -> PathBuf {
    root().join("data")
}

pub fn cache() -> PathBuf {
    layout().cache.clone()
}

pub fn secrets() -> PathBuf {
    root().join("secrets")
}

/// Dossier de l'exécutable (fichiers livrés avec l'application, ex. `google.json`).
pub fn install_dir() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(Path::to_path_buf)).unwrap_or_else(|| PathBuf::from("."))
}

/// Écrit un fichier sans risque de le laisser à moitié écrit (coupure, plantage) : fichier
/// temporaire à côté, puis renommage. Crée le dossier si besoin.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// Enregistre une valeur en JSON lisible (réglages, listes…), de façon atomique.
pub fn save_json<T: serde::Serialize + ?Sized>(path: &Path, value: &T) {
    if let Ok(s) = serde_json::to_string_pretty(value) {
        let _ = write_atomic(path, s.as_bytes());
    }
}

/// Copie récursive (`overwrite` : remplace les fichiers déjà présents).
fn copy_tree_with(from: &Path, to: &Path, overwrite: bool) -> usize {
    let Ok(rd) = std::fs::read_dir(from) else { return 0 };
    let _ = std::fs::create_dir_all(to);
    let mut n = 0;
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            n += copy_tree_with(&src, &dst, overwrite);
        } else if (overwrite || !dst.exists()) && std::fs::copy(&src, &dst).is_ok() {
            n += 1;
        }
    }
    n
}

/// Copie récursive sans écraser les fichiers déjà présents.
fn copy_tree(from: &Path, to: &Path) -> usize {
    copy_tree_with(from, to, false)
}

/// Importe des données d'un autre emplacement (ancien dossier `data\` d'une version portable ou
/// de développement, sauvegarde…) : ses sous-dossiers `config`, `data` et `secrets` remplacent
/// les fichiers du même nom. Renvoie le nombre de fichiers copiés.
pub fn import_from(dir: &Path) -> usize {
    let dir = if dir.join("config").is_dir() || dir.join("secrets").is_dir() { dir.to_path_buf() } else { dir.join("data") };
    let mut n = 0;
    for sub in ["config", "data", "secrets"] {
        n += copy_tree_with(&dir.join(sub), &root().join(sub), true);
    }
    n
}

/// Rassemble une fois les données de l'ancien emplacement dans le dossier des données.
pub fn migrate() {
    let root = root();
    let marker = root.join(".migrated");
    if marker.exists() {
        return;
    }
    let _ = std::fs::create_dir_all(root);
    let mut copied = 0;
    if let Some(old) = directories::ProjectDirs::from("fr", APP, APP) {
        copied += copy_tree(old.config_dir(), &config());
        copied += copy_tree(old.data_dir(), &data());
        // Identifiants Google : dans secrets\.
        let google = config().join("google.json");
        if google.exists() && !secrets().join("google.json").exists() {
            let _ = std::fs::create_dir_all(secrets());
            let _ = std::fs::rename(&google, secrets().join("google.json"));
        }
    }
    // Jeton de connexion Google (Gestionnaire d'identification Windows) → secrets\.
    if let Some(t) = crate::google::auth::keyring_refresh_token() {
        let p = secrets().join("google_refresh_token");
        if !p.exists() && write_atomic(&p, t.as_bytes()).is_ok() {
            copied += 1;
        }
    }
    let readme = "Données de WorkTable.\n\
                  config : réglages, tâches, contacts, liens… — data : notes — secrets : identifiants Google\n\
                  (à ne jamais publier ni partager).\n";
    let _ = std::fs::write(root.join("LISEZMOI.txt"), readme);
    let _ = std::fs::write(marker, format!("{copied} fichier(s) repris de l'ancien emplacement\n"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_file() {
        let dir = std::env::temp_dir().join("wt_atomic_test");
        let p = dir.join("a.json");
        write_atomic(&p, b"un").unwrap();
        write_atomic(&p, b"deux").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "deux");
        assert!(!dir.join("a.json.tmp").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
