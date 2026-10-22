//! Fiches contact : annuaire caché, chiffré sur cet ordinateur.
//!
//! - Chaque personne croisée dans l'app (expéditeurs et destinataires des mails, membres et
//!   auteurs Google Chat, invités des rendez-vous, propriétaires et auteurs des fichiers Drive)
//!   a sa fiche, qui garde la trace de ce qui la relie à l'utilisateur : mails, fichiers,
//!   rendez-vous. Ses conversations Google Chat sont reprises de la copie locale des messages.
//! - Les fiches sont rassemblées au fil de l'utilisation, sans requête supplémentaire à Google ;
//!   seul le bouton « Chercher ses mails » de la fiche interroge Gmail, à la demande.
//! - Enregistrées dans `cache\people.wtc`, chiffrées avec la même clé que la copie des
//!   conversations (`secrets\chat_cache.key`, protégée par Windows), liées au compte Google et
//!   effacées avec elle.
//! - Pas d'onglet : la fiche s'ouvre en overlay (même style que la recherche Ctrl+Espace) par
//!   un clic droit sur une trace de la personne (mail, message, fichier, rendez-vous), ou depuis
//!   la recherche, où elle est proposée en premier.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use chrono::Local;
use eframe::egui::{self, Align2, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, vec2};
use serde::{Deserialize, Serialize};

use crate::google::calendar::CalEvent;
use crate::google::chat::SpaceKind;
use crate::google::drive::DriveFile;
use crate::google::gmail::SharedFile;
use crate::google::{Account, Event, Summary, gmail};
use crate::tabs::calendar::CalendarState;
use crate::tabs::mail::MailState;
use crate::tabs::mail::contacts::{Contacts, fold, parse_mailbox, split_list};
use crate::tabs::mail::view::{one_line, short_date};
use crate::tabs::messages::ChatState;
use crate::theme;

/// Traces gardées par personne (les plus récentes).
pub const MAX_MAILS: usize = 40;
pub const MAX_FILES: usize = 40;
pub const MAX_EVENTS: usize = 30;
/// Au-delà, les destinataires d'un mail ne sont pas reliés (envois de masse).
const MAX_RECIPIENTS: usize = 8;
/// Au-delà, les invités d'un rendez-vous ne sont pas reliés (grandes réunions).
const MAX_ATTENDEES: usize = 25;
const MAGIC: &[u8; 4] = b"WTP1";
const SCAN_EVERY: Duration = Duration::from_secs(3);
const SAVE_EVERY: Duration = Duration::from_secs(15);

