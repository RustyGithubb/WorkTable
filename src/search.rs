//! Recherche globale (Ctrl+Espace) : barre au centre, par-dessus l'app, sur un fond flouté.
//! Cherche dans les mails (chargés + recherche Gmail), l'annuaire, les événements et les dates
//! du calendrier, Google Chat, les tâches, les fichiers partagés, les plannings HyperPlanning et
//! les commandes de l'app.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as CDuration, Local, NaiveDate, TimeZone};
use eframe::egui::{self, Align2, Color32, ColorImage, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::app::Tab;
use crate::google::calendar::CalEvent;
use crate::google::drive::DriveFile;
use crate::google::gmail::SharedFile;
use crate::google::{Account, Event, Summary, gmail};
use crate::prefs::Prefs;
use crate::tabs::calendar::CalendarState;
use crate::tabs::mail::MailState;
use crate::tabs::mail::contacts::fold;
use crate::tabs::messages::ChatState;
use crate::tasks::Tasks;
use crate::theme;

/// Destination d'un résultat.
#[derive(Clone)]
pub enum Target {
    Mail(String),
    /// Fiche contact (adresse ou identifiant Google Chat).
    Person(String),
    Contact(String),
    Event(CalEvent),
    Date(NaiveDate),
    Space(String),
    Task,
    Tab(Tab),
    Compose,
    NewEvent,
    File(SharedFile),
    Drive(DriveFile),
    Note(String),
    NewNote,
    /// Planning HyperPlanning d'un enseignant, d'une salle, d'une promotion…
    Planning(crate::tabs::plannings::Kind, u32),
    /// Plusieurs plannings HyperPlanning ouverts ensemble.
    Plannings(Vec<(crate::tabs::plannings::Kind, u32)>),
    /// Séance HyperPlanning : ouvre ces plannings au jour de la séance, séance sélectionnée.
    Seance {
        sel: Vec<(crate::tabs::plannings::Kind, u32)>,
        cours: u32,
        start: chrono::NaiveDateTime,
    },
}

/// Mot reconnu dans la recherche, affiché en pastille : catégorie, élément HyperPlanning
/// (groupe, salle, prof…) ou action.
#[derive(Clone, PartialEq, Debug)]
enum Pill {
    Scope(Scope),
    Entity(crate::tabs::plannings::Kind, u32, String),
    /// Nom de cours (matière) : toutes ses séances en liste. (nom replié, nom affiché)
    Course(String, String),
    Action(Action),
}

/// Actions cachées derrière un mot (« rdv », « nouvelle note », « réglages »…).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Compose,
    NewEvent,
    NewNote,
    Tab(Tab),
}

impl Action {
    fn target(self) -> Target {
        match self {
            Action::Compose => Target::Compose,
            Action::NewEvent => Target::NewEvent,
            Action::NewNote => Target::NewNote,
            Action::Tab(t) => Target::Tab(t),
        }
    }

    fn label(self) -> String {
        match self {
            Action::Compose => t!("Nouveau mail", "New mail").into(),
            Action::NewEvent => t!("Nouvel événement", "New event").into(),
            Action::NewNote => t!("Nouvelle note", "New note").into(),
            Action::Tab(t) => t.label().into(),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Action::Compose => "✉️",
            Action::NewEvent => "🗓️",
            Action::NewNote => "📝",
            Action::Tab(Tab::Dashboard) => "🏠",
            Action::Tab(Tab::Settings) => "⚙️",
            Action::Tab(Tab::Blueprint) => "🧩",
            Action::Tab(_) => "📁",
        }
    }
}

/// Mots (repliés, sans accents) qui cachent une action ; « nouveau/nouvelle… » + mot ci-dessous.
const ACTION_WORDS: [(&str, Action); 14] = [
    ("ecrire", Action::Compose),
    ("rediger", Action::Compose),
    ("compose", Action::Compose),
    ("rdv", Action::NewEvent),
    ("reunion", Action::NewEvent),
    ("dashboard", Action::Tab(Tab::Dashboard)),
    ("tdb", Action::Tab(Tab::Dashboard)),
    ("accueil", Action::Tab(Tab::Dashboard)),
    ("reglages", Action::Tab(Tab::Settings)),
    ("settings", Action::Tab(Tab::Settings)),
    ("parametres", Action::Tab(Tab::Settings)),
    ("blueprint", Action::Tab(Tab::Blueprint)),
    ("fichiers", Action::Tab(Tab::Files)),
    ("files", Action::Tab(Tab::Files)),
];
const NEW_WORDS: [&str; 6] = ["nouveau", "nouvelle", "nouvel", "new", "creer", "create"];
const NEW_WHAT: [(&str, Action); 9] = [
    ("mail", Action::Compose),
    ("message", Action::Compose),
    ("courriel", Action::Compose),
    ("note", Action::NewNote),
    ("evenement", Action::NewEvent),
    ("event", Action::NewEvent),
    ("rdv", Action::NewEvent),
    ("reunion", Action::NewEvent),
    ("rendez-vous", Action::NewEvent),
];

/// Reconnaît un mot (ou plusieurs : « nouvelle note », « Bachelor 2 JV ») au début de `words`
/// (repliés) : (pastille, nombre de mots).
fn recognize(
    words: &[String],
    names: &std::collections::HashMap<String, (crate::tabs::plannings::Kind, u32, String)>,
    courses: &std::collections::HashMap<String, String>,
) -> Option<(Pill, usize)> {
    if words.len() >= 2
        && NEW_WORDS.contains(&words[0].as_str())
        && let Some((_, a)) = NEW_WHAT.iter().find(|(w, _)| *w == words[1])
    {
        return Some((Pill::Action(*a), 2));
    }
    if words.len() >= 3 && words[..3].join(" ") == "tableau de bord" {
        return Some((Pill::Action(Action::Tab(Tab::Dashboard)), 3));
    }
    // Élément HyperPlanning : nom exact, le plus long d'abord.
    for n in (1..=words.len().min(6)).rev() {
        let key = words[..n].join(" ");
        if let Some((kind, k, name)) = names.get(&key) {
            return Some((Pill::Entity(*kind, *k, name.clone()), n));
        }
        if let Some(name) = courses.get(&key) {
            return Some((Pill::Course(key, name.clone()), n));
        }
    }
    let w = words.first()?;
    if let Some((scope, ..)) = SCOPES.iter().find(|(_, list, _, _)| list.contains(&w.as_str())) {
        return Some((Pill::Scope(*scope), 1));
    }
    ACTION_WORDS.iter().find(|(x, _)| x == w).map(|(_, a)| (Pill::Action(*a), 1))
}

/// Catégorie ciblée par le premier mot de la recherche (« mail facture », « hpp 101 »).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scope {
    Hpp,
    Calendar,
    Messages,
    Mail,
    Drive,
    Sheets,
    Docs,
    Notes,
}

/// Mots-clés (et abréviations) de chaque catégorie, puis son nom affiché.
const SCOPES: [(Scope, &[&str], &str, &str); 8] = [
    (Scope::Hpp, &["hyperplanning", "hpp", "hp"], "HyperPlanning", "HyperPlanning"),
    (Scope::Calendar, &["calendrier", "calendar", "agenda", "cal"], "Calendrier", "Calendar"),
    (Scope::Messages, &["messages", "message", "msg", "chat"], "Messages", "Messages"),
    (Scope::Mail, &["mails", "mail", "gmail", "ml"], "Mail", "Mail"),
    (Scope::Drive, &["drive", "drv"], "Drive", "Drive"),
    (Scope::Sheets, &["sheets", "sheet", "gsheet", "gsheets", "tableur", "sht"], "Sheets", "Sheets"),
    (Scope::Docs, &["docs", "doc", "gdoc", "gdocs"], "Docs", "Docs"),
    (Scope::Notes, &["notes", "note", "nt"], "Notes", "Notes"),
];

