//! Onglet Messages : Google Chat (espaces, messages privés, fils, réactions, pièces jointes).
//!
//! Économie de quota, comme pour Gmail :
//! - la liste des espaces (1 requête) sert de signal : seuls les espaces dont la date de
//!   dernière activité a changé sont relus, et seulement les messages postérieurs au dernier connu ;
//! - la fenêtre récente de l'espace ouvert est rafraîchie au plus une fois par minute
//!   (modifications, réactions, suppressions) ;
//! - les noms (API People) et les membres sont mis en cache, les noms sur disque ;
//! - toutes les requêtes passent par le limiteur et les reprises de la session Google.

pub mod emoji;
pub mod format;
pub mod gif;
pub mod gif_picker;
mod manage;
mod new_group;
pub mod pins;
pub mod store;
mod view;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::google::chat::{self, ChatEvent, ChatMessage, Person, Section, Space, SpaceKind};
use crate::google::{Account, Event};
use crate::tabs::mail::inline::{self, InlineImages};

pub use view::show;

/// Relève de la liste des espaces : onglet affiché / en arrière-plan.
const POLL_VISIBLE: Duration = Duration::from_secs(8);
const POLL_HIDDEN: Duration = Duration::from_secs(45);
/// Rafraîchissement complet de la fenêtre récente de l'espace ouvert.
const FULL_REFRESH: Duration = Duration::from_secs(60);
/// Relecture des sections (dossiers) de Google Chat.
const SECTIONS_EVERY: Duration = Duration::from_secs(300);
/// Nombre de personnes dans la catégorie « Récents ».
pub const RECENT_PEOPLE: usize = 8;
/// Taille d'une page de messages.
const PAGE: u32 = 60;
/// Photos de profil téléchargées en même temps.
const PHOTOS_IN_FLIGHT: usize = 6;
/// Nombre d'espaces récents dont on lit l'état de lecture et les membres.
const RECENT: usize = 60;
/// Copie locale chiffrée : enregistrée au plus toutes les 20 s quand quelque chose a changé.
const STORE_EVERY: Duration = Duration::from_secs(20);
/// Mise à jour automatique des conversations récentes : une demande toutes les 1,5 s, 2 à la fois.
const SYNC_GAP: Duration = Duration::from_millis(1500);
const SYNC_IN_FLIGHT: usize = 2;

#[derive(Default, Clone)]
pub struct NewGroup {
    pub name: String,
    pub members: String,
    /// Espace nommé (sinon discussion de groupe).
    pub space: bool,
}

pub struct PendingFile {
    pub name: String,
    pub mime: String,
    pub data: Vec<u8>,
}

#[derive(Default)]
pub struct ChatState {
    pub spaces: Vec<Space>,
    pub loading: bool,
    /// Sections de Google Chat (ordre et noms de l'utilisateur) ; vide = disposition par défaut.
    pub sections: Vec<Section>,
    /// Sections indisponibles (autorisation à accorder) : message affiché.
    pub sections_error: Option<String>,
    last_sections: Option<Instant>,
    /// Adresse du compte (pour reconnaître son propre identifiant Chat).
    own_email: String,
    /// Conversations rendues muettes (copie des préférences, mise à jour à chaque image).
    pub muted: Vec<String>,
    /// Images choisies pour les groupes : espace → fichier local.
    pub icons: HashMap<String, PathBuf>,
    icons_loaded: bool,
    photos_in_flight: usize,
    pub people_query: String,
    /// Réactions mises depuis WorkTable (message, émoji) : savoir s'il faut ajouter ou retirer.
    my_reactions: HashSet<(String, String)>,
    local_seq: u64,
    /// Messages en cours d'envoi : de quoi les renvoyer en cas d'échec.
    retry_data: HashMap<String, (String, String, Option<String>, Vec<PendingFile>)>,
    /// Formulaire de création de groupe ouvert.
    pub new_group: Option<NewGroup>,
    /// Sélecteur de réactions ouvert pour ce message (position à l'écran).
    pub react_picker: Option<(ChatMessage, eframe::egui::Pos2)>,
    pub emoji_query: String,
    /// Sélecteur d'émojis du champ de saisie ouvert (brouillon, position du bouton).
    pub composer_picker: Option<(String, eframe::egui::Pos2)>,
    pub recent_emojis: Vec<String>,
    pub emoji_group: usize,
    pub people: HashMap<String, Person>,
    people_pending: HashSet<String>,
    pub members: HashMap<String, Vec<String>>,
    members_pending: HashSet<String>,
    /// Messages par espace, du plus ancien au plus récent.
    pub msgs: HashMap<String, Vec<ChatMessage>>,
    pub has_more: HashMap<String, bool>,
    pub loading_msgs: HashSet<String>,
    pub last_read: HashMap<String, i64>,
    pub me: Option<String>,
    me_requested: bool,
    pub open: Option<String>,
    /// Fil ouvert dans le panneau de droite.
    pub thread: Option<String>,
    /// Brouillons par espace ou par fil.
    pub drafts: HashMap<String, String>,
    pub files: Vec<PendingFile>,
    pub sending: bool,
    pub editing: Option<(String, String)>,
    pub query: String,
    pub new_dm: Option<String>,
    /// Panneau de création de groupe (ouvert tant que `new_group` est `Some`).
    pub group_panel: new_group::Panel,
    /// Panneau de gestion d'un groupe (renommer, membres, supprimer).
    pub manage: Option<manage::Panel>,
    pub suggest: crate::tabs::mail::recipients::Suggest,
    pub need_consent: bool,
    pub api_disabled: bool,
    pub app_missing: bool,
    pub notice: Option<(String, bool, Instant)>,
    pub images: InlineImages,
    /// GIF animés (toutes leurs images).
    pub gifs: gif::Gifs,
    /// Sélecteur de GIF ouvert ; clé GIPHY et favoris (copiés des réglages à chaque image).
    pub gif_picker: Option<gif_picker::Picker>,
    pub giphy_key: String,
    pub gif_favorites: Vec<crate::prefs::GifItem>,
    pub scroll_bottom: bool,
    /// Demandes vers l'onglet Mail (écrire un mail à cette adresse).
    pub mail_to: Option<String>,
    /// Pièce jointe affichée dans la visionneuse.
    pub viewer: Option<chat::ChatAttachment>,
    /// Conversation privée à ouvrir dès que possible (demandée depuis l'onglet Mail).
    pending_dm: Option<String>,
    last_spaces: Option<Instant>,
    spaces_inflight: bool,
    read_requested: HashSet<String>,
    last_full: Option<Instant>,
    people_loaded: bool,
    /// Messages épinglés (affichés dans le tableau de bord).
    pub pins: pins::Pins,
    pins_loaded: bool,
    /// Copie locale chiffrée (voir `store`) : relue, à enregistrer.
    store_loaded: bool,
    /// Lecture de la copie en cours (fil à part).
    store_rx: Option<std::sync::mpsc::Receiver<Option<store::Snapshot>>>,
    /// Compte auquel appartiennent les données affichées (copie locale ou Google).
    store_account: String,
    /// Noms des conversations gardés dans la copie (avant que membres et personnes soient relus).
    titles: HashMap<String, String>,
    /// Ancien fichier des personnes en clair, à supprimer une fois repris dans la copie chiffrée.
    legacy_people: bool,
    store_dirty: bool,
    store_saved: Option<Instant>,
    /// Conversations récentes déjà mises à jour (espace → dernière activité connue).
    synced: HashMap<String, i64>,
    last_sync: Option<Instant>,
    /// Vérification des noms en arrière-plan : dernière passe.
    last_audit: Option<Instant>,
    /// Conversations dont les membres n'ont pas pu être lus : (dernier essai, nombre d'essais).
    members_retry: HashMap<String, (Instant, u32)>,
    /// Personnes sans nom ni adresse (ou en échec) : (dernier essai, nombre d'essais).
    people_retry: HashMap<String, (Instant, u32)>,
}

/// Passe de vérification des noms des conversations.
const AUDIT_EVERY: Duration = Duration::from_secs(20);
/// Conversations dont on relit les membres à chaque passe (au plus).
const AUDIT_MEMBERS: usize = 30;
/// Personnes relues à chaque passe (au plus).
const AUDIT_PEOPLE: usize = 40;

