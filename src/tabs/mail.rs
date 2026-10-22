//! Onglet Mail : état, réactions aux réponses de Gmail et actions.

pub mod attachments;
pub mod composer;
pub mod contacts;
pub mod directory;
pub mod groups;
pub mod inline;
pub mod pinned;
pub mod recipients;
pub(crate) mod view;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::google::compose::Draft;
use crate::google::{Account, Attachment, Event, FullMessage, Label, Summary, gmail};

pub use view::show;

pub const INBOX: &str = "INBOX";
const STARRED: &str = "STARRED";
const UNREAD: &str = "UNREAD";
/// Nombre de messages chargés par dossier.
const PAGE: usize = 40;
/// Vérification automatique des nouveaux mails.
const SYNC_EVERY: Duration = Duration::from_secs(60);

/// Élément d'un mail vers lequel une pastille du haut fait défiler.
#[derive(Clone, Debug, PartialEq)]
pub enum DocTarget {
    /// Pièce jointe (clé `Attachment::key`).
    Attachment(String),
    /// Paragraphe du corps contenant un lien de téléchargement.
    Block(usize),
}

pub struct Notice {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

/// Fichiers reçus par mail montrés au tableau de bord.
pub const SHARED_FILES: usize = 13;

pub struct MailState {
    pub labels: Vec<Label>,
    pub folder: String,
    pub messages: Vec<Summary>,
    pub loading: bool,
    pub starred: Vec<Summary>,
    /// Mails épinglés en haut de la boîte de réception (3 max).
    pub pinned: pinned::Pinned,
    pub open: Option<FullMessage>,
    /// Autres messages de la conversation ouverte (du plus ancien au plus récent).
    pub thread: Vec<FullMessage>,
    /// Messages de la conversation dépliés par l'utilisateur.
    pub thread_expanded: HashSet<String>,
    /// Nombre de messages par conversation (identifiant de fil → nombre).
    pub thread_sizes: HashMap<String, usize>,
    sizes_pending: HashSet<String>,
    /// Conversation à recharger quand la réponse est partie.
    sent_thread: Option<String>,
    /// Demande d'ouverture d'une conversation Google Chat avec cette adresse.
    pub chat_to: Option<String>,
    /// Non lus de la boîte de réception (pastille de la barre de navigation).
    pub inbox_unread: usize,
    /// Derniers fichiers reçus par mail (tableau de bord) et date de la dernière relève.
    pub shared_files: Vec<(Summary, gmail::SharedFile)>,
    shared_at: Option<Instant>,
    pub opening: Option<String>,
    pub draft: Option<Draft>,
    pub sending: bool,
    pub new_category: Option<String>,
    pub notice: Option<Notice>,
    pub attachments: attachments::Attachments,
    pub inline: inline::InlineImages,
    pub editor: composer::Editor,
    /// Annuaire local (contacts.json).
    pub contacts: contacts::Contacts,
    /// Vue « Annuaire » affichée à la place de la liste / lecture.
    pub directory: bool,
    pub dir_query: String,
    pub dir_selected: Option<String>,
    /// Mails du contact sélectionné dans l'annuaire : (adresse, mails), et chargement en cours.
    pub dir_mails: Option<(String, Vec<Summary>)>,
    pub dir_loading: Option<String>,
    /// Défilement demandé vers un document, puis surbrillance temporaire.
    pub jump: Option<DocTarget>,
    pub flash: Option<(DocTarget, Instant)>,
    /// Couleurs des adresses de groupe (copie des préférences pour l'affichage de la frame),
    /// modifications demandées, adresse du compte, saisie d'un nouveau groupe.
    pub groups: Vec<crate::prefs::GroupColor>,
    pub group_edits: Vec<groups::GroupEdit>,
    pub own: String,
    pub new_group: Option<String>,
    pub groups_open: bool,
    pub toggle_groups: bool,
    /// Signatures Gmail disponibles (la première est celle par défaut).
    pub signatures: Vec<gmail::Signature>,
    /// Destinataires du message en cours d'envoi (ajoutés à l'annuaire une fois envoyé).
    sending_to: Vec<String>,
    last_sync: Option<Instant>,
    polling: bool,
}

impl Default for MailState {
    fn default() -> Self {
        Self {
            labels: Vec::new(),
            folder: INBOX.into(),
            messages: Vec::new(),
            loading: false,
            starred: Vec::new(),
            pinned: pinned::Pinned::load(),
            open: None,
            thread: Vec::new(),
            thread_expanded: HashSet::new(),
            thread_sizes: HashMap::new(),
            sizes_pending: HashSet::new(),
            sent_thread: None,
            chat_to: None,
            inbox_unread: 0,
            shared_files: Vec::new(),
            shared_at: None,
            opening: None,
            draft: None,
            sending: false,
            new_category: None,
            notice: None,
            attachments: Default::default(),
            inline: Default::default(),
            editor: Default::default(),
            contacts: contacts::Contacts::load(),
            directory: false,
            dir_query: String::new(),
            dir_selected: None,
            dir_mails: None,
            dir_loading: None,
            jump: None,
            flash: None,
            groups: Vec::new(),
            group_edits: Vec::new(),
            own: String::new(),
            new_group: None,
            groups_open: true,
            toggle_groups: false,
            signatures: Vec::new(),
            sending_to: Vec::new(),
            last_sync: None,
            polling: false,
        }
    }
}

impl MailState {
    // --- Réception des résultats ------------------------------------------