/// Sépare un mot-clé de catégorie en tête de la requête : (catégorie, reste de la requête).
fn split_scope(q: &str) -> (Option<Scope>, String) {
    let q = q.trim();
    let (first, rest) = q.split_once(char::is_whitespace).unwrap_or((q, ""));
    let first = fold(first);
    match SCOPES.iter().find(|(_, words, _, _)| words.contains(&first.as_str())) {
        Some((scope, ..)) => (Some(*scope), rest.trim().to_string()),
        None => (None, q.to_string()),
    }
}

fn scope_name(scope: Scope) -> &'static str {
    SCOPES.iter().find(|(s, ..)| *s == scope).map(|(_, _, fr, en)| t!(*fr, *en)).unwrap_or("")
}

/// Le résultat appartient-il à la catégorie ? (actions : celles de l'onglet correspondant).
fn in_scope(h: &Hit, scope: Scope) -> bool {
    let drive_mime = |pred: fn(&str) -> bool| matches!(&h.target, Target::Drive(f) if pred(&f.mime));
    match scope {
        Scope::Hpp => matches!(h.target, Target::Planning(..) | Target::Plannings(_) | Target::Seance { .. } | Target::Tab(Tab::Plannings)),
        Scope::Calendar => matches!(h.target, Target::Event(_) | Target::Date(_) | Target::NewEvent | Target::Tab(Tab::Calendar)),
        Scope::Messages => matches!(h.target, Target::Space(_) | Target::Tab(Tab::Messages)),
        Scope::Mail => matches!(h.target, Target::Mail(_) | Target::Compose | Target::Tab(Tab::Mail)),
        Scope::Drive => matches!(h.target, Target::Drive(_) | Target::Tab(Tab::Files)),
        Scope::Sheets => drive_mime(|m| m == "application/vnd.google-apps.spreadsheet" || m.contains("sheet") || m.contains("excel") || m.contains("csv")),
        Scope::Docs => drive_mime(|m| m == "application/vnd.google-apps.document" || m.contains("word")),
        Scope::Notes => matches!(h.target, Target::Note(_) | Target::NewNote | Target::Tab(Tab::Notes)),
    }
}

#[derive(Clone)]
struct Hit {
    group: &'static str,
    icon: &'static str,
    title: String,
    sub: String,
    target: Target,
    /// Plus haut = plus pertinent (dans le groupe).
    score: i32,
}

#[derive(Default)]
pub struct Palette {
    pub open: bool,
    query: String,
    selected: usize,
    /// Texte de la recherche modifié : la liste revient en haut (une seule fois). Sinon la vue
    /// reste libre, même quand la sélection change (flèches, survol).
    scroll_top: bool,
    bg: Backdrop,
    /// Recherche Gmail côté serveur (complète les mails déjà chargés).
    remote: Vec<Summary>,
    remote_query: String,
    /// Recherche Google Drive (même requête).
    drive: Vec<DriveFile>,
    edited: Option<Instant>,
    /// Mots reconnus, sortis du texte et affichés en pastilles.
    pills: Vec<Pill>,
    /// Texte réécrit (pastilles) : curseur à replacer en fin de texte.
    cursor_end: bool,
}

/// Fond d'un overlay (recherche, fiche contact) : capture floutée de l'app, voile sombre,
/// apparition en fondu ; un clic dessus ferme l'overlay.
#[derive(Default)]
pub struct Backdrop {
    shot: Arc<Mutex<Option<Arc<ColorImage>>>>,
    texture: Option<egui::TextureHandle>,
    opened_at: Option<Instant>,
}

impl Backdrop {
    /// Ouverture : capture de l'image affichée (l'overlay est encore transparent).
    pub fn start(&mut self, ctx: &egui::Context) {
        self.texture = None;
        self.opened_at = Some(Instant::now());
        let slot = self.shot.clone();
        let c = ctx.clone();
        ctx.request_screenshot(move |img| {
            let blurred = blur(&img, 4, 3);
            if let Ok(mut s) = slot.lock() {
                *s = Some(Arc::new(blurred));
            }
            c.request_repaint();
        });
    }

    pub fn clear(&mut self) {
        self.texture = None;
    }

    /// Dessine le fond ; renvoie (avancement du fondu 0..1, clic sur le fond).
    pub fn show(&mut self, ctx: &egui::Context, id: &str) -> (f32, bool) {
        self.show_in(ctx, id, egui::Order::Tooltip)
    }

    /// Même fond, à un autre niveau (fenêtre de rédaction : ses menus et suggestions passent devant).
    pub fn show_in(&mut self, ctx: &egui::Context, id: &str, order: egui::Order) -> (f32, bool) {
        let screen = ctx.content_rect();
        if let Some(img) = self.shot.lock().ok().and_then(|mut s| s.take()) {
            self.texture = Some(ctx.load_texture(format!("{id}_tex"), img, egui::TextureOptions::LINEAR));
        }
        let k = self.opened_at.map(|t| (t.elapsed().as_secs_f32() / 0.14).min(1.0)).unwrap_or(1.0);
        if k < 1.0 {
            ctx.request_repaint();
        }
        let mut clicked = false;
        egui::Area::new(egui::Id::unique(id)).order(order).fixed_pos(screen.min).show(ctx, |ui| {
            let (r, resp) = ui.allocate_exact_size(screen.size(), Sense::click());
            let p = ui.painter();
            if let Some(t) = &self.texture {
                let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                p.image(t.id(), r, uv, crate::theme::overlay((255.0 * k) as u8));
            }
            p.rect_filled(r, CornerRadius::ZERO, Color32::from_black_alpha((80.0 * k) as u8));
            clicked = resp.clicked();
        });
        (k, clicked)
    }
}

/// Cadre de la boîte d'un overlay (recherche, fiche contact).
pub fn overlay_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_premultiplied(0x1C, 0x1C, 0x1C, 0xF4))
        .stroke(Stroke::new(1.0, crate::theme::overlay(28)))
        .corner_radius(CornerRadius::same(16))
        .shadow(egui::Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(170) })
        .inner_margin(egui::Margin::same(14))
}

