//! Pièces jointes : aperçu dans l'application, sinon ouverture avec l'application
//! par défaut du système, et en tout dernier recours, lien vers Gmail.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle};

use crate::google::{Account, Attachment, Event, gmail};

/// Extensions jamais ouvertes automatiquement (exécutables, scripts, raccourcis…).
const DANGEROUS: &[&str] = &[
    "exe",
    "msi",
    "msix",
    "appx",
    "bat",
    "cmd",
    "com",
    "scr",
    "pif",
    "cpl",
    "msc",
    "ps1",
    "psm1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "wsf",
    "wsh",
    "hta",
    "lnk",
    "url",
    "reg",
    "jar",
    "dll",
    "sys",
    "iso",
    "img",
    "vhd",
    "vhdx",
    "application",
    "gadget",
    "inf",
    "scf",
];
const IMAGE_EXT: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const TEXT_EXT: &[&str] = &["txt", "csv", "tsv", "md", "json", "log", "xml", "yaml", "yml", "ini", "toml", "rs", "py", "c", "cpp", "h", "hpp", "cs", "ts", "css", "sql"];
/// Au-delà, le texte est ouvert dans l'application externe plutôt qu'affiché.
const MAX_TEXT_PREVIEW: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Image,
    Text,
    External,
}

fn kind(a: &Attachment) -> Kind {
    let ext = a.extension();
    if IMAGE_EXT.contains(&ext.as_str()) || matches!(a.mime.as_str(), "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp") {
        Kind::Image
    } else if TEXT_EXT.contains(&ext.as_str()) || a.mime.starts_with("text/plain") || a.mime == "text/csv" || a.mime == "application/json" {
        Kind::Text
    } else {
        Kind::External
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Intent {
    Preview,
    OpenExternal,
}

pub enum Content {
    Loading,
    Image { image: Arc<ColorImage>, texture: Option<TextureHandle> },
    Text(String),
}

pub struct Preview {
    pub att: Attachment,
    pub content: Content,
}

#[derive(Default)]
pub struct Attachments {
    pub preview: Option<Preview>,
    /// Pièces jointes impossibles à afficher ou ouvrir → raison (affiche la pastille Gmail).
    pub blocked: HashMap<String, String>,
    /// Téléchargements en cours.
    pub pending: HashSet<String>,
    intents: HashMap<String, (Attachment, Intent)>,
    /// Contenus déjà téléchargés (évite de reconsommer du quota).
    cache: HashMap<String, Arc<Vec<u8>>>,
}

/// Résultat à signaler à l'utilisateur : (texte, est une erreur).
pub type Notice = Option<(String, bool)>;

impl Attachments {
    /// Clic sur une pièce jointe.
    pub fn activate(&mut self, acc: &Account, att: &Attachment) -> Notice {
        if DANGEROUS.contains(&att.extension().as_str()) {
            self.block(att, t!("Type de fichier bloqué par sécurité (exécutable ou script)", "File type blocked for safety (executable or script)"));
            return Some((tf!("« {} » n'est pas ouvert automatiquement par sécurité", "“{}” is not opened automatically for safety", att.filename), true));
        }
        match kind(att) {
            Kind::Image | Kind::Text => {
                self.preview = Some(Preview { att: att.clone(), content: Content::Loading });
                self.fetch(acc, att, Intent::Preview)
            }
            Kind::External => self.fetch(acc, att, Intent::OpenExternal),
        }
    }

    /// Bouton « Ouvrir avec l'application » de la fenêtre d'aperçu.
    pub fn open_external(&mut self, acc: &Account, att: &Attachment) -> Notice {
        self.fetch(acc, att, Intent::OpenExternal)
    }

    fn fetch(&mut self, acc: &Account, att: &Attachment, intent: Intent) -> Notice {
        let key = att.key();
        if let Some(data) = self.cache.get(&key).cloned() {
            return self.use_data(att, intent, &data);
        }
        self.intents.insert(key.clone(), (att.clone(), intent));
        if self.pending.insert(key.clone()) {
            let a = att.clone();
            acc.run(move |s| Ok(Event::AttachmentData { key, data: gmail::attachment(s, &a) }));
        }
        None
    }

    /// Réception du contenu téléchargé.
    pub fn on_data(&mut self, key: String, data: Result<Vec<u8>, String>) -> Notice {
        self.pending.remove(&key);
        let (att, intent) = self.intents.remove(&key)?;
        match data {
            Ok(bytes) => {
                let bytes = Arc::new(bytes);
                self.cache.insert(key, bytes.clone());
                self.use_data(&att, intent, &bytes)
            }
            Err(e) => {
                self.close_preview_of(&att);
                self.block(&att, &tf!("Téléchargement impossible : {e}", "Download failed: {e}"));
                Some((tf!("« {} » n'a pas pu être récupérée", "“{}” could not be retrieved", att.filename), true))
            }
        }
    }

    fn use_data(&mut self, att: &Attachment, intent: Intent, data: &[u8]) -> Notice {
        if intent == Intent::Preview {
            match kind(att) {
                Kind::Image => {
                    if let Ok(img) = image::load_from_memory(data) {
                        let rgba = img.to_rgba8();
                        let size = [rgba.width() as usize, rgba.height() as usize];
                        let image = Arc::new(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw()));
                        self.set_preview(att, Content::Image { image, texture: None });
                        return None;
                    }
                }
                Kind::Text if data.len() <= MAX_TEXT_PREVIEW => {
                    self.set_preview(att, Content::Text(String::from_utf8_lossy(data).into_owned()));
                    return None;
                }
                _ => {}
            }
            // Aperçu interne impossible → application par défaut.
            self.close_preview_of(att);
        }

        match write_temp(att, data).and_then(|path| open::that_detached(&path).map_err(|e| e.to_string())) {
            Ok(()) => Some((tf!("« {} » ouvert dans l'application par défaut", "“{}” opened in the default app", att.filename), false)),
            Err(e) => {
                self.block(att, &tf!("Aucune application n'a pu ouvrir le fichier : {e}", "No app could open the file: {e}"));
                Some((tf!("Impossible d'ouvrir « {} » sur cet ordinateur", "Cannot open “{}” on this computer", att.filename), true))
            }
        }
    }

    fn set_preview(&mut self, att: &Attachment, content: Content) {
        if let Some(p) = self.preview.as_mut().filter(|p| p.att.key() == att.key()) {
            p.content = content;
        }
    }

    fn close_preview_of(&mut self, att: &Attachment) {
        if self.preview.as_ref().is_some_and(|p| p.att.key() == att.key()) {
            self.preview = None;
        }
    }

    fn block(&mut self, att: &Attachment, reason: &str) {
        self.blocked.insert(att.key(), reason.to_string());
    }
}

/// Copie temporaire : %TEMP%\WorkTable\<message>\<fichier>.
fn write_temp(att: &Attachment, data: &[u8]) -> Result<PathBuf, String> {
    let safe: String = att.filename.chars().map(|c| if c.is_control() || r#"\/:*?"<>|"#.contains(c) { '_' } else { c }).collect();
    let dir = std::env::temp_dir().join("WorkTable").join(&att.message_id);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(if safe.trim().is_empty() { "piece-jointe".into() } else { safe });
    std::fs::write(&path, data).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Lien de dernier recours : le message dans Gmail (web), avec ses pièces jointes.
pub fn gmail_link(email: &str, message_id: &str) -> String {
    format!("https://mail.google.com/mail/?authuser={}#all/{}", urlencoding::encode(email), message_id)
}

pub fn human_size(bytes: u64) -> String {
    match bytes {
        0 => String::new(),
        b if b < 1024 => tf!("{b} o", "{b} B"),
        b if b < 1024 * 1024 => tf!("{:.0} Ko", "{:.0} KB", b as f64 / 1024.0),
        b if crate::i18n::en() => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
        b => format!("{:.1} Mo", b as f64 / (1024.0 * 1024.0)).replace('.', ","),
    }
}

/// Crée la texture de l'image à la première frame où l'aperçu est affiché.
pub fn texture_for(ctx: &egui::Context, content: &mut Content, name: &str) -> Option<TextureHandle> {
    if let Content::Image { image, texture } = content {
        if texture.is_none() {
            *texture = Some(ctx.load_texture(name, image.clone(), egui::TextureOptions::LINEAR));
        }
        return texture.clone();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn att(name: &str, mime: &str) -> Attachment {
        Attachment { message_id: "m1".into(), attachment_id: Some("a1".into()), filename: name.into(), mime: mime.into(), size: 10, inline_data: None, content_id: None }
    }

    #[test]
    fn classifies_files() {
        assert!(kind(&att("photo.JPG", "image/jpeg")) == Kind::Image);
        assert!(kind(&att("notes.md", "application/octet-stream")) == Kind::Text);
        assert!(kind(&att("rapport.pdf", "application/pdf")) == Kind::External);
        assert!(DANGEROUS.contains(&att("setup.exe", "").extension().as_str()));
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(2048), "2 Ko");
        assert_eq!(human_size(1_572_864), "1,5 Mo");
    }

    #[test]
    fn decodes_png_preview() {
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let a = att("x.png", "image/png");
        let mut s = Attachments { preview: Some(Preview { att: a.clone(), content: Content::Loading }), ..Default::default() };
        assert!(s.use_data(&a, Intent::Preview, &png).is_none());
        match &s.preview.unwrap().content {
            Content::Image { image, .. } => assert_eq!(image.size, [3, 2]),
            _ => panic!("aperçu image attendu"),
        }
    }
}
