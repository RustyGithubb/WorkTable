//! Synchronisation avec le NAS (Mac mini, dépôt RustyGithubb/NAS) : on retrouve ses données sur
//! chaque poste en se connectant avec son compte. Tout est chiffré ici avant l'envoi
//! (crate `nas-sync-client`) : le serveur ne voit ni les noms ni le contenu.
//!
//! Fonctionnement :
//! - au **démarrage** (avant le chargement des données) : les changements reçus des autres postes
//!   sont appliqués, fusionnés avec ceux d'ici si besoin ; l'envoi suit, fenêtre ouverte ;
//! - synchronisés : `config/` et `data/` ; `secrets/` et `cache/` restent propres à chaque poste ;
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

/// Compte enregistré (sans secret). Propre à ce poste (le dossier `sync` n'est pas synchronisé).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    pub server: String,
    pub login: String,
    /// « Retenir mon compte » décoché : le mot de passe est redemandé à chaque lancement (vérifié
    /// sur ce poste avec `verifier`, même hors ligne).
    #[serde(default)]
    pub forget: bool,
    /// Empreinte Argon2id du mot de passe (jamais le mot de passe), faite à la dernière connexion.
    #[serde(default)]
    pub verifier: Option<Verifier>,
}

/// Empreinte locale du mot de passe : sel aléatoire + SHA-256 de la clé Argon2id dérivée.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Verifier {
    salt: String,
    hash: String,
}

/// Paramètres Argon2id de l'empreinte locale (quelques dixièmes de seconde).
const VERIFIER_M_KIB: u32 = 19 * 1024;
const VERIFIER_T: u32 = 2;

impl Verifier {
    fn new(password: &str) -> Option<Verifier> {
        let salt = nas_sync_client::crypto::hex(&nas_sync_client::crypto::random::<16>());
        let keys = nas_sync_client::crypto::password_keys(password, &salt, VERIFIER_M_KIB, VERIFIER_T).ok()?;
        Some(Verifier { hash: nas_sync_client::crypto::sha256_hex(keys.auth_key.as_bytes()), salt })
    }

    fn matches(&self, password: &str) -> bool {
        nas_sync_client::crypto::password_keys(password, &self.salt, VERIFIER_M_KIB, VERIFIER_T)
            .is_ok_and(|k| nas_sync_client::crypto::sha256_hex(k.auth_key.as_bytes()) == self.hash)
    }
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
        forget_configured();
    }
}

/// Mot de passe du compte ouvert vérifié sur ce poste (écran de connexion, compte non retenu).
/// `None` : pas d'empreinte enregistrée, il faut se connecter au serveur.
pub fn verify_password(password: &str) -> Option<bool> {
    Account::load()?.verifier.map(|v| v.matches(password))
}

/// « Retenir mon compte » pour le compte ouvert ; `password` (s'il vient d'être vérifié)
/// renouvelle l'empreinte locale.
pub fn set_remember(remember: bool, password: Option<&str>) {
    let Some(mut a) = Account::load() else { return };
    a.forget = !remember;
    if let Some(p) = password
        && let Some(v) = Verifier::new(p)
    {
        a.verifier = Some(v);
    }
    a.save();
}

/// Fichier posé quand une connexion redémarre l'app sur un autre compte : l'écran de connexion
/// ne se réaffiche pas juste après (valable 2 min, une seule fois).
fn unlock_marker(login: &str) -> PathBuf {
    crate::paths::profile_root(login).join("sync").join("unlocked")
}

/// « Retenir mon compte » coché ou non sur l'écran de connexion : enregistré avec le compte par
/// toute connexion au serveur (`connect`).
static REMEMBER_NEW: AtomicBool = AtomicBool::new(true);

pub fn set_remember_new(remember: bool) {
    REMEMBER_NEW.store(remember, Ordering::Relaxed);
}

/// « Continuer sans compte » : ouvre les données de ce poste sans compte (redémarrage si un compte
/// est ouvert), sans repasser par l'écran de connexion.
pub fn open_without_account(ctx: &eframe::egui::Context) {
    let _ = crate::paths::write_atomic(&unlock_marker(crate::paths::LOCAL_PROFILE), LOCAL_MARK);
    switch_to(ctx, crate::paths::LOCAL_PROFILE);
}

/// Redémarrage voulu par l'utilisateur déjà connecté (mise à jour) : l'écran de connexion est
/// passé une fois.
pub fn mark_unlocked() {
    let _ = crate::paths::write_atomic(&unlock_marker(crate::paths::profile()), b"1");
}

