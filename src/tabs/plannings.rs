//! Onglet Plannings : emplois du temps HyperPlanning (enseignants, salles, promotions, TD/options,
//! regroupements), lus dans la copie locale — aucune requête au serveur. Même forme et mêmes
//! déplacements que l'onglet Calendrier.
//!
//! - Colonne de gauche masquable (S) : mini-calendrier, type de planning, filtre, liste (salles
//!   et promotions rangées par catégories).
//! - Vues Jour, 3 jours, Semaine (`grid`) et Mois (`month`) ; raccourcis T, J/K/←/→, D/W/M, 1–7.
//! - Clic sur une séance : détail à droite ; clic sur un nom du détail : planning de cet élément.
//! - Planning d'une salle : cliquer-glisser sur la grille pour demander le créneau (voir `book`).

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime};
use eframe::egui::{self, Align, Align2, CornerRadius, FontId, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, vec2};

use crate::hyperplanning::HyperPlanning;
use crate::hyperplanning::cache::{Seance, Snapshot};
use crate::{theme, widgets};

mod book;
mod grid;
mod month;

const SIDE_W: f32 = 250.0;
const PANEL_W: f32 = 320.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    #[default]
    Enseignant,
    Salle,
    Promotion,
    TDOption,
    Regroupement,
}

impl Kind {
    pub const ALL: [Kind; 5] = [Kind::Enseignant, Kind::Salle, Kind::Promotion, Kind::TDOption, Kind::Regroupement];

    fn from_u8(i: u8) -> Option<Kind> {
        Kind::ALL.get(i as usize).copied()
    }

    /// Nom au singulier (résultats de la recherche).
    pub fn singular(self) -> &'static str {
        match self {
            Kind::Enseignant => t!("Enseignant", "Teacher"),
            Kind::Salle => t!("Salle", "Room"),
            Kind::Promotion => "Promotion",
            Kind::TDOption => t!("TD / option", "Group / option"),
            Kind::Regroupement => t!("Regroupement", "Cluster"),
        }
    }

    /// Mots qui désignent ce type dans la recherche (« salle 101 », « prof dupont »).
    pub fn keywords(self) -> &'static str {
        match self {
            Kind::Enseignant => "enseignant prof professeur intervenant teacher",
            Kind::Salle => "salle room",
            Kind::Promotion => "promotion promo classe class",
            Kind::TDOption => "td option groupe group",
            Kind::Regroupement => "regroupement cluster",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Kind::Enseignant => t!("Enseignants", "Teachers"),
            Kind::Salle => t!("Salles", "Rooms"),
            Kind::Promotion => t!("Promos", "Classes"),
            Kind::TDOption => t!("TD", "Groups"),
            Kind::Regroupement => t!("Regroup.", "Clusters"),
        }
    }
}

/// Vue affichée (comme l'onglet Calendrier).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum View {
    /// N jours à partir de `anchor` (7 : semaine du lundi au dimanche).
    Days(u8),
    Month,
}

/// Séances de l'élément affiché sur la période affichée (recalculées quand l'un change).
/// Onglet de la colonne de gauche.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum SideTab {
    Favorites,
    #[default]
    Directory,
}

/// Favoris (préférences) : (type, clé).
type Favs = Vec<(u8, u32)>;

fn is_fav(favs: &Favs, kind: Kind, k: u32) -> bool {
    favs.contains(&(kind as u8, k))
}

fn toggle_fav(favs: &mut Favs, kind: Kind, k: u32) {
    let e = (kind as u8, k);
    if favs.contains(&e) {
        favs.retain(|x| *x != e);
    } else {
        favs.push(e);
    }
}

#[derive(Default)]
struct RangeCache {
    key: Option<((Kind, u32), NaiveDate, NaiveDate, i64)>,
    seances: Vec<(u32, Seance)>,
}

pub struct PlanningsState {
    /// Panneau de demande de réservation.
    book: book::BookPanel,
    /// Sélection en cours sur la grille : (jour, minute d'ancrage, minute courante).
    drag: Option<(NaiveDate, u32, u32)>,
    /// Message bref sous la barre (créneau occupé…).
    notice: Option<(String, std::time::Instant)>,
    kind: Kind,
    side_tab: SideTab,
    /// Onglet de gauche choisi au premier affichage (Favoris s'il y en a).
    side_tab_set: bool,
    /// Panneau des types (Enseignants, Salles…) déplié.
    kind_open: bool,
    filter: String,
    selected: Option<(Kind, u32)>,
    view: View,
    /// Premier jour affiché (lundi en vue Semaine, 1er du mois en vue Mois).
    anchor: NaiveDate,
    side_hidden: bool,
    /// Séance ouverte dans le détail : (cours, début) ; la dernière reste affichée pendant la
    /// fermeture animée du panneau.
    detail: Option<(u32, NaiveDateTime)>,
    detail_last: Option<(u32, NaiveDateTime)>,
    scroll_to_morning: bool,
    cache: RangeCache,
}

