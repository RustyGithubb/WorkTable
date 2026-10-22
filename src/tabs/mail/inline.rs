//! Images affichées dans le corps des mails reçus (intégrées `cid:`, hébergées, ou `data:`).

use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;

use eframe::egui::{self, ColorImage, TextureHandle};

use crate::google::html::{BodyBlock, ImageSrc};
use crate::google::{Account, Event, FullMessage, gmail};

/// Taille maximale téléchargée pour une image hébergée.
const MAX_REMOTE: u64 = 10 * 1024 * 1024;
/// Les très grandes images sont réduites (mémoire graphique).
const MAX_SIDE: u32 = 2400;

pub enum ImgState {
    Loading,
    Ready { image: Arc<ColorImage>, texture: Option<TextureHandle> },
    Failed,
}

#[derive(Default)]
pub struct InlineImages {
    map: HashMap<String, ImgState>,
}

pub fn key(msg_id: &str, index: usize, src: &ImageSrc) -> String {
    match src {
        ImageSrc::Cid(c) => format!("cid:{msg_id}/{c}"),
        ImageSrc::Url(u) => format!("url:{u}"),
        ImageSrc::Data(_) => format!("data:{msg_id}/{index}"),
    }
}

impl InlineImages {
    /// Lance le chargement des images d'un mail qui vient d'être ouvert.
    pub fn request(&mut self, acc: &Account, msg: &FullMessage) {
        self.request_blocks(acc, &msg.summary.id, &msg.blocks, |cid| msg.by_cid(cid).cloned());
    }

    /// Charge les images d'une suite de blocs (mail, signature…), identifiés par `owner`.
    pub fn request_blocks(&mut self, acc: &Account, owner: &str, blocks: &[BodyBlock], by_cid: impl Fn(&str) -> Option<crate::google::Attachment>) {
        for (i, b) in blocks.iter().enumerate() {
            let BodyBlock::Image { src, .. } = b else { continue };
            let k = key(owner, i, src);
            if self.map.contains_key(&k) {
                continue;
            }
            self.map.insert(k.clone(), ImgState::Loading);
            match src {
                ImageSrc::Cid(cid) => match by_cid(cid) {
                    Some(att) => acc.run(move |s| {
                        let image = gmail::attachment(s, &att).and_then(|d| decode(&d));
                        Ok(Event::InlineImage { key: k, image })
                    }),
                    None => {
                        self.map.insert(k, ImgState::Failed);
                    }
                },
                ImageSrc::Url(url) => {
                    let (agent, url) = (acc.agent(), url.clone());
                    acc.spawn(move || {
                        let image = fetch(&agent, &url).and_then(|d| decode(&d));
                        Ok(Event::InlineImage { key: k, image })
                    });
                }
                ImageSrc::Data(bytes) => {
                    let bytes = bytes.clone();
                    acc.spawn(move || Ok(Event::InlineImage { key: k, image: decode(&bytes) }));
                }
            }
        }
    }

    pub fn on_loaded(&mut self, key: String, image: Result<Arc<ColorImage>, String>) {
        let state = match image {
            Ok(image) => ImgState::Ready { image, texture: None },
            Err(_) => ImgState::Failed,
        };
        self.map.insert(key, state);
    }

    /// État d'une image, avec création de la texture à la première utilisation.
    /// Oublie une image (pour la recharger, ex. icône de groupe changée).
    pub fn forget(&mut self, key: &str) {
        self.map.remove(key);
    }

    /// État d'une image sans créer de texture.
    pub fn peek(&self, key: &str) -> Option<&ImgState> {
        self.map.get(key)
    }

    /// Réserve une clé avant un chargement externe ; `false` si elle est déjà connue.
    pub fn start(&mut self, key: &str) -> bool {
        if self.map.contains_key(key) {
            return false;
        }
        self.map.insert(key.to_string(), ImgState::Loading);
        true
    }

    pub fn get(&mut self, ctx: &egui::Context, key: &str) -> Option<&ImgState> {
        let state = self.map.get_mut(key)?;
        if let ImgState::Ready { image, texture } = state
            && texture.is_none()
        {
            *texture = Some(ctx.load_texture(key, image.clone(), egui::TextureOptions::LINEAR));
        }
        Some(state)
    }
}

fn fetch(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    let resp = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    resp.into_reader().take(MAX_REMOTE).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

pub fn decode(bytes: &[u8]) -> Result<Arc<ColorImage>, String> {
    let mut img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    if img.width() <= 2 && img.height() <= 2 {
        return Err("pixel de suivi".into());
    }
    if img.width() > MAX_SIDE || img.height() > MAX_SIDE {
        img = img.thumbnail(MAX_SIDE, MAX_SIDE);
    }
    let rgba = img.to_rgba8();
    let size = [rgba.width() as usize, rgba.height() as usize];
    Ok(Arc::new(ColorImage::from_rgba_unmultiplied(size, rgba.as_raw())))
}
