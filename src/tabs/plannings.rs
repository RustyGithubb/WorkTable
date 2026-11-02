//! Onglet Plannings : emplois du temps HyperPlanning (enseignants, salles, promotions, TD/options,
//! regroupements), lus dans la copie locale — aucune requête au serveur.
//!
//! - Colonne de gauche : type de planning, filtre, liste.
//! - Grille de la semaine (lundi → samedi, dimanche s'il y a des séances).
//! - Clic sur une séance : détail ; clic sur un nom du détail : planning de cet élément.
//! - Mode « Salles libres » : salles disponibles sur un créneau (voir `free`).

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Timelike};
use eframe::egui::{self, Align2, CornerRadius, CursorIcon, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, vec2};

use crate::hyperplanning::HyperPlanning;
use crate::hyperplanning::cache::{Seance, Snapshot};
use crate::{theme, widgets};

mod free;

const GUTTER: f32 = 54.0;
const HOUR_H: f32 = 46.0;
const HEADER_H: f32 = 40.0;
const LIST_W: f32 = 270.0;
const DETAIL_W: f32 = 270.0;

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
            Kind::Promotion => "Promotions",
            Kind::TDOption => t!("TD", "Groups"),
            Kind::Regroupement => t!("Regroup.", "Clusters"),
        }
    }
}

/// Séances de l'élément affiché pour la semaine affichée (recalculées quand l'un change).
#[derive(Default)]
struct WeekCache {
    key: Option<((Kind, u32), NaiveDate, i64)>,
    seances: Vec<(u32, Seance)>,
}

/// Affichage de l'onglet.
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum Mode {
    #[default]
    Plannings,
    Free,
}

#[derive(Default)]
pub struct PlanningsState {
    mode: Mode,
    free: free::FreeState,
    kind: Kind,
    filter: String,
    selected: Option<(Kind, u32)>,
    /// Lundi de la semaine affichée (`None` : semaine en cours).
    week: Option<NaiveDate>,
    /// Séance ouverte dans le détail : (cours, début).
    detail: Option<(u32, NaiveDateTime)>,
    scroll_to_morning: bool,
    cache: WeekCache,
}

impl PlanningsState {
    /// Ouvre le planning d'un élément sur la semaine en cours (recherche, liens).
    pub fn open(&mut self, kind: Kind, key: u32) {
        self.mode = Mode::Plannings;
        self.kind = kind;
        self.selected = Some((kind, key));
        self.week = None;
        self.detail = None;
        self.scroll_to_morning = true;
    }

    fn monday(&self) -> NaiveDate {
        self.week.unwrap_or_else(|| monday_of(Local::now().date_naive()))
    }
}

