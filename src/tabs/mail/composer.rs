//! Éditeur de mail : champs, barre d'outils (taille, gras, alignement, image, pièce jointe),
//! texte mis en forme avec images à leur place, pièces jointes.
//!
//! Le corps est une suite de paragraphes (chacun avec sa taille et son alignement)
//! et d'images. Entrée crée un nouveau paragraphe, Retour arrière en début de
//! paragraphe le fusionne avec le précédent (ou supprime l'image juste avant).

use std::collections::HashMap;
use std::sync::Arc;

use eframe::egui::text::{CCursor, CCursorRange, LayoutJob, TextFormat};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Align as EAlign, Color32, CornerRadius, Key, Layout, Modifiers, Rect, RichText, Sense, Stroke, TextureHandle, vec2};

use super::attachments::human_size;
use super::contacts::Contacts;
use super::{inline, recipients};
use crate::google::compose::{self, Align, Block, Draft, FileData, Mode, Size};
use crate::google::gmail::Signature;
use crate::{theme, widgets};

#[derive(Default)]
pub struct Editor {
    /// Paragraphe actif (index dans `blocks`) et sélection (caractères).
    focus: usize,
    sel: (usize, usize),
    /// Focus + sélection à appliquer à la prochaine frame.
    pending: Option<(usize, usize, usize)>,
    textures: HashMap<u64, TextureHandle>,
    /// Suggestions de l'annuaire dans les champs destinataires.
    suggest: recipients::Suggest,
    /// Signature par défaut déjà appliquée à ce brouillon.
    pub sig_applied: bool,
    /// Sélecteur d'émojis ouvert (position) et son état.
    picker: Option<egui::Pos2>,
    emoji_query: String,
    emoji_group: usize,
}

impl Editor {
    pub fn reset(&mut self) {
        *self = Editor { pending: Some((0, 0, 0)), ..Default::default() };
    }
}

pub enum Out {
    Send,
    Cancel,
    Error(String),
}

enum Op {
    Split(usize, usize),
    MergeBack(usize),
    RemoveImage(usize),
}

pub fn show(
    ui: &mut egui::Ui,
    d: &mut Draft,
    ed: &mut Editor,
    sending: bool,
    book: &Contacts,
    signatures: &[Signature],
    signature_preview: &mut dyn FnMut(&mut egui::Ui),
) -> Option<Out> {
    // Signature par défaut (la première : celle de Gmail), comme à l'ouverture d'un brouillon Gmail.
    if !ed.sig_applied && !signatures.is_empty() {
        d.signature = Some(signatures[0].html.clone());
        ed.sig_applied = true;
    }
    let mut out = None;
    let title = match d.mode {
        Mode::New => "Nouveau message",
        Mode::Reply => "Répondre",
        Mode::Forward => "Transférer",
    };
    ui.label(RichText::new(title).font(theme::title_font(20.0)));
    ui.add_space(12.0);
    recipients::field(ui, "À", &mut d.to, "to", "Nom ou adresse…", book, &mut ed.suggest);
    recipients::field(ui, "Cc", &mut d.cc, "cc", "", book, &mut ed.suggest);
    widgets::field(ui, "Objet", 52.0, egui::TextEdit::singleline(&mut d.subject));
    ui.add_space(8.0);

    ed.focus = nearest_para(d, ed.focus);
    if let Some(e) = toolbar(ui, d, ed, signatures) {
        out = Some(Out::Error(e));
    }
    emoji_picker(ui, d, ed);
    if let Some(e) = dropped_files(ui, d, ed) {
        out = Some(Out::Error(e));
    }

    // Pièces jointes du message.
    if !d.files.is_empty() {
        ui.add_space(6.0);
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for (i, f) in d.files.iter().enumerate() {
                file_chip(ui, f, || remove = Some(i));
            }
        });
        if let Some(i) = remove {
            d.files.remove(i);
        }
    }
    ui.add_space(8.0);

    // Zone de texte.
    let bottom = 44.0;
    let height = (ui.available_height() - bottom).max(140.0);
    egui::Frame::new().fill(theme::BG()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(8)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("compose_body").max_height(height - 26.0).auto_shrink([false, false]).show(ui, |ui| {
            // Signature : rendu fidèle, non modifiable ici (elle vient de Gmail).
            body(ui, d, ed, &mut |ui, d| {
                if d.signature.is_some() {
                    ui.add_space(10.0);
                    ui.label(RichText::new("--").color(theme::TEXT_FAINT()));
                    signature_preview(ui);
                }
            });
        });
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let label = if sending { "Envoi…" } else { "Envoyer" };
        if ui.add_enabled(!sending, egui::Button::new(label)).clicked() {
            out = Some(Out::Send);
        }
        if ui.add_enabled(!sending, egui::Button::new("Annuler")).clicked() {
            out = Some(Out::Cancel);
        }
        if sending {
            ui.add(egui::Spinner::new().size(14.0));
        }
        let total = d.total_size();
        if total > 0 {
            ui.with_layout(Layout::right_to_left(EAlign::Center), |ui| {
                ui.label(RichText::new(format!("{} / 25 Mo", human_size(total as u64))).small().color(theme::TEXT_FAINT()));
            });
        }
    });
    out
}

