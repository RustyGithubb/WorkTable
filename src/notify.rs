//! Notifications : nouveau mail, nouveau message Google Chat, document partagé sur Drive,
//! rendez-vous dans quelques minutes. Affichées dans l'app (en haut à droite) et, si activé,
//! comme notifications Windows (utile quand l'app est en arrière-plan).
//!
//! Mails et messages Chat : bandeau animé en haut de l'écran, façon « succès débloqué » de la
//! Xbox 360 (pastille qui apparaît, capsule qui se déploie, barre de durée de 4 s).

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::Local;
use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke};

use crate::google::calendar::CalEvent;
use crate::google::chat::{self, ChatEvent, ChatMessage};
use crate::google::drive::{self, DriveEvent, DriveFile, Source};
use crate::google::gmail::Summary;
use crate::google::{Account, Event};
use crate::prefs::Prefs;
use crate::tabs::calendar::CalendarState;
use crate::tabs::messages::ChatState;
use crate::theme;

/// Durée d'affichage dans l'app.
const SHOW_FOR: Duration = Duration::from_secs(30);
/// Bandeau « succès » : apparition, durée affichée (barre), disparition.
const ACH_IN: f32 = 0.65;
const ACH_HOLD: f32 = 4.0;
const ACH_OUT: f32 = 0.5;
const ACH_TOTAL: f32 = ACH_IN + ACH_HOLD + ACH_OUT;
/// Bandeaux en attente au maximum (les plus anciens sont abandonnés).
const ACH_QUEUE: usize = 5;
/// Relève des fichiers partagés avec moi sur Drive.
const SHARED_EVERY: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub enum Target {
    Mail(String),
    Space(String),
    Drive(DriveFile),
    Event(CalEvent),
}

pub struct Notif {
    pub icon: &'static str,
    pub title: String,
    pub text: String,
    pub target: Target,
    pub at: Instant,
}

/// Bandeau animé (mail, message).
struct Achievement {
    icon: &'static str,
    /// Petite ligne du haut (« Nouveau mail · Alice »).
    head: String,
    /// Ligne principale (objet, texte du message).
    text: String,
    target: Target,
    start: Option<Instant>,
    /// Son joué à l'apparition (`None` : sons coupés).
    sound: Option<crate::sound::Sound>,
}

pub struct Notifier {
    pub items: Vec<Notif>,
    achievements: VecDeque<Achievement>,
    /// Bordure Windows de la fenêtre du bandeau déjà retirée.
    border_done: bool,
    started_ms: i64,
    known_mail: HashSet<String>,
    /// Dernière activité connue de chaque conversation (None : premier passage).
    chat_seen: Option<HashMap<String, i64>>,
    /// Conversations dont on attend les nouveaux messages : depuis quand.
    chat_pending: HashMap<String, i64>,
    shared_seen: Option<HashSet<String>>,
    last_shared: Option<Instant>,
    events_done: HashSet<String>,
    last_events: Option<Instant>,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier {
            items: Vec::new(),
            achievements: VecDeque::new(),
            border_done: false,
            started_ms: Local::now().timestamp_millis(),
            known_mail: HashSet::new(),
            chat_seen: None,
            chat_pending: HashMap::new(),
            shared_seen: None,
            last_shared: None,
            events_done: HashSet::new(),
            last_events: None,
        }
    }
}

fn short(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() > n { s.chars().take(n).collect::<String>() + "…" } else { s }
}

impl Notifier {
    /// Notification Windows seule (sans rien dans l'app).
    fn system(prefs: &Prefs, icon: &'static str, title: &str, text: &str) {
        if !prefs.notif_no_system {
            let (t, b) = (format!("{icon} {title}"), text.to_string());
            std::thread::spawn(move || {
                let _ = notify_rust::Notification::new().appname("WorkTable").summary(&t).body(&b).show();
            });
        }
    }

