//! Rattacher une note : événement du calendrier, date, mail ou fichier Google Drive.

use std::time::{Duration, Instant};

use chrono::{Datelike, Duration as CDuration, Local, NaiveDate};
use eframe::egui::{self, Color32, CornerRadius, Pos2, RichText, Sense, Stroke, vec2};

use crate::google::drive::{self, DriveEvent, DriveFile, Source};
use crate::google::gmail::{self, Summary};
use crate::google::{Account, Event};
use crate::notes_store::NoteLink;
use crate::tabs::calendar::{CalendarState, date_of, hhmm};
use crate::tabs::mail::MailState;
use crate::tabs::messages::ChatState;
use crate::tabs::{long_date, month_name, month_short, weekday_short};
use crate::tasks::Tasks;
use crate::{theme, widgets};

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Event,
    Date,
    Mail,
    Chat,
    Task,
    Drive,
}

pub struct Picker {
    pub tab: Tab,
    query: String,
    day: NaiveDate,
    month: NaiveDate,
    edited: Option<Instant>,
    pub sent: String,
    pub drive: Vec<DriveFile>,
    pub mails: Vec<Summary>,
    searching: bool,
    focus: bool,
    /// Entrée dans le champ de recherche : choisit le premier résultat.
    pick_first: bool,
}

/// Préfixe des recherches lancées par les notes (pour router les résultats).
pub const PREFIX: &str = "note:";

impl Picker {
    pub fn new(tab: Tab) -> Self {
        let today = Local::now().date_naive();
        Picker {
            tab,
            query: String::new(),
            day: today,
            month: today.with_day(1).unwrap_or(today),
            edited: None,
            sent: String::new(),
            drive: Vec::new(),
            mails: Vec::new(),
            searching: false,
            focus: true,
            pick_first: false,
        }
    }

    pub fn on_drive(&mut self, query: &str, files: Vec<DriveFile>) {
        if query == self.sent {
            self.drive = files;
            self.searching = false;
        }
    }

    pub fn on_mail(&mut self, query: &str, list: Vec<Summary>) {
        if query == self.sent {
            self.mails = list;
            self.searching = false;
        }
    }
}

pub fn date_label(iso: &str) -> String {
    NaiveDate::parse_from_str(iso, "%Y-%m-%d").map(long_date).unwrap_or_else(|_| iso.to_string())
}

fn row(ui: &mut egui::Ui, icon: &str, color: Option<Color32>, title: &str, sub: &str) -> bool {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 42.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() {
        ui.painter().rect_filled(r, CornerRadius::same(8), theme::SURFACE_HI());
    }
    let p = ui.painter();
    if let Some(c) = color {
        p.rect_filled(egui::Rect::from_min_size(Pos2::new(r.left() + 10.0, r.top() + 10.0), vec2(4.0, 22.0)), CornerRadius::same(2), c);
    } else {
        p.text(Pos2::new(r.left() + 14.0, r.center().y), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(15.0), theme::TEXT());
    }
    let tw = w - 40.0;
    let t = crate::tabs::mail::view::one_line(p, title, egui::FontId::proportional(13.5), theme::TEXT(), tw);
    p.galley(Pos2::new(r.left() + 30.0, r.top() + 5.0), t, theme::TEXT());
    let s = crate::tabs::mail::view::one_line(p, sub, egui::FontId::proportional(11.5), theme::TEXT_MUTED(), tw);
    p.galley(Pos2::new(r.left() + 30.0, r.top() + 23.0), s, theme::TEXT_MUTED());
    resp.clicked()
}

