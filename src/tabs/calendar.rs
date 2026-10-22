//! Onglet Calendrier, inspiré de Notion Calendar et branché sur Google Agenda :
//! mini-calendrier et agendas masquables à gauche, grille heure par heure (1 à 7 jours)
//! ou vue mois au centre, panneau de détail à droite ; création par clic ou glisser,
//! déplacement / redimensionnement à la souris, raccourcis clavier.

mod grid;
mod month;
mod panel;
mod side;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as CDuration, Local, NaiveDate, TimeZone, Timelike};
use eframe::egui::{self, Align, Key, Layout, RichText, vec2};

use crate::google::calendar::{self, CalEvent, CalendarInfo};
use crate::google::{Account, Event, Status};
use crate::prefs::Prefs;
use crate::tabs::mail::contacts::Contacts;
use crate::tabs::mail::recipients::Suggest;
use crate::{theme, widgets};

const DAY_MS: i64 = 86_400_000;
/// Rechargement automatique des événements affichés.
const REFRESH_EVERY: Duration = Duration::from_secs(180);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum View {
    /// 1 à 7 jours côte à côte (7 = semaine du lundi au dimanche).
    Days(u8),
    Month,
}

/// Événement en cours d'édition dans le panneau de droite.
pub struct Editor {
    pub ev: CalEvent,
    /// `None` = nouvel événement.
    pub original: Option<CalEvent>,
    pub date: String,
    pub end_date: String,
    pub start_time: String,
    pub end_time: String,
    pub attendees: String,
    pub add_meet: bool,
    pub focus_title: bool,
    pub suggest: Suggest,
    pub error: Option<String>,
    pub saving: bool,
    pub confirm_delete: bool,
}

/// Geste souris en cours sur la grille.
#[derive(Clone, Debug)]
pub enum Drag {
    /// Création : jour et minutes de début / courante.
    Create { day: NaiveDate, from: i32, to: i32 },
    /// Déplacement : événement d'origine, décalage de saisie (min), position courante.
    Move { orig: CalEvent, grab: i32, day: NaiveDate, start: i32 },
    /// Redimensionnement par le bas.
    Resize { orig: CalEvent, end: i32 },
}

pub struct CalendarState {
    pub calendars: Vec<CalendarInfo>,
    pub events: HashMap<String, Vec<CalEvent>>,
    loaded: HashMap<String, Vec<(i64, i64)>>,
    pending: HashSet<(String, i64)>,
    pub view: View,
    /// Premier jour affiché (lundi en vue semaine, 1er du mois en vue mois).
    pub anchor: NaiveDate,
    /// Mois affiché dans le mini-calendrier.
    pub mini_month: NaiveDate,
    pub selected: Option<(String, String)>,
    pub editor: Option<Editor>,
    pub drag: Option<Drag>,
    pub loading_calendars: bool,
    pub need_consent: bool,
    pub api_disabled: bool,
    pub notice: Option<(String, bool, Instant)>,
    pub scroll_to_now: bool,
    last_refresh: Option<Instant>,
    last_calendar_attempt: Option<Instant>,
    view_loaded_from_prefs: bool,
}

impl Default for CalendarState {
    fn default() -> Self {
        let today = Local::now().date_naive();
        CalendarState {
            calendars: Vec::new(),
            events: HashMap::new(),
            loaded: HashMap::new(),
            pending: HashSet::new(),
            view: View::Days(7),
            anchor: monday(today),
            mini_month: today.with_day(1).unwrap_or(today),
            selected: None,
            editor: None,
            drag: None,
            loading_calendars: false,
            need_consent: false,
            api_disabled: false,
            notice: None,
            scroll_to_now: true,
            last_refresh: None,
            last_calendar_attempt: None,
            view_loaded_from_prefs: false,
        }
    }
}

// --- Dates -----------------------------------------------------------------------

pub fn monday(d: NaiveDate) -> NaiveDate {
    d - CDuration::days(d.weekday().num_days_from_monday() as i64)
}

/// Minuit local du jour `d`, en ms.
pub fn day_start(d: NaiveDate) -> i64 {
    Local.from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap()).earliest().map(|t| t.timestamp_millis()).unwrap_or(0)
}

/// Instant `minutes` après minuit le jour `d`.
pub fn at(d: NaiveDate, minutes: i32) -> i64 {
    day_start(d) + minutes as i64 * 60_000
}