fn monday_of(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

/// Nom d'un élément dans la copie locale.
fn name_of(d: &Snapshot, kind: Kind, k: u32) -> String {
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
fn involves(d: &Snapshot, s: &Seance, kind: Kind, k: u32) -> bool {
    match kind {
        Kind::Enseignant => s.enseignants.contains(&k),
        Kind::Salle => s.salles.iter().any(|r| *r == k || d.salles.get(&k).and_then(|x| x.group) == Some(*r) || d.salles.get(r).and_then(|x| x.group) == Some(k)),
        Kind::Promotion => s.promotions.contains(&k),
        Kind::TDOption => s.tdoptions.contains(&k),
        Kind::Regroupement => s.regroupements.contains(&k),
    }
}

fn week_seances(d: &Snapshot, sel: (Kind, u32), monday: NaiveDate) -> Vec<(u32, Seance)> {
    let (from, to) = (monday.and_hms_opt(0, 0, 0).unwrap_or_default(), (monday + Duration::days(7)).and_hms_opt(0, 0, 0).unwrap_or_default());
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

fn hhmm(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

fn end_of(s: &Seance) -> NaiveDateTime {
    s.start + Duration::minutes(s.minutes as i64)
}

fn names(d: &Snapshot, kind: Kind, ks: &[u32]) -> String {
    ks.iter().map(|k| name_of(d, kind, *k)).collect::<Vec<_>>().join(", ")
}

fn matiere(d: &Snapshot, s: &Seance) -> String {
    d.matieres.get(&s.matiere).filter(|m| !m.trim().is_empty()).cloned().unwrap_or_else(|| t!("(Sans matière)", "(No subject)").into())
}

pub fn show(ui: &mut egui::Ui, st: &mut PlanningsState, hp: &mut HyperPlanning) {
    widgets::page_header_with(ui, "HyperPlanning", "Plannings", |ui| {
        ui.add_space(8.0);
        widgets::segmented(ui, "hp_mode", &[(Mode::Plannings, "Plannings"), (Mode::Free, t!("Salles libres", "Free rooms"))], &mut st.mode);
        ui.add_space(8.0);
        sync_badge(ui, hp);
    });
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
    if st.mode == Mode::Free {
        let mut action = None;
        let (web_url, status) = (&hp.config.web_url, &hp.request_status);
        widgets::pane_columns(ui, &[LIST_W], |i, ui| match i {
            0 => free::form(ui, &mut st.free),
            _ => action = free::results(ui, &mut st.free, d, web_url, status),
        });
        match action {
            Some(free::Action::Open(k)) => {
                // Planning de la salle, sur la semaine du jour choisi.
                let day = st.free.date();
                st.open(Kind::Salle, k);
                st.week = Some(monday_of(day));
            }
            Some(free::Action::Request(k, req)) => hp.request_room(k, req),
            None => {}
        }
        return;
    }
    widgets::pane_columns(ui, &[LIST_W], |i, ui| match i {
        0 => list(ui, st, d),
        _ => planning(ui, st, d),
    });
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

fn list(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot) {
    let options: Vec<(Kind, &str)> = Kind::ALL.iter().map(|k| (*k, k.label())).collect();
    ui.spacing_mut().item_spacing.y = 6.0;
    widgets::segmented(ui, "hp_kind", &options[..2], &mut st.kind);
    widgets::segmented(ui, "hp_kind2", &options[2..], &mut st.kind);
    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(t!("Filtrer…", "Filter…")).desired_width(f32::INFINITY));
    ui.add_space(4.0);
    let needle = st.filter.trim().to_lowercase();
    let items: Vec<(u32, String)> = entries(d, st.kind).into_iter().filter(|(_, n)| needle.is_empty() || n.to_lowercase().contains(&needle)).collect();
    ui.label(RichText::new(tf!("{} élément(s)", "{} item(s)", items.len())).small().color(theme::TEXT_FAINT()));
    egui::ScrollArea::vertical().id_salt("hp_list").auto_shrink([false, false]).show_rows(ui, 24.0, items.len(), |ui, range| {
        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
            for (k, name) in &items[range] {
                let on = st.selected == Some((st.kind, *k));
                let mut label = name.clone();
                if st.kind == Kind::Salle
                    && let Some(s) = d.salles.get(k)
                    && s.capacity > 0
                {
                    label = format!("{label}  ·  {}", s.capacity);
                }
                if ui.add(egui::Button::selectable(on, label).truncate().min_size(vec2(0.0, 22.0))).clicked() {
                    st.selected = Some((st.kind, *k));
                    st.detail = None;
                    st.scroll_to_morning = true;
                }
            }
        });
    });
}

fn planning(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot) {
    let Some(sel) = st.selected else {
        widgets::empty_state(
            ui,
            t!("Choisis un planning", "Pick a timetable"),
            t!("Enseignant, salle, promotion… dans la liste de gauche.", "Teacher, room, group… in the list on the left."),
        );
        return;
    };
    let monday = st.monday();
    // Barre : nom, navigation de semaine.
    ui.horizontal(|ui| {
        ui.label(widgets::section_title(&name_of(d, sel.0, sel.1)));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("›").on_hover_text(t!("Semaine suivante", "Next week")).clicked() {
                st.week = Some(monday + Duration::days(7));
                st.detail = None;
            }
            if ui.button(t!("Aujourd'hui", "Today")).clicked() {
                st.week = None;
                st.detail = None;
                st.scroll_to_morning = true;
            }
            if ui.button("‹").on_hover_text(t!("Semaine précédente", "Previous week")).clicked() {
                st.week = Some(monday - Duration::days(7));
                st.detail = None;
            }
            let sat = monday + Duration::days(5);
            let range = if monday.month() == sat.month() {
                format!("{} – {} {} {}", monday.day(), sat.day(), crate::tabs::month_name(sat.month()), sat.year())
            } else {
                format!("{} {} – {} {} {}", monday.day(), crate::tabs::month_short(monday.month()), sat.day(), crate::tabs::month_short(sat.month()), sat.year())
            };
            ui.label(RichText::new(range).color(theme::TEXT_MUTED()));
        });
    });
    ui.add_space(6.0);

    let key = (sel, monday, d.synced_at);
    if st.cache.key != Some(key) {
        st.cache = WeekCache { key: Some(key), seances: week_seances(d, sel, monday) };
    }
    let seances = std::mem::take(&mut st.cache.seances);
    if st.detail.is_some() {
        let full = ui.available_rect_before_wrap();
        let grid_rect = Rect::from_min_max(full.min, Pos2::new(full.right() - DETAIL_W - 10.0, full.bottom()));
        let detail_rect = Rect::from_min_max(Pos2::new(grid_rect.right() + 10.0, full.top()), full.max);
        ui.scope_builder(egui::UiBuilder::new().max_rect(grid_rect), |ui| grid(ui, st, d, &seances, monday));
        ui.scope_builder(egui::UiBuilder::new().max_rect(detail_rect), |ui| detail(ui, st, d, &seances));
        ui.allocate_rect(full, Sense::hover());
    } else {
        grid(ui, st, d, &seances, monday);
    }
    st.cache.seances = seances;
}