    pub fn handle(&mut self, ev: Event, acc: &Account) {
        match ev {
            Event::Ready => {
                let contacts = std::mem::take(&mut self.contacts);
                let pinned = std::mem::take(&mut self.pinned);
                *self = MailState::default();
                self.contacts = contacts;
                self.pinned = pinned;
                self.full_load(acc);
                acc.run(|s| Ok(Event::Labels(gmail::labels(s)?)));
                // Signature : sans bloquer le reste si les réglages ne sont pas accessibles.
                acc.run(|s| Ok(Event::Signatures(gmail::signatures(s).unwrap_or_default())));
            }
            Event::Delta { changed, deleted } => {
                self.polling = false;
                self.ingest(acc, &changed);
                for m in &changed {
                    // Nouveau message dans un fil : son compteur sera relu.
                    if !self.sizes_pending.contains(&m.thread_id) {
                        self.thread_sizes.remove(&m.thread_id);
                    }
                }
                for m in changed {
                    self.apply_summary(m);
                }
                for id in deleted {
                    self.messages.retain(|m| m.id != id);
                    self.starred.retain(|m| m.id != id);
                    self.pinned.remove(&id);
                }
            }
            Event::Resync => {
                self.polling = false;
                self.full_load(acc);
            }
            Event::Labels(l) => self.labels = l,
            Event::Messages { label, list } => {
                self.ingest(acc, &list);
                if let Some(addr) = label.strip_prefix("from:")
                    && (self.dir_loading.as_deref() == Some(addr) || self.dir_selected.as_deref() == Some(addr))
                {
                    self.dir_loading = None;
                    self.dir_mails = Some((addr.to_string(), list.clone()));
                }
                if label == self.folder {
                    self.messages = list;
                    self.loading = false;
                }
            }
            Event::Starred(list) => {
                self.ingest(acc, &list);
                self.starred = list;
            }
            Event::Opened(m) => {
                if self.opening.as_deref() == Some(m.summary.id.as_str()) {
                    self.thread.clear();
                    self.set_open(acc, m);
                }
            }
            Event::Thread { open_id, mut messages } => {
                if let Some(first) = messages.first() {
                    self.thread_sizes.insert(first.summary.thread_id.clone(), messages.len());
                }
                for m in &messages {
                    self.inline.request(acc, m);
                }
                let opening = self.opening.as_deref() == Some(open_id.as_str());
                let refresh = !opening && self.open.as_ref().is_some_and(|o| o.summary.id == open_id);
                if opening || refresh {
                    let pos = messages.iter().position(|m| m.summary.id == open_id).unwrap_or(messages.len().saturating_sub(1));
                    if pos < messages.len() {
                        if opening {
                            // Autres messages non lus de la conversation : lus aussi.
                            let unread: Vec<String> = messages.iter().filter(|x| x.summary.unread() && x.summary.id != open_id).map(|x| x.summary.id.clone()).collect();
                            for id in unread {
                                self.modify(acc, &id, &[], &[UNREAD]);
                            }
                            for x in messages.iter_mut() {
                                x.summary.labels.retain(|l| l != UNREAD);
                            }
                        }
                        let m = messages.remove(pos);
                        self.thread = messages;
                        if opening {
                            self.thread_expanded.clear();
                            self.set_open(acc, m);
                        } else if let Some(o) = &mut self.open {
                            o.summary.labels = m.summary.labels;
                        }
                    } else if opening {
                        self.opening = None;
                    }
                }
            }
            Event::ThreadSizes(list) => {
                for (id, n) in list {
                    self.sizes_pending.remove(&id);
                    self.thread_sizes.insert(id, n);
                }
            }
            Event::Modified { id, labels } => self.apply_labels(&id, &labels),
            Event::LabelCreated(l) => {
                self.info(format!("Catégorie « {} » créée", l.name));
                self.labels.retain(|x| x.id != l.id);
                self.labels.push(l);
                self.labels.sort_by_key(|l| l.name.to_lowercase());
                // Resynchronise la liste avec Gmail (au cas où).
                acc.run(|s| Ok(Event::Labels(gmail::labels(s)?)));
            }
            Event::Sent => {
                let own = acc.session().map(|s| s.email.clone()).unwrap_or_default();
                let now = chrono::Local::now().timestamp_millis();
                for r in std::mem::take(&mut self.sending_to) {
                    self.contacts.record(&r, now, &format!("sent-{now}"), &own);
                }
                self.contacts.save();
                self.sending = false;
                self.draft = None;
                // Recharge la conversation ouverte : la réponse y apparaît.
                if let Some(t) = self.sent_thread.take() {
                    self.thread_sizes.remove(&t);
                    if let Some(o) = self.open.as_ref().filter(|o| o.summary.thread_id == t) {
                        let open_id = o.summary.id.clone();
                        acc.run(move |s| Ok(Event::Thread { messages: gmail::get_thread(s, &t)?, open_id }));
                    }
                }
                self.info("Message envoyé".into());
                self.poll(acc);
            }
            Event::Error(e) if e.contains("exists or conflicts") => {
                // Nom déjà pris dans Gmail : la catégorie existe, on recharge la liste.
                self.info("Cette catégorie existe déjà dans Gmail".into());
                acc.run(|s| Ok(Event::Labels(gmail::labels(s)?)));
            }
            Event::Error(e) => {
                self.dir_loading = None;
                self.polling = false;
                self.loading = false;
                self.sending = false;
                self.opening = None;
                self.notice = Some(Notice { text: e, error: true, at: Instant::now() });
            }
            Event::AttachmentData { key, data } => {
                let n = self.attachments.on_data(key, data);
                self.show_notice(n);
            }
            Event::InlineImage { key, image } => self.inline.on_loaded(key, image),
            Event::Signatures(list) => self.signatures = list,
            // Événements de l'onglet Calendrier : traités ailleurs.
            Event::Calendars(_) | Event::CalEvents { .. } | Event::CalSaved { .. } | Event::CalDeleted { .. } | Event::CalError(_) | Event::Chat(_) | Event::Connected(_) => {}
            Event::SharedFiles(list) => self.shared_files = list,
            Event::MailSearch { .. } | Event::Drive(_) => {}
        }
    }

