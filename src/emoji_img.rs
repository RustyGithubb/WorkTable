//! Émojis en couleur, style 3D proche d'iOS : jeu « Noto Emoji 3D » de Google (licence SIL OFL 1.1).
//!
//! Chaque émoji est téléchargé une seule fois (72 px) puis gardé sur le disque
//! (`data\cache\emoji`) ; en attendant ou hors ligne, le glyphe
//! monochrome de la police sert de repli.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Pos2, Rect, TextureHandle, vec2};

const SOURCES: [&str; 2] =
    ["https://raw.githubusercontent.com/googlefonts/noto-emoji/main/3D/png/72/emoji_u", "https://raw.githubusercontent.com/googlefonts/noto-emoji/main/2D/png/72/emoji_u"];
/// Chargements simultanés (disque ou réseau).
const MAX_DOWNLOADS: usize = 24;

enum Slot {
    Loading,
    Ready(Arc<ColorImage>),
    Texture(TextureHandle),
    Missing,
}

fn cache() -> &'static Mutex<HashMap<String, Slot>> {
    static C: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

static IN_FLIGHT: AtomicUsize = AtomicUsize::new(0);

/// Nom de fichier Noto : points de code en hexadécimal (4 chiffres min.), sans sélecteur de variante.
pub fn file_code(emoji: &str) -> String {
    emoji.chars().filter(|c| *c != '\u{FE0F}').map(|c| format!("{:04x}", c as u32)).collect::<Vec<_>>().join("_")
}

/// Émoji correspondant à un nom de fichier du cache (inverse de [`file_code`]).
fn emoji_of_code(code: &str) -> Option<String> {
    let s: String = code.split('_').map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32)).collect::<Option<String>>()?;
    // Le fichier ne garde pas le sélecteur de variante : on retrouve la forme du catalogue.
    if let Some(e) = emojis::get(&s) {
        return Some(e.as_str().to_string());
    }
    let with_fe0f: String = s.chars().flat_map(|c| [c, '\u{FE0F}']).collect();
    emojis::get(&with_fe0f).or_else(|| emojis::iter().find(|e| file_code(e.as_str()) == code)).map(|e| e.as_str().to_string())
}

/// Émojis déjà enregistrés sur cet ordinateur (images du cache) : affichés sans attendre, même
/// hors ligne. Relu au plus toutes les 5 s.
pub fn stored() -> Vec<String> {
    static LIST: OnceLock<Mutex<(Option<std::time::Instant>, Vec<String>)>> = OnceLock::new();
    let m = LIST.get_or_init(Default::default);
    let Ok(mut g) = m.lock() else { return Vec::new() };
    if g.0.is_none_or(|t| t.elapsed() >= std::time::Duration::from_secs(5)) {
        let mut list: Vec<String> = disk_dir()
            .and_then(|d| std::fs::read_dir(d).ok())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".png")).and_then(emoji_of_code))
            .collect();
        // Ordre du catalogue Unicode (comme les catégories).
        let order: std::collections::HashMap<&str, usize> = emojis::iter().enumerate().map(|(i, e)| (e.as_str(), i)).collect();
        list.sort_by_key(|e| order.get(e.as_str()).copied().unwrap_or(usize::MAX));
        list.dedup();
        *g = (Some(std::time::Instant::now()), list);
    }
    g.1.clone()
}

fn disk_dir() -> Option<PathBuf> {
    Some(crate::paths::cache().join("emoji"))
}

fn decode(bytes: &[u8]) -> Option<Arc<ColorImage>> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let size = [img.width() as usize, img.height() as usize];
    Some(Arc::new(ColorImage::from_rgba_unmultiplied(size, img.as_raw())))
}

/// Client HTTP partagé : la connexion (TLS) est réutilisée d'un émoji à l'autre.
fn agent() -> &'static ureq::Agent {
    static A: OnceLock<ureq::Agent> = OnceLock::new();
    A.get_or_init(|| ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(15)).max_idle_connections_per_host(16).try_proxy_from_env(true).build())
}

