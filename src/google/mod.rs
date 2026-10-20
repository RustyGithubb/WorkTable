//! Compte Google : connexion, session authentifiée, exécution des requêtes en arrière-plan.
//!
//! L'interface ne bloque jamais : chaque appel réseau part dans un thread, et le résultat
//! revient sous forme d'`Event` lu à chaque frame par `Account::poll`.

pub mod auth;
pub mod calendar;
pub mod chat;
pub mod drive;
pub mod config;
pub mod compose;
pub mod gmail;
pub mod html;

use std::sync::mpsc::{Receiver, Sender, channel};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;

pub use config::Config;
pub use gmail::{Attachment, FullMessage, Label, Summary};

pub(crate) fn http_err(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => status_msg(code, &resp.into_string().unwrap_or_default()),
        ureq::Error::Transport(t) => format!("Réseau : {t}"),
    }
}

fn status_msg(code: u16, body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| {
            v["error_description"]
                .as_str()
                .or(v["error"]["message"].as_str())
                .or(v["error"].as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.to_string());
    format!("Erreur {code} : {msg}")
}

// --- Limiteur de requêtes simultanées ---------------------------------------

/// Nombre maximal de requêtes Gmail en vol en même temps (toutes actions confondues).
/// Gmail refuse au-delà de quelques requêtes concurrentes par utilisateur.
const MAX_IN_FLIGHT: usize = 4;
/// Tentatives en cas de « rate limit » (attente exponentielle : 1 s, 2 s, 4 s, 8 s).
const MAX_RETRIES: u32 = 4;

#[derive(Default)]
struct Limiter {
    in_flight: Mutex<usize>,
    cv: Condvar,
}

struct Permit<'a>(&'a Limiter);

impl Limiter {
    fn acquire(&self) -> Permit<'_> {
        let mut n = self.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        while *n >= MAX_IN_FLIGHT {
            n = self.cv.wait(n).unwrap_or_else(|e| e.into_inner());
        }
        *n += 1;
        Permit(self)
    }
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        let mut n = self.0.in_flight.lock().unwrap_or_else(|e| e.into_inner());
        *n -= 1;
        self.0.cv.notify_one();
    }
}

// --- Session ---------------------------------------------------------------

#[derive(Clone)]
pub struct Session {
    agent: ureq::Agent,
    cfg: Config,
    tokens: Arc<Mutex<auth::Tokens>>,
    limiter: Arc<Limiter>,
    /// Métadonnées déjà téléchargées : un mail n'est demandé qu'une seule fois.
    pub(crate) cache: Arc<Mutex<HashMap<String, Summary>>>,
    /// Point de départ de la synchronisation incrémentale (API history).
    pub(crate) history_id: Arc<Mutex<Option<String>>>,
    pub email: String,
}

impl Session {
    fn new(agent: ureq::Agent, cfg: Config, tokens: auth::Tokens) -> Result<Self, String> {
        let mut s = Session {
            agent,
            cfg,
            tokens: Arc::new(Mutex::new(tokens)),
            limiter: Arc::new(Limiter::default()),
            cache: Arc::default(),
            history_id: Arc::default(),
            email: String::new(),
        };
        s.email = gmail::profile(&s)?.0;
        Ok(s)
    }

    /// Jeton d'accès valide (rafraîchi automatiquement s'il a expiré).
    fn token(&self) -> Result<String, String> {
        let mut t = self.tokens.lock().map_err(|_| "verrou empoisonné")?;
        if Instant::now() >= t.expires_at {
            *t = auth::refresh(&self.agent, &self.cfg, &t.refresh)?;
        }
        Ok(t.access.clone())
    }

    /// Envoie une requête en respectant la limite de concurrence, et réessaie
    /// automatiquement (avec attente croissante) si Gmail répond « trop de requêtes ».
    fn call(
        &self,
        send: impl Fn(&str) -> Result<ureq::Response, ureq::Error>,
    ) -> Result<serde_json::Value, String> {
        let mut attempt = 0;
        loop {
            let result = {
                let _permit = self.limiter.acquire();
                send(&format!("Bearer {}", self.token()?))
            };
            match result {
                Ok(r) => {
                    // Certaines réponses sont vides (204 après une suppression).
                    let body = r.into_string().map_err(|e| e.to_string())?;
                    if body.trim().is_empty() {
                        return Ok(serde_json::Value::Null);
                    }
                    return serde_json::from_str(&body).map_err(|e| e.to_string());
                }
                Err(ureq::Error::Status(code, resp)) => {
                    let retry_after = resp
                        .header("Retry-After")
                        .and_then(|v| v.trim().parse::<u64>().ok());
                    let body = resp.into_string().unwrap_or_default();
                    let rate_limited = code == 429
                        || code >= 500
                        || (code == 403 && body.to_lowercase().contains("ratelimitexceeded"));
                    if !rate_limited || attempt >= MAX_RETRIES {
                        return Err(status_msg(code, &body));
                    }
                    let wait = retry_after.unwrap_or(1 << attempt).min(30);
                    std::thread::sleep(Duration::from_secs(wait));
                    attempt += 1;
                }
                Err(e) => return Err(http_err(e)),
            }
        }
    }