impl Palette {
    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.open {
            self.close();
            return;
        }
        self.open = true;
        self.query.clear();
        self.pills.clear();
        self.selected = 0;
        self.bg.start(ctx);
    }

    fn close(&mut self) {
        self.open = false;
        self.bg.clear();
    }

    /// Mots terminés (suivis d'une espace) reconnus : sortis du texte, ajoutés en pastilles.
    fn absorb(&mut self, hp: Option<&crate::hyperplanning::cache::Snapshot>) {
        let complete = self.query.ends_with(char::is_whitespace);
        let raw: Vec<String> = self.query.split_whitespace().map(str::to_string).collect();
        let usable = if complete { raw.len() } else { raw.len().saturating_sub(1) };
        if usable == 0 {
            return;
        }
        let folded: Vec<String> = raw.iter().map(|w| fold(w)).collect();
        let mut names = std::collections::HashMap::new();
        let mut courses = std::collections::HashMap::new();
        if let Some(d) = hp {
            for name in d.matieres.values().filter(|n| !n.trim().is_empty()) {
                courses.entry(course_key(name)).or_insert_with(|| name.clone());
            }
            for kind in crate::tabs::plannings::Kind::ALL {
                for (k, name) in crate::tabs::plannings::entries(d, kind) {
                    names.entry(fold(&name).split_whitespace().collect::<Vec<_>>().join(" ")).or_insert((kind, k, name));
                }
            }
        }
        let mut keep: Vec<String> = Vec::new();
        let mut found = false;
        let mut i = 0;
        while i < usable {
            match recognize(&folded[i..usable], &names, &courses) {
                Some((pill, n)) => {
                    if !self.pills.contains(&pill) {
                        self.pills.push(pill);
                    }
                    found = true;
                    i += n;
                }
                None => {
                    keep.push(raw[i].clone());
                    i += 1;
                }
            }
        }
        if !found {
            return;
        }
        keep.extend(raw[usable..].iter().cloned());
        self.query = keep.join(" ");
        if complete && !keep.is_empty() {
            self.query.push(' ');
        }
        // Le curseur suit la fin du texte modifié.
        self.cursor_end = true;
    }

    /// Résultats de la recherche Google Drive.
    pub fn on_drive(&mut self, query: String, files: Vec<DriveFile>) {
        if query == self.remote_query {
            self.drive = files;
        }
    }

    /// Résultats de la recherche Gmail côté serveur.
    pub fn on_remote(&mut self, query: String, list: Vec<Summary>) {
        if query == self.remote_query {
            self.remote = list;
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        acc: &Account,
        mail: &MailState,
        cal: &CalendarState,
        chat: &ChatState,
        tasks: &Tasks,
        notes: &crate::tabs::notes::NotesState,
        people: &crate::people::People,
        prefs: &Prefs,
        hp: Option<&crate::hyperplanning::cache::Snapshot>,
    ) -> Option<Target> {
        if !self.open {
            return None;
        }
        let screen = ctx.content_rect();
        // Fond : capture floutée + voile sombre ; un clic à côté ferme.
        let (k, mut close) = self.bg.show(ctx, "wt_search_backdrop");

        // Recherche Gmail après une courte pause de frappe.
        let (text_scope, q) = split_scope(&self.query);
        let pills = self.pills.clone();
        let has_entity = pills.iter().any(|p| matches!(p, Pill::Entity(..) | Pill::Course(..)));
        let scope = pills.iter().find_map(|p| if let Pill::Scope(s) = p { Some(*s) } else { None }).or(text_scope).or(has_entity.then_some(Scope::Hpp));
        if q.chars().count() >= 3 && q != self.remote_query && self.edited.is_some_and(|t| t.elapsed() > Duration::from_millis(350)) {
            self.remote_query = q.clone();
            self.remote.clear();
            self.drive.clear();
            let query = q.clone();
            acc.run(move |s| {
                let list = gmail::list(s, &format!("search:{query}"), 8).unwrap_or_default();
                Ok(Event::MailSearch { query, list })
            });
            let query = q.clone();
            acc.run(move |s| {
                let files = crate::google::drive::list(s, &crate::google::drive::Source::Search(query.clone()), None).map(|r| r.0).unwrap_or_default();
                Ok(Event::Drive(crate::google::drive::DriveEvent::Found { query, files }))
            });
        } else if self.edited.is_some() {
            ctx.request_repaint_after(Duration::from_millis(120));
        }

        let hits = if q.is_empty() && scope.is_none() && pills.is_empty() {
            suggestions(cal, prefs)
        } else {
            search(&q, scope, &pills, mail, cal, chat, tasks, notes, people, prefs, &self.remote, &self.drive, hp)
        };
        if self.selected >= hits.len() {
            self.selected = hits.len().saturating_sub(1);
        }
        let mut go = None;
        let mut picked: Option<Hit> = None;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                close = true;
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !hits.is_empty() {
                self.selected = (self.selected + 1) % hits.len();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) && !hits.is_empty() {
                self.selected = (self.selected + hits.len() - 1) % hits.len();
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) {
                go = hits.get(self.selected).map(|h| h.target.clone());
                picked = hits.get(self.selected).cloned();
            }
        });

        let w = 640.0_f32.min(screen.width() - 40.0);
        let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
        egui::Area::new(egui::Id::unique("wt_search_box")).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
            ui.multiply_opacity(k);
            overlay_frame().show(ui, |ui| {
                ui.set_width(w - 28.0);
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                    magnifier(ui.painter(), r.center(), theme::TEXT_MUTED());
                    // Pastilles (mots reconnus) : clic pour retirer.
                    let mut remove = None;
                    let mut entity_i = 0;
                    for (i, p) in self.pills.iter().enumerate() {
                        let (label, fill, stroke) = match p {
                            Pill::Scope(s) => (scope_name(*s).to_string(), theme::overlay(16), theme::BORDER()),
                            Pill::Entity(_, _, name) => {
                                let c = crate::tabs::plannings::planning_color(entity_i);
                                entity_i += 1;
                                (name.clone(), c.gamma_multiply(0.30), c)
                            }
                            Pill::Course(_, name) => (format!("📚 {name}"), theme::overlay(10), theme::TEXT_MUTED()),
                            Pill::Action(a) => (a.label(), theme::TODAY().gamma_multiply(0.25), theme::TODAY()),
                        };
                        let text = egui::text::LayoutJob::simple_singleline(format!("{label}  ✕"), FontId::proportional(13.0), theme::TEXT());
                        if ui
                            .add(egui::Button::new(text).fill(fill).stroke(Stroke::new(1.0, stroke)).corner_radius(CornerRadius::same(10)))
                            .on_hover_text(t!("Retirer", "Remove"))
                            .clicked()
                        {
                            remove = Some(i);
                        }
                    }
                    if let Some(i) = remove {
                        self.pills.remove(i);
                        self.selected = 0;
                    }
                    let was_empty = self.query.is_empty();
                    let hint = if self.pills.is_empty() {
                        t!("Rechercher un mail, une date, un événement, une personne, une tâche…", "Search a mail, a date, an event, a person, a task…")
                    } else {
                        t!("Ajouter un mot…", "Add a word…")
                    };
                    let te =
                        ui.add(egui::TextEdit::singleline(&mut self.query).hint_text(hint).font(FontId::proportional(18.0)).frame(egui::Frame::NONE).desired_width(f32::INFINITY));
                    te.request_focus();
                    if std::mem::take(&mut self.cursor_end)
                        && let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), te.id)
                    {
                        let end = egui::text::CCursor::new(self.query.chars().count());
                        state.cursor.set_char_range(Some(egui::text::CCursorRange::one(end)));
                        state.store(ui.ctx(), te.id);
                    }
                    // Retour arrière dans le champ vide : retire la dernière pastille.
                    if was_empty && ui.input(|i| i.key_pressed(Key::Backspace)) && self.pills.pop().is_some() {
                        self.selected = 0;
                    }
                    if te.changed() {
                        self.selected = 0;
                        self.scroll_top = true;
                        self.edited = Some(Instant::now());
                        self.absorb(hp);
                    }
                });
                ui.add_space(8.0);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));
                ui.add_space(6.0);
                if hits.is_empty() {
                    ui.add_space(14.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new(t!("Aucun résultat", "No results")).color(theme::TEXT_MUTED()));
                        ui.label(
                            RichText::new(t!("Essaie un nom, un sujet, « demain », « 25/10 »…", "Try a name, a subject, “tomorrow”, “25/10”…")).small().color(theme::TEXT_FAINT()),
                        );
                    });
                    ui.add_space(14.0);
                }
                let mut area = egui::ScrollArea::vertical().max_height(screen.height() * 0.55).auto_shrink([false, true]);
                // Texte saisi : retour en haut de la liste (une fois). Sinon la vue reste libre.
                if std::mem::take(&mut self.scroll_top) {
                    area = area.vertical_scroll_offset(0.0);
                }
                area.show(ui, |ui| {
                    let mut group = "";
                    for (i, h) in hits.iter().enumerate() {
                        if h.group != group {
                            group = h.group;
                            ui.add_space(4.0);
                            ui.label(RichText::new(group_label(group).to_uppercase()).small().strong().color(theme::TEXT_FAINT()));
                        }
                        let sel = i == self.selected;
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                            self.selected = i;
                        }
                        let p = ui.painter();
                        if sel {
                            p.rect_filled(r, CornerRadius::same(8), crate::theme::overlay(14));
                        }
                        let icon_r = Rect::from_center_size(Pos2::new(r.left() + 18.0, r.center().y), vec2(20.0, 20.0));
                        match &h.target {
                            // Fichier Drive : pastille de son type, comme dans l'onglet Fichiers.
                            Target::Drive(f) => crate::tabs::files::file_icon(p, icon_r, f),
                            // Personne : sa photo Google Chat si elle existe, sinon l'icône.
                            Target::Person(key)
                                if crate::people::photo_key(people, key, chat).is_some_and(|k| crate::tabs::messages::image_avatar(p, chat, &k, icon_r.center(), 10.0, false)) => {}
                            _ => crate::emoji_img::paint(p, icon_r, h.icon, theme::TEXT()),
                        }
                        let left = r.left() + 40.0;
                        let g = crate::tabs::mail::view::one_line(p, &h.title, theme::editor_font(14.0, sel), theme::TEXT(), r.right() - left - 70.0);
                        p.galley(Pos2::new(left, r.top() + 3.0), g, theme::TEXT());
                        let g = crate::tabs::mail::view::one_line(p, &h.sub, FontId::proportional(11.5), theme::TEXT_MUTED(), r.right() - left - 70.0);
                        p.galley(Pos2::new(left, r.top() + 22.0), g, theme::TEXT_MUTED());
                        if sel {
                            p.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, t!("Entrée ↵", "Enter ↵"), FontId::proportional(11.0), theme::TEXT_FAINT());
                        }
                        if resp.clicked() {
                            go = Some(h.target.clone());
                            picked = Some(h.clone());
                        }
                    }
                });
                ui.add_space(4.0);
                let hint = match scope {
                    Some(s) => tf!("Catégorie : {} · ↑ ↓ naviguer · Entrée ouvrir · Échap fermer", "Category: {} · ↑ ↓ navigate · Enter open · Esc close", scope_name(s)),
                    None => t!(
                        "↑ ↓ naviguer · Entrée ouvrir · Échap fermer · catégorie : hpp, cal, msg, mail, drive, sheet, doc, note + recherche",
                        "↑ ↓ navigate · Enter open · Esc close · category: hpp, cal, msg, mail, drive, sheet, doc, note + search"
                    )
                    .to_string(),
                };
                ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT()));
            });
        });
        // La boîte reste toujours au-dessus du fond : un clic sur le fond le fait passer au premier
        // plan de son niveau (comportement d'egui), et il recouvrait alors la boîte à la
        // réouverture (seul le flou s'affichait).
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("wt_search_box")));
        // Élément HyperPlanning proposé (pas encore en pastille) : devient une pastille, la
        // recherche reste ouverte (la pastille elle-même, dans « Sélection », ouvre le planning).
        if let Some(h) = &picked
            && h.group == "HyperPlanning"
            && let Target::Planning(kind, k) = h.target
        {
            let pill = Pill::Entity(kind, k, h.title.clone());
            if !self.pills.contains(&pill) {
                self.pills.push(pill);
            }
            self.query.clear();
            self.selected = 0;
            self.cursor_end = true;
            return None;
        }
        // Cours choisi après une pastille de groupe : pastille du nom de cours, pour voir
        // toutes ses séances en liste (une séance de cette liste ouvre ensuite le planning).
        if let Some(h) = &picked
            && h.group == "Cours HPP"
            && self.pills.iter().any(|p| matches!(p, Pill::Entity(..)))
            && !self.pills.iter().any(|p| matches!(p, Pill::Course(..)))
        {
            self.pills.push(Pill::Course(course_key(&h.title), h.title.clone()));
            self.query.clear();
            self.selected = 0;
            self.cursor_end = true;
            return None;
        }
        if go.is_some() || close {
            self.close();
        }
        // Fiche contact : s'ouvre en overlay à la place de la recherche.
        if let Some(Target::Person(key)) = &go {
            crate::people::open(ctx, key, "");
            return None;
        }
        go
    }
}

