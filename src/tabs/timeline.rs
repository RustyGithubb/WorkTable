//! Quadrillage des jours (tableau de bord) : un carré par jour, du premier au dernier jour réglés
//! dans Réglages › Quadrillage (par défaut, l'année scolaire française en cours).
//!
//! - jours écoulés en gris clair, jours à venir en blanc, week-ends et vacances (déclarées depuis
//!   le calendrier ou les Réglages) en gris foncé ;
//! - jours des événements mis en évidence (bouton « Mettre en évidence » d'un événement du
//!   calendrier) dans la couleur du panneau « Liens Sites Important » ;
//! - au survol d'un carré : la date, sa place dans le quadrillage et les événements mis en
//!   évidence ce jour-là (cliquables : ouverts dans le calendrier) ;
//! - clic sur un carré : ce jour dans le calendrier, sans changer sa vue (semaine, mois…).

use std::collections::BTreeMap;

use chrono::{Datelike, Duration, Local, NaiveDate, Weekday};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::google::calendar::CalEvent;
use crate::prefs::{Highlight, Holiday, Prefs, TimelinePrefs};
use crate::tabs::calendar::{self, CalendarState};
use crate::theme;

/// Couleur des jours mis en évidence : celle du panneau « Liens Sites Important ».
pub const ACCENT: Color32 = Color32::from_rgb(0xD0, 0x6A, 0x4E);
const WEEKEND: Color32 = Color32::from_rgb(0x34, 0x34, 0x34);
const ELAPSED: Color32 = Color32::from_rgb(0x8C, 0x8C, 0x8C);
const AHEAD: Color32 = Color32::from_rgb(0xEE, 0xEE, 0xEE);
/// Au-delà, le quadrillage est tronqué (carrés trop petits pour être lisibles).
const MAX_DAYS: i64 = 800;

/// Année scolaire française qui contient `today` (ou la prochaine, l'été) : du 1er septembre au
/// premier samedi de juillet (début des grandes vacances).
pub fn school_year(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let year = if today.month() >= 8 { today.year() } else { today.year() - 1 };
    let start = NaiveDate::from_ymd_opt(year, 9, 1).expect("1er septembre");
    let mut end = NaiveDate::from_ymd_opt(year + 1, 7, 1).expect("1er juillet");
    while end.weekday() != Weekday::Sat {
        end += Duration::days(1);
    }
    (start, end)
}

/// « 2026-09-01 » ou « 01/09/2026 ».
pub fn parse_date(s: &str) -> Option<NaiveDate> {
    let s = s.trim();
    NaiveDate::parse_from_str(s, "%Y-%m-%d").or_else(|_| NaiveDate::parse_from_str(s, "%d/%m/%Y")).ok()
}

/// Premier et dernier jour du quadrillage (réglés, sinon l'année scolaire en cours).
pub fn range(t: &TimelinePrefs) -> (NaiveDate, NaiveDate) {
    let (ds, de) = school_year(Local::now().date_naive());
    let start = parse_date(&t.start).unwrap_or(ds);
    let end = parse_date(&t.end).unwrap_or(de);
    if end < start { (ds, de) } else { (start, end.min(start + Duration::days(MAX_DAYS - 1))) }
}

pub fn title(t: &TimelinePrefs) -> String {
    if t.title.trim().is_empty() { t!("Année scolaire", "School year").to_string() } else { t.title.trim().to_string() }
}

// --- Mise en évidence ------------------------------------------------------------------------

fn snapshot(e: &CalEvent) -> Highlight {
    Highlight { calendar: e.calendar.clone(), id: e.id.clone(), title: e.title.clone(), start: e.start, end: e.end, all_day: e.all_day }
}

// --- Bases de jours mis en évidence -------------------------------------------------------------
// `None` : la base d'origine (quadrillage du tableau de bord) ; `Some(id)` : une autre base.

/// Bases existantes : identifiant et nom.
pub fn bases(prefs: &Prefs) -> Vec<(Option<u64>, String)> {
    std::iter::once((None, title(&prefs.timeline))).chain(prefs.highlight_sets.iter().map(|s| (Some(s.id), s.name.clone()))).collect()
}