impl Default for PlanningsState {
    fn default() -> Self {
        let today = Local::now().date_naive();
        Self {
            book: Default::default(),
            drag: None,
            notice: None,
            kind: Kind::default(),
            side_tab: SideTab::default(),
            side_tab_set: false,
            kind_open: false,
            filter: String::new(),
            selected: None,
            view: View::Days(7),
            anchor: monday_of(today),
            side_hidden: false,
            detail: None,
            detail_last: None,
            scroll_to_morning: true,
            cache: RangeCache::default(),
        }
    }
}

impl PlanningsState {
    /// Ouvre le planning d'un élément sur la période en cours (recherche, liens).
    pub fn open(&mut self, kind: Kind, key: u32) {
        self.show_entity(kind, key);
        self.go_to(Local::now().date_naive());
    }

    /// Affiche le planning d'un élément, sur la période affichée.
    fn show_entity(&mut self, kind: Kind, key: u32) {
        self.kind = kind;
        self.selected = Some((kind, key));
        self.detail = None;
        self.scroll_to_morning = true;
    }

    /// Premier jour de la période qui contient `d`.
    fn align(&self, d: NaiveDate) -> NaiveDate {
        match self.view {
            View::Days(7) => monday_of(d),
            View::Days(_) => d,
            View::Month => first_of_month(d),
        }
    }

    fn go_to(&mut self, d: NaiveDate) {
        self.anchor = self.align(d);
        self.detail = None;
    }

    fn today(&mut self) {
        self.go_to(Local::now().date_naive());
        self.scroll_to_morning = true;
    }

    /// Période suivante (1) ou précédente (-1).
    fn shift(&mut self, dir: i64) {
        self.anchor = match self.view {
            View::Days(n) => self.anchor + Duration::days(dir * n as i64),
            View::Month => add_months(self.anchor, dir as i32),
        };
        self.detail = None;
    }

    fn set_view(&mut self, v: View) {
        if self.view == v {
            return;
        }
        // Garde aujourd'hui s'il est affiché, sinon le premier jour affiché.
        let today = Local::now().date_naive();
        let keep = if self.days().contains(&today) { today } else { self.anchor };
        self.view = v;
        self.anchor = self.align(keep);
        self.detail = None;
    }

    /// Jours affichés (vue Mois : 6 semaines complètes).
    pub(super) fn days(&self) -> Vec<NaiveDate> {
        match self.view {
            View::Days(n) => (0..n as i64).map(|i| self.anchor + Duration::days(i)).collect(),
            View::Month => {
                let start = monday_of(self.anchor);
                (0..42).map(|i| start + Duration::days(i)).collect()
            }
        }
    }
}

pub(super) fn monday_of(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn first_of_month(d: NaiveDate) -> NaiveDate {
    d.with_day(1).unwrap_or(d)
}

pub(super) fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let m0 = d.month0() as i32 + n;
    let y = d.year() + m0.div_euclid(12);
    NaiveDate::from_ymd_opt(y, m0.rem_euclid(12) as u32 + 1, 1).unwrap_or(d)
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub(super) fn month_title(d: NaiveDate) -> String {
    format!("{} {}", capitalize(crate::tabs::month_name(d.month())), d.year())
}

fn range_seances(d: &Snapshot, sel: (Kind, u32), first: NaiveDate, last: NaiveDate) -> Vec<(u32, Seance)> {
    let (from, to) = (first.and_hms_opt(0, 0, 0).unwrap_or_default(), (last + Duration::days(1)).and_hms_opt(0, 0, 0).unwrap_or_default());
    let mut v: Vec<(u32, Seance)> = d
        .cours
        .iter()
        .flat_map(|(k, c)| c.seances.iter().map(move |s| (*k, s)))
        .filter(|(_, s)| s.start >= from && s.start < to && involves(d, s, sel.0, sel.1))
        .map(|(k, s)| (k, s.clone()))
        .collect();
    v.sort_by_key(|(_, s)| s.start);
    v
}

/// Nom d'un élément dans la copie locale.
pub(super) fn name_of(d: &Snapshot, kind: Kind, k: u32) -> String {
    let s = match kind {
        Kind::Enseignant => d.enseignants.get(&k).cloned(),
        Kind::Salle => d.salles.get(&k).map(|s| s.name.clone()),
        Kind::Promotion => d.promotions.get(&k).cloned(),
        Kind::TDOption => d.tdoptions.get(&k).cloned(),
        Kind::Regroupement => d.regroupements.get(&k).cloned(),
    };
    s.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| format!("#{k}"))
}

