//! Sélecteur de GIF façon Discord (bouton « GIF » du champ de saisie) : tendances, recherche,
//! favoris (étoile au survol) ; un clic envoie le GIF aussitôt, en message à part.
//!
//! Les GIF viennent de GIPHY (comme Discord depuis l'arrêt de l'API Tenor, le 30 juin 2026) :
//! il faut une clé d'API gratuite (developers.giphy.com), enregistrée dans les réglages. Google
//! Chat ne permet pas d'insérer un GIF de son propre sélecteur par l'API : le GIF est téléchargé
//! puis envoyé en pièce jointe (animé dans Chat comme dans WorkTable).

use std::time::{Duration, Instant};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, vec2};
use serde_json::Value;

use super::ChatState;
use crate::google::chat::ChatAttachment;
use crate::prefs::GifItem;
use crate::theme;

const API: &str = "https://api.giphy.com/v1/gifs";
/// Résultats par page.
const LIMIT: usize = 24;
/// Délai après la frappe avant de chercher.
const DEBOUNCE: Duration = Duration::from_millis(350);
const W: f32 = 420.0;
const H: f32 = 460.0;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Trending,
    Favorites,
}

/// Sélecteur ouvert : brouillon concerné, position du bouton, recherche et résultats.
pub struct Picker {
    pub draft: String,
    pub anchor: Pos2,
    pub tab: Tab,
    pub query: String,
    /// Recherche affichée (ou en cours), et moment de la dernière frappe.
    shown: Option<String>,
    typed: Option<Instant>,
    pub loading: bool,
    pub results: Vec<GifItem>,
    pub error: Option<String>,
    key_input: String,
}

impl Picker {
    pub fn new(draft: String, anchor: Pos2) -> Self {
        Self { draft, anchor, tab: Tab::Trending, query: String::new(), shown: None, typed: None, loading: false, results: Vec::new(), error: None, key_input: String::new() }
    }

    /// Clé changée : tout est redemandé.
    pub fn reset(&mut self) {
        self.shown = None;
        self.results.clear();
        self.error = None;
        self.loading = false;
        self.key_input.clear();
    }

