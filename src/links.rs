//! Galerie « Liens Sites Important » du tableau de bord : 9 cases (3 × 3), chacune un logo
//! (émoji ou image 256 × 256) qui ouvre un site. Enregistrée dans `config\links.json`,
//! images dans `config\links\`.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const COUNT: usize = 9;
/// Taille des images de logo.
pub const IMAGE_PX: u32 = 256;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tile {
    pub name: String,
    pub url: String,
    /// Logo émoji (utilisé si pas d'image).
    pub emoji: String,
    /// Logo image : nom du fichier dans `config\links\`.
    pub image: Option<String>,
}

impl Tile {
    pub fn is_empty(&self) -> bool {
        self.url.trim().is_empty()
    }
}

/// Case en cours de modification.
pub struct Edit {
    pub index: usize,
    pub tile: Tile,
    pub use_image: bool,
    pub picker: bool,
    pub emoji_query: String,
    pub emoji_group: usize,
    pub error: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Links {
    pub tiles: Vec<Tile>,
    #[serde(skip)]
    pub edit: Option<Edit>,
    #[serde(skip)]
    pub textures: HashMap<String, Option<eframe::egui::TextureHandle>>,
}

fn config_dir() -> Option<PathBuf> {
    Some(crate::paths::config())
}

pub fn images_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("links"))
}

fn defaults() -> Vec<Tile> {
    [
        ("Gmail", "https://mail.google.com", "✉️"),
        ("Google Agenda", "https://calendar.google.com", "📅"),
        ("Google Drive", "https://drive.google.com", "📁"),
        ("Google Docs", "https://docs.google.com", "📄"),
        ("Google Sheets", "https://sheets.google.com", "📊"),
        ("Google Meet", "https://meet.google.com", "🎥"),
        ("Notion", "https://www.notion.so", "📝"),
        ("GitHub", "https://github.com", "🐙"),
        ("YouTube", "https://www.youtube.com", "▶️"),
    ]
    .iter()
    .map(|(n, u, e)| Tile { name: n.to_string(), url: u.to_string(), emoji: e.to_string(), image: None })
    .collect()
}

impl Links {
    pub fn load() -> Self {
        let mut l: Links = config_dir()
            .and_then(|d| std::fs::read_to_string(d.join("links.json")).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| Links { tiles: defaults(), ..Default::default() });
        l.tiles.resize(COUNT, Tile::default());
        l
    }

    pub fn save(&self) {
        if let Some(d) = config_dir() {
            let _ = std::fs::create_dir_all(&d);
            if let Ok(t) = serde_json::to_string_pretty(self) {
                let _ = std::fs::write(d.join("links.json"), t);
            }
        }
    }

    pub fn start_edit(&mut self, index: usize) {
        let tile = self.tiles[index].clone();
        let use_image = tile.image.is_some();
        self.edit = Some(Edit { index, tile, use_image, picker: false, emoji_query: String::new(), emoji_group: 0, error: None });
    }

    /// Enregistre la case modifiée.
    pub fn commit(&mut self) -> Result<(), String> {
        let Some(e) = &mut self.edit else { return Ok(()) };
        let mut t = e.tile.clone();
        t.url = normalize_url(&t.url);
        if t.url.is_empty() {
            return Err("Indique l'adresse du site".into());
        }
        if !e.use_image {
            t.image = None;
        } else if t.image.is_none() {
            return Err("Choisis une image (ou passe en émoji)".into());
        }
        if t.name.trim().is_empty() {
            t.name = host_of(&t.url);
        }
        let old = std::mem::replace(&mut self.tiles[e.index], t);
        // Ancienne image remplacée : retirée du disque.
        if old.image.is_some() && old.image != self.tiles[e.index].image {
            remove_image(old.image.as_deref());
        }
        self.edit = None;
        self.save();
        Ok(())
    }

    pub fn clear(&mut self, index: usize) {
        let old = std::mem::take(&mut self.tiles[index]);
        remove_image(old.image.as_deref());
        self.save();
    }

    pub fn swap(&mut self, a: usize, b: usize) {
        if a != b && a < COUNT && b < COUNT {
            self.tiles.swap(a, b);
            self.save();
        }
    }
}

fn remove_image(name: Option<&str>) {
    if let (Some(n), Some(d)) = (name, images_dir()) {
        let _ = std::fs::remove_file(d.join(n));
    }
}

/// Ajoute « https:// » si l'adresse n'a pas de protocole.
pub fn normalize_url(u: &str) -> String {
    let u = u.trim();
    if u.is_empty() || u.contains("://") || u.starts_with("mailto:") {
        u.to_string()
    } else {
        format!("https://{u}")
    }
}

pub fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    rest.split('/').next().unwrap_or(rest).trim_start_matches("www.").to_string()
}

/// Importe une image : recadrée au centre en carré puis redimensionnée en 256 × 256 (PNG).
pub fn import_image(src: &std::path::Path) -> Result<String, String> {
    let img = image::open(src).map_err(|e| format!("Image illisible : {e}"))?;
    let (w, h) = (img.width(), img.height());
    let side = w.min(h).max(1);
    let square = img.crop_imm((w - side) / 2, (h - side) / 2, side, side);
    let out = square.resize_exact(IMAGE_PX, IMAGE_PX, image::imageops::FilterType::Lanczos3);
    let dir = images_dir().ok_or("dossier introuvable")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name = format!("logo-{}.png", chrono::Local::now().format("%Y%m%d%H%M%S%3f"));
    out.save_with_format(dir.join(&name), image::ImageFormat::Png).map_err(|e| e.to_string())?;
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(normalize_url("notion.so"), "https://notion.so");
        assert_eq!(normalize_url(" http://a.b "), "http://a.b");
        assert_eq!(host_of("https://www.notion.so/page"), "notion.so");
    }

    #[test]
    fn image_is_square_256() {
        let src = std::env::temp_dir().join("wt_logo_test.png");
        image::RgbaImage::new(640, 300).save(&src).unwrap();
        let name = import_image(&src).unwrap();
        let path = images_dir().unwrap().join(&name);
        let img = image::open(&path).unwrap();
        assert_eq!((img.width(), img.height()), (IMAGE_PX, IMAGE_PX));
        let _ = std::fs::remove_file(path);
    }
}