pub fn entries(d: &Snapshot, kind: Kind) -> Vec<(u32, String)> {
    let mut v: Vec<(u32, String)> = match kind {
        Kind::Enseignant => d.enseignants.iter().map(|(k, n)| (*k, n.clone())).collect(),
        Kind::Salle => d.salles.iter().map(|(k, s)| (*k, s.name.clone())).collect(),
        Kind::Promotion => d.promotions.iter().map(|(k, n)| (*k, n.clone())).collect(),
        Kind::TDOption => d.tdoptions.iter().map(|(k, n)| (*k, n.clone())).collect(),
        Kind::Regroupement => d.regroupements.iter().map(|(k, n)| (*k, n.clone())).collect(),
    };
    v.retain(|(_, n)| !n.trim().is_empty());
    v.sort_by_key(|(_, n)| n.to_lowercase());
    v
}

/// La séance concerne-t-elle l'élément ? Une salle est aussi occupée par son groupe de salles,
/// et un groupe par ses salles.
pub(super) fn involves(d: &Snapshot, s: &Seance, kind: Kind, k: u32) -> bool {
    match kind {
        Kind::Enseignant => s.enseignants.contains(&k),
        Kind::Salle => s.salles.iter().any(|r| *r == k || d.salles.get(&k).and_then(|x| x.group) == Some(*r) || d.salles.get(r).and_then(|x| x.group) == Some(k)),
        Kind::Promotion => s.promotions.contains(&k),
        Kind::TDOption => s.tdoptions.contains(&k),
        Kind::Regroupement => s.regroupements.contains(&k),
    }
}

pub(super) fn hhmm(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

pub(super) fn end_of(s: &Seance) -> NaiveDateTime {
    s.start + Duration::minutes(s.minutes as i64)
}

pub(super) fn names(d: &Snapshot, kind: Kind, ks: &[u32]) -> String {
    ks.iter().map(|k| name_of(d, kind, *k)).collect::<Vec<_>>().join(", ")
}

pub(super) fn matiere(d: &Snapshot, s: &Seance) -> String {
    d.matieres.get(&s.matiere).filter(|m| !m.trim().is_empty()).cloned().unwrap_or_else(|| t!("(Sans matière)", "(No subject)").into())
}

pub fn show(ui: &mut egui::Ui, st: &mut PlanningsState, hp: &mut HyperPlanning, favs: &mut Favs) {
    header(ui, st, hp);
    if !hp.is_configured() {
        widgets::card(ui, "", 0.0, |ui| {
            widgets::empty_state(
                ui,
                t!("HyperPlanning n'est pas connecté", "HyperPlanning is not connected"),
                t!("Saisis tes identifiants dans Réglages › HyperPlanning.", "Enter your credentials in Settings › HyperPlanning."),
            )
        });
        return;
    }
    let Some(d) = hp.data().filter(|d| d.synced_at > 0) else {
        widgets::card(ui, "", 0.0, |ui| widgets::empty_state(ui, t!("Pas encore de données", "No data yet"), t!("Première synchronisation en cours…", "First sync in progress…")));
        return;
    };
    if !st.side_tab_set {
        st.side_tab_set = true;
        if !favs.is_empty() {
            st.side_tab = SideTab::Favorites;
        }
    }
    if !st.book.is_open() {
        shortcuts(ui, st);
    }
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(30)); // ligne « maintenant »

    // Séances de la période affichée.
    let days = st.days();
    if let Some(sel) = st.selected {
        let key = (sel, days[0], *days.last().unwrap_or(&days[0]), d.synced_at);
        if st.cache.key != Some(key) {
            st.cache = RangeCache { key: Some(key), seances: range_seances(d, sel, key.1, key.2) };
        }
    } else {
        st.cache = RangeCache::default();
    }
    let seances = std::mem::take(&mut st.cache.seances);

    // Colonnes : gauche masquable, centre, détail à droite (ouvertures animées comme le Calendrier).
    let k = widgets::reveal(ui.ctx(), "hp_side", !st.side_hidden);
    let side_w = widgets::reveal_width(SIDE_W, k);
    if st.detail.is_some() {
        st.detail_last = st.detail;
    }
    let kp = widgets::reveal(ui.ctx(), "hp_panel", st.detail.is_some());
    let panel_w = if st.detail_last.is_some() { widgets::reveal_width(PANEL_W, kp) } else { 0.0 };
    if st.detail.is_none() && panel_w == 0.0 {
        st.detail_last = None;
    }
    let gaps = if side_w > 0.0 { 8.0 } else { 0.0 } + if panel_w > 0.0 { 8.0 } else { 0.0 };
    let center_w = (ui.available_width() - side_w - panel_w - gaps).max(200.0);
    let mut fixed = Vec::new();
    if side_w > 0.0 {
        fixed.push(side_w);
    }
    if panel_w > 0.0 {
        fixed.push(center_w);
    }
    let offset = if side_w > 0.0 { 0 } else { 1 };
    widgets::pane_columns(ui, &fixed, |i, ui| match i + offset {
        0 => widgets::animated_column(ui, side_w, SIDE_W, k, |ui| side(ui, st, d, favs)),
        1 => center(ui, st, d, &seances, favs),
        _ => widgets::animated_column(ui, panel_w, PANEL_W, kp, |ui| {
            if let Some(which) = st.detail.or(st.detail_last) {
                detail(ui, st, d, &seances, which);
            }
        }),
    });
    st.cache.seances = seances;

    // Panneau de réservation (par-dessus l'app).
    if let Some((room, req)) = st.book.show(ui.ctx(), d, &hp.request_status) {
        hp.request_room(room, req);
    }
}