    /// Jeton d'accès courant (pour les rares appels hors session, ex. tokeninfo).
    pub(crate) fn access_token(&self) -> Result<String, String> {
        self.token()
    }

    /// Téléchargement brut (fichiers, images) : même limiteur et mêmes reprises que les appels JSON.
    pub(crate) fn get_bytes(&self, url: &str, query: &[(&str, &str)], max: u64) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let mut attempt = 0;
        loop {
            let result = {
                let _permit = self.limiter.acquire();
                let mut req = self.agent.get(url).set("Authorization", &format!("Bearer {}", self.token()?));
                for (k, v) in query {
                    req = req.query(k, v);
                }
                req.call()
            };
            match result {
                Ok(r) => {
                    let mut buf = Vec::new();
                    r.into_reader().take(max).read_to_end(&mut buf).map_err(|e| e.to_string())?;
                    return Ok(buf);
                }
                Err(ureq::Error::Status(code, resp)) if (code == 429 || code >= 500) && attempt < MAX_RETRIES => {
                    drop(resp);
                    std::thread::sleep(Duration::from_secs(1 << attempt));
                    attempt += 1;
                }
                Err(e) => return Err(http_err(e)),
            }
        }
    }

    pub(crate) fn get(&self, url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, String> {
        self.call(|auth| {
            let mut req = self.agent.get(url).set("Authorization", auth);
            for (k, v) in query {
                req = req.query(k, v);
            }
            req.call()
        })
    }

    /// Envoi d'un corps brut (téléversement de message volumineux).
    pub(crate) fn post_bytes(&self, url: &str, content_type: &str, body: &[u8]) -> Result<serde_json::Value, String> {
        self.call(|auth| {
            self.agent
                .post(url)
                .set("Authorization", auth)
                .set("Content-Type", content_type)
                .send_bytes(body)
        })
    }

    /// Requête JSON avec méthode au choix (PATCH, PUT…), paramètres d'URL inclus.
    pub(crate) fn send(
        &self,
        method: &str,
        url: &str,
        query: &[(&str, &str)],
        body: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        self.call(|auth| {
            let mut req = self.agent.request(method, url).set("Authorization", auth);
            for (k, v) in query {
                req = req.query(k, v);
            }
            match &body {
                Some(b) => req.send_json(b.clone()),
                None => req.call(),
            }
        })
    }

    pub(crate) fn post(&self, url: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
        self.call(|auth| self.agent.post(url).set("Authorization", auth).send_json(body.clone()))
    }
}

// --- Événements ------------------------------------------------------------

pub enum Event {
    /// Connexion établie (géré par `Account`, puis relayé en `Ready`).
    Connected(Session),
    /// Le compte est prêt : les onglets peuvent charger leurs données.
    Ready,
    Labels(Vec<Label>),
    Messages { label: String, list: Vec<Summary> },
    Starred(Vec<Summary>),
    /// Changements depuis la dernière relève (nouveaux mails, lu/non lu, favoris, catégories…).
    Delta { changed: Vec<Summary>, deleted: Vec<String> },
    /// L'historique Gmail est trop ancien : il faut tout recharger.
    Resync,
    Opened(FullMessage),
    /// Conversation complète ; `open_id` = message à afficher en premier plan.
    Thread { open_id: String, messages: Vec<FullMessage> },
    /// Nombre de messages par conversation (pastille de la liste).
    ThreadSizes(Vec<(String, usize)>),
    /// Résultats d'une recherche Gmail (barre de recherche globale).
    MailSearch { query: String, list: Vec<Summary> },
    /// Derniers fichiers reçus par mail (tableau de bord).
    SharedFiles(Vec<(Summary, gmail::SharedFile)>),
    /// Résultats Google Drive (onglet Fichiers, recherche globale).
    Drive(drive::DriveEvent),
    /// Résultats Google Chat (onglet Messages).
    Chat(chat::ChatEvent),
    Modified { id: String, labels: Vec<String> },
    LabelCreated(Label),
    Sent,
    /// Contenu d'une pièce jointe (ou la raison de l'échec), identifiée par `Attachment::key`.
    AttachmentData { key: String, data: Result<Vec<u8>, String> },
    /// Image du corps d'un mail, décodée et prête à afficher.
    InlineImage { key: String, image: Result<std::sync::Arc<eframe::egui::ColorImage>, String> },
    /// Signatures Gmail disponibles (réglages + signature trouvée dans les mails envoyés).
    Signatures(Vec<gmail::Signature>),
    // --- Agenda ---
    Calendars(Vec<calendar::CalendarInfo>),
    CalEvents { calendar: String, from: i64, to: i64, events: Vec<calendar::CalEvent> },
    /// Événement créé ou modifié (`previous` : ancien agenda/identifiant si déplacé).
    CalSaved { event: calendar::CalEvent, previous: Option<(String, String)> },
    CalDeleted { calendar: String, id: String },
    CalError(String),
    Error(String),
}