/// Fenêtre de choix. Renvoie le lien choisi ; `close` passe à vrai si la fenêtre est fermée.
#[allow(clippy::too_many_arguments)]
pub fn show(ctx: &egui::Context, p: &mut Picker, cal: &CalendarState, mail: &MailState, chat: &ChatState, tasks: &Tasks, acc: &Account, close: &mut bool) -> Option<NoteLink> {
    let mut out = None;
    // Recherche Gmail / Drive après une courte pause de frappe.
    let q = p.query.trim().to_string();
    if matches!(p.tab, Tab::Mail | Tab::Drive) && q.chars().count() >= 2 && p.edited.is_some_and(|t| t.elapsed() > Duration::from_millis(350)) {
        let key = format!("{PREFIX}{}:{q}", if p.tab == Tab::Mail { "m" } else { "d" });
        if key != p.sent {
            p.sent = key.clone();
            p.searching = true;
            p.edited = None;
            if p.tab == Tab::Mail {
                acc.run(move |s| {
                    let list = gmail::list(s, &format!("search:{q}"), 15).unwrap_or_default();
                    Ok(Event::MailSearch { query: key, list })
                });
            } else {
                acc.run(move |s| {
                    let files = drive::list(s, &Source::Search(q), None).map(|r| r.0).unwrap_or_default();
                    Ok(Event::Drive(DriveEvent::Found { query: key, files }))
                });
            }
        }
    } else if p.edited.is_some() {
        ctx.request_repaint_after(Duration::from_millis(120));
    }

    let resp = egui::Modal::new(egui::Id::unique("wt_note_links")).backdrop_color(Color32::from_black_alpha(140)).show(ctx, |ui| {
        ui.set_width(580.0);
        ui.set_min_height(470.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(t!("Rattacher à la note", "Link to the note")).size(17.0).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add(egui::Button::new("✕").frame(false)).clicked() {
                    *close = true;
                }
            });
        });
        ui.add_space(6.0);
        let before = p.tab;
        widgets::segmented(
            ui,
            "note_links",
            &[(Tab::Event, t!("Événement", "Event")), (Tab::Date, "Date"), (Tab::Mail, "Mail"), (Tab::Chat, "Message"), (Tab::Task, t!("Tâche", "Task")), (Tab::Drive, "Drive")],
            &mut p.tab,
        );
        if p.tab != before {
            p.query.clear();
            p.focus = true;
            ui.ctx().request_repaint();
        }
        ui.add_space(10.0);

        if p.tab != Tab::Date {
            let hint = match p.tab {
                Tab::Event => t!("Rechercher un événement chargé…", "Search a loaded event…"),
                Tab::Mail => t!("Rechercher un mail (objet, expéditeur…)", "Search a mail (subject, sender…)"),
                Tab::Chat => t!("Rechercher une conversation ou un message…", "Search a conversation or message…"),
                Tab::Task => t!("Rechercher une tâche…", "Search a task…"),
                _ => t!("Rechercher un fichier Drive…", "Search a Drive file…"),
            };
            let r = ui.add(egui::TextEdit::singleline(&mut p.query).hint_text(hint).desired_width(f32::INFINITY).margin(vec2(10.0, 7.0)));
            if p.focus {
                r.request_focus();
                p.focus = false;
            }
            if r.changed() {
                p.edited = Some(Instant::now());
            }
            p.pick_first = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.add_space(8.0);
        }

        egui::ScrollArea::vertical().max_height(380.0).min_scrolled_height(380.0).auto_shrink([false, false]).show(ui, |ui| match p.tab {
            Tab::Event => out = events(ui, p, cal),
            Tab::Date => out = dates(ui, p),
            Tab::Mail => out = mails(ui, p, mail),
            Tab::Chat => out = chats(ui, p, chat),
            Tab::Task => out = task_list(ui, p, tasks),
            Tab::Drive => out = files(ui, p),
        });
    });
    if resp.should_close() {
        *close = true;
    }
    out
}

