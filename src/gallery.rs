//! Galerie des onglets personnalisés : clic droit sur le logo de la barre → panneau flottant avec
//! une grille 3 × 3. Une case pleine ouvre son onglet ; une case vide (« + ») en crée un ; clic
//! droit sur une case : renommer, changer le logo (émoji ou image), supprimer.
//!
//! Le contenu d'un onglet (galerie de panneaux) est dans `tabs::custom`.

use std::sync::Mutex;

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::prefs::{CustomTab, Prefs};
use crate::theme;

/// Cases de la grille.
pub const SLOTS: usize = 9;
const TILE: f32 = 84.0;
const GAP: f32 = 10.0;
const ICONS: [&str; 9] = ["📁", "⭐", "🚀", "🎨", "📊", "🧪", "🎮", "📚", "💡"];

/// Panneau ouvert : à côté de quel point, depuis quand (animation), case en cours de renommage,
/// sélecteur d'émojis ouvert pour une case.
#[derive(Default)]
struct State {
    anchor: Option<Pos2>,
    opened_at: f64,
    renaming: Option<(u8, String, bool)>,
    emoji: Option<(u8, Pos2)>,
    emoji_query: String,
    emoji_group: usize,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn state() -> std::sync::MutexGuard<'static, Option<State>> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

/// Clic droit sur le logo : ouvre (ou ferme) la galerie à droite de ce point.
pub fn toggle(ctx: &egui::Context, anchor: Pos2) {
    let mut g = state();
    let st = g.get_or_insert_with(State::default);
    if st.anchor.is_some() {
        *st = State::default();
    } else {
        *st = State { anchor: Some(anchor), opened_at: ctx.input(|i| i.time), ..Default::default() };
    }
    ctx.request_repaint();
}

pub fn is_open() -> bool {
    state().as_ref().is_some_and(|s| s.anchor.is_some())
}

fn close() {
    if let Some(s) = state().as_mut() {
        *s = State::default();
    }
}

/// Onglet personnalisé de cette case.
pub fn tab_at(prefs: &Prefs, slot: u8) -> Option<&CustomTab> {
    prefs.custom_tabs.iter().find(|t| t.slot == slot)
}

pub fn tab(prefs: &Prefs, id: u64) -> Option<&CustomTab> {
    prefs.custom_tabs.iter().find(|t| t.id == id)
}

/// Logo d'un onglet : son image (si elle se lit), sinon son émoji.
pub fn paint_icon(p: &egui::Painter, r: Rect, t: &CustomTab) {
    if let Some(tex) = t.image.as_deref().and_then(|path| image_texture(p.ctx(), path)) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        p.image(tex.id(), r, uv, Color32::WHITE);
        return;
    }
    let e = if t.emoji.is_empty() { "📁" } else { t.emoji.as_str() };
    crate::emoji_img::paint(p, r, e, theme::TEXT());
}