fn base(prefs: &Prefs, b: Option<u64>) -> Option<&Vec<Highlight>> {
    match b {
        None => Some(&prefs.timeline.highlights),
        Some(id) => prefs.highlight_sets.iter().find(|s| s.id == id).map(|s| &s.highlights),
    }
}

fn base_mut(prefs: &mut Prefs, b: Option<u64>) -> Option<&mut Vec<Highlight>> {
    match b {
        None => Some(&mut prefs.timeline.highlights),
        Some(id) => prefs.highlight_sets.iter_mut().find(|s| s.id == id).map(|s| &mut s.highlights),
    }
}

/// L'événement est-il dans cette base ?
pub fn in_base(prefs: &Prefs, b: Option<u64>, e: &CalEvent) -> bool {
    base(prefs, b).is_some_and(|l| l.iter().any(|h| h.id == e.id && h.calendar == e.calendar))
}

/// Ajoute l'événement à la base, ou l'en retire.
pub fn toggle_in(prefs: &mut Prefs, b: Option<u64>, e: &CalEvent) {
    let on = in_base(prefs, b, e);
    if let Some(l) = base_mut(prefs, b) {
        if on {
            l.retain(|h| !(h.id == e.id && h.calendar == e.calendar));
        } else {
            l.push(snapshot(e));
        }
    }
}

/// Nouvelle base (vide) : son identifiant.
pub fn new_base(prefs: &mut Prefs, name: &str) -> u64 {
    let id = prefs.highlight_sets.iter().map(|s| s.id).max().unwrap_or(0) + 1;
    prefs.highlight_sets.push(crate::prefs::HighlightSet { id, name: name.trim().to_string(), highlights: Vec::new() });
    id
}

/// Les copies suivent l'événement quand il est chargé (déplacé, renommé), dans toutes les bases.
pub fn sync_all(cal: &CalendarState, prefs: &mut Prefs) {
    sync(cal, &mut prefs.timeline);
    for s in prefs.highlight_sets.iter_mut() {
        sync_list(cal, &mut s.highlights);
    }
}

/// Les copies suivent l'événement quand il est chargé (déplacé, renommé).
pub fn sync(cal: &CalendarState, t: &mut TimelinePrefs) {
    sync_list(cal, &mut t.highlights);
}

fn sync_list(cal: &CalendarState, list: &mut [Highlight]) {
    for h in list.iter_mut() {
        if let Some(e) = cal.events.get(&h.calendar).and_then(|l| l.iter().find(|e| e.id == h.id)) {
            let fresh = snapshot(e);
            if *h != fresh {
                *h = fresh;
            }
        }
    }
}

/// Jours couverts par un événement (la fin est exclue).
fn days_of(h: &Highlight) -> impl Iterator<Item = NaiveDate> {
    let first = calendar::date_of(h.start);
    let last = calendar::date_of((h.end - 1).max(h.start));
    first.iter_days().take_while(move |d| *d <= last)
}

/// L'événement complet s'il est chargé, sinon reconstruit depuis la copie.
fn event_of(cal: &CalendarState, h: &Highlight) -> CalEvent {
    cal.events.get(&h.calendar).and_then(|l| l.iter().find(|e| e.id == h.id)).cloned().unwrap_or_else(|| CalEvent {
        id: h.id.clone(),
        calendar: h.calendar.clone(),
        title: h.title.clone(),
        start: h.start,
        end: h.end,
        all_day: h.all_day,
        ..Default::default()
    })
}

// --- Vacances ---------------------------------------------------------------------------------

/// Périodes valides, triées : (début, fin incluse, nom).
fn periods(t: &TimelinePrefs) -> Vec<(NaiveDate, NaiveDate, String)> {
    let mut v: Vec<_> = t
        .holidays
        .iter()
        .filter_map(|h| {
            let (a, b) = (parse_date(&h.start)?, parse_date(&h.end)?);
            Some((a.min(b), a.max(b), h.name.trim().to_string()))
        })
        .collect();
    v.sort_by_key(|p| (p.0, p.1));
    v
}