fn events(ui: &mut egui::Ui, p: &mut Picker, cal: &CalendarState) -> Option<NoteLink> {
    let mut pick = std::mem::take(&mut p.pick_first);
    let q = p.query.trim().to_lowercase();
    let mut list: Vec<_> = cal.events.values().flatten().collect();
    if q.is_empty() {
        ui.horizontal(|ui| {
            if ui.button("◀").clicked() {
                p.day -= CDuration::days(1);
            }
            if ui.button(t!("Aujourd'hui", "Today")).clicked() {
                p.day = Local::now().date_naive();
            }
            if ui.button("▶").clicked() {
                p.day += CDuration::days(1);
            }
            ui.label(RichText::new(date_label(&p.day.format("%Y-%m-%d").to_string())).strong());
        });
        ui.add_space(6.0);
        list.retain(|e| date_of(e.start) <= p.day && (date_of(e.end - 1) >= p.day || date_of(e.start) == p.day));
        list.sort_by_key(|e| (!e.all_day, e.start));
    } else {
        let now = Local::now().timestamp_millis();
        list.retain(|e| e.title.to_lowercase().contains(&q));
        list.sort_by_key(|e| (e.start - now).abs());
        list.truncate(40);
    }
    list.dedup_by(|a, b| a.id == b.id && a.calendar == b.calendar);
    if list.is_empty() {
        widgets::empty_state(
            ui,
            t!("Aucun événement", "No events"),
            t!("Seuls les événements déjà chargés par le calendrier apparaissent.", "Only events already loaded by the calendar appear."),
        );
    }
    for e in list {
        let [r, g, b] = cal.color_of(e);
        let when = if e.all_day {
            tf!("{} · journée", "{} · all day", short(date_of(e.start)))
        } else {
            format!("{} · {} – {}", short(date_of(e.start)), hhmm(e.start), hhmm(e.end))
        };
        let title = if e.title.is_empty() { t!("(sans titre)", "(untitled)") } else { &e.title };
        if row(ui, "", Some(Color32::from_rgb(r, g, b)), title, &when) || std::mem::take(&mut pick) {
            return Some(NoteLink::Event { calendar: e.calendar.clone(), id: e.id.clone(), title: title.to_string(), start: e.start });
        }
    }
    None
}

fn short(d: NaiveDate) -> String {
    format!("{} {} {}", weekday_short(d.weekday()), d.day(), month_short(d.month()))
}