/// Place de la personne dans un mail.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Elle l'a envoyé.
    #[default]
    From,
    /// L'utilisateur le lui a envoyé.
    To,
    /// Elle était en copie (ou co-destinataire).
    Cc,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MailTrace {
    pub id: String,
    pub thread: String,
    pub subject: String,
    pub date: i64,
    pub role: Role,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct EventTrace {
    pub id: String,
    pub calendar: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
    pub all_day: bool,
    pub location: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Card {
    /// Adresse (minuscules), vide si seule l'identité Google Chat est connue.
    pub email: String,
    /// Identifiant Google Chat (« users/… »).
    pub chat_id: String,
    pub name: String,
    /// Du plus récent au plus ancien.
    pub mails: Vec<MailTrace>,
    pub files: Vec<DriveFile>,
    pub events: Vec<EventTrace>,
    /// Dernière recherche de ses mails dans Gmail (ms), 0 si jamais.
    pub mails_checked: i64,
}

impl Card {
    /// Clé de la fiche : son adresse, sinon son identifiant Google Chat.
    pub fn key(&self) -> &str {
        if self.email.is_empty() { &self.chat_id } else { &self.email }
    }

    fn add_mail(&mut self, t: MailTrace) -> bool {
        if let Some(old) = self.mails.iter_mut().find(|m| m.id == t.id) {
            if *old == t {
                return false;
            }
            *old = t;
        } else {
            self.mails.push(t);
        }
        self.mails.sort_by_key(|m| std::cmp::Reverse(m.date));
        self.mails.truncate(MAX_MAILS);
        true
    }

    fn add_file(&mut self, f: &DriveFile) -> bool {
        if let Some(old) = self.files.iter_mut().find(|x| x.id == f.id) {
            if old.modified == f.modified && old.name == f.name {
                return false;
            }
            *old = f.clone();
        } else {
            self.files.push(f.clone());
        }
        self.files.sort_by_key(|x| std::cmp::Reverse(x.modified));
        self.files.truncate(MAX_FILES);
        true
    }

    fn add_event(&mut self, e: EventTrace) -> bool {
        if let Some(old) = self.events.iter_mut().find(|x| x.id == e.id && x.calendar == e.calendar) {
            if *old == e {
                return false;
            }
            *old = e;
        } else {
            self.events.push(e);
        }
        // Les plus récents (ou à venir) d'abord.
        self.events.sort_by_key(|x| std::cmp::Reverse(x.start));
        self.events.truncate(MAX_EVENTS);
        true
    }

    fn absorb(&mut self, other: Card) {
        if self.chat_id.is_empty() {
            self.chat_id = other.chat_id;
        }
        if better_name(&self.name, &self.email) {
            self.name = other.name;
        }
        for m in other.mails {
            self.add_mail(m);
        }
        for f in &other.files {
            self.add_file(f);
        }
        for e in other.events {
            self.add_event(e);
        }
        self.mails_checked = self.mails_checked.max(other.mails_checked);
    }
}

/// Le nom actuel gagne à être remplacé (absent, ou simple adresse).
fn better_name(current: &str, email: &str) -> bool {
    current.trim().is_empty() || current.eq_ignore_ascii_case(email) || current.contains('@')
}

/// Adresses automatiques : pas de fiche.
fn is_robot(email: &str) -> bool {
    ["noreply", "no-reply", "donotreply", "do-not-reply", "mailer-daemon", "postmaster", "notifications@", "calendar-notification", "calendar-server"]
        .iter()
        .any(|p| email.contains(p))
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Snapshot {
    account: String,
    saved: i64,
    cards: Vec<Card>,
}

pub fn file() -> PathBuf {
    crate::paths::cache().join("people.wtc")
}

fn load() -> Option<Snapshot> {
    serde_json::from_slice(&crate::tabs::messages::store::read_sealed(&file(), MAGIC)?).ok()
}

fn write(snap: &Snapshot) {
    if let Ok(plain) = serde_json::to_vec(snap) {
        let _ = crate::tabs::messages::store::write_sealed(&file(), MAGIC, &plain);
    }
}

/// Taille du fichier des fiches (octets), s'il existe.
pub fn size() -> Option<u64> {
    std::fs::metadata(file()).ok().map(|m| m.len())
}

/// Personne trouvée par la recherche globale.
pub struct Found {
    pub key: String,
    pub name: String,
    pub sub: String,
    pub score: i32,
}

#[derive(Default)]
pub struct People {
    pub cards: Vec<Card>,
    by_email: HashMap<String, usize>,
    by_chat: HashMap<String, usize>,
    /// Traces déjà relevées pendant cette session (évite de tout reparcourir).
    seen: HashSet<String>,
    account: String,
    started: bool,
    rx: Option<Receiver<Option<Snapshot>>>,
    dirty: bool,
    saved: Option<Instant>,
    scanned: Option<Instant>,
    /// Recherches Gmail en cours (adresses).
    pub searching: HashSet<String>,
    /// Dernière recherche Gmail en échec : (adresse, message).
    pub search_error: Option<(String, String)>,
}

impl People {
    fn reindex(&mut self) {
        self.by_email.clear();
        self.by_chat.clear();
        for (i, c) in self.cards.iter().enumerate() {
            if !c.email.is_empty() {
                self.by_email.insert(c.email.clone(), i);
            }
            if !c.chat_id.is_empty() {
                self.by_chat.insert(c.chat_id.clone(), i);
            }
        }
    }

    /// Fiche d'une adresse ou d'un identifiant Google Chat.
    pub fn find(&self, key: &str) -> Option<&Card> {
        self.index_of(key).map(|i| &self.cards[i])
    }

    fn index_of(&self, key: &str) -> Option<usize> {
        if key.contains('@') { self.by_email.get(&key.trim().to_lowercase()).copied() } else { self.by_chat.get(key).copied() }
    }

    /// Crée ou complète la fiche d'une personne ; renvoie sa position.
    pub fn upsert(&mut self, email: &str, chat_id: &str, name: &str) -> Option<usize> {
        let email = email.trim().to_lowercase();
        let chat_id = chat_id.trim();
        if email.is_empty() && chat_id.is_empty() {
            return None;
        }
        if !email.is_empty() && (!email.contains('@') || is_robot(&email) || email == self.account) {
            return None;
        }
        let by_mail = (!email.is_empty()).then(|| self.by_email.get(&email).copied()).flatten();
        let by_chat = (!chat_id.is_empty()).then(|| self.by_chat.get(chat_id).copied()).flatten();
        let i = match (by_mail, by_chat) {
            (Some(a), Some(b)) if a != b => {
                // Deux fiches pour la même personne (Chat sans adresse, puis adresse apprise) : fusion.
                let other = self.cards.remove(b);
                let a = if b < a { a - 1 } else { a };
                self.cards[a].absorb(other);
                self.reindex();
                self.dirty = true;
                a
            }
            (Some(a), _) => a,
            (None, Some(b)) => b,
            (None, None) => {
                self.cards.push(Card { email: email.clone(), chat_id: chat_id.to_string(), name: String::new(), ..Default::default() });
                self.dirty = true;
                let i = self.cards.len() - 1;
                if !email.is_empty() {
                    self.by_email.insert(email.clone(), i);
                }
                if !chat_id.is_empty() {
                    self.by_chat.insert(chat_id.to_string(), i);
                }
                i
            }
        };
        let c = &mut self.cards[i];
        if c.email.is_empty() && !email.is_empty() {
            c.email = email.clone();
            self.by_email.insert(email, i);
            self.dirty = true;
        }
        let c = &mut self.cards[i];
        if c.chat_id.is_empty() && !chat_id.is_empty() {
            c.chat_id = chat_id.to_string();
            self.by_chat.insert(chat_id.to_string(), i);
            self.dirty = true;
        }
        let c = &mut self.cards[i];
        let name = name.trim().trim_matches('"').trim();
        if !name.is_empty() && !name.contains('@') && better_name(&c.name, &c.email) {
            c.name = name.to_string();
            self.dirty = true;
        }
        Some(i)
    }

    /// Relie un mail à ses personnes (expéditeur, destinataires d'un envoi restreint).
    pub fn add_mail(&mut self, m: &Summary, own: &str) {
        let from = parse_mailbox(&m.from);
        let by_me = from.as_ref().is_some_and(|(_, a)| a == own);
        let trace = |role| MailTrace { id: m.id.clone(), thread: m.thread_id.clone(), subject: m.subject.clone(), date: m.date, role };
        if let Some((name, addr)) = &from
            && !by_me
            && let Some(i) = self.upsert(addr, "", name)
        {
            self.dirty |= self.cards[i].add_mail(trace(Role::From));
        }
        // Listes de diffusion : seul l'expéditeur est relié.
        if !m.list_id.is_empty() {
            return;
        }
        let rcpt: Vec<(String, String)> = split_list(&m.recipients).iter().filter_map(|r| parse_mailbox(r)).filter(|(_, a)| a != own).collect();
        if rcpt.len() > MAX_RECIPIENTS {
            return;
        }
        for (name, addr) in rcpt {
            if let Some(i) = self.upsert(&addr, "", &name) {
                self.dirty |= self.cards[i].add_mail(trace(if by_me { Role::To } else { Role::Cc }));
            }
        }
    }

    /// Relève, à intervalles, ce que les onglets ont chargé ; relit puis enregistre les fiches.
    pub fn background(&mut self, ctx: &egui::Context, acc: &Account, mail: &MailState, chat: &ChatState, cal: &CalendarState, drive: &crate::tabs::files::DriveState) {
        if !self.started {
            self.started = true;
            let (tx, rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(load());
            });
            self.rx = Some(rx);
        }
        if let Some(rx) = &self.rx {
            match rx.try_recv() {
                Ok(snap) => {
                    self.rx = None;
                    if let Some(snap) = snap {
                        let fresh = std::mem::take(&mut self.cards);
                        self.account = snap.account;
                        self.cards = snap.cards;
                        self.reindex();
                        for c in fresh {
                            if let Some(i) = self.upsert(&c.email, &c.chat_id, &c.name) {
                                self.cards[i].absorb(c);
                            }
                        }
                    }
                }
                Err(TryRecvError::Empty) => {
                    ctx.request_repaint_after(Duration::from_millis(50));
                    return;
                }
                Err(TryRecvError::Disconnected) => self.rx = None,
            }
        }
        let Some(own) = acc.session().map(|s| s.email.to_lowercase()) else {
            // Hors connexion : les fiches restent consultables (et complétées par ce qui est
            // affiché), mais ne sont pas réécrites (effacées avec la copie locale à la déconnexion).
            if !self.account.is_empty() && self.scanned.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
                self.scanned = Some(Instant::now());
                let own = self.account.clone();
                self.scan(&own, mail, chat, cal, drive);
            }
            self.dirty = false;
            return;
        };
        // Fiches d'un autre compte : rien ne doit en rester.
        if self.account != own {
            if !self.account.is_empty() {
                self.cards.clear();
                self.seen.clear();
                self.reindex();
            }
            self.account = own.clone();
            self.dirty = true;
        }
        if self.scanned.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
            self.scanned = Some(Instant::now());
            self.scan(&own, mail, chat, cal, drive);
        }
        if self.dirty && self.saved.is_none_or(|t| t.elapsed() >= SAVE_EVERY) {
            self.dirty = false;
            self.saved = Some(Instant::now());
            let snap = self.snapshot();
            std::thread::spawn(move || write(&snap));
        }
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { account: self.account.clone(), saved: Local::now().timestamp_millis(), cards: self.cards.clone() }
    }

    /// À la fermeture de l'app : dernier enregistrement, sans attendre.
    pub fn flush(&mut self) {
        if self.dirty && !self.account.is_empty() && self.rx.is_none() {
            self.dirty = false;
            write(&self.snapshot());
        }
    }

    fn scan(&mut self, own: &str, mail: &MailState, chat: &ChatState, cal: &CalendarState, drive: &crate::tabs::files::DriveState) {
        // Google Chat : identité (identifiant, nom, adresse).
        for p in chat.people.values() {
            if chat.me.as_deref() == Some(p.id.as_str()) || p.email.eq_ignore_ascii_case(own) {
                continue;
            }
            if self.seen.insert(format!("c:{}:{}:{}", p.id, p.email, p.name)) {
                self.upsert(&p.email, &p.id, &p.name);
            }
        }
        // Mails chargés (boîte, favoris, épinglés, annuaire, fichiers reçus).
        let mails = mail
            .messages
            .iter()
            .chain(mail.starred.iter())
            .chain(mail.pinned.list.iter())
            .chain(mail.dir_mails.iter().flat_map(|(_, l)| l.iter()))
            .chain(mail.shared_files.iter().map(|(m, _)| m));
        for m in mails {
            if self.seen.insert(format!("m:{}", m.id)) {
                self.add_mail(m, own);
            }
        }
        // Agenda : invités.
        for e in cal.events.values().flatten() {
            if e.attendees.len() > MAX_ATTENDEES || !self.seen.insert(format!("e:{}:{}:{}:{}", e.calendar, e.id, e.start, e.title)) {
                continue;
            }
            for a in &e.attendees {
                if a.eq_ignore_ascii_case(own) {
                    continue;
                }
                if let Some(i) = self.upsert(a, "", "") {
                    let t = EventTrace {
                        id: e.id.clone(),
                        calendar: e.calendar.clone(),
                        title: e.title.clone(),
                        start: e.start,
                        end: e.end,
                        all_day: e.all_day,
                        location: e.location.clone(),
                    };
                    self.dirty |= self.cards[i].add_event(t);
                }
            }
        }
        // Drive : propriétaire et dernière personne à avoir modifié.
        for f in drive.known_files() {
            if !self.seen.insert(format!("f:{}:{}", f.id, f.modified)) {
                continue;
            }
            for (email, name) in [(&f.owner_email, &f.owner), (&f.modified_by_email, &f.modified_by)] {
                if email.is_empty() || email.eq_ignore_ascii_case(own) {
                    continue;
                }
                if let Some(i) = self.upsert(email, "", name) {
                    self.dirty |= self.cards[i].add_file(f);
                }
            }
        }
    }

    /// Recherche Gmail de tous les mails échangés avec une adresse (bouton de la fiche).
    pub fn search_mails(&mut self, acc: &Account, email: &str) {
        let email = email.to_lowercase();
        if email.is_empty() || !self.searching.insert(email.clone()) {
            return;
        }
        self.search_error = None;
        acc.run(move |s| {
            let q = format!("search:from:{email} OR to:{email} OR cc:{email}");
            let list = gmail::list(s, &q, MAX_MAILS);
            Ok(Event::PersonMails { email, list })
        });
    }

    /// Résultat de [`People::search_mails`].
    pub fn on_mails(&mut self, email: String, list: Result<Vec<Summary>, String>, own: &str) {
        self.searching.remove(&email);
        match list {
            Ok(list) => {
                for m in &list {
                    self.add_mail(m, own);
                }
                if let Some(i) = self.upsert(&email, "", "") {
                    self.cards[i].mails_checked = Local::now().timestamp_millis();
                }
                self.dirty = true;
                self.saved = None;
            }
            Err(e) => self.search_error = Some((email, e)),
        }
    }

    /// Recherche globale : personnes dont le nom ou l'adresse correspond (fiches, annuaire).
    pub fn search(&self, q: &str, contacts: &Contacts, chat: &ChatState, limit: usize) -> Vec<Found> {
        let words: Vec<String> = fold(q).split_whitespace().map(str::to_string).collect();
        if words.is_empty() {
            return Vec::new();
        }
        let matches = |name: &str, email: &str| -> Option<i32> {
            let (n, e) = (fold(name), fold(email));
            let mut s = 0;
            for w in &words {
                let starts = |h: &str| h.split(|c: char| !c.is_alphanumeric()).any(|p| p.starts_with(w.as_str()));
                s += if starts(&n) {
                    30
                } else if n.contains(w.as_str()) {
                    15
                } else if starts(&e) {
                    20
                } else if e.contains(w.as_str()) {
                    8
                } else {
                    return None;
                };
            }
            Some(s)
        };
        let mut out: Vec<Found> = Vec::new();
        let mut keys = HashSet::new();
        for c in &self.cards {
            let name = display_name(c, contacts);
            if let Some(s) = matches(&name, &c.email) {
                let n = c.mails.len() + c.files.len() + c.events.len() + chat_spaces(c, chat).len();
                keys.insert(c.key().to_string());
                out.push(Found { key: c.key().to_string(), sub: summary_line(c, chat), name, score: s + n.min(30) as i32 });
            }
        }
        for c in contacts.search(q, 8) {
            if keys.insert(c.email.clone())
                && let Some(s) = matches(c.display_name(), &c.email)
            {
                out.push(Found { key: c.email.clone(), name: c.display_name().to_string(), sub: c.email.clone(), score: s });
            }
        }
        out.sort_by_key(|f| -f.score);
        out.truncate(limit);
        out
    }
}