fn fetch(code: &str) -> Option<Vec<u8>> {
    use std::io::Read;
    let agent = agent();
    for base in SOURCES {
        if let Ok(r) = agent.get(&format!("{base}{code}.png")).call() {
            let mut buf = Vec::new();
            if r.into_reader().take(1024 * 1024).read_to_end(&mut buf).is_ok() && buf.starts_with(b"\x89PNG") {
                return Some(buf);
            }
        }
    }
    None
}

/// Texture de l'émoji si elle est prête ; lance sinon son chargement (disque, puis réseau).
pub fn texture(ctx: &egui::Context, emoji: &str) -> Option<TextureHandle> {
    let code = file_code(emoji);
    if code.is_empty() {
        return None;
    }
    let mut map = cache().lock().ok()?;
    match map.get(&code) {
        Some(Slot::Texture(t)) => return Some(t.clone()),
        Some(Slot::Ready(img)) => {
            let t = ctx.load_texture(format!("emoji_{code}"), img.clone(), egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR });
            map.insert(code, Slot::Texture(t.clone()));
            return Some(t);
        }
        Some(Slot::Loading) | Some(Slot::Missing) => return None,
        None => {}
    }
    if IN_FLIGHT.load(Ordering::Relaxed) >= MAX_DOWNLOADS {
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
        return None;
    }
    IN_FLIGHT.fetch_add(1, Ordering::Relaxed);
    map.insert(code.clone(), Slot::Loading);
    drop(map);
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        let path = disk_dir().map(|d| d.join(format!("{code}.png")));
        let bytes = path.as_ref().and_then(|p| std::fs::read(p).ok()).or_else(|| {
            let b = fetch(&code)?;
            if let Some(p) = &path {
                let _ = std::fs::create_dir_all(p.parent().unwrap_or(p));
                let _ = std::fs::write(p, &b);
            }
            Some(b)
        });
        let slot = bytes.as_deref().and_then(decode).map(Slot::Ready).unwrap_or(Slot::Missing);
        if let Ok(mut map) = cache().lock() {
            map.insert(code, slot);
        }
        IN_FLIGHT.fetch_sub(1, Ordering::Relaxed);
        ctx.request_repaint();
    });
    None
}

/// Télécharge à l'avance (en arrière-plan, 6 à la fois) les émojis absents du disque :
/// ils apparaissent ensuite instantanément.
pub fn prefetch(list: Vec<String>) {
    let Some(dir) = disk_dir() else { return };
    let todo: Vec<String> =
        list.iter().map(|e| file_code(e)).filter(|c| !c.is_empty() && !dir.join(format!("{c}.png")).exists()).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
    if todo.is_empty() {
        return;
    }
    let _ = std::fs::create_dir_all(&dir);
    let queue = Arc::new(Mutex::new(todo));
    for _ in 0..6 {
        let (queue, dir) = (queue.clone(), dir.clone());
        std::thread::spawn(move || {
            while let Some(code) = queue.lock().ok().and_then(|mut q| q.pop()) {
                if let Some(b) = fetch(&code) {
                    let _ = std::fs::write(dir.join(format!("{code}.png")), b);
                }
            }
        });
    }
}

/// Dessine un émoji dans `rect` : image couleur si disponible, sinon le glyphe de la police.
pub fn paint(p: &egui::Painter, rect: Rect, emoji: &str, fallback: Color32) {
    match texture(p.ctx(), emoji) {
        Some(t) => {
            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
            p.image(t.id(), rect, uv, Color32::WHITE);
        }
        None => {
            p.text(rect.center(), Align2::CENTER_CENTER, emoji, FontId::proportional(rect.height() * 0.82), fallback);
        }
    }
}

