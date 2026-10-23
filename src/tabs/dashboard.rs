//! Tableau de bord : planning du jour, mails non lus / épinglés, rappels, Kanban de tâches
//! (avec archives quotidiennes) et derniers fichiers reçus par mail.

use chrono::{Local, TimeZone};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::google::calendar::CalEvent;
use crate::google::gmail::{SharedFile, SharedKind};
use crate::google::{Account, Summary};
use crate::prefs::Prefs;
use crate::tabs::calendar::{self, CalendarState};
use crate::tabs::mail::MailState;
use crate::tabs::mail::view::{draw_pin, one_line, short_date};
use crate::tabs::messages::pins::Pins;
use crate::tasks::{self, Status, Tasks};
use crate::{liquid, theme, widgets};

/// Navigation demandée vers un autre onglet.
pub enum DashAction {
    Mail(String),
    Event(CalEvent),
    /// Conversation Google Chat (espace) d'un message épinglé.
    Chat(String),
}

const GAP: f32 = 12.0;

#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    cal: &CalendarState,
    mail: &mut MailState,
    pins: &mut Pins,
    acc: &Account,
    prefs: &Prefs,
    tasks: &mut Tasks,
    links: &mut crate::links::Links,
) -> Option<DashAction> {
    let mut out = None;
    tasks.roll_over(Local::now().timestamp_millis());
    mail.refresh_shared_files(acc);
    ui.ctx().request_repaint_after(std::time::Duration::from_secs(30));

    let greeting = greeting();
    let today = super::today_long();
    widgets::page_header_with(ui, t!("Tableau de bord", "Dashboard"), greeting, |ui| widgets::header_badge(ui, &today, false));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let w = ui.available_width() - 12.0;
        let wide = w > 1050.0;
        // Ligne 1 : liens importants (en premier) | planning | mails | messages épinglés.
        let h1 = 360.0;
        let total = ui.available_width() - 12.0;
        if !wide {
            // Écran étroit : la galerie seule sur sa ligne.
            row(ui, &[1.0], h1, |_, ui| links_panel(ui, links));
            ui.add_space(GAP);
        }
        // Galerie à peu près carrée (3 × 3), le reste partagé entre planning et mails.
        let lw = (h1 - 20.0) / total;
        let rest = 1.0 - lw;
        let ratios: Vec<f32> = if wide { vec![lw, rest / 3.0, rest / 3.0, rest / 3.0] } else { vec![0.5, 0.5] };
        let shift = if wide { 0 } else { 1 };
        let pinned_badge = if pins.list.is_empty() { String::new() } else { pins.list.len().to_string() };
        row(ui, &ratios, h1, |i, ui| match i + shift {
            0 => links_panel(ui, links),
            1 => panel(ui, t!("Planning du jour", "Today's schedule"), "", |ui| planning(ui, cal, prefs, &mut out)),
            3 => panel(ui, t!("Messages épinglés", "Pinned messages"), &pinned_badge, |ui| pinned_messages(ui, pins, &mut out)),
            _ => {
                let unread = inbox_unread(mail).len();
                let badge = if unread == 0 {
                    String::new()
                } else if crate::i18n::en() {
                    format!("{unread} unread")
                } else {
                    format!("{unread} non lu{}", if unread > 1 { "s" } else { "" })
                };
                panel(ui, "Mails", &badge, |ui| mails(ui, mail, &mut out))
            }
        });
        if !wide {
            // Écran étroit : les messages épinglés sur leur propre ligne, sous les mails.
            ui.add_space(GAP);
            let badge = if pins.list.is_empty() { String::new() } else { pins.list.len().to_string() };
            row(ui, &[1.0], 260.0, |_, ui| panel(ui, t!("Messages épinglés", "Pinned messages"), &badge, |ui| pinned_messages(ui, pins, &mut out)));
        }
        ui.add_space(GAP);
        // Ligne 2 : rappels | tâches (Kanban).
        let h2 = 400.0;
        let doing = tf!("{} en cours", "{} in progress", tasks.column(Status::Doing).len());
        if wide {
            row(ui, &[0.28, 0.72], h2, |i, ui| match i {
                0 => panel(ui, t!("Rappels", "Reminders"), "", |ui| reminders(ui, tasks)),
                _ => tasks_panel(ui, tasks, &doing),
            });
        } else {
            row(ui, &[1.0], 220.0, |_, ui| panel(ui, t!("Rappels", "Reminders"), "", |ui| reminders(ui, tasks)));
            ui.add_space(GAP);
            row(ui, &[1.0], h2, |_, ui| tasks_panel(ui, tasks, &doing));
        }
        ui.add_space(GAP);
        // Ligne 3 : fichiers reçus, sur deux colonnes.
        let rows = crate::tabs::mail::SHARED_FILES.div_ceil(if wide { 2 } else { 1 });
        let h3 = rows as f32 * FILE_ROW + 84.0;
        row(ui, &[1.0], h3, |_, ui| panel(ui, t!("Fichiers reçus", "Received files"), "", |ui| files(ui, mail, acc, wide, &mut out)));
        ui.add_space(24.0);
    });
    tasks.save_if_needed();
    out
}

/// Ligne de panneaux de largeurs relatives et de hauteur fixe.
fn row(ui: &mut egui::Ui, ratios: &[f32], h: f32, mut body: impl FnMut(usize, &mut egui::Ui)) {
    let total = ui.available_width() - 12.0 - GAP * (ratios.len() as f32 - 1.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = GAP;
        for (i, r) in ratios.iter().enumerate() {
            ui.allocate_ui_with_layout(vec2(total * r, h), Layout::top_down(Align::Min), |ui| {
                ui.set_min_size(vec2(total * r, h));
                ui.set_max_size(vec2(total * r, h));
                // Vague « Ne pas déranger » (le panneau vibre quand l'onde l'atteint) et tremblement
                // pendant que le logo de la barre tourne.
                let rect = egui::Rect::from_min_size(ui.max_rect().min, vec2(total * r, h));
                let off = widgets::wave_offset(ui.ctx(), rect) + widgets::shake_offset(ui.ctx(), rect.left() * 0.011 + rect.top() * 0.007);
                widgets::shifted(ui, off, |ui| body(i, ui));
            });
        }
    });
}

fn panel(ui: &mut egui::Ui, title: &str, badge: &str, body: impl FnOnce(&mut egui::Ui)) {
    panel_with(
        ui,
        |ui| {
            ui.label(widgets::section_title(title));
            if !badge.is_empty() {
                ui.label(RichText::new(badge).small().color(theme::TODAY()));
            }
        },
        body,
    );
}

fn panel_with(ui: &mut egui::Ui, title: impl FnOnce(&mut egui::Ui), body: impl FnOnce(&mut egui::Ui)) {
    panel_styled(ui, theme::SURFACE(), theme::BORDER(), title, body);
}

/// Panneau avec un fond et un contour choisis.
fn panel_styled(ui: &mut egui::Ui, fill: Color32, border: Color32, title: impl FnOnce(&mut egui::Ui), body: impl FnOnce(&mut egui::Ui)) {
    let h = ui.available_height();
    egui::Frame::new().fill(fill).stroke(Stroke::new(1.0, border)).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(16)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.set_height(h - 34.0);
        ui.horizontal(title);
        ui.add_space(10.0);
        body(ui);
    });
}

// --- Planning du jour -------------------------------------------------------------------