// --- Barre d'outils --------------------------------------------------------

fn toolbar(ui: &mut egui::Ui, d: &mut Draft, ed: &mut Editor, signatures: &[Signature]) -> Option<String> {
    let mut error = None;
    let focus = ed.focus;
    let (cur_size, cur_align) = match d.blocks.get(focus) {
        Some(Block::Para(p)) => (p.size, p.align),
        _ => (Size::Normal, Align::Left),
    };

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;

        // Taille
        egui::ComboBox::from_id_salt("compose_size")
            .selected_text(cur_size.label())
            .width(84.0)
            .show_ui(ui, |ui| {
                for s in Size::ALL {
                    if ui.selectable_label(s == cur_size, RichText::new(s.label()).size(s.px().min(18.0))).clicked() {
                        if let Some(Block::Para(p)) = d.blocks.get_mut(focus) {
                            p.size = s;
                        }
                        ed.pending = Some((focus, ed.sel.0, ed.sel.1));
                    }
                }
            })
            .response
            .on_hover_text("Taille du texte (paragraphe)");

        // Gras
        let b = ui.add(egui::Button::new(RichText::new("B").font(theme::editor_font(14.0, true))).min_size(vec2(28.0, 24.0))).on_hover_text("Gras (sélectionne du texte)");
        if b.clicked()
            && let Some(Block::Para(p)) = d.blocks.get_mut(focus)
        {
            let (a, z) = ed.sel;
            let sel = compose::toggle_bold(&mut p.text, a, z);
            ed.pending = Some((focus, sel.0, sel.1));
        }

        // Émojis (même sélecteur que Google Chat ; « :nom: » marche aussi en tapant).
        let er = ui.add(egui::Button::new(RichText::new("🙂")).min_size(vec2(28.0, 24.0))).on_hover_text("Émojis (ou tape « :nom: »)");
        if er.clicked() {
            ed.picker = if ed.picker.is_some() { None } else { Some(er.rect.left_bottom() + vec2(0.0, 4.0)) };
        }

        ui.add_space(6.0);
        let mut hl = crate::liquid::Liquid::begin(ui, "compose_align");
        for a in Align::ALL {
            if align_button(ui, a, a == cur_align, &mut hl).clicked() {
                if let Some(Block::Para(p)) = d.blocks.get_mut(focus) {
                    p.align = a;
                }
                ed.pending = Some((focus, ed.sel.0, ed.sel.1));
            }
        }

        hl.end(ui, theme::TEXT(), 6.0);

        ui.add_space(6.0);
        signature_menu(ui, d, signatures);
        if ui.button("Image").on_hover_text("Insérer une image à l'emplacement du curseur").clicked()
            && let Some(paths) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_files()
        {
            for path in paths {
                if let Err(e) = insert_image_file(d, ed, &path) {
                    error = Some(e);
                }
            }
        }
        if ui.button("Pièce jointe").clicked()
            && let Some(paths) = rfd::FileDialog::new().pick_files()
        {
            for path in paths {
                if let Err(e) = attach_file(d, &path) {
                    error = Some(e);
                }
            }
        }
    });
    error
}

/// Choix de la signature : celles de Gmail / des mails envoyés, ou aucune.
fn signature_menu(ui: &mut egui::Ui, d: &mut Draft, signatures: &[Signature]) {
    let current = signatures.iter().position(|s| Some(&s.html) == d.signature.as_ref());
    let label = match (current, &d.signature) {
        (_, None) => "Signature : aucune".to_string(),
        (Some(_), _) if signatures.len() == 1 => "Signature".to_string(),
        (Some(i), _) => format!("Signature {}", i + 1),
        (None, Some(_)) => "Signature".to_string(),
    };
    egui::ComboBox::from_id_salt("compose_signature")
        .selected_text(label)
        .width(120.0)
        .show_ui(ui, |ui| {
            ui.set_min_width(260.0);
            if signatures.is_empty() {
                ui.label(RichText::new("Aucune signature trouvée (réglages Gmail ou mails envoyés)").small().color(theme::TEXT_MUTED()));
            }
            for s in signatures {
                if ui.selectable_label(d.signature.as_ref() == Some(&s.html), &s.label).clicked() {
                    d.signature = Some(s.html.clone());
                }
            }
            if ui.selectable_label(d.signature.is_none(), "Aucune signature").clicked() {
                d.signature = None;
            }
        })
        .response
        .on_hover_text("Signature insérée au format Gmail");
}