/// Répartit les séances qui se chevauchent d'un même jour en colonnes : (indice, colonne, nb colonnes).
fn layout(day: &[(usize, &Seance)]) -> Vec<(usize, usize, usize)> {
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

fn grid(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)], monday: NaiveDate) {
    let has_sunday = seances.iter().any(|(_, s)| s.start.weekday().num_days_from_monday() == 6);
    let days: Vec<NaiveDate> = (0..if has_sunday { 7 } else { 6 }).map(|i| monday + Duration::days(i)).collect();
    // Heures affichées : 8 h – 19 h, élargies aux séances de la semaine.
    let first = seances.iter().map(|(_, s)| s.start.hour()).min().unwrap_or(8).min(8);
    let last = seances.iter().map(|(_, s)| (end_of(s).hour() + u32::from(end_of(s).minute() > 0)).min(24)).max().unwrap_or(19).max(19);
    let (h0, h1) = (first as i32, last as i32);
    let width = ui.available_width();
    let col_w = ((width - GUTTER - 10.0) / days.len() as f32).max(40.0);
    let today = Local::now().date_naive();

    let (head, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    for (i, day) in days.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(head.left() + GUTTER + i as f32 * col_w, head.top()), vec2(col_w, HEADER_H));
        let label = format!("{} {}", crate::tabs::weekday_short(day.weekday()).to_uppercase(), day.day());
        let color = if *day == today { theme::TODAY() } else { theme::TEXT_MUTED() };
        ui.painter().text(r.center(), Align2::CENTER_CENTER, label, theme::editor_font(13.0, *day == today), color);
    }
    ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));

    let mut area = egui::ScrollArea::vertical().id_salt("hp_grid").auto_shrink([false, false]);
    if st.scroll_to_morning {
        let start = seances.iter().map(|(_, s)| s.start.hour()).min().unwrap_or(8) as i32;
        area = area.vertical_scroll_offset(((start - h0) as f32 - 0.5).max(0.0) * HOUR_H);
        st.scroll_to_morning = false;
    }
    area.show(ui, |ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(width - 10.0, (h1 - h0) as f32 * HOUR_H), Sense::hover());
        let p = ui.painter().clone();
        let grid_left = rect.left() + GUTTER;
        let y_of = |t: NaiveDateTime| rect.top() + ((t.hour() as i32 - h0) as f32 + t.minute() as f32 / 60.0) * HOUR_H;
        for (i, day) in days.iter().enumerate() {
            if *day == today {
                p.rect_filled(Rect::from_min_size(Pos2::new(grid_left + i as f32 * col_w, rect.top()), vec2(col_w, rect.height())), CornerRadius::ZERO, theme::overlay(5));
            }
        }
        for h in h0..=h1 {
            let y = rect.top() + (h - h0) as f32 * HOUR_H;
            if h > h0 {
                p.hline(grid_left..=rect.right(), y, Stroke::new(1.0, theme::overlay(14)));
            }
            p.text(Pos2::new(grid_left - 8.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), FontId::proportional(10.5), theme::TEXT_FAINT());
        }
        for i in 0..=days.len() {
            p.vline(grid_left + i as f32 * col_w, rect.y_range(), Stroke::new(1.0, theme::overlay(12)));
        }
        // Heure actuelle.
        let now = Local::now().naive_local();
        if let Some(i) = days.iter().position(|day| *day == now.date())
            && (h0..h1).contains(&(now.hour() as i32))
        {
            let x = grid_left + i as f32 * col_w;
            p.hline(x..=x + col_w, y_of(now), Stroke::new(1.5, theme::TODAY()));
        }

        for (i, day) in days.iter().enumerate() {
            let of_day: Vec<(usize, &Seance)> = seances.iter().enumerate().filter(|(_, (_, s))| s.start.date() == *day).map(|(j, (_, s))| (j, s)).collect();
            for (j, col, cols) in layout(&of_day) {
                let (cours, s) = &seances[j];
                let c = Rect::from_min_size(Pos2::new(grid_left + i as f32 * col_w, rect.top()), vec2(col_w, rect.height()));
                let w = (c.width() - 6.0) / cols as f32;
                let r = Rect::from_min_max(
                    Pos2::new(c.left() + 3.0 + col as f32 * w, y_of(s.start) + 1.0),
                    Pos2::new(c.left() + 3.0 + (col + 1) as f32 * w - 2.0, (y_of(end_of(s)) - 1.0).max(y_of(s.start) + 18.0)),
                );
                let selected = st.detail == Some((*cours, s.start));
                let resp = ui.interact(r, ui.scope_id().with(("hp_seance", *cours, s.start)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                paint_seance(&p, r, d, s, st.selected.map(|x| x.0), selected || resp.hovered(), end_of(s) < now);
                if resp.clicked() {
                    st.detail = if selected { None } else { Some((*cours, s.start)) };
                }
            }
        }
    });
}

fn paint_seance(p: &egui::Painter, r: Rect, d: &Snapshot, s: &Seance, viewing: Option<Kind>, hl: bool, past: bool) {
    let ink = if past { theme::TEXT_MUTED() } else { theme::TEXT() };
    p.rect_filled(r, CornerRadius::same(6), theme::overlay(if hl { 34 } else { 20 }));
    p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, ink);
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

fn detail(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)]) {
    let Some((cours, start)) = st.detail else { return };
    let Some((_, s)) = seances.iter().find(|(c, s)| *c == cours && s.start == start) else {
        st.detail = None;
        return;
    };
    let mut go = None;
    widgets::card(ui, "", 0.0, |ui| {
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
    });
    if let Some((kind, k)) = go {
        // Même semaine, autre élément.
        let week = st.week;
        st.open(kind, k);
        st.week = week;
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
