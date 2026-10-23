use eframe::egui::{self, Margin};

use crate::google::{Account, Event};
use crate::prefs::Prefs;
use crate::tabs::mail::MailState;
use crate::{sidebar, tabs, theme};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Dashboard,
    Calendar,
    Mail,
    Messages,
    Notes,
    Files,
    Settings,
}

impl Tab {
    /// Onglets de la barre principale (Réglages a son propre bouton).
    pub const ALL: [Tab; 6] = [Tab::Dashboard, Tab::Calendar, Tab::Mail, Tab::Messages, Tab::Notes, Tab::Files];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => t!("Tableau de bord", "Dashboard"),
            Tab::Calendar => t!("Calendrier", "Calendar"),
            Tab::Mail => "Mail",
            Tab::Messages => "Messages",
            Tab::Notes => t!("Prise de notes", "Notes"),
            Tab::Files => t!("Fichiers", "Files"),
            Tab::Settings => t!("Réglages", "Settings"),
        }
    }
}

pub struct WorkTableApp {
    current: Tab,
    account: Account,
    mail: MailState,
    calendar: tabs::calendar::CalendarState,
    chat: tabs::messages::ChatState,
    tasks: crate::tasks::Tasks,
    search: crate::search::Palette,
    drive: tabs::files::DriveState,
    notes: tabs::notes::NotesState,
    links: crate::links::Links,
    notify: crate::notify::Notifier,
    /// Fiches contact (annuaire caché, chiffré) et leur overlay.
    people: crate::people::People,
    card: crate::people::CardView,
    prefs: Prefs,
    /// Mises à jour publiées sur GitHub (vérifiées au lancement).
    update: crate::update::Updater,
}

impl WorkTableApp {
    /// Mails non lus de la boîte de réception (aperçu au survol de l'icône Mail).
    fn mail_peek(&self) -> Vec<sidebar::PeekItem> {
        let mut seen = std::collections::HashSet::new();
        let mut v: Vec<&crate::google::gmail::Summary> =
            self.mail.messages.iter().filter(|m| m.unread() && m.labels.iter().any(|l| l == "INBOX") && seen.insert(m.thread_id.clone())).collect();
        v.sort_by_key(|m| -m.date);
        v.into_iter()
            .map(|m| sidebar::PeekItem {
                icon: "✉️",
                title: m.sender_name().to_string(),
                text: if m.subject.trim().is_empty() { t!("(sans objet)", "(no subject)").into() } else { m.subject.clone() },
                when: m.date,
                target: crate::search::Target::Mail(m.id.clone()),
            })
            .collect()
    }

    /// Conversations Chat non lues (aperçu au survol de l'icône Messages).
    fn chat_peek(&self) -> Vec<sidebar::PeekItem> {
        let mut v: Vec<_> = self.chat.spaces.iter().filter(|s| self.chat.unread(s) && !self.chat.is_muted(&s.name)).collect();
        v.sort_by_key(|s| -s.last_active);
        v.into_iter()
            .map(|sp| {
                let last = self.chat.msgs.get(&sp.name).and_then(|l| l.iter().rev().find(|m| !m.text.trim().is_empty()));
                let text = match last {
                    Some(m) => tf!("{} : {}", "{}: {}", self.chat.name_of(&m.sender), m.text.replace(['\n', '\r'], " ")),
                    None => t!("Nouveaux messages", "New messages").into(),
                };
                sidebar::PeekItem { icon: "💬", title: self.chat.title(sp), text, when: sp.last_active, target: crate::search::Target::Space(sp.name.clone()) }
            })
            .collect()
    }

    pub fn new(ctx: &egui::Context) -> Self {
        let mut prefs = Prefs::load();
        crate::i18n::set(prefs.language);
        if prefs.theme == theme::ThemeId::Retired {
            prefs.theme = theme::ThemeId::Monochrome;
        }
        theme::set_fonts(ctx, &prefs.fonts);
        theme::set_theme(ctx, prefs.theme);
        tabs::messages::emoji::set_recent(prefs.chat_recent_emojis.clone());
        tabs::messages::emoji::set_tone(prefs.emoji_tone);
        // Nouvelle version sur GitHub ? (en arrière-plan ; pastille sur le bouton Réglages)
        let update = crate::update::Updater::default();
        update.check(ctx);
        Self {
            current: Tab::Dashboard,
            account: Account::new(ctx.clone()),
            mail: MailState::default(),
            calendar: Default::default(),
            chat: Default::default(),
            tasks: crate::tasks::Tasks::load(),
            search: Default::default(),
            drive: Default::default(),
            notes: Default::default(),
            links: crate::links::Links::load(),
            notify: Default::default(),
            people: Default::default(),
            card: Default::default(),
            prefs,
            update,
        }
    }
}

