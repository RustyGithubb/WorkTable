//! Synchronisation avec le NAS (Mac mini, dépôt RustyGithubb/NAS) : on retrouve ses données sur
//! chaque poste en se connectant avec son compte. Tout est chiffré ici avant l'envoi
//! (crate `nas-sync-client`) : le serveur ne voit ni les noms ni le contenu.
//!
//! Fonctionnement :
//! - au **démarrage** (avant le chargement des données) : les changements reçus des autres postes
//!   sont appliqués, fusionnés avec ceux d'ici si besoin, puis on envoie ce qui a changé ici ;
//! - **pendant** que l'app tourne : un fil envoie les fichiers modifiés (quelques secondes après
//!   chaque enregistrement, et chaque minute) et reçoit ceux des autres postes. Ces derniers sont
//!   mis de côté (l'app a ses données en mémoire et les réécrirait) : Réglages › Compte NAS
//!   propose de redémarrer pour les appliquer.
//!
//! État local : `sync/` dans le dossier des données (jamais synchronisé) — compte, état de chaque
//! fichier, dernières versions des JSON (pour fusionner), changements reçus en attente.
//! Jeton de session et clé du compte : coffre du système (Gestionnaire d'identification).

mod engine;
mod merge;
pub mod ui;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};

const KEYRING_SERVICE: &str = "WorkTable";
const KEYRING_USER: &str = "nas-sync";

/// Dossier d'état de la synchronisation.
fn dir() -> PathBuf {
    crate::paths::root().join("sync")
}

/// Compte enregistré (sans secret).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    pub server: String,
    pub login: String,
}

impl Account {
    fn path() -> PathBuf {
        dir().join("account.json")
    }

    pub fn load() -> Option<Account> {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    fn save(&self) {
        crate::paths::save_json(&Self::path(), self);
    }
}

/// Secrets de la session : coffre du système.
#[derive(Clone, Serialize, Deserialize)]
struct Secret {
    token: String,
    /// Clé maître du compte (hexadécimal).
    master: String,
    device_id: u64,
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

fn load_secret() -> Option<Secret> {
    entry().ok()?.get_password().ok().and_then(|s| serde_json::from_str(&s).ok())
}

fn save_secret(s: &Secret) -> Result<(), String> {
    entry()?.set_password(&serde_json::to_string(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

fn forget_secret() {
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}

/// Nom de ce poste, vu dans la console du NAS (« PC-ECOLE »).
pub fn device_name() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "PC".into())
}

// ───────────────────────────── État affiché ─────────────────────────────

#[derive(Clone, Debug, Default, PartialEq)]
pub enum Phase {
    /// Pas de compte NAS.
    #[default]
    Off,
    Syncing,
    Idle,
    /// Serveur injoignable : on réessaie.
    Offline(String),
    /// Session révoquée ou expirée : se reconnecter.
    SignedOut,
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub phase: Phase,
    /// Dernière synchronisation réussie (ms).
    pub last_sync: Option<i64>,
    /// Fichiers reçus d'un autre poste (chemin, poste), appliqués au prochain démarrage.
    pub pending: Vec<(String, String)>,
    /// Copies créées au dernier démarrage (même fichier modifié sur deux postes).
    pub conflicts: Vec<String>,
    /// Fichiers trop gros, non synchronisés.
    pub skipped: Vec<String>,
    pub used: u64,
    pub quota: u64,
    pub files: u64,
    pub devices: Vec<nas_proto::sync::Device>,
    pub device_id: u64,
    /// Connexion en cours (Réglages).
    pub connecting: bool,
    pub connect_error: Option<String>,
    /// Code de récupération à montrer une fois (premier login).
    pub recovery: Option<String>,
}

static STATUS: Mutex<Option<Status>> = Mutex::new(None);
static POKE: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static CTX: Mutex<Option<eframe::egui::Context>> = Mutex::new(None);

pub fn status() -> Status {
    STATUS.lock().unwrap_or_else(|e| e.into_inner()).clone().unwrap_or_default()
}

fn with_status(f: impl FnOnce(&mut Status)) {
    {
        let mut s = STATUS.lock().unwrap_or_else(|e| e.into_inner());
        f(s.get_or_insert_with(Status::default));
    }
    if let Some(ctx) = CTX.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
}

/// Un fichier vient d'être enregistré : envoi dans quelques secondes (appelé par `paths`).
pub fn poke() {
    POKE.store(true, Ordering::Relaxed);
}

/// Connecté (compte et session présents) ?
pub fn configured() -> bool {
    Account::load().is_some() && load_secret().is_some()
}

/// Démarrage de l'app, **avant** le chargement des données : applique ce qui est arrivé des
/// autres postes, puis échange avec le serveur (court délai si hors ligne). Lance ensuite le fil
/// de synchronisation.
pub fn startup() {
    let (Some(account), Some(secret)) = (Account::load(), load_secret()) else { return };
    with_status(|s| s.device_id = secret.device_id);
    engine::cycle(&account, &secret, true);
    spawn();
}

/// Lance le fil de synchronisation (une seule fois).
fn spawn() {
    if RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let mut last = std::time::Instant::now();
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3));
            // Après un enregistrement : au plus toutes les 10 s ; sinon chaque minute.
            let due = last.elapsed() >= std::time::Duration::from_secs(60) || (POKE.load(Ordering::Relaxed) && last.elapsed() >= std::time::Duration::from_secs(10));
            if !due {
                continue;
            }
            POKE.store(false, Ordering::Relaxed);
            let (Some(account), Some(secret)) = (Account::load(), load_secret()) else {
                RUNNING.store(false, Ordering::SeqCst);
                return;
            };
            if status().phase == Phase::SignedOut {
                continue;
            }
            engine::cycle(&account, &secret, false);
            last = std::time::Instant::now();
        }
    });
}

