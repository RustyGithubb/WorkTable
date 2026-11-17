//! Image du compte (pastille « Compte » en haut de la barre latérale) : photo Google, initiale
//! ou image importée. Choix dans `config\avatar.json`, image importée dans `config\avatar.png`
//! (256 × 256) : tous deux synchronisés avec le compte.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, TextureHandle, vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use crate::google::Account;
use crate::tabs::messages::ChatState;

/// Origine de l'image du compte.
#[derive(Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// Photo du compte Google (sinon l'initiale).
    #[default]
    Google,
    Initial,
    /// Image importée (`config\avatar.png`).
    Image,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    source: Source,
}

#[derive(Default)]
pub struct Avatar {
    pub source: Source,
    /// Nom dont on tire l'initiale (Google, sinon identifiant du compte NAS).
    pub name: String,
    texture: Option<TextureHandle>,
    /// Dates des fichiers lus : relus s'ils changent (reçus d'un autre poste, par exemple).
    stamps: (Option<SystemTime>, Option<SystemTime>),
    checked: Option<Instant>,
}

fn json_path() -> PathBuf {
    crate::paths::config().join("avatar.json")
}

fn image_path() -> PathBuf {
    crate::paths::config().join("avatar.png")
}

fn mtime(p: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn load_texture(ctx: &egui::Context) -> Option<TextureHandle> {
    let img = image::open(image_path()).ok()?.to_rgba8();
    let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    Some(ctx.load_texture("account_avatar", ci, egui::TextureOptions::LINEAR))
}

impl Avatar {
    /// À chaque image : relit le choix, l'image et le nom s'ils ont changé (toutes les 2 s) et
    /// demande la photo Google si c'est elle qui est affichée.
    pub fn tick(&mut self, ctx: &egui::Context, chat: &mut ChatState, acc: &Account) {
        if self.checked.is_none_or(|t| t.elapsed() >= Duration::from_secs(2)) {
            self.checked = Some(Instant::now());
            let stamps = (mtime(&json_path()), mtime(&image_path()));
            if stamps != self.stamps {
                self.stamps = stamps;
                self.source = std::fs::read_to_string(json_path()).ok().and_then(|s| serde_json::from_str::<Saved>(&s).ok()).unwrap_or_default().source;
                self.texture = load_texture(ctx);
            }
            // Prénom et nom du compte Google (fiche de l'annuaire, pas « Vous »).
            let google = chat.me.as_ref().and_then(|me| chat.people.get(me)).map(|p| p.name.clone()).filter(|n| !n.trim().is_empty());
            self.name = google.or_else(|| crate::sync::Account::load().map(|a| a.login)).unwrap_or_default();
        }
        // Fiche Google (nom pour les initiales) et photo, quel que soit le choix.
        if let Some(me) = chat.me.clone() {
            chat.request_photo(acc, &me);
            let _ = chat.images.get(ctx, &format!("photo:{me}"));
        }
    }

    /// Une image importée existe (choix « Image » possible).
    pub fn has_image(&self) -> bool {
        self.texture.is_some()
    }

    /// La photo du compte Google est connue.
    pub fn has_google_photo(&self, chat: &ChatState) -> bool {
        chat.me.as_ref().and_then(|me| chat.photo_key(me)).is_some()
    }

    /// Change l'origine de l'image (enregistrée, donc synchronisée).
    pub fn set_source(&mut self, source: Source) {
        self.source = source;
        crate::paths::save_json(&json_path(), &Saved { source });
        // Relu au prochain passage.
        self.stamps = (None, None);
        self.checked = None;
    }

    /// Importe une image : recadrée au centre en carré, 256 × 256, puis choisie.
    pub fn import(&mut self, src: &std::path::Path) -> Result<(), String> {
        let img = image::open(src).map_err(|e| tf!("Image illisible : {e}", "Unreadable image: {e}"))?;
        let (w, h) = (img.width(), img.height());
        let side = w.min(h).max(1);
        let out = img.crop_imm((w - side) / 2, (h - side) / 2, side, side).resize_exact(256, 256, image::imageops::FilterType::Lanczos3);
        let mut png = Vec::new();
        out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| e.to_string())?;
        crate::paths::write_atomic(&image_path(), &png).map_err(|e| e.to_string())?;
        self.set_source(Source::Image);
        Ok(())
    }

    /// Dessine l'image du compte, ronde, centrée sur `c`, de rayon `r`.
    pub fn paint(&self, p: &egui::Painter, chat: &ChatState, c: Pos2, r: f32) {
        if self.source == Source::Image
            && let Some(t) = &self.texture
        {
            let rect = Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r));
            let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
            p.add(egui::epaint::RectShape::filled(rect, CornerRadius::same(r.min(255.0) as u8), Color32::WHITE).with_texture(t.id(), uv));
            return;
        }
        if self.source == Source::Google
            && let Some(key) = chat.me.as_ref().and_then(|me| chat.photo_key(me))
            && crate::tabs::messages::image_avatar(p, chat, &key, c, r, false)
        {
            return;
        }
        crate::tabs::messages::initials_avatar(p, c, r, if self.name.is_empty() { "?" } else { &self.name }, false);
    }
}