/// Nom à afficher : celui de la fiche, sinon de l'annuaire, sinon l'adresse.
pub fn display_name(c: &Card, contacts: &Contacts) -> String {
    if !better_name(&c.name, &c.email) {
        return c.name.clone();
    }
    match contacts.get(&c.email) {
        Some(k) if !k.name.is_empty() => k.name.clone(),
        _ if !c.email.is_empty() => c.email.clone(),
        _ => t!("Utilisateur", "User").into(),
    }
}

/// Identifiant Google Chat de la personne (fiche, sinon d'après son adresse).
fn chat_id(c: &Card, chat: &ChatState) -> Option<String> {
    if !c.chat_id.is_empty() {
        return Some(c.chat_id.clone());
    }
    (!c.email.is_empty()).then(|| chat.people.values().find(|p| p.email.eq_ignore_ascii_case(&c.email)).map(|p| p.id.clone())).flatten()
}

/// Conversations Google Chat où la personne est membre (copie locale), de la plus récente.
fn chat_spaces<'a>(c: &Card, chat: &'a ChatState) -> Vec<&'a crate::google::chat::Space> {
    let Some(id) = chat_id(c, chat) else { return Vec::new() };
    let mut v: Vec<_> = chat.spaces.iter().filter(|s| chat.members.get(&s.name).is_some_and(|m| m.contains(&id))).collect();
    v.sort_by_key(|s| (s.kind != SpaceKind::Dm, std::cmp::Reverse(s.last_active)));
    v
}