/// Attente avant un nouvel essai : 1 min, 5 min, 30 min, puis toutes les 3 h.
fn retry_due(entry: Option<&(Instant, u32)>) -> bool {
    let Some((at, n)) = entry else { return true };
    let wait = match n {
        0 | 1 => 60,
        2 => 300,
        3 => 1800,
        _ => 3 * 3600,
    };
    at.elapsed() >= Duration::from_secs(wait)
}

impl ChatState {
    // --- Relève ---------------------------------------------------------------------

    /// À appeler à chaque image : relève espacée, plus fréquente quand l'onglet est affiché.
    pub fn background(&mut self, acc: &Account, visible: bool, ctx: &eframe::egui::Context) {
        // Conversations gardées sur cet ordinateur : relues dès le lancement, dans un fil à part
        // (avant même la connexion à Google), puis affichées aussitôt, même hors ligne.
        if !self.store_loaded {
            self.store_loaded = true;
            self.store_rx = Some(store::load_async());
        }
        if let Some(rx) = &self.store_rx {
            match rx.try_recv() {
                Ok(snap) => {
                    self.store_rx = None;
                    if let Some(snap) = snap {
                        self.restore(snap);
                    }
                    ctx.request_repaint();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(30)),
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.store_rx = None,
            }
        }
        if !self.people_loaded {
            self.load_people();
        }
        if !self.pins_loaded {
            self.pins = pins::Pins::load();
            self.pins_loaded = true;
        }
        if let Some(s) = acc.session()
            && self.own_email.is_empty()
        {
            self.own_email = s.email.to_lowercase();
        }
        // Copie d'un autre compte : rien ne doit en rester.
        if self.store_rx.is_none() && !self.own_email.is_empty() && self.store_account != self.own_email {
            if !self.store_account.is_empty() {
                self.forget_account();
            }
            self.store_account = self.own_email.clone();
        }
        if acc.session().is_none() {
            // Déconnecté : la copie (effacée à la déconnexion) n'est plus réécrite.
            self.store_dirty = false;
        } else if self.store_dirty && self.store_saved.is_none_or(|t| t.elapsed() >= STORE_EVERY) {
            self.save_store();
        }
        if !self.icons_loaded {
            self.load_icons(acc);
            // Émojis les plus utilisés téléchargés d'avance : réactions instantanées.
            let mut list: Vec<String> = ["👍", "❤️", "😂", "🎉", "👀", "🔥", "🙏", "✅", "👏", "😮", "😢", "🚀", "💯", "🤔", "😍", "😅"].iter().map(|s| s.to_string()).collect();
            list.extend(self.recent_emojis.iter().cloned());
            list.extend(
                emojis::iter()
                    .filter(|e| matches!(e.group(), emojis::Group::SmileysAndEmotion) || e.group() == emojis::Group::PeopleAndBody)
                    .filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
                    .take(260)
                    .map(|e| e.as_str().to_string()),
            );
            crate::emoji_img::prefetch(list);
        }
        if visible {
            self.request_photos(acc);
        }
        if acc.session().is_none() || self.need_consent || self.api_disabled || self.app_missing {
            return;
        }
        if self.me.is_none() && !self.me_requested {
            self.me_requested = true;
            // Silencieux : à défaut, l'identifiant est déduit des membres des conversations.
            acc.run(|s| {
                Ok(Event::Chat(match chat::me(s) {
                    Some(id) => ChatEvent::Me(id),
                    None => ChatEvent::ReadStates(Vec::new()),
                }))
            });
        }
        if let Some(email) = self.pending_dm.take() {
            self.open_dm(acc, &email);
        }
        if self.last_sections.is_none_or(|t| t.elapsed() >= SECTIONS_EVERY) {
            self.last_sections = Some(Instant::now());
            acc.run(|s| Ok(Event::Chat(ChatEvent::Sections(chat::sections(s)))));
        }
        let every = if visible { POLL_VISIBLE } else { POLL_HIDDEN };
        if !self.spaces_inflight && self.last_spaces.is_none_or(|t| t.elapsed() >= every) {
            self.spaces_inflight = true;
            self.loading = self.spaces.is_empty();
            self.last_spaces = Some(Instant::now());
            run(acc, |s| chat::spaces(s).map(ChatEvent::Spaces));
        }
        if visible
            && let Some(sp) = self.open.clone()
            && self.msgs.contains_key(&sp)
            && !self.loading_msgs.contains(&sp)
            && self.last_full.is_some_and(|t| t.elapsed() >= FULL_REFRESH)
        {
            self.refresh_window(acc, &sp);
        }
        self.sync_recent(acc);
        if !self.spaces.is_empty() {
            self.audit_names(acc);
        }
        ctx.request_repaint_after(every);
    }

    // --- Copie locale -----------------------------------------------------------------

    /// Reprend la copie locale (sans écraser ce qui est déjà chargé depuis Google).
    fn restore(&mut self, snap: store::Snapshot) {
        self.store_account = snap.account;
        if self.spaces.is_empty() {
            self.spaces = snap.spaces;
            self.loading = false;
        }
        for (sp, title) in snap.titles {
            self.titles.entry(sp).or_insert(title);
        }
        for (sp, m) in snap.members {
            self.members.entry(sp).or_insert(m);
        }
        for (id, name, email, photo) in snap.people {
            self.people.entry(id.clone()).or_insert(Person { id, name, email, photo });
        }
        if self.sections.is_empty() {
            self.sections = snap.sections;
        }
        for (sp, list) in snap.msgs {
            if !self.msgs.contains_key(&sp) {
                self.has_more.insert(sp.clone(), list.len() >= PAGE as usize);
                self.msgs.insert(sp, list);
            }
        }
        for (sp, t) in snap.last_read {
            self.last_read.entry(sp).or_insert(t);
        }
        if self.me.is_none() {
            self.me = snap.me;
        }
    }

    fn snapshot(&self) -> Option<store::Snapshot> {
        if self.own_email.is_empty() || self.spaces.is_empty() || self.store_account != self.own_email {
            return None;
        }
        let mut snap = store::Snapshot::new(&self.own_email, self.me.clone(), &self.spaces, &self.msgs, &self.last_read);
        snap.titles = self.spaces.iter().filter_map(|s| self.known_title(s).map(|t| (s.name.clone(), t))).collect();
        snap.members = self.members.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        snap.people = self.people.values().map(|p| (p.id.clone(), p.name.clone(), p.email.clone(), p.photo.clone())).collect();
        snap.sections = self.sections.clone();
        Some(snap)
    }

    fn save_store(&mut self) {
        self.store_dirty = false;
        self.store_saved = Some(Instant::now());
        if let Some(snap) = self.snapshot() {
            let legacy = std::mem::take(&mut self.legacy_people).then(people_path).flatten();
            store::save(snap, legacy);
        }
    }

    /// Données d'un autre compte (copie locale d'un compte précédent) : effacées.
    fn forget_account(&mut self) {
        store::clear();
        self.spaces.clear();
        self.msgs.clear();
        self.has_more.clear();
        self.members.clear();
        self.last_read.clear();
        self.sections.clear();
        self.titles.clear();
        self.synced.clear();
        self.open = None;
        self.me = None;
    }

    /// À la fermeture de l'app : dernier enregistrement, sans attendre.
    pub fn flush_store(&mut self) {
        if self.store_dirty
            && let Some(snap) = self.snapshot()
        {
            self.store_dirty = false;
            store::save_now(&snap);
        }
    }

    /// Garde à jour, en arrière-plan, les messages des conversations les plus récentes :
    /// une conversation dont l'activité a changé est relue (seulement les nouveaux messages).
    fn sync_recent(&mut self, acc: &Account) {
        if self.loading_msgs.len() >= SYNC_IN_FLIGHT || self.last_sync.is_some_and(|t| t.elapsed() < SYNC_GAP) {
            return;
        }
        let next =
            store::recent_spaces(&self.spaces, store::KEEP_SPACES).into_iter().find(|s| !self.loading_msgs.contains(&s.name) && self.synced.get(&s.name) != Some(&s.last_active));
        if let Some(sp) = next {
            self.last_sync = Some(Instant::now());
            self.synced.insert(sp.name.clone(), sp.last_active);
            self.fetch_new(acc, &sp.name);
        }
    }

