//! Vue « Annuaire » : liste des contacts (favoris mis en avant) et fiche du contact.

use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Painter, Pos2, Rect, RichText, Sense, Stroke, vec2};

use super::MailState;
use crate::liquid::Liquid;
use super::contacts::Contact;
use crate::{tabs, theme, widgets};

pub enum DirAction {
    Select(String),
    Favorite(String),
    WriteTo(String),
    ChatTo(String),
    ShowMails(String),
    Remove(String),
    /// Ouvre un mail du contact (adresse, identifiant du mail).
    OpenMail(String, String),
}

const ROW_H: f32 = 50.0;

/// Colonne du milieu : recherche + liste.
pub fn list(ui: &mut egui::Ui, mail: &mut MailState, out: &mut Vec<DirAction>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Annuaire").strong());
        ui.label(RichText::new(format!("{} contacts", mail.contacts.len())).small().color(theme::TEXT_FAINT()));
    });
    ui.add_space(8.0);
    ui.add(
        egui::TextEdit::singleline(&mut mail.dir_query)
            .hint_text("Rechercher un nom ou une adresse")
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);

    let found: Vec<Contact> = mail.contacts.search(&mail.dir_query, usize::MAX).into_iter().cloned().collect();
    if found.is_empty() {
        let hint = if mail.contacts.len() == 0 {
            "Les adresses des mails reçus s'ajoutent ici automatiquement"
        } else {
            "Aucun contact ne correspond"
        };
        widgets::empty_state(ui, "Aucun contact", hint);
        return;
    }

    let favorites = found.iter().filter(|c| c.favorite).count();
    egui::ScrollArea::vertical()
        .id_salt("dir_list")
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, found.len(), |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let mut hl = Liquid::begin(ui, "dir_list");
            for i in range {
                let c = &found[i];
                let selected = mail.dir_selected.as_deref() == Some(c.email.as_str());
                // Séparation visuelle entre les favoris et les autres contacts.
                let first_regular = favorites > 0 && i == favorites;
                row(ui, c, selected, first_regular, &mut hl, out);
            }
            hl.end(ui, crate::theme::gray(0x30), 8.0);
        });
}

fn row(ui: &mut egui::Ui, c: &Contact, selected: bool, separator: bool, hl: &mut Liquid, out: &mut Vec<DirAction>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let star_rect = Rect::from_center_size(rect.right_center() - vec2(18.0, 0.0), vec2(24.0, 24.0));
    let star = ui
        .interact(star_rect, resp.id.with("fav"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if c.favorite { "Retirer des favoris" } else { "Mettre en favori" });
    if star.clicked() {
        out.push(DirAction::Favorite(c.email.clone()));
    } else if resp.clicked() {
        out.push(DirAction::Select(c.email.clone()));
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

    let p = ui.painter();
    let inner = rect.shrink2(vec2(0.0, 2.0));
    if separator {
        p.hline(rect.x_range(), rect.top(), Stroke::new(1.0, theme::BORDER()));
    }
    // Sélection : forme liquide. Favoris : fond éclairé translucide + liseré blanc.
    if selected {
        hl.target(inner, &c.email);
    }
    let bg = if c.favorite {
        Some(crate::theme::overlay(14))
    } else if resp.hovered() && !selected {
        Some(crate::theme::overlay(7))
    } else {
        None
    };
    if let Some(bg) = bg {
        p.rect_filled(inner, CornerRadius::same(8), bg);
    }
    if c.favorite {
        let bar = Rect::from_min_size(inner.left_top() + vec2(0.0, 8.0), vec2(3.0, inner.height() - 16.0));
        p.rect_filled(bar, CornerRadius::same(2), theme::TEXT());
    }

    let x = inner.left() + 14.0;
    let w = inner.width() - 60.0;
    let name_color = if c.favorite || selected { theme::TEXT() } else { crate::theme::gray(0xCC) };
    let name_font = if c.favorite { theme::editor_font(14.0, true) } else { FontId::proportional(14.0) };
    let g = one_line(p, c.display_name(), name_font, name_color, w);
    p.galley(Pos2::new(x, inner.top() + 7.0), g, name_color);
    let sub = if c.name.is_empty() { String::new() } else { c.email.clone() };
    let g = one_line(p, &sub, FontId::proportional(12.0), theme::TEXT_MUTED(), w);
    p.galley(Pos2::new(x, inner.top() + 26.0), g, theme::TEXT_MUTED());

    let col = if c.favorite { theme::TEXT() } else if star.hovered() { theme::TEXT_MUTED() } else { theme::TEXT_FAINT() };
    star_shape(p, star_rect.center(), 7.0, c.favorite, col);
}

/// Colonne de droite : fiche du contact sélectionné.
pub fn detail(ui: &mut egui::Ui, mail: &MailState, out: &mut Vec<DirAction>) {
    let Some(c) = mail.dir_selected.as_deref().and_then(|e| mail.contacts.get(e)) else {
        ui.add_space(ui.available_height() * 0.42);
        widgets::empty_state(ui, "Aucun contact sélectionné", "Choisis un contact dans l'annuaire");
        return;
    };

    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        let p = ui.painter();
        p.circle_filled(r.center(), 22.0, if c.favorite { theme::TEXT() } else { theme::SURFACE_HI() });
        let initial: String = c.display_name().chars().next().map(|ch| ch.to_uppercase().collect()).unwrap_or_default();
        p.text(
            r.center(),
            egui::Align2::CENTER_CENTER,
            initial,
            theme::title_font(18.0),
            if c.favorite { theme::BG() } else { theme::TEXT() },
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(c.display_name()).size(20.0).strong());
            ui.add(egui::Label::new(RichText::new(&c.email).color(theme::TEXT_MUTED())).selectable(true));
        });
    });

    ui.add_space(14.0);
    ui.horizontal_wrapped(|ui| {
        if ui.button("Écrire").clicked() {
            out.push(DirAction::WriteTo(c.email.clone()));
        }
        if ui.button("Message Google Chat").on_hover_text("Ouvre (ou crée) la conversation privée dans l'onglet Messages").clicked() {
            out.push(DirAction::ChatTo(c.email.clone()));
        }
        if ui.button("Voir ses mails").clicked() {
            out.push(DirAction::ShowMails(c.email.clone()));
        }
        let fav = if c.favorite { "Retirer des favoris" } else { "Mettre en favori" };
        if ui.button(fav).clicked() {
            out.push(DirAction::Favorite(c.email.clone()));
        }
    });

    ui.add_space(16.0);
    ui.separator();
    ui.add_space(8.0);
    let info = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.add_sized([130.0, 20.0], egui::Label::new(RichText::new(k).color(theme::TEXT_MUTED())));
            ui.label(v);
        });
    };
    info(ui, "Mails échangés", c.count.to_string());
    if c.last_seen > 0 {
        info(ui, "Dernier échange", last_seen(c.last_seen));
    }
    info(ui, "Favori", if c.favorite { "Oui".into() } else { "Non".into() });

    // Mails rattachés à ce contact.
    ui.add_space(14.0);
    ui.label(RichText::new("SES MAILS").small().strong().color(theme::TEXT_MUTED()));
    ui.add_space(6.0);
    let mails = mail.dir_mails.as_ref().filter(|(e, _)| *e == c.email).map(|(_, l)| l);
    let list_h = (ui.available_height() - 34.0).max(80.0);
    egui::ScrollArea::vertical()
        .id_salt("dir_contact_mails")
        .max_height(list_h)
        .auto_shrink([false, true])
        .show(ui, |ui| match mails {
            None if mail.dir_loading.as_deref() == Some(c.email.as_str()) => {
                for r in [0.8, 0.6, 0.7] {
                    widgets::ghost_row(ui, r);
                    widgets::ghost_row(ui, r * 0.5);
                    ui.add_space(8.0);
                }
            }
            None => {
                ui.label(RichText::new("Mails non chargés").small().color(theme::TEXT_FAINT()));
            }
            Some(list) if list.is_empty() => {
                ui.label(RichText::new("Aucun mail reçu de cette adresse").small().color(theme::TEXT_FAINT()));
            }
            Some(list) => {
                ui.spacing_mut().item_spacing.y = 2.0;
                for m in list {
                    if mail_row(ui, m).clicked() {
                        out.push(DirAction::OpenMail(c.email.clone(), m.id.clone()));
                    }
                }
            }
        });

    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        if ui
            .add(egui::Button::new(RichText::new("Retirer de l'annuaire").small().color(theme::TEXT_MUTED())).frame(false))
            .on_hover_text("L'adresse sera de nouveau ajoutée si elle t'écrit")
            .clicked()
        {
            out.push(DirAction::Remove(c.email.clone()));
        }
    });
}