pub fn date_of(ms: i64) -> NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|t| t.date_naive()).unwrap_or_default()
}

pub fn minutes_of(ms: i64) -> i32 {
    Local.timestamp_millis_opt(ms).single().map(|t| (t.hour() * 60 + t.minute()) as i32).unwrap_or(0)
}

pub fn hhmm(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default()
}

pub fn snap(minutes: i32, step: i32) -> i32 {
    ((minutes as f32 / step as f32).round() as i32 * step).clamp(0, 24 * 60)
}

fn month_title(d: NaiveDate) -> String {
    let m = super::month_name(d.month());
    let mut c = m.chars();
    let cap: String = c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default();
    format!("{cap} {}", d.year())
}

impl CalendarState {
    // --- Plage affichée ------------------------------------------------------------

    pub fn days(&self) -> Vec<NaiveDate> {
        match self.view {
            View::Days(n) => (0..n as i64).map(|i| self.anchor + CDuration::days(i)).collect(),
            View::Month => {
                let start = monday(self.anchor);
                (0..42).map(|i| start + CDuration::days(i)).collect()
            }
        }
    }

    pub fn range(&self) -> (i64, i64) {
        let days = self.days();
        (day_start(days[0]), day_start(*days.last().unwrap()) + DAY_MS)
    }

    pub fn visible_calendars(&self, prefs: &Prefs) -> Vec<&CalendarInfo> {
        self.calendars.iter().filter(|c| !prefs.hidden_calendars.contains(&c.id)).collect()
    }

    pub fn calendar(&self, id: &str) -> Option<&CalendarInfo> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn color_of(&self, e: &CalEvent) -> [u8; 3] {
        e.color.or_else(|| self.calendar(&e.calendar).map(|c| c.color)).unwrap_or([0x4C, 0x9A, 0xFF])
    }

    /// Événements visibles qui touchent l'intervalle [from, to).
    pub fn events_between(&self, prefs: &Prefs, from: i64, to: i64) -> Vec<&CalEvent> {
        let mut out: Vec<&CalEvent> = self.visible_calendars(prefs).into_iter().filter_map(|c| self.events.get(&c.id)).flatten().filter(|e| e.start < to && e.end > from).collect();
        out.sort_by_key(|e| (e.start, -(e.end - e.start)));
        out
    }

    // --- Navigation ---------------------------------------------------------------

    pub fn set_view(&mut self, v: View) {
        let focus = self.focus_date();
        self.view = v;
        self.go_to(focus);
    }

    /// Date de référence (aujourd'hui si visible, sinon le premier jour affiché).
    fn focus_date(&self) -> NaiveDate {
        let today = Local::now().date_naive();
        let days = self.days();
        match self.view {
            View::Month if today.year() == self.anchor.year() && today.month() == self.anchor.month() => today,
            View::Month => self.anchor,
            _ if days.contains(&today) => today,
            _ => days[0],
        }
    }

    pub fn go_to(&mut self, d: NaiveDate) {
        self.anchor = match self.view {
            View::Days(7) => monday(d),
            View::Days(_) => d,
            View::Month => d.with_day(1).unwrap_or(d),
        };
        self.mini_month = d.with_day(1).unwrap_or(d);
    }

    pub fn shift(&mut self, dir: i64) {
        self.anchor = match self.view {
            View::Days(n) => self.anchor + CDuration::days(dir * n as i64),
            View::Month => {
                let m = self.anchor.month0() as i64 + dir;
                let y = self.anchor.year() as i64 + m.div_euclid(12);
                NaiveDate::from_ymd_opt(y as i32, m.rem_euclid(12) as u32 + 1, 1).unwrap_or(self.anchor)
            }
        };
        self.mini_month = self.anchor.with_day(1).unwrap_or(self.anchor);
    }

    pub fn today(&mut self) {
        self.go_to(Local::now().date_naive());
        self.scroll_to_now = true;
    }

    // --- Données -----------------------------------------------------------------

    pub fn on_ready(&mut self, acc: &Account) {
        let keep = (self.view, self.anchor, self.view_loaded_from_prefs);
        *self = CalendarState { view: keep.0, anchor: keep.1, view_loaded_from_prefs: keep.2, ..Default::default() };
        self.load_calendars(acc);
    }