/// Contenu du marqueur posé par « Continuer sans compte » (sinon : connexion réussie).
const LOCAL_MARK: &[u8] = b"local";

/// Marqueur à poser une fois la connexion terminée (code de récupération noté), juste avant le
/// redémarrage sur le compte.
static PENDING_MARKER: Mutex<Option<PathBuf>> = Mutex::new(None);

/// L'app vient de redémarrer après une connexion réussie (passe l'écran de connexion). Un marqueur
/// « sans compte » n'ouvre jamais des données qui portent un compte.
pub fn take_unlock_marker() -> bool {
    let p = unlock_marker(crate::paths::profile());
    let fresh = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|d| d < std::time::Duration::from_secs(120));
    let local = std::fs::read(&p).is_ok_and(|b| b == LOCAL_MARK);
    let _ = std::fs::remove_file(&p);
    fresh && !(local && Account::load().is_some())
}

/// Secrets de la session : coffre du système.
#[derive(Clone, Serialize, Deserialize)]
struct Secret {
    token: String,
    /// Clé maître du compte (hexadécimal).
    master: String,
    device_id: u64,
}

/// Entrée du coffre d'un compte (une par identifiant : un poste peut garder plusieurs comptes).
fn entry(login: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, &format!("{KEYRING_USER}:{login}")).map_err(|e| e.to_string())
}

/// Session du compte dont les données sont ouvertes. Reprend une fois l'entrée unique des
/// versions précédentes (`nas-sync`).
fn load_secret() -> Option<Secret> {
    let login = Account::load()?.login;
    let read = |e: keyring::Entry| e.get_password().ok().and_then(|s| serde_json::from_str::<Secret>(&s).ok());
    if let Some(s) = entry(&login).ok().and_then(read) {
        return Some(s);
    }
    // Ancienne entrée : reprise seulement si ce poste n'a qu'un compte (celui des données
    // déplacées), jamais par un autre compte.
    if known_accounts().iter().filter(|(l, signed_in)| *signed_in && l != crate::paths::LOCAL_PROFILE).count() > 1 {
        return None;
    }
    let old = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).ok()?;
    let s = read(old)?;
    if save_secret(&login, &s).is_ok()
        && let Ok(old) = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
    {
        let _ = old.delete_credential();
    }
    Some(s)
}

fn save_secret(login: &str, s: &Secret) -> Result<(), String> {
    let r = entry(login)?.set_password(&serde_json::to_string(s).map_err(|e| e.to_string())?).map_err(|e| e.to_string());
    forget_configured();
    r
}

fn forget_secret() {
    if let Some(a) = Account::load()
        && let Ok(e) = entry(&a.login)
    {
        let _ = e.delete_credential();
    }
    forget_configured();
}

/// Réponse de [`configured`] gardée 2 s : l'interface l'interroge plusieurs fois par image, et
/// chaque réponse lit un fichier et le coffre du système. Seul ce oui/non est gardé (aucun secret).
static CONFIGURED: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);

fn forget_configured() {
    *CONFIGURED.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// Clé maître du compte connecté (chiffrement des données propres au compte, ex. Google).
pub fn master_key() -> Option<nas_sync_client::MasterKey> {
    let s = load_secret()?;
    nas_sync_client::crypto::from_hex(&s.master).and_then(|v| <[u8; 32]>::try_from(v).ok()).map(nas_sync_client::MasterKey)
}

/// Données présentes sur ce poste, par compte : (identifiant, connecté ?). `_local` : données
/// sans compte.
pub fn known_accounts() -> Vec<(String, bool)> {
    crate::paths::profiles()
        .into_iter()
        .map(|p| {
            let signed_in = crate::paths::profile_root(&p).join("sync").join("account.json").exists();
            (p, signed_in)
        })
        .collect()
}

/// Ouvre les données d'un autre compte de ce poste : noté pour le prochain lancement, puis
/// redémarrage de l'application (rien de l'ancien compte ne reste en mémoire).
pub fn switch_to(ctx: &eframe::egui::Context, login: &str) {
    crate::paths::set_next_profile(login, None);
    crate::update::restart(ctx);
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
    /// Connexion réussie à un autre compte : ses données s'ouvrent au redémarrage (après le code
    /// de récupération s'il y en a un).
    pub switch_to: Option<String>,
}

/// Redémarre sur le compte qui vient de se connecter, une fois le code de récupération noté.
pub fn finish_switch(ctx: &eframe::egui::Context) {
    let st = status();
    if st.switch_to.is_some() && st.recovery.is_none() {
        with_status(|s| s.switch_to = None);
        if let Some(p) = PENDING_MARKER.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = crate::paths::write_atomic(&p, b"1");
        }
        crate::update::restart(ctx);
    }
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
    if let Some((at, v)) = *CONFIGURED.lock().unwrap_or_else(|e| e.into_inner())
        && at.elapsed() < std::time::Duration::from_secs(2)
    {
        return v;
    }
    let v = Account::load().is_some() && load_secret().is_some();
    *CONFIGURED.lock().unwrap_or_else(|e| e.into_inner()) = Some((std::time::Instant::now(), v));
    v
}