    /// À appeler à chaque frame : relève automatiquement la boîte.
    pub fn tick(&mut self, acc: &Account, ctx: &eframe::egui::Context) {
        self.contacts.autosave();
        if !self.loading {
            self.count_unread();
        }
        if acc.session().is_none() {
            return;
        }
        if self.last_sync.is_some_and(|t| t.elapsed() >= SYNC_EVERY) {
            self.poll(acc);
        }
        self.request_sizes(acc);
        ctx.request_repaint_after(SYNC_EVERY);
    }

    // --- Actions ----------------------------------------------------------

    /// Chargement complet (connexion, ou historique Gmail expiré).
    fn full_load(&mut self, acc: &Account) {
        self.last_sync = Some(Instant::now());
        self.loading = true;
        let label = self.folder.clone();
        acc.run(move |s| {
            gmail::mark_history(s)?;
            Ok(Event::Messages { list: gmail::list(s, &label, PAGE)?, label })
        });
        acc.run(|s| Ok(Event::Starred(gmail::list(s, STARRED, 100)?)));
    }

    /// Relève incrémentale : seulement ce qui a changé depuis la dernière fois.
    fn poll(&mut self, acc: &Account) {
        self.last_sync = Some(Instant::now());
        if self.polling {
            return;
        }
        self.polling = true;
        acc.run(|s| {
            Ok(match gmail::history(s)? {
                gmail::HistoryResult::Delta { changed, deleted } => Event::Delta { changed, deleted },
                gmail::HistoryResult::Expired => Event::Resync,
            })
        });
    }

