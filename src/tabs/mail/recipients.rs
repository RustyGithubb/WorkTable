//! Champs « À » / « Cc » avec suggestions de l'annuaire pendant la frappe.
//! ↑/↓ pour choisir, Tab ou Entrée pour ajouter, Échap pour masquer, ou clic.

use std::collections::HashSet;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Color32, CornerRadius, Key, Modifiers, Rect, RichText, Sense, Stroke, vec2};

use super::contacts::{self, Contact, Contacts};
use super::directory::star_shape;
use crate::theme;

#[derive(Default)]
struct FieldState {
    index: usize,
    token: String,
    dismissed: Option<String>,
}

#[derive(Default)]
pub struct Suggest {
    /// État propre à chaque champ (À, Cc).
    fields: std::collections::HashMap<egui::Id, FieldState>,
    popup: Option<(egui::Id, Rect)>,
    refocus: Option<egui::Id>,
}

/// Sépare le texte en (destinataires déjà saisis, adresse en cours de frappe).
pub fn split_token(text: &str) -> (String, String) {
    let mut quoted = false;
    let mut sep = None;
    for (i, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' | ';' if !quoted => sep = Some(i),
            _ => {}
        }
    }
    match sep {
        Some(i) => {
            let head = text[..=i].trim_end();
            (format!("{head} "), text[i + 1..].trim_start().to_string())
        }
        None => (String::new(), text.trim_start().to_string()),
    }
}

/// Remplace l'adresse en cours de frappe par le contact choisi.
pub fn accept(text: &str, c: &Contact) -> String {
    let (prefix, _) = split_token(text);
    format!("{prefix}{}, ", c.mailbox())
}

fn move_cursor_to_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut st = TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(text.chars().count()))));
    st.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

pub fn field(ui: &mut egui::Ui, label: &str, text: &mut String, salt: &str, hint: &str, book: &Contacts, st: &mut Suggest) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique(("wt_recipients", salt));
    if st.refocus == Some(id) {
        move_cursor_to_end(&ctx, id, text);
        st.refocus = None;
    }

    let focused = ctx.memory(|m| m.has_focus(id));
    let over_popup = st.popup.filter(|(pid, _)| *pid == id).is_some_and(|(_, r)| ctx.pointer_hover_pos().is_some_and(|p| r.contains(p)));
    let (_, token) = split_token(text);
    let refocus = &mut st.refocus;
    let shared_popup = &mut st.popup;
    let st = st.fields.entry(id).or_default();
    if token != st.token {
        st.token = token.clone();
        st.index = 0;
        st.dismissed = None;
    }

    let already: HashSet<String> = contacts::split_list(text).iter().filter_map(|m| contacts::parse_mailbox(m).map(|(_, e)| e)).collect();
    let active = (focused || over_popup) && !token.trim().is_empty() && st.dismissed.as_deref() != Some(token.as_str());
    let suggestions: Vec<Contact> = if active { book.search(&token, 12).into_iter().filter(|c| !already.contains(&c.email)).take(6).cloned().collect() } else { Vec::new() };
    if !suggestions.is_empty() {
        st.index = st.index.min(suggestions.len() - 1);
    }

    let mut chosen: Option<usize> = None;
    if focused && !suggestions.is_empty() {
        ui.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                st.index = (st.index + 1) % suggestions.len();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                st.index = (st.index + suggestions.len() - 1) % suggestions.len();
            }
            if i.consume_key(Modifiers::NONE, Key::Tab) || i.consume_key(Modifiers::NONE, Key::Enter) {
                chosen = Some(st.index);
            }
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                st.dismissed = Some(token.clone());
            }
        });
    }

    let resp = ui
        .horizontal(|ui| {
            ui.add_sized([52.0, 24.0], egui::Label::new(RichText::new(label).color(theme::TEXT_MUTED())));
            ui.add(
                egui::TextEdit::singleline(text)
                    .id(id)
                    .hint_text(hint)
                    // Tab sert à valider la suggestion au lieu de changer de champ.
                    .lock_focus(!suggestions.is_empty())
                    .desired_width(ui.available_width()),
            )
        })
        .inner;

    let show_popup = !suggestions.is_empty() && st.dismissed.as_deref() != Some(token.as_str()) && chosen.is_none();
    if show_popup {
        let area = egui::Area::new(id.with("suggestions")).order(egui::Order::Foreground).fixed_pos(resp.rect.left_bottom() + vec2(0.0, 4.0)).show(&ctx, |ui| {
            egui::Frame::new()
                .fill(theme::SURFACE())
                .stroke(Stroke::new(1.0, crate::theme::gray(0x3A)))
                .corner_radius(CornerRadius::same(10))
                .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(150) })
                .inner_margin(egui::Margin::same(4))
                .show(ui, |ui| {
                    ui.set_width(resp.rect.width().max(260.0) - 8.0);
                    ui.spacing_mut().item_spacing.y = 0.0;
                    for (i, c) in suggestions.iter().enumerate() {
                        if suggestion_row(ui, c, i == st.index).clicked() {
                            chosen = Some(i);
                        }
                    }
                    ui.add_space(4.0);
                    ui.label(RichText::new("  Tab ou Entrée pour ajouter · ↑ ↓ pour choisir · Échap").small().color(theme::TEXT_FAINT()));
                });
        });
        *shared_popup = Some((id, area.response.rect));
    } else if shared_popup.is_some_and(|(pid, _)| pid == id) {
        *shared_popup = None;
    }

    if let Some(i) = chosen {
        *text = accept(text, &suggestions[i]);
        st.token.clear();
        st.index = 0;
        *shared_popup = None;
        move_cursor_to_end(&ctx, id, text);
        *refocus = Some(id);
    }
}

fn suggestion_row(ui: &mut egui::Ui, c: &Contact, highlighted: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if highlighted || resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(7), crate::theme::gray(if highlighted { 0x2E } else { 0x24 }));
    }
    let x = rect.left() + if c.favorite { 30.0 } else { 12.0 };
    if c.favorite {
        star_shape(p, rect.left_center() + vec2(15.0, 0.0), 6.0, true, theme::TEXT());
    }
    let name_font = theme::editor_font(13.5, c.favorite);
    p.text(rect.left_top() + vec2(x - rect.left(), 5.0), egui::Align2::LEFT_TOP, c.display_name(), name_font, theme::TEXT());
    if !c.name.is_empty() {
        p.text(rect.left_top() + vec2(x - rect.left(), 22.0), egui::Align2::LEFT_TOP, &c.email, egui::FontId::proportional(11.5), theme::TEXT_MUTED());
    }
    resp
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_and_accept() {
        assert_eq!(split_token("lea"), (String::new(), "lea".into()));
        assert_eq!(split_token("a@b.fr, hu"), ("a@b.fr, ".into(), "hu".into()));
        assert_eq!(split_token("\"Dupont, Jean\" <j@d.fr>, "), ("\"Dupont, Jean\" <j@d.fr>, ".into(), String::new()));
        let c = Contact { email: "hugo@gmail.com".into(), name: "Hugo Bernard".into(), favorite: false, last_seen: 0, count: 0 };
        assert_eq!(accept("a@b.fr, hu", &c), "a@b.fr, Hugo Bernard <hugo@gmail.com>, ");
        assert_eq!(accept("hu", &c), "Hugo Bernard <hugo@gmail.com>, ");
    }
}