/// Démarrage de l'app, **avant** le chargement des données : reçoit et applique ce qui est arrivé
/// des autres postes (court délai si hors ligne). Lance ensuite le fil de synchronisation, qui
/// envoie les changements d'ici quelques secondes après l'ouverture de la fenêtre.
pub fn startup() {
    let (Some(account), Some(secret)) = (Account::load(), load_secret()) else { return };
    with_status(|s| s.device_id = secret.device_id);
    engine::cycle(&account, &secret, true);
    poke();
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
                if let Err(e) = save_secret(&login, &secret) {
                    with_status(|s| {
                        s.connecting = false;
                        s.connect_error = Some(tf!("coffre du système inaccessible : {e}", "system vault unavailable: {e}"));
                    });
                    return;
                }
                // Empreinte locale du mot de passe (écran de connexion, compte non retenu) ; le choix
                // « retenir » déjà fait sur ce poste est gardé.
                let forget = !REMEMBER_NEW.load(Ordering::Relaxed);
                let account = Account { server: server.trim().to_string(), login: login.clone(), forget, verifier: Verifier::new(&password) };
                let current = crate::paths::profile();
                if login == current {
                    // Même compte (reconnexion) : sur ses données, sans redémarrer.
                    account.save();
                    // Filet de sécurité avant la première fusion avec les données d'autres postes.
                    engine::backup_before_first_sync();
                    with_status(|s| {
                        *s = Status { recovery, device_id: secret.device_id, phase: Phase::Syncing, ..Default::default() };
                    });
                    engine::cycle(&account, &secret, false);
                    spawn();
                    return;
                }
                // Autre compte : ses propres données (dossier à son nom), ouvertes après un
                // redémarrage. Les données sans compte sont reprises par le premier compte connecté
                // qui n'a encore rien sur ce poste.
                let adopt = current == crate::paths::LOCAL_PROFILE && !crate::paths::profile_root(&login).exists();
                if adopt {
                    engine::backup_before_first_sync();
                    account.save();
                } else {
                    crate::paths::save_json(&crate::paths::profile_root(&login).join("sync").join("account.json"), &account);
                }
                // Connexion faite : l'écran de connexion ne se réaffiche pas au redémarrage. Repris :
                // posé dans les données actuelles, renommées au nom du compte au redémarrage.
                *PENDING_MARKER.lock().unwrap_or_else(|e| e.into_inner()) = Some(unlock_marker(if adopt { crate::paths::LOCAL_PROFILE } else { &login }));
                crate::paths::set_next_profile(&login, adopt.then_some(crate::paths::LOCAL_PROFILE));
                with_status(|s| *s = Status { recovery, switch_to: Some(login), ..Default::default() });
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

/// Envoie l'image de profil du compte (copie en clair, affichée par la console du NAS) ;
/// `None` la retire. Bloquant : à appeler hors de l'interface.
pub fn upload_avatar(png: Option<&[u8]>) -> Result<(), String> {
    let (Some(a), Some(s)) = (Account::load(), load_secret()) else { return Err("non connecté".into()) };
    let c = nas_sync_client::Client::new(&a.server, nas_proto::sync::WORKTABLE, &device_name()).with_token(&s.token);
    match png {
        Some(p) => c.set_avatar(p),
        None => c.clear_avatar(),
    }
    .map_err(|e| e.to_string())
}

/// Mémorise le contexte d'affichage (rafraîchir l'interface quand l'état change).
pub fn set_ctx(ctx: &eframe::egui::Context) {
    let mut c = CTX.lock().unwrap_or_else(|e| e.into_inner());
    if c.is_none() {
        *c = Some(ctx.clone());
    }
}