/// En-tête : mois et année en gros, numéro de semaine, état de la synchronisation.
fn header(ui: &mut egui::Ui, st: &PlanningsState, hp: &mut HyperPlanning) {
    let days = st.days();
    let (a, b) = (days[0], *days.last().unwrap_or(&days[0]));
    let (title, week) = match st.view {
        View::Month => (month_title(st.anchor), None),
        View::Days(_) => {
            let w = Some(tf!("Semaine {}", "Week {}", a.iso_week().week()));
            if a.month() == b.month() {
                (month_title(a), w)
            } else if a.year() == b.year() {
                (format!("{} – {}", capitalize(crate::tabs::month_name(a.month())), month_title(b)), w)
            } else {
                (format!("{} – {}", month_title(a), month_title(b)), w)
            }
        }
    };
    widgets::page_header_with(ui, "Plannings", &title, |ui| {
        if let Some(w) = week {
            widgets::header_badge(ui, &w, false);
        }
        ui.add_space(8.0);
        sync_badge(ui, hp);
    });
}

/// Raccourcis du Calendrier (inactifs pendant la saisie de texte).
fn shortcuts(ui: &mut egui::Ui, st: &mut PlanningsState) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k) && i.modifiers.is_none());
    if pressed(Key::T) {
        st.today();
    }
    if pressed(Key::D) {
        st.set_view(View::Days(1));
    }
    if pressed(Key::W) {
        st.set_view(View::Days(7));
    }
    if pressed(Key::M) {
        st.set_view(View::Month);
    }
    for (k, n) in [(Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5), (Key::Num6, 6), (Key::Num7, 7)] {
        if pressed(k) {
            st.set_view(View::Days(n));
        }
    }
    if pressed(Key::J) || pressed(Key::ArrowRight) {
        st.shift(1);
    }
    if pressed(Key::K) || pressed(Key::ArrowLeft) {
        st.shift(-1);
    }
    if pressed(Key::S) {
        st.side_hidden = !st.side_hidden;
    }
    if pressed(Key::Escape) {
        st.drag = None;
        st.detail = None;
    }
}

/// Colonne de gauche : onglets Favoris / Répertoire.
fn side(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, favs: &mut Favs) {
    ui.spacing_mut().item_spacing.y = 6.0;
    let tabs = [(SideTab::Favorites, t!("★  Favoris", "★  Favourites")), (SideTab::Directory, t!("Répertoire", "Directory"))];
    widgets::segmented(ui, "hp_side_tab", &tabs, &mut st.side_tab);
    match st.side_tab {
        SideTab::Favorites => favorites(ui, st, d, favs),
        SideTab::Directory => list(ui, st, d, favs),
    }
}

/// Favoris de tous types, rangés par type.
fn favorites(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, favs: &mut Favs) {
    if favs.is_empty() {
        ui.add_space(20.0);
        widgets::empty_state(
            ui,
            t!("Aucun favori", "No favourites"),
            t!("Dans le Répertoire, survole un planning et clique sur l'étoile pour l'ajouter.", "In the Directory, hover a timetable and click the star to add it."),
        );
        return;
    }
    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(t!("Filtrer…", "Filter…")).desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let needle = st.filter.trim().to_lowercase();
    let mut groups: Vec<(Kind, Vec<(u32, String)>)> = Vec::new();
    for kind in Kind::ALL {
        let mut items: Vec<(u32, String)> = favs
            .iter()
            .filter(|(t, _)| Kind::from_u8(*t) == Some(kind))
            .map(|(_, k)| (*k, name_of(d, kind, *k)))
            .filter(|(_, n)| needle.is_empty() || n.to_lowercase().contains(&needle))
            .collect();
        items.sort_by_key(|(_, n)| n.to_lowercase());
        if !items.is_empty() {
            groups.push((kind, items));
        }
    }
    egui::ScrollArea::vertical().id_salt("hp_favs").auto_shrink([false, false]).show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            for (kind, items) in &groups {
                ui.add_space(4.0);
                ui.label(RichText::new(kind.label().to_uppercase()).small().color(theme::TEXT_FAINT()));
                for (k, name) in items {
                    item_button(ui, st, d, favs, *kind, *k, name);
                }
            }
        });
    });
}