fn summary_line(c: &Card, chat: &ChatState) -> String {
    let mut parts = Vec::new();
    if !c.email.is_empty() {
        parts.push(c.email.clone());
    }
    let n = |k: usize, fr: &str, en: &str| format!("{k} {}", if crate::i18n::en() { en } else { fr });
    if !c.mails.is_empty() {
        parts.push(n(c.mails.len(), "mail(s)", "mail(s)"));
    }
    let sp = chat_spaces(c, chat).len();
    if sp > 0 {
        parts.push(n(sp, "conversation(s)", "conversation(s)"));
    }
    if !c.files.is_empty() {
        parts.push(n(c.files.len(), "fichier(s)", "file(s)"));
    }
    if !c.events.is_empty() {
        parts.push(n(c.events.len(), "rendez-vous", "event(s)"));
    }
    parts.join(" · ")
}

// --- Ouverture depuis n'importe où ------------------------------------------------------

fn request_id() -> egui::Id {
    egui::Id::unique("wt_contact_card_request")
}

/// Demande l'ouverture de la fiche d'une personne (adresse ou identifiant Google Chat).
pub fn open(ctx: &egui::Context, key: &str, name: &str) {
    if key.trim().is_empty() {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(request_id(), (key.trim().to_string(), name.to_string())));
    ctx.request_repaint();
}

/// Entrée « Fiche contact » d'un menu contextuel.
pub fn menu_item(ui: &mut egui::Ui, key: &str, name: &str) {
    if key.trim().is_empty() {
        return;
    }
    let who = if !name.trim().is_empty() && !name.contains('@') {
        name.trim()
    } else if key.contains('@') {
        key.trim()
    } else {
        ""
    };
    let label = if who.is_empty() { t!("👤  Fiche contact", "👤  Contact card").to_string() } else { tf!("👤  Fiche contact · {}", "👤  Contact card · {}", who) };
    if ui.button(label).clicked() {
        open(ui.ctx(), key, name);
        ui.close();
    }
}

