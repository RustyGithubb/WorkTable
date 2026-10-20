//! Assistant IA (Claude, via l'API Anthropic) : chat rapide ouvert depuis le logo de la barre,
//! résumés de journée, rappels avant les événements, notification résumée à l'arrivée d'un mail.
//!
//! La clé API (console.anthropic.com) est gardée dans le Gestionnaire d'identification Windows.
//! Les données envoyées à Claude : agenda, mails (expéditeur, objet, extrait), tâches, notes non
//! protégées, derniers messages Chat — uniquement quand on pose une question ou qu'un mail arrive.

use std::collections::HashSet;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, TimeZone};
use serde_json::{Value, json};

use crate::google::gmail::Summary;
use crate::prefs::Prefs;
use crate::tabs::calendar::CalendarState;
use crate::tabs::mail::MailState;
use crate::tabs::messages::ChatState;
use crate::tabs::notes::NotesState;
use crate::tasks::{Status, Tasks};

const API: &str = "https://api.anthropic.com/v1/messages";
const KEYRING_SERVICE: &str = "WorkTable";
const KEYRING_USER: &str = "anthropic-api-key";

/// Modèles proposés : (identifiant API, libellé).
pub const MODELS: [(&str, &str); 3] = [
    ("claude-haiku-5-5", "Claude Haiku 5.5 — rapide, économique"),
    ("claude-sonnet-5-5", "Claude Sonnet 5.5 — équilibré"),
    ("claude-opus-5-5", "Claude Opus 5.5 — le plus capable"),
];
pub const DEFAULT_MODEL: &str = "claude-sonnet-5-5";
/// Modèle des résumés de mails (courts, fréquents).
const SUMMARY_MODEL: &str = "claude-haiku-5-5";

#[derive(Clone, Copy, PartialEq)]
pub enum Role {
    User,
    Assistant,
}

pub enum NotifKind {
    Mail(String),
    Event,
}

pub struct Notif {
    pub kind: NotifKind,
    pub title: String,
    pub text: String,
    pub at: Instant,
}

enum Reply {
    Chat(Result<String, String>),
    MailSummary { id: String, from: String, subject: String, result: Result<String, String> },
    KeyCheck(Result<String, String>),
}

pub struct AiState {
    key: Option<String>,
    key_loaded: bool,
    pub key_input: String,
    pub key_status: Option<(bool, String)>,
    pub open: bool,
    pub messages: Vec<(Role, String)>,
    pub input: String,
    pub thinking: bool,
    pub focus_input: bool,
    pub scroll_bottom: bool,
    pub notifications: Vec<Notif>,
    tx: Sender<Reply>,
    rx: Receiver<Reply>,
    /// Mails déjà vus (pas de notification pour eux).
    known_mail: HashSet<String>,
    started_ms: i64,
    notified_events: HashSet<String>,
    last_event_check: Option<Instant>,
}

impl Default for AiState {
    fn default() -> Self {
        let (tx, rx) = channel();
        AiState {
            key: None,
            key_loaded: false,
            key_input: String::new(),
            key_status: None,
            open: false,
            messages: Vec::new(),
            input: String::new(),
            thinking: false,
            focus_input: false,
            scroll_bottom: false,
            notifications: Vec::new(),
            tx,
            rx,
            known_mail: HashSet::new(),
            started_ms: Local::now().timestamp_millis(),
            notified_events: HashSet::new(),
            last_event_check: None,
        }
    }
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

impl AiState {
    pub fn has_key(&mut self) -> bool {
        self.load_key();
        self.key.is_some()
    }

    fn load_key(&mut self) {
        if !self.key_loaded {
            self.key_loaded = true;
            self.key = entry().ok().and_then(|e| e.get_password().ok()).filter(|k| !k.trim().is_empty());
        }
    }

    /// Enregistre la clé saisie et la vérifie par un petit appel.
    pub fn save_key(&mut self) {
        let k = self.key_input.trim().to_string();
        if k.is_empty() {
            return;
        }
        match entry().and_then(|e| e.set_password(&k).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.key = Some(k.clone());
                self.key_loaded = true;
                self.key_input.clear();
                self.key_status = Some((true, "Vérification de la clé…".into()));
                let tx = self.tx.clone();
                std::thread::spawn(move || {
                    let r = ask(&k, SUMMARY_MODEL, "Réponds uniquement « OK ».", &[(Role::User, "Test".into())], 10);
                    let _ = tx.send(Reply::KeyCheck(r));
                });
            }
            Err(e) => self.key_status = Some((false, format!("Impossible d'enregistrer la clé : {e}"))),
        }
    }

