//! Image du compte (pastille « Compte » en haut de la barre latérale) : photo Google, initiale
//! ou image importée. Choix dans `config\avatar.json`, image importée dans `config\avatar.png`
//! (256 × 256) : tous deux synchronisés avec le compte. Une petite copie en clair de l'image
//! affichée est aussi envoyée au NAS, pour la console (onglet Synchro).

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, TextureHandle, vec2};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant, SystemTime};

use crate::google::Account;
use crate::tabs::mail::inline::ImgState;
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
    /// Copie pour la console du NAS : à envoyer (image ou compte changés), envoi en cours,
    /// échec (nouvel essai dans une minute), compte NAS connu.
    upload_due: bool,
    sending: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    retry_at: Option<Instant>,
    login: Option<String>,
    photo_ready: bool,
}

/// Image envoyée à la console du NAS : carré de 96 px (moins de 64 Ko en PNG, même au pire).
const UPLOAD_PX: u32 = 96;

/// Ce qu'il faut envoyer à la console du NAS.
enum Upload {
    /// Photo Google pas encore chargée : attendre.
    Wait,
    /// Image à envoyer, ou `None` : la retirer (la console affiche alors les initiales).
    Send(Option<Vec<u8>>),
}

/// Recadre au centre en carré, réduit à `UPLOAD_PX` et encode en PNG.
fn upload_png(img: image::DynamicImage) -> Option<Vec<u8>> {
    let (w, h) = (img.width(), img.height());
    let side = w.min(h).max(1);
    let out = img.crop_imm((w - side) / 2, (h - side) / 2, side, side).resize_exact(UPLOAD_PX, UPLOAD_PX, image::imageops::FilterType::Lanczos3);
    let mut png = Vec::new();
    out.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    Some(png)
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
                self.upload_due = true;
            }
            let login = crate::sync::Account::load().map(|a| a.login);
            // Prénom et nom du compte Google (fiche de l'annuaire, pas « Vous »).
            let google = chat.me.as_ref().and_then(|me| chat.people.get(me)).map(|p| p.name.clone()).filter(|n| !n.trim().is_empty());
            self.name = google.or_else(|| login.clone()).unwrap_or_default();
            // Connexion au NAS (ou autre compte) : la copie de l'image est renvoyée.
            if login != self.login {
                self.login = login;
                self.upload_due = true;
            }
            // Photo Google arrivée (ou perdue) : renvoyée si c'est elle qui est affichée.
            let photo = chat.me.as_ref().and_then(|me| chat.photo_key(me)).is_some_and(|k| matches!(chat.images.peek(&k), Some(ImgState::Ready { .. })));
            if photo != self.photo_ready {
                self.photo_ready = photo;
                self.upload_due |= self.source == Source::Google;
            }
            self.upload(chat);
        }
        // Fiche Google (nom pour les initiales) et photo, quel que soit le choix.
        if let Some(me) = chat.me.clone() {
            chat.request_photo(acc, &me);
            let _ = chat.images.get(ctx, &format!("photo:{me}"));
        }
    }

    /// Envoie au NAS, en arrière-plan, une copie en clair de l'image affichée (la console du NAS
    /// ne peut pas lire les fichiers synchronisés, chiffrés). Au lancement, puis à chaque
    /// changement d'image ou de compte ; nouvel essai une minute après un échec.
    fn upload(&mut self, chat: &ChatState) {
        if self.failed.swap(false, Ordering::Relaxed) {
            self.upload_due = true;
            self.retry_at = Some(Instant::now() + Duration::from_secs(60));
        }
        if !self.upload_due || self.login.is_none() || self.sending.load(Ordering::Relaxed) || self.retry_at.is_some_and(|t| Instant::now() < t) {
            return;
        }
        let png = match self.payload(chat) {
            Upload::Wait => return,
            Upload::Send(png) => png,
        };
        self.upload_due = false;
        self.retry_at = None;
        self.sending.store(true, Ordering::Relaxed);
        let (sending, failed) = (self.sending.clone(), self.failed.clone());
        std::thread::spawn(move || {
            if crate::sync::upload_avatar(png.as_deref()).is_err() {
                failed.store(true, Ordering::Relaxed);
            }
            sending.store(false, Ordering::Relaxed);
        });
    }

    fn payload(&self, chat: &ChatState) -> Upload {
        match self.source {
            Source::Initial => Upload::Send(None),
            // Image illisible (en cours d'écriture…) : on attend plutôt que de la retirer du NAS.
            Source::Image => match image::open(image_path()).ok().and_then(upload_png) {
                Some(png) => Upload::Send(Some(png)),
                None => Upload::Wait,
            },
            Source::Google => {
                // Compte Google ou fiche pas encore chargés (lancement) : attendre.
                let Some(person) = chat.me.as_ref().and_then(|me| chat.people.get(me)) else { return Upload::Wait };
                if person.photo.is_none() {
                    return Upload::Send(None);
                }
                let Some(key) = chat.photo_key(&person.id) else { return Upload::Send(None) };
                match chat.images.peek(&key) {
                    Some(ImgState::Ready { image, .. }) => {
                        let [w, h] = image.size;
                        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
                        let img = image::RgbaImage::from_raw(w as u32, h as u32, rgba).map(image::DynamicImage::ImageRgba8);
                        Upload::Send(img.and_then(upload_png))
                    }
                    Some(ImgState::Failed) => Upload::Send(None),
                    _ => Upload::Wait,
                }
            }
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