/// Fiches contact des invités d'un rendez-vous (menu contextuel).
pub fn guests_menu(ui: &mut egui::Ui, attendees: &[String]) {
    ui.set_min_width(220.0);
    if attendees.is_empty() {
        ui.label(RichText::new(t!("Aucun invité", "No guests")).small().color(theme::TEXT_FAINT()));
        return;
    }
    ui.label(RichText::new(t!("INVITÉS", "GUESTS")).small().color(theme::TEXT_FAINT()));
    for a in attendees.iter().take(25) {
        menu_item(ui, a, "");
    }
}

/// Entrée « Fiche contact » pour l'expéditeur d'un mail (« Nom <adresse> »).
pub fn menu_item_mailbox(ui: &mut egui::Ui, mailbox: &str) {
    if let Some((name, email)) = parse_mailbox(mailbox) {
        menu_item(ui, &email, &name);
    }
}

// --- Overlay ----------------------------------------------------------------------------

/// Ce que la fiche demande à l'application.
pub enum Action {
    Mail(String),
    Space(String),
    Drive(DriveFile),
    File(SharedFile),
    /// Rendez-vous (agenda, identifiant, début).
    Event(String, String, i64),
    WriteMail(String),
    ChatWith(String),
    Directory(String),
}

struct Row {
    group: &'static str,
    icon: &'static str,
    title: String,
    sub: String,
    when: String,
    action: Action,
}

#[derive(Default)]
pub struct CardView {
    key: Option<String>,
    /// Demande reçue, ouverte à l'image suivante (menu ou recherche refermés : fond net).
    next: Option<(String, String)>,
    bg: crate::search::Backdrop,
    selected: usize,
}

impl CardView {
    fn close(&mut self) {
        self.key = None;
        self.bg.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(&mut self, ctx: &egui::Context, people: &mut People, mail: &MailState, chat: &ChatState, acc: &Account) -> Option<Action> {
        if let Some((key, name)) = self.next.take() {
            // Identifiant Google Chat : relié à son adresse quand elle est connue.
            let (email, id) = if key.contains('@') { (key.clone(), String::new()) } else { (chat.email_of(&key).unwrap_or_default(), key.clone()) };
            let i = people.upsert(&email, &id, &name).or_else(|| people.index_of(&key));
            if let Some(i) = i {
                self.key = Some(people.cards[i].key().to_string());
                self.selected = 0;
                self.bg.start(ctx);
            }
        }
        if let Some(req) = ctx.data_mut(|d| d.remove_temp::<(String, String)>(request_id())) {
            self.next = Some(req);
            ctx.request_repaint();
        }
        let key = self.key.clone()?;
        let Some(card) = people.find(&key).cloned() else {
            self.close();
            return None;
        };
        let screen = ctx.content_rect();
        let (k, mut close) = self.bg.show(ctx, "wt_card_backdrop");
        let rows = rows(&card, mail, chat);
        if self.selected >= rows.len() {
            self.selected = rows.len().saturating_sub(1);
        }
        let mut go: Option<Action> = None;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                close = true;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !rows.is_empty() {
                self.selected = (self.selected + 1) % rows.len();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) && !rows.is_empty() {
                self.selected = (self.selected + rows.len() - 1) % rows.len();
            }
            if i.consume_key(Modifiers::NONE, Key::Enter)
                && let Some(r) = rows.get(self.selected)
            {
                go = Some(clone_action(&r.action));
            }
        });

