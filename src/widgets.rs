//! Composants visuels partagés entre les onglets.

use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke};

use crate::theme;

/// En-tête d'onglet, identique partout : sur-titre discret en capitales + grand titre (police
/// « Titres »), suivis d'éléments optionnels sur la même ligne (pastille, numéro de semaine…).
pub fn page_header(ui: &mut egui::Ui, overline: &str, title: &str) {
    page_header_with(ui, overline, title, |_| {});
}

pub fn page_header_with(ui: &mut egui::Ui, overline: &str, title: &str, extra: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(overline.to_uppercase()).small().color(theme::TEXT_MUTED()).strong());
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_TITLE_H);
        ui.label(RichText::new(title).font(theme::title_font(32.0)).color(theme::TEXT()));
        ui.add_space(4.0);
        extra(ui);
    });
    ui.add_space(18.0);
}

/// Information à côté du titre (non lus en terracotta, numéro de semaine en gris).
pub fn header_badge(ui: &mut egui::Ui, text: &str, accent: bool) {
    ui.label(RichText::new(text).size(14.0).color(if accent { theme::TODAY() } else { theme::TEXT_MUTED() }));
}

/// Hauteur de la ligne de titre (alignement identique d'un onglet à l'autre).
const HEADER_TITLE_H: f32 = 40.0;

/// Titre de section (cartes, colonnes) : police « Titres ».
pub fn section_title(text: &str) -> RichText {
    RichText::new(text).font(theme::title_font(15.0)).color(theme::TEXT())
}

/// Carte de contenu avec bordure fine.
pub fn card<R>(
    ui: &mut egui::Ui,
    title: &str,
    min_height: f32,
    body: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    egui::Frame::new()
        .fill(theme::SURFACE())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(min_height);
            if !title.is_empty() {
                ui.label(section_title(title));
                ui.add_space(10.0);
            }
            body(ui)
        })
        .inner
}

/// Panneau plein (utilisé pour les colonnes Mail / Notes).
pub fn pane<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::SURFACE())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            body(ui)
        })
        .inner
}

/// État vide : message centré, en gris.
pub fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(title).color(theme::TEXT_MUTED()));
        ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT()));
    });
}

/// Ligne fantôme (placeholder de liste).
pub fn ghost_row(ui: &mut egui::Ui, width_ratio: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 14.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(w * width_ratio, 10.0));
    ui.painter().rect_filled(bar, CornerRadius::same(5), theme::SURFACE_HI());
}

/// Découpe l'espace restant en colonnes de largeurs fixes (la dernière prend le reste)
/// et affiche un panneau dans chacune, en disposition verticale.
pub fn pane_columns(ui: &mut egui::Ui, fixed: &[f32], mut body: impl FnMut(usize, &mut egui::Ui)) {
    let full = ui.available_rect_before_wrap();
    let gap = ui.spacing().item_spacing.x;
    let mut x = full.left();
    for i in 0..=fixed.len() {
        let w = fixed.get(i).copied().unwrap_or(full.right() - x);
        let rect = egui::Rect::from_min_size(egui::pos2(x, full.top()), egui::vec2(w, full.height()));
        ui.scope_builder(
            egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)),
            |ui| pane(ui, |ui| body(i, ui)),
        );
        x += w + gap;
    }
    ui.allocate_rect(full, egui::Sense::hover());
}

/// Ligne « libellé + champ texte » occupant toute la largeur disponible.
pub fn field(ui: &mut egui::Ui, label: &str, label_w: f32, edit: egui::TextEdit) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([label_w, 24.0], egui::Label::new(RichText::new(label).color(theme::TEXT_MUTED())));
        ui.add(edit.desired_width(ui.available_width()))
    })
    .inner
}

/// Sélecteur segmenté : la pastille de sélection glisse (animation liquide) d'une option à l'autre.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, salt: &str, options: &[(T, &str)], value: &mut T) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(theme::BG())
        .stroke(egui::Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut hl = crate::liquid::Liquid::begin(ui, ("segmented", salt));
                for (i, (opt, label)) in options.iter().enumerate() {
                    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), egui::Color32::PLACEHOLDER);
                    let size = egui::vec2(galley.size().x + 24.0, 26.0);
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if *opt == *value {
                        hl.target(rect, i);
                    }
                    let cov = hl.coverage(rect);
                    let base = if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
                    let color = crate::sidebar::lerp_color(base, theme::BG(), cov);
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
                    if resp.clicked() && *opt != *value {
                        *value = *opt;
                        changed = true;
                    }
                }
                hl.end(ui, theme::TEXT(), 13.0);
            });
        });
    changed
}