    /// Mail ou message : bandeau « succès » en haut de l'écran (+ notification Windows).
    fn push_achievement(&mut self, prefs: &Prefs, icon: &'static str, head: String, text: String, target: Target) {
        Self::system(prefs, icon, &head, &text);
        let sound = (!prefs.notif_no_sound).then_some(if matches!(target, Target::Mail(_)) { crate::sound::Sound::Mail } else { crate::sound::Sound::Message });
        self.achievements.push_back(Achievement { icon, head, text, target, start: None, sound });
        while self.achievements.len() > ACH_QUEUE {
            // On garde celui en cours et les plus récents.
            self.achievements.remove(1);
        }
    }

    fn push(&mut self, prefs: &Prefs, icon: &'static str, title: String, text: String, target: Target) {
        if !prefs.notif_no_system {
            // Notification Windows (centre de notifications), sans bloquer l'interface.
            let (t, b) = (format!("{icon} {title}"), text.clone());
            std::thread::spawn(move || {
                let _ = notify_rust::Notification::new().appname("WorkTable").summary(&t).body(&b).show();
            });
        }
        self.items.push(Notif { icon, title, text, target, at: Instant::now() });
        if self.items.len() > 6 {
            self.items.remove(0);
        }
    }

    /// Mails reçus (chargement complet : `initial`, sans notification ; relève : nouveaux mails).
    pub fn on_mails(&mut self, list: &[Summary], prefs: &Prefs, initial: bool) {
        for m in list {
            let fresh = self.known_mail.insert(m.id.clone());
            let inbox = m.labels.iter().any(|l| l == "INBOX");
            if initial || !fresh || !inbox || !m.unread() || m.date < self.started_ms - 120_000 || prefs.notif_no_mail {
                continue;
            }
            let subject = if m.subject.is_empty() { "(sans objet)".to_string() } else { short(&m.subject, 120) };
            self.push_achievement(prefs, "✉️", "Nouveau mail".into(), format!("{} - {subject}", m.sender_name()), Target::Mail(m.id.clone()));
        }
    }

    /// Liste des conversations mise à jour : celles qui ont bougé sont relues pour la notification.
    pub fn on_chat_spaces(&mut self, chat: &ChatState, acc: &Account, prefs: &Prefs) {
        let first = self.chat_seen.is_none();
        let seen = self.chat_seen.get_or_insert_with(HashMap::new);
        for sp in &chat.spaces {
            let prev = seen.insert(sp.name.clone(), sp.last_active);
            let Some(prev) = prev else { continue };
            if first || prefs.notif_no_chat || sp.last_active <= prev || sp.last_active < self.started_ms || chat.is_muted(&sp.name) {
                continue;
            }
            // Conversation ouverte à l'écran : pas de notification.
            if chat.open.as_deref() == Some(sp.name.as_str()) {
                continue;
            }
            self.chat_pending.insert(sp.name.clone(), prev);
            let name = sp.name.clone();
            acc.run(move |s| {
                let list = chat::messages(s, &name, Some(prev), None, 10).map(|r| r.0).unwrap_or_default();
                Ok(Event::Chat(ChatEvent::Messages { space: name, list, replace_from: None, older: false, more: false }))
            });
        }
    }

    /// Nouveaux messages d'une conversation attendue : notification du dernier (pas les miens).
    pub fn on_chat_messages(&mut self, space: &str, list: &[ChatMessage], chat: &ChatState, prefs: &Prefs) {
        let Some(since) = self.chat_pending.remove(space) else { return };
        let me = chat.me.as_deref();
        let new: Vec<&ChatMessage> = list.iter().filter(|m| m.created > since && Some(m.sender.as_str()) != me && !m.text.trim().is_empty()).collect();
        let Some(last) = new.last() else { return };
        let title = chat.space(space).map(|s| chat.title(s)).unwrap_or_else(|| "Google Chat".into());
        let who = chat.name_of(&last.sender);
        let more = if new.len() > 1 { format!(" (+{})", new.len() - 1) } else { String::new() };
        let head = if title == who { "Nouveau message".to_string() } else { format!("Nouveau message · {title}") };
        self.push_achievement(prefs, "💬", head, format!("{who} - {}{more}", short(&last.text, 120)), Target::Space(space.to_string()));
    }