    fn load_calendars(&mut self, acc: &Account) {
        self.loading_calendars = true;
        self.last_calendar_attempt = Some(Instant::now());
        acc.run(|s| {
            Ok(match calendar::calendars(s) {
                Ok(c) => Event::Calendars(c),
                Err(e) => Event::CalError(e),
            })
        });
    }

    /// À appeler à chaque frame (même hors de l'onglet) : agendas et événements à jour,
    /// pour l'onglet Calendrier et le tableau de bord. En cas d'erreur, nouvel essai après 1 min.
    pub fn background(&mut self, acc: &Account, prefs: &Prefs) {
        if acc.session().is_none() || self.need_consent || self.api_disabled {
            return;
        }
        if self.calendars.is_empty() && !self.loading_calendars && self.last_calendar_attempt.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            self.load_calendars(acc);
        }
        if self.last_refresh.is_none_or(|t| t.elapsed() > REFRESH_EVERY) {
            self.loaded.clear();
            self.last_refresh = Some(Instant::now());
        }
        self.ensure_loaded(acc, prefs);
    }

    /// Événements d'aujourd'hui (agendas visibles), pour le tableau de bord.
    pub fn today_events(&self, prefs: &Prefs) -> Vec<&CalEvent> {
        let d = Local::now().date_naive();
        self.events_between(prefs, day_start(d), day_start(d) + DAY_MS)
    }

    /// Charge (si besoin) les événements de la plage affichée, pour les agendas visibles.
    fn ensure_loaded(&mut self, acc: &Account, prefs: &Prefs) {
        if acc.session().is_none() || self.calendars.is_empty() {
            return;
        }
        let (from, to) = self.range();
        let ids: Vec<String> = self.visible_calendars(prefs).iter().map(|c| c.id.clone()).collect();
        for id in ids {
            let covered = self.loaded.get(&id).is_some_and(|v| v.iter().any(|(a, b)| *a <= from && *b >= to));
            // Marge d'une semaine de chaque côté : navigation fluide.
            let (f, t) = (from - 7 * DAY_MS, to + 7 * DAY_MS);
            if covered || !self.pending.insert((id.clone(), f)) {
                continue;
            }
            acc.run(move |s| {
                Ok(match calendar::events(s, &id, f, t) {
                    Ok(events) => Event::CalEvents { calendar: id, from: f, to: t, events },
                    Err(e) => Event::CalError(e),
                })
            });
        }
    }

    pub fn handle(&mut self, ev: Event, acc: &Account) {
        match ev {
            Event::Calendars(list) => {
                self.loading_calendars = false;
                self.need_consent = false;
                self.api_disabled = false;
                self.calendars = list;
            }
            Event::CalEvents { calendar, from, to, events } => {
                self.pending.remove(&(calendar.clone(), from));
                let list = self.events.entry(calendar.clone()).or_default();
                list.retain(|e| e.end <= from || e.start >= to);
                list.extend(events);
                self.loaded.entry(calendar).or_default().push((from, to));
            }
            Event::CalSaved { event, previous } => {
                if let Some((cal, id)) = previous
                    && let Some(l) = self.events.get_mut(&cal)
                {
                    l.retain(|e| e.id != id);
                }
                let l = self.events.entry(event.calendar.clone()).or_default();
                l.retain(|e| e.id != event.id);
                self.selected = Some((event.calendar.clone(), event.id.clone()));
                l.push(event);
                if let Some(ed) = &mut self.editor
                    && ed.saving
                {
                    self.editor = None;
                }
                self.info(t!("Événement enregistré", "Event saved"));
            }
            Event::CalDeleted { calendar, id } => {
                if let Some(l) = self.events.get_mut(&calendar) {
                    l.retain(|e| e.id != id);
                }
                self.selected = None;
                self.editor = None;
                self.info(t!("Événement supprimé", "Event deleted"));
            }
            Event::CalError(e) => {
                self.loading_calendars = false;
                self.pending.clear();
                let low = e.to_lowercase();
                if low.contains("insufficient") || low.contains("scope") {
                    self.need_consent = true;
                } else if low.contains("has not been used") || low.contains("accessnotconfigured") || low.contains("service_disabled") || low.contains("is disabled") {
                    self.api_disabled = true;
                } else {
                    if let Some(ed) = &mut self.editor {
                        ed.saving = false;
                        ed.error = Some(e.clone());
                    }
                    self.notice = Some((e, true, Instant::now()));
                }
                // Si l'agenda n'a pas pu être chargé, on recharge l'interface sans les événements.
                let _ = acc;
            }
            _ => {}
        }
    }

    fn info(&mut self, text: &str) {
        self.notice = Some((text.to_string(), false, Instant::now()));
    }

    // --- Actions sur les événements -------------------------------------------------

    pub fn default_calendar(&self, prefs: &Prefs) -> Option<&CalendarInfo> {
        let vis = self.visible_calendars(prefs);
        vis.iter().find(|c| c.primary && c.writable).or_else(|| vis.iter().find(|c| c.writable)).copied().or_else(|| self.calendars.iter().find(|c| c.primary))
    }

    /// Ouvre le panneau sur un nouvel événement [start, end).
    pub fn new_event(&mut self, prefs: &Prefs, start: i64, end: i64, all_day: bool) {
        let Some(cal) = self.default_calendar(prefs) else {
            self.notice = Some((t!("Aucun agenda modifiable", "No editable calendar").into(), true, Instant::now()));
            return;
        };
        let ev = CalEvent { calendar: cal.id.clone(), start, end, all_day, ..Default::default() };
        self.selected = None;
        self.editor = Some(Editor::new(ev, None));
    }

    /// Affiche un jour donné (vue Jour) depuis un autre onglet.
    pub fn show_date(&mut self, d: NaiveDate) {
        self.view_loaded_from_prefs = true;
        self.view = View::Days(1);
        self.go_to(d);
        self.scroll_to_now = true;
    }

    /// Ouvre un événement depuis un autre onglet : vue Jour sur sa date, panneau de détail.
    pub fn show_event(&mut self, e: &CalEvent) {
        self.view_loaded_from_prefs = true;
        self.view = View::Days(1);
        self.go_to(date_of(e.start));
        self.scroll_to_now = true;
        self.open_event(e);
    }

    pub fn open_event(&mut self, e: &CalEvent) {
        self.selected = Some((e.calendar.clone(), e.id.clone()));
        self.editor = Some(Editor::new(e.clone(), Some(e.clone())));
    }

    /// Enregistre un événement modifié à la souris (déplacement / durée) : mise à jour
    /// immédiate à l'écran, puis envoi à Google Agenda.
    pub fn commit_drag(&mut self, acc: &Account, mut e: CalEvent) {
        let original_cal = e.calendar.clone();
        if let Some(l) = self.events.get_mut(&e.calendar)
            && let Some(x) = l.iter_mut().find(|x| x.id == e.id)
        {
            *x = e.clone();
        }
        if let Some(ed) = &mut self.editor
            && ed.original.as_ref().is_some_and(|o| o.id == e.id)
        {
            *ed = Editor::new(e.clone(), Some(e.clone()));
        }
        e.attendees.retain(|a| !a.is_empty());
        acc.run(move |s| {
            Ok(match calendar::update(s, &e, &original_cal, false) {
                Ok(event) => Event::CalSaved { event, previous: None },
                Err(err) => Event::CalError(err),
            })
        });
    }

    pub fn save_editor(&mut self, acc: &Account) {
        let Some(ed) = &mut self.editor else { return };
        let ev = match ed.build() {
            Ok(e) => e,
            Err(e) => {
                ed.error = Some(e);
                return;
            }
        };
        ed.saving = true;
        ed.error = None;
        let meet = ed.add_meet;
        match ed.original.clone() {
            None => acc.run(move |s| {
                Ok(match calendar::create(s, &ev, meet) {
                    Ok(event) => Event::CalSaved { event, previous: None },
                    Err(e) => Event::CalError(e),
                })
            }),
            Some(orig) => acc.run(move |s| {
                Ok(match calendar::update(s, &ev, &orig.calendar, meet) {
                    Ok(event) => {
                        let previous = (orig.calendar != event.calendar).then(|| (orig.calendar.clone(), orig.id.clone()));
                        Event::CalSaved { event, previous }
                    }
                    Err(e) => Event::CalError(e),
                })
            }),
        }
    }

    pub fn delete_editor(&mut self, acc: &Account) {
        let Some(ed) = &mut self.editor else { return };
        let Some(orig) = ed.original.clone() else {
            self.editor = None;
            return;
        };
        ed.saving = true;
        let notify = !orig.attendees.is_empty();
        acc.run(move |s| {
            Ok(match calendar::delete(s, &orig.calendar, &orig.id, notify) {
                Ok(()) => Event::CalDeleted { calendar: orig.calendar, id: orig.id },
                Err(e) => Event::CalError(e),
            })
        });
    }
}