fn mail_row(ui: &mut egui::Ui, m: &crate::google::Summary) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Ouvrir le mail");
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(8));
    }
    let unread = m.unread();
    let date = last_seen(m.date);
    let dg = one_line(p, &date, FontId::proportional(11.0), theme::TEXT_MUTED(), 140.0);
    let dw = dg.size().x;
    p.galley(Pos2::new(rect.right() - 10.0 - dw, rect.top() + 7.0), dg, theme::TEXT_MUTED());
    if unread {
        p.circle_filled(Pos2::new(rect.left() + 6.0, rect.top() + 14.0), 3.0, theme::TEXT());
    }
    let subject = if m.subject.is_empty() { "(sans objet)" } else { &m.subject };
    let font = if unread { theme::editor_font(13.0, true) } else { FontId::proportional(13.0) };
    let g = one_line(p, subject, font, theme::TEXT(), rect.width() - dw - 34.0);
    p.galley(Pos2::new(rect.left() + 14.0, rect.top() + 5.0), g, theme::TEXT());
    let g = one_line(p, &m.snippet, FontId::proportional(11.5), theme::TEXT_FAINT(), rect.width() - 24.0);
    p.galley(Pos2::new(rect.left() + 14.0, rect.top() + 25.0), g, theme::TEXT_FAINT());
    resp
}

fn last_seen(ms: i64) -> String {
    use chrono::{Datelike, Local, TimeZone};
    let Some(d) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    format!("{} {} {}", d.day(), tabs::month_fr(d.month()), d.year())
}

fn one_line(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
    job.wrap = egui::text::TextWrapping {
        max_width: max_w.max(10.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    p.layout_job(job)
}

/// Étoile à 5 branches (pleine ou contour).
pub fn star_shape(p: &Painter, c: Pos2, r: f32, filled: bool, color: Color32) {
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rad = if i % 2 == 0 { r } else { r * 0.45 };
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    if filled {
        let inner: Vec<Pos2> = pts.iter().skip(1).step_by(2).copied().collect();
        p.add(egui::Shape::convex_polygon(inner, color, Stroke::NONE));
        for i in 0..5 {
            let (a, tip, b) = (pts[(i * 2 + 9) % 10], pts[i * 2], pts[i * 2 + 1]);
            p.add(egui::Shape::convex_polygon(vec![a, tip, b], color, Stroke::NONE));
        }
    } else {
        p.add(egui::Shape::closed_line(pts, Stroke::new(1.2, color)));
    }
}