    /// Fichiers « Partagés avec moi » : notification des nouveaux.
    pub fn on_shared(&mut self, files: &[DriveFile], prefs: &Prefs) {
        let first = self.shared_seen.is_none();
        let seen = self.shared_seen.get_or_insert_with(HashSet::new);
        let mut fresh = Vec::new();
        for f in files {
            if seen.insert(f.id.clone()) && !first && !f.is_folder() {
                fresh.push(f.clone());
            }
        }
        if prefs.notif_no_share {
            return;
        }
        for f in fresh.into_iter().take(3) {
            let who = if f.owner.is_empty() { "Quelqu'un".to_string() } else { f.owner.clone() };
            self.push(prefs, "📄", format!("{who} a partagé un document"), format!("{} · {}", f.name, drive::kind_label(&f)), Target::Drive(f));
        }
    }

    /// À chaque image : relève Drive, rendez-vous proches, expiration des notifications.
    pub fn background(&mut self, ctx: &egui::Context, acc: &Account, cal: &CalendarState, prefs: &Prefs) {
        if acc.session().is_some() && !prefs.notif_no_share && self.last_shared.is_none_or(|t| t.elapsed() >= SHARED_EVERY) {
            self.last_shared = Some(Instant::now());
            acc.run(|s| {
                Ok(match drive::list(s, &Source::Shared, None) {
                    Ok((files, next)) => Event::Drive(DriveEvent::Listing { source: Source::Shared, files, next, append: false }),
                    Err(_) => Event::Chat(ChatEvent::Noop),
                })
            });
        }
        if !prefs.notif_no_event && self.last_events.is_none_or(|t| t.elapsed() >= Duration::from_secs(15)) {
            self.last_events = Some(Instant::now());
            let now = Local::now().timestamp_millis();
            let mins = if prefs.notif_event_minutes == 0 { 10 } else { prefs.notif_event_minutes.min(120) } as i64;
            let evs: Vec<CalEvent> = cal.events_between(prefs, now, now + mins * 60_000).into_iter().filter(|e| !e.all_day && e.start > now).cloned().collect();
            for e in evs {
                if self.events_done.insert(format!("{}/{}/{}", e.calendar, e.id, e.start)) {
                    let left = ((e.start - now) / 60_000).max(1);
                    let title = if e.title.is_empty() { "(sans titre)".to_string() } else { e.title.clone() };
                    let place = if e.location.is_empty() { String::new() } else { format!(" · {}", short(&e.location, 50)) };
                    let when = crate::tabs::calendar::hhmm(e.start);
                    self.push(prefs, "📅", format!("Dans {left} min : {title}"), format!("À {when}{place}"), Target::Event(e));
                }
            }
            ctx.request_repaint_after(Duration::from_secs(15));
        }
        self.items.retain(|n| n.at.elapsed() < SHOW_FOR);
        if !self.items.is_empty() || !self.achievements.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Pile de notifications en haut à droite ; renvoie la cible cliquée.
    pub fn show(&mut self, ctx: &egui::Context, window: Rect) -> Option<Target> {
        let clicked = self.achievement(ctx, window);
        if self.items.is_empty() {
            return clicked;
        }
        let mut out = None;
        let mut close = None;
        let w = 340.0;
        egui::Area::new(egui::Id::unique("wt_notifications")).order(egui::Order::Foreground).fixed_pos(Pos2::new(window.right() - w - 24.0, window.top() + 20.0)).show(ctx, |ui| {
            ui.set_width(w);
            for (i, n) in self.items.iter().enumerate().rev().take(4) {
                // Apparition et disparition en fondu.
                let age = n.at.elapsed().as_secs_f32();
                let k = (age / 0.25).min(1.0).min(((SHOW_FOR.as_secs_f32() - age) / 0.6).max(0.0));
                ui.set_opacity(k);
                let frame = egui::Frame::new()
                    .fill(theme::SURFACE_HI())
                    .stroke(Stroke::new(1.0, theme::BORDER()))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(Margin::symmetric(12, 10))
                    .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(90) })
                    .show(ui, |ui| {
                        ui.set_width(w - 24.0);
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(20.0, 20.0), Sense::hover());
                            crate::emoji_img::paint(ui.painter(), r, n.icon, theme::TEXT());
                            ui.add(egui::Label::new(RichText::new(&n.title).strong().color(theme::TEXT())).truncate());
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(egui::Button::new(RichText::new("✕").small()).frame(false)).on_hover_text("Fermer").clicked() {
                                    close = Some(i);
                                }
                            });
                        });
                        ui.add(egui::Label::new(RichText::new(&n.text).color(theme::TEXT_MUTED())).wrap());
                    });
                let click = ui.interact(frame.response.rect, egui::Id::unique(("wt_notif", i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
                if click.clicked() && close.is_none() {
                    out = Some(n.target.clone());
                    close = Some(i);
                }
                ui.add_space(8.0);
                ui.set_opacity(1.0);
            }
        });
        if let Some(i) = close {
            self.items.remove(i);
        }
        out.or(clicked)
    }

    /// Bandeau « succès débloqué » en haut au centre de l'écran, dans une petite fenêtre
    /// transparente toujours au premier plan (par-dessus les autres applications).
    /// Renvoie la cible s'il est cliqué.
    fn achievement(&mut self, ctx: &egui::Context, window: Rect) -> Option<Target> {
        let a = self.achievements.front_mut()?;
        let start = *a.start.get_or_insert_with(|| {
            // Apparition du bandeau : petit son (différent pour un mail et un message).
            if let Some(s) = a.sound {
                crate::sound::play(s);
            }
            Instant::now()
        });
        let t = start.elapsed().as_secs_f32();
        if t >= ACH_TOTAL {
            self.achievements.pop_front();
            if self.achievements.is_empty() {
                // La fenêtre se ferme : la suivante devra être retouchée à nouveau.
                self.border_done = false;
            }
            ctx.request_repaint();
            return None;
        }
        ctx.request_repaint();
        let k = |a: f32, d: f32| ((t - a) / d).clamp(0.0, 1.0);
        let out_t = ACH_IN + ACH_HOLD;
        let anim = Anim {
            // Pastille : rebond à l'arrivée, rétrécit à la toute fin.
            circle: ease_out_back(k(0.0, 0.32)) * (1.0 - ease_in(k(ACH_TOTAL - 0.18, 0.18))),
            // Capsule : se déploie depuis la pastille, se replie avant de partir.
            open: ease_out_cubic(k(0.22, 0.4)) * (1.0 - ease_in(k(out_t, 0.32))),
            text: k(0.45, 0.22) * (1.0 - k(out_t - 0.05, 0.18)),
            bar: 1.0 - k(ACH_IN, ACH_HOLD),
            // Descend du haut de l'écran.
            drop: 1.0 - ease_out_cubic(k(0.0, 0.25)),
            clickable: t < out_t,
        };
        // Largeur finale d'après les textes.
        let (head_w, text_w) = ctx.fonts_mut(|f| {
            let mut w = |s: &str, font: egui::FontId| f.layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x;
            (w(&a.head, ach_font()), w(&a.text, ach_font()))
        });
        let full = (ACH_H / 2.0 + ACH_D / 2.0 + 14.0 + head_w.max(text_w).min(ACH_TEXT_MAX) + 34.0).clamp(320.0, ACH_H + ACH_TEXT_MAX + 48.0);

        let clicked = if ctx.embed_viewports() {
            // Pas de fenêtres multiples (certains systèmes) : en haut au centre de l'app.
            let cx = ctx.content_rect().center().x;
            let pos = Pos2::new(cx - full / 2.0 - ACH_MARGIN, window.top());
            let mut clicked = false;
            egui::Area::new(egui::Id::unique("wt_achievement")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
                clicked = paint_achievement(ui, pos, full, a, &anim);
            });
            clicked
        } else {
            // Écran où se trouve l'app (écrans côte à côte), en haut au centre.
            let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
            let mw = monitor.map(|m| m.x).filter(|w| *w > 0.0).unwrap_or(1920.0);
            let left = outer.map(|r| (r.center().x / mw).floor() * mw).unwrap_or(0.0);
            let size = egui::vec2(full + 2.0 * ACH_MARGIN, ACH_H + 2.0 * ACH_MARGIN);
            let builder = egui::ViewportBuilder::default()
                .with_title(ACH_TITLE)
                .with_decorations(false)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_active(false)
                .with_window_level(egui::WindowLevel::AlwaysOnTop)
                .with_inner_size(size)
                .with_position(Pos2::new(left + ((mw - size.x) / 2.0).round(), 4.0));
            let clicked = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("wt_achievement"), builder, |ui, _| {
                let clicked = paint_achievement(ui, Pos2::ZERO, full, a, &anim);
                crate::emoji_img::colorize_frame(ui.ctx());
                clicked
            });
            if !self.border_done {
                self.border_done = strip_border(ACH_TITLE);
            }
            clicked
        };
        if clicked {
            // Ramène l'app au premier plan.
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            let target = a.target.clone();
            // Clic : on passe directement à la disparition.
            a.start = Some(Instant::now() - Duration::from_secs_f32(out_t));
            return Some(target);
        }
        None
    }
}