impl Editor {
    pub fn new(ev: CalEvent, original: Option<CalEvent>) -> Self {
        let end_for_date = if ev.all_day { ev.end - DAY_MS } else { ev.end };
        Editor {
            date: date_of(ev.start).format("%d/%m/%Y").to_string(),
            end_date: date_of(end_for_date.max(ev.start)).format("%d/%m/%Y").to_string(),
            start_time: hhmm(ev.start),
            end_time: hhmm(ev.end),
            attendees: ev.attendees.iter().map(|a| format!("{a}, ")).collect(),
            add_meet: false,
            focus_title: original.is_none(),
            suggest: Suggest::default(),
            error: None,
            saving: false,
            confirm_delete: false,
            original,
            ev,
        }
    }

    /// Construit l'événement à partir des champs saisis (dates jj/mm/aaaa, heures hh:mm).
    pub fn build(&self) -> Result<CalEvent, String> {
        let date = |s: &str| NaiveDate::parse_from_str(s.trim(), "%d/%m/%Y").map_err(|_| tf!("Date invalide : « {s} » (jj/mm/aaaa)", "Invalid date: “{s}” (dd/mm/yyyy)"));
        let time = |s: &str| -> Result<i32, String> {
            let t = s.trim().replace('h', ":");
            let (h, m) = t.split_once(':').unwrap_or((t.as_str(), "0"));
            let (h, m) = (h.trim().parse::<i32>(), if m.trim().is_empty() { Ok(0) } else { m.trim().parse::<i32>() });
            match (h, m) {
                (Ok(h), Ok(m)) if (0..=24).contains(&h) && (0..60).contains(&m) => Ok(h * 60 + m),
                _ => Err(tf!("Heure invalide : « {s} » (hh:mm)", "Invalid time: “{s}” (hh:mm)")),
            }
        };
        let mut e = self.ev.clone();
        let (d0, d1) = (date(&self.date)?, date(&self.end_date)?);
        if e.all_day {
            e.start = day_start(d0);
            e.end = day_start(d1.max(d0)) + DAY_MS;
        } else {
            e.start = at(d0, time(&self.start_time)?);
            e.end = at(d1, time(&self.end_time)?);
            if e.end <= e.start {
                return Err(t!("La fin doit être après le début", "The end must be after the start").into());
            }
        }
        e.attendees = crate::tabs::mail::contacts::split_list(&self.attendees).iter().filter_map(|m| crate::tabs::mail::contacts::parse_mailbox(m).map(|(_, a)| a)).collect();
        if e.title.trim().is_empty() {
            e.title = t!("(Sans titre)", "(Untitled)").into();
        }
        Ok(e)
    }
}