    /// Bouton « Actualiser ».
    pub fn refresh_folder(&mut self, acc: &Account) {
        if self.messages.is_empty() {
            self.load_folder(acc);
        } else {
            self.poll(acc);
        }
    }

    /// Liste du dossier courant (les mails déjà vus viennent du cache).
    fn load_folder(&mut self, acc: &Account) {
        self.loading = true;
        let label = self.folder.clone();
        acc.run(move |s| Ok(Event::Messages { list: gmail::list(s, &label, PAGE)?, label }));
    }

    pub fn select_folder(&mut self, acc: &Account, id: &str) {
        if self.folder != id {
            self.folder = id.to_string();
            self.messages.clear();
            self.load_folder(acc);
        }
    }

    pub fn open_message(&mut self, acc: &Account, id: &str) {
        self.draft = None;
        if self.open.as_ref().is_some_and(|m| m.summary.id == id) {
            return;
        }
        self.opening = Some(id.to_string());
        let id = id.to_string();
        // Conversation entière quand le fil est connu, sinon le message seul.
        let thread = self
            .messages
            .iter()
            .chain(self.starred.iter())
            .chain(self.pinned.list.iter())
            .chain(self.dir_mails.iter().flat_map(|(_, l)| l.iter()))
            .find(|m| m.id == id)
            .map(|m| m.thread_id.clone())
            .filter(|t| !t.is_empty());
        // Lu tout de suite (toute la conversation) : la pastille « non lus » disparaît sans
        // attendre le chargement, d'où qu'on ouvre le mail (liste, tableau de bord, aperçu…).
        self.mark_read(acc, &id, thread.as_deref());
        match thread {
            Some(t) => acc.run(move |s| Ok(Event::Thread { messages: gmail::get_thread(s, &t)?, open_id: id })),
            None => acc.run(move |s| Ok(Event::Opened(gmail::get_full(s, &id)?))),
        }
    }

    /// Marque comme lus un message et les autres messages non lus de sa conversation.
    fn mark_read(&mut self, acc: &Account, id: &str, thread: Option<&str>) {
        let mut ids: Vec<String> = Vec::new();
        for m in self.messages.iter().chain(self.starred.iter()).chain(self.pinned.list.iter()) {
            let same = m.id == id || thread.is_some_and(|t| m.thread_id == t);
            if same && m.unread() && !ids.contains(&m.id) {
                ids.push(m.id.clone());
            }
        }
        for i in ids {
            self.modify(acc, &i, &[], &[UNREAD]);
        }
        self.count_unread();
    }

    /// Non lus de la boîte de réception (une conversation compte pour un).
    fn count_unread(&mut self) {
        if self.folder == INBOX {
            let mut seen = HashSet::new();
            self.inbox_unread = self.messages.iter().filter(|m| m.unread() && !m.labels.iter().any(|l| l == "TRASH") && seen.insert(m.thread_id.clone())).count();
        }
    }

    fn set_open(&mut self, acc: &Account, mut m: FullMessage) {
        self.opening = None;
        if m.summary.unread() {
            self.modify(acc, &m.summary.id, &[], &[UNREAD]);
            // La copie reçue peut dater d'avant le marquage : on ne la laisse pas rétablir « non lu ».
            m.summary.labels.retain(|l| l != UNREAD);
        }
        self.inline.request(acc, &m);
        self.jump = None;
        self.flash = None;
        self.ingest(acc, std::slice::from_ref(&m.summary));
        self.open = Some(m);
    }

    /// Demande le nombre de messages des conversations affichées (une seule requête de fond à la fois).
    fn request_sizes(&mut self, acc: &Account) {
        if !self.sizes_pending.is_empty() {
            return;
        }
        let mut ids: Vec<String> = Vec::new();
        for m in self.messages.iter().chain(self.pinned.list.iter()) {
            if !m.thread_id.is_empty() && !self.thread_sizes.contains_key(&m.thread_id) && !ids.contains(&m.thread_id) {
                ids.push(m.thread_id.clone());
            }
        }
        if ids.is_empty() {
            return;
        }
        self.sizes_pending = ids.iter().cloned().collect();
        acc.run(move |s| Ok(Event::ThreadSizes(gmail::thread_sizes(s, &ids)?)));
    }