fn align_button(ui: &mut egui::Ui, a: Align, active: bool, hl: &mut crate::liquid::Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(28.0, 24.0), Sense::click());
    let resp = resp.on_hover_text(a.label()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if active {
        hl.target(rect, a.label());
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), theme::SURFACE_HI());
    }
    let c = crate::sidebar::lerp_color(theme::TEXT_MUTED(), theme::BG(), hl.coverage(rect));
    let r = rect.shrink2(vec2(7.0, 6.0));
    for (i, w) in [1.0, 0.6, 0.85, 0.5].iter().enumerate() {
        let w = if a == Align::Justify { 1.0 } else { *w } * r.width();
        let y = r.top() + i as f32 * r.height() / 3.0;
        let x0 = match a {
            Align::Left | Align::Justify => r.left(),
            Align::Center => r.center().x - w / 2.0,
            Align::Right => r.right() - w,
        };
        p.hline(x0..=x0 + w, y, Stroke::new(1.5, c));
    }
    resp
}

fn file_chip(ui: &mut egui::Ui, f: &FileData, mut on_remove: impl FnMut()) {
    egui::Frame::new()
        .fill(theme::SURFACE_HI())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(egui::Margin { left: 10, right: 4, top: 2, bottom: 2 })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(&f.name).size(13.0));
            ui.label(RichText::new(human_size(f.data.len() as u64)).small().color(theme::TEXT_FAINT()));
            if ui.small_button("×").on_hover_text("Retirer").clicked() {
                on_remove();
            }
        });
}

// --- Fichiers --------------------------------------------------------------

fn read(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "fichier".into());
    let data = std::fs::read(path).map_err(|e| format!("Lecture de « {name} » impossible : {e}"))?;
    Ok((name, data))
}

fn insert_image_file(d: &mut Draft, ed: &mut Editor, path: &std::path::Path) -> Result<(), String> {
    let (name, data) = read(path)?;
    let preview = inline::decode(&data).map_err(|_| format!("« {name} » n'est pas une image lisible"))?;
    let mime = compose::mime_for(&name).to_string();
    let pos = ed.sel.0.max(ed.sel.1);
    let next = d.insert_image(ed.focus, pos, name, mime, data, Some(preview));
    ed.focus = next;
    ed.sel = (0, 0);
    ed.pending = Some((next, 0, 0));
    Ok(())
}

fn attach_file(d: &mut Draft, path: &std::path::Path) -> Result<(), String> {
    let (name, data) = read(path)?;
    let mime = compose::mime_for(&name).to_string();
    d.files.push(FileData { name, mime, data: Arc::new(data) });
    Ok(())
}

/// Glisser-déposer : images insérées dans le texte, autres fichiers en pièces jointes.
fn dropped_files(ui: &mut egui::Ui, d: &mut Draft, ed: &mut Editor) -> Option<String> {
    let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
    let mut error = None;
    for f in dropped {
        let path = f.path().to_path_buf();
        if path.as_os_str().is_empty() {
            continue;
        }
        let is_image = compose::mime_for(&path.to_string_lossy()).starts_with("image/");
        let r = if is_image { insert_image_file(d, ed, &path) } else { attach_file(d, &path) };
        if let Err(e) = r {
            error = Some(e);
        }
    }
    error
}

// --- Corps -----------------------------------------------------------------

fn nearest_para(d: &Draft, i: usize) -> usize {
    let i = i.min(d.blocks.len().saturating_sub(1));
    (0..=i).rev().chain(i..d.blocks.len()).find(|&j| matches!(d.blocks[j], Block::Para(_))).unwrap_or(0)
}

fn para_id(id: u64) -> egui::Id {
    egui::Id::unique(("wt_compose_para", id))
}