/// Réécrit les périodes : fusion des chevauchements et des périodes qui se touchent (le premier
/// nom non vide est gardé).
fn store(t: &mut TimelinePrefs, mut v: Vec<(NaiveDate, NaiveDate, String)>) {
    v.sort_by_key(|p| (p.0, p.1));
    let mut out: Vec<(NaiveDate, NaiveDate, String)> = Vec::new();
    for (a, b, name) in v {
        match out.last_mut() {
            Some(last) if a <= last.1 + Duration::days(1) => {
                last.1 = last.1.max(b);
                if last.2.is_empty() {
                    last.2 = name;
                }
            }
            _ => out.push((a, b, name)),
        }
    }
    let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    t.holidays = out.into_iter().map(|(a, b, name)| Holiday { start: fmt(a), end: fmt(b), name }).collect();
}

/// Période de vacances contenant `d` : son nom (vide s'il n'en a pas).
pub fn holiday_on(t: &TimelinePrefs, d: NaiveDate) -> Option<String> {
    periods(t).into_iter().find(|p| p.0 <= d && d <= p.1).map(|p| p.2)
}

/// Marque (ou retire) les jours de `a` à `b` inclus comme vacances ; retirer coupe les périodes.
pub fn set_holidays(t: &mut TimelinePrefs, a: NaiveDate, b: NaiveDate, on: bool, name: &str) {
    let (a, b) = (a.min(b), a.max(b));
    let mut v = Vec::new();
    for (s, e, n) in periods(t) {
        if on || e < a || s > b {
            v.push((s, e, n));
            continue;
        }
        if s < a {
            v.push((s, a - Duration::days(1), n.clone()));
        }
        if e > b {
            v.push((b + Duration::days(1), e, n));
        }
    }
    if on {
        v.push((a, b, name.trim().to_string()));
    }
    store(t, v);
}

fn requests_id() -> egui::Id {
    egui::Id::unique("wt_holiday_requests")
}

/// Menu contextuel d'un jour du calendrier : marquer / retirer le jour ou sa semaine. Les vues
/// n'ont pas accès aux réglages en écriture : la demande est appliquée par [`apply_requests`].
pub fn day_menu(ui: &mut egui::Ui, t: &TimelinePrefs, d: NaiveDate) {
    let on = holiday_on(t, d).is_some();
    let mut req = None;
    if ui
        .button(if on { t!("Retirer ce jour des vacances", "Remove this day from the holidays") } else { t!("Marquer ce jour comme vacances", "Mark this day as holiday") })
        .clicked()
    {
        req = Some((d, d, !on));
    }
    let mon = d - Duration::days(d.weekday().num_days_from_monday() as i64);
    let sun = mon + Duration::days(6);
    let week_on = (0..7).all(|k| holiday_on(t, mon + Duration::days(k)).is_some());
    if ui
        .button(if week_on {
            t!("Retirer la semaine des vacances", "Remove the week from the holidays")
        } else {
            t!("Marquer la semaine comme vacances", "Mark the week as holiday")
        })
        .clicked()
    {
        req = Some((mon, sun, !week_on));
    }
    if let Some(r) = req {
        ui.ctx().data_mut(|m| m.get_temp_mut_or_default::<Vec<(NaiveDate, NaiveDate, bool)>>(requests_id()).push(r));
        ui.ctx().request_repaint();
        ui.close();
    }
}

/// Applique les demandes de [`day_menu`] (appelé par l'onglet Calendrier, qui tient les réglages).
pub fn apply_requests(ctx: &egui::Context, t: &mut TimelinePrefs) {
    let reqs: Vec<(NaiveDate, NaiveDate, bool)> = ctx.data_mut(|m| m.remove_temp(requests_id())).unwrap_or_default();
    for (a, b, on) in reqs {
        set_holidays(t, a, b, on, "");
    }
}

// --- Panneau ----------------------------------------------------------------------------------

/// Avancement (pastille à côté du titre, courte : le panneau est étroit) : « 18 % ».
pub fn badge(t: &TimelinePrefs) -> String {
    let (start, end) = range(t);
    let today = Local::now().date_naive();
    let n = (end - start).num_days() + 1;
    if today < start {
        tf!("dans {} j", "in {} d", (start - today).num_days())
    } else if today > end {
        t!("terminé", "finished").to_string()
    } else {
        let k = (today - start).num_days() + 1;
        format!("{} %", k * 100 / n)
    }
}

/// Survol en cours : jour, carré et carte affichée (pour la garder ouverte pendant qu'on la
/// rejoint avec la souris).
#[derive(Clone, Copy)]
struct Hover {
    day: NaiveDate,
    cell: Rect,
    card: Rect,
}