impl WorkTableApp {
    /// Fiche « Auteur » d'un fichier Drive : ouvre l'élément demandé dans son onglet.
    fn drive_action(&mut self, a: tabs::files::author::DriveAction) {
        use tabs::files::author::DriveAction as A;
        match a {
            A::OpenMail(id) => self.go_to(crate::search::Target::Mail(id)),
            A::MailsFrom(email) => {
                self.mail.show_from(&self.account, &email);
                self.current = Tab::Mail;
            }
            A::WriteMail(email) => {
                let mailbox = self.mail.contacts.get(&email).map(|c| c.mailbox()).unwrap_or(email);
                self.mail.write_to(mailbox);
                self.current = Tab::Mail;
            }
            A::ChatWith(email) => {
                self.chat.request_dm(email);
                self.current = Tab::Messages;
            }
            A::OpenSpace(space) => self.go_to(crate::search::Target::Space(space)),
            A::OpenEvent(e) => {
                self.calendar.show_event(&e);
                self.current = Tab::Calendar;
            }
        }
    }
}

impl WorkTableApp {
    /// Rappel minuté : panneau Rappels du tableau de bord, carte mise en évidence.
    fn show_timer(&mut self, ctx: &egui::Context, id: u64) {
        self.current = Tab::Dashboard;
        tabs::dashboard::flash_timer(ctx, id);
    }

    /// Élément ouvert depuis une fiche contact.
    fn card_action(&mut self, a: crate::people::Action) {
        use crate::people::Action as A;
        use crate::search::Target;
        use tabs::files::author::DriveAction as D;
        match a {
            A::Mail(id) => self.go_to(Target::Mail(id)),
            A::Space(s) => self.go_to(Target::Space(s)),
            A::Drive(f) => self.go_to(Target::Drive(f)),
            A::File(f) => self.go_to(Target::File(f)),
            A::Event(cal, id, start) => match crate::people::live_event(&self.calendar, &cal, &id, start) {
                Some(e) => self.go_to(Target::Event(e)),
                None => self.go_to(Target::Date(tabs::calendar::date_of(start))),
            },
            A::WriteMail(email) => self.drive_action(D::WriteMail(email)),
            A::ChatWith(email) => self.drive_action(D::ChatWith(email)),
            A::Directory(email) => self.go_to(Target::Contact(email)),
        }
    }
}

impl eframe::App for WorkTableApp {
    /// Fond transparent : la fenêtre du bandeau de notification ne montre que la capsule
    /// (la fenêtre principale est entièrement recouverte par l'interface).
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// Fermeture : dernier enregistrement de la copie locale des conversations.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.chat.flush_store();
        self.people.flush();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let window_rect = ui.max_rect();
        crate::liquid::set_settings(ui.ctx(), self.prefs.animation);
        let prefs_before = self.prefs.clone();
        // Visionneuse d'images (avant les onglets : elle prend Échap pour elle).
        crate::lightbox::show(ui.ctx());

