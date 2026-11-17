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
//! **Un dossier par compte** : `profiles\<identifiant NAS>\` (`_local` sans compte) contient
//! `config\`, `data\`, `secrets\` et `sync\` ; le cache aussi est rangé par compte. Le compte ouvert
//! est noté dans `profiles.json` ; en changer relance l'application.
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
    /// Dossier de base (choix du compte, `profiles\`).
    base: PathBuf,
    /// Compte actif : son dossier `profiles\<nom>\` est la racine des données.
    profile: String,
    root: PathBuf,
    cache: PathBuf,
}

/// Dossier des données sans compte NAS (`profiles\_local\`).
pub const LOCAL_PROFILE: &str = "_local";

/// Choix du compte actif (`profiles.json` dans le dossier de base). `rename_from` : dossier d'un
/// compte à renommer au prochain lancement (données locales reprises par un compte qui se connecte).
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Profiles {
    active: String,
    rename_from: Option<String>,
}

fn base_layout() -> (Mode, PathBuf, PathBuf) {
    let one = |mode: Mode, root: PathBuf| (mode, root.clone(), root.join("cache"));
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
        Some(b) => (Mode::Installed, b.data_dir().join(APP), b.data_local_dir().join(APP).join("cache")),
        None => one(Mode::Portable, exe_dir.join("data")),
    }
}

/// Mode et dossiers de base, sans rien toucher sur le disque (lu avant l'instance unique).
fn base() -> &'static (Mode, PathBuf, PathBuf) {
    static BASE: OnceLock<(Mode, PathBuf, PathBuf)> = OnceLock::new();
    BASE.get_or_init(base_layout)
}

/// Dossiers du compte ouvert. Le premier appel range les données (déplacement, renommage) : à
/// faire seulement une fois l'instance précédente fermée (voir `main`).
fn layout() -> &'static Layout {
    static LAYOUT: OnceLock<Layout> = OnceLock::new();
    LAYOUT.get_or_init(|| {
        let (_, base, cache) = base().clone();
        let (profile, root, cache) = prepare_profile(&base, &cache);
        Layout { root, cache, profile, base }
    })
}

/// Données d'avant les comptes séparés (`config\`, `data\`, `secrets\`, `sync\` directement dans
/// le dossier de base) : déplacées dans le dossier du compte connecté (sinon `_local`). Puis
/// renommage éventuel demandé au lancement précédent. Renvoie (compte ouvert, dossier des
/// données, dossier du cache). Un déplacement impossible (dossier verrouillé par OneDrive, un
/// antivirus…) ne perd rien : tout reste ou revient en place, nouvel essai au lancement suivant.
fn prepare_profile(base: &Path, cache: &Path) -> (String, PathBuf, PathBuf) {
    const OLD: [&str; 4] = ["config", "data", "secrets", "sync"];
    let profiles = base.join("profiles");
    if OLD.iter().any(|d| base.join(d).exists()) {
        let login = std::fs::read_to_string(base.join("sync").join("account.json"))
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v["login"].as_str().map(str::to_string))
            .filter(|l| nas_proto::sync::valid_login(l))
            .unwrap_or_else(|| LOCAL_PROFILE.into());
        let dir = profiles.join(&login);
        let mut moved = Vec::new();
        let mut ok = std::fs::create_dir_all(&dir).is_ok();
        if ok {
            for d in OLD {
                let (from, to) = (base.join(d), dir.join(d));
                if !from.exists() {
                    continue;
                }
                if to.exists() || std::fs::rename(&from, &to).is_err() {
                    ok = false;
                    break;
                }
                moved.push(d);
            }
        }
        if !ok {
            // Remis en place : l'ancien emplacement reste utilisé pour cette fois.
            for d in moved {
                let _ = std::fs::rename(dir.join(d), base.join(d));
            }
            return (login, base.to_path_buf(), cache.to_path_buf());
        }
        move_children(cache, &cache.join(&login));
        write_profiles(base, &Profiles { active: login, rename_from: None });
    }
    let mut p = read_profiles(base);
    if p.active.is_empty() {
        p.active = LOCAL_PROFILE.into();
    }
    if let Some(from) = p.rename_from.clone() {
        let (old, new) = (profiles.join(&from), profiles.join(&p.active));
        if old.is_dir() && !new.exists() {
            if std::fs::rename(&old, &new).is_err() {
                // Pas encore possible : les données restent ouvertes là où elles sont (demande gardée).
                return (from.clone(), old, cache.join(&from));
            }
            let _ = std::fs::rename(cache.join(&from), cache.join(&p.active));
        }
        p.rename_from = None;
        write_profiles(base, &p);
    }
    (p.active.clone(), profiles.join(&p.active), cache.join(&p.active))
}

/// Déplace le contenu d'un dossier (sauf `to`, s'il est dedans) vers `to`.
fn move_children(from: &Path, to: &Path) {
    let Ok(rd) = std::fs::read_dir(from) else { return };
    if std::fs::create_dir_all(to).is_err() {
        return;
    }
    for e in rd.flatten() {
        if e.path() != to {
            let _ = std::fs::rename(e.path(), to.join(e.file_name()));
        }
    }
}