fn planning(ui: &mut egui::Ui, cal: &CalendarState, prefs: &Prefs, out: &mut Option<DashAction>) {
    let mut evs: Vec<&CalEvent> = cal.today_events(prefs);
    if evs.is_empty() {
        ui.add_space(30.0);
        let hint =
            if cal.calendars.is_empty() { t!("Google Agenda non connecté", "Google Calendar not connected") } else { t!("Rien de prévu aujourd'hui", "Nothing planned today") };
        widgets::empty_state(ui, t!("Journée libre", "Free day"), hint);
        return;
    }
    evs.sort_by_key(|e| (!e.all_day, e.start));
    let now = Local::now().timestamp_millis();
    let next = evs.iter().find(|e| !e.all_day && e.start > now).map(|e| e.id.clone());
    egui::ScrollArea::vertical().id_salt("dash_planning").auto_shrink([false, false]).show(ui, |ui| {
        let mut now_drawn = false;
        for e in evs {
            if !e.all_day && !now_drawn && e.start > now {
                now_drawn = true;
                now_line(ui);
            }
            let col = calendar::color32(cal.color_of(e));
            let past = !e.all_day && e.end <= now;
            let current = !e.all_day && e.start <= now && now < e.end;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if e.location.is_empty() { 40.0 } else { 52.0 }), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Ouvrir dans le calendrier", "Open in the calendar"));
            let p = ui.painter();
            if current {
                p.rect_filled(rect, CornerRadius::same(8), col.gamma_multiply(0.18));
            } else if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(6));
            }
            let fade = |c: Color32| if past { c.gamma_multiply(0.45) } else { c };
            let time = if e.all_day { t!("Journée", "All day").to_string() } else { format!("{}\n{}", calendar::hhmm(e.start), calendar::hhmm(e.end)) };
            p.text(Pos2::new(rect.left() + 8.0, rect.top() + 6.0), Align2::LEFT_TOP, time, FontId::monospace(11.0), fade(theme::TEXT_MUTED()));
            p.rect_filled(Rect::from_min_size(Pos2::new(rect.left() + 56.0, rect.top() + 6.0), vec2(3.0, rect.height() - 12.0)), CornerRadius::same(2), fade(col));
            let left = rect.left() + 68.0;
            let title = if e.title.is_empty() { t!("(Sans titre)", "(Untitled)") } else { &e.title };
            let g = one_line(p, title, theme::editor_font(13.5, current), fade(theme::TEXT()), rect.right() - left - 90.0);
            p.galley(Pos2::new(left, rect.top() + 6.0), g, theme::TEXT());
            let mut sub = String::new();
            if current {
                sub = tf!("En cours · fin à {}", "In progress · ends at {}", calendar::hhmm(e.end));
            } else if next.as_deref() == Some(e.id.as_str()) {
                let mins = (e.start - now) / 60_000;
                sub = if mins < 60 { tf!("Dans {mins} min", "In {mins} min") } else { tf!("Dans {} h {:02}", "In {} h {:02}", mins / 60, mins % 60) };
            }
            if !sub.is_empty() {
                p.text(Pos2::new(rect.right() - 8.0, rect.top() + 8.0), Align2::RIGHT_TOP, sub, FontId::proportional(11.0), if current { theme::TODAY() } else { theme::LINK() });
            }
            if e.meet_link.is_some() {
                p.text(Pos2::new(left, rect.top() + 25.0), Align2::LEFT_TOP, "Google Meet", FontId::proportional(11.0), fade(theme::TEXT_FAINT()));
            }
            if !e.location.is_empty() {
                let g = one_line(p, &e.location, FontId::proportional(11.0), fade(theme::TEXT_FAINT()), rect.right() - left - 8.0);
                p.galley(Pos2::new(left, rect.top() + if e.meet_link.is_some() { 38.0 } else { 25.0 }), g, theme::TEXT_FAINT());
            }
            if resp.clicked() {
                *out = Some(DashAction::Event(e.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button(t!("Ouvrir dans le calendrier", "Open in the calendar")).clicked() {
                    *out = Some(DashAction::Event(e.clone()));
                    ui.close();
                }
                ui.separator();
                crate::people::guests_menu(ui, &e.attendees);
            });
        }
        if !now_drawn {
            now_line(ui);
        }
    });
}

fn now_line(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), Sense::hover());
    let p = ui.painter();
    let y = r.center().y;
    p.circle_filled(Pos2::new(r.left() + 58.0, y), 4.0, theme::TODAY());
    p.hline(r.left() + 58.0..=r.right(), y, Stroke::new(1.5, theme::TODAY()));
    p.text(Pos2::new(r.left() + 8.0, y), Align2::LEFT_CENTER, Local::now().format("%H:%M").to_string(), FontId::monospace(10.5), theme::TODAY());
}

// --- Mails ---------------------------------------------------------------------------------

fn inbox_unread(mail: &MailState) -> Vec<&Summary> {
    if mail.folder != crate::tabs::mail::INBOX {
        return Vec::new();
    }
    let mut seen = std::collections::HashSet::new();
    mail.messages.iter().filter(|m| m.unread() && seen.insert(m.thread_id.clone())).collect()
}

fn mails(ui: &mut egui::Ui, mail: &mut MailState, out: &mut Option<DashAction>) {
    let pinned: Vec<Summary> = mail.pinned.list.clone();
    let unread: Vec<Summary> = inbox_unread(mail).into_iter().filter(|m| !pinned.iter().any(|p| p.id == m.id)).take(5).cloned().collect();
    if pinned.is_empty() && unread.is_empty() {
        ui.add_space(30.0);
        widgets::empty_state(
            ui,
            t!("Boîte de réception à jour", "Inbox up to date"),
            if mail.loading { t!("Chargement…", "Loading…") } else { t!("Aucun mail non lu ni épinglé", "No unread or pinned mail") },
        );
        return;
    }
    egui::ScrollArea::vertical().id_salt("dash_mails").auto_shrink([false, false]).show(ui, |ui| {
        let mut unpin = None;
        if !pinned.is_empty() {
            ui.label(RichText::new(t!("ÉPINGLÉS", "PINNED")).small().color(theme::TEXT_MUTED()));
            for m in &pinned {
                mail_row(ui, m, true, out, &mut unpin);
            }
            ui.add_space(6.0);
        }
        if !unread.is_empty() {
            ui.label(RichText::new(t!("NON LUS", "UNREAD")).small().color(theme::TEXT_MUTED()));
            for m in &unread {
                mail_row(ui, m, false, out, &mut unpin);
            }
        }
        if let Some(id) = unpin {
            mail.pinned.remove(&id);
        }
    });
}

/// Ligne de mail ; clic droit : menu (ouvrir, fiche contact, désépingler → `unpin`).
fn mail_row(ui: &mut egui::Ui, m: &Summary, pinned: bool, out: &mut Option<DashAction>, unpin: &mut Option<String>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let tip = if pinned {
        t!("Ouvrir dans Mail (clic droit : désépingler, fiche contact)", "Open in Mail (right-click: unpin, contact card)")
    } else {
        t!("Ouvrir dans Mail (clic droit : fiche contact)", "Open in Mail (right-click: contact card)")
    };
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
    let p = ui.painter();
    if pinned {
        p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(8));
    }
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(6));
    }
    let left = rect.left() + 22.0;
    if pinned {
        draw_pin(p, Pos2::new(rect.left() + 10.0, rect.top() + 14.0), 5.5, true, theme::TEXT_MUTED());
    } else if m.unread() {
        p.circle_filled(Pos2::new(rect.left() + 10.0, rect.top() + 14.0), 3.5, theme::TODAY());
    }
    let date = short_date(m.date);
    let dg = one_line(p, &date, FontId::proportional(10.5), theme::TEXT_FAINT(), 80.0);
    let dw = dg.size().x;
    p.galley(Pos2::new(rect.right() - 6.0 - dw, rect.top() + 6.0), dg, theme::TEXT_FAINT());
    let ng = one_line(p, m.sender_name(), theme::editor_font(13.0, m.unread()), theme::TEXT(), rect.right() - left - dw - 14.0);
    p.galley(Pos2::new(left, rect.top() + 4.0), ng, theme::TEXT());
    let subject = if m.subject.is_empty() { t!("(sans objet)", "(no subject)") } else { &m.subject };
    let sg = one_line(p, subject, FontId::proportional(12.0), theme::TEXT_MUTED(), rect.right() - left - 6.0);
    p.galley(Pos2::new(left, rect.top() + 23.0), sg, theme::TEXT_MUTED());
    if resp.clicked() {
        *out = Some(DashAction::Mail(m.id.clone()));
    }
    // Même menu que pour les messages épinglés.
    resp.context_menu(|ui| {
        if ui.button(t!("Ouvrir le mail", "Open the mail")).clicked() {
            *out = Some(DashAction::Mail(m.id.clone()));
            ui.close();
        }
        crate::people::menu_item_mailbox(ui, &m.from);
        if pinned && ui.button(t!("📌  Désépingler", "📌  Unpin")).clicked() {
            *unpin = Some(m.id.clone());
            ui.close();
        }
    });
}

// --- Messages épinglés (Google Chat) -----------------------------------------------------