    /// Relève des derniers fichiers reçus (au plus toutes les 10 minutes).
    /// Nombre de fichiers reçus affichés au tableau de bord.
    pub fn refresh_shared_files(&mut self, acc: &Account) {
        if acc.session().is_none() || self.shared_at.is_some_and(|t| t.elapsed() < Duration::from_secs(600)) {
            return;
        }
        self.shared_at = Some(Instant::now());
        acc.run(|s| Ok(Event::SharedFiles(gmail::recent_shared(s, SHARED_FILES).unwrap_or_default())));
    }

    /// Ouvre un mail depuis un autre onglet (tableau de bord) : boîte de réception, mail affiché.
    pub fn open_from_elsewhere(&mut self, acc: &Account, id: &str) {
        self.directory = false;
        self.open_message(acc, id);
    }

    /// Ouvre la rédaction d'une réponse à tous.
    pub fn reply_all(&self, own: &str) -> Option<Draft> {
        self.open.as_ref().map(|m| Draft::reply_all(m, own))
    }

    pub fn toggle_star(&mut self, acc: &Account, msg: &Summary) {
        if msg.starred() {
            self.modify(acc, &msg.id, &[], &[STARRED]);
        } else {
            // On l'ajoute tout de suite à la liste des favoris (affichage immédiat).
            let mut m = msg.clone();
            m.labels.push(STARRED.into());
            self.starred.insert(0, m);
            self.modify(acc, &msg.id, &[STARRED], &[]);
        }
    }

    /// Range un message dans une catégorie (le retire de la boîte de réception
    /// ou de la catégorie courante).
    pub fn move_to(&mut self, acc: &Account, id: &str, target: &Label) {
        let current_is_folder = self.folder == INBOX || self.labels.iter().any(|l| l.user && l.id == self.folder);
        let mut remove: Vec<&str> = Vec::new();
        if current_is_folder && self.folder != target.id {
            remove.push(&self.folder);
        }
        let remove: Vec<String> = remove.into_iter().map(str::to_string).collect();
        let remove_ref: Vec<&str> = remove.iter().map(String::as_str).collect();
        self.modify(acc, id, &[target.id.as_str()], &remove_ref);
        self.info(format!("Rangé dans « {} »", target.name));
    }

    /// Met un mail dans les spams (ou l'en sort : retour en boîte de réception).
    /// Gmail apprend de ce signalement pour les prochains mails de l'expéditeur.
    pub fn mark_spam(&mut self, acc: &Account, id: &str, spam: bool) {
        if spam {
            self.modify(acc, id, &["SPAM"], &[INBOX]);
            self.info("Mail déplacé dans les spams".into());
        } else {
            self.modify(acc, id, &[INBOX], &["SPAM"]);
            self.info("Mail replacé dans la boîte de réception".into());
        }
        // Le mail quitte le dossier affiché : on ferme sa lecture.
        if self.open.as_ref().is_some_and(|m| m.summary.id == id) && (spam || self.folder == "SPAM") {
            self.open = None;
        }
    }

    /// Met un mail à la corbeille (ou l'en sort depuis le dossier Corbeille).
    pub fn trash(&mut self, acc: &Account, id: &str, trash: bool) {
        // Toute la conversation (une ligne de la liste = une conversation), comme dans Gmail.
        let thread =
            self.messages.iter().chain(self.starred.iter()).chain(self.open.iter().map(|m| &m.summary)).find(|m| m.id == id).map(|m| m.thread_id.clone()).filter(|t| !t.is_empty());
        let mut ids: Vec<String> = vec![id.to_string()];
        if let Some(t) = &thread {
            for m in self.messages.iter().chain(self.starred.iter()).chain(self.pinned.list.iter()) {
                if &m.thread_id == t && !ids.contains(&m.id) {
                    ids.push(m.id.clone());
                }
            }
        }
        // Mise à jour locale immédiate : la conversation quitte la boîte de réception.
        for i in &ids {
            if let Some(mut labels) = self.labels_of(i) {
                if trash {
                    labels.retain(|l| l != INBOX && l != "SPAM" && l != "TRASH");
                    labels.push("TRASH".into());
                } else {
                    labels.retain(|l| l != "TRASH" && l != INBOX);
                    labels.push(INBOX.into());
                }
                self.apply_labels(i, &labels);
            }
        }
        if trash {
            for i in &ids {
                self.pinned.remove(i);
            }
        }
        self.info(if trash { "Mail mis à la corbeille".into() } else { "Mail replacé dans la boîte de réception".into() });
        if self.open.as_ref().is_some_and(|m| m.summary.id == id) {
            self.open = None;
        }
        self.count_unread();
        let id = id.to_string();
        acc.run(move |s| {
            let labels = match &thread {
                Some(t) => gmail::trash_thread(s, t, trash)?.into_iter().find(|(i, _)| *i == id).map(|(_, l)| l).unwrap_or_default(),
                None => gmail::trash(s, &id, trash)?,
            };
            Ok(Event::Modified { labels, id })
        });
    }