/// Bouton carré contenant un émoji (sélecteur, réactions rapides).
pub fn button(ui: &mut egui::Ui, emoji: &str, size: f32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, egui::CornerRadius::same(6), crate::theme::SURFACE_HI());
    }
    let inner = size * 0.72;
    paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(inner, inner)), emoji, crate::theme::TEXT());
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Repère les émojis d'un texte : (index du premier caractère, nombre de caractères, émoji).
pub fn find(text: &str) -> Vec<(usize, usize, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i] as u32;
        // Raccourci : ignore l'ASCII et les lettres courantes (sauf # * 0-9 suivis d'un keycap).
        let maybe = c >= 0x2000 || (i + 1 < chars.len() && (chars[i + 1] == '\u{20E3}' || chars[i + 1] == '\u{FE0F}'));
        if maybe {
            let mut found = None;
            for len in (1..=chars.len().saturating_sub(i).min(12)).rev() {
                let s: String = chars[i..i + len].iter().collect();
                if emojis::get(&s).is_some() {
                    found = Some((len, s));
                    break;
                }
            }
            if let Some((len, s)) = found {
                out.push((i, len, s));
                i += len;
                continue;
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_names_map_back_to_emojis() {
        for e in ["😀", "❤️", "👍", "🏳️‍🌈", "#️⃣"] {
            assert_eq!(emoji_of_code(&file_code(e)).as_deref(), Some(e), "{e}");
        }
        assert!(emoji_of_code("zzzz").is_none());
    }

    #[test]
    fn file_codes_and_detection() {
        assert_eq!(file_code("😀"), "1f600");
        assert_eq!(file_code("❤️"), "2764");
        assert_eq!(file_code("#️⃣"), "0023_20e3");
        let f = find("Salut 👍 et ❤️ !");
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].2, "👍");
        assert!(find("bonjour 2024").is_empty());
    }
}

// --- Émojis couleur partout ------------------------------------------------------------------

/// Peut-il y avoir un émoji dans ce texte ? (test rapide avant l'analyse complète)
fn maybe_emoji(text: &str) -> bool {
    text.chars().any(|c| {
        let u = c as u32;
        u >= 0x1F000 || (0x2190..=0x2BFF).contains(&u) || u == 0x00A9 || u == 0x00AE || u == 0x203C || u == 0x2049 || (0x3030..=0x303D).contains(&u)
    })
}

/// Émojis d'un texte dessiné : (glyphes concernés par ligne, émoji).
fn emoji_runs(galley: &egui::Galley) -> Vec<(Vec<(usize, usize)>, String)> {
    // Texte reconstitué à partir des glyphes (le « \n » n'a pas de glyphe).
    let mut chars = Vec::new();
    for (ri, row) in galley.rows.iter().enumerate() {
        for gi in 0..row.glyphs.len() {
            chars.push((ri, gi, row.glyphs[gi].chr));
        }
    }
    let text: String = chars.iter().map(|c| c.2).collect();
    if !maybe_emoji(&text) {
        return Vec::new();
    }
    find(&text).into_iter().map(|(i, n, e)| (chars[i..(i + n).min(chars.len())].iter().map(|(r, g, _)| (*r, *g)).collect(), e)).collect()
}

fn collect(shape: &egui::Shape, out: &mut Vec<String>) {
    match shape {
        egui::Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
        egui::Shape::Text(t) if t.angle == 0.0 && maybe_emoji(&t.galley.job.text) => {
            out.extend(emoji_runs(&t.galley).into_iter().map(|(_, e)| e));
        }
        _ => {}
    }
}