    pub fn handle(&mut self, ev: ChatEvent, acc: &Account) {
        // Ce qui modifie les conversations (contenu, noms, membres, sections) est à recopier sur le disque.
        self.store_dirty |= matches!(
            ev,
            ChatEvent::Me(_)
                | ChatEvent::Spaces(_)
                | ChatEvent::Messages { .. }
                | ChatEvent::ReadStates(_)
                | ChatEvent::Members(_)
                | ChatEvent::People(_)
                | ChatEvent::Sections(_)
                | ChatEvent::Sent { .. }
                | ChatEvent::Updated { .. }
                | ChatEvent::Deleted { .. }
                | ChatEvent::Reacted { .. }
        );
        match ev {
            ChatEvent::Me(id) => self.me = Some(id),
            ChatEvent::Spaces(list) => {
                self.spaces_inflight = false;
                self.loading = false;
                let before: HashMap<&str, i64> = self.spaces.iter().map(|s| (s.name.as_str(), s.last_active)).collect();
                let changed_open = self.open.as_ref().is_some_and(|o| list.iter().any(|s| s.name == *o && before.get(o.as_str()).is_some_and(|t| s.last_active > *t)));
                self.spaces = list;
                if changed_open && let Some(o) = self.open.clone() {
                    self.fetch_new(acc, &o);
                }
                self.request_read_states(acc);
                self.request_members(acc);
            }
            ChatEvent::Messages { space, list, replace_from, older, more } => {
                self.loading_msgs.remove(&space);
                if older || replace_from == Some(0) {
                    self.has_more.insert(space.clone(), more);
                }
                if replace_from.is_some() && !older {
                    self.last_full = Some(Instant::now());
                }
                let senders: Vec<String> = list.iter().map(|m| m.sender.clone()).collect();
                let added = merge(self.msgs.entry(space.clone()).or_default(), list, replace_from);
                self.request_people(acc, senders);
                if self.open.as_deref() == Some(space.as_str()) && added && !older {
                    self.scroll_bottom = true;
                    self.mark_read(acc, &space);
                }
            }
            ChatEvent::ReadStates(list) => self.last_read.extend(list),
            ChatEvent::Members(list) => {
                let mut users = Vec::new();
                for (sp, m) in list {
                    self.members_pending.remove(&sp);
                    let Some(m) = m else {
                        // Lecture impossible : nouvel essai plus tard (vérification en arrière-plan).
                        let n = self.members_retry.get(&sp).map(|r| r.1).unwrap_or(0) + 1;
                        self.members_retry.insert(sp, (Instant::now(), n));
                        continue;
                    };
                    // Nom affiché fourni avec les membres : repris si la personne n'en a pas.
                    for (id, name) in &m {
                        if name.trim().is_empty() {
                            continue;
                        }
                        let p = self.people.entry(id.clone()).or_insert_with(|| Person { id: id.clone(), name: String::new(), email: String::new(), photo: None });
                        if p.name.is_empty() {
                            p.name = name.clone();
                        }
                    }
                    let ids: Vec<String> = m.into_iter().map(|(id, _)| id).collect();
                    // Conversation privée sans l'autre personne : membres à relire plus tard.
                    if ids.iter().all(|u| Some(u.as_str()) == self.me.as_deref()) {
                        let n = self.members_retry.get(&sp).map(|r| r.1).unwrap_or(0) + 1;
                        self.members_retry.insert(sp.clone(), (Instant::now(), n));
                    } else {
                        self.members_retry.remove(&sp);
                    }
                    users.extend(ids.iter().cloned());
                    self.members.insert(sp, ids);
                }
                self.request_people(acc, users);
                self.infer_me();
            }
            ChatEvent::People(list) => {
                for p in list {
                    self.people_pending.remove(&p.id);
                    if p.name.is_empty() && p.email.is_empty() {
                        // Introuvable pour l'instant : nouvel essai plus tard ; on garde ce qu'on savait.
                        let n = self.people_retry.get(&p.id).map(|r| r.1).unwrap_or(0) + 1;
                        self.people_retry.insert(p.id.clone(), (Instant::now(), n));
                        self.people.entry(p.id.clone()).or_insert(p);
                        continue;
                    }
                    self.people_retry.remove(&p.id);
                    let known = self.people.get(&p.id).map(|x| x.name.clone()).unwrap_or_default();
                    let mut p = p;
                    if p.name.is_empty() {
                        p.name = known;
                    }
                    self.people.insert(p.id.clone(), p);
                }
                self.save_people();
                self.infer_me();
            }
            ChatEvent::PeopleFailed(ids) => {
                for id in ids {
                    self.people_pending.remove(&id);
                    let n = self.people_retry.get(&id).map(|r| r.1).unwrap_or(0) + 1;
                    self.people_retry.insert(id, (Instant::now(), n));
                }
            }
            ChatEvent::Sent { space, msg, local } => {
                if self.me.is_none() && !msg.sender.is_empty() {
                    self.me = Some(msg.sender.clone());
                }
                self.retry_data.remove(&local);
                let list = self.msgs.entry(space.clone()).or_default();
                list.retain(|m| m.name != local);
                merge(list, vec![msg], None);
                self.scroll_bottom = true;
                self.mark_read(acc, &space);
            }
            ChatEvent::SendFailed { space, local, error } => {
                if let Some(m) = self.msgs.get_mut(&space).and_then(|l| l.iter_mut().find(|m| m.name == local)) {
                    m.name = local.replacen("local/", "failed/", 1);
                }
                self.error(tf!("Message non envoyé : {error}", "Message not sent: {error}"));
            }
            ChatEvent::Noop => {}
            ChatEvent::Updated(m) | ChatEvent::Reacted(m) => {
                let sp = m.space.clone();
                merge(self.msgs.entry(sp).or_default(), vec![m], None);
            }
            ChatEvent::Deleted { space, name } => {
                if let Some(l) = self.msgs.get_mut(&space) {
                    l.retain(|m| m.name != name);
                }
                self.info(t!("Message supprimé", "Message deleted"));
            }
            ChatEvent::Gif { key, frames } => self.gifs.on_loaded(key, frames),
            ChatEvent::GifResults { query, result } => {
                if let Some(p) = self.gif_picker.as_mut() {
                    p.on_results(&query, result);
                }
            }
            ChatEvent::Image { key, image } => {
                if key.starts_with("photo:") {
                    self.photos_in_flight = self.photos_in_flight.saturating_sub(1);
                }
                self.images.on_loaded(key, image);
            }
            ChatEvent::File { path } => {
                if let Err(e) = open::that(&path) {
                    self.error(tf!("Impossible d'ouvrir le fichier : {e}", "Cannot open the file: {e}"));
                }
            }
            ChatEvent::Dm(sp) => {
                let name = sp.name.clone();
                if !self.spaces.iter().any(|s| s.name == name) {
                    self.spaces.insert(0, sp);
                }
                self.request_members(acc);
                self.select(acc, &name);
            }
            ChatEvent::Sections(r) => match r {
                Ok(list) => {
                    self.sections = list;
                    self.sections_error = None;
                }
                Err(e) => self.sections_error = Some(e),
            },
            ChatEvent::Memberships { space, list } => {
                if let Ok(list) = &list {
                    self.members.insert(space.clone(), list.iter().map(|m| m.user.clone()).collect());
                    self.titles.remove(&space);
                }
                if let Some(p) = self.manage.as_mut().filter(|p| p.space == space) {
                    p.members = Some(list);
                    p.busy = false;
                }
            }
            ChatEvent::MembersChanged { space, error } => {
                if let Some(p) = self.manage.as_mut().filter(|p| p.space == space) {
                    p.error = error.map(|e| manage_error(&e));
                    p.busy = true;
                }
                self.fetch_memberships(acc, &space);
            }
            ChatEvent::SpaceRenamed(sp) => {
                if let Some(s) = self.spaces.iter_mut().find(|s| s.name == sp.name) {
                    s.display_name = sp.display_name.clone();
                }
                self.titles.remove(&sp.name);
                self.manage = None;
                self.info(t!("Groupe renommé", "Group renamed"));
            }
            ChatEvent::SpaceDeleted(name) => {
                self.spaces.retain(|s| s.name != name);
                self.msgs.remove(&name);
                self.members.remove(&name);
                if self.open.as_deref() == Some(name.as_str()) {
                    self.open = None;
                    self.thread = None;
                }
                self.manage = None;
                self.info(t!("Groupe supprimé", "Group deleted"));
            }
            ChatEvent::ManageFailed(e) => {
                if let Some(p) = self.manage.as_mut() {
                    p.busy = false;
                    p.error = Some(manage_error(&e));
                } else {
                    self.error(manage_error(&e));
                }
            }
            ChatEvent::MoveFailed(e) => {
                let low = e.to_lowercase();
                let msg = if low.contains("scope") || low.contains("insufficient") {
                    t!(
                        "Déplacement impossible : reconnecte-toi (Réglages › Google) pour autoriser la modification des sections.",
                        "Cannot move: sign in again (Settings › Google) to allow editing sections."
                    )
                    .to_string()
                } else {
                    tf!("Déplacement impossible : {e}", "Cannot move: {e}")
                };
                self.error(msg);
                self.last_sections = None;
            }
            ChatEvent::Error(e) => {
                self.spaces_inflight = false;
                self.sending = false;
                self.loading = false;
                self.loading_msgs.clear();
                let low = e.to_lowercase();
                if low.contains("chat app not found") || low.contains("google chat app") {
                    self.app_missing = true;
                } else if low.contains("insufficient") || low.contains("scope") {
                    self.need_consent = true;
                } else if low.contains("has not been used") || low.contains("accessnotconfigured") || low.contains("service_disabled") || low.contains("is disabled") {
                    self.api_disabled = true;
                } else {
                    self.error(e);
                }
            }
        }
    }