fn pinned_messages(ui: &mut egui::Ui, pins: &mut Pins, out: &mut Option<DashAction>) {
    if pins.list.is_empty() {
        ui.add_space(30.0);
        widgets::empty_state(
            ui,
            t!("Aucun message épinglé", "No pinned messages"),
            t!("Épingle un message depuis l'onglet Messages (📌 au survol)", "Pin a message from the Messages tab (📌 on hover)"),
        );
        return;
    }
    let mut unpin = None;
    egui::ScrollArea::vertical().id_salt("dash_chat_pins").auto_shrink([false, false]).show(ui, |ui| {
        for m in &pins.list {
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
            let resp = resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(t!("Ouvrir la conversation (clic droit : désépingler)", "Open the conversation (right-click: unpin)"));
            let p = ui.painter();
            p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(8));
            if resp.hovered() {
                p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(6));
            }
            let left = rect.left() + 22.0;
            draw_pin(p, Pos2::new(rect.left() + 10.0, rect.top() + 14.0), 5.5, true, theme::TEXT_MUTED());
            let date = short_date(m.created);
            let dg = one_line(p, &date, FontId::proportional(10.5), theme::TEXT_FAINT(), 80.0);
            let dw = dg.size().x;
            p.galley(Pos2::new(rect.right() - 6.0 - dw, rect.top() + 6.0), dg, theme::TEXT_FAINT());
            // Expéditeur, et la conversation si elle porte un autre nom (groupe, espace).
            let who = if m.space_title.is_empty() || m.space_title == m.sender { m.sender.clone() } else { format!("{} · {}", m.sender, m.space_title) };
            let ng = one_line(p, &who, theme::editor_font(13.0, true), theme::TEXT(), rect.right() - left - dw - 14.0);
            p.galley(Pos2::new(left, rect.top() + 4.0), ng, theme::TEXT());
            let text = m.text.replace(['\n', '\r'], " ");
            let sg = one_line(p, &text, FontId::proportional(12.0), theme::TEXT_MUTED(), rect.right() - left - 6.0);
            p.galley(Pos2::new(left, rect.top() + 23.0), sg, theme::TEXT_MUTED());
            if resp.clicked() {
                *out = Some(DashAction::Chat(m.space.clone()));
            }
            resp.context_menu(|ui| {
                if ui.button(t!("Ouvrir la conversation", "Open the conversation")).clicked() {
                    *out = Some(DashAction::Chat(m.space.clone()));
                    ui.close();
                }
                if !m.sender_id.is_empty() {
                    crate::people::menu_item(ui, &m.sender_id, &m.sender);
                }
                if ui.button(t!("📌  Désépingler", "📌  Unpin")).clicked() {
                    unpin = Some(m.name.clone());
                    ui.close();
                }
            });
            ui.add_space(2.0);
        }
    });
    if let Some(name) = unpin {
        pins.remove(&name);
    }
}

// --- Rappels -------------------------------------------------------------------------------

fn reminders(ui: &mut egui::Ui, tasks: &mut Tasks) {
    let now = Local::now().timestamp_millis();
    let panel = ui.max_rect();
    let list: Vec<(u64, String, Status, i64, String)> = tasks.reminders(now).into_iter().map(|(t, d)| (t.id, t.title.clone(), t.status, d, t.summary.clone())).collect();
    let timers = tasks.timers.clone();
    if !timers.is_empty() {
        // Compte à rebours : rafraîchi chaque seconde.
        ui.ctx().request_repaint_after(std::time::Duration::from_secs(1));
    }
    let flash = ui.ctx().data(|d| d.get_temp::<(u64, f64)>(flash_id()));
    let t_now = ui.input(|i| i.time);
    if list.is_empty() && timers.is_empty() {
        ui.add_space(30.0);
        widgets::empty_state(
            ui,
            t!("Rien en retard", "Nothing overdue"),
            t!("Les tâches oubliées et les rappels minutés (+) apparaîtront ici", "Forgotten tasks and timed reminders (+) will appear here"),
        );
    } else {
        egui::ScrollArea::vertical().id_salt("dash_reminders").auto_shrink([false, false]).show(ui, |ui| {
            // Rappels minutés (le plus proche d'abord).
            for tm in &timers {
                let late = tm.due <= now;
                let fill = if late { theme::TODAY().gamma_multiply(0.22) } else { theme::SURFACE_HI() };
                let r = egui::Frame::new().fill(fill).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (ir, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                        let ring = if late { ((t_now * 0.7).fract() as f32 * 2.0).min(1.0) } else { 1.0 };
                        widgets::alarm_icon(ui.painter(), ir.center(), 17.0, if late { theme::TODAY() } else { theme::TEXT() }, ring);
                        ui.add(egui::Label::new(RichText::new(&tm.title).color(theme::TEXT())).wrap());
                    });
                    let when = if late { tf!("Échu · {}", "Due · {}", due_label(tm.due)) } else { format!("{} · {}", until(tm.due - now), due_label(tm.due)) };
                    ui.add(egui::Label::new(RichText::new(when).small().color(if late { theme::TODAY() } else { theme::TEXT_MUTED() })).wrap());
                    ui.horizontal(|ui| {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button("✕").on_hover_text(t!("Annuler le rappel", "Cancel the reminder")).clicked() {
                                tasks.remove_timer(tm.id);
                            }
                            if late && ui.small_button(t!("+10 min", "+10 min")).on_hover_text(t!("Me le rappeler dans 10 minutes", "Remind me in 10 minutes")).clicked() {
                                tasks.snooze(tm.id, 10);
                            }
                            if ui.small_button(t!("✓ Fait", "✓ Done")).on_hover_text(t!("Archiver la tâche", "Archive the task")).clicked() {
                                tasks.set_status(tm.task, Status::Done);
                                tasks.remove_timer(tm.id);
                            }
                        });
                    });
                });
                // Arrivée depuis le calendrier ou une notification : la carte s'illumine.
                if let Some((id, at)) = flash
                    && id == tm.id
                {
                    let k = (1.0 - (t_now - at) / 2.0).clamp(0.0, 1.0) as f32;
                    if k > 0.0 {
                        ui.painter().rect_stroke(r.response.rect, CornerRadius::same(8), Stroke::new(2.0, theme::TEXT().gamma_multiply(k)), egui::StrokeKind::Outside);
                        if k > 0.95 {
                            ui.scroll_to_rect(r.response.rect, Some(Align::Center));
                        }
                        ui.ctx().request_repaint();
                    }
                }
                ui.add_space(6.0);
            }
            // Tâches oubliées.
            for (id, title, status, days, summary) in list {
                egui::Frame::new().fill(theme::SURFACE_HI()).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    let r = ui.add(egui::Label::new(RichText::new(&title).color(theme::TEXT())).wrap());
                    if !summary.is_empty() {
                        r.on_hover_text(&summary);
                    }
                    let what = match status {
                        Status::Todo => tf!("À faire · {}", "To do · {}", tasks::ago(days)),
                        _ => tf!("En cours · {}", "In progress · {}", tasks::ago(days)),
                    };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(what).small().color(if days >= 3 { theme::TODAY() } else { theme::TEXT_MUTED() }));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.small_button(t!("✓ Archiver", "✓ Archive")).clicked() {
                                tasks.set_status(id, Status::Done);
                            }
                            if status == Status::Todo && ui.small_button(t!("▶ Commencer", "▶ Start")).clicked() {
                                tasks.set_status(id, Status::Doing);
                            }
                        });
                    });
                });
                ui.add_space(6.0);
            }
            // Place pour le bouton « + ».
            ui.add_space(34.0);
        });
    }
    add_button(ui, panel, tasks);
}

fn flash_id() -> egui::Id {
    egui::Id::unique("wt_timer_flash")
}

/// Met en évidence un rappel minuté dans le panneau Rappels (depuis le calendrier ou une notification).
pub fn flash_timer(ctx: &egui::Context, id: u64) {
    let t = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(flash_id(), (id, t)));
}

/// « dans 12 min », « dans 2 h 05 », « dans 3 jours ».
fn until(ms: i64) -> String {
    let min = (ms + 59_999) / 60_000;
    match min {
        ..=1 => t!("dans moins d'une minute", "in less than a minute").into(),
        2..=59 => tf!("dans {min} min", "in {min} min"),
        60..=1439 => tf!("dans {} h {:02}", "in {} h {:02}", min / 60, min % 60),
        _ => {
            let d = (min + 720) / 1440;
            tf!("dans {d} jour(s)", "in {d} day(s)")
        }
    }
}

/// « aujourd'hui 21:15 », « demain 09:00 », « ven. 24 oct. 09:00 ».
pub fn due_label(ms: i64) -> String {
    use chrono::Datelike;
    let Some(dt) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let d = dt.date_naive();
    let today = Local::now().date_naive();
    let day = if d == today {
        t!("aujourd'hui", "today").to_string()
    } else if d == today + chrono::Duration::days(1) {
        t!("demain", "tomorrow").to_string()
    } else {
        format!("{} {} {}", crate::tabs::weekday_short(d.weekday()), d.day(), crate::tabs::month_short(d.month()))
    };
    format!("{day} {}", dt.format("%H:%M"))
}

// --- Bouton « + » et rappel minuté -----------------------------------------------------------

/// Formulaire du rappel minuté (gardé entre les images).
#[derive(Clone, Default)]
struct TimerForm {
    open: bool,
    /// Tâche existante, sinon nouvelle tâche (`title`).
    task: Option<u64>,
    title: String,
    /// Échéance : après un délai (`false`) ou à une date fixe (`true`).
    fixed: bool,
    hours: u32,
    minutes: u32,
    date: String,
    time: String,
}

fn form_id() -> egui::Id {
    egui::Id::unique("wt_timer_form")
}