        // Résultats des requêtes Google arrivés depuis la dernière frame.
        for ev in self.account.poll() {
            match ev {
                Event::Ready => {
                    self.calendar.on_ready(&self.account);
                    self.chat.on_ready();
                    self.drive.on_ready();
                    self.mail.handle(Event::Ready, &self.account);
                }
                e @ (Event::Calendars(_) | Event::CalEvents { .. } | Event::CalSaved { .. } | Event::CalDeleted { .. } | Event::CalError(_)) => {
                    self.calendar.handle(e, &self.account)
                }
                Event::Chat(e) => {
                    if let crate::google::chat::ChatEvent::Messages { space, list, .. } = &e {
                        self.notify.on_chat_messages(space, list, &self.chat, &self.prefs);
                    }
                    let spaces = matches!(e, crate::google::chat::ChatEvent::Spaces(_));
                    self.chat.handle(e, &self.account);
                    if spaces {
                        self.notify.on_chat_spaces(&self.chat, &self.account, &self.prefs);
                    }
                }
                // Mails : chargement complet (référence) ou relève (nouveaux mails à notifier).
                Event::Messages { label, list } => {
                    self.notify.on_mails(&list, &self.prefs, true);
                    self.mail.handle(Event::Messages { label, list }, &self.account);
                }
                Event::Delta { changed, deleted } => {
                    self.notify.on_mails(&changed, &self.prefs, false);
                    self.mail.handle(Event::Delta { changed, deleted }, &self.account);
                }
                Event::MailSearch { query, list } => {
                    if let Some(list) = self.notes.on_mail(&query, list) {
                        self.search.on_remote(query, list);
                    }
                }
                Event::Drive(crate::google::drive::DriveEvent::Found { query, files }) => {
                    if let Some(files) = self.notes.on_drive(&query, files) {
                        self.search.on_drive(query, files);
                    }
                }
                Event::PersonMails { email, list } => {
                    let own = self.account.session().map(|s| s.email.to_lowercase()).unwrap_or_default();
                    self.people.on_mails(email, list, &own);
                }
                Event::Drive(e) => {
                    if let crate::google::drive::DriveEvent::Listing { source: crate::google::drive::Source::Shared, files, .. } = &e {
                        self.notify.on_shared(files, &self.prefs);
                    }
                    self.drive.handle(e, &self.account)
                }
                e => self.mail.handle(e, &self.account),
            }
        }
        self.mail.tick(&self.account, ui.ctx());
        self.calendar.background(&self.account, &self.prefs);
        self.chat.muted.clone_from(&self.prefs.chat_muted);
        self.chat.recent_emojis = tabs::messages::emoji::recent();
        self.chat.background(&self.account, self.current == Tab::Messages, ui.ctx());
        self.notes.background(ui.ctx());
        self.people.background(ui.ctx(), &self.account, &self.mail, &self.chat, &self.calendar, &self.drive);
        self.notify.background(ui.ctx(), &self.account, &self.calendar, &self.prefs);
        // Rappels minutés : notification à l'échéance, pastilles dans le calendrier.
        let now = chrono::Local::now().timestamp_millis();
        for t in self.tasks.fire_due(now) {
            self.notify.on_timer(&self.prefs, &t, t.due < now - 60_000);
        }
        self.tasks.save_if_needed();
        if let Some(next) = self.tasks.timers.iter().filter(|t| !t.fired).map(|t| t.due).min() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis((next - now).clamp(200, 60_000) as u64));
        }
        self.calendar.timers.clone_from(&self.tasks.timers);
        if let Some(id) = self.calendar.open_timer.take() {
            self.show_timer(ui.ctx(), id);
        }
        let mut note_action = None;

        // 1. L'onglet ouvert occupe toute la fenêtre.
        //    La marge gauche réserve la place de la barre flottante pour garder le contenu lisible.
        egui::CentralPanel::no_frame().frame(egui::Frame::new().fill(theme::BG()).inner_margin(Margin { left: theme::CONTENT_LEFT_INSET, right: 32, top: 36, bottom: 32 })).show(
            ui,
            |ui| match self.current {
                Tab::Dashboard => {
                    match tabs::dashboard::show(ui, &self.calendar, &mut self.mail, &mut self.chat.pins, &self.account, &self.prefs, &mut self.tasks, &mut self.links) {
                        Some(tabs::dashboard::DashAction::Mail(id)) => {
                            self.mail.open_from_elsewhere(&self.account, &id);
                            self.current = Tab::Mail;
                        }
                        Some(tabs::dashboard::DashAction::Event(e)) => {
                            self.calendar.show_event(&e);
                            self.current = Tab::Calendar;
                        }
                        Some(tabs::dashboard::DashAction::Chat(space)) => self.go_to(crate::search::Target::Space(space)),
                        None => {}
                    }
                }
                Tab::Calendar => tabs::calendar::show(ui, &mut self.calendar, &mut self.account, &mut self.prefs, &self.mail.contacts),
                Tab::Mail => tabs::mail::show(ui, &mut self.mail, &mut self.account, &mut self.prefs),
                Tab::Messages => tabs::messages::show(ui, &mut self.chat, &mut self.account, &mut self.prefs, &self.mail.contacts),
                Tab::Notes => note_action = tabs::notes::show(ui, &mut self.notes, &self.calendar, &self.mail, &self.chat, &self.tasks, &self.account),
                Tab::Files => {
                    let links = tabs::files::author::Links { mail: &self.mail, chat: &self.chat, cal: &self.calendar };
                    let action = tabs::files::show(ui, &mut self.drive, &mut self.account, &mut self.prefs, &links);
                    if let Some(a) = action {
                        self.drive_action(a);
                    }
                }
                Tab::Settings => tabs::settings::show(ui, &mut self.account, &mut self.prefs, &self.update),
            },
        );

        // Liens d'une note vers le calendrier, un mail ou un fichier Drive.
        match note_action {
            Some(tabs::notes::NotesAction::Event(e)) => self.go_to(crate::search::Target::Event(e)),
            Some(tabs::notes::NotesAction::Date(d)) => self.go_to(crate::search::Target::Date(d)),
            Some(tabs::notes::NotesAction::Mail(id)) => self.go_to(crate::search::Target::Mail(id)),
            Some(tabs::notes::NotesAction::Drive(f)) => self.go_to(crate::search::Target::Drive(f)),
            Some(tabs::notes::NotesAction::Chat(s)) => self.go_to(crate::search::Target::Space(s)),
            Some(tabs::notes::NotesAction::Task) => self.go_to(crate::search::Target::Task),
            None => {}
        }

        // Liens entre onglets : écrire sur Chat depuis l'annuaire, un mail depuis Chat.
        if let Some(email) = self.mail.chat_to.take() {
            self.chat.request_dm(email);
            self.current = Tab::Messages;
        }
        if let Some(email) = self.chat.mail_to.take() {
            let mailbox = self.mail.contacts.get(&email).map(|c| c.mailbox()).unwrap_or(email);
            self.mail.write_to(mailbox);
            self.current = Tab::Mail;
        }

        // Heure en haut à droite : un clic la masque ou la réaffiche (animation élastique).
        if crate::widgets::clock(ui.ctx(), window_rect, self.prefs.clock_hidden) {
            self.prefs.clock_hidden = !self.prefs.clock_hidden;
        }
        // « Ne pas déranger », à gauche de l'heure, seulement dans le tableau de bord : la bascule
        // fait vibrer les panneaux en vague à partir du bouton.
        if self.current == Tab::Dashboard
            && let Some(origin) = crate::widgets::do_not_disturb(ui.ctx(), window_rect, self.prefs.do_not_disturb)
        {
            self.prefs.do_not_disturb = !self.prefs.do_not_disturb;
            if self.prefs.do_not_disturb {
                self.notify.silence();
            }
            crate::widgets::start_wave(ui.ctx(), origin);
        }

        // 2. Barre de navigation verticale en pilule, dessinée par-dessus (pastilles « non lus »).
        let badges = [(Tab::Mail, self.mail.inbox_unread), (Tab::Messages, self.chat.unread_count())];
        let peeks = [(Tab::Mail, self.mail_peek()), (Tab::Messages, self.chat_peek())];
        match sidebar::show(ui.ctx(), window_rect, self.current, &badges, &peeks) {
            Some(sidebar::Nav::Tab(tab)) => self.current = tab,
            Some(sidebar::Nav::Go(t)) => self.go_to(*t),
            None => {}
        }

        // Notifications (mail, message, document partagé, rendez-vous proche) : clic = y aller.
        if let Some(t) = self.notify.show(ui.ctx(), window_rect) {
            use crate::notify::Target as N;
            use crate::search::Target;
            match t {
                N::Mail(id) => self.go_to(Target::Mail(id)),
                N::Space(s) => self.go_to(Target::Space(s)),
                N::Drive(f) => self.go_to(Target::Drive(f)),
                N::Event(e) => self.go_to(Target::Event(e)),
                N::Reminder(id) => self.show_timer(ui.ctx(), id),
            }
        }

        // 3. Bouton Réglages, ancré en bas à gauche, toujours visible.
        if sidebar::settings_button(ui.ctx(), window_rect, self.current == Tab::Settings, self.update.badge()) {
            self.current = Tab::Settings;
        }

        // Barre de titre intégrée : déplacement, réduire / agrandir / fermer, redimensionnement.
        crate::titlebar::show(ui.ctx(), window_rect);

        // 4. Recherche globale (Ctrl+Espace), par-dessus tout le reste.
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::Space)) {
            self.search.toggle(ui.ctx());
        }
        if let Some(t) = self.search.show(ui.ctx(), &self.account, &self.mail, &self.calendar, &self.chat, &self.tasks, &self.notes, &self.people, &self.prefs) {
            self.go_to(t);
        }
        // Fiche contact (clic droit sur une trace d'une personne, ou depuis la recherche).
        if let Some(a) = self.card.show(ui.ctx(), &mut self.people, &self.mail, &self.chat, &self.account) {
            self.card_action(a);
        }

        let recent = tabs::messages::emoji::recent();
        let tone = tabs::messages::emoji::tone();
        if tone != self.prefs.emoji_tone {
            self.prefs.emoji_tone = tone;
        }
        if recent != self.prefs.chat_recent_emojis {
            self.prefs.chat_recent_emojis = recent;
        }

        // 5. Émojis en couleur (même rendu que dans Google Chat) dans tous les onglets.
        crate::emoji_img::colorize_frame(ui.ctx());

        if self.prefs.language != prefs_before.language {
            crate::i18n::set(self.prefs.language);
        }
        if self.prefs.theme != prefs_before.theme {
            theme::set_theme(ui.ctx(), self.prefs.theme);
        }
        if self.prefs.fonts != prefs_before.fonts {
            theme::set_fonts(ui.ctx(), &self.prefs.fonts);
        }
        if self.prefs != prefs_before {
            self.prefs.save();
        }
    }
}

