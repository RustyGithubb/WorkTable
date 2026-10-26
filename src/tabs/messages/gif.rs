//! GIF animés des messages Google Chat : GIF choisis dans Chat (`attachedGifs`, hébergés chez
//! Tenor / Giphy) et fichiers `.gif` envoyés en pièce jointe. Toutes les images sont décodées en
//! arrière-plan, réduites pour l'affichage dans la conversation, puis jouées en boucle (aussi dans
//! la visionneuse).

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle};
use image::AnimationDecoder;

/// Taille maximale téléchargée.
const MAX_BYTES: u64 = 20 * 1024 * 1024;
/// Côté maximal des images gardées (mémoire graphique : un GIF compte souvent des dizaines d'images).
const MAX_SIDE: u32 = 640;
/// Nombre d'images gardées au plus.
const MAX_FRAMES: usize = 400;
/// Durée minimale d'une image (ms) : comme les navigateurs, les durées nulles ou très courtes
/// passent à 100 ms.
const MIN_DELAY: u32 = 20;

/// Image et durée d'affichage (ms).
pub type Frame = (Arc<ColorImage>, u32);

pub enum GifState {
    Loading,
    Ready { frames: Vec<Frame>, textures: Vec<TextureHandle>, total: u32 },
    Failed,
}

#[derive(Default)]
pub struct Gifs {
    map: HashMap<String, GifState>,
}

impl Gifs {
    /// Réserve une clé avant le chargement ; `false` si elle est déjà connue.
    pub fn start(&mut self, key: &str) -> bool {
        if self.map.contains_key(key) {
            return false;
        }
        self.map.insert(key.to_string(), GifState::Loading);
        true
    }

    pub fn on_loaded(&mut self, key: String, frames: Result<Vec<Frame>, String>) {
        let state = match frames {
            Ok(frames) if !frames.is_empty() => {
                let total = frames.iter().map(|f| f.1).sum::<u32>().max(1);
                GifState::Ready { frames, textures: Vec::new(), total }
            }
            _ => GifState::Failed,
        };
        self.map.insert(key, state);
    }

    pub fn is_known(&self, key: &str) -> bool {
        self.map.contains_key(key)
    }

    pub fn failed(&self, key: &str) -> bool {
        matches!(self.map.get(key), Some(GifState::Failed))
    }

    /// Crée les textures d'un GIF chargé (avant l'affichage, avec accès mutable).
    pub fn prepare(&mut self, ctx: &egui::Context, key: &str) {
        if let Some(GifState::Ready { frames, textures, .. }) = self.map.get_mut(key)
            && textures.is_empty()
        {
            *textures = frames.iter().enumerate().map(|(i, (img, _))| ctx.load_texture(format!("gif:{key}:{i}"), img.clone(), egui::TextureOptions::LINEAR)).collect();
        }
    }

    /// Image à afficher maintenant et taille du GIF ; demande l'image suivante à temps.
    pub fn frame(&self, ctx: &egui::Context, key: &str) -> Option<(TextureHandle, egui::Vec2)> {
        let GifState::Ready { frames, textures, total } = self.map.get(key)? else {
            return None;
        };
        let now = (ctx.input(|i| i.time) * 1000.0) as u64;
        let (i, left) = at(frames, *total, now);
        if frames.len() > 1 {
            ctx.request_repaint_after(std::time::Duration::from_millis(left as u64));
        }
        let t = textures.get(i)?.clone();
        let size = t.size_vec2();
        Some((t, size))
    }
}

/// Image affichée à l'instant `now` (ms) et temps restant avant la suivante.
fn at(frames: &[Frame], total: u32, now: u64) -> (usize, u32) {
    let mut t = (now % total as u64) as u32;
    for (i, (_, d)) in frames.iter().enumerate() {
        if t < *d {
            return (i, d - t);
        }
        t -= d;
    }
    (0, frames.first().map_or(100, |f| f.1))
}

/// Télécharge un GIF public (sans connexion Google).
pub fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let r = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.into_reader().take(MAX_BYTES).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Toutes les images d'un GIF (une seule pour une image fixe).
pub fn decode(bytes: &[u8]) -> Result<Vec<Frame>, String> {
    if !bytes.starts_with(b"GIF8") {
        return crate::tabs::mail::inline::decode(bytes).map(|img| vec![(img, 1000)]);
    }
    let decoder = image::codecs::gif::GifDecoder::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for f in decoder.into_frames().take(MAX_FRAMES) {
        let f = f.map_err(|e| e.to_string())?;
        let (num, den) = f.delay().numer_denom_ms();
        let ms = num.checked_div(den).unwrap_or(100);
        let ms = if ms < MIN_DELAY { 100 } else { ms };
        let mut buf = f.into_buffer();
        if buf.width() > MAX_SIDE || buf.height() > MAX_SIDE {
            buf = image::DynamicImage::ImageRgba8(buf).thumbnail(MAX_SIDE, MAX_SIDE).to_rgba8();
        }
        let size = [buf.width() as usize, buf.height() as usize];
        out.push((Arc::new(ColorImage::from_rgba_unmultiplied(size, buf.as_raw())), ms));
    }
    if out.is_empty() {
        return Err("GIF vide".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GIF de 2 images 1×1 (rouge 100 ms, bleu 200 ms).
    fn two_frames() -> Vec<u8> {
        use image::codecs::gif::{GifEncoder, Repeat};
        use image::{Delay, Frame as IFrame, Rgba, RgbaImage};
        let mut bytes = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut bytes);
            enc.set_repeat(Repeat::Infinite).unwrap();
            for (c, ms) in [([255, 0, 0, 255], 100), ([0, 0, 255, 255], 200)] {
                let img = RgbaImage::from_pixel(3, 3, Rgba(c));
                enc.encode_frame(IFrame::from_parts(img, 0, 0, Delay::from_numer_denom_ms(ms, 1))).unwrap();
            }
        }
        bytes
    }

    #[test]
    fn decodes_every_frame_with_its_delay() {
        let frames = decode(&two_frames()).unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!((frames[0].1, frames[1].1), (100, 200));
        assert_eq!(frames[0].0.size, [3, 3]);
        // Lecture en boucle : 0-99 → image 0, 100-299 → image 1, puis on recommence.
        assert_eq!(at(&frames, 300, 50), (0, 50));
        assert_eq!(at(&frames, 300, 150), (1, 150));
        assert_eq!(at(&frames, 300, 310), (0, 90));
    }
}
