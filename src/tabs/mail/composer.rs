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
use crate::theme;

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
    /// État de l'enregistrement du brouillon (« Brouillon enregistré »…), affiché sous le titre.
    pub save_label: Option<String>,
    /// Fenêtre « Lien » ouverte (paragraphe, sélection, texte, adresse, position, lien existant).
    link: Option<LinkEdit>,
    /// Fenêtre ouverte (fond flouté capturé à l'ouverture).
    opened: bool,
    bg: crate::search::Backdrop,
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

/// Lien en cours de création ou de modification.
struct LinkEdit {
    para: usize,
    /// Caractères remplacés par le lien.
    a: usize,
    b: usize,
    label: String,
    url: String,
    pos: egui::Pos2,
    /// Lien déjà là (on peut le retirer).
    existing: bool,
    focus: bool,
}

pub enum Out {
    Send,
    /// Fermer : le brouillon est rangé, pas perdu.
    Cancel,
    /// Supprimer le brouillon.
    Discard,
    Error(String),
}

enum Op {
    Split(usize, usize),
    MergeBack(usize),
    RemoveImage(usize),
}

/// Hauteur réservée au pied de la fenêtre (boutons, raccourcis).
const FOOTER_H: f32 = 46.0;

/// Fenêtre de rédaction, dans le style de la recherche (Ctrl+Espace) et de la fiche contact :
/// fond flouté, carte centrée qui apparaît en fondu. Fermer (✕, Échap, clic à côté) range le
/// brouillon ; « Envoyer » ou Ctrl+Entrée envoie.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    d: &mut Draft,
    ed: &mut Editor,
    sending: bool,
    book: &Contacts,
    signatures: &[Signature],
    signature_preview: &mut dyn FnMut(&mut egui::Ui),
) -> Option<Out> {
    let ctx = ui.ctx().clone();
    // Signature par défaut (la première : celle de Gmail), comme à l'ouverture d'un brouillon Gmail.
    if !ed.sig_applied && !signatures.is_empty() {
        d.signature = Some(signatures[0].html.clone());
        ed.sig_applied = true;
    }
    let card_id = egui::Id::unique("wt_compose_card");
    if !ed.opened {
        ed.opened = true;
        ed.bg.start(&ctx);
    }
    let mut out = None;
    // Fond : capture floutée et voile ; un clic à côté ferme (le brouillon est gardé).
    let popups_open = ed.picker.is_some() || ed.link.is_some() || crate::spell::menu_open(&ctx) || ed.suggest.popup_open();
    // Ctrl+K : lien sur la sélection.
    if ed.link.is_none()
        && ctx.memory(|m| m.focused()).is_some_and(|f| matches!(d.blocks.get(ed.focus), Some(Block::Para(p)) if para_id(p.id) == f))
        && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::K))
    {
        let pos = ctx.pointer_hover_pos().unwrap_or(ctx.content_rect().center());
        open_link(d, ed, pos);
    }
    let (k, backdrop_click) = ed.bg.show_in(&ctx, "wt_compose_backdrop", egui::Order::Foreground);
    if backdrop_click && !sending && !popups_open {
        out = Some(Out::Cancel);
    }
    // Ctrl+Entrée : envoyer.
    if !sending && ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Enter)) {
        out = Some(Out::Send);
    }

    let screen = ctx.content_rect();
    let w = 820.0_f32.min(screen.width() - 48.0);
    let h = (screen.height() * 0.84).min(900.0);
    let top = screen.top() + (screen.height() - h) * 0.42 - (1.0 - k) * 12.0;
    egui::Area::new(card_id).order(egui::Order::Foreground).fixed_pos(egui::pos2(screen.center().x - w / 2.0, top)).show(&ctx, |ui| {
        ui.multiply_opacity(k);
        crate::search::overlay_frame().show(ui, |ui| {
            ui.set_width(w - 28.0);
            ui.set_height(h - 28.0);
            if let Some(o) = header(ui, d, ed, sending) {
                out = Some(o);
            }
            ui.add_space(8.0);
            separator(ui);

            // Destinataires et objet : étiquettes en petites capitales, champs sans cadre.
            ui.add_space(2.0);
            let _ = recipients::field_bare(ui, t!("À", "To"), &mut d.to, "to", t!("Nom ou adresse…", "Name or address…"), book, &mut ed.suggest);
            separator(ui);
            let _ = recipients::field_bare(ui, "Cc", &mut d.cc, "cc", "", book, &mut ed.suggest);
            separator(ui);
            ui.horizontal(|ui| {
                field_label(ui, t!("Objet", "Subject"));
                let o = egui::TextEdit::singleline(&mut d.subject)
                    .hint_text(t!("Objet du message", "Subject of the message"))
                    .font(theme::editor_font(15.0, true))
                    .frame(egui::Frame::NONE)
                    .desired_width(ui.available_width())
                    .show(ui);
                crate::spell::check(ui, &o, &mut d.subject);
            });
            separator(ui);
            ui.add_space(6.0);

            ed.focus = nearest_para(d, ed.focus);
            if let Some(e) = toolbar(ui, d, ed, signatures) {
                out = Some(Out::Error(e));
            }
            emoji_picker(ui, d, ed);
            link_window(ui, d, ed);
            if let Some(e) = dropped_files(ui, d, ed) {
                out = Some(Out::Error(e));
            }
            // Pièces jointes du message.
            if !d.files.is_empty() {
                ui.add_space(6.0);
                ui.label(RichText::new(tf!("PIÈCES JOINTES · {}", "ATTACHMENTS · {}", d.files.len())).small().strong().color(theme::TEXT_FAINT()));
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
            ui.add_space(6.0);
            separator(ui);
            ui.add_space(8.0);

            // Texte du message.
            let height = (ui.available_height() - FOOTER_H).max(120.0);
            egui::ScrollArea::vertical().id_salt("compose_body").max_height(height).min_scrolled_height(height).auto_shrink([false, false]).show(ui, |ui| {
                // Signature : rendu fidèle, non modifiable ici (elle vient de Gmail).
                body(ui, d, ed, &mut |ui, d| {
                    if d.signature.is_some() {
                        ui.add_space(10.0);
                        ui.label(RichText::new("--").color(theme::TEXT_FAINT()));
                        signature_preview(ui);
                    }
                });
            });

            separator(ui);
            ui.add_space(8.0);
            if let Some(o) = footer(ui, d, sending) {
                out = Some(o);
            }
        });
    });
    // La carte reste juste au-dessus de son fond ; ses menus (suggestions, émojis, corrections,
    // listes) s'ouvrent au-dessus d'elle.
    ctx.set_sublayer(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("wt_compose_backdrop")), egui::LayerId::new(egui::Order::Foreground, card_id));
    // Échap : fermer (après les champs, qui le prennent pour leurs suggestions).
    if out.is_none() && !sending && !popups_open && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
        out = Some(Out::Cancel);
    }
    if matches!(out, Some(Out::Cancel | Out::Discard)) {
        ed.bg.clear();
        ed.opened = false;
    }
    out
}