    /// Un message fait-il partie du dossier affiché ? (Ceux de la corbeille n'apparaissent que
    /// dans le dossier Corbeille, même s'ils gardent le libellé « boîte de réception ».)
    fn belongs(&self, labels: &[String]) -> bool {
        labels.contains(&self.folder) && (self.folder == "TRASH" || !labels.iter().any(|l| l == "TRASH"))
    }

    pub fn create_category(&mut self, acc: &Account, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.info(format!("Création de « {name} »…"));
        acc.run(move |s| Ok(Event::LabelCreated(gmail::create_label(s, &name)?)));
    }

    /// Épingle / décroche un mail (3 au maximum).
    pub fn toggle_pin(&mut self, m: &Summary) {
        match self.pinned.toggle(m) {
            Ok(true) => self.info("Mail épinglé en haut de la boîte".into()),
            Ok(false) => self.info("Mail décroché".into()),
            Err(()) => self.info(format!("{} mails épinglés au maximum : décroches-en un d'abord", pinned::MAX)),
        }
    }

    pub fn send(&mut self, acc: &Account) {
        let (Some(draft), Some(session)) = (self.draft.clone(), acc.session()) else { return };
        match draft.to_mime(&session.email) {
            Ok(mime) => {
                self.sending = true;
                self.sending_to = vec![draft.to.clone(), draft.cc.clone()];
                let thread = draft.thread_id.clone();
                self.sent_thread = thread.clone();
                acc.run(move |s| {
                    gmail::send(s, &mime, thread.as_deref())?;
                    Ok(Event::Sent)
                });
            }
            Err(e) => self.notice = Some(Notice { text: e, error: true, at: Instant::now() }),
        }
    }