// --- Affichage ----------------------------------------------------------------------

pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, acc: &mut Account, prefs: &mut Prefs, book: &Contacts) {
    if !cal.view_loaded_from_prefs {
        // 1 à 7 = nombre de jours, 8 = mois, 0 (jamais réglé) = semaine.
        cal.view = match prefs.calendar_days {
            n @ 1..=7 => View::Days(n),
            8 => View::Month,
            _ => View::Days(7),
        };
        let today = Local::now().date_naive();
        cal.go_to(today);
        cal.view_loaded_from_prefs = true;
    }

    calendar_header(ui, cal);

    if acc.session().is_none() {
        connect_card(ui, acc, t!("Connecte ton compte Google pour afficher tes agendas.", "Connect your Google account to show your calendars."));
        return;
    }
    if cal.need_consent {
        connect_card_consent(ui, acc);
        return;
    }
    if cal.api_disabled {
        api_disabled_card(ui, acc, cal);
        return;
    }
    // Agendas + événements de la plage affichée (avec nouvel essai espacé en cas d'erreur).
    cal.background(acc, prefs);
    ui.ctx().request_repaint_after(Duration::from_secs(30)); // ligne « maintenant »

    shortcuts(ui, cal, prefs);
    if !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(Key::S) && i.modifiers.is_none()) {
        prefs.calendar_side_hidden = !prefs.calendar_side_hidden;
    }

    // Agendas découverts pour la première fois : masqués s'ils le sont dans Google Agenda.
    for c in &cal.calendars {
        if !prefs.known_calendars.contains(&c.id) {
            prefs.known_calendars.push(c.id.clone());
            if !c.selected && !prefs.hidden_calendars.contains(&c.id) {
                prefs.hidden_calendars.push(c.id.clone());
            }
        }
    }

    // Colonne de gauche masquable : l'agenda prend alors toute la largeur.
    // Ouverture / fermeture rapide avec un léger rebond élastique.
    const SIDE_W: f32 = 250.0;
    let k = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("wt_cal_side"), !prefs.calendar_side_hidden, 0.28, |t| crate::prefs::Curve::Elastic.apply(t));
    let side_w = if SIDE_W * k < 24.0 { 0.0 } else { SIDE_W * k };
    let gaps = if side_w > 0.0 { 8.0 } else { 0.0 };
    let panel_w = if cal.editor.is_some() { 360.0 } else { 0.0 };
    let center_w = (ui.available_width() - side_w - panel_w - gaps - if panel_w > 0.0 { 8.0 } else { 0.0 }).max(200.0);
    let mut fixed: Vec<f32> = Vec::new();
    if side_w > 0.0 {
        fixed.push(side_w);
    }
    if panel_w > 0.0 {
        fixed.push(center_w);
    }
    let offset = if side_w > 0.0 { 0 } else { 1 };
    let acc_ref: &Account = acc;
    let mut toggle_side = false;
    let side_hidden = prefs.calendar_side_hidden;
    widgets::pane_columns(ui, &fixed, |i, ui| match i + offset {
        0 => {
            // Contenu dessiné à sa largeur normale et rogné par la colonne qui s'anime.
            let inner = ui.available_rect_before_wrap();
            let margins = side_w - inner.width();
            let content = egui::Rect::from_min_size(inner.min, egui::vec2((SIDE_W - margins).max(inner.width()), inner.height()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(*ui.layout()));
            child.set_clip_rect(inner.expand(2.0).intersect(ui.clip_rect()));
            child.multiply_opacity(k.clamp(0.0, 1.0));
            side::show(&mut child, cal, prefs);
            ui.allocate_rect(inner, egui::Sense::hover());
        }
        1 => center(ui, cal, prefs, acc_ref, side_hidden, &mut toggle_side),
        _ => panel::show(ui, cal, acc_ref, book),
    });
    if toggle_side {
        prefs.calendar_side_hidden = !prefs.calendar_side_hidden;
    }

    prefs.calendar_days = match cal.view {
        View::Days(n) => n,
        View::Month => 8,
    };
    toast(ui, cal);
}