impl WorkTableApp {
    /// Se rend sur un résultat de la recherche globale.
    fn go_to(&mut self, t: crate::search::Target) {
        use crate::search::Target;
        match t {
            Target::Mail(id) => {
                self.mail.open_from_elsewhere(&self.account, &id);
                self.current = Tab::Mail;
            }
            // Ouverte par la recherche elle-même (overlay de la fiche contact).
            Target::Person(_) => {}
            Target::Contact(email) => {
                self.mail.directory = true;
                self.mail.select_contact(&self.account, &email);
                self.current = Tab::Mail;
            }
            Target::Event(e) => {
                self.calendar.show_event(&e);
                self.current = Tab::Calendar;
            }
            Target::Date(d) => {
                self.calendar.show_date(d);
                self.current = Tab::Calendar;
            }
            Target::Space(s) => {
                self.chat.select(&self.account, &s);
                self.current = Tab::Messages;
            }
            Target::Task => self.current = Tab::Dashboard,
            Target::Drive(f) => {
                self.drive.reveal(&self.account, f);
                self.current = Tab::Files;
            }
            Target::Tab(t) => self.current = t,
            Target::Note(id) => {
                self.notes.select(&id);
                self.current = Tab::Notes;
            }
            Target::NewNote => {
                self.notes.new_note();
                self.current = Tab::Notes;
            }
            Target::Compose => {
                self.mail.write_to(String::new());
                if let Some(d) = &mut self.mail.draft {
                    d.to.clear();
                }
                self.current = Tab::Mail;
            }
            Target::NewEvent => {
                let now = chrono::Local::now().timestamp_millis();
                let start = (now / 1_800_000 + 1) * 1_800_000;
                self.calendar.show_date(chrono::Local::now().date_naive());
                self.calendar.new_event(&self.prefs, start, start + 3_600_000, false);
                self.current = Tab::Calendar;
            }
            Target::File(f) => match (&f.url, &f.attachment) {
                (Some(u), _) => {
                    let _ = webbrowser::open(u);
                }
                (None, Some(a)) => self.mail.open_attachment_external(&self.account, a),
                _ => {}
            },
        }
    }
}