/// Clic dans le quadrillage : un jour (carré) ou un événement (carte de survol).
pub enum Pick {
    Day(NaiveDate),
    Event(Box<CalEvent>),
}

/// Le quadrillage avec les jours mis en évidence d'une base (`highlights`) ; dates et vacances
/// du quadrillage d'origine.
pub fn show_with(ui: &mut egui::Ui, cal: &CalendarState, prefs: &Prefs, highlights: &[Highlight]) -> Option<Pick> {
    let t = &prefs.timeline;
    let (start, end) = range(t);
    let n = (end - start).num_days() + 1;
    let today = Local::now().date_naive();
    // Jours mis en évidence → événements.
    let off = periods(t);
    let mut marked: BTreeMap<NaiveDate, Vec<&Highlight>> = BTreeMap::new();
    for h in highlights {
        for d in days_of(h).filter(|d| *d >= start && *d <= end) {
            marked.entry(d).or_default().push(h);
        }
    }

    // Place : légende en bas, le reste pour le quadrillage.
    let area = ui.available_rect_before_wrap();
    // Légende (mesurée d'abord : le quadrillage prend le reste), dessinée à la main, avec retour à la ligne : elle ne doit pas élargir le panneau.
    let items = [
        (ELAPSED, t!("Écoulés", "Past")),
        (AHEAD, t!("À venir", "Ahead")),
        (WEEKEND, t!("Week-ends, vacances", "Weekends, holidays")),
        (ACCENT, t!("Mis en évidence", "Highlighted")),
    ];
    let font = FontId::proportional(10.5);
    let galleys: Vec<_> = items.iter().map(|(_, l)| ui.painter().layout_no_wrap(l.to_string(), font.clone(), theme::TEXT_FAINT())).collect();
    let (mut x, mut y, line) = (0.0_f32, 0.0_f32, 15.0);
    let mut spots = Vec::new();
    for g in &galleys {
        let w = 13.0 + g.size().x;
        if x > 0.0 && x + w > area.width() {
            x = 0.0;
            y += line;
        }
        spots.push(vec2(x, y));
        x += w + 10.0;
    }
    let spacing = ui.spacing().item_spacing.y;
    let grid_area = Rect::from_min_max(area.min, Pos2::new(area.right(), area.bottom() - (y + line) - 6.0 - 2.0 * spacing));
    let lead = start.weekday().num_days_from_monday() as i64;
    let total = lead + n;
    // Colonnes par semaines entières : la taille de carré la plus grande qui tient.
    let gap = 2.0;
    let (cols, size) = (1..=12)
        .map(|w| {
            let cols = 7 * w;
            let rows = (total + cols - 1) / cols;
            let s = ((grid_area.width() - gap * (cols - 1) as f32) / cols as f32).min((grid_area.height() - gap * (rows - 1) as f32) / rows as f32);
            (cols, s)
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .unwrap_or((7, 10.0));
    let size = size.clamp(2.0, 28.0).floor();
    let step = size + gap;
    let rows = (total + cols - 1) / cols;
    let used = vec2(cols as f32 * step - gap, rows as f32 * step - gap);
    let origin = Pos2::new(grid_area.center().x - used.x / 2.0, grid_area.top());
    let (resp_rect, resp) = ui.allocate_exact_size(vec2(area.width(), used.y), Sense::click());
    let _ = resp_rect;
    let cell_of = |i: i64| -> Rect {
        let k = lead + i;
        Rect::from_min_size(origin + vec2((k % cols) as f32 * step, (k / cols) as f32 * step), vec2(size, size))
    };
    let p = ui.painter();
    let round = CornerRadius::same((size * 0.22).round().clamp(1.0, 5.0) as u8);
    for i in 0..n {
        let d = start + Duration::days(i);
        let r = cell_of(i);
        let weekend = d.weekday().num_days_from_monday() >= 5 || off.iter().any(|p| p.0 <= d && d <= p.1);
        let fill = if marked.contains_key(&d) {
            if d < today { ACCENT.gamma_multiply(0.75) } else { ACCENT }
        } else if weekend {
            WEEKEND
        } else if d < today {
            ELAPSED
        } else {
            AHEAD
        };
        p.rect_filled(r, round, fill);
        if d == today {
            p.rect_stroke(r.expand(1.5), round, Stroke::new(1.6, ACCENT), egui::StrokeKind::Outside);
        }
    }

    // Survol : carré sous la souris, ou carte encore survolée.
    let hover_id = egui::Id::unique("wt_timeline_hover");
    let prev: Option<Hover> = ui.data(|d| d.get_temp(hover_id)).flatten();
    let pointer = ui.ctx().pointer_hover_pos();
    let under = pointer.filter(|_| resp.hovered() || resp.contains_pointer()).and_then(|q| {
        let rel = q - origin;
        if rel.x < 0.0 || rel.y < 0.0 {
            return None;
        }
        let (cx, cy) = ((rel.x / step) as i64, (rel.y / step) as i64);
        // Dans le carré, pas dans l'espace entre deux carrés.
        if cx >= cols || rel.x - cx as f32 * step > size || rel.y - cy as f32 * step > size {
            return None;
        }
        let i = cy * cols + cx - lead;
        (0..n).contains(&i).then_some(i)
    });
    let mut hover = match (under, prev) {
        (Some(i), prev) => {
            let day = start + Duration::days(i);
            let card = prev.filter(|h| h.day == day).map(|h| h.card).unwrap_or(Rect::NOTHING);
            Some(Hover { day, cell: cell_of(i), card })
        }
        (None, Some(h)) if pointer.is_some_and(|q| h.card.expand(6.0).contains(q) || h.cell.union(h.card).expand(2.0).contains(q)) => Some(h),
        _ => None,
    };
    let mut clicked = None;
    if let Some(h) = hover.as_mut() {
        p.rect_stroke(h.cell.expand(1.0), round, Stroke::new(1.5, theme::TEXT()), egui::StrokeKind::Outside);
        let i = (h.day - start).num_days();
        let events = marked.get(&h.day).cloned().unwrap_or_default();
        let holiday = off.iter().find(|p| p.0 <= h.day && h.day <= p.1).map(|p| p.2.as_str());
        let (card, ev) = card(ui.ctx(), h, i, n, lead, today, &events, holiday, cal);
        h.card = card;
        clicked = ev.map(|e| Pick::Event(Box::new(e)));
        ui.ctx().request_repaint();
    }
    // Clic sur un carré : ce jour dans le calendrier.
    if let Some(i) = under {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        if resp.clicked() {
            clicked = Some(Pick::Day(start + Duration::days(i)));
        }
    }
    ui.data_mut(|d| d.insert_temp(hover_id, hover));

    // Légende.
    ui.add_space((grid_area.height() - used.y).max(0.0) + 6.0 - spacing);
    let (r, _) = ui.allocate_exact_size(vec2(area.width(), y + line), Sense::hover());
    for (((c, _), g), at) in items.iter().zip(galleys).zip(spots) {
        let o = r.min + at;
        ui.painter().rect_filled(Rect::from_min_size(o + vec2(0.0, 3.0), vec2(9.0, 9.0)), CornerRadius::same(2), *c);
        ui.painter().galley(o + vec2(13.0, 0.0), g, theme::TEXT_FAINT());
    }
    clicked
}

/// Carte de survol d'un jour ; renvoie son rectangle et l'événement cliqué.
#[allow(clippy::too_many_arguments)]
fn card(
    ctx: &egui::Context,
    h: &Hover,
    i: i64,
    n: i64,
    lead: i64,
    today: NaiveDate,
    events: &[&Highlight],
    holiday: Option<&str>,
    cal: &CalendarState,
) -> (Rect, Option<CalEvent>) {
    let screen = ctx.content_rect();
    let w = 250.0;
    // À droite du carré, sinon à gauche ; un peu chevauchante pour la rejoindre sans la perdre.
    let x = if h.cell.right() + w + 4.0 < screen.right() { h.cell.right() + 2.0 } else { h.cell.left() - w - 2.0 };
    let pos = Pos2::new(x, h.cell.top() - 6.0);
    let mut clicked = None;
    let resp = egui::Area::new(egui::Id::unique("wt_timeline_card")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        egui::Frame::new()
            .fill(theme::SURFACE())
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(10))
            .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) })
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(w - 20.0);
                ui.label(RichText::new(super::long_date(h.day)).strong().color(theme::TEXT()));
                let week = (lead + i) / 7 + 1;
                ui.label(RichText::new(tf!("Jour {} sur {} · semaine {week}", "Day {} of {} · week {week}", i + 1, n)).small().color(theme::TEXT_MUTED()));
                let left = n - 1 - i;
                let status = if h.day == today {
                    t!("Aujourd'hui", "Today").to_string()
                } else if h.day < today {
                    tf!("Il y a {} j", "{} d ago", (today - h.day).num_days())
                } else {
                    tf!("Dans {} j", "In {} d", (h.day - today).num_days())
                };
                let rest = if left == 0 { t!("dernier jour", "last day").to_string() } else { tf!("{left} j restants après", "{left} d left after") };
                ui.label(RichText::new(format!("{status} · {rest}")).small().color(theme::TEXT_FAINT()));
                if let Some(name) = holiday {
                    let label = if name.is_empty() { t!("Vacances", "Holidays").to_string() } else { tf!("Vacances : {name}", "Holidays: {name}") };
                    ui.label(RichText::new(label).small().strong().color(theme::TEXT_MUTED()));
                }
                if !events.is_empty() {
                    ui.add_space(6.0);
                    for e in events {
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Ouvrir dans le calendrier", "Open in the calendar"));
                        let pa = ui.painter();
                        pa.rect_filled(r, CornerRadius::same(6), if resp.hovered() { ACCENT.gamma_multiply(0.35) } else { ACCENT.gamma_multiply(0.18) });
                        pa.rect_filled(Rect::from_min_size(r.min + vec2(6.0, 7.0), vec2(3.0, r.height() - 14.0)), CornerRadius::same(2), ACCENT);
                        let title = if e.title.trim().is_empty() { t!("(Sans titre)", "(Untitled)") } else { e.title.as_str() };
                        let g = crate::tabs::mail::view::one_line(pa, title, theme::editor_font(13.0, true), theme::TEXT(), r.width() - 24.0);
                        pa.galley(Pos2::new(r.left() + 16.0, r.top() + 3.0), g, theme::TEXT());
                        let when = if e.all_day { t!("Journée entière", "All day").to_string() } else { format!("{} – {}", calendar::hhmm(e.start), calendar::hhmm(e.end)) };
                        pa.text(Pos2::new(r.left() + 16.0, r.top() + 20.0), Align2::LEFT_TOP, when, FontId::proportional(11.0), theme::TEXT_MUTED());
                        if resp.clicked() {
                            clicked = Some(event_of(cal, e));
                        }
                    }
                }
            });
    });
    (resp.response.rect, clicked)
}