/// Remplace le glyphe monochrome par l'image couleur ; renvoie les images à dessiner par-dessus.
fn colorize(shape: &mut egui::Shape, tex: &HashMap<String, egui::TextureId>) -> Vec<egui::Shape> {
    match shape {
        egui::Shape::Vec(v) => {
            let mut extra = Vec::new();
            for s in v.iter_mut() {
                extra.extend(colorize(s, tex));
            }
            v.extend(extra);
            Vec::new()
        }
        egui::Shape::Text(t) if t.angle == 0.0 && maybe_emoji(&t.galley.job.text) => {
            let runs = emoji_runs(&t.galley);
            let mut images = Vec::new();
            let mut hide: Vec<(usize, usize)> = Vec::new();
            for (glyphs, e) in runs {
                let Some(id) = tex.get(&e) else { continue };
                let Some(&(r0, g0)) = glyphs.first() else { continue };
                let row = &t.galley.rows[r0];
                let first = &row.glyphs[g0];
                // Émoji déjà dessiné en image par l'onglet (glyphe transparent) : rien à faire.
                let color =
                    if first.uv_rect.is_nothing() { Color32::WHITE } else { row.visuals.mesh.vertices.get(first.first_vertex as usize).map(|v| v.color).unwrap_or(Color32::WHITE) };
                if color.a() == 0 || t.override_text_color.is_some_and(|c| c.a() == 0) {
                    continue;
                }
                let mut rect = first.logical_rect();
                for &(r, g) in glyphs.iter().filter(|(r, _)| *r == r0) {
                    let gl = &t.galley.rows[r].glyphs[g];
                    rect = rect.union(gl.logical_rect());
                }
                let rect = rect.translate(t.pos.to_vec2() + row.pos.to_vec2());
                // Taille du glyphe remplacé (sans déborder sur les lettres voisines).
                let s = (first.font_height.max(4.0) * 1.08).min(rect.width().max(4.0) * 1.12);
                let r = Rect::from_center_size(rect.center(), vec2(s, s));
                let tint = Color32::WHITE.gamma_multiply(t.opacity_factor);
                images.push(egui::Shape::image(*id, r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), tint));
                hide.extend(glyphs);
            }
            if !hide.is_empty() {
                let galley = std::sync::Arc::make_mut(&mut t.galley);
                for (r, g) in hide {
                    let placed = &mut galley.rows[r];
                    let row = std::sync::Arc::make_mut(&mut placed.row);
                    let gl = &row.glyphs[g];
                    if gl.uv_rect.is_nothing() {
                        continue;
                    }
                    let v0 = gl.first_vertex as usize;
                    for v in row.visuals.mesh.vertices.iter_mut().skip(v0).take(4) {
                        v.color = Color32::TRANSPARENT;
                    }
                }
            }
            images
        }
        _ => Vec::new(),
    }
}

/// À appeler une fois par image, après tout le dessin : chaque émoji affiché par l'application
/// (titres, listes, champs de saisie, notes, mails, agenda…) devient l'image couleur du chat.
pub fn colorize_frame(ctx: &egui::Context) {
    let mut layers: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    if !layers.contains(&egui::LayerId::background()) {
        layers.push(egui::LayerId::background());
    }
    // 1. Émojis présents à l'écran.
    let mut found = Vec::new();
    ctx.graphics(|g| {
        for l in &layers {
            if let Some(list) = g.get(*l) {
                for cs in list.all_entries() {
                    collect(&cs.shape, &mut found);
                }
            }
        }
    });
    if found.is_empty() {
        return;
    }
    found.sort();
    found.dedup();
    // 2. Textures (chargées en arrière-plan si besoin : le glyphe reste affiché en attendant).
    let tex: HashMap<String, egui::TextureId> = found.into_iter().filter_map(|e| texture(ctx, &e).map(|t| (e, t.id()))).collect();
    if tex.is_empty() {
        return;
    }
    // 3. Remplacement, à la même place dans l'ordre de dessin.
    ctx.graphics_mut(|g| {
        for l in &layers {
            let Some(list) = g.get_mut(*l) else { continue };
            for i in 0..list.next_idx().0 {
                list.mutate_shape(egui::layers::ShapeIdx(i), |cs| {
                    let extra = colorize(&mut cs.shape, &tex);
                    if !extra.is_empty() {
                        let old = std::mem::replace(&mut cs.shape, egui::Shape::Noop);
                        let mut v = vec![old];
                        v.extend(extra);
                        cs.shape = egui::Shape::Vec(v);
                    }
                });
            }
        }
    });
}