    /// Résultats reçus (ignorés s'ils ne correspondent plus à la recherche).
    pub fn on_results(&mut self, query: &str, result: Result<Vec<GifItem>, String>) {
        if self.shown.as_deref() != Some(query) {
            return;
        }
        self.loading = false;
        match result {
            Ok(r) => {
                self.results = r;
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
    }
}

// --- API GIPHY ------------------------------------------------------------------------------

/// Tendances (`query` vide) ou recherche.
pub fn fetch(agent: &ureq::Agent, key: &str, query: &str) -> Result<Vec<GifItem>, String> {
    let lang = if crate::i18n::en() { "en" } else { "fr" };
    let limit = LIMIT.to_string();
    let mut params = vec![("api_key", key), ("limit", limit.as_str()), ("rating", "pg-13"), ("bundle", "messaging_non_clips")];
    let url = if query.trim().is_empty() {
        format!("{API}/trending")
    } else {
        params.push(("q", query.trim()));
        params.push(("lang", lang));
        format!("{API}/search")
    };
    let mut req = agent.get(&url);
    for (k, v) in params {
        req = req.query(k, v);
    }
    let v: Value = match req.call() {
        Ok(r) => r.into_json().map_err(|e| e.to_string())?,
        Err(ureq::Error::Status(401 | 403, _)) => return Err(t!("Clé GIPHY refusée : vérifie-la dans le sélecteur.", "GIPHY key rejected: check it in the picker.").into()),
        Err(ureq::Error::Status(429, _)) => {
            return Err(t!("Trop de recherches GIPHY pour l'instant : réessaie dans un moment.", "Too many GIPHY searches for now: try again later.").into());
        }
        Err(e) => return Err(e.to_string()),
    };
    Ok(parse(&v))
}

/// Liste de GIF d'une réponse GIPHY.
fn parse(v: &Value) -> Vec<GifItem> {
    let Some(data) = v["data"].as_array() else { return Vec::new() };
    data.iter()
        .filter_map(|g| {
            let im = &g["images"];
            let preview = im["fixed_width_small"]["url"].as_str().or(im["fixed_width_downsampled"]["url"].as_str()).or(im["fixed_width"]["url"].as_str())?;
            // Version envoyée : « downsized » (≤ 2 Mo) si elle existe, sinon l'originale.
            let url = im["downsized"]["url"].as_str().filter(|u| !u.is_empty()).or(im["original"]["url"].as_str())?;
            let dim = |k: &str| im["fixed_width_small"][k].as_str().and_then(|s| s.parse().ok()).unwrap_or(100);
            Some(GifItem {
                id: g["id"].as_str().unwrap_or_default().to_string(),
                title: g["title"].as_str().unwrap_or_default().to_string(),
                preview: preview.to_string(),
                url: url.to_string(),
                width: dim("width"),
                height: dim("height"),
            })
        })
        .collect()
}

/// Pièce jointe « GIF » d'un aperçu (chargée et animée par [`super::gif`]).
pub fn preview_attachment(g: &GifItem) -> ChatAttachment {
    ChatAttachment { name: "GIF".into(), mime: "image/gif".into(), download_uri: Some(g.preview.clone()), ..Default::default() }
}

// --- Affichage ------------------------------------------------------------------------------

/// Ce que demande le sélecteur à cette image.
pub enum Want {
    /// Chercher (ou tendances si vide).
    Search(String),
    Send(GifItem),
    ToggleFavorite(GifItem),
    SaveKey(String),
    /// Aperçu à télécharger.
    Load(ChatAttachment),
    Close,
}

/// Dessine le sélecteur ouvert ; renvoie les demandes (envoi, recherche, favoris, clé…).
pub fn show(ui: &egui::Ui, chat: &mut ChatState) -> Vec<Want> {
    let ctx = ui.ctx().clone();
    let mut wants = Vec::new();
    let has_key = !chat.giphy_key.trim().is_empty();
    let favorites = chat.gif_favorites.clone();
    // Aperçus animés : chargés à la demande, textures préparées avant le dessin.
    let mut frames = GridFrames::new();
    if let Some(p) = chat.gif_picker.as_ref() {
        let list: Vec<GifItem> = if p.tab == Tab::Trending { p.results.clone() } else { favorites.clone() };
        for g in &list {
            let a = preview_attachment(g);
            let key = a.key();
            if !chat.gifs.is_known(&key) {
                wants.push(Want::Load(a));
                continue;
            }
            chat.gifs.prepare(&ctx, &key);
            if let Some(f) = chat.gifs.frame(&ctx, &key) {
                frames.insert(g.preview.clone(), f);
            }
        }
    }
    let Some(p) = chat.gif_picker.as_mut() else { return wants };
    // Recherche différée pendant la frappe ; tendances au premier affichage.
    if has_key && p.tab == Tab::Trending {
        let due = p.typed.is_none_or(|t| t.elapsed() >= DEBOUNCE);
        if due && p.shown.as_deref() != Some(p.query.trim()) {
            p.shown = Some(p.query.trim().to_string());
            p.loading = true;
            p.typed = None;
            wants.push(Want::Search(p.query.trim().to_string()));
        } else if !due {
            ctx.request_repaint_after(DEBOUNCE);
        }
    }
    let pos = Pos2::new(p.anchor.x - W + 40.0, p.anchor.y - H - 8.0);
    let area = egui::Area::new(egui::Id::unique("wt_gif_picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(&ctx, |ui| {
        egui::Frame::new()
            .fill(theme::SURFACE())
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(12))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
            .inner_margin(egui::Margin::same(10))
            .show(ui, |ui| {
                ui.set_width(W - 20.0);
                ui.set_height(H - 20.0);
                if !has_key {
                    key_form(ui, p, &mut wants);
                    return;
                }
                // Onglets et recherche.
                ui.horizontal(|ui| {
                    for (t, label) in [(Tab::Trending, t!("GIF", "GIFs")), (Tab::Favorites, t!("★ Favoris", "★ Favorites"))] {
                        if ui.add(egui::Button::new(RichText::new(label).strong()).selected(p.tab == t).corner_radius(CornerRadius::same(255))).clicked() {
                            p.tab = t;
                        }
                    }
                });
                ui.add_space(6.0);
                let list: Vec<GifItem> = if p.tab == Tab::Trending {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut p.query).hint_text(t!("🔍  Rechercher sur GIPHY", "🔍  Search GIPHY")).desired_width(f32::INFINITY).margin(vec2(10.0, 6.0)),
                    );
                    if r.changed() {
                        p.typed = Some(Instant::now());
                    }
                    if !r.has_focus() && p.query.is_empty() && p.results.is_empty() {
                        r.request_focus();
                    }
                    ui.add_space(4.0);
                    let title =
                        if p.query.trim().is_empty() { t!("Tendances", "Trending").to_string() } else { tf!("Résultats pour « {} »", "Results for “{}”", p.query.trim()) };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(title).small().color(theme::TEXT_MUTED()));
                        if p.loading {
                            ui.add(egui::Spinner::new().size(12.0));
                        }
                    });
                    if let Some(e) = &p.error {
                        ui.label(RichText::new(e).small().color(theme::TODAY()));
                    }
                    p.results.clone()
                } else {
                    if favorites.is_empty() {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(t!("Aucun favori pour l'instant.", "No favorite yet.")).color(theme::TEXT_MUTED()));
                            ui.label(
                                RichText::new(t!("Survole un GIF puis clique ★ pour le garder ici.", "Hover a GIF, then click ★ to keep it here."))
                                    .small()
                                    .color(theme::TEXT_FAINT()),
                            );
                        });
                    }
                    favorites.clone()
                };
                ui.add_space(4.0);
                let footer = 18.0;
                egui::ScrollArea::vertical().id_salt(("gif_grid", p.tab == Tab::Favorites)).max_height(ui.available_height() - footer).auto_shrink([false, false]).show(ui, |ui| {
                    grid(ui, &frames, &list, &favorites, &mut wants);
                });
                // Mention demandée par GIPHY ; changer de clé.
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new(t!("Clé GIPHY…", "GIPHY key…")).small().color(theme::TEXT_FAINT())).frame(false)).clicked() {
                        wants.push(Want::SaveKey(String::new()));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new("Powered by GIPHY").small().strong().color(theme::TEXT_FAINT()));
                    });
                });
            });
    });
    // Clic ailleurs (hors du bouton) ou Échap : fermé.
    let outside = ctx.input(|i| {
        i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|q| !area.response.rect.contains(q) && !Rect::from_min_size(p.anchor, vec2(40.0, 30.0)).contains(q))
    });
    if outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        wants.push(Want::Close);
    }
    wants
}