    pub fn forget_key(&mut self) {
        if let Ok(e) = entry() {
            let _ = e.delete_credential();
        }
        self.key = None;
        self.key_status = Some((true, "Clé supprimée".into()));
    }

    /// Envoie une question (avec les données de l'application en contexte).
    pub fn send(&mut self, text: String, context: String, prefs: &Prefs) {
        let text = text.trim().to_string();
        if text.is_empty() || self.thinking {
            return;
        }
        self.load_key();
        self.messages.push((Role::User, text));
        self.scroll_bottom = true;
        let Some(key) = self.key.clone() else {
            self.messages.push((Role::Assistant, "Ajoute d'abord ta clé API Anthropic (Réglages › Assistant IA).".into()));
            return;
        };
        self.thinking = true;
        let model = if prefs.ai_model.is_empty() { DEFAULT_MODEL.to_string() } else { prefs.ai_model.clone() };
        let system = system_prompt(&context);
        // Les 20 derniers échanges suffisent.
        let history: Vec<(Role, String)> = self.messages.iter().rev().take(20).rev().cloned().collect();
        let tx = self.tx.clone();
        std::thread::spawn(move || {
            let r = ask(&key, &model, &system, &history, 1500);
            let _ = tx.send(Reply::Chat(r));
        });
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    /// À chaque image : réponses arrivées, rappels d'événements.
    pub fn background(&mut self, ctx: &eframe::egui::Context, cal: &CalendarState, prefs: &Prefs) {
        while let Ok(r) = self.rx.try_recv() {
            match r {
                Reply::Chat(r) => {
                    self.thinking = false;
                    self.scroll_bottom = true;
                    self.messages.push((Role::Assistant, r.unwrap_or_else(|e| format!("⚠️ {e}"))));
                }
                Reply::MailSummary { id, from, subject, result } => {
                    let text = match result {
                        Ok(t) => t,
                        Err(_) => subject.clone(),
                    };
                    self.notifications.push(Notif { kind: NotifKind::Mail(id), title: format!("✉️ {from}"), text, at: Instant::now() });
                }
                Reply::KeyCheck(r) => {
                    self.key_status = Some(match r {
                        Ok(_) => (true, "Clé valide : l'assistant est prêt.".into()),
                        Err(e) => (false, e),
                    });
                }
            }
            ctx.request_repaint();
        }
        if self.thinking {
            ctx.request_repaint_after(Duration::from_millis(200));
        }
        // Rappels : événements qui commencent bientôt.
        if !prefs.ai_no_event_notify && self.last_event_check.is_none_or(|t| t.elapsed() > Duration::from_secs(20)) {
            self.last_event_check = Some(Instant::now());
            let now = Local::now().timestamp_millis();
            let lead = if prefs.ai_event_minutes == 0 { 10 } else { prefs.ai_event_minutes.min(120) } as i64 * 60_000;
            for e in cal.events_between(prefs, now, now + lead) {
                if e.all_day || e.start < now {
                    continue;
                }
                let key = format!("{}/{}/{}", e.calendar, e.id, e.start);
                if self.notified_events.insert(key) {
                    let mins = ((e.start - now) / 60_000).max(1);
                    let place = if e.location.is_empty() { String::new() } else { format!(" · {}", e.location) };
                    self.notifications.push(Notif {
                        kind: NotifKind::Event,
                        title: format!("📅 Dans {mins} min"),
                        text: format!("{}{place}", if e.title.is_empty() { "(sans titre)" } else { &e.title }),
                        at: Instant::now(),
                    });
                }
            }
            ctx.request_repaint_after(Duration::from_secs(20));
        }
        // Les notifications disparaissent seules au bout de 45 s.
        self.notifications.retain(|n| n.at.elapsed() < Duration::from_secs(45));
        if !self.notifications.is_empty() {
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    /// Mails reçus (relève Gmail) : notification résumée pour les nouveaux.
    pub fn on_mails(&mut self, list: &[Summary], prefs: &Prefs, initial: bool) {
        for m in list {
            let fresh = self.known_mail.insert(m.id.clone());
            let inbox = m.labels.iter().any(|l| l == "INBOX");
            // Seuls les mails arrivés depuis le lancement, non lus, dans la boîte de réception.
            if initial || !fresh || !inbox || !m.unread() || m.date < self.started_ms - 120_000 || prefs.ai_no_mail_notify {
                continue;
            }
            let from = m.sender_name().to_string();
            let subject = if m.subject.is_empty() { "(sans objet)".to_string() } else { m.subject.clone() };
            self.load_key();
            match self.key.clone() {
                Some(key) => {
                    let tx = self.tx.clone();
                    let (id, snippet) = (m.id.clone(), m.snippet.clone());
                    let prompt = format!("De : {from}\nObjet : {subject}\nExtrait : {snippet}");
                    std::thread::spawn(move || {
                        let result = ask(
                            &key,
                            SUMMARY_MODEL,
                            "Résume ce mail en une seule phrase courte (20 mots maximum), en français, sans préambule. \
                             Mentionne une action attendue s'il y en a une.",
                            &[(Role::User, prompt)],
                            120,
                        );
                        let _ = tx.send(Reply::MailSummary { id, from, subject, result });
                    });
                }
                None => self.notifications.push(Notif { kind: NotifKind::Mail(m.id.clone()), title: format!("✉️ {from}"), text: subject, at: Instant::now() }),
            }
        }
    }
}

fn system_prompt(context: &str) -> String {
    format!(
        "Tu es l'assistant intégré à WorkTable, l'application d'organisation de travail de l'utilisateur \
         (agenda Google, Gmail, Google Chat, Drive, notes, tâches). Réponds en français, de façon concise et \
         chaleureuse, en Markdown simple (titres courts, listes à puces, **gras**). Appuie-toi uniquement sur \
         les données ci-dessous ; si une information manque, dis-le. Tu ne peux pas agir dans l'application \
         (envoyer, supprimer, créer) : propose plutôt quoi faire.\n\n{context}"
    )
}

/// Appel de l'API Messages d'Anthropic.
fn ask(key: &str, model: &str, system: &str, messages: &[(Role, String)], max_tokens: u32) -> Result<String, String> {
    let agent = ureq::AgentBuilder::new().timeout(Duration::from_secs(90)).try_proxy_from_env(true).build();
    let msgs: Vec<Value> = messages
        .iter()
        .map(|(r, t)| json!({ "role": if *r == Role::User { "user" } else { "assistant" }, "content": t }))
        .collect();
    let body = json!({ "model": model, "max_tokens": max_tokens, "system": system, "messages": msgs });
    let resp = agent
        .post(API)
        .set("x-api-key", key)
        .set("anthropic-version", "2023-06-01")
        .set("content-type", "application/json")
        .send_json(body);
    let v: Value = match resp {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_json::<Value>().ok().and_then(|v| v["error"]["message"].as_str().map(str::to_string)).unwrap_or_default();
            return Err(match code {
                401 => "Clé API invalide (vérifie-la dans Réglages › Assistant IA).".into(),
                402 | 403 => format!("Accès refusé par l'API Anthropic : {detail}"),
                429 => "Trop de demandes ou crédit épuisé : réessaie dans un instant.".into(),
                529 | 503 => "Claude est surchargé pour le moment : réessaie dans un instant.".into(),
                _ => format!("Erreur {code} : {detail}"),
            });
        }
        Err(e) => return Err(format!("Connexion impossible : {e}")),
    };
    let text: String = v["content"].as_array().into_iter().flatten().filter_map(|b| b["text"].as_str()).collect::<Vec<_>>().join("");
    if text.trim().is_empty() { Err("Réponse vide".into()) } else { Ok(text.trim().to_string()) }
}

// --- Données de l'application envoyées en contexte ------------------------------------------

fn day_label(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| format!("{} {:02}/{:02}", &crate::tabs::weekday_fr(d.weekday())[..3], d.day(), d.month())).unwrap_or_default()
}

fn hm(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

fn short(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() > n { s.chars().take(n).collect::<String>() + "…" } else { s }
}

/// Instantané lisible des données de l'application (ce que l'assistant « voit »).
pub fn context(mail: &MailState, cal: &CalendarState, prefs: &Prefs, tasks: &Tasks, notes: &NotesState, chat: &ChatState) -> String {
    let now = Local::now();
    let mut out = format!("## Maintenant\n{} {}, {}\n", crate::tabs::today_long(), now.format("%H:%M"), now.format("%Z"));

    // Agenda : aujourd'hui et les 7 prochains jours.
    out.push_str("\n## Agenda (aujourd'hui et 7 jours)\n");
    let from = crate::tabs::calendar::day_start(now.date_naive());
    let evs = cal.events_between(prefs, from, from + 8 * 86_400_000);
    if evs.is_empty() {
        out.push_str("(aucun événement chargé — l'agenda n'est peut-être pas connecté)\n");
    }
    for e in evs.iter().take(60) {
        let when = if e.all_day { format!("{} (journée)", day_label(e.start)) } else { format!("{} {}–{}", day_label(e.start), hm(e.start), hm(e.end)) };
        let mut line = format!("- {when} : {}", if e.title.is_empty() { "(sans titre)" } else { &e.title });
        if !e.location.is_empty() {
            line.push_str(&format!(" · lieu : {}", short(&e.location, 60)));
        }
        if !e.attendees.is_empty() {
            line.push_str(&format!(" · {} participant(s)", e.attendees.len()));
        }
        out.push_str(&line);
        out.push('\n');
    }

    // Mails : non lus de la boîte de réception, épinglés, favoris récents.
    out.push_str("\n## Mails\n");
    let mut seen = HashSet::new();
    let mut line = |out: &mut String, tag: &str, m: &Summary| {
        if seen.insert(m.id.clone()) {
            out.push_str(&format!("- [{tag}] {} · de {} · {} : {}\n", day_label(m.date), m.sender_name(), short(&m.subject, 90), short(&m.snippet, 160)));
        }
    };
    for m in &mail.pinned.list {
        line(&mut out, "épinglé", m);
    }
    for m in mail.messages.iter().filter(|m| m.unread() && m.labels.iter().any(|l| l == "INBOX")).take(25) {
        line(&mut out, "non lu", m);
    }
    for m in mail.messages.iter().filter(|m| m.labels.iter().any(|l| l == "INBOX")).take(10) {
        line(&mut out, "reçu", m);
    }
    for m in mail.starred.iter().take(8) {
        line(&mut out, "favori", m);
    }
    if seen.is_empty() {
        out.push_str("(aucun mail chargé)\n");
    }

    // Tâches.
    out.push_str("\n## Tâches\n");
    for (s, label) in [(Status::Doing, "en cours"), (Status::Todo, "à faire"), (Status::Done, "archivée aujourd'hui")] {
        for t in tasks.column(s) {
            let days = (now.timestamp_millis() - t.since) / 86_400_000;
            out.push_str(&format!("- [{label}] {} ({})\n", short(&t.title, 100), crate::tasks::ago(days)));
        }
    }

    // Notes (les notes protégées par mot de passe ne sont jamais envoyées).
    out.push_str("\n## Notes\n");
    for (_, title, text, updated, protected) in notes.entries().into_iter().take(20) {
        if protected {
            continue;
        }
        out.push_str(&format!("- « {} » (modifiée {}) : {}\n", short(&title, 80), day_label(updated), short(&text, 220)));
    }

    // Google Chat : derniers messages.
    out.push_str("\n## Google Chat (derniers messages)\n");
    let mut msgs: Vec<_> = chat.msgs.values().flatten().filter(|m| !m.text.trim().is_empty()).collect();
    msgs.sort_by_key(|m| -m.created);
    for m in msgs.into_iter().take(20) {
        let space = chat.space(&m.space).map(|s| chat.title(s)).unwrap_or_default();
        out.push_str(&format!("- {} {} · {} dans « {} » : {}\n", day_label(m.created), hm(m.created), chat.name_of(&m.sender), short(&space, 40), short(&m.text, 160)));
    }
    out
}

/// Questions rapides proposées dans le chat.
pub const QUICK: [(&str, &str); 4] = [
    ("☀️ Ma journée", "Fais-moi un résumé de ma journée : agenda, mails importants, tâches et ce qui demande mon attention."),
    ("⏭️ À venir", "Qu'est-ce qui arrive bientôt (prochaines heures et prochains jours) ? Signale ce qui demande une préparation."),
    ("✉️ Mails non lus", "Résume mes mails non lus en une ligne chacun et dis-moi lesquels demandent une réponse."),
    ("✅ Priorités", "Quelles sont mes priorités maintenant, d'après mes tâches, mon agenda et mes mails ?"),
];
