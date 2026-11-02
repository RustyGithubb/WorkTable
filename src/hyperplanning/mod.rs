//! HyperPlanning (logiciel de planning de l'école), via son service web SOAP.
//!
//! - **Lecture seule** : aucune opération d'écriture n'est envoyée au serveur (voir `soap`).
//! - Identifiants saisis dans Réglages › HyperPlanning, gardés dans `secrets\hyperplanning.json`
//!   (mot de passe protégé par Windows, voir `config`).
//! - Copie locale chiffrée (`cache`), relue au lancement puis tenue à jour en arrière-plan
//!   toutes les [`SYNC_EVERY`] (`sync`) : les lectures ne font aucune requête.

pub mod cache;
pub mod config;
pub mod soap;
pub mod sync;
pub mod web;

use eframe::egui;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

/// Intervalle entre deux synchronisations automatiques.
pub const SYNC_EVERY: Duration = Duration::from_secs(15 * 60);

/// État de la connexion, tel qu'affiché dans les réglages.
#[derive(Clone, PartialEq)]
pub enum Status {
    /// Pas encore testée depuis le lancement.
    Unknown,
    Testing,
    /// Connexion réussie : version du serveur.
    Connected(String),
    Failed(String),
}

/// Demande de réservation de salle (par salle, clé du service web).
#[derive(Clone, PartialEq)]
pub enum RequestStatus {
    Idle,
    Sending(u32),
    Sent(u32),
    Failed(u32, String),
}

/// État de la dernière synchronisation.
#[derive(Clone, PartialEq)]
pub enum SyncStatus {
    Idle,
    Running,
    /// Réussie, avec le nombre de requêtes envoyées.
    Done(u32),
    Failed(String),
}

pub struct HyperPlanning {
    pub config: config::Config,
    /// Mot de passe en cours de saisie (vide : garder celui enregistré).
    pub password_input: String,
    pub status: Status,
    pub sync_status: SyncStatus,
    pub request_status: RequestStatus,
    request_rx: Option<Receiver<Result<String, String>>>,
    /// Copie locale (`None` tant qu'elle n'est pas relue du disque).
    snapshot: Option<cache::Snapshot>,
    test_rx: Option<Receiver<Result<String, String>>>,
    /// Mot de passe de l'espace web en cours de saisie (vide : garder celui enregistré).
    pub web_password_input: String,
    /// Test de connexion à l'espace web (nom de l'utilisateur connecté).
    pub web_status: Status,
    web_rx: Option<Receiver<Result<String, String>>>,
    load_rx: Option<Receiver<Option<cache::Snapshot>>>,
    sync_rx: Option<Receiver<Result<(cache::Snapshot, u32), String>>>,
    last_sync: Option<Instant>,
    ctx: egui::Context,
}