/// Titre (invisible) de la fenêtre du bandeau, qui sert à la retrouver sous Windows.
const ACH_TITLE: &str = "WorkTable — notification";

/// Windows 11 dessine un liseré clair et arrondit les coins des fenêtres sans cadre :
/// on retire l'un et l'autre (renvoie `false` tant que la fenêtre n'existe pas encore).
#[cfg(windows)]
fn strip_border(title: &str) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY : appels Win32 sur une fenêtre de ce processus, avec des pointeurs valides
    // le temps de l'appel.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        let none: u32 = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&none as *const u32).cast(), 4);
        let corner: i32 = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&corner as *const i32).cast(), 4);
    }
    true
}

#[cfg(not(windows))]
fn strip_border(_title: &str) -> bool {
    true
}

/// Hauteur de la capsule (elle entoure la pastille), diamètre de la pastille, marge autour (ombre), largeur maximale du texte.
const ACH_H: f32 = 68.0;
const ACH_D: f32 = 60.0;
const ACH_MARGIN: f32 = 16.0;
const ACH_TEXT_MAX: f32 = 400.0;

fn ach_font() -> egui::FontId {
    egui::FontId::new(18.0, egui::FontFamily::Name(theme::ACHIEVEMENT_FONT.into()))
}

/// État de l'animation du bandeau à cet instant (0 → 1).
struct Anim {
    circle: f32,
    open: f32,
    text: f32,
    bar: f32,
    drop: f32,
    clickable: bool,
}

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t.clamp(0.0, 1.0)).round() as u8;
    Color32::from_rgba_unmultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// Arc épais (anneau) entre deux angles, en radians (0 = droite, sens horaire), avec