fn dates(ui: &mut egui::Ui, p: &mut Picker) -> Option<NoteLink> {
    let today = Local::now().date_naive();
    let mut out = None;
    ui.horizontal(|ui| {
        for (label, d) in
            [(t!("Aujourd'hui", "Today"), today), (t!("Demain", "Tomorrow"), today + CDuration::days(1)), (t!("Dans une semaine", "In a week"), today + CDuration::days(7))]
        {
            if ui.button(label).clicked() {
                out = Some(d);
            }
        }
    });
    ui.add_space(10.0);
    ui.horizontal(|ui| {
        if ui.button("◀").clicked() {
            p.month = (p.month - CDuration::days(1)).with_day(1).unwrap_or(p.month);
        }
        ui.label(RichText::new(format!("{} {}", month_name(p.month.month()), p.month.year())).strong().size(15.0));
        if ui.button("▶").clicked() {
            p.month = (p.month + CDuration::days(32)).with_day(1).unwrap_or(p.month);
        }
    });
    ui.add_space(6.0);
    let cell = vec2(70.0, 34.0);
    let (rect, _) = ui.allocate_exact_size(vec2(cell.x * 7.0, cell.y * 7.0), Sense::hover());
    let painter = ui.painter().clone();
    for (k, d) in std::iter::successors(Some(chrono::Weekday::Mon), |d| Some(d.succ())).take(7).enumerate() {
        let d = weekday_short(d).trim_end_matches('.');
        painter.text(rect.min + vec2(cell.x * (k as f32 + 0.5), cell.y * 0.5), egui::Align2::CENTER_CENTER, d, egui::FontId::proportional(11.5), theme::TEXT_MUTED());
    }
    let start = crate::tabs::calendar::monday(p.month);
    for k in 0..42 {
        let d = start + CDuration::days(k);
        let r = egui::Rect::from_min_size(rect.min + vec2(cell.x * (k % 7) as f32, cell.y * (1 + k / 7) as f32), cell).shrink(2.0);
        let resp = ui.interact(r, egui::Id::unique(("wt_note_day", d)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if resp.hovered() {
            painter.rect_filled(r, CornerRadius::same(8), theme::SURFACE_HI());
        }
        let col = if d == today {
            theme::TODAY()
        } else if d.month() == p.month.month() {
            theme::TEXT()
        } else {
            theme::TEXT_FAINT()
        };
        if d == today {
            painter.circle_stroke(r.center(), 13.0, Stroke::new(1.5, theme::TODAY()));
        }
        painter.text(r.center(), egui::Align2::CENTER_CENTER, d.day().to_string(), egui::FontId::proportional(13.0), col);
        if resp.clicked() {
            out = Some(d);
        }
    }
    out.map(|d| NoteLink::Date { date: d.format("%Y-%m-%d").to_string() })
}

fn mails(ui: &mut egui::Ui, p: &mut Picker, mail: &MailState) -> Option<NoteLink> {
    let mut pick = std::mem::take(&mut p.pick_first);
    let q = p.query.trim().to_lowercase();
    let mut seen = std::collections::HashSet::new();
    let list: Vec<&Summary> = mail
        .pinned
        .list
        .iter()
        .chain(p.mails.iter())
        .chain(mail.starred.iter())
        .chain(mail.messages.iter())
        .filter(|m| seen.insert(m.id.clone()))
        .filter(|m| q.is_empty() || m.subject.to_lowercase().contains(&q) || m.from.to_lowercase().contains(&q) || p.mails.iter().any(|r| r.id == m.id))
        .take(40)
        .collect();
    if p.searching {
        ui.label(RichText::new(t!("Recherche dans Gmail…", "Searching Gmail…")).small().color(theme::TEXT_MUTED()));
    }
    if list.is_empty() && !p.searching {
        widgets::empty_state(ui, t!("Aucun mail", "No mail"), t!("Essaie un autre mot-clé.", "Try another keyword."));
    }
    for m in list {
        let from = m.sender_name().to_string();
        let sub = format!("{from} · {}", crate::tabs::mail::view::short_date(m.date));
        let subject = if m.subject.is_empty() { t!("(sans objet)", "(no subject)") } else { &m.subject };
        if row(ui, "✉", None, subject, &sub) || std::mem::take(&mut pick) {
            return Some(NoteLink::Mail { id: m.id.clone(), subject: subject.to_string(), from });
        }
    }
    None
}

fn files(ui: &mut egui::Ui, p: &mut Picker) -> Option<NoteLink> {
    let mut pick = std::mem::take(&mut p.pick_first);
    if p.query.trim().chars().count() < 2 {
        widgets::empty_state(ui, "Google Drive", t!("Tape le nom d'un fichier (Docs, Sheets, PDF…).", "Type a file name (Docs, Sheets, PDF…)."));
        return None;
    }
    if p.searching {
        ui.label(RichText::new(t!("Recherche dans Drive…", "Searching Drive…")).small().color(theme::TEXT_MUTED()));
    } else if p.drive.is_empty() {
        widgets::empty_state(ui, t!("Aucun fichier", "No files"), t!("Essaie un autre nom.", "Try another name."));
    }
    for f in &p.drive {
        let sub = format!("{} · {}", drive::kind_label(f), if f.owner.is_empty() { "Drive" } else { &f.owner });
        if row(ui, file_icon(&f.mime), None, &f.name, &sub) || std::mem::take(&mut pick) {
            return Some(NoteLink::Drive { id: f.id.clone(), name: f.name.clone(), mime: f.mime.clone(), url: f.web_link.clone() });
        }
    }
    None
}

pub fn file_icon(mime: &str) -> &'static str {
    match mime {
        drive::FOLDER => "📁",
        "application/vnd.google-apps.spreadsheet" => "📊",
        "application/vnd.google-apps.presentation" => "📽",
        "application/pdf" => "📕",
        m if m.starts_with("image/") => "🖼",
        _ => "📄",
    }
}

fn chats(ui: &mut egui::Ui, p: &mut Picker, chat: &ChatState) -> Option<NoteLink> {
    let mut pick = std::mem::take(&mut p.pick_first);
    let q = p.query.trim().to_lowercase();
    let mut spaces: Vec<_> = chat.spaces.iter().filter(|sp| q.is_empty() || chat.title(sp).to_lowercase().contains(&q)).collect();
    spaces.sort_by_key(|sp| -sp.last_active);
    if !spaces.is_empty() {
        ui.label(RichText::new("CONVERSATIONS").small().strong().color(theme::TEXT_MUTED()));
    }
    for sp in spaces.into_iter().take(if q.is_empty() { 12 } else { 8 }) {
        let title = chat.title(sp);
        let sub =
            format!("{} · {}", if sp.members > 2 { t!("Groupe", "Group") } else { t!("Message privé", "Direct message") }, crate::tabs::mail::view::short_date(sp.last_active));
        if row(ui, "💬", None, &title, &sub) || std::mem::take(&mut pick) {
            return Some(NoteLink::Chat { space: sp.name.clone(), message: None, title, text: String::new() });
        }
    }
    // Messages déjà chargés.
    let mut msgs: Vec<_> = chat.msgs.values().flatten().filter(|m| !m.text.trim().is_empty() && (q.is_empty() || m.text.to_lowercase().contains(&q))).collect();
    msgs.sort_by_key(|m| -m.created);
    if !msgs.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new("MESSAGES").small().strong().color(theme::TEXT_MUTED()));
    }
    for m in msgs.into_iter().take(25) {
        let title = chat.space(&m.space).map(|sp| chat.title(sp)).unwrap_or_default();
        let first: String = m.text.lines().next().unwrap_or_default().chars().take(90).collect();
        let sub = format!("{} · {} · {}", chat.name_of(&m.sender), title, crate::tabs::mail::view::short_date(m.created));
        if row(ui, "🗨", None, &first, &sub) || std::mem::take(&mut pick) {
            return Some(NoteLink::Chat { space: m.space.clone(), message: Some(m.name.clone()), title, text: first });
        }
    }
    if chat.spaces.is_empty() {
        widgets::empty_state(
            ui,
            t!("Google Chat non chargé", "Google Chat not loaded"),
            t!("Ouvre l'onglet Messages une fois pour charger tes conversations.", "Open the Messages tab once to load your conversations."),
        );
    }
    None
}

fn task_list(ui: &mut egui::Ui, p: &mut Picker, tasks: &Tasks) -> Option<NoteLink> {
    let mut pick = std::mem::take(&mut p.pick_first);
    let q = p.query.trim().to_lowercase();
    let mut n = 0;
    for (t, archived) in tasks.list.iter().map(|t| (t, false)).chain(tasks.archive.iter().rev().map(|t| (t, true))) {
        if !q.is_empty() && !t.title.to_lowercase().contains(&q) {
            continue;
        }
        n += 1;
        if n > 40 {
            break;
        }
        let sub = if archived { t!("Archivée", "Archived").to_string() } else { t.status.label().to_string() };
        if row(ui, "✅", None, &t.title, &sub) || std::mem::take(&mut pick) {
            return Some(NoteLink::Task { id: t.id, title: t.title.clone() });
        }
    }
    if n == 0 {
        widgets::empty_state(ui, t!("Aucune tâche", "No tasks"), t!("Les tâches se créent dans le Tableau de bord.", "Tasks are created in the Dashboard."));
    }
    None
}