/// Choix du type (Enseignants, Salles, Promos, TD, Regroup.) dans un panneau dépliable.
fn kind_picker(ui: &mut egui::Ui, st: &mut PlanningsState) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if st.kind_open || resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), theme::SURFACE_HI());
    } else {
        p.rect_stroke(rect, CornerRadius::same(8), Stroke::new(1.0, theme::BORDER()), StrokeKind::Inside);
    }
    p.text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, t!("Type", "Type"), FontId::proportional(11.0), theme::TEXT_FAINT());
    p.text(rect.left_center() + vec2(48.0, 0.0), Align2::LEFT_CENTER, st.kind.label(), FontId::proportional(13.0), theme::TEXT());
    p.text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, if st.kind_open { "▴" } else { "▾" }, FontId::proportional(12.0), theme::TEXT_MUTED());
    if resp.clicked() {
        st.kind_open = !st.kind_open;
    }
    let k = widgets::reveal(ui.ctx(), "hp_kind_open", st.kind_open);
    if k > 0.0 {
        ui.scope(|ui| {
            ui.multiply_opacity(k);
            ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                for kind in Kind::ALL {
                    if ui.add(egui::Button::selectable(st.kind == kind, kind.label()).min_size(vec2(0.0, 22.0))).clicked() {
                        st.kind = kind;
                        st.kind_open = false;
                    }
                }
            });
        });
    }
}

/// Barre d'outils (comme le Calendrier), la vue, puis le bandeau de l'année scolaire.
fn center(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)], favs: &mut Favs) {
    ui.horizontal(|ui| {
        let (icon, tip) = if st.side_hidden {
            ("»", t!("Afficher le panneau de gauche (S)", "Show the left panel (S)"))
        } else {
            ("«", t!("Masquer le panneau de gauche (S)", "Hide the left panel (S)"))
        };
        if ui.button(icon).on_hover_text(tip).clicked() {
            st.side_hidden = !st.side_hidden;
        }
        if ui.button(t!("Aujourd'hui", "Today")).on_hover_text("T").clicked() {
            st.today();
        }
        if ui.button("‹").on_hover_text(t!("Précédent (K ou ←)", "Previous (K or ←)")).clicked() {
            st.shift(-1);
        }
        if ui.button("›").on_hover_text(t!("Suivant (J ou →)", "Next (J or →)")).clicked() {
            st.shift(1);
        }
        if let Some((kind, k)) = st.selected {
            ui.add_space(6.0);
            // Nom du planning, tronqué pour laisser la place au choix de la vue.
            let mut title = name_of(d, kind, k);
            if kind == Kind::Salle
                && let Some(c) = d.salles.get(&k).map(|s| s.capacity).filter(|c| *c > 0)
            {
                title = format!("{title}  ·  {}", tf!("{c} places", "{c} seats"));
            }
            let fav = is_fav(favs, kind, k);
            let tip = if fav { t!("Retirer des favoris", "Remove from favourites") } else { t!("Ajouter aux favoris", "Add to favourites") };
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 24.0), Sense::click());
            let resp = resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
            let color = if fav || resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
            widgets::paint_star(ui.painter(), r.center(), 7.0, color, fav);
            if resp.clicked() {
                toggle_fav(favs, kind, k);
            }
            let w = (ui.available_width() - 270.0).max(0.0);
            ui.allocate_ui(vec2(w, 24.0), |ui| ui.add(egui::Label::new(widgets::section_title(&title)).truncate()));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut v = match st.view {
                View::Days(1) => 1,
                View::Days(7) => 7,
                View::Days(n) => n as i32 + 100,
                View::Month => 0,
            };
            let custom = match st.view {
                View::Days(n) if n != 1 && n != 7 => tf!("{n} jours", "{n} days"),
                _ => t!("3 jours", "3 days").into(),
            };
            let custom_v = match st.view {
                View::Days(n) if n != 1 && n != 7 => n as i32 + 100,
                _ => 103,
            };
            let opts = [(0, t!("Mois", "Month")), (7, t!("Semaine", "Week")), (custom_v, custom.as_str()), (1, t!("Jour", "Day"))];
            if widgets::segmented(ui, "hp_view", &opts, &mut v) {
                st.set_view(match v {
                    0 => View::Month,
                    1 => View::Days(1),
                    7 => View::Days(7),
                    n => View::Days((n - 100) as u8),
                });
            }
        });
    });
    if let Some((msg, t)) = &st.notice {
        if t.elapsed().as_secs() < 4 {
            ui.label(RichText::new(msg).small().color(theme::TODAY()));
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        } else {
            st.notice = None;
        }
    }
    ui.add_space(10.0);
    let view_h = (ui.available_height() - month::YEAR_STRIP_H - 10.0).max(120.0);
    ui.allocate_ui(vec2(ui.available_width(), view_h), |ui| {
        ui.set_min_height(view_h);
        if st.selected.is_none() {
            widgets::empty_state(
                ui,
                t!("Choisis un planning", "Pick a timetable"),
                t!("Enseignant, salle, promotion… dans la liste de gauche (S pour l'afficher).", "Teacher, room, group… in the list on the left (S to show it)."),
            );
            return;
        }
        match st.view {
            View::Month => month::show(ui, st, d, seances),
            View::Days(_) => grid::show(ui, st, d, seances),
        }
    });
    ui.add_space(6.0);
    month::school_year(ui, st, d);
}