        let name = display_name(&card, &mail.contacts);
        let email = card.email.clone();
        let searching = people.searching.contains(&email);
        let error = people.search_error.as_ref().filter(|(e, _)| *e == email).map(|(_, m)| m.clone());
        let mut search = false;
        let w = 640.0_f32.min(screen.width() - 40.0);
        let top = screen.top() + screen.height() * 0.12 - (1.0 - k) * 12.0;
        let area_id = egui::Id::unique("wt_card_box");
        egui::Area::new(area_id).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
            ui.multiply_opacity(k);
            crate::search::overlay_frame().show(ui, |ui| {
                ui.set_width(w - 28.0);
                // En-tête : initiale, nom, adresse, actions.
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(48.0, 48.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 24.0, theme::SURFACE_HI());
                    let initial: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, theme::title_font(21.0), theme::TEXT());
                    ui.vertical(|ui| {
                        ui.add_space(3.0);
                        ui.add(egui::Label::new(RichText::new(&name).font(theme::title_font(19.0)).color(theme::TEXT())).truncate());
                        let sub = if email.is_empty() { t!("Google Chat · adresse inconnue", "Google Chat · unknown address").to_string() } else { email.clone() };
                        ui.add(egui::Label::new(RichText::new(sub).small().color(theme::TEXT_MUTED())).truncate().selectable(true));
                    });
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if !email.is_empty() && ui.button(t!("✉  Écrire", "✉  Write")).clicked() {
                        go = Some(Action::WriteMail(email.clone()));
                    }
                    let dm = chat_spaces(&card, chat).into_iter().find(|s| s.kind == SpaceKind::Dm).map(|s| s.name.clone());
                    if (dm.is_some() || !email.is_empty()) && ui.button("💬  Chat").clicked() {
                        go = Some(match dm {
                            Some(s) => Action::Space(s),
                            None => Action::ChatWith(email.clone()),
                        });
                    }
                    if !email.is_empty() {
                        if ui.button(t!("📇  Annuaire", "📇  Directory")).clicked() {
                            go = Some(Action::Directory(email.clone()));
                        }
                        let label = if searching { t!("Recherche dans Gmail…", "Searching Gmail…") } else { t!("🔎  Chercher ses mails", "🔎  Find their mails") };
                        let tip = match card.mails_checked {
                            0 => t!(
                                "Interroge Gmail pour relier tous les mails échangés avec cette personne (sinon, seuls les mails déjà vus dans l'app sont reliés)",
                                "Asks Gmail for every mail exchanged with this person (otherwise only mails already seen in the app are linked)"
                            )
                            .to_string(),
                            at => tf!("Dernière recherche dans Gmail : {}", "Last Gmail search: {}", short_date(at)),
                        };
                        if ui.add_enabled(!searching, egui::Button::new(label)).on_hover_text(tip).clicked() {
                            search = true;
                        }
                        if searching {
                            ui.add(egui::Spinner::new().size(14.0));
                        }
                    }
                });
                if let Some(e) = &error {
                    ui.label(RichText::new(tf!("Recherche impossible : {}", "Search failed: {}", e)).small().color(theme::TEXT_FAINT()));
                }
                ui.add_space(8.0);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), egui::Stroke::new(1.0, theme::BORDER()));
                ui.add_space(6.0);
                if rows.is_empty() {
                    ui.add_space(14.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(t!("Rien de relié pour l'instant", "Nothing linked yet")).color(theme::TEXT_MUTED()));
                        ui.label(
                            RichText::new(t!(
                                "Mails, conversations, fichiers et rendez-vous s'ajoutent au fil de l'utilisation.",
                                "Mails, conversations, files and events are added as you use the app."
                            ))
                            .small()
                            .color(theme::TEXT_FAINT()),
                        );
                    });
                    ui.add_space(14.0);
                }
                egui::ScrollArea::vertical().max_height(screen.height() * 0.58).auto_shrink([false, true]).show(ui, |ui| {
                    let mut group = "";
                    for (i, r) in rows.iter().enumerate() {
                        if r.group != group {
                            group = r.group;
                            ui.add_space(4.0);
                            let n = rows.iter().filter(|x| x.group == group).count();
                            ui.label(RichText::new(format!("{} · {n}", group_label(group).to_uppercase())).small().strong().color(theme::TEXT_FAINT()));
                        }
                        let sel = i == self.selected;
                        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                            self.selected = i;
                        }
                        let p = ui.painter();
                        if sel {
                            p.rect_filled(rect, CornerRadius::same(8), theme::overlay(14));
                            ui.scroll_to_rect(rect, None);
                        }
                        crate::emoji_img::paint(p, Rect::from_center_size(Pos2::new(rect.left() + 18.0, rect.center().y), vec2(20.0, 20.0)), r.icon, theme::TEXT());
                        let left = rect.left() + 40.0;
                        let dg = one_line(p, &r.when, FontId::proportional(11.0), theme::TEXT_FAINT(), 110.0);
                        let dw = dg.size().x;
                        p.galley(Pos2::new(rect.right() - 10.0 - dw, rect.top() + 5.0), dg, theme::TEXT_FAINT());
                        let g = one_line(p, &r.title, theme::editor_font(14.0, sel), theme::TEXT(), rect.right() - left - dw - 24.0);
                        p.galley(Pos2::new(left, rect.top() + 3.0), g, theme::TEXT());
                        let g = one_line(p, &r.sub, FontId::proportional(11.5), theme::TEXT_MUTED(), rect.right() - left - 70.0);
                        p.galley(Pos2::new(left, rect.top() + 22.0), g, theme::TEXT_MUTED());
                        if resp.clicked() {
                            go = Some(clone_action(&r.action));
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new(t!("↑ ↓ naviguer · Entrée ouvrir · Échap fermer", "↑ ↓ navigate · Enter open · Esc close")).small().color(theme::TEXT_FAINT()));
            });
        });
        // Toujours au-dessus de son fond (voir la recherche).
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, area_id));
        if search {
            people.search_mails(acc, &email);
        }
        if go.is_some() || close {
            self.close();
        }
        go
    }
}

fn clone_action(a: &Action) -> Action {
    match a {
        Action::Mail(x) => Action::Mail(x.clone()),
        Action::Space(x) => Action::Space(x.clone()),
        Action::Drive(f) => Action::Drive(f.clone()),
        Action::File(f) => Action::File(f.clone()),
        Action::Event(c, id, s) => Action::Event(c.clone(), id.clone(), *s),
        Action::WriteMail(x) => Action::WriteMail(x.clone()),
        Action::ChatWith(x) => Action::ChatWith(x.clone()),
        Action::Directory(x) => Action::Directory(x.clone()),
    }
}