/// Synchroniser maintenant (bouton des Réglages).
pub fn sync_now() {
    poke();
}

/// Connexion au compte (fil à part). `invite` : code donné par la console du NAS au premier login.
pub fn connect(server: String, login: String, password: String, invite: Option<String>) {
    with_status(|s| {
        s.connecting = true;
        s.connect_error = None;
    });
    std::thread::spawn(move || {
        let login = login.trim().to_lowercase();
        let mut client = nas_sync_client::Client::new(&server, nas_proto::sync::WORKTABLE, &device_name());
        let result = match &invite {
            Some(code) => client.register(&login, code.trim(), &password).map(|(s, r)| (s, Some(r))),
            None => client.login(&login, &password).map(|s| (s, None)),
        };
        match result {
            Ok((session, recovery)) => {
                let secret = Secret { token: session.token, master: nas_sync_client::crypto::hex(&session.master.0), device_id: session.device_id };
                if let Err(e) = save_secret(&secret) {
                    with_status(|s| {
                        s.connecting = false;
                        s.connect_error = Some(tf!("coffre du système inaccessible : {e}", "system vault unavailable: {e}"));
                    });
                    return;
                }
                let account = Account { server: server.trim().to_string(), login };
                account.save();
                // Filet de sécurité avant la première fusion avec les données d'autres postes.
                engine::backup_before_first_sync();
                with_status(|s| {
                    *s = Status { recovery, device_id: secret.device_id, phase: Phase::Syncing, ..Default::default() };
                });
                engine::cycle(&account, &secret, false);
                spawn();
            }
            Err(e) => with_status(|s| {
                s.connecting = false;
                s.connect_error = Some(e.to_string());
            }),
        }
    });
}

/// Déconnexion : ce poste ne synchronise plus (ses fichiers restent).
pub fn disconnect() {
    if let (Some(a), Some(s)) = (Account::load(), load_secret()) {
        let mut c = nas_sync_client::Client::new(&a.server, nas_proto::sync::WORKTABLE, &device_name()).with_token(&s.token);
        std::thread::spawn(move || {
            let _ = c.logout();
        });
    }
    forget_secret();
    let _ = std::fs::remove_dir_all(dir());
    with_status(|s| *s = Status::default());
}

/// Révoque un autre poste.
pub fn revoke(device: u64) {
    let (Some(a), Some(s)) = (Account::load(), load_secret()) else { return };
    std::thread::spawn(move || {
        let c = nas_sync_client::Client::new(&a.server, nas_proto::sync::WORKTABLE, &device_name()).with_token(&s.token);
        let _ = c.revoke_device(device);
        poke();
    });
}

/// Mémorise le contexte d'affichage (rafraîchir l'interface quand l'état change).
pub fn set_ctx(ctx: &eframe::egui::Context) {
    let mut c = CTX.lock().unwrap_or_else(|e| e.into_inner());
    if c.is_none() {
        *c = Some(ctx.clone());
    }
}