/// État de la synchronisation dans l'en-tête, avec le bouton pour la relancer.
fn sync_badge(ui: &mut egui::Ui, hp: &mut HyperPlanning) {
    use crate::hyperplanning::SyncStatus;
    let running = hp.sync_status == SyncStatus::Running;
    if hp.is_configured() && ui.add_enabled(!running, egui::Button::new("⟳")).on_hover_text(t!("Synchroniser", "Sync")).clicked() {
        hp.sync_now();
    }
    if running {
        ui.add(egui::Spinner::new().size(14.0));
    } else if let SyncStatus::Failed(e) = &hp.sync_status {
        widgets::header_badge(ui, t!("Hors ligne", "Offline"), false);
        ui.label("").on_hover_text(e.as_str());
    } else if let Some(t) = hp.data().map(|d| d.synced_at).filter(|t| *t > 0).and_then(|t| chrono::DateTime::from_timestamp(t, 0)) {
        let when = t.with_timezone(&Local).format("%H:%M").to_string();
        ui.label(RichText::new(tf!("À jour · {when}", "Up to date · {when}")).small().color(theme::TEXT_FAINT()));
    }
}

fn list(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, favs: &mut Favs) {
    kind_picker(ui, st);
    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(t!("Filtrer…", "Filter…")).desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let needle = st.filter.trim().to_lowercase();
    let items: Vec<(u32, String)> = entries(d, st.kind).into_iter().filter(|(_, n)| needle.is_empty() || n.to_lowercase().contains(&needle)).collect();
    ui.label(RichText::new(tf!("{} élément(s)", "{} item(s)", items.len())).small().color(theme::TEXT_FAINT()));
    // Salles et promotions : rangées par catégorie (familles HYPERPLANNING), sauf pendant un filtre.
    let cats = match st.kind {
        Kind::Salle => Some(&d.salle_cats),
        Kind::Promotion => Some(&d.promo_cats),
        _ => None,
    }
    .filter(|c| !c.is_empty() && needle.is_empty());
    if let Some(cats) = cats {
        grouped_list(ui, st, d, favs, &items, cats);
        return;
    }
    egui::ScrollArea::vertical().id_salt("hp_list").auto_shrink([false, false]).show_rows(ui, 24.0, items.len(), |ui, range| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            for (k, name) in &items[range] {
                item_button(ui, st, d, favs, st.kind, *k, name);
            }
        });
    });
}

/// Un élément de la liste (salle : avec sa capacité) ; étoile à droite (favori, ou au survol).
fn item_button(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, favs: &mut Favs, kind: Kind, k: u32, name: &str) {
    let on = st.selected == Some((kind, k));
    let mut label = name.to_string();
    if kind == Kind::Salle
        && let Some(s) = d.salles.get(&k)
        && s.capacity > 0
    {
        label = format!("{label}  ·  {}", s.capacity);
    }
    let fav = is_fav(favs, kind, k);
    let resp = ui.add(egui::Button::selectable(on, label).truncate().min_size(vec2(ui.available_width(), 22.0)));
    let star = egui::Rect::from_center_size(resp.rect.right_center() - vec2(12.0, 0.0), vec2(20.0, resp.rect.height()));
    let star_resp = ui
        .interact(star, ui.scope_id().with(("hp_fav", kind as u8, k)), Sense::click())
        .on_hover_text(if fav { t!("Retirer des favoris", "Remove from favourites") } else { t!("Ajouter aux favoris", "Add to favourites") });
    if fav || ui.rect_contains_pointer(resp.rect) {
        let color = if fav { theme::TEXT() } else if star_resp.hovered() { theme::TEXT() } else { theme::TEXT_FAINT() };
        // Fond sous l'étoile pour masquer la fin d'un nom long.
        let bg = if on { ui.visuals().selection.bg_fill } else if ui.rect_contains_pointer(resp.rect) { ui.visuals().widgets.hovered.weak_bg_fill } else { theme::SURFACE() };
        ui.painter().rect_filled(star.shrink2(vec2(0.0, 2.0)), CornerRadius::same(4), bg);
        widgets::paint_star(ui.painter(), star.center(), 6.0, color, fav);
    }
    if star_resp.clicked() {
        toggle_fav(favs, kind, k);
    } else if resp.clicked() {
        st.show_entity(kind, k);
    }
}