/// Bouton « + » en bas à droite du panneau : menu d'actions (rappel minuté…).
fn add_button(ui: &mut egui::Ui, panel: Rect, tasks: &mut Tasks) {
    let c = Pos2::new(panel.right() - 16.0, panel.bottom() - 16.0);
    let r = Rect::from_center_size(c, vec2(30.0, 30.0));
    let resp = ui.interact(r, egui::Id::unique("wt_reminders_add"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Ajouter…", "Add…"));
    let k = ui.ctx().animate_bool_with_time(resp.id.with("hover"), resp.hovered(), 0.15);
    let p = ui.painter();
    p.circle_filled(c, 15.0 + 1.5 * k, crate::sidebar::lerp_color(theme::TEXT(), theme::TEXT_MUTED(), k));
    let s = Stroke::new(2.0, theme::BG());
    p.line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], s);
    p.line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], s);
    egui::Popup::menu(&resp).align(egui::RectAlign::TOP_END).show(|ui| {
        ui.set_min_width(220.0);
        ui.label(RichText::new(t!("AJOUTER", "ADD")).small().color(theme::TEXT_FAINT()));
        if ui
            .button(t!("⏰  Rappel minuté…", "⏰  Timed reminder…"))
            .on_hover_text(t!("Être rappelé d'une tâche après un délai ou à une date fixe", "Get reminded of a task after a delay or at a set date"))
            .clicked()
        {
            let now = Local::now() + chrono::Duration::hours(1);
            let form = TimerForm {
                open: true,
                task: tasks.column(Status::Todo).first().or(tasks.column(Status::Doing).first()).map(|t| t.id),
                minutes: 30,
                date: now.format("%d/%m/%Y").to_string(),
                time: now.format("%H:00").to_string(),
                ..Default::default()
            };
            ui.ctx().data_mut(|d| d.insert_temp(form_id(), form));
            ui.close();
        }
    });
    timer_form(ui.ctx(), r, tasks);
}

/// Fenêtre de création du rappel, au-dessus du bouton « + ».
fn timer_form(ctx: &egui::Context, anchor: Rect, tasks: &mut Tasks) {
    let Some(mut f) = ctx.data(|d| d.get_temp::<TimerForm>(form_id())) else { return };
    if !f.open {
        return;
    }
    let now = Local::now();
    let mut close = false;
    let mut submit = false;
    egui::Area::new(egui::Id::unique("wt_timer_form_area")).order(egui::Order::Foreground).pivot(Align2::RIGHT_BOTTOM).fixed_pos(anchor.right_top() - vec2(0.0, 8.0)).show(
        ctx,
        |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(14)).corner_radius(CornerRadius::same(12)).show(ui, |ui| {
                ui.set_width(320.0);
                ui.horizontal(|ui| {
                    widgets::alarm_icon(ui.painter(), ui.cursor().min + vec2(9.0, 10.0), 18.0, theme::TEXT(), 1.0);
                    ui.add_space(22.0);
                    ui.label(RichText::new(t!("Rappel minuté", "Timed reminder")).font(theme::title_font(16.0)));
                });
                ui.add_space(8.0);
                // Tâche : existante, ou nouvelle (ajoutée à « À faire »).
                ui.label(RichText::new(t!("TÂCHE", "TASK")).small().color(theme::TEXT_FAINT()));
                let open_tasks: Vec<(u64, String)> = tasks.column(Status::Todo).into_iter().chain(tasks.column(Status::Doing)).map(|t| (t.id, t.title.clone())).collect();
                let current =
                    f.task.and_then(|id| open_tasks.iter().find(|(i, _)| *i == id)).map(|(_, t)| t.clone()).unwrap_or_else(|| t!("➕ Nouvelle tâche", "➕ New task").into());
                egui::ComboBox::from_id_salt("wt_timer_task").selected_text(current).width(ui.available_width()).show_ui(ui, |ui| {
                    if ui.selectable_label(f.task.is_none(), t!("➕ Nouvelle tâche", "➕ New task")).clicked() {
                        f.task = None;
                    }
                    for (id, title) in &open_tasks {
                        if ui.selectable_label(f.task == Some(*id), title).clicked() {
                            f.task = Some(*id);
                        }
                    }
                });
                if f.task.is_none() {
                    let te = ui.add(egui::TextEdit::singleline(&mut f.title).hint_text(t!("Que faut-il faire ?", "What needs doing?")).desired_width(f32::INFINITY));
                    if f.title.is_empty() && !te.has_focus() && ui.memory(|m| m.focused().is_none()) {
                        te.request_focus();
                    }
                }
                ui.add_space(10.0);
                ui.label(RichText::new(t!("QUAND", "WHEN")).small().color(theme::TEXT_FAINT()));
                let mut fixed = f.fixed;
                widgets::segmented(ui, "wt_timer_mode", &[(false, t!("Dans…", "In…")), (true, t!("Le…", "On…"))], &mut fixed);
                f.fixed = fixed;
                ui.add_space(6.0);
                if !f.fixed {
                    ui.horizontal_wrapped(|ui| {
                        for (label, m) in [("5 min", 5), ("15 min", 15), ("30 min", 30), ("1 h", 60), ("2 h", 120), (t!("1 jour", "1 day"), 1440)] {
                            if ui.add(egui::Button::new(label).selected(f.hours * 60 + f.minutes == m)).clicked() {
                                f.hours = m / 60;
                                f.minutes = m % 60;
                            }
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::DragValue::new(&mut f.hours).range(0..=720).suffix(" h"));
                        ui.add(egui::DragValue::new(&mut f.minutes).range(0..=59).suffix(" min"));
                    });
                } else {
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut f.date).desired_width(96.0).hint_text("jj/mm/aaaa"));
                        ui.add(egui::TextEdit::singleline(&mut f.time).desired_width(60.0).hint_text("hh:mm"));
                    });
                    ui.horizontal_wrapped(|ui| {
                        let today = now.date_naive();
                        let mut quick = |ui: &mut egui::Ui, label: &str, d: chrono::NaiveDate, t: &str| {
                            if ui.button(label).clicked() {
                                f.date = d.format("%d/%m/%Y").to_string();
                                f.time = t.to_string();
                            }
                        };
                        quick(ui, t!("Ce soir 18:00", "Tonight 18:00"), today, "18:00");
                        quick(ui, t!("Demain 9:00", "Tomorrow 9:00"), today + chrono::Duration::days(1), "09:00");
                        let monday = today + chrono::Duration::days(7 - chrono::Datelike::weekday(&today).num_days_from_monday() as i64);
                        quick(ui, t!("Lundi 9:00", "Monday 9:00"), monday, "09:00");
                    });
                }
                let due = due_of(&f, now.timestamp_millis());
                ui.add_space(6.0);
                match &due {
                    Ok(ms) => {
                        ui.label(RichText::new(tf!("Rappel {} ({})", "Reminder {} ({})", due_label(*ms), until(*ms - now.timestamp_millis()))).small().color(theme::TEXT_MUTED()))
                    }
                    Err(e) => ui.label(RichText::new(e).small().color(theme::TODAY())),
                };
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let ready = due.is_ok() && (f.task.is_some() || !f.title.trim().is_empty());
                    if ui.add_enabled(ready, egui::Button::new(t!("Valider", "Confirm"))).clicked() {
                        submit = true;
                    }
                    if ui.button(t!("Annuler", "Cancel")).clicked() {
                        close = true;
                    }
                });
                if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                    close = true;
                }
                if ui.input(|i| i.key_pressed(egui::Key::Enter)) && due.is_ok() && (f.task.is_some() || !f.title.trim().is_empty()) {
                    submit = true;
                }
            });
        },
    );
    if submit && let Ok(ms) = due_of(&f, now.timestamp_millis()) {
        let task = match f.task {
            Some(id) => Some(id),
            None => tasks.add_task(&f.title),
        };
        if let Some(id) = task {
            tasks.add_timer(id, ms);
        }
        close = true;
    }
    if close {
        f.open = false;
    }
    ctx.data_mut(|d| d.insert_temp(form_id(), f));
}

/// Échéance du formulaire (ms), ou la raison pour laquelle elle n'est pas valable.
fn due_of(f: &TimerForm, now: i64) -> Result<i64, String> {
    if !f.fixed {
        let min = f.hours as i64 * 60 + f.minutes as i64;
        if min == 0 {
            return Err(t!("Choisis un délai", "Choose a delay").into());
        }
        return Ok(now + min * 60_000);
    }
    let d = chrono::NaiveDate::parse_from_str(f.date.trim(), "%d/%m/%Y").map_err(|_| t!("Date invalide (jj/mm/aaaa)", "Invalid date (dd/mm/yyyy)").to_string())?;
    let t = f.time.trim().replace(['h', 'H'], ":");
    let t = if t.ends_with(':') {
        format!("{t}00")
    } else if !t.contains(':') {
        format!("{t}:00")
    } else {
        t
    };
    let t = chrono::NaiveTime::parse_from_str(&t, "%H:%M").map_err(|_| t!("Heure invalide (hh:mm)", "Invalid time (hh:mm)").to_string())?;
    let ms = Local.from_local_datetime(&d.and_time(t)).earliest().map(|x| x.timestamp_millis()).ok_or_else(|| t!("Heure invalide", "Invalid time").to_string())?;
    if ms <= now {
        return Err(t!("Cette heure est déjà passée", "This time has already passed").into());
    }
    Ok(ms)
}