/// En-tête : mois et année en gros et en gras, numéro de semaine à côté.
fn calendar_header(ui: &mut egui::Ui, cal: &CalendarState) {
    let days = cal.days();
    let (title, week) = match cal.view {
        View::Month => (month_title(cal.anchor), None),
        View::Days(_) => {
            let (a, b) = (days[0], *days.last().unwrap());
            let w = Some(tf!("Semaine {}", "Week {}", a.iso_week().week()));
            if a.month() == b.month() {
                (month_title(a), w)
            } else if a.year() == b.year() {
                (format!("{} – {}", capitalize(super::month_name(a.month())), month_title(b)), w)
            } else {
                (format!("{} – {}", month_title(a), month_title(b)), w)
            }
        }
    };
    widgets::page_header_with(ui, t!("Agenda", "Calendar"), &title, |ui| {
        if let Some(w) = week {
            widgets::header_badge(ui, &w, false);
        }
    });
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn center(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &Prefs, acc: &Account, side_hidden: bool, toggle_side: &mut bool) {
    // Barre d'outils : panneau latéral, aujourd'hui, précédent / suivant, vues.
    ui.horizontal(|ui| {
        let (icon, tip) = if side_hidden {
            ("»", t!("Afficher le panneau de gauche (S)", "Show the left panel (S)"))
        } else {
            ("«", t!("Masquer le panneau de gauche (S)", "Hide the left panel (S)"))
        };
        if ui.button(icon).on_hover_text(tip).clicked() {
            *toggle_side = true;
        }
        if ui.button(t!("Aujourd'hui", "Today")).on_hover_text("T").clicked() {
            cal.today();
        }
        if ui.button("‹").on_hover_text(t!("Précédent (K ou ←)", "Previous (K or ←)")).clicked() {
            cal.shift(-1);
        }
        if ui.button("›").on_hover_text(t!("Suivant (J ou →)", "Next (J or →)")).clicked() {
            cal.shift(1);
        }
        if cal.loading_calendars || !cal.pending.is_empty() {
            ui.add(egui::Spinner::new().size(14.0));
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut v = match cal.view {
                View::Days(1) => 1,
                View::Days(7) => 7,
                View::Days(n) => n as i32 + 100,
                View::Month => 0,
            };
            let custom = match cal.view {
                View::Days(n) if n != 1 && n != 7 => tf!("{n} jours", "{n} days"),
                _ => t!("3 jours", "3 days").into(),
            };
            let custom_v = match cal.view {
                View::Days(n) if n != 1 && n != 7 => n as i32 + 100,
                _ => 103,
            };
            let opts = [(0, t!("Mois", "Month")), (7, t!("Semaine", "Week")), (custom_v, custom.as_str()), (1, t!("Jour", "Day"))];
            if widgets::segmented(ui, "cal_view", &opts, &mut v) {
                cal.set_view(match v {
                    0 => View::Month,
                    1 => View::Days(1),
                    7 => View::Days(7),
                    n => View::Days((n - 100) as u8),
                });
            }
        });
    });
    ui.add_space(10.0);

    match cal.view {
        View::Month => month::show(ui, cal, prefs),
        View::Days(_) => grid::show(ui, cal, prefs, acc),
    }
}

