//! Visionneuse d'images commune à tous les onglets (Chat, Mail, Notes, Drive) : l'image en grand
//! par-dessus l'app. Molette ou +/− pour zoomer, glisser pour déplacer, double-clic pour 100 % / 200 %,
//! 0 pour réinitialiser, Échap ou clic à côté pour fermer.

use std::path::PathBuf;
use std::sync::Mutex;

use eframe::egui::{self, Color32, CornerRadius, Key, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, vec2};

use crate::theme;

/// Action du bouton « Ouvrir ».
pub enum OpenWith {
    None,
    /// Fichier local, ouvert avec l'application par défaut.
    Path(PathBuf),
    /// Page web (Drive…).
    Url(String),
    /// Géré par l'onglet d'origine, qui le récupère avec [`take_tag`].
    Tag(String),
}

struct Viewer {
    key: String,
    tex: TextureHandle,
    name: String,
    open: OpenWith,
    zoom: f32,
    pan: egui::Vec2,
}

static VIEWER: Mutex<Option<Viewer>> = Mutex::new(None);
static TAG: Mutex<Option<String>> = Mutex::new(None);

/// Affiche une image en grand. `key` identifie l'image (pour la remplacer par une version plus nette).
pub fn open(ctx: &egui::Context, key: impl Into<String>, tex: TextureHandle, name: impl Into<String>, open: OpenWith) {
    // Le champ de texte actif ne doit pas recevoir les touches de la visionneuse (+, −, 0…).
    ctx.memory_mut(|m| m.stop_text_input());
    *VIEWER.lock().unwrap() = Some(Viewer { key: key.into(), tex, name: name.into(), open, zoom: 1.0, pan: egui::Vec2::ZERO });
    ctx.request_repaint();
}

/// Image affichée (sa clé), s'il y en a une.
pub fn current() -> Option<String> {
    VIEWER.lock().unwrap().as_ref().map(|v| v.key.clone())
}

/// Remplace l'image affichée par une version plus grande, si c'est toujours la même.
pub fn upgrade(key: &str, tex: TextureHandle) {
    if let Some(v) = VIEWER.lock().unwrap().as_mut().filter(|v| v.key == key && v.tex.id() != tex.id()) {
        v.tex = tex;
    }
}

/// « Ouvrir » demandé pour une image dont l'action est `OpenWith::Tag` commençant par `prefix`.
pub fn take_tag(prefix: &str) -> Option<String> {
    let mut t = TAG.lock().unwrap();
    if t.as_deref().is_some_and(|s| s.starts_with(prefix)) { t.take() } else { None }
}

/// À appeler une fois par image, avant les onglets (la visionneuse prend Échap pour elle).
pub fn show(ctx: &egui::Context) {
    let mut guard = VIEWER.lock().unwrap();
    let Some(v) = guard.as_mut() else { return };
    let screen = ctx.content_rect();
    let size = v.tex.size_vec2();
    let mut close = ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, Key::Escape));
    let mut open = false;
    egui::Area::new(egui::Id::unique("wt_lightbox")).order(egui::Order::Tooltip).fixed_pos(screen.min).show(ctx, |ui| {
        let (bg, bg_resp) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(bg, CornerRadius::ZERO, Color32::from_black_alpha(215));
        // Taille d'origine réduite pour tenir dans la fenêtre, puis zoom.
        let avail = screen.size() - vec2(80.0, 140.0);
        let fit = (avail.x / size.x).min(avail.y / size.y).min(1.0);
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            v.zoom = (v.zoom * (1.0 + scroll * 0.002)).clamp(0.2, 8.0);
        }
        if ui.input(|i| i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)) {
            v.zoom = (v.zoom * 1.25).min(8.0);
        }
        if ui.input(|i| i.key_pressed(Key::Minus)) {
            v.zoom = (v.zoom / 1.25).max(0.2);
        }
        if ui.input(|i| i.key_pressed(Key::Num0)) {
            v.zoom = 1.0;
            v.pan = egui::Vec2::ZERO;
        }
        let shown = size * fit * v.zoom;
        let img = Rect::from_center_size(screen.center() - vec2(0.0, 20.0) + v.pan, shown);
        let img_resp = ui.interact(img, egui::Id::unique("wt_lightbox_img"), Sense::click_and_drag());
        if img_resp.dragged() {
            v.pan += img_resp.drag_delta();
        }
        if img_resp.double_clicked() {
            v.zoom = if v.zoom > 1.01 { 1.0 } else { 2.0 };
            v.pan = egui::Vec2::ZERO;
        }
        if img_resp.hovered() {
            ui.ctx().set_cursor_icon(if img_resp.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        ui.painter().image(v.tex.id(), img, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        // Barre du bas : nom, zoom, actions.
        let bar = Rect::from_center_size(Pos2::new(screen.center().x, screen.bottom() - 44.0), vec2(560.0_f32.min(screen.width() - 40.0), 44.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            egui::Frame::new()
                .fill(theme::SURFACE())
                .stroke(Stroke::new(1.0, theme::BORDER()))
                .corner_radius(CornerRadius::same(255))
                .inner_margin(egui::Margin::symmetric(14, 6))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.add(egui::Label::new(RichText::new(&v.name).color(theme::TEXT())).truncate());
                        ui.label(RichText::new(format!("{:.0} %", fit * v.zoom * 100.0)).small().color(theme::TEXT_MUTED()));
                        if ui.small_button("−").on_hover_text(t!("Dézoomer (−)", "Zoom out (−)")).clicked() {
                            v.zoom = (v.zoom / 1.25).max(0.2);
                        }
                        if ui.small_button("+").on_hover_text(t!("Zoomer (+ ou molette)", "Zoom in (+ or mouse wheel)")).clicked() {
                            v.zoom = (v.zoom * 1.25).min(8.0);
                        }
                        let label = match v.open {
                            OpenWith::None => None,
                            OpenWith::Url(_) => Some((t!("Ouvrir", "Open"), t!("Ouvrir dans le navigateur", "Open in the browser"))),
                            _ => Some((t!("Ouvrir", "Open"), t!("Ouvrir avec l'application par défaut", "Open with the default app"))),
                        };
                        if let Some((l, tip)) = label
                            && ui.small_button(l).on_hover_text(tip).clicked()
                        {
                            open = true;
                        }
                        if ui.small_button("✕").on_hover_text(t!("Fermer (Échap)", "Close (Esc)")).clicked() {
                            close = true;
                        }
                    });
                });
        });
        if bg_resp.clicked() && !img_resp.hovered() {
            close = true;
        }
    });
    if open {
        match &v.open {
            OpenWith::Path(p) => {
                let _ = open::that_detached(p);
            }
            OpenWith::Url(u) => {
                let _ = webbrowser::open(u);
            }
            OpenWith::Tag(t) => *TAG.lock().unwrap() = Some(t.clone()),
            OpenWith::None => {}
        }
    }
    if close {
        *guard = None;
    }
}