/// En-tête : pastille, titre (nouveau message, réponse, transfert), état du brouillon, ✕.
fn header(ui: &mut egui::Ui, d: &Draft, ed: &Editor, sending: bool) -> Option<Out> {
    let mut out = None;
    let (title, icon) = match d.mode {
        Mode::New => (t!("Nouveau message", "New message"), "✉"),
        Mode::Reply => (t!("Répondre", "Reply"), "↩"),
        Mode::Forward => (t!("Transférer", "Forward"), "➦"),
    };
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 22.0, theme::SURFACE_HI());
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(19.0), theme::TEXT());
        ui.vertical(|ui| {
            ui.add_space(2.0);
            ui.label(RichText::new(title).font(theme::title_font(19.0)).color(theme::TEXT()));
            let sub = match (&ed.save_label, d.mode) {
                (Some(l), _) => l.clone(),
                (None, Mode::New) => t!("Brouillon enregistré automatiquement", "Draft saved automatically").to_string(),
                (None, _) => d.subject.clone(),
            };
            ui.add(egui::Label::new(RichText::new(sub).small().color(theme::TEXT_MUTED())).truncate());
        });
        ui.with_layout(Layout::right_to_left(EAlign::Center), |ui| {
            let close = ui
                .add_enabled(!sending, egui::Button::new(RichText::new("✕").size(14.0)).frame(false).min_size(vec2(30.0, 30.0)).corner_radius(CornerRadius::same(255)))
                .on_hover_text(t!("Fermer (le brouillon est gardé) · Échap", "Close (the draft is kept) · Esc"));
            if close.clicked() {
                out = Some(Out::Cancel);
            }
        });
    });
    out
}

