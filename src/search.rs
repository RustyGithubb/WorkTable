//! Recherche globale (Ctrl+Espace) : barre au centre, par-dessus l'app, sur un fond flouté.
//! Cherche dans les mails (chargés + recherche Gmail), l'annuaire, les événements et les dates
//! du calendrier, Google Chat, les tâches, les fichiers partagés et les commandes de l'app.

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
    bg: Backdrop,
    /// Recherche Gmail côté serveur (complète les mails déjà chargés).
    remote: Vec<Summary>,
    remote_query: String,
    /// Recherche Google Drive (même requête).
    drive: Vec<DriveFile>,
    edited: Option<Instant>,
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
        let screen = ctx.content_rect();
        if let Some(img) = self.shot.lock().ok().and_then(|mut s| s.take()) {
            self.texture = Some(ctx.load_texture(format!("{id}_tex"), img, egui::TextureOptions::LINEAR));
        }
        let k = self.opened_at.map(|t| (t.elapsed().as_secs_f32() / 0.14).min(1.0)).unwrap_or(1.0);
        if k < 1.0 {
            ctx.request_repaint();
        }
        let mut clicked = false;
        egui::Area::new(egui::Id::unique(id)).order(egui::Order::Tooltip).fixed_pos(screen.min).show(ctx, |ui| {
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
        self.selected = 0;
        self.bg.start(ctx);
    }

    fn close(&mut self) {
        self.open = false;
        self.bg.clear();
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
    ) -> Option<Target> {
        if !self.open {
            return None;
        }
        let screen = ctx.content_rect();
        // Fond : capture floutée + voile sombre ; un clic à côté ferme.
        let (k, mut close) = self.bg.show(ctx, "wt_search_backdrop");

        // Recherche Gmail après une courte pause de frappe.
        let q = self.query.trim().to_string();
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

        let hits = if q.is_empty() { suggestions(cal, prefs) } else { search(&q, mail, cal, chat, tasks, notes, people, prefs, &self.remote, &self.drive) };
        if self.selected >= hits.len() {
            self.selected = hits.len().saturating_sub(1);
        }
        let mut go = None;
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
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut self.query)
                            .hint_text(t!("Rechercher un mail, une date, un événement, une personne, une tâche…", "Search a mail, a date, an event, a person, a task…"))
                            .font(FontId::proportional(18.0))
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY),
                    );
                    te.request_focus();
                    if te.changed() {
                        self.selected = 0;
                        self.edited = Some(Instant::now());
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
                egui::ScrollArea::vertical().max_height(screen.height() * 0.55).auto_shrink([false, true]).show(ui, |ui| {
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
                            ui.scroll_to_rect(r, None);
                        }
                        crate::emoji_img::paint(p, Rect::from_center_size(Pos2::new(r.left() + 18.0, r.center().y), vec2(20.0, 20.0)), h.icon, theme::TEXT());
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
                        }
                    }
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(t!("↑ ↓ naviguer · Entrée ouvrir · Échap fermer · Ctrl+Espace", "↑ ↓ navigate · Enter open · Esc close · Ctrl+Space"))
                        .small()
                        .color(theme::TEXT_FAINT()),
                );
            });
        });
        // La boîte reste toujours au-dessus du fond : un clic sur le fond le fait passer au premier
        // plan de son niveau (comportement d'egui), et il recouvrait alors la boîte à la
        // réouverture (seul le flou s'affichait).
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("wt_search_box")));
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
        c("🧩", "Blueprint", t!("Automatisations · graphs de nodes", "Automations · node graphs"), Target::Tab(Tab::Blueprint)),
        c("⚙️", t!("Réglages", "Settings"), t!("Compte Google, animations…", "Google account, animations…"), Target::Tab(Tab::Settings)),
    ]
}

#[allow(clippy::too_many_arguments)]
fn search(
    q: &str,
    mail: &MailState,
    cal: &CalendarState,
    chat: &ChatState,
    tasks: &Tasks,
    notes: &crate::tabs::notes::NotesState,
    people: &crate::people::People,
    prefs: &Prefs,
    remote: &[Summary],
    drive: &[DriveFile],
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

    // Tri : par groupe (ordre ci-dessous), puis pertinence ; 6 résultats par groupe.
    // Priorité : contacts, agenda (événements puis dates), mails, chat, Drive, notes, puis le reste.
    const ORDER: [&str; 11] = ["Fiches contact", "Calendrier", "Date", "Mails", "Google Chat", "Drive", "Notes", "Tâches", "Fichiers", "Actions", "À venir"];
    out.sort_by_key(|h| (ORDER.iter().position(|g| *g == h.group).unwrap_or(99), -h.score));
    let mut counts = std::collections::HashMap::new();
    out.retain(|h| {
        let n = counts.entry(h.group).or_insert(0);
        *n += 1;
        *n <= if h.group == "Actions" { 3 } else { 6 }
    });
    out
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