// --- Kanban -----------------------------------------------------------------------------------

struct TaskDrag(u64);

fn kanban(ui: &mut egui::Ui, tasks: &mut Tasks) {
    let gap = 10.0;
    let col_w = (ui.available_width() - 2.0 * gap) / 3.0;
    let h = ui.available_height();
    let dragging = egui::DragAndDrop::payload::<TaskDrag>(ui.ctx());
    let mut edits = CardEdits::default();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for status in Status::ALL {
            let r = ui
                .allocate_ui_with_layout(vec2(col_w, h), Layout::top_down(Align::Min), |ui| {
                    ui.set_min_size(vec2(col_w, h));
                    egui::Frame::new().fill(theme::BG()).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                        ui.set_width(col_w - 16.0);
                        ui.set_min_height(h - 16.0);
                        let items = tasks.column(status);
                        ui.horizontal(|ui| {
                            let c = match status {
                                Status::Todo => theme::TEXT_MUTED(),
                                Status::Doing => theme::LINK(),
                                Status::Done => Color32::from_rgb(0x4C, 0xAF, 0x7A),
                            };
                            let (d, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(d.center(), 4.0, c);
                            ui.label(RichText::new(status.label().to_uppercase()).small().strong().color(theme::TEXT_MUTED()));
                            ui.label(RichText::new(items.len().to_string()).small().color(theme::TEXT_FAINT()));
                            if status == Status::Done {
                                ui.label(RichText::new(t!("· vidé chaque jour", "· cleared every day")).small().color(theme::TEXT_FAINT()));
                            }
                        });
                        ui.add_space(6.0);
                        if status == Status::Todo {
                            let id = egui::Id::unique("wt_dash_new_task");
                            let mut text: String = ui.ctx().data(|d| d.get_temp(id)).unwrap_or_default();
                            let r =
                                ui.add(egui::TextEdit::singleline(&mut text).hint_text(t!("+ Ajouter une tâche (Entrée)", "+ Add a task (Enter)")).desired_width(f32::INFINITY));
                            if r.changed()
                                && let Some(t) = crate::tabs::messages::emoji::replace_codes(&text)
                            {
                                text = t;
                            }
                            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                                tasks.add(&text);
                                text.clear();
                                r.request_focus();
                            }
                            ui.ctx().data_mut(|d| d.insert_temp(id, text));
                            ui.add_space(6.0);
                        }
                        let items: Vec<tasks::Task> = tasks.column(status).into_iter().cloned().collect();
                        let alarms: Vec<(u64, i64)> = tasks.timers.iter().map(|t| (t.task, t.due)).collect();
                        egui::ScrollArea::vertical().id_salt(("dash_kanban", status.label())).auto_shrink([false, false]).show(ui, |ui| {
                            for t in &items {
                                let alarm = alarms.iter().filter(|(x, _)| *x == t.id).map(|(_, d)| *d).min();
                                task_card(ui, t, alarm, &mut edits);
                            }
                            if tasks.column(status).is_empty() {
                                ui.label(RichText::new(if dragging.is_some() { t!("Déposer ici", "Drop here") } else { "—" }).small().color(theme::TEXT_FAINT()));
                            }
                        });
                    });
                })
                .response;
            // Dépôt d'une tâche glissée dans cette colonne.
            if let Some(d) = &dragging
                && ui.ctx().pointer_latest_pos().is_some_and(|p| r.rect.contains(p))
            {
                ui.painter().rect_stroke(r.rect, CornerRadius::same(10), Stroke::new(1.5, theme::TODAY()), egui::StrokeKind::Inside);
                if ui.input(|i| i.pointer.any_released()) {
                    edits.moves.push((d.0, status));
                    egui::DragAndDrop::clear_payload(ui.ctx());
                }
            }
        }
    });
    if let (Some(d), Some(p)) = (dragging, ui.ctx().pointer_latest_pos())
        && let Some(t) = tasks.list.iter().find(|t| t.id == d.0)
    {
        let title = t.title.clone();
        egui::Area::new(egui::Id::unique("wt_task_ghost")).order(egui::Order::Tooltip).fixed_pos(p + vec2(12.0, 8.0)).interactable(false).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(RichText::new(title).strong());
            });
        });
    }
    for (id, s) in edits.moves {
        tasks.set_status(id, s);
    }
    for (id, t) in edits.renames {
        tasks.rename(id, &t);
    }
    for (id, t) in edits.summaries {
        tasks.set_summary(id, &t);
    }
    for id in edits.deletes {
        tasks.remove(id);
    }
}

/// Modifications demandées depuis les cartes du Kanban (appliquées après l'affichage).
#[derive(Default)]
struct CardEdits {
    moves: Vec<(u64, Status)>,
    deletes: Vec<u64>,
    renames: Vec<(u64, String)>,
    summaries: Vec<(u64, String)>,
}

/// Lignes du résumé affichées sur la carte (le reste au survol).
const SUMMARY_LINES: usize = 4;