    /// Après une connexion (ou reconnexion) : tout est relu depuis Google, mais ce qui est déjà
    /// affiché (copie locale comprise) reste visible en attendant.
    pub fn on_ready(&mut self) {
        let old = std::mem::take(self);
        *self = ChatState {
            people: old.people,
            people_loaded: true,
            pins: old.pins,
            pins_loaded: true,
            spaces: old.spaces,
            msgs: old.msgs,
            has_more: old.has_more,
            members: old.members,
            last_read: old.last_read,
            sections: old.sections,
            me: old.me,
            open: old.open,
            titles: old.titles,
            store_loaded: old.store_loaded,
            store_rx: old.store_rx,
            store_account: old.store_account,
            legacy_people: old.legacy_people,
            ..Default::default()
        };
    }

    pub fn retry(&mut self) {
        self.need_consent = false;
        self.api_disabled = false;
        self.app_missing = false;
        self.last_spaces = None;
        self.last_sections = None;
        self.me_requested = false;
    }

    /// Range une conversation dans une autre section (affichage immédiat, puis Google Chat).
    pub fn move_space(&mut self, acc: &Account, space: &str, target_id: &str) {
        let Some(target) = self.sections.iter().find(|x| x.id == target_id).map(|x| x.name.clone()) else { return };
        let mut item = None;
        for sec in self.sections.iter_mut() {
            if let Some(i) = sec.items.remove(space) {
                item = Some(i);
            }
            sec.spaces.retain(|x| x != space);
        }
        if let Some(sec) = self.sections.iter_mut().find(|x| x.id == target_id) {
            sec.spaces.push(space.to_string());
        }
        let space = space.to_string();
        acc.run(move |s| {
            Ok(Event::Chat(match chat::move_to_section(s, &space, item.as_deref(), &target) {
                Ok(()) => ChatEvent::Sections(chat::sections(s)),
                Err(e) => ChatEvent::MoveFailed(e),
            }))
        });
    }

    pub fn create_group(&mut self, acc: &Account) {
        let Some(g) = self.new_group.clone() else { return };
        let emails: Vec<String> =
            crate::tabs::mail::contacts::split_list(&g.members).iter().filter_map(|m| crate::tabs::mail::contacts::parse_mailbox(m).map(|(_, e)| e)).collect();
        if g.space && g.name.trim().is_empty() {
            return self.info(t!("Donne un nom à l'espace", "Give the space a name"));
        }
        if !g.space && emails.len() < 2 {
            return self.info(t!("Une discussion de groupe demande au moins deux personnes", "A group chat needs at least two people"));
        }
        self.new_group = None;
        self.info(t!("Création du groupe…", "Creating the group…"));
        let name = if g.space { g.name } else { String::new() };
        run(acc, move |s| chat::create_space(s, &name, &emails).map(ChatEvent::Dm));
    }

    // --- Gestion d'un groupe (renommer, membres, supprimer) --------------------------

    pub fn open_manage(&mut self, ctx: &eframe::egui::Context, acc: &Account, space: &str, mode: manage::Mode) {
        let title = self.space(space).map(|s| self.title(s)).unwrap_or_default();
        self.manage = Some(manage::Panel::new(ctx, space, &title, mode));
        if mode == manage::Mode::Members {
            self.fetch_memberships(acc, space);
        }
    }

    fn fetch_memberships(&self, acc: &Account, space: &str) {
        let space = space.to_string();
        run(acc, move |s| Ok(ChatEvent::Memberships { list: chat::memberships(s, &space), space }));
    }

    pub fn manage_request(&mut self, acc: &Account, req: manage::Request) {
        let Some(p) = self.manage.as_mut() else { return };
        let space = p.space.clone();
        p.error = None;
        match req {
            manage::Request::Close => self.manage = None,
            manage::Request::Rename(name) => {
                p.busy = true;
                run(acc, move |s| Ok(chat::rename_space(s, &space, &name).map(ChatEvent::SpaceRenamed).unwrap_or_else(ChatEvent::ManageFailed)));
            }
            manage::Request::Delete => {
                p.busy = true;
                run(acc, move |s| Ok(chat::delete_space(s, &space).map(|_| ChatEvent::SpaceDeleted(space.clone())).unwrap_or_else(ChatEvent::ManageFailed)));
            }
            manage::Request::Add(email) => {
                p.busy = true;
                run(acc, move |s| Ok(ChatEvent::MembersChanged { error: chat::add_member(s, &space, &email).err(), space }));
            }
            manage::Request::Remove(membership) => {
                p.busy = true;
                run(acc, move |s| Ok(ChatEvent::MembersChanged { error: chat::remove_member(s, &membership).err(), space }));
            }
        }
    }

    pub fn retry_sections(&mut self) {
        self.sections_error = None;
        self.last_sections = None;
    }

    // --- Actions --------------------------------------------------------------------

    pub fn select(&mut self, acc: &Account, space: &str) {
        if self.open.as_deref() != Some(space) {
            self.thread = None;
            self.editing = None;
            self.files.clear();
        }
        self.open = Some(space.to_string());
        self.scroll_bottom = true;
        if self.msgs.contains_key(space) {
            self.fetch_new(acc, space);
        } else {
            self.refresh_window(acc, space);
        }
        self.mark_read(acc, space);
    }

    /// Relit les messages récents (remplace la fenêtre : modifications et suppressions comprises).
    fn refresh_window(&mut self, acc: &Account, space: &str) {
        if !self.loading_msgs.insert(space.to_string()) {
            return;
        }
        self.last_full = Some(Instant::now());
        let first = !self.msgs.contains_key(space);
        let sp = space.to_string();
        run(acc, move |s| {
            let (list, more) = chat::messages(s, &sp, None, None, PAGE)?;
            let from = if first { 0 } else { list.first().map(|m| m.created).unwrap_or(0) };
            Ok(ChatEvent::Messages { space: sp, list, replace_from: Some(from), older: false, more })
        });
    }