/// Image choisie pour un onglet, chargée une fois (carrée, réduite).
fn image_texture(ctx: &egui::Context, path: &str) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique(("wt_custom_tab_img", path));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let tex = std::fs::read(path).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|img| {
        let s = img.width().min(img.height());
        let img = img.crop_imm((img.width() - s) / 2, (img.height() - s) / 2, s, s).thumbnail(128, 128).to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        ctx.load_texture(format!("custom_tab_{path}"), ci, egui::TextureOptions::LINEAR)
    });
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Dessine la galerie si elle est ouverte ; renvoie l'onglet personnalisé à ouvrir.
pub fn show(ctx: &egui::Context, prefs: &mut Prefs) -> Option<u64> {
    let (anchor, opened_at) = {
        let g = state();
        let st = g.as_ref()?;
        (st.anchor?, st.opened_at)
    };
    let now = ctx.input(|i| i.time);
    // Apparition élastique (comme les aperçus de la barre).
    let t = ((now - opened_at) as f32 / 0.35).clamp(0.0, 1.0);
    let k = 1.0 - (1.0 - t).powi(3);
    let scale = 0.86 + 0.14 * back_out(t);
    if t < 1.0 {
        ctx.request_repaint();
    }
    let mut open_tab = None;
    let mut menu_open = false;
    let w = 3.0 * TILE + 2.0 * GAP + 28.0;
    let pos = Pos2::new(anchor.x, anchor.y - 22.0);
    let area_id = egui::Id::unique("wt_gallery");
    let area = egui::Area::new(area_id).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        ui.multiply_opacity(k);
        // Même carte que la recherche, mais opaque (pas de fond flouté derrière).
        crate::search::overlay_frame().fill(theme::SURFACE()).show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                crate::sidebar::paint_logo(ui.painter(), r, theme::TEXT_MUTED());
                ui.label(RichText::new(t!("ONGLETS PERSONNALISÉS", "CUSTOM TABS")).small().strong().color(theme::TEXT_FAINT()));
            });
            ui.add_space(8.0);
            let (grid, _) = ui.allocate_exact_size(vec2(3.0 * TILE + 2.0 * GAP, 3.0 * TILE + 2.0 * GAP), Sense::hover());
            for slot in 0..SLOTS as u8 {
                let (c, r) = ((slot % 3) as f32, (slot / 3) as f32);
                let rect = Rect::from_min_size(grid.min + vec2(c * (TILE + GAP), r * (TILE + GAP)), vec2(TILE, TILE));
                // Petit décalage d'apparition case par case.
                let lag = ((t - slot as f32 * 0.04) / 0.8).clamp(0.0, 1.0);
                let rect = Rect::from_center_size(rect.center(), rect.size() * (scale * (0.9 + 0.1 * lag)));
                let resp = ui.interact(rect, ui.scope_id().with(("gallery_tile", slot)), Sense::click());
                let hovered = resp.hovered();
                let p = ui.painter();
                match tab_at(prefs, slot).cloned() {
                    Some(tab) => {
                        p.rect_filled(rect, CornerRadius::same(14), if hovered { theme::SURFACE_HI() } else { theme::BG() });
                        p.rect_stroke(rect, CornerRadius::same(14), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
                        paint_icon(p, Rect::from_center_size(rect.center() - vec2(0.0, 10.0), vec2(34.0, 34.0)), &tab);
                        let renaming = state().as_ref().and_then(|s| s.renaming.clone()).filter(|(s, ..)| *s == slot);
                        match renaming {
                            Some((_, mut name, focus)) => {
                                let er = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 14.0), vec2(TILE - 8.0, 20.0));
                                let te =
                                    ui.put(er, egui::TextEdit::singleline(&mut name).font(FontId::proportional(11.5)).align(egui::Align2::CENTER_CENTER).margin(vec2(2.0, 1.0)));
                                if focus {
                                    // Tout le nom sélectionné : la frappe le remplace.
                                    let mut st = egui::text_edit::TextEditState::load(ui.ctx(), te.id).unwrap_or_default();
                                    let n = name.chars().count();
                                    st.cursor.set_char_range(Some(egui::text_selection::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(n))));
                                    st.store(ui.ctx(), te.id);
                                    te.request_focus();
                                }
                                let done = te.lost_focus();
                                if let Some(s) = state().as_mut() {
                                    s.renaming = if done { None } else { Some((slot, name.clone(), false)) };
                                }
                                if done
                                    && !ui.input(|i| i.key_pressed(egui::Key::Escape))
                                    && let Some(t) = prefs.custom_tabs.iter_mut().find(|t| t.slot == slot)
                                    && !name.trim().is_empty()
                                {
                                    t.name = name.trim().to_string();
                                }
                                menu_open = true;
                            }
                            None => {
                                let g = crate::tabs::mail::view::one_line(p, &tab.name, FontId::proportional(11.5), theme::TEXT(), TILE - 10.0);
                                p.galley(Pos2::new(rect.center().x - g.size().x / 2.0, rect.bottom() - 22.0), g, theme::TEXT());
                            }
                        }
                        let resp = resp
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(t!("Ouvrir · clic droit : renommer, logo, supprimer", "Open · right-click: rename, logo, delete"));
                        if resp.clicked() {
                            open_tab = Some(tab.id);
                        }
                        resp.context_menu(|ui| {
                            menu_open = true;
                            tile_menu(ui, prefs, &tab, rect);
                        });
                    }
                    None => {
                        // Case vide : « + » en pointillés.
                        let ink = if hovered { theme::TEXT_MUTED() } else { theme::TEXT_FAINT() };
                        crate::tabs::dash_layout::dashed(p, rect.shrink(1.0), ink);
                        let c = rect.center();
                        p.line_segment([c - vec2(8.0, 0.0), c + vec2(8.0, 0.0)], Stroke::new(1.6, ink));
                        p.line_segment([c - vec2(0.0, 8.0), c + vec2(0.0, 8.0)], Stroke::new(1.6, ink));
                        if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Créer un onglet", "Create a tab")).clicked() {
                            create(prefs, slot);
                            menu_open = true;
                        }
                    }
                }
            }
            ui.add_space(8.0);
            ui.label(
                RichText::new(t!("Clic : ouvrir · « + » : créer · clic droit : modifier · Échap : fermer", "Click: open · “+”: create · right-click: edit · Esc: close"))
                    .small()
                    .color(theme::TEXT_FAINT()),
            );
        });
    });
    // Sélecteur d'émojis d'une case.
    menu_open |= emoji_picker(ctx, prefs);
    // Clic ailleurs ou Échap : fermée (sauf pendant un renommage ou un menu ouvert).
    let editing = state().as_ref().is_some_and(|s| s.renaming.is_some() || s.emoji.is_some());
    // Le logo lui-même n'est pas « ailleurs » (son clic droit referme la galerie).
    let logo = Rect::from_min_size(anchor - vec2(70.0, 0.0), vec2(48.0, 48.0));
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !logo.contains(p)));
    let any_popup = egui::Popup::is_any_open(ctx);
    if open_tab.is_some() || (!menu_open && !editing && !any_popup && (outside || ctx.input(|i| i.key_pressed(egui::Key::Escape)))) {
        close();
    }
    open_tab
}