/// Carte d'une tâche : titre, résumé, ancienneté, rappel (`alarm`), boutons.
/// Double-clic : renommer ; clic droit : renommer, résumé, effacer.
fn task_card(ui: &mut egui::Ui, task: &tasks::Task, alarm: Option<i64>, out: &mut CardEdits) {
    let (id, title, since, status) = (task.id, task.title.as_str(), task.since, task.status);
    let edit_id = egui::Id::unique(("wt_task_edit", id));
    let editing: Option<String> = ui.ctx().data(|d| d.get_temp(edit_id));
    let sum_id = egui::Id::unique(("wt_task_summary", id));
    let summary_edit: Option<String> = ui.ctx().data(|d| d.get_temp(sum_id));
    // Zone de la carte (glisser, double-clic, clic droit) enregistrée AVANT son contenu, à la place
    // qu'elle occupait à l'image précédente : les boutons et le résumé, dessinés ensuite, restent
    // au-dessus et cliquables. Pendant une saisie, la carte ne capte pas la souris.
    let rect_id = egui::Id::unique(("wt_task_rect", id));
    let busy = editing.is_some() || summary_edit.is_some();
    let card = if busy { None } else { ui.ctx().data(|d| d.get_temp::<Rect>(rect_id)).map(|r| ui.interact(r, egui::Id::unique(("wt_task_card", id)), Sense::click_and_drag())) };
    let frame = egui::Frame::new()
        .fill(theme::SURFACE_HI())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // Textes non sélectionnables : un glisser sur la carte la déplace.
            ui.style_mut().interaction.selectable_labels = false;
            if let Some(mut t) = editing.clone() {
                // Renommage : Entrée ou clic ailleurs pour valider, Échap pour annuler.
                let field_id = edit_id.with("field");
                let mut done = false;
                let mut cancel = false;
                ui.horizontal(|ui| {
                    let r = ui.add(egui::TextEdit::singleline(&mut t).id(field_id).desired_width(ui.available_width() - 60.0));
                    if r.changed()
                        && let Some(x) = crate::tabs::messages::emoji::replace_codes(&t)
                    {
                        t = x;
                    }
                    // Focus donné une seule fois, à l'ouverture (sinon le champ ne peut jamais être quitté).
                    let started = edit_id.with("started");
                    if !ui.ctx().data(|d| d.get_temp::<bool>(started)).unwrap_or(false) {
                        r.request_focus();
                        ui.ctx().data_mut(|d| d.insert_temp(started, true));
                    }
                    if r.lost_focus() {
                        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                            cancel = true;
                        } else {
                            done = true;
                        }
                    }
                    if ui.small_button("✓").on_hover_text(t!("Valider (Entrée)", "Confirm (Enter)")).clicked() {
                        done = true;
                    }
                    if ui.small_button("✕").on_hover_text(t!("Annuler (Échap)", "Cancel (Esc)")).clicked() {
                        cancel = true;
                    }
                });
                if done || cancel {
                    if done && !t.trim().is_empty() {
                        out.renames.push((id, t.trim().to_string()));
                    }
                    ui.ctx().data_mut(|d| {
                        d.remove::<String>(edit_id);
                        d.remove::<bool>(edit_id.with("started"));
                    });
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(edit_id, t));
                }
                return;
            }
            ui.add(egui::Label::new(RichText::new(title).color(if status == Status::Done { theme::TEXT_MUTED() } else { theme::TEXT() })).wrap());
            if let Some(mut s) = summary_edit.clone() {
                // Résumé : champ sur plusieurs lignes (Entrée = nouvelle ligne).
                ui.add_space(4.0);
                let field_id = sum_id.with("field");
                let r = ui.add(
                    egui::TextEdit::multiline(&mut s)
                        .id(field_id)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .hint_text(t!("Résumé, notes, étapes… (plusieurs lignes)", "Summary, notes, steps… (several lines)")),
                );
                if r.changed()
                    && let Some(x) = crate::tabs::messages::emoji::replace_codes(&s)
                {
                    s = x;
                }
                let started = sum_id.with("started");
                if !ui.ctx().data(|d| d.get_temp::<bool>(started)).unwrap_or(false) {
                    r.request_focus();
                    ui.ctx().data_mut(|d| d.insert_temp(started, true));
                }
                let mut done = r.has_focus() && ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Enter));
                let mut cancel = ui.input(|i| i.key_pressed(egui::Key::Escape));
                ui.horizontal(|ui| {
                    if ui.small_button(t!("✓ Enregistrer", "✓ Save")).on_hover_text(t!("Ctrl + Entrée", "Ctrl + Enter")).clicked() {
                        done = true;
                    }
                    if ui.small_button(t!("Annuler", "Cancel")).on_hover_text(t!("Échap", "Esc")).clicked() {
                        cancel = true;
                    }
                });
                if done || cancel {
                    if done {
                        out.summaries.push((id, s));
                    }
                    ui.ctx().data_mut(|d| {
                        d.remove::<String>(sum_id);
                        d.remove::<bool>(sum_id.with("started"));
                    });
                } else {
                    ui.ctx().data_mut(|d| d.insert_temp(sum_id, s));
                }
                return;
            }
            // Résumé : premières lignes, le texte complet au survol ; clic = le modifier.
            if !task.summary.trim().is_empty() {
                let lines: Vec<&str> = task.summary.lines().collect();
                let mut shown = lines.iter().take(SUMMARY_LINES).copied().collect::<Vec<_>>().join("\n");
                if lines.len() > SUMMARY_LINES {
                    shown.push_str("\n…");
                }
                ui.add_space(2.0);
                let r = ui.add(egui::Label::new(RichText::new(shown).small().color(theme::TEXT_MUTED())).wrap().sense(Sense::click())).on_hover_cursor(egui::CursorIcon::Text);
                let r = if lines.len() > SUMMARY_LINES { r.on_hover_text(&task.summary) } else { r.on_hover_text(t!("Clic : modifier le résumé", "Click: edit the summary")) };
                if r.clicked() {
                    ui.ctx().data_mut(|d| d.insert_temp(sum_id, task.summary.clone()));
                }
                ui.add_space(2.0);
            }
            let days = (Local::now().timestamp_millis() - since) / 86_400_000;
            ui.horizontal(|ui| {
                let when = match status {
                    Status::Done => tf!("archivée à {}", "archived at {}", Local.timestamp_millis_opt(since).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()),
                    _ => tasks::ago(days),
                };
                ui.label(RichText::new(when).small().color(theme::TEXT_FAINT()));
                if let Some(due) = alarm {
                    let late = due <= Local::now().timestamp_millis();
                    let color = if late { theme::TODAY() } else { theme::TEXT_MUTED() };
                    let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
                    widgets::alarm_icon(ui.painter(), r.center(), 12.0, color, 1.0);
                    let hm = Local.timestamp_millis_opt(due).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default();
                    ui.label(RichText::new(hm).small().color(color)).on_hover_text(tf!("Rappel {}", "Reminder {}", due_label(due)));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let small = |ui: &mut egui::Ui, t: &str, tip: &str| ui.add(egui::Button::new(RichText::new(t).small()).frame(false)).on_hover_text(tip).clicked();
                    if small(ui, "✕", t!("Supprimer", "Delete")) {
                        out.deletes.push(id);
                    }
                    match status {
                        Status::Todo => {
                            if small(ui, "▶", t!("Passer en cours", "Move to in progress")) {
                                out.moves.push((id, Status::Doing));
                            }
                        }
                        Status::Doing => {
                            if small(ui, "✓", t!("Archiver", "Archive")) {
                                out.moves.push((id, Status::Done));
                            }
                            if small(ui, "◀", t!("Remettre à faire", "Move back to to-do")) {
                                out.moves.push((id, Status::Todo));
                            }
                        }
                        Status::Done => {
                            if small(ui, "◀", t!("Reprendre", "Resume")) {
                                out.moves.push((id, Status::Doing));
                            }
                        }
                    }
                    let tip = if task.summary.is_empty() { t!("Ajouter un résumé", "Add a summary") } else { t!("Modifier le résumé", "Edit the summary") };
                    if small(ui, "≡", tip) {
                        ui.ctx().data_mut(|d| d.insert_temp(sum_id, task.summary.clone()));
                    }
                });
            });
        });
    ui.ctx().data_mut(|d| d.insert_temp(rect_id, frame.response.rect));
    // Glisser la carte d'une colonne à l'autre ; double-clic pour renommer.
    let Some(resp) = card else {
        ui.add_space(6.0);
        return;
    };
    resp.dnd_set_drag_payload(TaskDrag(id));
    if resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }
    if resp.double_clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(edit_id, title.to_string()));
    }
    // Clic droit : petit menu (renommer, résumé, effacer).
    resp.context_menu(|ui| {
        if ui.button(t!("✏  Renommer", "✏  Rename")).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(edit_id, title.to_string()));
            ui.close();
        }
        let label = if task.summary.is_empty() { t!("≡  Ajouter un résumé", "≡  Add a summary") } else { t!("≡  Modifier le résumé", "≡  Edit the summary") };
        if ui.button(label).clicked() {
            ui.ctx().data_mut(|d| d.insert_temp(sum_id, task.summary.clone()));
            ui.close();
        }
        if ui.button(RichText::new(t!("🗑  Effacer la tâche", "🗑  Delete the task")).color(theme::TODAY())).clicked() {
            out.deletes.push(id);
            ui.close();
        }
    });
    ui.add_space(6.0);
}

/// Phrase d'accueil, tirée au hasard au lancement parmi une douzaine.
fn greeting() -> &'static str {
    const PHRASES: [&str; 12] = [
        "Ravi de te revoir",
        "Prêt pour une belle journée ?",
        "Content de te voir ici",
        "On s'y met ?",
        "Hello, toi !",
        "Bienvenue à bord",
        "Que la journée soit douce",
        "Un café et c'est parti ☕",
        "Allez, on avance ensemble",
        "Belle énergie aujourd'hui !",
        "Coucou, quoi de neuf ?",
        "Bon retour parmi nous",
    ];
    const PHRASES_EN: [&str; 12] = [
        "Good to see you again",
        "Ready for a great day?",
        "Glad to have you here",
        "Shall we get started?",
        "Hello, you!",
        "Welcome aboard",
        "Have a gentle day",
        "Coffee, and off we go ☕",
        "Come on, let's move forward",
        "Great energy today!",
        "Hey, what's new?",
        "Welcome back",
    ];
    static PICK: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let i = *PICK.get_or_init(|| {
        let mut b = [0u8; 8];
        let seed = getrandom::fill(&mut b).map(|_| u64::from_le_bytes(b)).unwrap_or_else(|_| Local::now().timestamp_nanos_opt().unwrap_or(0) as u64);
        (seed % PHRASES.len() as u64) as usize
    });
    t!(PHRASES, PHRASES_EN)[i]
}