/// Images à afficher dans la grille à cette image : aperçu → (texture, taille).
type GridFrames = std::collections::HashMap<String, (egui::TextureHandle, egui::Vec2)>;

/// Grille sur deux colonnes (hauteurs variables, comme Discord).
fn grid(ui: &mut egui::Ui, frames: &GridFrames, list: &[GifItem], favorites: &[GifItem], wants: &mut Vec<Want>) {
    let gap = 6.0;
    let col_w = ((ui.available_width() - gap) / 2.0).floor();
    let top = ui.cursor().top();
    let left = ui.cursor().left();
    let mut heights = [0.0f32; 2];
    for g in list {
        let c = if heights[0] <= heights[1] { 0 } else { 1 };
        let h = (col_w * g.height.max(1) as f32 / g.width.max(1) as f32).clamp(50.0, 260.0);
        let rect = Rect::from_min_size(Pos2::new(left + c as f32 * (col_w + gap), top + heights[c]), vec2(col_w, h));
        heights[c] += h + gap;
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let resp = ui.interact(rect, ui.scope_id().with(("gif", &g.id)), Sense::click());
        let p = ui.painter();
        match frames.get(&g.preview) {
            Some((tex, _)) => {
                p.image(tex.id(), rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                p.rect_filled(rect, CornerRadius::same(6), theme::BG());
            }
        }
        let fav = favorites.iter().any(|f| f.id == g.id);
        if resp.hovered() {
            p.rect_stroke(rect, CornerRadius::same(6), Stroke::new(2.0, theme::TODAY()), egui::StrokeKind::Inside);
            // Étoile : ajouter / retirer des favoris.
            let star = Rect::from_min_size(rect.right_top() + vec2(-28.0, 4.0), vec2(24.0, 24.0));
            let sr = ui.interact(star, ui.scope_id().with(("gif_star", &g.id)), Sense::click());
            p.circle_filled(star.center(), 12.0, Color32::from_black_alpha(160));
            p.text(
                star.center(),
                Align2::CENTER_CENTER,
                if fav { "★" } else { "☆" },
                FontId::proportional(15.0),
                if fav { Color32::from_rgb(0xF5, 0xC5, 0x42) } else { Color32::WHITE },
            );
            if sr.on_hover_text(if fav { t!("Retirer des favoris", "Remove from favorites") } else { t!("Ajouter aux favoris", "Add to favorites") }).clicked() {
                wants.push(Want::ToggleFavorite(g.clone()));
                continue;
            }
        } else if fav {
            p.text(rect.right_top() + vec2(-14.0, 14.0), Align2::CENTER_CENTER, "★", FontId::proportional(13.0), Color32::from_rgb(0xF5, 0xC5, 0x42));
        }
        if resp.on_hover_text(if g.title.is_empty() { t!("Envoyer", "Send").to_string() } else { g.title.clone() }).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
            wants.push(Want::Send(g.clone()));
        }
    }
    ui.allocate_space(vec2(ui.available_width(), heights[0].max(heights[1])));
}

