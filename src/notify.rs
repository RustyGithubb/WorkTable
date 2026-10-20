//! Notifications : nouveau mail, nouveau message Google Chat, document partagé sur Drive,
//! rendez-vous dans quelques minutes. Affichées dans l'app (en haut à droite) et, si activé,
//! comme notifications Windows (utile quand l'app est en arrière-plan).

use std::collections::{HashMap, HashSet};
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

pub struct Notifier {
    pub items: Vec<Notif>,
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
            let subject = if m.subject.is_empty() { "(sans objet)".to_string() } else { m.subject.clone() };
            let text = if m.snippet.is_empty() { subject.clone() } else { format!("{subject} — {}", short(&m.snippet, 90)) };
            self.push(prefs, "✉️", m.sender_name().to_string(), text, Target::Mail(m.id.clone()));
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
        let head = if title == who { who.clone() } else { format!("{who} · {title}") };
        self.push(prefs, "💬", head, format!("{}{more}", short(&last.text, 120)), Target::Space(space.to_string()));
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
        if !self.items.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Pile de notifications en haut à droite ; renvoie la cible cliquée.
    pub fn show(&mut self, ctx: &egui::Context, window: Rect) -> Option<Target> {
        if self.items.is_empty() {
            return None;
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
        out
    }
}