/// Panneau Tâches : Kanban, ou les archives quand l'onglet « Archives » (à côté du titre) est ouvert.
fn tasks_panel(ui: &mut egui::Ui, tasks: &mut Tasks, doing: &str) {
    let id = egui::Id::unique("wt_dash_archives_open");
    let open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let n = tasks.archive.len();
    let mut toggle = false;
    panel_with(
        ui,
        |ui| {
            ui.label(widgets::section_title(t!("Tâches", "Tasks")));
            ui.label(RichText::new(doing).small().color(theme::TODAY()));
            ui.add_space(8.0);
            // Petit onglet dépliable « Archives » : chevron qui pivote.
            let label = tf!("ARCHIVES · {n}", "ARCHIVE · {n}");
            let g = ui.painter().layout_no_wrap(label, FontId::proportional(11.5), theme::TEXT_MUTED());
            let (r, resp) = ui.allocate_exact_size(vec2(g.size().x + 32.0, 22.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if open {
                t!("Revenir aux tâches", "Back to tasks")
            } else {
                t!("Voir les tâches archivées", "See archived tasks")
            });
            let k = liquid::animate(ui.ctx(), "dash_archives", open);
            let p = ui.painter();
            let bg = if open || resp.hovered() { theme::SURFACE_HI() } else { Color32::TRANSPARENT };
            p.rect(r, CornerRadius::same(255), bg, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
            let c = Pos2::new(r.left() + 13.0, r.center().y);
            let a = k * std::f32::consts::FRAC_PI_2;
            let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
            let col = if open || resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
            p.add(egui::Shape::line(vec![rot(-2.0, -4.0), rot(2.5, 0.0), rot(-2.0, 4.0)], Stroke::new(1.5, col)));
            p.galley(Pos2::new(r.left() + 23.0, r.center().y - g.size().y / 2.0), g, col);
            toggle = resp.clicked();
        },
        |ui| {
            if open {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| archives_list(ui, tasks));
            } else {
                kanban(ui, tasks);
            }
        },
    );
    if toggle {
        ui.ctx().data_mut(|d| d.insert_temp(id, !open));
    }
}

/// Tâches archivées, groupées par jour, avec restauration ou suppression.
fn archives_list(ui: &mut egui::Ui, tasks: &mut Tasks) {
    if tasks.archive.is_empty() {
        ui.add_space(30.0);
        widgets::empty_state(
            ui,
            t!("Aucune archive", "No archive"),
            t!("Les tâches archivées les jours précédents sont rangées ici.", "Tasks archived on previous days are kept here."),
        );
        return;
    }
    let mut restore = None;
    let mut delete = None;
    let mut day = String::new();
    for t in tasks.archive.iter().take(200) {
        let d = Local.timestamp_millis_opt(t.since).single().map(|d| d.date_naive()).unwrap_or_default();
        let label = format!("{} {} {}", super::weekday_name(chrono::Datelike::weekday(&d)), chrono::Datelike::day(&d), super::month_name(chrono::Datelike::month(&d)));
        if label != day {
            ui.add_space(6.0);
            ui.label(RichText::new(&label).small().strong().color(theme::TEXT_MUTED()));
            day = label;
        }
        ui.horizontal(|ui| {
            ui.add_space(12.0);
            ui.label(RichText::new("✓").small().color(Color32::from_rgb(0x4C, 0xAF, 0x7A)));
            ui.label(RichText::new(&t.title).color(theme::TEXT_MUTED()));
            if ui.small_button(t!("Restaurer", "Restore")).on_hover_text(t!("Remettre dans « À faire »", "Move back to “To do”")).clicked() {
                restore = Some(t.id);
            }
            if ui.small_button("✕").on_hover_text(t!("Supprimer définitivement", "Delete permanently")).clicked() {
                delete = Some(t.id);
            }
        });
    }
    if let Some(i) = restore {
        tasks.restore(i);
    }
    if let Some(i) = delete {
        tasks.remove(i);
    }
}

// --- Fichiers reçus ----------------------------------------------------------------------

/// Hauteur d'une ligne de la liste des fichiers reçus.
const FILE_ROW: f32 = 46.0;

fn files(ui: &mut egui::Ui, mail: &mut MailState, acc: &Account, two_cols: bool, out: &mut Option<DashAction>) {
    let list: Vec<(Summary, SharedFile)> = mail.shared_files.clone();
    if list.is_empty() {
        ui.add_space(30.0);
        widgets::empty_state(
            ui,
            t!("Aucun fichier partagé", "No shared files"),
            if acc.session().is_some() {
                t!("PDF et documents Google reçus par mail", "PDFs and Google documents received by mail")
            } else {
                t!("Gmail non connecté", "Gmail not connected")
            },
        );
        return;
    }
    let cols = if two_cols { 2 } else { 1 };
    let n = list.len().min(crate::tabs::mail::SHARED_FILES);
    let gap = 16.0;
    let col_w = (ui.available_width() - gap * (cols - 1) as f32) / cols as f32;
    let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), n.div_ceil(cols) as f32 * FILE_ROW), Sense::hover());
    // Remplissage colonne par colonne (les plus récents en haut à gauche).
    let per_col = n.div_ceil(cols);
    for (k, (m, f)) in list.iter().take(n).enumerate() {
        let (c, r) = (k / per_col, k % per_col);
        let rect = Rect::from_min_size(area.min + vec2(c as f32 * (col_w + gap), r as f32 * FILE_ROW), vec2(col_w, FILE_ROW - 2.0));
        let resp = ui.interact(rect, egui::Id::unique(("wt_dash_file", k)), Sense::click());
        let tip = if f.url.is_some() {
            t!("Ouvrir dans le navigateur (clic droit : voir le mail)", "Open in the browser (right-click: see the mail)")
        } else {
            t!("Ouvrir le PDF (clic droit : voir le mail)", "Open the PDF (right-click: see the mail)")
        };
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        let p = ui.painter();
        if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(6));
        }
        let (label, color) = kind_style(f.kind);
        let tag = Rect::from_min_size(rect.left_top() + vec2(4.0, 8.0), vec2(44.0, 28.0));
        p.rect_filled(tag, CornerRadius::same(6), color);
        p.text(tag.center(), Align2::CENTER_CENTER, label, theme::editor_font(10.0, true), Color32::WHITE);
        let left = rect.left() + 58.0;
        let g = one_line(p, &f.title, theme::editor_font(13.0, false), theme::LINK(), rect.right() - left - 4.0);
        p.galley(Pos2::new(left, rect.top() + 5.0), g, theme::LINK());
        let sub = format!("{} · {}", m.sender_name(), short_date(m.date));
        let g = one_line(p, &sub, FontId::proportional(11.0), theme::TEXT_FAINT(), rect.right() - left - 4.0);
        p.galley(Pos2::new(left, rect.top() + 24.0), g, theme::TEXT_FAINT());
        let mut open = resp.clicked();
        resp.context_menu(|ui| {
            if ui.button(t!("Voir le mail", "See the mail")).clicked() {
                *out = Some(DashAction::Mail(m.id.clone()));
                ui.close();
            }
            if ui.button(t!("Ouvrir le fichier", "Open the file")).clicked() {
                open = true;
                ui.close();
            }
            crate::people::menu_item_mailbox(ui, &m.from);
        });
        if open {
            match (&f.url, &f.attachment) {
                (Some(u), _) => {
                    let _ = webbrowser::open(u);
                }
                (None, Some(a)) => mail.open_attachment_external(acc, a),
                _ => {}
            }
        }
    }
    if let Some(n) = &mail.notice {
        ui.add_space(4.0);
        ui.label(RichText::new(&n.text).small().color(if n.error { theme::TODAY() } else { theme::TEXT_MUTED() }));
    }
}

/// Pastille du type de fichier (couleurs des applications Google).
fn kind_style(k: SharedKind) -> (&'static str, Color32) {
    match k {
        SharedKind::Pdf => ("PDF", Color32::from_rgb(0xC6, 0x45, 0x3B)),
        SharedKind::Doc => ("DOCS", Color32::from_rgb(0x2B, 0x6C, 0xD6)),
        SharedKind::Sheet => ("SHEETS", Color32::from_rgb(0x1E, 0x8E, 0x4E)),
        SharedKind::Form => ("FORMS", Color32::from_rgb(0x67, 0x3A, 0xB7)),
    }
}

// --- Liens Sites Important ---------------------------------------------------------------

/// Galerie 3 × 3 de logos cliquables vers des sites.
fn links_panel(ui: &mut egui::Ui, links: &mut crate::links::Links) {
    // Fond terracotta semi-transparent (posé sur le fond sombre de l'app).
    panel_styled(
        ui,
        Color32::from_rgba_unmultiplied(0xD0, 0x6A, 0x4E, 56),
        Color32::from_rgba_unmultiplied(0xD0, 0x6A, 0x4E, 120),
        |ui| {
            // Titre en terracotta sur fond rouge semi-transparent.
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(0xD0, 0x4E, 0x3A, 46))
                .corner_radius(CornerRadius::same(255))
                .inner_margin(egui::Margin::symmetric(10, 3))
                .show(ui, |ui| {
                    ui.label(RichText::new(t!("Liens Sites Important", "Important site links")).font(theme::title_font(14.0)).color(Color32::from_rgb(0xD0, 0x6A, 0x4E)));
                });
        },
        |ui| links_grid(ui, links),
    );
    links_editor(ui.ctx(), links);
}

#[derive(Clone, Copy)]
struct LinkDrag(usize);