/// Pied : Envoyer (bouton principal), Fermer, Supprimer ; taille du message, raccourcis.
fn footer(ui: &mut egui::Ui, d: &Draft, sending: bool) -> Option<Out> {
    let mut out = None;
    ui.horizontal(|ui| {
        let label = if sending { t!("Envoi…", "Sending…") } else { t!("Envoyer", "Send") };
        let send = egui::Button::new(RichText::new(label).strong().color(Color32::WHITE)).fill(theme::TODAY()).corner_radius(CornerRadius::same(10)).min_size(vec2(104.0, 32.0));
        if ui.add_enabled(!sending, send).on_hover_text(t!("Envoyer · Ctrl+Entrée", "Send · Ctrl+Enter")).clicked() {
            out = Some(Out::Send);
        }
        if sending {
            ui.add(egui::Spinner::new().size(14.0));
        }
        let close = egui::Button::new(t!("Fermer", "Close")).corner_radius(CornerRadius::same(10)).min_size(vec2(0.0, 32.0));
        if ui
            .add_enabled(!sending, close)
            .on_hover_text(t!("Le brouillon est gardé (barre des brouillons, en bas à droite)", "The draft is kept (drafts bar, bottom right)"))
            .clicked()
        {
            out = Some(Out::Cancel);
        }
        let bin = egui::Button::new("🗑").corner_radius(CornerRadius::same(10)).min_size(vec2(32.0, 32.0));
        if ui.add_enabled(!sending, bin).on_hover_text(t!("Supprimer le brouillon", "Delete the draft")).clicked() {
            out = Some(Out::Discard);
        }
        ui.with_layout(Layout::right_to_left(EAlign::Center), |ui| {
            ui.label(RichText::new(t!("Ctrl+Entrée envoyer · Échap fermer", "Ctrl+Enter send · Esc close")).small().color(theme::TEXT_FAINT()));
            let total = d.total_size();
            if total > 0 {
                ui.label(RichText::new(format!("{} / 25 Mo  ·", human_size(total as u64))).small().color(theme::TEXT_FAINT()));
            }
        });
    });
    out
}

/// Étiquette d'un champ (« À », « CC », « OBJET ») : petites capitales, comme les groupes de la
/// recherche.
pub fn field_label(ui: &mut egui::Ui, label: &str) {
    ui.add_sized([58.0, 30.0], egui::Label::new(RichText::new(label.to_uppercase()).small().strong().color(theme::TEXT_FAINT())));
}