/// Raccourcis façon Notion Calendar (inactifs pendant la saisie de texte).
fn shortcuts(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &Prefs) {
    if ui.ctx().egui_wants_keyboard_input() {
        return;
    }
    let pressed = |k: Key| ui.input(|i| i.key_pressed(k) && i.modifiers.is_none());
    if pressed(Key::T) {
        cal.today();
    }
    if pressed(Key::D) {
        cal.set_view(View::Days(1));
    }
    if pressed(Key::W) {
        cal.set_view(View::Days(7));
    }
    if pressed(Key::M) {
        cal.set_view(View::Month);
    }
    for (k, n) in [(Key::Num1, 1), (Key::Num2, 2), (Key::Num3, 3), (Key::Num4, 4), (Key::Num5, 5), (Key::Num6, 6), (Key::Num7, 7)] {
        if pressed(k) {
            cal.set_view(View::Days(n));
        }
    }
    if pressed(Key::J) || pressed(Key::ArrowRight) {
        cal.shift(1);
    }
    if pressed(Key::K) || pressed(Key::ArrowLeft) {
        cal.shift(-1);
    }
    if pressed(Key::C) {
        // Nouvel événement à la prochaine demi-heure.
        let now = Local::now();
        let d = now.date_naive();
        let m = snap((now.hour() * 60 + now.minute()) as i32 + 15, 30);
        cal.new_event(prefs, at(d, m), at(d, m + 60), false);
    }
    if pressed(Key::Escape) {
        cal.editor = None;
        cal.selected = None;
        cal.drag = None;
    }
}

fn connect_card(ui: &mut egui::Ui, acc: &mut Account, text: &str) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, t!("Google Agenda", "Google Calendar"), 0.0, |ui| {
                ui.label(RichText::new(text).color(theme::TEXT_MUTED()));
                ui.add_space(12.0);
                super::settings::google_account_form(ui, acc);
            });
        });
    });
}

fn connect_card_consent(ui: &mut egui::Ui, acc: &mut Account) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, t!("Autoriser Google Agenda", "Allow Google Calendar"), 0.0, |ui| {
                ui.label(
                    RichText::new(t!(
                        "Ton compte est connecté pour Gmail, mais WorkTable n'a pas encore l'autorisation \
                         d'accéder à Google Agenda. Reconnecte-toi pour l'accorder (une seule fois).",
                        "Your account is connected for Gmail, but WorkTable is not yet allowed \
                         to access Google Calendar. Sign in again to grant it (only once)."
                    ))
                    .color(theme::TEXT_MUTED()),
                );
                ui.add_space(12.0);
                if acc.status == Status::Connecting {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(t!("Termine l'autorisation dans le navigateur…", "Finish the authorization in the browser…"));
                    });
                } else if ui.button(t!("Autoriser l'accès à Google Agenda", "Allow access to Google Calendar")).clicked() {
                    acc.connect();
                }
            });
        });
    });
}