fn body(ui: &mut egui::Ui, d: &mut Draft, ed: &mut Editor, after: &mut dyn FnMut(&mut egui::Ui, &Draft)) {
    let ctx = ui.ctx().clone();
    let mut ops = Vec::new();
    let mut last_align = Align::Left;
    ui.spacing_mut().item_spacing.y = 2.0;
    let single = d.blocks.len() == 1;

    for i in 0..d.blocks.len() {
        match &mut d.blocks[i] {
            Block::Para(p) => {
                last_align = p.align;
                let id = para_id(p.id);

                // Focus / sélection demandés (après Entrée, insertion d'image, barre d'outils…).
                if let Some((bi, a, b)) = ed.pending.filter(|(bi, ..)| *bi == i) {
                    let _ = bi;
                    let mut st = TextEditState::load(&ctx, id).unwrap_or_default();
                    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                    st.store(&ctx, id);
                    ctx.memory_mut(|m| m.request_focus(id));
                    ed.pending = None;
                }

                // Raccourcis gérés avant le champ (il ne doit pas les recevoir).
                if ctx.memory(|m| m.has_focus(id)) {
                    let cur = TextEditState::load(&ctx, id).and_then(|s| s.cursor.char_range()).map(|r| (r.primary.index.0, r.secondary.index.0));
                    if let Some((a, b)) = cur {
                        if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Enter)) {
                            ops.push(Op::Split(i, a.min(b)));
                        } else if a == 0 && b == 0 && i > 0 && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Backspace)) {
                            ops.push(Op::MergeBack(i));
                        }
                    }
                }

                let (size, align) = (p.size, p.align);
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| ui.painter().layout_job(layout(buf.as_str(), size, align, wrap));
                let hint = if single { "Écris ton message…" } else { "" };
                let output = egui::TextEdit::multiline(&mut p.text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .align(egui::Align2([to_egui_align(align), EAlign::Min]))
                    .layouter(&mut layouter)
                    .show(ui);
                if output.response.has_focus() {
                    ed.focus = i;
                    if let Some(r) = output.cursor_range {
                        ed.sel = (r.primary.index.0, r.secondary.index.0);
                    }
                }
                // « :nom: » complet → émoji.
                if output.response.changed() {
                    let cur = output.cursor_range.map(|r| r.primary.index.0).unwrap_or(p.text.chars().count());
                    if let Some((t, c)) = crate::tabs::messages::emoji::replace_codes_at(&p.text, cur) {
                        p.text = t;
                        ed.pending = Some((i, c, c));
                    }
                }
            }
            Block::Image(img) => {
                let tex = img
                    .preview
                    .as_ref()
                    .map(|pv| ed.textures.entry(img.id).or_insert_with(|| ctx.load_texture(format!("compose_{}", img.id), pv.clone(), egui::TextureOptions::LINEAR)).clone());
                let layout = match last_align {
                    Align::Center => Layout::top_down(EAlign::Center),
                    Align::Right => Layout::top_down(EAlign::Max),
                    _ => Layout::top_down(EAlign::Min),
                };
                ui.with_layout(layout, |ui| {
                    let resp = match (&tex, &img.preview) {
                        (Some(t), Some(pv)) => {
                            let [w, h] = pv.size;
                            let scale = (ui.available_width() / w as f32).min(1.0);
                            ui.add(egui::Image::new((t.id(), vec2(w as f32 * scale, h as f32 * scale))).sense(Sense::hover()))
                        }
                        _ => ui.label(format!("[image : {}]", img.name)),
                    };
                    // Bouton de suppression au survol.
                    if resp.hovered() || ui.rect_contains_pointer(resp.rect) {
                        let r = Rect::from_min_size(resp.rect.right_top() + vec2(-30.0, 6.0), vec2(24.0, 24.0));
                        let del = ui.put(r, egui::Button::new("×").fill(Color32::from_black_alpha(200)).corner_radius(CornerRadius::same(255)));
                        if del.on_hover_text("Retirer l'image").clicked() {
                            ops.push(Op::RemoveImage(i));
                        }
                    }
                });
            }
        }
    }

    // Contenu ajouté sous le texte (signature), avant la zone de clic.
    after(ui, d);

    // Clic sous le texte → curseur à la fin du dernier paragraphe.
    let rest = ui.available_size_before_wrap();
    let (_, resp) = ui.allocate_exact_size(vec2(rest.x, rest.y.max(40.0)), Sense::click());
    if resp.clicked() {
        let last = nearest_para(d, d.blocks.len().saturating_sub(1));
        if let Some(Block::Para(p)) = d.blocks.get(last) {
            let n = p.text.chars().count();
            ed.pending = Some((last, n, n));
        }
    }

    for op in ops {
        apply(d, ed, op);
    }
}