fn magnifier(p: &egui::Painter, c: Pos2, color: Color32) {
    p.circle_stroke(c + vec2(-2.0, -2.0), 6.0, Stroke::new(1.8, color));
    p.line_segment([c + vec2(2.5, 2.5), c + vec2(7.0, 7.0)], Stroke::new(2.0, color));
}

// --- Recherche ---------------------------------------------------------------------------

/// Score de correspondance : tous les mots de la requête doivent être présents.
fn score(q: &[String], fields: &[&str]) -> Option<i32> {
    let hay: Vec<String> = fields.iter().map(|f| fold(f)).collect();
    let mut s = 0;
    for w in q {
        let mut best = None;
        for (i, h) in hay.iter().enumerate() {
            if let Some(pos) = h.find(w.as_str()) {
                let starts = pos == 0 || !h[..pos].ends_with(char::is_alphanumeric);
                let v = (if i == 0 { 20 } else { 8 }) + if starts { 10 } else { 0 };
                best = best.max(Some(v));
            }
        }
        s += best?;
    }
    Some(s)
}

/// Nom affiché d'un groupe de résultats (les groupes sont repérés par leur nom français).
fn group_label(group: &'static str) -> &'static str {
    if !crate::i18n::en() {
        return group;
    }
    match group {
        "Fiches contact" => "Contact cards",
        "Calendrier" => "Calendar",
        "Tâches" => "Tasks",
        "Fichiers" => "Files",
        "À venir" => "Upcoming",
        "Cours HPP" => "HPP classes",
        "Raccourci" => "Shortcut",
        "Sélection" => "Selection",
        "Séances" => "Sessions",
        other => other,
    }
}

fn date_label(d: NaiveDate) -> String {
    let today = Local::now().date_naive();
    let rel = if d == today {
        t!(" · aujourd'hui", " · today")
    } else if d == today + CDuration::days(1) {
        t!(" · demain", " · tomorrow")
    } else if d == today - CDuration::days(1) {
        t!(" · hier", " · yesterday")
    } else {
        ""
    };
    format!("{} {} {} {}{rel}", crate::tabs::weekday_name(d.weekday()), d.day(), crate::tabs::month_name(d.month()), d.year())
}

/// Dates reconnues (en français et en anglais) : aujourd'hui/today, demain/tomorrow, hier/yesterday,
/// lundi…/monday…, 25/10, 25/10/2026, 25 octobre (2026), october 25 (2026).
pub fn parse_date(q: &str) -> Option<NaiveDate> {
    let q = fold(q.trim());
    let today = Local::now().date_naive();
    match q.as_str() {
        "aujourd'hui" | "aujourdhui" | "auj" | "today" => return Some(today),
        "demain" | "tomorrow" => return Some(today + CDuration::days(1)),
        "apres-demain" | "apres demain" => return Some(today + CDuration::days(2)),
        "hier" | "yesterday" => return Some(today - CDuration::days(1)),
        _ => {}
    }
    let day = |names: [&str; 7]| names.iter().position(|d| fold(d) == q || (q.len() >= 4 && fold(d).starts_with(q.as_str())));
    if let Some(i) = day(crate::tabs::WEEKDAYS_FR).or_else(|| day(crate::tabs::WEEKDAYS_EN)) {
        let cur = today.weekday().num_days_from_monday() as i64;
        let delta = (i as i64 - cur).rem_euclid(7);
        return Some(today + CDuration::days(if delta == 0 { 7 } else { delta }));
    }
    // 25/10 ou 25/10/2026 (ou avec - ou .)
    let nums: Vec<&str> = q.split(['/', '-', '.']).collect();
    if (2..=3).contains(&nums.len()) && nums.iter().all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
        let d: u32 = nums[0].parse().ok()?;
        let m: u32 = nums[1].parse().ok()?;
        let y: i32 = match nums.get(2) {
            Some(y) if y.len() == 2 => 2000 + y.parse::<i32>().ok()?,
            Some(y) => y.parse().ok()?,
            None => today.year(),
        };
        return NaiveDate::from_ymd_opt(y, m, d);
    }
    // 25 octobre (2026), 25 october, october 25
    let parts: Vec<&str> = q.split_whitespace().map(|p| p.trim_end_matches(',')).collect();
    if (2..=3).contains(&parts.len()) {
        let num = |p: &str| p.trim_end_matches("er").trim_end_matches(['s', 't', 'n', 'd', 'r', 'h']).parse::<u32>().ok();
        let (d, name) = match num(parts[0]) {
            Some(d) => (d, parts[1]),
            None => (num(parts[1])?, parts[0]),
        };
        let month_of = |names: [&str; 12]| names.iter().position(|m| name.len() >= 3 && fold(m).starts_with(name)).map(|i| i as u32 + 1);
        let month = month_of(crate::tabs::MONTHS_FR).or_else(|| month_of(crate::tabs::MONTHS_EN))?;
        let y: i32 = parts.get(2).and_then(|y| y.parse().ok()).unwrap_or(today.year());
        return NaiveDate::from_ymd_opt(y, month, d);
    }
    None
}