    /// Seulement les messages postérieurs au dernier connu.
    fn fetch_new(&mut self, acc: &Account, space: &str) {
        let Some(newest) = self.msgs.get(space).and_then(|l| l.last()).map(|m| m.created) else {
            return self.refresh_window(acc, space);
        };
        if !self.loading_msgs.insert(space.to_string()) {
            return;
        }
        let sp = space.to_string();
        run(acc, move |s| {
            let (list, _) = chat::messages(s, &sp, Some(newest), None, PAGE)?;
            Ok(ChatEvent::Messages { space: sp, list, replace_from: None, older: false, more: false })
        });
    }

    pub fn load_older(&mut self, acc: &Account, space: &str) {
        let Some(oldest) = self.msgs.get(space).and_then(|l| l.first()).map(|m| m.created) else { return };
        if !self.loading_msgs.insert(space.to_string()) {
            return;
        }
        let sp = space.to_string();
        run(acc, move |s| {
            let (list, more) = chat::messages(s, &sp, None, Some(oldest), PAGE)?;
            Ok(ChatEvent::Messages { space: sp, list, replace_from: None, older: true, more })
        });
    }

    fn mark_read(&mut self, acc: &Account, space: &str) {
        let now = chrono::Utc::now().timestamp_millis();
        let newest = self.spaces.iter().find(|s| s.name == space).map(|s| s.last_active).unwrap_or(0);
        if self.last_read.get(space).is_some_and(|t| *t >= newest) {
            return;
        }
        self.last_read.insert(space.to_string(), now);
        let sp = space.to_string();
        acc.run(move |s| {
            // Échec silencieux : l'état de lecture n'est qu'un confort.
            let _ = chat::mark_read(s, &sp, now);
            Ok(Event::Chat(ChatEvent::ReadStates(Vec::new())))
        });
    }

    /// Envoi « optimiste » : le message apparaît tout de suite (estompé, « Envoi… »), le champ
    /// est vidé, puis l'aperçu est remplacé par le message confirmé par Google.
    pub fn send(&mut self, acc: &Account, in_thread: bool) {
        let Some(space) = self.open.clone() else { return };
        let thread = if in_thread { self.thread.clone() } else { None };
        let key = draft_key(&space, thread.as_deref());
        let text = self.drafts.get(&key).map(|t| format::markdown_to_chat(t.trim())).unwrap_or_default();
        let files: Vec<PendingFile> = if in_thread { Vec::new() } else { std::mem::take(&mut self.files) };
        if text.is_empty() && files.is_empty() {
            return;
        }
        self.drafts.remove(&key);
        self.local_seq += 1;
        let local = format!("local/{}", self.local_seq);
        let preview = ChatMessage {
            name: local.clone(),
            space: space.clone(),
            sender: self.me.clone().unwrap_or_else(|| ME_LOCAL.into()),
            created: chrono::Utc::now().timestamp_millis(),
            text: text.clone(),
            thread: thread.clone().unwrap_or_default(),
            thread_reply: thread.is_some(),
            attachments: files.iter().map(|f| chat::ChatAttachment { name: f.name.clone(), mime: f.mime.clone(), ..Default::default() }).collect(),
            ..Default::default()
        };
        self.msgs.entry(space.clone()).or_default().push(preview);
        self.scroll_bottom = true;
        self.retry_data.insert(
            local.clone(),
            (space.clone(), text.clone(), thread.clone(), files.iter().map(|f| PendingFile { name: f.name.clone(), mime: f.mime.clone(), data: f.data.clone() }).collect()),
        );
        self.dispatch(acc, local, space, text, thread, files);
    }

    fn dispatch(&mut self, acc: &Account, local: String, space: String, text: String, thread: Option<String>, files: Vec<PendingFile>) {
        acc.run(move |s| {
            let result = (|| {
                let mut atts = Vec::new();
                for f in &files {
                    atts.push(chat::upload(s, &space, &f.name, &f.mime, &f.data)?);
                }
                chat::send(s, &space, &text, thread.as_deref(), atts)
            })();
            Ok(Event::Chat(match result {
                Ok(msg) => ChatEvent::Sent { space, msg, local },
                Err(error) => ChatEvent::SendFailed { space, local, error },
            }))
        });
    }

    /// Renvoie un message dont l'envoi a échoué.
    pub fn retry_send(&mut self, acc: &Account, name: &str) {
        let local = name.replacen("failed/", "local/", 1);
        let Some((space, text, thread, files)) = self.retry_data.get(&local).map(|(a, b, c, d)| {
            (a.clone(), b.clone(), c.clone(), d.iter().map(|f| PendingFile { name: f.name.clone(), mime: f.mime.clone(), data: f.data.clone() }).collect::<Vec<_>>())
        }) else {
            return;
        };
        if let Some(m) = self.msgs.get_mut(&space).and_then(|l| l.iter_mut().find(|m| m.name == name)) {
            m.name = local.clone();
        }
        self.dispatch(acc, local, space, text, thread, files);
    }

    /// Abandonne un message non envoyé.
    pub fn discard_failed(&mut self, name: &str) {
        for l in self.msgs.values_mut() {
            l.retain(|m| m.name != name);
        }
        self.retry_data.remove(&name.replacen("failed/", "local/", 1));
    }

    pub fn save_edit(&mut self, acc: &Account) {
        let Some((name, text)) = self.editing.take() else { return };
        // Mise à jour immédiate à l'écran.
        for l in self.msgs.values_mut() {
            if let Some(m) = l.iter_mut().find(|m| m.name == name) {
                m.text = text.clone();
                m.edited = true;
            }
        }
        run(acc, move |s| chat::edit(s, &name, &text).map(ChatEvent::Updated));
    }

    pub fn delete(&mut self, acc: &Account, m: &ChatMessage) {
        self.pins.remove(&m.name);
        let (space, name) = (m.space.clone(), m.name.clone());
        run(acc, move |s| {
            chat::delete(s, &name)?;
            Ok(ChatEvent::Deleted { space, name })
        });
    }

    /// Réaction « optimiste » : le compteur change tout de suite, Google est prévenu ensuite
    /// (un seul appel pour ajouter) ; en cas d'écart, le message est relu.
    /// Épingle ou désépingle un message (il apparaît alors dans le tableau de bord).
    pub fn toggle_pin(&mut self, m: &ChatMessage) {
        let sender = self.name_of(&m.sender);
        let title = self.space(&m.space).map(|s| self.title(s)).unwrap_or_default();
        if self.pins.toggle(m, sender, title) {
            self.info(t!("Message épinglé au tableau de bord", "Message pinned to the dashboard"));
        } else {
            self.info(t!("Message désépinglé", "Message unpinned"));
        }
    }

    pub fn react(&mut self, acc: &Account, m: &ChatMessage, emoji: &str) {
        if m.name.contains("local/") || m.name.starts_with("failed/") {
            return;
        }
        let key = (m.name.clone(), emoji.to_string());
        let add = !self.my_reactions.contains(&key);
        if let Some(x) = self.msgs.get_mut(&m.space).and_then(|l| l.iter_mut().find(|x| x.name == m.name)) {
            match x.reactions.iter_mut().position(|(e, _)| e == emoji) {
                Some(i) if add => x.reactions[i].1 += 1,
                Some(i) => {
                    x.reactions[i].1 = x.reactions[i].1.saturating_sub(1);
                    if x.reactions[i].1 == 0 {
                        x.reactions.remove(i);
                    }
                }
                None if add => x.reactions.push((emoji.to_string(), 1)),
                None => {}
            }
        }
        if add {
            self.my_reactions.insert(key.clone());
        } else {
            self.my_reactions.remove(&key);
        }
        let (name, emoji, me) = (m.name.clone(), emoji.to_string(), self.me.clone());
        acc.run(move |s| {
            Ok(Event::Chat(match chat::set_reaction(s, &name, &emoji, add, me.as_deref()) {
                Ok(now) if now == add => ChatEvent::Noop,
                // Écart (réaction déjà mise ailleurs) ou erreur : on relit le message.
                _ => chat::get_message(s, &name).map(ChatEvent::Reacted).unwrap_or(ChatEvent::Noop),
            }))
        });
    }