/// anticrénelage (trait d'egui plutôt que maillage brut).
fn ring_arc(p: &egui::Painter, c: Pos2, r_in: f32, r_out: f32, a0: f32, a1: f32, color: Color32) {
    let (mid, width) = ((r_in + r_out) / 2.0, r_out - r_in);
    if (a1 - a0).abs() >= std::f32::consts::TAU - 1e-3 {
        p.circle_stroke(c, mid, Stroke::new(width, color));
        return;
    }
    let steps = (((a1 - a0).abs() * mid / 1.5).ceil() as usize).max(8);
    let pts: Vec<Pos2> = (0..=steps).map(|i| c + egui::Vec2::angled(a0 + (a1 - a0) * i as f32 / steps as f32) * mid).collect();
    p.add(egui::Shape::line(pts, Stroke::new(width, color)));
}

/// Dessine le bandeau (pastille + capsule façon succès Xbox 360) à partir de `origin`.
fn paint_achievement(ui: &mut egui::Ui, origin: Pos2, full: f32, a: &Achievement, an: &Anim) -> bool {
    use std::f32::consts::PI;
    let area = Rect::from_min_size(origin, egui::vec2(full + 2.0 * ACH_MARGIN, ACH_H + 2.0 * ACH_MARGIN));
    let c = Pos2::new(origin.x + ACH_MARGIN + ACH_H / 2.0, origin.y + ACH_MARGIN + ACH_H / 2.0 - an.drop * 50.0);
    // La capsule épouse la pastille (son bout gauche l'entoure) puis se déploie vers la droite.
    let h = ACH_H * an.circle.min(1.0);
    let r = Rect::from_min_size(Pos2::new(c.x - h / 2.0, c.y - h / 2.0), egui::vec2(h + (full - ACH_H) * an.open, h));
    ui.allocate_rect(area, Sense::hover());
    let hit = ui.interact(r.union(Rect::from_center_size(c, egui::vec2(ACH_D, ACH_D))), egui::Id::unique("wt_achievement_hit"), Sense::click());
    let hit = hit.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter().with_clip_rect(area);

    if h > 1.0 {
        let round = CornerRadius::same((h / 2.0) as u8);
        p.add(egui::Shadow { offset: [0, 4], blur: 14, spread: 0, color: Color32::from_black_alpha(150) }.as_shape(r, round));
        // Capsule unie, aux couleurs des panneaux de l'app (fond et liseré du thème).
        p.rect_filled(r, round, theme::SURFACE());
        p.rect_stroke(r, round, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        // Texte blanc sur deux lignes, coupé par la capsule pendant qu'elle se déploie.
        let job = |s: &str| {
            let mut j = egui::text::LayoutJob::simple_singleline(s.to_string(), ach_font(), theme::TEXT().gamma_multiply(an.text));
            j.wrap = egui::text::TextWrapping { max_width: ACH_TEXT_MAX, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            j
        };
        let head = p.layout_job(job(&a.head));
        let text = p.layout_job(job(&a.text));
        let clip = p.with_clip_rect(r.shrink(2.0).intersect(area));
        let x = c.x + ACH_D / 2.0 + 12.0;
        clip.galley(Pos2::new(x, c.y - 24.0), head, Color32::WHITE);
        clip.galley(Pos2::new(x, c.y - 2.0), text, Color32::WHITE);
        // Durée restante : fine barre en bas de la capsule (couleur des pastilles « non lus »).
        let track = Rect::from_min_max(Pos2::new(x, r.bottom() - 9.0), Pos2::new(c.x - ACH_H / 2.0 + full - 30.0, r.bottom() - 7.0));
        clip.rect_filled(track, CornerRadius::same(1), theme::BORDER().gamma_multiply(an.text));
        let fill = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * an.bar, track.bottom()));
        clip.rect_filled(fill, CornerRadius::same(1), theme::TODAY().gamma_multiply(an.text));
    }

    // Pastille : anneau noir, couronne (portion colorée à gauche), sphère grise et icône.
    let rad = ACH_D / 2.0 * an.circle;
    if rad > 0.5 {
        let k = an.circle;
        p.circle_filled(c, rad, Color32::from_rgb(16, 16, 17));
        let (r_in, r_out) = (rad - 10.5 * k, rad - 3.5 * k);
        ring_arc(&p, c, r_in, r_out, 0.0, 2.0 * PI, lerp_color(Color32::from_rgb(16, 16, 17), theme::TODAY(), 0.2));
        // Portion allumée, côté gauche (comme sur la Xbox 360), couleur des pastilles du thème.
        ring_arc(&p, c, r_in, r_out, 0.62 * PI, 1.38 * PI, theme::TODAY());
        // Disque gris uni (sans dégradé ni reflet) derrière l'icône.
        let sr = r_in - 2.0 * k;
        p.circle_filled(c, sr, Color32::from_rgb(96, 98, 100));
        let s = sr * 1.15;
        crate::emoji_img::paint(&p, Rect::from_center_size(c, egui::vec2(s, s)), a.icon, Color32::WHITE);
    }
    hit.clicked() && an.clickable
}

fn ease_out_cubic(x: f32) -> f32 {
    1.0 - (1.0 - x).powi(3)
}

fn ease_in(x: f32) -> f32 {
    x * x
}

/// Léger dépassement puis retour (effet « pop »).
fn ease_out_back(x: f32) -> f32 {
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (x - 1.0).powi(3) + c1 * (x - 1.0).powi(2)
}