fn apply(d: &mut Draft, ed: &mut Editor, op: Op) {
    match op {
        Op::Split(i, pos) => {
            let Some(Block::Para(p)) = d.blocks.get_mut(i) else { return };
            let tail = p.text.split_off(compose::char_to_byte(&p.text, pos));
            let (size, align) = (p.size, p.align);
            let mut next = d.paragraph(tail);
            next.size = size;
            next.align = align;
            d.blocks.insert(i + 1, Block::Para(next));
            ed.pending = Some((i + 1, 0, 0));
        }
        Op::MergeBack(i) => match d.blocks.get(i - 1) {
            Some(Block::Para(_)) => merge(d, ed, i - 1),
            Some(Block::Image(_)) => {
                d.blocks.remove(i - 1);
                if i >= 2 && matches!(d.blocks.get(i - 2), Some(Block::Para(_))) {
                    merge(d, ed, i - 2);
                } else {
                    ed.pending = Some((i - 1, 0, 0));
                }
            }
            None => {}
        },
        Op::RemoveImage(i) => {
            d.blocks.remove(i);
            if i >= 1 && matches!(d.blocks.get(i - 1), Some(Block::Para(_))) && matches!(d.blocks.get(i), Some(Block::Para(_))) {
                merge(d, ed, i - 1);
            }
        }
    }
}

/// Fusionne le paragraphe `i + 1` dans le paragraphe `i`.
fn merge(d: &mut Draft, ed: &mut Editor, i: usize) {
    let Some(Block::Para(next)) = d.blocks.get(i + 1).cloned() else { return };
    if let Some(Block::Para(p)) = d.blocks.get_mut(i) {
        let n = p.text.chars().count();
        p.text.push_str(&next.text);
        d.blocks.remove(i + 1);
        ed.pending = Some((i, n, n));
    }
}

fn to_egui_align(a: Align) -> EAlign {
    match a {
        Align::Left | Align::Justify => EAlign::Min,
        Align::Center => EAlign::Center,
        Align::Right => EAlign::Max,
    }
}

/// Mise en forme d'un paragraphe : taille, alignement, gras (marqueurs `**` estompés).
fn layout(text: &str, size: Size, align: Align, wrap: f32) -> LayoutJob {
    let px = size.px();
    let mut job = LayoutJob::default();
    for (seg, bold, marker) in compose::bold_spans(text) {
        let format =
            if marker { TextFormat::simple(theme::editor_font(px * 0.75, false), theme::TEXT_FAINT()) } else { TextFormat::simple(theme::editor_font(px, bold), theme::TEXT()) };
        job.append(seg, 0.0, format);
    }
    if text.is_empty() {
        job.append("", 0.0, TextFormat::simple(theme::editor_font(px, false), theme::TEXT()));
    }
    job.wrap.max_width = wrap;
    job.halign = to_egui_align(align);
    job.justify = align == Align::Justify;
    job
}

/// Sélecteur d'émojis (identique à Google Chat) : insère l'émoji au curseur du paragraphe actif.
fn emoji_picker(ui: &mut egui::Ui, d: &mut Draft, ed: &mut Editor) {
    use crate::tabs::messages::emoji;
    let Some(pos) = ed.picker else { return };
    let recent = emoji::recent();
    let (mut q, mut g) = (std::mem::take(&mut ed.emoji_query), ed.emoji_group);
    let mut picked = None;
    let area = egui::Area::new(egui::Id::unique("wt_compose_emoji_picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            picked = emoji::picker(ui, &mut q, &mut g, &recent);
        });
    });
    ed.emoji_query = q;
    ed.emoji_group = g;
    let outside = ui.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !Rect::from_center_size(pos - vec2(0.0, 18.0), vec2(40.0, 36.0)).contains(p))
    });
    if let Some(e) = &picked {
        if let Some(Block::Para(p)) = d.blocks.get_mut(ed.focus) {
            let n = p.text.chars().count();
            let (a, b) = (ed.sel.0.min(ed.sel.1).min(n), ed.sel.0.max(ed.sel.1).min(n));
            let ba = p.text.char_indices().nth(a).map(|(i, _)| i).unwrap_or(p.text.len());
            let bb = p.text.char_indices().nth(b).map(|(i, _)| i).unwrap_or(p.text.len());
            p.text.replace_range(ba..bb, e);
            let c = a + e.chars().count();
            ed.sel = (c, c);
            ed.pending = Some((ed.focus, c, c));
        }
        emoji::remember(e);
    }
    if picked.is_some() || outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        ed.picker = None;
    }
}