// --- Réglages ---------------------------------------------------------------------------------

/// Carte « Quadrillage » des Réglages : titre, premier et dernier jour, mises en évidence.
pub fn settings(ui: &mut egui::Ui, prefs: &mut Prefs) {
    crate::widgets::card(ui, t!("Quadrillage (tableau de bord)", "Day grid (dashboard)"), 0.0, |ui| {
        let t = &mut prefs.timeline;
        ui.label(
            RichText::new(t!(
                "Un carré par jour, à côté du planning. Vides, les dates suivent l'année scolaire française en cours (1er septembre → début des grandes vacances).",
                "One square per day, next to the schedule. Left empty, the dates follow the current French school year (1 September → start of the summer holidays)."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        ui.add_space(8.0);
        let (ds, de) = range(t);
        egui::Grid::new("wt_timeline_settings").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label(t!("Titre", "Title"));
            ui.add(egui::TextEdit::singleline(&mut t.title).hint_text(t!("Année scolaire", "School year")).desired_width(220.0));
            ui.end_row();
            for (label, value, shown) in [(t!("Premier jour", "First day"), &mut t.start, ds), (t!("Dernier jour", "Last day"), &mut t.end, de)] {
                ui.label(label);
                date_field(ui, label, value, shown);
                ui.end_row();
            }
        });
        let (s, e) = range(t);
        ui.add_space(4.0);
        ui.label(
            RichText::new(tf!("{} jours, du {} au {}", "{} days, from {} to {}", (e - s).num_days() + 1, super::long_date(s), super::long_date(e)))
                .small()
                .color(theme::TEXT_MUTED()),
        );
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui.button(t!("Année scolaire en cours", "Current school year")).clicked() {
                t.start.clear();
                t.end.clear();
            }
            let k = t.highlights.len();
            if k > 0 && ui.button(tf!("Retirer les mises en évidence ({k})", "Remove the highlights ({k})")).clicked() {
                t.highlights.clear();
            }
        });
        ui.label(
            RichText::new(t!(
                "Pour colorer un jour : ouvre un événement dans le Calendrier › « Mettre en évidence ».",
                "To colour a day: open an event in the Calendar › “Highlight”."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        holidays_settings(ui, t);
    });
    ui.add_space(12.0);
}

/// Vacances (Réglages › Quadrillage) : liste des périodes et ajout d'une période.
fn holidays_settings(ui: &mut egui::Ui, t: &mut TimelinePrefs) {
    ui.add_space(12.0);
    ui.label(RichText::new(t!("Vacances", "Holidays")).strong().color(theme::TEXT()));
    ui.label(
        RichText::new(t!("Affichées comme les week-ends. Aussi depuis le Calendrier : clic droit sur un jour.", "Shown like weekends. Also from the Calendar: right-click a day."))
            .small()
            .color(theme::TEXT_FAINT()),
    );
    ui.add_space(4.0);
    let mut remove = None;
    for (k, (a, b, name)) in periods(t).into_iter().enumerate() {
        ui.horizontal(|ui| {
            let days = (b - a).num_days() + 1;
            let when = if a == b { a.format("%d/%m/%Y").to_string() } else { format!("{} → {}", a.format("%d/%m/%Y"), b.format("%d/%m/%Y")) };
            ui.label(RichText::new(when).color(theme::TEXT()));
            ui.label(RichText::new(tf!("{days} j", "{days} d")).small().color(theme::TEXT_MUTED()));
            if !name.is_empty() {
                ui.label(RichText::new(name).color(theme::TEXT_MUTED()));
            }
            if ui.small_button("✕").on_hover_text(t!("Supprimer cette période", "Delete this period")).clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        let mut v = periods(t);
        v.remove(k);
        store(t, v);
    }
    // Ajout : du / au / nom.
    let id = egui::Id::unique("wt_holiday_form");
    let mut form: (String, String, String) = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let today = Local::now().date_naive();
    ui.horizontal_wrapped(|ui| {
        ui.label(t!("Du", "From"));
        date_field(ui, "holiday_from", &mut form.0, today);
        ui.label(t!("au", "to"));
        let to_hint = parse_date(&form.0).unwrap_or(today);
        date_field(ui, "holiday_to", &mut form.1, to_hint);
        ui.add(egui::TextEdit::singleline(&mut form.2).hint_text(t!("Nom (facultatif)", "Name (optional)")).desired_width(140.0));
        let from = parse_date(&form.0);
        let to = parse_date(&form.1).or(from);
        if ui.add_enabled(from.is_some(), egui::Button::new(t!("Ajouter", "Add"))).clicked() {
            if let (Some(a), Some(b)) = (from, to) {
                set_holidays(t, a, b, true, &form.2);
            }
            form = Default::default();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, form));
}

/// Champ de date « JJ/MM/AAAA » : enregistré (« AAAA-MM-JJ ») dès qu'il est valide ; vide = défaut.
fn date_field(ui: &mut egui::Ui, salt: &str, value: &mut String, shown: NaiveDate) {
    let id = egui::Id::unique(("wt_timeline_date", salt));
    let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| parse_date(value).map(|d| d.format("%d/%m/%Y").to_string()).unwrap_or_default());
    let resp = ui.add(egui::TextEdit::singleline(&mut text).hint_text(shown.format("%d/%m/%Y").to_string()).desired_width(120.0));
    let bad = !text.trim().is_empty() && parse_date(&text).is_none();
    if resp.changed() {
        if text.trim().is_empty() {
            value.clear();
        } else if let Some(d) = parse_date(&text) {
            *value = d.format("%Y-%m-%d").to_string();
        }
    }
    if bad {
        ui.label(RichText::new(t!("Format : JJ/MM/AAAA", "Format: DD/MM/YYYY")).small().color(theme::TODAY()));
    }
    // Le texte tapé reste tel quel pendant la saisie ; hors saisie, il suit la valeur enregistrée.
    if resp.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
    } else {
        ui.data_mut(|d| d.remove::<String>(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, day).unwrap()
    }

    #[test]
    fn french_school_year() {
        // Rentrée 2026 : 1er septembre ; vacances d'été 2027 : samedi 3 juillet.
        assert_eq!(school_year(d(2026, 10, 25)), (d(2026, 9, 1), d(2027, 7, 3)));
        assert_eq!(school_year(d(2027, 3, 1)), (d(2026, 9, 1), d(2027, 7, 3)));
        // L'été : l'année qui arrive.
        assert_eq!(school_year(d(2026, 8, 20)).0, d(2026, 9, 1));
        // 2025-2026 : fin le samedi 4 juillet 2026.
        assert_eq!(school_year(d(2026, 1, 10)).1, d(2026, 7, 4));
    }

    #[test]
    fn dates_and_range() {
        assert_eq!(parse_date("01/09/2026"), Some(d(2026, 9, 1)));
        assert_eq!(parse_date("2026-09-01"), Some(d(2026, 9, 1)));
        assert_eq!(parse_date("31/02/2026"), None);
        let t = TimelinePrefs { start: "2026-01-05".into(), end: "2026-01-11".into(), ..Default::default() };
        assert_eq!(range(&t), (d(2026, 1, 5), d(2026, 1, 11)));
        // Fin avant le début : retour à l'année scolaire.
        let t = TimelinePrefs { start: "2026-05-01".into(), end: "2026-01-01".into(), ..Default::default() };
        assert_eq!(range(&t), school_year(Local::now().date_naive()));
    }

    #[test]
    fn highlight_covers_its_days() {
        use chrono::TimeZone;
        let ms = |y, m, day| Local.with_ymd_and_hms(y, m, day, 0, 0, 0).unwrap().timestamp_millis();
        // Journée entière du 3 au 4 (fin exclue le 5 à minuit).
        let h = Highlight { start: ms(2026, 11, 3), end: ms(2026, 11, 5), all_day: true, ..Default::default() };
        assert_eq!(days_of(&h).collect::<Vec<_>>(), vec![d(2026, 11, 3), d(2026, 11, 4)]);
        let mut p = Prefs::default();
        let e = CalEvent { id: "e".into(), calendar: "c".into(), ..Default::default() };
        toggle_in(&mut p, None, &e);
        assert!(in_base(&p, None, &e));
        // Une autre base est indépendante de celle d'origine.
        let b = new_base(&mut p, "Prod PFE");
        assert!(!in_base(&p, Some(b), &e));
        toggle_in(&mut p, Some(b), &e);
        assert!(in_base(&p, Some(b), &e));
        toggle_in(&mut p, None, &e);
        assert!(!in_base(&p, None, &e) && in_base(&p, Some(b), &e));
    }

    #[test]
    fn holidays_merge_and_split() {
        let mut t = TimelinePrefs::default();
        set_holidays(&mut t, d(2026, 10, 17), d(2026, 10, 25), true, "Toussaint");
        set_holidays(&mut t, d(2026, 10, 26), d(2026, 11, 1), true, "");
        // Périodes contiguës fusionnées, nom gardé.
        assert_eq!(t.holidays.len(), 1);
        assert_eq!(holiday_on(&t, d(2026, 11, 1)).as_deref(), Some("Toussaint"));
        assert_eq!(holiday_on(&t, d(2026, 11, 2)), None);
        // Retirer un jour au milieu coupe la période en deux.
        set_holidays(&mut t, d(2026, 10, 20), d(2026, 10, 20), false, "");
        assert_eq!(t.holidays.len(), 2);
        assert_eq!(holiday_on(&t, d(2026, 10, 20)), None);
        assert!(holiday_on(&t, d(2026, 10, 19)).is_some() && holiday_on(&t, d(2026, 10, 21)).is_some());
        // Ajouter le jour retiré refusionne ; dates inversées acceptées.
        set_holidays(&mut t, d(2026, 10, 20), d(2026, 10, 20), true, "");
        assert_eq!(t.holidays.len(), 1);
        set_holidays(&mut t, d(2026, 11, 1), d(2026, 10, 17), false, "");
        assert!(t.holidays.is_empty());
    }
}