/// Pas encore de clé : explications et champ pour la coller.
fn key_form(ui: &mut egui::Ui, p: &mut Picker, wants: &mut Vec<Want>) {
    ui.label(RichText::new(t!("GIF (GIPHY)", "GIFs (GIPHY)")).strong());
    ui.add_space(6.0);
    ui.label(
        RichText::new(t!(
            "Comme Discord, WorkTable cherche les GIF sur GIPHY (l'API Tenor a fermé le 30 juin 2026). Il faut une clé d'API gratuite :",
            "Like Discord, WorkTable looks GIFs up on GIPHY (the Tenor API closed on 30 June 2026). A free API key is needed:"
        ))
        .small()
        .color(theme::TEXT_MUTED()),
    );
    ui.add_space(4.0);
    for step in [
        t!("1. Crée un compte sur developers.giphy.com", "1. Create an account on developers.giphy.com"),
        t!("2. « Create an App » › « API » (gratuit)", "2. “Create an App” › “API” (free)"),
        t!("3. Copie la clé et colle-la ci-dessous", "3. Copy the key and paste it below"),
    ] {
        ui.label(RichText::new(step).small().color(theme::TEXT_MUTED()));
    }
    if ui.link(t!("Ouvrir developers.giphy.com ↗", "Open developers.giphy.com ↗")).clicked() {
        let _ = webbrowser::open("https://developers.giphy.com/dashboard/");
    }
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut p.key_input).hint_text(t!("Clé d'API GIPHY", "GIPHY API key")).desired_width(260.0));
        if ui.add_enabled(!p.key_input.trim().is_empty(), egui::Button::new(t!("Enregistrer", "Save"))).clicked() {
            wants.push(Want::SaveKey(p.key_input.trim().to_string()));
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new(t!(
            "La clé est gardée dans les réglages de WorkTable ; « Clé GIPHY… » en bas du sélecteur pour la changer.",
            "The key is kept in WorkTable's settings; “GIPHY key…” at the bottom of the picker to change it."
        ))
        .small()
        .color(theme::TEXT_FAINT()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_giphy_response() {
        let v = serde_json::json!({ "data": [
            { "id": "abc", "title": "Cat GIF", "images": {
                "fixed_width_small": { "url": "https://media.giphy.com/media/abc/100w.gif", "width": "100", "height": "75" },
                "downsized": { "url": "https://media.giphy.com/media/abc/giphy-downsized.gif" },
                "original": { "url": "https://media.giphy.com/media/abc/giphy.gif" } } },
            { "id": "nopreview", "images": {} }
        ]});
        let r = parse(&v);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].width, r[0].height), (100, 75));
        assert!(r[0].url.ends_with("giphy-downsized.gif") && r[0].preview.ends_with("100w.gif"));
    }
}