fn read_profiles(base: &Path) -> Profiles {
    std::fs::read_to_string(base.join("profiles.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn write_profiles(base: &Path, p: &Profiles) {
    if let Ok(s) = serde_json::to_string_pretty(p) {
        let _ = std::fs::write(base.join("profiles.json"), s);
    }
}

/// Compte dont les données sont ouvertes (`_local` sans compte NAS).
pub fn profile() -> &'static str {
    &layout().profile
}

/// Comptes dont les données sont sur ce poste (noms des dossiers `profiles\…`).
pub fn profiles() -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(layout().base.join("profiles"))
        .map(|rd| rd.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Dossier des données d'un compte (qu'il existe ou non).
pub fn profile_root(name: &str) -> PathBuf {
    layout().base.join("profiles").join(name)
}

/// Compte à ouvrir au prochain lancement (`rename_from` : dossier à renommer en `name`).
pub fn set_next_profile(name: &str, rename_from: Option<&str>) {
    write_profiles(&layout().base, &Profiles { active: name.into(), rename_from: rename_from.map(str::to_string) });
}

pub fn mode() -> Mode {
    base().0
}

/// Dossier racine des données du compte ouvert (`profiles\<compte>\`).
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
    std::fs::rename(&tmp, path)?;
    // Synchronisation avec le NAS : envoi dans quelques secondes.
    crate::sync::poke();
    Ok(())
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

/// Crée, à chaque lancement, l'arborescence des données de l'utilisateur (`config\`, `data\`,
/// `secrets\`, cache) : elle existe dès le premier démarrage, avant toute écriture, et se
/// recrée si un dossier a été supprimé. Renvoie les dossiers qui n'ont pas pu être créés.
pub fn ensure_dirs() -> Vec<PathBuf> {
    create_tree(root(), &layout().cache)
}

fn create_tree(root: &Path, cache: &Path) -> Vec<PathBuf> {
    [root.join("config"), root.join("data"), root.join("data").join("notes"), root.join("secrets"), cache.to_path_buf()]
        .into_iter()
        .filter(|d| std::fs::create_dir_all(d).is_err())
        .collect()
}

/// Rassemble une fois les données de l'ancien emplacement dans le dossier des données.
pub fn migrate() {
    let root = root();
    // Marqueur dans le dossier de base : la reprise ne se fait qu'une fois, pas par compte.
    let marker = layout().base.join(".migrated");
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
    let readme = "Données de WorkTable / WorkTable data.\n\n\
                  config : réglages, tâches, contacts, liens… — data : notes — secrets : identifiants Google\n\
                  (à ne jamais publier ni partager).\n\n\
                  config: settings, tasks, contacts, links… — data: notes — secrets: Google credentials\n\
                  (never publish or share them).\n";
    let _ = std::fs::write(root.join("LISEZMOI.txt"), readme);
    let _ = std::fs::write(marker, format!("{copied} fichier(s) repris de l'ancien emplacement\n"));
}

/// Fenêtre « Enregistrer sous » pour un fichier reçu (image jointe…) : dossier Téléchargements
/// par défaut, nom d'origine (caractères interdits remplacés).
pub fn save_as(name: &str) -> Option<PathBuf> {
    let safe: String = name.chars().map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let mut d = rfd::FileDialog::new().set_file_name(if safe.trim().is_empty() { "image" } else { safe.trim() });
    if let Some(dir) = directories::UserDirs::new().and_then(|u| u.download_dir().map(|p| p.to_path_buf())) {
        d = d.set_directory(dir);
    }
    d.save_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_folders_are_created() {
        let dir = std::env::temp_dir().join("wt_dirs_test");
        let _ = std::fs::remove_dir_all(&dir);
        assert!(create_tree(&dir, &dir.join("cache")).is_empty());
        for sub in ["config", "data", "data/notes", "secrets", "cache"] {
            assert!(dir.join(sub).is_dir(), "{sub} manquant");
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn old_data_moves_into_account_folder() {
        let base = std::env::temp_dir().join("wt_profiles_test");
        let _ = std::fs::remove_dir_all(&base);
        let cache = base.join("cache");
        std::fs::create_dir_all(base.join("config")).unwrap();
        std::fs::create_dir_all(base.join("sync")).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(base.join("config").join("a.json"), "{}").unwrap();
        std::fs::write(base.join("sync").join("account.json"), r#"{"server":"s","login":"dorian"}"#).unwrap();
        std::fs::write(cache.join("x.bin"), "x").unwrap();
        assert_eq!(prepare_profile(&base, &cache).0, "dorian");
        assert!(base.join("profiles").join("dorian").join("config").join("a.json").exists());
        assert!(!base.join("config").exists());
        assert!(cache.join("dorian").join("x.bin").exists());
        // Données locales reprises par un compte : renommées au lancement suivant.
        write_profiles(&base, &Profiles { active: "lea".into(), rename_from: Some("dorian".into()) });
        assert_eq!(prepare_profile(&base, &cache).0, "lea");
        assert!(base.join("profiles").join("lea").join("config").join("a.json").exists());
        assert!(read_profiles(&base).rename_from.is_none());
        let _ = std::fs::remove_dir_all(base);
    }

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