impl HyperPlanning {
    pub fn new(ctx: egui::Context) -> Self {
        // Copie locale relue dans un fil à part (déchiffrement sans bloquer l'affichage).
        let (tx, rx) = std::sync::mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(cache::load());
            c.request_repaint();
        });
        Self {
            config: config::Config::load(),
            password_input: String::new(),
            status: Status::Unknown,
            sync_status: SyncStatus::Idle,
            request_status: RequestStatus::Idle,
            request_rx: None,
            snapshot: None,
            test_rx: None,
            web_password_input: String::new(),
            web_status: Status::Unknown,
            web_rx: None,
            load_rx: Some(rx),
            sync_rx: None,
            last_sync: None,
            ctx,
        }
    }

    /// Adresse, identifiant et mot de passe enregistrés.
    pub fn is_configured(&self) -> bool {
        self.config.is_complete()
    }

    /// Client prêt à interroger le serveur (identifiants enregistrés).
    pub fn client(&self) -> Option<soap::Client> {
        let password = self.config.password()?;
        Some(soap::Client::new(&self.config.url, &self.config.login, &password))
    }

    /// Copie locale des données (lecture sans requête).
    pub fn data(&self) -> Option<&cache::Snapshot> {
        self.snapshot.as_ref()
    }

    /// À appeler à chaque image : relève les résultats en cours et lance la synchronisation
    /// automatique quand elle est due.
    pub fn tick(&mut self) {
        self.poll_test();
        if let Some(rx) = &self.web_rx {
            match rx.try_recv() {
                Ok(r) => {
                    self.web_status = match r {
                        Ok(user) => Status::Connected(user),
                        Err(e) => Status::Failed(e),
                    };
                    self.web_rx = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.web_status = Status::Failed(t!("Test interrompu", "Test interrupted").into());
                    self.web_rx = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.load_rx
            && let Ok(snap) = rx.try_recv()
        {
            self.snapshot = Some(snap.unwrap_or_default());
            self.load_rx = None;
        }
        self.poll_sync();
        if let Some(rx) = &self.request_rx {
            let key = match self.request_status {
                RequestStatus::Sending(k) => k,
                _ => 0,
            };
            match rx.try_recv() {
                Ok(Ok(_)) => {
                    self.request_status = RequestStatus::Sent(key);
                    self.request_rx = None;
                }
                Ok(Err(e)) => {
                    self.request_status = RequestStatus::Failed(key, e);
                    self.request_rx = None;
                }
                Err(TryRecvError::Disconnected) => {
                    self.request_status = RequestStatus::Failed(key, t!("Envoi interrompu", "Sending interrupted").into());
                    self.request_rx = None;
                }
                Err(TryRecvError::Empty) => {}
            }
        }
        if self.snapshot.is_none() || self.sync_rx.is_some() || !self.is_configured() {
            return;
        }
        match self.last_sync.map(|t| t.elapsed()) {
            Some(e) if e < SYNC_EVERY => self.ctx.request_repaint_after(SYNC_EVERY - e),
            _ => self.sync_now(),
        }
    }

    /// Lance une synchronisation (dans un fil à part).
    pub fn sync_now(&mut self) {
        if self.sync_rx.is_some() {
            return;
        }
        let (Some(client), Some(snap)) = (self.client(), self.snapshot.clone()) else { return };
        let account = cache::account_id(&self.config.url, &self.config.login);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = sync::run(&client, &account, snap);
            let to_save = result.as_ref().ok().map(|(s, _)| s.clone());
            // Pas d'enregistrement si la copie a été effacée entre-temps (« Oublier »).
            if tx.send(result).is_ok()
                && let Some(s) = to_save
            {
                let _ = cache::save(&s);
            }
            ctx.request_repaint();
        });
        self.sync_rx = Some(rx);
        self.last_sync = Some(Instant::now());
        self.sync_status = SyncStatus::Running;
    }

    fn poll_sync(&mut self) {
        let Some(rx) = &self.sync_rx else { return };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err(t!("Synchronisation interrompue", "Sync interrupted").into()),
        };
        self.sync_rx = None;
        match result {
            Ok((snap, calls)) => {
                self.snapshot = Some(snap);
                self.sync_status = SyncStatus::Done(calls);
            }
            Err(e) => self.sync_status = SyncStatus::Failed(e),
        }
    }

    fn poll_test(&mut self) {
        let Some(rx) = &self.test_rx else { return };
        match rx.try_recv() {
            Ok(Ok(version)) => {
                self.status = Status::Connected(version);
                self.test_rx = None;
                // Identifiants valides : synchronisation tout de suite.
                self.last_sync = None;
            }
            Ok(Err(e)) => {
                self.status = Status::Failed(e);
                self.test_rx = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.status = Status::Failed(t!("Test interrompu", "Test interrupted").into());
                self.test_rx = None;
            }
        }
    }

    /// Enregistre les identifiants saisis puis teste la connexion (dans un fil à part).
    pub fn save_and_test(&mut self) {
        self.config.url = soap::normalize_base(&self.config.url);
        self.config.login = self.config.login.trim().to_string();
        if !self.password_input.is_empty() {
            if let Err(e) = self.config.set_password(&self.password_input) {
                self.status = Status::Failed(e);
                return;
            }
            self.password_input.clear();
        }
        if let Err(e) = self.config.save() {
            self.status = Status::Failed(e);
            return;
        }
        let Some(client) = self.client() else {
            self.status = Status::Failed(t!("Mot de passe manquant", "Missing password").into());
            return;
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(client.version());
            ctx.request_repaint();
        });
        self.test_rx = Some(rx);
        self.status = Status::Testing;
    }

    /// Enregistre les identifiants de l'espace web puis teste la connexion (dans un fil à part).
    pub fn save_and_test_web(&mut self) {
        self.config.web_url = self.config.web_url.trim().to_string();
        self.config.web_login = self.config.web_login.trim().to_string();
        if !self.web_password_input.is_empty() {
            if let Err(e) = self.config.set_web_password(&self.web_password_input) {
                self.web_status = Status::Failed(e);
                return;
            }
            self.web_password_input.clear();
        }
        if let Err(e) = self.config.save() {
            self.web_status = Status::Failed(e);
            return;
        }
        let Some((login, password)) = self.config.web_credentials() else {
            self.web_status = Status::Failed(t!("Identifiant ou mot de passe manquant", "Missing login or password").into());
            return;
        };
        let url = self.config.web_url.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(web::Session::connect(&url, &login, &password).map(|s| s.user));
            ctx.request_repaint();
        });
        self.web_rx = Some(rx);
        self.web_status = Status::Testing;
    }

    /// Envoie une demande de réservation de salle à l'espace web (dans un fil à part).
    /// `room_key` : clé de la salle dans la copie locale (pour l'affichage de l'état).
    pub fn request_room(&mut self, room_key: u32, req: web::RoomRequest) {
        if self.request_rx.is_some() {
            return;
        }
        let Some((login, password)) = self.config.web_credentials() else {
            self.request_status = RequestStatus::Failed(
                room_key,
                t!("Identifiants de l'espace web manquants (Réglages › HyperPlanning)", "Web space credentials missing (Settings › HyperPlanning)").into(),
            );
            return;
        };
        let url = self.config.web_url.clone();
        // Repli : premier jour de la base, donné par le service web.
        let first_monday = self.snapshot.as_ref().and_then(|s| s.window).map(|(start, _)| start);
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let r = web::Session::connect(&url, &login, &password).and_then(|mut s| s.request_room(&req, first_monday));
            let _ = tx.send(r);
            ctx.request_repaint();
        });
        self.request_rx = Some(rx);
        self.request_status = RequestStatus::Sending(room_key);
    }

    /// Efface les identifiants enregistrés et la copie locale.
    pub fn forget(&mut self) {
        config::Config::clear();
        cache::clear();
        self.config = config::Config::load();
        self.password_input.clear();
        self.status = Status::Unknown;
        self.sync_status = SyncStatus::Idle;
        self.snapshot = Some(Default::default());
        self.test_rx = None;
        self.web_password_input.clear();
        self.web_status = Status::Unknown;
        self.web_rx = None;
        self.sync_rx = None;
        self.last_sync = None;
    }
}