fn separator(ui: &mut egui::Ui) {
    let y = ui.cursor().top();
    ui.painter().hline(ui.max_rect().x_range(), y, Stroke::new(1.0, theme::BORDER()));
    ui.add_space(1.0);
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
            .on_hover_text(t!("Taille du texte (paragraphe)", "Text size (paragraph)"));

        // Gras
        let b = ui
            .add(egui::Button::new(RichText::new("B").font(theme::editor_font(14.0, true))).min_size(vec2(28.0, 24.0)))
            .on_hover_text(t!("Gras (sélectionne du texte)", "Bold (select some text)"));
        if b.clicked()
            && let Some(Block::Para(p)) = d.blocks.get_mut(focus)
        {
            let (a, z) = ed.sel;
            let sel = compose::toggle_bold(&mut p.text, a, z);
            ed.pending = Some((focus, sel.0, sel.1));
        }

        // Lien sur la sélection (ou le lien sous le curseur).
        let lr = ui
            .add(egui::Button::new(RichText::new("🔗")).min_size(vec2(28.0, 24.0)))
            .on_hover_text(t!("Lien (Ctrl+K) : sélectionne un mot ou une phrase", "Link (Ctrl+K): select a word or a phrase"));
        if lr.clicked() {
            open_link(d, ed, lr.rect.left_bottom() + vec2(0.0, 4.0));
        }

        // Émojis (même sélecteur que Google Chat ; « :nom: » marche aussi en tapant).
        let er = ui.add(egui::Button::new(RichText::new("🙂")).min_size(vec2(28.0, 24.0))).on_hover_text(t!("Émojis (ou tape « :nom: »)", "Emoji (or type “:name:”)"));
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
        if ui.button("Image").on_hover_text(t!("Insérer une image à l'emplacement du curseur", "Insert an image at the cursor")).clicked()
            && let Some(paths) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_files()
        {
            for path in paths {
                if let Err(e) = insert_image_file(d, ed, &path) {
                    error = Some(e);
                }
            }
        }
        if ui.button(t!("Pièce jointe", "Attachment")).clicked()
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
        (_, None) => t!("Signature : aucune", "Signature: none").to_string(),
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
                ui.label(
                    RichText::new(t!("Aucune signature trouvée (réglages Gmail ou mails envoyés)", "No signature found (Gmail settings or sent mails)"))
                        .small()
                        .color(theme::TEXT_MUTED()),
                );
            }
            for s in signatures {
                if ui.selectable_label(d.signature.as_ref() == Some(&s.html), &s.label).clicked() {
                    d.signature = Some(s.html.clone());
                }
            }
            if ui.selectable_label(d.signature.is_none(), t!("Aucune signature", "No signature")).clicked() {
                d.signature = None;
            }
        })
        .response
        .on_hover_text(t!("Signature insérée au format Gmail", "Signature inserted in Gmail format"));
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
            if ui.small_button("×").on_hover_text(t!("Retirer", "Remove")).clicked() {
                on_remove();
            }
        });
}

// --- Fichiers --------------------------------------------------------------

fn read(path: &std::path::Path) -> Result<(String, Vec<u8>), String> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "fichier".into());
    let data = std::fs::read(path).map_err(|e| tf!("Lecture de « {name} » impossible : {e}", "Cannot read “{name}”: {e}"))?;
    Ok((name, data))
}