/// Liste rangée par famille › rubrique (sections repliables) ; un élément peut figurer dans
/// plusieurs rubriques. Une seule famille : ses rubriques directement.
fn grouped_list(
    ui: &mut egui::Ui,
    st: &mut PlanningsState,
    d: &Snapshot,
    favs: &mut Favs,
    items: &[(u32, String)],
    cats: &std::collections::BTreeMap<u32, Vec<crate::hyperplanning::cache::Category>>,
) {
    use std::collections::BTreeMap;
    let mut tree: BTreeMap<&str, BTreeMap<&str, Vec<&(u32, String)>>> = BTreeMap::new();
    let mut other = Vec::new();
    for it in items {
        match cats.get(&it.0).filter(|c| !c.is_empty()) {
            Some(list) => {
                for c in list {
                    tree.entry(&c.famille).or_default().entry(&c.rubrique).or_default().push(it);
                }
            }
            None => other.push(it),
        }
    }
    let kind = st.kind;
    egui::ScrollArea::vertical().id_salt(("hp_tree", kind as u8)).auto_shrink([false, false]).show(ui, |ui| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            let rubriques = |ui: &mut egui::Ui, st: &mut PlanningsState, favs: &mut Favs, famille: &str, rubs: &BTreeMap<&str, Vec<&(u32, String)>>| {
                for (rubrique, list) in rubs {
                    let open_sel = list.iter().any(|(k, _)| st.selected == Some((kind, *k)));
                    egui::CollapsingHeader::new(RichText::new(format!("{rubrique}  ({})", list.len())))
                        .id_salt(("hp_rub", kind as u8, famille, *rubrique))
                        .default_open(open_sel)
                        .show(ui, |ui| {
                            for (k, name) in list {
                                item_button(ui, st, d, favs, kind, *k, name);
                            }
                        });
                }
            };
            if tree.len() == 1 {
                let (famille, rubs) = tree.iter().next().unwrap();
                rubriques(ui, st, favs, famille, rubs);
            } else {
                for (famille, rubs) in &tree {
                    egui::CollapsingHeader::new(RichText::new(*famille).strong()).id_salt(("hp_fam", kind as u8, *famille)).default_open(true).show(ui, |ui| {
                        rubriques(ui, st, favs, famille, rubs);
                    });
                }
            }
            if !other.is_empty() {
                egui::CollapsingHeader::new(RichText::new(tf!("Sans catégorie  ({})", "Uncategorised  ({})", other.len())).color(theme::TEXT_MUTED()))
                    .id_salt(("hp_other", kind as u8))
                    .show(ui, |ui| {
                        for (k, name) in &other {
                            item_button(ui, st, d, favs, kind, *k, name);
                        }
                    });
            }
        });
    });
}

/// Répartit les séances qui se chevauchent d'un même jour en colonnes : (indice, colonne, nb colonnes).
pub(super) fn layout(day: &[(usize, &Seance)]) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut group: Vec<(usize, usize, NaiveDateTime)> = Vec::new(); // (indice, colonne, fin)
    let mut group_end = NaiveDateTime::MIN;
    let flush = |group: &mut Vec<(usize, usize, NaiveDateTime)>, out: &mut Vec<(usize, usize, usize)>| {
        let n = group.iter().map(|g| g.1 + 1).max().unwrap_or(1);
        out.extend(group.drain(..).map(|(i, c, _)| (i, c, n)));
    };
    for (i, s) in day {
        if s.start >= group_end && !group.is_empty() {
            flush(&mut group, &mut out);
        }
        let col = (0..).find(|c| !group.iter().any(|g| g.1 == *c && g.2 > s.start)).unwrap_or(0);
        group.push((*i, col, end_of(s)));
        group_end = group_end.max(end_of(s));
    }
    flush(&mut group, &mut out);
    out
}

/// Couleur fixe d'un enseignant : teintes réparties par le nombre d'or (voisines différentes),
/// saturation et luminosité adaptées au thème.
pub fn teacher_color(key: u32) -> egui::Color32 {
    let hue = (key as f32 * 0.618_034).fract();
    let (s, v) = if theme::is_light() { (0.55, 0.78) } else { (0.50, 0.92) };
    egui::ecolor::Hsva::new(hue, s, v, 1.0).into()
}

pub(super) fn paint_seance(p: &egui::Painter, r: Rect, d: &Snapshot, s: &Seance, viewing: Option<Kind>, hl: bool, past: bool) {
    let ink = if past { theme::TEXT_MUTED() } else { theme::TEXT() };
    // Couleur du premier enseignant du cours (gris s'il n'y en a pas).
    let color = s.enseignants.first().map(|k| teacher_color(*k)).unwrap_or(theme::TEXT_MUTED());
    let color = if past { color.gamma_multiply(0.55) } else { color };
    p.rect_filled(r, CornerRadius::same(6), color.gamma_multiply(if hl { 0.42 } else { 0.26 }));
    p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, color);
    if hl {
        p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, theme::TEXT_MUTED()), StrokeKind::Inside);
    }
    let tp = p.with_clip_rect(r.shrink(1.0).intersect(p.clip_rect()));
    let x = r.left() + 8.0;
    let title = matiere(d, s);
    let time = format!("{} – {}", hhmm(s.start), hhmm(end_of(s)));
    if r.height() < 30.0 {
        tp.text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, format!("{title}  {}", hhmm(s.start)), theme::editor_font(11.5, true), ink);
        return;
    }
    tp.text(Pos2::new(x, r.top() + 5.0), Align2::LEFT_TOP, title, theme::editor_font(12.0, true), ink);
    tp.text(Pos2::new(x, r.top() + 21.0), Align2::LEFT_TOP, time, FontId::proportional(11.0), ink.gamma_multiply(0.75));
    // Lignes suivantes : ce qui n'est pas l'élément affiché (salles, enseignants, groupes).
    let mut lines = Vec::new();
    if viewing != Some(Kind::Salle) && !s.salles.is_empty() {
        lines.push(names(d, Kind::Salle, &s.salles));
    }
    if viewing != Some(Kind::Enseignant) && !s.enseignants.is_empty() {
        lines.push(names(d, Kind::Enseignant, &s.enseignants));
    }
    let groups = [names(d, Kind::Promotion, &s.promotions), names(d, Kind::TDOption, &s.tdoptions), names(d, Kind::Regroupement, &s.regroupements)];
    let groups: Vec<&str> = groups.iter().map(String::as_str).filter(|g| !g.is_empty()).collect();
    if !groups.is_empty() {
        lines.push(groups.join(", "));
    }
    for (n, line) in lines.into_iter().enumerate() {
        let y = r.top() + 36.0 + n as f32 * 14.0;
        if y + 12.0 > r.bottom() {
            break;
        }
        tp.text(Pos2::new(x, y), Align2::LEFT_TOP, line, FontId::proportional(11.0), ink.gamma_multiply(0.6));
    }
}