fn group_label(g: &'static str) -> &'static str {
    if !crate::i18n::en() {
        return g;
    }
    match g {
        "Conversation" => "Conversation",
        "Groupes" => "Groups",
        "Mails" => "Mails",
        "Fichiers" => "Files",
        "Agenda" => "Calendar",
        other => other,
    }
}

/// Tout ce qui est relié à la personne, par groupe.
fn rows(c: &Card, mail: &MailState, chat: &ChatState) -> Vec<Row> {
    let mut out = Vec::new();
    // Google Chat : message privé, puis groupes.
    for sp in chat_spaces(c, chat) {
        let last = chat.msgs.get(&sp.name).and_then(|l| l.iter().rev().find(|m| !m.text.trim().is_empty()));
        let (group, icon, title) =
            if sp.kind == SpaceKind::Dm { ("Conversation", "💬", t!("Message privé", "Direct message").to_string()) } else { ("Groupes", "👥", chat.title(sp)) };
        let sub = match last {
            Some(m) => format!("{} : {}", chat.name_of(&m.sender), m.text.replace(['\n', '\r'], " ")),
            None if sp.kind == SpaceKind::Dm => "Google Chat".into(),
            None => tf!("Groupe · {} membres", "Group · {} members", chat.members.get(&sp.name).map(|m| m.len()).unwrap_or(sp.members as usize)),
        };
        let when = if sp.last_active > 0 { short_date(sp.last_active) } else { String::new() };
        out.push(Row { group, icon, title, sub, when, action: Action::Space(sp.name.clone()) });
    }
    // Mails.
    let mut threads = HashSet::new();
    for m in c.mails.iter().filter(|m| threads.insert(m.thread.clone())) {
        let sub = match m.role {
            Role::From => t!("Reçu", "Received"),
            Role::To => t!("Envoyé", "Sent"),
            Role::Cc => t!("En copie", "In copy"),
        };
        let title = if m.subject.trim().is_empty() { t!("(sans objet)", "(no subject)").to_string() } else { m.subject.clone() };
        out.push(Row {
            group: "Mails",
            icon: if m.role == Role::From { "📩" } else { "✉️" },
            title,
            sub: sub.into(),
            when: short_date(m.date),
            action: Action::Mail(m.id.clone()),
        });
    }
    // Fichiers : Drive, puis fichiers reçus par mail.
    for f in &c.files {
        let role = if f.owner_email.eq_ignore_ascii_case(&c.email) { t!("Propriétaire", "Owner") } else { t!("Modifié par", "Modified by") };
        let sub = format!("Drive · {} · {role}", crate::google::drive::kind_label(f));
        out.push(Row {
            group: "Fichiers",
            icon: if f.is_folder() { "📁" } else { "📄" },
            title: f.name.clone(),
            sub,
            when: short_date(f.modified),
            action: Action::Drive(f.clone()),
        });
    }
    if !c.email.is_empty() {
        for (m, f) in mail.shared_files.iter().filter(|(m, _)| parse_mailbox(&m.from).is_some_and(|(_, a)| a == c.email)) {
            if c.files.iter().any(|x| x.name == f.title) {
                continue;
            }
            out.push(Row {
                group: "Fichiers",
                icon: "📎",
                title: f.title.clone(),
                sub: t!("Reçu par mail", "Received by mail").into(),
                when: short_date(m.date),
                action: Action::File(f.clone()),
            });
        }
    }
    // Agenda : prochains rendez-vous d'abord, puis les passés (du plus récent).
    let now = Local::now().timestamp_millis();
    let (mut next, past): (Vec<&EventTrace>, Vec<&EventTrace>) = c.events.iter().partition(|e| e.end > now);
    next.sort_by_key(|e| e.start);
    for e in next.into_iter().chain(past) {
        let title = if e.title.is_empty() { t!("(sans titre)", "(untitled)").to_string() } else { e.title.clone() };
        let time = if e.all_day { t!("journée", "all day").to_string() } else { crate::tabs::calendar::hhmm(e.start) };
        let sub = if e.location.is_empty() { time } else { format!("{time} · {}", e.location) };
        let upcoming = e.end > now;
        out.push(Row {
            group: "Agenda",
            icon: if upcoming { "📅" } else { "🗓️" },
            title,
            sub,
            when: short_date(e.start),
            action: Action::Event(e.calendar.clone(), e.id.clone(), e.start),
        });
    }
    out
}

/// Rendez-vous chargé correspondant à une trace (sinon `None` : seul le jour est ouvert).
pub fn live_event(cal: &CalendarState, calendar: &str, id: &str, start: i64) -> Option<CalEvent> {
    let all: Vec<&CalEvent> = cal.events.values().flatten().filter(|e| e.calendar == calendar && e.id == id).collect();
    all.iter().find(|e| e.start == start).or(all.first()).map(|e| (*e).clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, from: &str, to: &str, date: i64) -> Summary {
        Summary { id: id.into(), thread_id: id.into(), from: from.into(), recipients: to.into(), subject: format!("Sujet {id}"), date, ..Default::default() }
    }

    #[test]
    fn links_mails_to_people() {
        let mut p = People::default();
        let own = "moi@studio.fr";
        p.add_mail(&mail("1", "Léa Martin <LEA@studio.fr>", "moi@studio.fr, hugo@studio.fr", 10), own);
        p.add_mail(&mail("2", "Moi <moi@studio.fr>", "Léa <lea@studio.fr>", 20), own);
        p.add_mail(&mail("3", "Robot <noreply@x.fr>", "moi@studio.fr", 30), own);
        let lea = p.find("lea@studio.fr").expect("fiche de Léa");
        assert_eq!(lea.name, "Léa Martin");
        assert_eq!(lea.mails.len(), 2);
        assert_eq!(lea.mails[0].role, Role::To, "le plus récent d'abord : envoyé par moi");
        assert_eq!(lea.mails[1].role, Role::From);
        assert_eq!(p.find("hugo@studio.fr").map(|c| c.mails[0].role), Some(Role::Cc), "co-destinataire");
        assert!(p.find("moi@studio.fr").is_none(), "pas de fiche pour soi-même");
        assert!(p.find("noreply@x.fr").is_none(), "pas de fiche pour les robots");
        // Même mail revu : pas de doublon.
        p.add_mail(&mail("1", "Léa Martin <lea@studio.fr>", "", 10), own);
        assert_eq!(p.find("lea@studio.fr").unwrap().mails.len(), 2);
    }

    #[test]
    fn merges_chat_identity_with_email() {
        let mut p = People::default();
        // Connue d'abord par Google Chat seulement, puis par ses fichiers.
        p.upsert("", "users/7", "Léa M.");
        let f = DriveFile { id: "f1".into(), name: "Planning".into(), owner_email: "lea@studio.fr".into(), modified: 5, ..Default::default() };
        let i = p.upsert("lea@studio.fr", "", "Léa").unwrap();
        p.cards[i].add_file(&f);
        assert_eq!(p.cards.len(), 2);
        // L'adresse de l'identifiant Chat est apprise : une seule fiche.
        p.upsert("lea@studio.fr", "users/7", "");
        assert_eq!(p.cards.len(), 1);
        let c = p.find("users/7").unwrap();
        assert_eq!(c.email, "lea@studio.fr");
        assert_eq!(c.files.len(), 1);
        assert_eq!(p.find("LEA@studio.fr").map(|c| c.chat_id.as_str()), Some("users/7"));
    }

    #[test]
    fn keeps_most_recent_traces() {
        let mut c = Card::default();
        for i in 0..(MAX_MAILS + 5) as i64 {
            c.add_mail(MailTrace { id: i.to_string(), date: i, ..Default::default() });
        }
        assert_eq!(c.mails.len(), MAX_MAILS);
        assert_eq!(c.mails[0].date, MAX_MAILS as i64 + 4);
    }

    #[test]
    fn search_finds_people_by_name_and_address() {
        let mut p = People::default();
        p.upsert("lea.martin@studio.fr", "", "Léa Martin");
        p.upsert("hugo@studio.fr", "", "Hugo Bernard");
        let chat = ChatState::default();
        let book = Contacts::default();
        let r = p.search("lea", &book, &chat, 5);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].key, "lea.martin@studio.fr");
        assert_eq!(p.search("bernard hugo", &book, &chat, 5).len(), 1);
        assert!(p.search("zorglub", &book, &chat, 5).is_empty());
    }

    /// Données de démonstration (copie des conversations + fiches) pour tester l'overlay à l'écran :
    /// `WORKTABLE_DATA=<dossier> cargo test -- --ignored seed_demo`.
    #[test]
    #[ignore]
    fn seed_demo() {
        use crate::google::chat::{ChatMessage, Space};
        use crate::tabs::messages::store;
        assert!(std::env::var_os("WORKTABLE_DATA").is_some(), "à lancer avec WORKTABLE_DATA");
        let now = Local::now().timestamp_millis();
        let sp = |n: &str, name: &str, kind, at| Space { name: n.into(), display_name: name.into(), kind, threaded: false, last_active: at, members: 3, bot_dm: false };
        let spaces = vec![sp("spaces/dm", "", SpaceKind::Dm, now - 3_600_000), sp("spaces/g1", "Équipe rendu", SpaceKind::Space, now - 86_400_000)];
        let mut msgs = HashMap::new();
        msgs.insert(
            "spaces/dm".to_string(),
            vec![ChatMessage {
                name: "spaces/dm/messages/1".into(),
                space: "spaces/dm".into(),
                sender: "users/7".into(),
                text: "Tu as vu le planning ?".into(),
                created: now - 3_600_000,
                ..Default::default()
            }],
        );
        let mut snap = store::Snapshot::new("moi@studio.fr", Some("users/me".into()), &spaces, &msgs, &HashMap::new());
        snap.members = vec![("spaces/dm".into(), vec!["users/me".into(), "users/7".into()]), ("spaces/g1".into(), vec!["users/me".into(), "users/7".into(), "users/8".into()])];
        snap.people = vec![("users/7".into(), "Léa Martin".into(), "lea@studio.fr".into(), None), ("users/8".into(), "Hugo".into(), "hugo@studio.fr".into(), None)];
        store::save_now(&snap);
        let mut p = People { account: "moi@studio.fr".into(), ..Default::default() };
        let i = p.upsert("lea@studio.fr", "users/7", "Léa Martin").unwrap();
        let c = &mut p.cards[i];
        for (k, s) in ["Maquettes v2", "Retours sur le rendu", "Réunion de lundi"].iter().enumerate() {
            c.add_mail(MailTrace {
                id: format!("x{k}"),
                thread: format!("x{k}"),
                subject: s.to_string(),
                date: now - k as i64 * 90_000_000,
                role: if k == 1 { Role::To } else { Role::From },
            });
        }
        c.add_file(&DriveFile {
            id: "f1".into(),
            name: "Planning des rendus".into(),
            mime: "application/vnd.google-apps.spreadsheet".into(),
            owner: "Léa Martin".into(),
            owner_email: "lea@studio.fr".into(),
            modified: now - 7_200_000,
            ..Default::default()
        });
        c.add_file(&DriveFile {
            id: "f2".into(),
            name: "Moodboard".into(),
            mime: "application/vnd.google-apps.folder".into(),
            owner_email: "lea@studio.fr".into(),
            modified: now - 500_000_000,
            ..Default::default()
        });
        c.add_event(EventTrace {
            id: "e1".into(),
            calendar: "primary".into(),
            title: "Revue du rendu".into(),
            start: now + 86_400_000,
            end: now + 90_000_000,
            location: "Salle 2".into(),
            ..Default::default()
        });
        c.add_event(EventTrace {
            id: "e0".into(),
            calendar: "primary".into(),
            title: "Point hebdo".into(),
            start: now - 600_000_000,
            end: now - 596_000_000,
            ..Default::default()
        });
        write(&p.snapshot());
        assert!(load().is_some_and(|s| s.cards.len() == 1));
    }

    #[test]
    fn snapshot_round_trip() {
        let mut p = People::default();
        p.upsert("lea@studio.fr", "users/7", "Léa");
        p.account = "moi@studio.fr".into();
        let json = serde_json::to_vec(&p.snapshot()).unwrap();
        let back: Snapshot = serde_json::from_slice(&json).unwrap();
        assert_eq!(back.account, "moi@studio.fr");
        assert_eq!(back.cards[0].chat_id, "users/7");
    }
}