fn insert_image_file(d: &mut Draft, ed: &mut Editor, path: &std::path::Path) -> Result<(), String> {
    let (name, data) = read(path)?;
    let preview = inline::decode(&data).map_err(|_| tf!("« {name} » n'est pas une image lisible", "“{name}” is not a readable image"))?;
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
                let hint = if single { t!("Écris ton message…", "Write your message…") } else { "" };
                let output = egui::TextEdit::multiline(&mut p.text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .align(egui::Align2([to_egui_align(align), EAlign::Min]))
                    .layouter(&mut layouter)
                    .show(ui);
                crate::spell::check(ui, &output, &mut p.text);
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
                        _ => ui.label(tf!("[image : {}]", "[image: {}]", img.name)),
                    };
                    // Bouton de suppression au survol.
                    if resp.hovered() || ui.rect_contains_pointer(resp.rect) {
                        let r = Rect::from_min_size(resp.rect.right_top() + vec2(-30.0, 6.0), vec2(24.0, 24.0));
                        let del = ui.put(r, egui::Button::new("×").fill(Color32::from_black_alpha(200)).corner_radius(CornerRadius::same(255)));
                        if del.on_hover_text(t!("Retirer l'image", "Remove the image")).clicked() {
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
    for sp in compose::spans(text) {
        let format = if sp.marker {
            TextFormat::simple(theme::editor_font(px * 0.75, false), theme::TEXT_FAINT())
        } else if sp.link.is_some() {
            TextFormat { underline: Stroke::new(1.0, theme::LINK()), ..TextFormat::simple(theme::editor_font(px, sp.bold), theme::LINK()) }
        } else {
            TextFormat::simple(theme::editor_font(px, sp.bold), theme::TEXT())
        };
        job.append(sp.text, 0.0, format);
    }
    if text.is_empty() {
        job.append("", 0.0, TextFormat::simple(theme::editor_font(px, false), theme::TEXT()));
    }
    job.wrap.max_width = wrap;
    job.halign = to_egui_align(align);
    job.justify = align == Align::Justify;
    job
}

/// Ouvre la fenêtre « Lien » : lien sous le curseur (modifier / retirer) ou sélection.
fn open_link(d: &Draft, ed: &mut Editor, pos: egui::Pos2) {
    let Some(Block::Para(p)) = d.blocks.get(ed.focus) else { return };
    let (a, b) = (ed.sel.0.min(ed.sel.1), ed.sel.0.max(ed.sel.1));
    ed.link = Some(match compose::link_at(&p.text, a).filter(|_| a == b || compose::link_at(&p.text, b).is_some()) {
        Some((s, e, label, url)) => LinkEdit { para: ed.focus, a: s, b: e, label, url, pos, existing: true, focus: true },
        None => {
            let label: String = p.text.chars().skip(a).take(b - a).collect();
            LinkEdit {
                para: ed.focus,
                a,
                b,
                label: compose::spans(&label).iter().filter(|s| !s.marker).map(|s| s.text).collect(),
                url: String::new(),
                pos,
                existing: false,
                focus: true,
            }
        }
    });
}

/// Fenêtre « Lien » : texte affiché et adresse ; Entrée applique, Échap ferme.
fn link_window(ui: &mut egui::Ui, d: &mut Draft, ed: &mut Editor) {
    let Some(mut le) = ed.link.take() else { return };
    enum Do {
        Apply,
        Remove,
        Close,
    }
    let mut action = None;
    let area = egui::Area::new(egui::Id::unique("wt_compose_link")).order(egui::Order::Foreground).fixed_pos(le.pos).constrain(true).show(ui.ctx(), |ui| {
        crate::search::overlay_frame().show(ui, |ui| {
            ui.set_width(340.0);
            ui.label(
                RichText::new(if le.existing { t!("MODIFIER LE LIEN", "EDIT LINK") } else { t!("AJOUTER UN LIEN", "ADD A LINK") }).small().strong().color(theme::TEXT_FAINT()),
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                field_label(ui, t!("Texte", "Text"));
                ui.add(egui::TextEdit::singleline(&mut le.label).hint_text(t!("Mot ou phrase affiché", "Word or phrase shown")).desired_width(f32::INFINITY));
            });
            ui.horizontal(|ui| {
                field_label(ui, t!("Lien", "Link"));
                let r = ui.add(egui::TextEdit::singleline(&mut le.url).hint_text("https://…").desired_width(f32::INFINITY));
                if le.focus {
                    r.request_focus();
                    le.focus = false;
                }
                // Entrée : appliquer (la touche est prise, le texte ne doit pas la recevoir).
                if r.lost_focus() && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                    action = Some(Do::Apply);
                }
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let ok = !le.url.trim().is_empty();
                let apply = egui::Button::new(RichText::new(t!("Appliquer", "Apply")).strong().color(Color32::WHITE)).fill(theme::TODAY()).corner_radius(CornerRadius::same(8));
                if ui.add_enabled(ok, apply).clicked() {
                    action = Some(Do::Apply);
                }
                if le.existing && ui.button(t!("Retirer le lien", "Remove link")).clicked() {
                    action = Some(Do::Remove);
                }
                if ui.button(t!("Annuler", "Cancel")).clicked() {
                    action = Some(Do::Close);
                }
            });
        });
    });
    let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if ui.input(|i| i.key_pressed(Key::Escape)) || (outside && action.is_none()) {
        action = Some(Do::Close);
    }
    match action {
        None => ed.link = Some(le),
        Some(Do::Close) => ed.pending = Some((le.para, le.b, le.b)),
        Some(Do::Apply) if !le.url.trim().is_empty() => {
            if let Some(Block::Para(p)) = d.blocks.get_mut(le.para) {
                let end = compose::set_link(&mut p.text, le.a, le.b, &le.label, &le.url);
                ed.pending = Some((le.para, end, end));
            }
        }
        Some(Do::Remove) => {
            if let Some(Block::Para(p)) = d.blocks.get_mut(le.para) {
                let end = compose::remove_link(&mut p.text, le.a, le.b, &le.label);
                ed.pending = Some((le.para, end, end));
            }
        }
        Some(Do::Apply) => ed.link = Some(le),
    }
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