/// Détail d'une séance (panneau de droite) ; `which` : séance affichée (aussi pendant la fermeture).
fn detail(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)], which: (u32, NaiveDateTime)) {
    let (cours, start) = which;
    let Some((_, s)) = seances.iter().find(|(c, s)| *c == cours && s.start == start) else {
        st.detail = None;
        return;
    };
    let mut go = None;
    {
        ui.horizontal(|ui| {
            ui.label(widgets::section_title(&matiere(d, s)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                if ui.small_button("✕").clicked() {
                    st.detail = None;
                }
            });
        });
        let day = crate::tabs::long_date(s.start.date());
        ui.label(RichText::new(format!("{day}\n{} – {}", hhmm(s.start), hhmm(end_of(s)))).color(theme::TEXT_MUTED()));
        if !s.type_cours.trim().is_empty() {
            ui.label(RichText::new(&s.type_cours).small().color(theme::TEXT_FAINT()));
        }
        let rows: [(&str, Kind, &[u32]); 5] = [
            (t!("Enseignants", "Teachers"), Kind::Enseignant, &s.enseignants),
            (t!("Salles", "Rooms"), Kind::Salle, &s.salles),
            ("Promotions", Kind::Promotion, &s.promotions),
            (t!("TD / options", "Groups / options"), Kind::TDOption, &s.tdoptions),
            (t!("Regroupements", "Clusters"), Kind::Regroupement, &s.regroupements),
        ];
        for (title, kind, ks) in rows {
            if ks.is_empty() {
                continue;
            }
            ui.add_space(8.0);
            ui.label(RichText::new(title).small().color(theme::TEXT_FAINT()));
            ui.horizontal_wrapped(|ui| {
                for k in ks {
                    let mut text = name_of(d, kind, *k);
                    if kind == Kind::Salle
                        && let Some(c) = d.salles.get(k).map(|x| x.capacity).filter(|c| *c > 0)
                    {
                        text = format!("{text} ({c})");
                    }
                    if ui.link(text).on_hover_text(t!("Voir son planning", "Show its timetable")).clicked() {
                        go = Some((kind, *k));
                    }
                }
            });
        }
        if let Some(memo) = d.cours.get(&cours).map(|c| c.memo.trim()).filter(|m| !m.is_empty()) {
            ui.add_space(8.0);
            ui.label(RichText::new(t!("Mémo", "Memo")).small().color(theme::TEXT_FAINT()));
            ui.add(egui::Label::new(memo).wrap());
        }
    }
    if let Some((kind, k)) = go {
        // Même période, autre élément.
        st.show_entity(kind, k);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(h: u32, m: u32, min: u32) -> Seance {
        Seance {
            start: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap().and_hms_opt(h, m, 0).unwrap(),
            minutes: min,
            matiere: 0,
            type_cours: String::new(),
            enseignants: vec![],
            salles: vec![],
            promotions: vec![],
            tdoptions: vec![],
            regroupements: vec![],
        }
    }

    #[test]
    fn overlapping_sessions_share_columns() {
        let (a, b, c, e) = (s(9, 0, 120), s(10, 0, 60), s(11, 30, 60), s(14, 0, 60));
        let l = layout(&[(0, &a), (1, &b), (2, &c), (3, &e)]);
        assert_eq!(l, vec![(0, 0, 2), (1, 1, 2), (2, 0, 1), (3, 0, 1)]);
    }

    #[test]
    fn monday_is_found() {
        assert_eq!(monday_of(NaiveDate::from_ymd_opt(2026, 11, 7).unwrap()), NaiveDate::from_ymd_opt(2026, 11, 2).unwrap());
    }
}