    pub fn open_dm(&mut self, acc: &Account, email: &str) {
        let email = email.trim().to_string();
        if email.is_empty() {
            return;
        }
        if acc.session().is_none() {
            self.pending_dm = Some(email);
            return;
        }
        self.info(t!("Ouverture de la conversation…", "Opening the conversation…"));
        run(acc, move |s| chat::open_dm(s, &email).map(ChatEvent::Dm));
    }

    /// Demandé depuis un autre onglet (ex. fiche contact de l'onglet Mail).
    pub fn request_dm(&mut self, email: String) {
        self.pending_dm = Some(email);
    }

    /// Tendances ou recherche GIPHY pour le sélecteur de GIF.
    pub fn search_gifs(&mut self, acc: &Account, query: String) {
        let (agent, key) = (acc.agent(), self.giphy_key.clone());
        acc.spawn(move || {
            let result = gif_picker::fetch(&agent, &key, &query);
            Ok(Event::Chat(ChatEvent::GifResults { query, result }))
        });
    }

    /// Envoie un GIF du sélecteur, en message à part (comme Discord) : affiché aussitôt, puis
    /// téléchargé et envoyé en pièce jointe (Google Chat n'accepte pas de lien GIF par l'API).
    pub fn send_gif(&mut self, acc: &Account, g: &crate::prefs::GifItem, in_thread: bool) {
        let Some(space) = self.open.clone() else { return };
        let thread = if in_thread { self.thread.clone() } else { None };
        self.local_seq += 1;
        let local = format!("local/{}", self.local_seq);
        let preview = ChatMessage {
            name: local.clone(),
            space: space.clone(),
            sender: self.me.clone().unwrap_or_else(|| ME_LOCAL.into()),
            created: chrono::Utc::now().timestamp_millis(),
            thread: thread.clone().unwrap_or_default(),
            thread_reply: thread.is_some(),
            attachments: vec![chat::ChatAttachment { name: "GIF".into(), mime: "image/gif".into(), download_uri: Some(g.url.clone()), ..Default::default() }],
            ..Default::default()
        };
        self.msgs.entry(space.clone()).or_default().push(preview);
        self.scroll_bottom = true;
        let (agent, url) = (acc.agent(), g.url.clone());
        let name = format!("{}.gif", if g.id.is_empty() { "giphy" } else { g.id.as_str() });
        acc.run(move |s| {
            let result = (|| {
                let data = gif::fetch(&agent, &url)?;
                let att = chat::upload(s, &space, &name, "image/gif", &data)?;
                chat::send(s, &space, "", thread.as_deref(), vec![att])
            })();
            Ok(Event::Chat(match result {
                Ok(msg) => ChatEvent::Sent { space, msg, local },
                Err(error) => ChatEvent::SendFailed { space, local, error },
            }))
        });
    }

    /// Charge un GIF (lien public ou fichier envoyé dans Chat) et toutes ses images.
    pub fn request_gif(&mut self, acc: &Account, att: &chat::ChatAttachment) {
        let key = att.key();
        if !self.gifs.start(&key) {
            return;
        }
        if let Some(res) = att.resource.clone() {
            acc.run(move |s| {
                let frames = chat::media(s, &res).and_then(|d| gif::decode(&d));
                Ok(Event::Chat(ChatEvent::Gif { key, frames }))
            });
        } else if let Some(url) = att.download_uri.clone() {
            let agent = acc.agent();
            acc.spawn(move || {
                let frames = gif::fetch(&agent, &url).and_then(|d| gif::decode(&d));
                Ok(Event::Chat(ChatEvent::Gif { key, frames }))
            });
        }
    }

    pub fn request_image(&mut self, acc: &Account, att: &chat::ChatAttachment) {
        let Some(res) = att.resource.clone() else { return };
        let key = att.key();
        if !self.images.start(&key) {
            return;
        }
        acc.run(move |s| {
            let image = chat::media(s, &res).and_then(|d| inline::decode(&d));
            Ok(Event::Chat(ChatEvent::Image { key, image }))
        });
    }

    /// Ouvre une pièce jointe : fichier Drive dans le navigateur, sinon téléchargement
    /// puis application par défaut ; lien Google Chat en dernier recours.
    pub fn open_attachment(&mut self, acc: &Account, att: &chat::ChatAttachment) {
        if let Some(id) = &att.drive_id {
            let _ = webbrowser::open(&format!("https://drive.google.com/open?id={id}"));
            return;
        }
        let Some(res) = att.resource.clone() else {
            if let Some(u) = &att.download_uri {
                let _ = webbrowser::open(u);
            }
            return;
        };
        self.info(&tf!("Téléchargement de « {} »…", "Downloading “{}”…", att.name));
        let name = sanitize(&att.name);
        let fallback = att.download_uri.clone();
        run(acc, move |s| match chat::media(s, &res) {
            Ok(data) => {
                let dir = std::env::temp_dir().join("WorkTable").join("chat");
                std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
                let path = dir.join(name);
                std::fs::write(&path, data).map_err(|e| e.to_string())?;
                Ok(ChatEvent::File { path })
            }
            Err(e) => {
                if let Some(u) = fallback {
                    let _ = webbrowser::open(&u);
                }
                Err(e)
            }
        });
    }

    // --- Noms, membres, état de lecture ----------------------------------------------

    fn request_read_states(&mut self, acc: &Account) {
        let todo: Vec<String> = self.spaces.iter().take(RECENT).filter(|s| !self.read_requested.contains(&s.name)).map(|s| s.name.clone()).collect();
        if todo.is_empty() {
            return;
        }
        self.read_requested.extend(todo.iter().cloned());
        run(acc, move |s| chat::read_states(s, &todo).map(ChatEvent::ReadStates));
    }

    /// Membres des conversations sans nom (messages privés, groupes sans titre) : toutes, pas
    /// seulement les plus récentes, par lots (la suite est demandée à la passe suivante).
    fn request_members(&mut self, acc: &Account) {
        let todo: Vec<String> = self
            .spaces
            .iter()
            .filter(|s| s.kind != SpaceKind::Space && s.display_name.is_empty())
            .filter(|s| !self.members.contains_key(&s.name) && !self.members_pending.contains(&s.name))
            .filter(|s| retry_due(self.members_retry.get(&s.name)))
            .take(AUDIT_MEMBERS)
            .map(|s| s.name.clone())
            .collect();
        if todo.is_empty() {
            return;
        }
        self.members_pending.extend(todo.iter().cloned());
        run(acc, move |s| chat::members(s, &todo).map(ChatEvent::Members));
    }

    fn request_people(&mut self, acc: &Account, ids: Vec<String>) {
        let ids: Vec<String> = ids.into_iter().filter(|id| !self.people.contains_key(id)).collect();
        self.fetch_people(acc, ids);
    }

    /// Lit des personnes dans l'annuaire (même déjà connues : nouvel essai pour les noms manquants).
    fn fetch_people(&mut self, acc: &Account, ids: Vec<String>) {
        let mut todo: Vec<String> = Vec::new();
        for id in ids {
            if id.starts_with("users/") && !self.people_pending.contains(&id) && !todo.contains(&id) {
                todo.push(id);
            }
        }
        if todo.is_empty() {
            return;
        }
        self.people_pending.extend(todo.iter().cloned());
        let ids = todo.clone();
        acc.run(move |s| {
            // Annuaire inaccessible (erreur, quota) : rien n'est enregistré, nouvel essai plus tard.
            Ok(Event::Chat(match chat::people(s, &ids) {
                Ok(list) => ChatEvent::People(list),
                Err(_) => ChatEvent::PeopleFailed(ids),
            }))
        });
    }