    fn modify(&mut self, acc: &Account, id: &str, add: &[&str], remove: &[&str]) {
        // Mise à jour locale immédiate (si le message est connu), confirmée ensuite par Gmail.
        if let Some(mut labels) = self.labels_of(id) {
            labels.retain(|l| !remove.contains(&l.as_str()));
            for a in add {
                if !labels.iter().any(|l| l == a) {
                    labels.push(a.to_string());
                }
            }
            self.apply_labels(id, &labels);
        }

        let (id, add, remove) = (id.to_string(), add.iter().map(|s| s.to_string()).collect::<Vec<_>>(), remove.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        acc.run(move |s| Ok(Event::Modified { labels: gmail::modify(s, &id, &add, &remove)?, id }));
    }

    fn labels_of(&self, id: &str) -> Option<Vec<String>> {
        self.messages.iter().chain(self.starred.iter()).chain(self.open.iter().map(|m| &m.summary)).find(|m| m.id == id).map(|m| m.labels.clone())
    }

    fn apply_labels(&mut self, id: &str, labels: &[String]) {
        let source = self.messages.iter().chain(self.starred.iter()).chain(self.open.iter().map(|m| &m.summary)).find(|m| m.id == id).cloned();

        for m in self.messages.iter_mut().filter(|m| m.id == id) {
            m.labels = labels.to_vec();
        }
        self.pinned.set_labels(id, labels);
        // Le message a quitté le dossier affiché (rangé ailleurs).
        let keep: Vec<bool> = self.messages.iter().map(|m| m.id != id || self.belongs(&m.labels)).collect();
        let mut k = keep.into_iter();
        self.messages.retain(|_| k.next().unwrap_or(true));

        if let Some(open) = self.open.as_mut().filter(|m| m.summary.id == id) {
            open.summary.labels = labels.to_vec();
        }

        // Favoris : présents tant qu'ils portent le libellé STARRED.
        let starred = labels.iter().any(|l| l == STARRED);
        self.starred.retain(|m| m.id != id || starred);
        if let Some(m) = self.starred.iter_mut().find(|m| m.id == id) {
            m.labels = labels.to_vec();
        } else if starred && let Some(mut m) = source {
            m.labels = labels.to_vec();
            self.starred.insert(0, m);
        }
    }

    /// Intègre un message mis à jour (relève incrémentale) dans les listes affichées.
    fn apply_summary(&mut self, m: Summary) {
        fn upsert(list: &mut Vec<Summary>, m: &Summary, belongs: bool) {
            list.retain(|x| x.id != m.id);
            if belongs {
                let pos = list.iter().position(|x| x.date < m.date).unwrap_or(list.len());
                list.insert(pos, m.clone());
            }
        }
        let in_folder = match self.folder.strip_prefix("from:") {
            Some(addr) => contacts::parse_mailbox(&m.from).is_some_and(|(_, e)| e == addr),
            None => self.belongs(&m.labels),
        };
        let known = self.messages.iter().any(|x| x.id == m.id);
        // Un mail ancien hors de la page affichée n'y est pas ajouté.
        let newer = self.messages.last().is_none_or(|last| m.date >= last.date);
        if known || newer {
            upsert(&mut self.messages, &m, in_folder);
        }
        upsert(&mut self.starred, &m, m.starred());
        self.pinned.update(&m);
        if let Some(open) = self.open.as_mut().filter(|o| o.summary.id == m.id) {
            open.summary.labels = m.labels.clone();
        }
    }

    pub fn activate_attachment(&mut self, acc: &Account, att: &Attachment) {
        let n = self.attachments.activate(acc, att);
        self.show_notice(n);
    }

    pub fn open_attachment_external(&mut self, acc: &Account, att: &Attachment) {
        let n = self.attachments.open_external(acc, att);
        self.show_notice(n);
    }

    /// Enregistre les expéditeurs dans l'annuaire.
    fn ingest(&mut self, acc: &Account, list: &[Summary]) {
        let own = acc.session().map(|s| s.email.as_str()).unwrap_or("");
        for m in list {
            // Mes propres envois (dossier Envoyés) : l'expéditeur c'est moi, on l'ignore.
            if !m.labels.iter().any(|l| l == "SENT") {
                self.contacts.record(&m.from, m.date, &m.id, own);
            }
        }
    }

    /// Affiche les mails d'une adresse (recherche Gmail `from:`).
    pub fn show_from(&mut self, acc: &Account, email: &str) {
        self.directory = false;
        self.select_folder(acc, &format!("from:{}", email.to_lowercase()));
    }

    /// Sélection d'un contact dans l'annuaire : charge ses derniers mails.
    pub fn select_contact(&mut self, acc: &Account, email: &str) {
        let email = email.to_lowercase();
        self.dir_selected = Some(email.clone());
        if self.dir_mails.as_ref().is_some_and(|(e, _)| *e == email) {
            return;
        }
        self.dir_mails = None;
        self.dir_loading = Some(email.clone());
        let label = format!("from:{email}");
        acc.run(move |s| Ok(Event::Messages { list: gmail::list(s, &label, 15)?, label }));
    }

    /// Nouveau message adressé à un contact.
    pub fn write_to(&mut self, mailbox: String) {
        let mut d = Draft::new();
        d.to = format!("{mailbox}, ");
        self.draft = Some(d);
        self.editor.reset();
        self.directory = false;
    }

    fn show_notice(&mut self, n: attachments::Notice) {
        if let Some((text, error)) = n {
            self.notice = Some(Notice { text, error, at: Instant::now() });
        }
    }

    fn info(&mut self, text: String) {
        self.notice = Some(Notice { text, error: false, at: Instant::now() });
    }

    pub fn folder_name(&self, id: &str) -> String {
        match id {
            INBOX => "Boîte de réception".into(),
            "SENT" => "Envoyés".into(),
            "SPAM" => "Spam".into(),
            "TRASH" => "Corbeille".into(),
            STARRED => "Favoris".into(),
            other if other.starts_with("from:") => {
                let addr = &other[5..];
                let name = self.contacts.get(addr).map(|c| c.display_name().to_string());
                format!("De : {}", name.unwrap_or_else(|| addr.to_string()))
            }
            other => self.labels.iter().find(|l| l.id == other).map(|l| l.name.clone()).unwrap_or_else(|| other.to_string()),
        }
    }
}