/// Nouvel onglet dans cette case, nom à saisir aussitôt.
fn create(prefs: &mut Prefs, slot: u8) {
    let id = chrono::Utc::now().timestamp_millis() as u64;
    let n = prefs.custom_tabs.len() + 1;
    let name = tf!("Onglet {n}", "Tab {n}");
    prefs.custom_tabs.push(CustomTab { id, slot, name: name.clone(), emoji: ICONS[slot as usize % ICONS.len()].to_string(), image: None, ..Default::default() });
    if let Some(s) = state().as_mut() {
        s.renaming = Some((slot, name, true));
    }
}

/// Menu d'une case : renommer, émoji, image, supprimer.
fn tile_menu(ui: &mut egui::Ui, prefs: &mut Prefs, tab: &CustomTab, rect: Rect) {
    if ui.button(t!("Renommer", "Rename")).clicked() {
        if let Some(s) = state().as_mut() {
            s.renaming = Some((tab.slot, tab.name.clone(), true));
        }
        ui.close();
    }
    if ui.button(t!("Changer l'émoji…", "Change the emoji…")).clicked() {
        if let Some(s) = state().as_mut() {
            s.emoji = Some((tab.slot, rect.right_top() + vec2(8.0, 0.0)));
        }
        ui.close();
    }
    if ui.button(t!("Choisir une image…", "Choose an image…")).clicked() {
        ui.close();
        if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_file()
            && let Some(saved) = save_image(tab.id, &path)
            && let Some(t) = prefs.custom_tabs.iter_mut().find(|t| t.id == tab.id)
        {
            ui.ctx().data_mut(|d| d.remove::<Option<egui::TextureHandle>>(egui::Id::unique(("wt_custom_tab_img", saved.as_str()))));
            t.image = Some(saved);
        }
    }
    if tab.image.is_some() && ui.button(t!("Retirer l'image", "Remove the image")).clicked() {
        if let Some(t) = prefs.custom_tabs.iter_mut().find(|t| t.id == tab.id) {
            t.image = None;
        }
        ui.close();
    }
    ui.separator();
    if ui.button(RichText::new(t!("Supprimer l'onglet", "Delete the tab")).color(theme::TODAY())).clicked() {
        prefs.custom_tabs.retain(|t| t.id != tab.id);
        ui.close();
    }
}

/// Copie de l'image choisie dans les données de l'app (le fichier d'origine peut bouger).
fn save_image(id: u64, src: &std::path::Path) -> Option<String> {
    let dir = crate::paths::config().join("custom_tabs");
    std::fs::create_dir_all(&dir).ok()?;
    let ext = src.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_else(|| "png".into());
    let dest = dir.join(format!("{id}.{ext}"));
    std::fs::copy(src, &dest).ok()?;
    Some(dest.to_string_lossy().to_string())
}

/// Sélecteur d'émojis ouvert pour une case ; renvoie `true` s'il est affiché.
fn emoji_picker(ctx: &egui::Context, prefs: &mut Prefs) -> bool {
    use crate::tabs::messages::emoji;
    let (slot, pos, mut q, mut g) = {
        let st = state();
        let Some(s) = st.as_ref() else { return false };
        let Some((slot, pos)) = s.emoji else { return false };
        (slot, pos, s.emoji_query.clone(), s.emoji_group)
    };
    let recent = emoji::recent();
    let mut picked = None;
    let area = egui::Area::new(egui::Id::unique("wt_gallery_emoji")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            picked = emoji::picker(ui, &mut q, &mut g, &recent);
        });
    });
    let outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    let done = picked.is_some() || outside || ctx.input(|i| i.key_pressed(egui::Key::Escape));
    if let Some(e) = picked
        && let Some(t) = prefs.custom_tabs.iter_mut().find(|t| t.slot == slot)
    {
        t.emoji = e;
        t.image = None;
    }
    if let Some(s) = state().as_mut() {
        s.emoji_query = q;
        s.emoji_group = g;
        if done {
            s.emoji = None;
        }
    }
    true
}

/// Contenu d'un onglet personnalisé (vide pour l'instant).
/// Élan en sortie : dépasse un peu la cible puis s'y pose.
fn back_out(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}