fn hhmm(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

fn event_sub(e: &CalEvent) -> String {
    let d = Local.timestamp_millis_opt(e.start).single().map(|d| d.date_naive()).unwrap_or_default();
    let time = if e.all_day { t!("journée", "all day").to_string() } else { format!("{} – {}", hhmm(e.start), hhmm(e.end)) };
    let mut s = format!("{} · {time}", date_label(d));
    if !e.location.is_empty() {
        s.push_str(&format!(" · {}", e.location));
    }
    s
}

/// Sans requête : prochains événements et commandes rapides.
fn suggestions(cal: &CalendarState, prefs: &Prefs) -> Vec<Hit> {
    let now = Local::now().timestamp_millis();
    let mut out = Vec::new();
    let mut next: Vec<&CalEvent> = cal.events.values().flatten().filter(|e| !prefs.hidden_calendars.contains(&e.calendar) && e.end > now).collect();
    next.sort_by_key(|e| e.start);
    next.dedup_by_key(|e| e.id.clone());
    for e in next.into_iter().take(3) {
        out.push(Hit { group: "À venir", icon: "📅", title: e.title.clone(), sub: event_sub(e), target: Target::Event(e.clone()), score: 0 });
    }
    out.extend(commands());
    out
}

fn commands() -> Vec<Hit> {
    let c = |icon, title: &str, sub: &str, target| Hit { group: "Actions", icon, title: title.into(), sub: sub.into(), target, score: 0 };
    vec![
        c("✉️", t!("Nouveau message", "New message"), t!("Mail · écrire un mail", "Mail · write a mail"), Target::Compose),
        c("🗓️", t!("Nouvel événement", "New event"), t!("Calendrier · créer un rendez-vous", "Calendar · create an event"), Target::NewEvent),
        c("📝", t!("Nouvelle note", "New note"), t!("Prise de notes · créer une note", "Notes · create a note"), Target::NewNote),
        c("🏠", t!("Tableau de bord", "Dashboard"), t!("Aller à l'onglet", "Go to tab"), Target::Tab(Tab::Dashboard)),
        c("📆", t!("Calendrier", "Calendar"), t!("Aller à l'onglet", "Go to tab"), Target::Tab(Tab::Calendar)),
        c("📬", "Mail", t!("Aller à l'onglet", "Go to tab"), Target::Tab(Tab::Mail)),
        c("💬", "Messages", "Google Chat", Target::Tab(Tab::Messages)),
        c("📝", t!("Prise de notes", "Notes"), t!("Aller à l'onglet", "Go to tab"), Target::Tab(Tab::Notes)),
        c("📁", t!("Fichiers", "Files"), t!("Aller à l'onglet", "Go to tab"), Target::Tab(Tab::Files)),
        c("🗓️", "HyperPlanning", t!("HPP · emplois du temps de l'école", "HPP · school timetables"), Target::Tab(Tab::Plannings)),
        c("🧩", "Blueprint", t!("Automatisations · graphs de nodes", "Automations · node graphs"), Target::Tab(Tab::Blueprint)),
        c("⚙️", t!("Réglages", "Settings"), t!("Compte Google, animations…", "Google account, animations…"), Target::Tab(Tab::Settings)),
    ]
}

#[allow(clippy::too_many_arguments)]
fn search(
    q: &str,
    scope: Option<Scope>,
    pills: &[Pill],
    mail: &MailState,
    cal: &CalendarState,
    chat: &ChatState,
    tasks: &Tasks,
    notes: &crate::tabs::notes::NotesState,
    people: &crate::people::People,
    prefs: &Prefs,
    remote: &[Summary],
    drive: &[DriveFile],
    hp: Option<&crate::hyperplanning::cache::Snapshot>,
) -> Vec<Hit> {
    let words: Vec<String> = fold(q).split_whitespace().map(str::to_string).collect();
    let mut out: Vec<Hit> = Vec::new();
    let push = |out: &mut Vec<Hit>, h: Hit| out.push(h);

    // Date du calendrier.
    if let Some(d) = parse_date(q) {
        push(
            &mut out,
            Hit {
                group: "Date",
                icon: "📆",
                title: date_label(d),
                sub: t!("Ouvrir ce jour dans le calendrier", "Open this day in the calendar").into(),
                target: Target::Date(d),
                score: 100,
            },
        );
    }

    // Commandes.
    for mut h in commands() {
        if let Some(s) = score(&words, &[&h.title, &h.sub]) {
            h.score = s;
            push(&mut out, h);
        }
    }

    // Mails chargés (boîte, favoris, épinglés) puis résultats Gmail.
    let mut seen = std::collections::HashSet::new();
    let local = mail.messages.iter().chain(mail.starred.iter()).chain(mail.pinned.list.iter());
    for m in local.chain(remote.iter()) {
        if !seen.insert(m.id.clone()) {
            continue;
        }
        let from_remote = remote.iter().any(|r| r.id == m.id);
        let s = score(&words, &[&m.subject, m.sender_name(), &m.from, &m.snippet]).or(from_remote.then_some(1));
        if let Some(s) = s {
            let subject = if m.subject.is_empty() { t!("(sans objet)", "(no subject)").to_string() } else { m.subject.clone() };
            push(
                &mut out,
                Hit {
                    group: "Mails",
                    icon: if m.unread() { "📩" } else { "✉️" },
                    title: subject,
                    sub: format!("{} · {}", m.sender_name(), crate::tabs::mail::view::short_date(m.date)),
                    target: Target::Mail(m.id.clone()),
                    score: s + (m.date / 86_400_000 / 30) as i32 % 3,
                },
            );
        }
    }

    // Fiches contact (annuaire caché + annuaire des mails) : proposées en premier.
    for f in people.search(q, &mail.contacts, chat, 4) {
        let favorite = mail.contacts.get(&f.key).is_some_and(|c| c.favorite);
        push(&mut out, Hit { group: "Fiches contact", icon: if favorite { "⭐" } else { "👤" }, title: f.name, sub: f.sub, target: Target::Person(f.key), score: f.score });
    }

    // Événements (agendas affichés).
    let mut evs: Vec<&CalEvent> = cal.events.values().flatten().filter(|e| !prefs.hidden_calendars.contains(&e.calendar)).collect();
    evs.sort_by_key(|e| e.start);
    evs.dedup_by_key(|e| e.id.clone());
    let now = Local::now().timestamp_millis();
    for e in evs {
        if let Some(s) = score(&words, &[&e.title, &e.location, &e.description]) {
            // Les prochains d'abord.
            let bonus = if e.end >= now { 10 } else { 0 };
            push(
                &mut out,
                Hit {
                    group: "Calendrier",
                    icon: "📅",
                    title: if e.title.is_empty() { t!("(Sans titre)", "(Untitled)").into() } else { e.title.clone() },
                    sub: event_sub(e),
                    target: Target::Event(e.clone()),
                    score: s + bonus,
                },
            );
        }
    }

    // Google Chat : conversations et messages chargés.
    for sp in &chat.spaces {
        let title = chat.title(sp);
        if let Some(s) = score(&words, &[&title]) {
            push(&mut out, Hit { group: "Google Chat", icon: "💬", title, sub: "Conversation".into(), target: Target::Space(sp.name.clone()), score: s + 5 });
        }
    }
    for (space, list) in &chat.msgs {
        for m in list.iter().rev().take(200) {
            if let Some(s) = score(&words, &[&m.text]) {
                let sp_title = chat.space(space).map(|x| chat.title(x)).unwrap_or_default();
                push(
                    &mut out,
                    Hit {
                        group: "Google Chat",
                        icon: "💬",
                        title: m.text.lines().next().unwrap_or_default().chars().take(90).collect(),
                        sub: format!("{} · {} · {}", chat.name_of(&m.sender), sp_title, crate::tabs::mail::view::short_date(m.created)),
                        target: Target::Space(space.clone()),
                        score: s,
                    },
                );
            }
        }
    }

    // Tâches (y compris archives).
    for t in tasks.list.iter().chain(tasks.archive.iter()) {
        if let Some(s) = score(&words, &[&t.title, &t.summary]) {
            let archived = tasks.archive.iter().any(|a| a.id == t.id);
            let mut sub = if archived { t!("Tâche archivée", "Archived task").to_string() } else { tf!("Tâche · {}", "Task · {}", t.status.label()) };
            if let Some(first) = t.summary.lines().find(|l| !l.trim().is_empty()) {
                sub.push_str(&format!(" · {}", first.trim()));
            }
            push(&mut out, Hit { group: "Tâches", icon: "✅", title: t.title.clone(), sub, target: Target::Task, score: s });
        }
    }

    // Notes (texte des notes protégées non consulté).
    for (id, title, text, updated, protected) in notes.entries() {
        let s = score(&words, &[&title]).map(|s| s + 10).or_else(|| score(&words, &[&text]));
        if let Some(s) = s {
            let sub = if protected { t!("Note protégée", "Protected note").to_string() } else { text.chars().take(80).collect::<String>() };
            let sub = format!("{} · {sub}", crate::tabs::mail::view::short_date(updated));
            push(&mut out, Hit { group: "Notes", icon: if protected { "🔒" } else { "📝" }, title, sub, target: Target::Note(id), score: s });
        }
    }

    // Fichiers partagés.
    for (m, f) in &mail.shared_files {
        if let Some(s) = score(&words, &[&f.title, m.sender_name()]) {
            push(
                &mut out,
                Hit {
                    group: "Fichiers",
                    icon: "📄",
                    title: f.title.clone(),
                    sub: format!("{} · {}", m.sender_name(), crate::tabs::mail::view::short_date(m.date)),
                    target: Target::File(f.clone()),
                    score: s,
                },
            );
        }
    }

    // Google Drive (recherche côté serveur : nom ou contenu).
    for f in drive {
        let s = score(&words, &[&f.name]).unwrap_or(1);
        let icon = if f.is_folder() { "📁" } else { "📄" };
        let sub = format!("Drive · {}{}", crate::google::drive::kind_label(f), if f.owner.is_empty() { String::new() } else { format!(" · {}", f.owner) });
        push(&mut out, Hit { group: "Drive", icon, title: f.name.clone(), sub, target: Target::Drive(f.clone()), score: s });
    }

    // Actions des pastilles : en tête.
    for p in pills {
        if let Pill::Action(a) = p {
            push(&mut out, Hit { group: "Raccourci", icon: a.icon(), title: a.label(), sub: t!("Action", "Action").into(), target: a.target(), score: 1000 });
        }
    }
    // Pastilles d'action seules (sans texte ni catégorie) : rien d'autre.
    if words.is_empty() && scope.is_none() {
        out.retain(|h| h.group == "Raccourci");
        return out;
    }

    // Plannings HyperPlanning (copie locale) : enseignants, salles, promotions, TD, regroupements.
    // Le type compte comme un mot cherché (« salle 101 »).
    let must: Vec<(crate::tabs::plannings::Kind, u32)> = pills.iter().filter_map(|p| if let Pill::Entity(kind, k, _) = p { Some((*kind, *k)) } else { None }).collect();
    let course_pills: Vec<String> = pills.iter().filter_map(|p| if let Pill::Course(key, _) = p { Some(key.clone()) } else { None }).collect();
    if let Some(d) = hp {
        use crate::tabs::plannings::Kind;
        // Pastilles d'éléments : chacun, puis tous ensemble (une salle reste seule).
        let ents: Vec<(Kind, u32, String)> = pills.iter().filter_map(|p| if let Pill::Entity(kind, k, n) = p { Some((*kind, *k, n.clone())) } else { None }).collect();
        let group: Vec<&(Kind, u32, String)> = ents.iter().filter(|(kind, ..)| *kind != Kind::Salle).collect();
        if group.len() >= 2 {
            let title = tf!("Ouvrir ensemble : {}", "Open together: {}", group.iter().map(|(_, _, n)| n.as_str()).collect::<Vec<_>>().join(" + "));
            let sel = group.iter().map(|(kind, k, _)| (*kind, *k)).collect();
            push(
                &mut out,
                Hit {
                    group: "Sélection", icon: "🗓️", title, sub: t!("HPP · plannings combinés", "HPP · combined timetables").into(), target: Target::Plannings(sel), score: 1000
                },
            );
        }
        for (kind, k, name) in &ents {
            push(
                &mut out,
                Hit {
                    group: "Sélection", icon: "🗓️", title: name.clone(), sub: tf!("HPP · {}", "HPP · {}", kind.singular()), target: Target::Planning(*kind, *k), score: 900
                },
            );
        }
        // Éléments rattachés aux pastilles (présents dans leurs séances communes) : proposés en
        // premier, les plus fréquents en tête.
        let mut related: std::collections::HashMap<(u8, u32), i32> = std::collections::HashMap::new();
        if !must.is_empty() {
            for s in d.cours.values().flat_map(|c| &c.seances) {
                if !must.iter().all(|(kind, k)| crate::tabs::plannings::involves(d, s, *kind, *k)) {
                    continue;
                }
                let all = [
                    (Kind::Enseignant, &s.enseignants),
                    (Kind::Salle, &s.salles),
                    (Kind::Promotion, &s.promotions),
                    (Kind::TDOption, &s.tdoptions),
                    (Kind::Regroupement, &s.regroupements),
                ];
                for (kind, ks) in all {
                    for k in ks.iter() {
                        if !must.contains(&(kind, *k)) {
                            *related.entry((kind as u8, *k)).or_default() += 1;
                        }
                    }
                }
            }
        }
        for kind in Kind::ALL {
            for (k, name) in crate::tabs::plannings::entries(d, kind) {
                let rel = related.get(&(kind as u8, k)).copied();
                // Avec des pastilles et sans texte : seulement les éléments rattachés.
                if (!ents.is_empty() && words.is_empty() && rel.is_none()) || (!course_pills.is_empty() && words.is_empty()) {
                    continue;
                }
                let Some(s) = score(&words, &[&name, kind.keywords()]) else { continue };
                let s = s + rel.map(|n| 500 + n.min(400)).unwrap_or(0);
                let mut sub = tf!("HPP · {}", "HPP · {}", kind.singular());
                if kind == Kind::Salle
                    && let Some(c) = d.salles.get(&k).map(|x| x.capacity).filter(|c| *c > 0)
                {
                    sub.push_str(&tf!(" · {c} places", " · {c} seats"));
                }
                if ents.iter().any(|(kk, x, _)| *kk == kind && *x == k) {
                    continue;
                }
                push(&mut out, Hit { group: "HyperPlanning", icon: "🗓️", title: name, sub, target: Target::Planning(kind, k), score: s });
            }
        }
        if course_pills.is_empty() {
            seances(d, &words, &must, &mut out);
        } else {
            course_list(d, &words, &must, &course_pills, &mut out);
        }
    }

    // Catégorie demandée (« mail … », « hpp … ») : ses résultats seulement.
    if let Some(scope) = scope {
        out.retain(|h| h.group == "Raccourci" || in_scope(h, scope));
    }

    // Tri : par groupe (ordre ci-dessous), puis pertinence ; 6 résultats par groupe.
    // Priorité : contacts, agenda (événements puis dates), mails, chat, Drive, notes, puis le reste.
    // Avec des pastilles HPP : leurs cours avant les autres éléments HyperPlanning.
    let hpp_first = if must.is_empty() { ["HyperPlanning", "Cours HPP"] } else { ["Cours HPP", "HyperPlanning"] };
    let order: [&str; 16] = [
        "Raccourci",
        "Sélection",
        "Séances",
        "Fiches contact",
        hpp_first[0],
        hpp_first[1],
        "Calendrier",
        "Date",
        "Mails",
        "Google Chat",
        "Drive",
        "Notes",
        "Tâches",
        "Fichiers",
        "Actions",
        "À venir",
    ];
    out.sort_by_key(|h| (order.iter().position(|g| *g == h.group).unwrap_or(99), -h.score));
    let mut counts = std::collections::HashMap::new();
    out.retain(|h| {
        let n = counts.entry(h.group).or_insert(0);
        *n += 1;
        // Liste de toutes les séances d'un cours : complète (défilement).
        if h.group == "Séances" {
            return true;
        }
        // Avec des pastilles HPP : 6 cours, pour voir aussi les éléments rattachés dessous.
        *n <= if h.group == "Actions" || (h.group == "Cours HPP" && !must.is_empty()) {
            if h.group == "Actions" { 3 } else { 6 }
        } else if scope.is_some() {
            20
        } else {
            6
        }
    });
    out
}

/// Nom de cours comparable (replié, espaces simples) : « même nom probable ».
fn course_key(name: &str) -> String {
    fold(name).split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Toutes les séances des cours nommés (pastilles de cours), filtrées par les pastilles de
/// groupe et le texte : à venir d'abord (la plus proche en tête), puis passées (la plus récente
/// en tête).
fn course_list(d: &crate::hyperplanning::cache::Snapshot, words: &[String], must: &[(crate::tabs::plannings::Kind, u32)], courses: &[String], out: &mut Vec<Hit>) {
    use crate::tabs::plannings::{Kind, end_of, hhmm, involves, matiere, name_of};
    let now = Local::now().naive_local();
    for (cours, c) in &d.cours {
        for s in &c.seances {
            let title = matiere(d, s);
            if !courses.contains(&course_key(&title)) || !must.iter().all(|(kind, k)| involves(d, s, *kind, *k)) {
                continue;
            }
            let names: Vec<String> =
                [(Kind::Enseignant, &s.enseignants), (Kind::Salle, &s.salles)].iter().flat_map(|(kind, ks)| ks.iter().map(|k| name_of(d, *kind, *k))).collect();
            let mut fields: Vec<&str> = vec![&title, &s.type_cours];
            fields.extend(names.iter().map(String::as_str));
            if score(words, &fields).is_none() {
                continue;
            }
            let past = end_of(s) < now;
            let mut sub = format!("{}{} · {} – {}", if past { t!("Passée · ", "Past · ") } else { "" }, date_label(s.start.date()), hhmm(s.start), hhmm(end_of(s)));
            for n in &names {
                sub.push_str(&format!(" · {n}"));
            }
            let mins = (s.start - now).num_minutes().clamp(-100_000_000, 100_000_000) as i32;
            let score = if past { -1_000_000_000 + mins } else { 1_000_000_000 - mins };
            let sel = if must.is_empty() {
                s.promotions.first().map(|k| vec![(Kind::Promotion, *k)]).or_else(|| s.enseignants.first().map(|k| vec![(Kind::Enseignant, *k)])).unwrap_or_default()
            } else {
                must.to_vec()
            };
            if sel.is_empty() {
                continue;
            }
            out.push(Hit { group: "Séances", icon: if past { "🕘" } else { "📚" }, title, sub, target: Target::Seance { sel, cours: *cours, start: s.start }, score });
        }
    }
}

/// Séances HyperPlanning (matière, enseignants, salles, groupes) : pour chaque cours, sa prochaine
/// séance (sinon la dernière). Ouvre le planning de l'élément cherché (sinon du premier
/// enseignant, groupe ou salle) au jour de la séance. `must` (pastilles) : seulement les
/// séances communes à tous ces éléments, ouvertes avec leurs plannings ensemble.
fn seances(d: &crate::hyperplanning::cache::Snapshot, words: &[String], must: &[(crate::tabs::plannings::Kind, u32)], out: &mut Vec<Hit>) {
    use crate::tabs::plannings::{Kind, end_of, hhmm, involves, matiere, name_of};
    let now = Local::now().naive_local();
    // Avec des pastilles : un résultat par nom de cours (sa séance la plus proche) ; le choisir
    // liste toutes ses séances.
    let mut by_name: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for (cours, c) in &d.cours {
        let ok = |s: &&crate::hyperplanning::cache::Seance| must.iter().all(|(kind, k)| involves(d, s, *kind, *k));
        let Some(s) = c.seances.iter().filter(ok).find(|s| end_of(s) >= now).or(c.seances.iter().filter(ok).last()) else { continue };
        let title = matiere(d, s);
        let groups: [(Kind, &[u32]); 5] = [
            (Kind::Enseignant, &s.enseignants),
            (Kind::Promotion, &s.promotions),
            (Kind::TDOption, &s.tdoptions),
            (Kind::Regroupement, &s.regroupements),
            (Kind::Salle, &s.salles),
        ];
        let named: Vec<(Kind, u32, String)> = groups.iter().flat_map(|(kind, ks)| ks.iter().map(|k| (*kind, *k, name_of(d, *kind, *k)))).collect();
        let mut fields: Vec<&str> = vec![&title, &s.type_cours];
        fields.extend(named.iter().map(|(_, _, n)| n.as_str()));
        let Some(score) = score(words, &fields) else { continue };
        // Planning à ouvrir : un élément dont le nom correspond à la recherche, sinon le premier.
        let hit = named.iter().find(|(_, _, n)| words.iter().any(|w| fold(n).contains(w.as_str()))).or(named.first());
        let Some((kind, key, _)) = hit else { continue };
        let sel = if must.is_empty() { vec![(*kind, *key)] } else { must.to_vec() };
        let mut sub = format!("HPP · {} · {} – {}", date_label(s.start.date()), hhmm(s.start), hhmm(end_of(s)));
        for (kind, ks) in [(Kind::Salle, &s.salles), (Kind::Enseignant, &s.enseignants)] {
            if let Some(k) = ks.first() {
                sub.push_str(&format!(" · {}", name_of(d, kind, *k)));
            }
        }
        // Les prochaines d'abord, les plus proches en tête.
        let days = (s.start.date() - now.date()).num_days();
        let bonus = if days >= 0 { 30 - days.min(29) as i32 } else { 0 };
        let hit = Hit { group: "Cours HPP", icon: "📚", title, sub, target: Target::Seance { sel, cours: *cours, start: s.start }, score: score + bonus };
        if must.is_empty() {
            out.push(hit);
            continue;
        }
        *counts.entry(course_key(&hit.title)).or_insert(0) += c.seances.iter().filter(ok).count();
        match by_name.get(&course_key(&hit.title)) {
            Some(&i) => {
                if hit.score > out[i].score {
                    out[i] = hit;
                }
            }
            None => {
                by_name.insert(course_key(&hit.title), out.len());
                out.push(hit);
            }
        }
    }
    // Nombre de séances de chaque nom (clic : la liste complète).
    for (key, i) in by_name {
        if let Some(n) = counts.get(&key).filter(|n| **n > 1) {
            let rest = out[i].sub.trim_start_matches("HPP · ").to_string();
            out[i].sub = tf!("HPP · {n} séances (clic : toutes) · {rest}", "HPP · {n} sessions (click: all) · {rest}");
        }
    }
}

// --- Flou ----------------------------------------------------------------------------------

/// Flou approchant un flou gaussien : réduction d'un facteur `down`, puis `passes` flous boîte.
pub fn blur(img: &ColorImage, down: usize, passes: usize) -> ColorImage {
    let (w, h) = (img.size[0], img.size[1]);
    let (sw, sh) = ((w / down).max(1), (h / down).max(1));
    let mut px: Vec<[f32; 3]> = vec![[0.0; 3]; sw * sh];
    for y in 0..sh {
        for x in 0..sw {
            let mut acc = [0.0f32; 3];
            let mut n = 0.0;
            for dy in 0..down {
                for dx in 0..down {
                    let (ix, iy) = (x * down + dx, y * down + dy);
                    if ix < w && iy < h {
                        let c = img.pixels[iy * w + ix];
                        acc[0] += c.r() as f32;
                        acc[1] += c.g() as f32;
                        acc[2] += c.b() as f32;
                        n += 1.0;
                    }
                }
            }
            px[y * sw + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
        }
    }
    let r = 3i64;
    for _ in 0..passes {
        let mut tmp = px.clone();
        for y in 0..sh {
            for x in 0..sw {
                let mut acc = [0.0f32; 3];
                let mut n = 0.0;
                for k in -r..=r {
                    let xx = (x as i64 + k).clamp(0, sw as i64 - 1) as usize;
                    let c = px[y * sw + xx];
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    n += 1.0;
                }
                tmp[y * sw + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
            }
        }
        for y in 0..sh {
            for x in 0..sw {
                let mut acc = [0.0f32; 3];
                let mut n = 0.0;
                for k in -r..=r {
                    let yy = (y as i64 + k).clamp(0, sh as i64 - 1) as usize;
                    let c = tmp[yy * sw + x];
                    acc[0] += c[0];
                    acc[1] += c[1];
                    acc[2] += c[2];
                    n += 1.0;
                }
                px[y * sw + x] = [acc[0] / n, acc[1] / n, acc[2] / n];
            }
        }
    }
    let rgba: Vec<u8> = px.iter().flat_map(|c| [c[0] as u8, c[1] as u8, c[2] as u8, 255]).collect();
    ColorImage::from_rgba_unmultiplied([sw, sh], &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_understood() {
        let today = Local::now().date_naive();
        assert_eq!(parse_date("demain"), Some(today + CDuration::days(1)));
        assert_eq!(parse_date("25/10/2026"), NaiveDate::from_ymd_opt(2026, 10, 25));
        assert_eq!(parse_date("3 novembre 2026"), NaiveDate::from_ymd_opt(2026, 11, 3));
        assert_eq!(parse_date("1er déc 2026"), NaiveDate::from_ymd_opt(2026, 12, 1));
        assert!(parse_date("rendu projet").is_none());
        assert!(parse_date("lundi").is_some_and(|d| d.weekday() == chrono::Weekday::Mon && d > today));
        // En anglais aussi.
        assert_eq!(parse_date("tomorrow"), Some(today + CDuration::days(1)));
        assert_eq!(parse_date("October 25, 2026"), NaiveDate::from_ymd_opt(2026, 10, 25));
        assert_eq!(parse_date("3rd november 2026"), NaiveDate::from_ymd_opt(2026, 11, 3));
        assert!(parse_date("friday").is_some_and(|d| d.weekday() == chrono::Weekday::Fri && d > today));
    }

    #[test]
    fn scoring_needs_every_word() {
        let q: Vec<String> = vec!["rendu".into(), "gmtk".into()];
        assert!(score(&q, &["Rendu du projet GMTK"]).is_some());
        assert!(score(&q, &["Rendu du projet"]).is_none());
        assert!(score(&["lea".into()], &["Léa Martin"]).is_some());
    }

    #[test]
    fn blur_keeps_size_ratio() {
        let img = ColorImage::new([40, 20], vec![Color32::WHITE; 800]);
        let b = blur(&img, 4, 2);
        assert_eq!(b.size, [10, 5]);
    }
}