    /// Vérification en arrière-plan : chaque conversation doit avoir un vrai nom (celui de la
    /// personne ou des membres). Celles qui n'en ont pas voient leurs membres puis leurs
    /// personnes relus, par lots, avec de nouveaux essais espacés en cas d'échec.
    fn audit_names(&mut self, acc: &Account) {
        if self.last_audit.is_some_and(|t| t.elapsed() < AUDIT_EVERY) {
            return;
        }
        self.last_audit = Some(Instant::now());
        // 1. Membres manquants (ou conversation privée sans l'autre personne).
        let stale: Vec<String> = self
            .spaces
            .iter()
            .filter(|s| s.kind != SpaceKind::Space && s.display_name.is_empty() && !s.bot_dm)
            .filter(|s| self.members.get(&s.name).is_some_and(|m| m.iter().all(|u| Some(u.as_str()) == self.me.as_deref())))
            .filter(|s| retry_due(self.members_retry.get(&s.name)))
            .map(|s| s.name.clone())
            .collect();
        for sp in stale {
            self.members.remove(&sp);
        }
        self.request_members(acc);
        // 2. Personnes sans nom ni adresse dans les conversations sans nom.
        let mut todo: Vec<String> = Vec::new();
        for s in self.spaces.iter().filter(|s| s.kind != SpaceKind::Space && s.display_name.is_empty() && !s.bot_dm) {
            for u in self.others(&s.name) {
                let nameless = self.people.get(&u).is_none_or(|p| p.name.is_empty() && p.email.is_empty());
                if nameless && retry_due(self.people_retry.get(&u)) && !todo.contains(&u) {
                    todo.push(u);
                }
            }
            if todo.len() >= AUDIT_PEOPLE {
                break;
            }
        }
        for id in &todo {
            let n = self.people_retry.get(id).map(|r| r.1).unwrap_or(0);
            self.people_retry.insert(id.clone(), (Instant::now(), n));
        }
        self.fetch_people(acc, todo);
    }

    /// Nom réel connu : celui de la conversation, ou celui (ou l'adresse) de chacun des autres membres.
    fn has_real_name(&self, sp: &Space) -> bool {
        if !sp.display_name.is_empty() {
            return true;
        }
        let others = self.others(&sp.name);
        !others.is_empty() && others.iter().all(|u| self.people.get(u).is_some_and(|p| !p.name.is_empty() || !p.email.is_empty()))
    }

    /// Conversations encore sans vrai nom dont la vérification est en cours (premiers essais) ;
    /// celles qui restent introuvables ne sont plus comptées (nouvel essai toutes les 3 h).
    pub fn unnamed_count(&self) -> usize {
        let active = |r: Option<&(Instant, u32)>| r.is_none_or(|(_, n)| *n < 4);
        self.spaces
            .iter()
            .filter(|s| s.kind != SpaceKind::Space && s.display_name.is_empty() && !s.bot_dm && !self.has_real_name(s))
            .filter(|s| {
                if !self.members.contains_key(&s.name) {
                    return active(self.members_retry.get(&s.name));
                }
                self.others(&s.name).iter().any(|u| active(self.people_retry.get(u)))
            })
            .count()
    }

    pub fn name_of(&self, user: &str) -> String {
        if self.me.as_deref() == Some(user) || user == ME_LOCAL {
            return t!("Vous", "You").into();
        }
        match self.people.get(user) {
            Some(p) if !p.name.is_empty() => p.name.clone(),
            Some(p) if !p.email.is_empty() => p.email.clone(),
            _ => t!("Utilisateur", "User").into(),
        }
    }

    pub fn email_of(&self, user: &str) -> Option<String> {
        self.people.get(user).map(|p| p.email.clone()).filter(|e| !e.is_empty())
    }

    /// Autres membres d'une conversation (sans soi-même).
    pub fn others(&self, space: &str) -> Vec<String> {
        self.members.get(space).map(|m| m.iter().filter(|u| Some(u.as_str()) != self.me.as_deref()).cloned().collect()).unwrap_or_default()
    }

    pub fn title(&self, sp: &Space) -> String {
        if let Some(t) = self.known_title(sp) {
            return t;
        }
        match sp.kind {
            SpaceKind::Dm if sp.bot_dm => t!("Application", "App").into(),
            SpaceKind::Dm => t!("Message privé", "Direct message").into(),
            _ => t!("Discussion de groupe", "Group chat").into(),
        }
    }

    /// Vrai nom d'une conversation (sans libellé générique) : son nom, sinon les noms de ses
    /// membres, sinon celui gardé dans la copie locale (en attendant que membres et personnes
    /// soient relus).
    fn known_title(&self, sp: &Space) -> Option<String> {
        if !sp.display_name.is_empty() {
            return Some(sp.display_name.clone());
        }
        let others = self.others(&sp.name);
        let names = || others.iter().map(|u| self.name_of(u)).collect::<Vec<_>>().join(", ");
        let all_known = others.iter().all(|u| self.people.get(u).is_some_and(|p| !p.name.is_empty() || !p.email.is_empty()));
        if !others.is_empty() && all_known {
            return Some(names());
        }
        self.titles.get(&sp.name).cloned().or_else(|| (!others.is_empty()).then(names))
    }

    pub fn unread(&self, sp: &Space) -> bool {
        if self.open.as_deref() == Some(sp.name.as_str()) {
            return false;
        }
        self.last_read.get(&sp.name).is_some_and(|t| sp.last_active > *t + 1000)
    }

    /// Identifiant Chat de l'utilisateur quand Google ne le donne pas directement :
    /// celui dont l'adresse est celle du compte, sinon le seul membre présent dans tous les messages privés.
    fn infer_me(&mut self) {
        if self.me.is_some() {
            return;
        }
        if !self.own_email.is_empty()
            && let Some(p) = self.people.values().find(|p| p.email.eq_ignore_ascii_case(&self.own_email))
        {
            self.me = Some(p.id.clone());
            return;
        }
        let dms: Vec<&Vec<String>> = self.spaces.iter().filter(|s| s.kind == SpaceKind::Dm).filter_map(|s| self.members.get(&s.name)).filter(|m| m.len() == 2).collect();
        if dms.len() >= 2 {
            let common: Vec<&String> = dms[0].iter().filter(|u| dms.iter().all(|m| m.contains(u))).collect();
            if common.len() == 1 {
                self.me = Some(common[0].clone());
            }
        }
    }

    /// Dernières personnes avec qui l'utilisateur a échangé (messages privés, du plus récent).
    pub fn recent_people(&self) -> Vec<&Space> {
        let mut dms: Vec<&Space> = self.spaces.iter().filter(|s| s.kind == SpaceKind::Dm && !s.bot_dm && s.last_active > 0).collect();
        dms.sort_by_key(|s| -s.last_active);
        dms.truncate(RECENT_PEOPLE);
        dms
    }

    pub fn is_muted(&self, sp: &str) -> bool {
        self.muted.iter().any(|m| m == sp)
    }

    /// Non lus hors conversations muettes (pastilles, compteurs).
    pub fn unread_count(&self) -> usize {
        self.spaces.iter().filter(|s| self.unread(s) && !self.is_muted(&s.name)).count()
    }

    pub fn space(&self, name: &str) -> Option<&Space> {
        self.spaces.iter().find(|s| s.name == name)
    }

    pub fn info(&mut self, t: &str) {
        self.notice = Some((t.to_string(), false, Instant::now()));
    }

    fn error(&mut self, t: String) {
        self.notice = Some((t, true, Instant::now()));
    }

    // --- Photos et images de groupes ------------------------------------------------

    /// Clé d'image d'un utilisateur (photo de profil), si elle existe.
    pub fn photo_key(&self, user: &str) -> Option<String> {
        self.people.get(user).and_then(|p| p.photo.as_ref()).map(|_| format!("photo:{user}"))
    }

    /// Clé d'image d'une conversation : image choisie, sinon photo de l'interlocuteur (message privé).
    pub fn space_image_key(&self, sp: &Space) -> Option<String> {
        if self.icons.contains_key(&sp.name) {
            return Some(format!("icon:{}", sp.name));
        }
        if sp.kind == SpaceKind::Dm {
            return self.others(&sp.name).first().and_then(|u| self.photo_key(u));
        }
        None
    }