fn links_grid(ui: &mut egui::Ui, links: &mut crate::links::Links) {
    const GAP_T: f32 = 10.0;
    let avail = ui.available_size();
    // Place gardée sous la grille pour la ligne d'aide.
    let side = ((avail.x.min(avail.y - 4.0) - 2.0 * GAP_T) / 3.0).floor().max(36.0);
    let grid_w = side * 3.0 + GAP_T * 2.0;
    let (area, _) = ui.allocate_exact_size(vec2(avail.x, grid_w), Sense::hover());
    let origin = Pos2::new(area.center().x - grid_w / 2.0, area.top());
    let mut open = None;
    let mut edit = None;
    let mut clear = None;
    let mut swap = None;
    for k in 0..crate::links::COUNT {
        let r = Rect::from_min_size(origin + vec2((k % 3) as f32 * (side + GAP_T), (k / 3) as f32 * (side + GAP_T)), vec2(side, side));
        let resp = ui.interact(r, egui::Id::unique(("wt_link_tile", k)), Sense::click_and_drag());
        let t = links.tiles[k].clone();
        let hover = resp.hovered();
        let p = ui.painter();
        if t.is_empty() {
            // Case libre : « + ».
            let col = if hover { theme::TEXT_MUTED() } else { theme::BORDER() };
            p.rect_stroke(r.shrink(1.0), CornerRadius::same(14), Stroke::new(1.0, col), egui::StrokeKind::Inside);
            p.text(r.center(), Align2::CENTER_CENTER, "+", egui::FontId::proportional(side * 0.3), col);
            if resp.clone().on_hover_text(t!("Ajouter un lien", "Add a link")).clicked() {
                edit = Some(k);
            }
        } else {
            let lift = ui.ctx().animate_bool_with_time(egui::Id::unique(("wt_link_hover", k)), hover, 0.12);
            let rr = r.shrink(2.0 - 2.0 * lift);
            p.rect_filled(rr, CornerRadius::same(14), if hover { crate::theme::gray(0x2C) } else { theme::SURFACE_HI() });
            match t.image.as_ref().and_then(|f| logo_texture(ui.ctx(), links, f)) {
                Some(tex) => {
                    let ir = rr.shrink(side * 0.12);
                    ui.painter()
                        .add(egui::epaint::RectShape::filled(ir, CornerRadius::same(10), Color32::WHITE).with_texture(tex, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))));
                }
                None => {
                    let e = if t.emoji.is_empty() { "🔗" } else { t.emoji.as_str() };
                    let s = side * 0.52;
                    crate::emoji_img::paint(ui.painter(), Rect::from_center_size(rr.center(), vec2(s, s)), e, theme::TEXT());
                }
            }
            let resp = resp.clone().on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(format!("{}\n{}", t.name, t.url));
            if resp.clicked() {
                open = Some(t.url.clone());
            }
            resp.dnd_set_drag_payload(LinkDrag(k));
            resp.context_menu(|ui| {
                if ui.button(t!("🌐  Ouvrir", "🌐  Open")).clicked() {
                    open = Some(t.url.clone());
                }
                if ui.button(t!("✏  Modifier…", "✏  Edit…")).clicked() {
                    edit = Some(k);
                }
                if ui.button(t!("🗑  Vider la case", "🗑  Clear the slot")).clicked() {
                    clear = Some(k);
                }
            });
        }
        // Glisser une case sur une autre : échange.
        if let Some(d) = resp.dnd_hover_payload::<LinkDrag>()
            && d.0 != k
        {
            ui.painter().rect_stroke(r, CornerRadius::same(14), Stroke::new(1.5, theme::TEXT()), egui::StrokeKind::Inside);
        }
        if let Some(d) = resp.dnd_release_payload::<LinkDrag>() {
            swap = Some((d.0, k));
        }
    }
    if let Some(u) = open {
        let _ = webbrowser::open(&u);
    }
    if let Some(k) = edit {
        links.start_edit(k);
    }
    if let Some(k) = clear {
        links.clear(k);
    }
    if let Some((a, b)) = swap {
        links.swap(a, b);
    }
}

/// Texture d'un logo image (chargée une fois depuis le disque).
fn logo_texture(ctx: &egui::Context, links: &mut crate::links::Links, file: &str) -> Option<egui::TextureId> {
    links
        .textures
        .entry(file.to_string())
        .or_insert_with(|| {
            let bytes = std::fs::read(crate::links::images_dir()?.join(file)).ok()?;
            let img = image::load_from_memory(&bytes).ok()?.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            Some(ctx.load_texture(format!("wt_link_{file}"), ci, egui::TextureOptions::LINEAR))
        })
        .as_ref()
        .map(|t| t.id())
}

/// Fenêtre de modification d'une case : nom, adresse, logo (émoji ou image 256 × 256).
fn links_editor(ctx: &egui::Context, links: &mut crate::links::Links) {
    use crate::tabs::messages::emoji;
    if links.edit.is_none() {
        return;
    }
    let mut save = false;
    let mut close = false;
    let mut preview_tex = None;
    if let Some(f) = links.edit.as_ref().and_then(|e| e.tile.image.clone()) {
        preview_tex = logo_texture(ctx, links, &f);
    }
    let Some(e) = links.edit.as_mut() else { return };
    let resp = egui::Modal::new(egui::Id::unique("wt_link_editor")).show(ctx, |ui| {
        ui.set_width(420.0);
        ui.label(RichText::new(tf!("Case {}", "Slot {}", e.index + 1)).small().color(theme::TEXT_MUTED()));
        ui.label(RichText::new(t!("Lien", "Link")).font(theme::title_font(18.0)));
        ui.add_space(8.0);
        widgets::field(ui, t!("Nom", "Name"), 70.0, egui::TextEdit::singleline(&mut e.tile.name).hint_text(t!("Ex. Notion", "E.g. Notion")));
        let url = widgets::field(ui, t!("Adresse", "Address"), 70.0, egui::TextEdit::singleline(&mut e.tile.url).hint_text("https://…"));
        if e.tile.url.is_empty() && !url.has_focus() && e.error.is_none() {
            url.request_focus();
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_sized([70.0, 24.0], egui::Label::new(RichText::new("Logo").color(theme::TEXT_MUTED())));
            widgets::segmented(ui, "link_logo_kind", &[(false, t!("Émoji", "Emoji")), (true, "Image 256 × 256")], &mut e.use_image);
        });
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.add_space(78.0);
            // Aperçu.
            let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
            ui.painter().rect_filled(r, CornerRadius::same(14), theme::SURFACE_HI());
            if e.use_image {
                if let Some(t) = preview_tex {
                    ui.painter().add(
                        egui::epaint::RectShape::filled(r.shrink(8.0), CornerRadius::same(8), Color32::WHITE).with_texture(t, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))),
                    );
                }
            } else {
                let em = if e.tile.emoji.is_empty() { "🔗" } else { e.tile.emoji.as_str() };
                crate::emoji_img::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(40.0, 40.0)), em, theme::TEXT());
            }
            ui.vertical(|ui| {
                if e.use_image {
                    if ui.button(t!("Choisir une image…", "Choose an image…")).clicked()
                        && let Some(p) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_file()
                    {
                        match crate::links::import_image(&p) {
                            Ok(name) => {
                                e.tile.image = Some(name);
                                e.error = None;
                            }
                            Err(err) => e.error = Some(err),
                        }
                    }
                    ui.label(
                        RichText::new(t!("Recadrée en carré et redimensionnée en 256 × 256 px.", "Cropped to a square and resized to 256 × 256 px."))
                            .small()
                            .color(theme::TEXT_FAINT()),
                    );
                } else {
                    ui.horizontal(|ui| {
                        let r = ui.add(egui::TextEdit::singleline(&mut e.tile.emoji).desired_width(70.0).hint_text("🙂"));
                        if r.changed() {
                            if let Some(t) = emoji::replace_codes(&e.tile.emoji) {
                                e.tile.emoji = t;
                            }
                            // Un seul émoji : le dernier saisi.
                            if let Some((_, _, last)) = crate::emoji_img::find(&e.tile.emoji).pop() {
                                e.tile.emoji = last;
                            }
                        }
                        if ui.button(t!("Choisir…", "Choose…")).clicked() {
                            e.picker = !e.picker;
                        }
                    });
                    ui.label(RichText::new(t!("Colle un émoji, tape « :nom: » ou choisis-le.", "Paste an emoji, type “:name:” or pick it.")).small().color(theme::TEXT_FAINT()));
                }
            });
        });
        if !e.use_image && e.picker {
            ui.add_space(6.0);
            let recent = emoji::recent();
            if let Some(p) = emoji::picker(ui, &mut e.emoji_query, &mut e.emoji_group, &recent) {
                emoji::remember(&p);
                e.tile.emoji = p;
                e.picker = false;
            }
        }
        if let Some(err) = &e.error {
            ui.add_space(4.0);
            ui.label(RichText::new(err).small().color(theme::TODAY()));
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            if ui.button(t!("Enregistrer", "Save")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Enter) && !e.picker) {
                save = true;
            }
            if ui.button(t!("Annuler", "Cancel")).clicked() {
                close = true;
            }
        });
    });
    if resp.should_close() {
        close = true;
    }
    if save {
        if let Err(err) = links.commit()
            && let Some(e) = links.edit.as_mut()
        {
            e.error = Some(err);
        }
    } else if close {
        links.edit = None;
    }
}