// --- Compte ----------------------------------------------------------------

#[derive(Clone, PartialEq)]
pub enum Status {
    Disconnected,
    Connecting,
    Connected(String),
    Failed(String),
}

pub struct Account {
    pub config: Config,
    pub status: Status,
    session: Option<Session>,
    agent: ureq::Agent,
    tx: Sender<Event>,
    rx: Receiver<Event>,
    ctx: egui::Context,
}

impl Account {
    pub fn new(ctx: egui::Context) -> Self {
        let (tx, rx) = channel();
        let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build();
        let mut acc = Account {
            config: Config::load(),
            status: Status::Disconnected,
            session: None,
            agent,
            tx,
            rx,
            ctx,
        };
        // Reconnexion silencieuse si un jeton est déjà enregistré.
        if acc.config.is_complete() {
            if let Some(refresh) = auth::stored_refresh_token() {
                acc.status = Status::Connecting;
                let (agent, cfg) = (acc.agent.clone(), acc.config.clone());
                acc.spawn_raw(move || {
                    let tokens = auth::refresh(&agent, &cfg, &refresh)?;
                    Session::new(agent, cfg, tokens).map(Event::Connected)
                });
            }
        }
        acc
    }

    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// Enregistre les identifiants et lance la connexion via le navigateur.
    pub fn connect(&mut self) {
        if let Err(e) = self.config.save() {
            self.status = Status::Failed(e);
            return;
        }
        self.status = Status::Connecting;
        let (agent, cfg) = (self.agent.clone(), self.config.clone());
        self.spawn_raw(move || {
            let tokens = auth::login(&agent, &cfg)?;
            Session::new(agent, cfg, tokens).map(Event::Connected)
        });
    }

    pub fn disconnect(&mut self) {
        auth::forget_refresh_token();
        self.session = None;
        self.status = Status::Disconnected;
    }

    /// Exécute une requête authentifiée en arrière-plan.
    pub fn run(&self, f: impl FnOnce(&Session) -> Result<Event, String> + Send + 'static) {
        if let Some(s) = self.session.clone() {
            self.spawn_raw(move || f(&s));
        }
    }

    /// Tâche de fond sans session Gmail (ex. télécharger une image hébergée).
    pub fn spawn(&self, f: impl FnOnce() -> Result<Event, String> + Send + 'static) {
        self.spawn_raw(f);
    }

    pub fn agent(&self) -> ureq::Agent {
        self.agent.clone()
    }

    fn spawn_raw(&self, f: impl FnOnce() -> Result<Event, String> + Send + 'static) {
        let (tx, ctx) = (self.tx.clone(), self.ctx.clone());
        std::thread::spawn(move || {
            let ev = f().unwrap_or_else(Event::Error);
            let _ = tx.send(ev);
            ctx.request_repaint();
        });
    }

    /// Récupère les résultats arrivés depuis la dernière frame.
    pub fn poll(&mut self) -> Vec<Event> {
        let mut out = Vec::new();
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Connected(s) => {
                    self.status = Status::Connected(s.email.clone());
                    self.session = Some(s);
                    out.push(Event::Ready);
                }
                Event::Error(e) if self.status == Status::Connecting => {
                    self.status = Status::Failed(e)
                }
                other => out.push(other),
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn session() -> Session {
        Session {
            agent: ureq::AgentBuilder::new().build(),
            cfg: Config::default(),
            tokens: Arc::new(Mutex::new(auth::Tokens {
                access: "x".into(),
                expires_at: Instant::now() + Duration::from_secs(3600),
                refresh: String::new(),
            })),
            limiter: Arc::default(),
            cache: Arc::default(),
            history_id: Arc::default(),
            email: String::new(),
        }
    }

    #[test]
    fn retries_after_rate_limit() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let replies = [
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 11\r\nConnection: close\r\n\r\n{\"ok\":true}",
            ];
            for r in replies {
                let (mut s, _) = listener.accept().unwrap();
                let mut buf = [0u8; 2048];
                let _ = s.read(&mut buf);
                s.write_all(r.as_bytes()).unwrap();
            }
        });
        let v = session().get(&format!("http://127.0.0.1:{port}/x"), &[]).unwrap();
        assert_eq!(v["ok"], true);
    }

    #[test]
    fn limiter_caps_concurrency() {
        let lim = Arc::new(Limiter::default());
        let (now, max) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let hs: Vec<_> = (0..16)
            .map(|_| {
                let (lim, now, max) = (lim.clone(), now.clone(), max.clone());
                std::thread::spawn(move || {
                    let _p = lim.acquire();
                    let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                    max.fetch_max(n, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(20));
                    now.fetch_sub(1, Ordering::SeqCst);
                })
            })
            .collect();
        hs.into_iter().for_each(|h| h.join().unwrap());
        assert!(max.load(Ordering::SeqCst) <= MAX_IN_FLIGHT);
    }
}