    fn request_photos(&mut self, acc: &Account) {
        if self.photos_in_flight >= PHOTOS_IN_FLIGHT {
            return;
        }
        let todo: Vec<(String, String)> = self
            .people
            .values()
            .filter_map(|p| p.photo.as_ref().map(|u| (format!("photo:{}", p.id), u.clone())))
            .filter(|(k, _)| self.images.peek(k).is_none())
            .take(PHOTOS_IN_FLIGHT - self.photos_in_flight)
            .collect();
        for (key, url) in todo {
            self.images.start(&key);
            self.photos_in_flight += 1;
            let agent = acc.agent();
            acc.spawn(move || {
                let image = chat::fetch_photo(&agent, &url).and_then(|d| inline::decode(&d));
                Ok(Event::Chat(ChatEvent::Image { key, image }))
            });
        }
    }

    fn load_icons(&mut self, acc: &Account) {
        self.icons_loaded = true;
        let Some(dir) = icons_dir() else { return };
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        for e in rd.flatten() {
            let path = e.path();
            let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else { continue };
            let space = format!("spaces/{stem}");
            self.icons.insert(space.clone(), path.clone());
            self.load_icon(acc, &space, path);
        }
    }

    fn load_icon(&mut self, acc: &Account, space: &str, path: PathBuf) {
        let key = format!("icon:{space}");
        self.images.forget(&key);
        self.images.start(&key);
        acc.spawn(move || {
            let image = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|d| inline::decode(&d));
            Ok(Event::Chat(ChatEvent::Image { key, image }))
        });
    }

    /// Image choisie pour un groupe : copiée dans le dossier de configuration.
    pub fn set_icon(&mut self, acc: &Account, space: &str, src: &std::path::Path) {
        let Some(dir) = icons_dir() else { return };
        let _ = std::fs::create_dir_all(&dir);
        self.clear_icon(space);
        let id = space.trim_start_matches("spaces/");
        let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
        let dst = dir.join(format!("{id}.{ext}"));
        match std::fs::copy(src, &dst) {
            Ok(_) => {
                self.icons.insert(space.to_string(), dst.clone());
                self.load_icon(acc, space, dst);
            }
            Err(e) => self.error(tf!("Image impossible à copier : {e}", "Could not copy the image: {e}")),
        }
    }

    pub fn clear_icon(&mut self, space: &str) {
        if let Some(p) = self.icons.remove(space) {
            let _ = std::fs::remove_file(p);
        }
        self.images.forget(&format!("icon:{space}"));
    }

    // --- Cache des noms -------------------------------------------------------------

    /// Ancien fichier des personnes en clair (versions précédentes) : repris une fois, puis
    /// supprimé dès que la copie chiffrée est écrite.
    fn load_people(&mut self) {
        self.people_loaded = true;
        let Some(p) = people_path() else { return };
        let Ok(text) = std::fs::read_to_string(p) else { return };
        let list: Vec<store::PersonRow> = serde_json::from_str(&text).unwrap_or_default();
        for (id, name, email, photo) in list {
            self.people.entry(id.clone()).or_insert(Person { id, name, email, photo });
        }
        self.legacy_people = true;
        self.store_dirty = true;
    }

    /// Les personnes sont gardées dans la copie chiffrée (plus de fichier en clair).
    fn save_people(&mut self) {
        self.store_dirty = true;
    }
}

/// Expéditeur provisoire d'un message en cours d'envoi tant que l'identifiant est inconnu.
pub const ME_LOCAL: &str = "users/local-me";

pub fn draft_key(space: &str, thread: Option<&str>) -> String {
    thread.map(str::to_string).unwrap_or_else(|| space.to_string())
}

fn icons_dir() -> Option<PathBuf> {
    Some(crate::paths::config().join("chat_icons"))
}

fn people_path() -> Option<PathBuf> {
    Some(crate::paths::config().join("chat_people.json"))
}

fn sanitize(name: &str) -> String {
    let s: String = name.chars().map(|c| if r#"<>:"/\|?*"#.contains(c) || c.is_control() { '_' } else { c }).collect();
    if s.trim().is_empty() { "fichier".into() } else { s }
}

/// Tâche Chat en arrière-plan : les erreurs restent dans l'onglet Messages.
/// Message clair pour un refus de Google lors de la gestion d'un groupe.
fn manage_error(e: &str) -> String {
    let low = e.to_lowercase();
    if low.contains("scope") || low.contains("insufficient") {
        t!(
            "Autorisation manquante : reconnecte ton compte Google (Réglages › Google) pour autoriser la gestion des groupes.",
            "Missing permission: sign in to Google again (Settings › Google) to allow managing groups."
        )
        .to_string()
    } else if low.contains("permission") || low.contains("403") || low.contains("forbidden") {
        t!("Refusé par Google : seul un gestionnaire du groupe peut faire ça.", "Refused by Google: only a group manager can do this.").to_string()
    } else {
        tf!("Refusé par Google : {e}", "Refused by Google: {e}")
    }
}

fn run(acc: &Account, f: impl FnOnce(&crate::google::Session) -> Result<ChatEvent, String> + Send + 'static) {
    acc.run(move |s| Ok(Event::Chat(f(s).unwrap_or_else(ChatEvent::Error))));
}

/// Fusionne des messages dans la liste triée. `replace_from` : tout ce qui est créé à partir
/// de cette date est remplacé par le lot (messages supprimés compris). Renvoie `true` si un
/// nouveau message est apparu.
fn merge(list: &mut Vec<ChatMessage>, batch: Vec<ChatMessage>, replace_from: Option<i64>) -> bool {
    let before: HashSet<String> = list.iter().map(|m| m.name.clone()).collect();
    let pending = |m: &ChatMessage| m.name.starts_with("local/") || m.name.starts_with("failed/");
    if let Some(from) = replace_from {
        // Les messages en cours d'envoi restent affichés.
        list.retain(|m| m.created < from || pending(m));
    }
    let mut added = false;
    for m in batch {
        // Le vrai message est arrivé par la relève avant la confirmation : l'aperçu disparaît.
        if let Some(i) = list.iter().position(|x| x.name.starts_with("local/") && x.text == m.text && x.thread == m.thread) {
            list.remove(i);
        }
        added |= !before.contains(&m.name);
        match list.iter_mut().find(|x| x.name == m.name) {
            Some(x) => *x = m,
            None => list.push(m),
        }
    }
    list.sort_by_key(|m| m.created);
    added
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str, t: i64) -> ChatMessage {
        ChatMessage { name: name.into(), created: t, ..Default::default() }
    }

    #[test]
    fn retries_are_spaced_out() {
        assert!(retry_due(None), "premier essai tout de suite");
        let now = Instant::now();
        assert!(!retry_due(Some(&(now, 1))), "pas de nouvel essai immédiat");
        let old = now.checked_sub(Duration::from_secs(120)).unwrap_or(now);
        assert!(retry_due(Some(&(old, 1))), "après 1 min");
        assert!(!retry_due(Some(&(old, 2))), "5 min au 2e échec");
    }

    #[test]
    fn unnamed_dm_is_counted_until_resolved() {
        let mut c = ChatState { me: Some("users/me".into()), ..Default::default() };
        c.spaces.push(Space { name: "spaces/dm".into(), display_name: String::new(), kind: SpaceKind::Dm, threaded: false, last_active: 1, members: 2, bot_dm: false });
        assert_eq!(c.unnamed_count(), 1, "membres inconnus");
        assert_eq!(c.title(&c.spaces[0].clone()), "Message privé");
        c.members.insert("spaces/dm".into(), vec!["users/me".into(), "users/7".into()]);
        c.people.insert("users/7".into(), Person { id: "users/7".into(), name: String::new(), email: String::new(), photo: None });
        assert_eq!(c.unnamed_count(), 1, "personne sans nom");
        c.people.get_mut("users/7").unwrap().name = "Léa Martin".into();
        assert_eq!(c.unnamed_count(), 0);
        assert_eq!(c.title(&c.spaces[0].clone()), "Léa Martin");
    }

    #[test]
    fn merge_replaces_recent_window() {
        let mut l = vec![m("a", 1), m("b", 2), m("c", 3)];
        // « c » a été supprimé, « d » est nouveau.
        assert!(merge(&mut l, vec![m("b", 2), m("d", 4)], Some(2)));
        let names: Vec<&str> = l.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["a", "b", "d"]);
        assert!(!merge(&mut l, vec![m("b", 2)], None));
    }
}