fn api_disabled_card(ui: &mut egui::Ui, acc: &mut Account, cal: &mut CalendarState) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, t!("Activer l'API Google Agenda", "Enable the Google Calendar API"), 0.0, |ui| {
                ui.label(
                    RichText::new(t!(
                        "L'API Google Calendar n'est pas activée dans ton projet Google Cloud. \
                         Active-la (bouton ci-dessous), attends une minute, puis réessaie.",
                        "The Google Calendar API is not enabled in your Google Cloud project. \
                         Enable it (button below), wait a minute, then try again."
                    ))
                    .color(theme::TEXT_MUTED()),
                );
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("Ouvrir la page d'activation", "Open the activation page")).clicked() {
                        let _ = webbrowser::open("https://console.cloud.google.com/apis/library/calendar-json.googleapis.com");
                    }
                    if ui.button(t!("Réessayer", "Try again")).clicked() {
                        cal.api_disabled = false;
                        cal.load_calendars(acc);
                    }
                });
            });
        });
    });
}

fn toast(ui: &mut egui::Ui, cal: &mut CalendarState) {
    let Some((text, error, at)) = &cal.notice else { return };
    let ttl = if *error { 8.0 } else { 2.5 };
    if at.elapsed().as_secs_f32() > ttl {
        cal.notice = None;
        return;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    let (text, error) = (text.clone(), *error);
    egui::Area::new(egui::Id::unique("cal_toast")).order(egui::Order::Foreground).anchor(egui::Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0)).show(ui.ctx(), |ui| {
        ui.set_max_width(420.0);
        egui::Frame::new()
            .fill(if error { theme::TEXT() } else { theme::SURFACE_HI() })
            .stroke(egui::Stroke::new(1.0, theme::BORDER()))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(text).color(if error { theme::BG() } else { theme::TEXT() })).wrap());
            });
    });
}

/// Rendu lisible d'une couleur d'agenda sur le fond sombre.
pub fn color32(c: [u8; 3]) -> egui::Color32 {
    egui::Color32::from_rgb(c[0], c[1], c[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_and_ranges() {
        let mut c = CalendarState::default();
        c.view = View::Days(7);
        c.go_to(NaiveDate::from_ymd_opt(2026, 10, 22).unwrap());
        assert_eq!(c.anchor, NaiveDate::from_ymd_opt(2026, 10, 19).unwrap(), "semaine du lundi");
        c.shift(1);
        assert_eq!(c.anchor, NaiveDate::from_ymd_opt(2026, 10, 26).unwrap());
        c.set_view(View::Month);
        assert_eq!(c.anchor.day(), 1);
        c.shift(-1);
        assert_eq!(c.anchor, NaiveDate::from_ymd_opt(2026, 9, 1).unwrap());
        assert_eq!(c.days().len(), 42);
        c.view = View::Days(3);
        c.go_to(NaiveDate::from_ymd_opt(2026, 12, 31).unwrap());
        assert_eq!(c.days()[2], NaiveDate::from_ymd_opt(2027, 1, 2).unwrap());
    }

    #[test]
    fn editor_builds_event() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap();
        let ev = CalEvent { calendar: "primary".into(), start: at(d, 600), end: at(d, 660), ..Default::default() };
        let mut ed = Editor::new(ev, None);
        assert_eq!((ed.date.as_str(), ed.start_time.as_str(), ed.end_time.as_str()), ("21/10/2026", "10:00", "11:00"));
        ed.start_time = "9h30".into();
        ed.end_time = "11:15".into();
        ed.attendees = "Léa <lea@studio.fr>, hugo@gmail.com, ".into();
        let e = ed.build().unwrap();
        assert_eq!(minutes_of(e.start), 570);
        assert_eq!(e.end - e.start, 105 * 60_000);
        assert_eq!(e.attendees, vec!["lea@studio.fr", "hugo@gmail.com"]);
        assert_eq!(e.title, "(Sans titre)");
        ed.end_time = "08:00".into();
        assert!(ed.build().is_err());
        ed.ev.all_day = true;
        let e = ed.build().unwrap();
        assert_eq!(e.end - e.start, DAY_MS);
    }
}
