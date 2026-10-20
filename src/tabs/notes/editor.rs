//! Éditeur par blocs façon Notion / Obsidian : chaque paragraphe est un bloc (titre, liste,
//! case à cocher, citation, code, séparateur, image). Markdown en ligne, commandes « / »,
//! raccourcis Markdown, glisser-déposer d'images et de fichiers.

use std::collections::HashMap;

use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{self, Color32, CornerRadius, Key, Modifiers, Pos2, Rect, Sense, Stroke, Vec2, vec2};

use super::inline::{self, Base};
use crate::tabs::messages::emoji;
use crate::notes_store::{self, Block, Kind, Note};
use crate::theme;

/// État d'édition (non enregistré).
#[derive(Default)]
pub struct Ed {
    /// Focus demandé : bloc, sélection (caractères).
    pub focus: Option<(u64, usize, usize)>,
    /// Dernière position connue du curseur (barre d'outils).
    pub cursor: Option<(u64, usize, usize)>,
    pub slash: Option<Slash>,
    textures: HashMap<String, Option<(egui::TextureHandle, Vec2)>>,
    pub error: Option<String>,
    /// Commande « / » de rattachement : type de lien, bloc et position de la mention.
    pub link_request: Option<(LinkTab, u64, usize)>,
    /// Code « :nom » en cours de frappe : bloc, position du « : », préfixe, position à l'écran.
    pub emoji: Option<(u64, usize, String, Pos2)>,
    /// Sélecteur d'émojis ouvert (position du bouton).
    pub picker: Option<Pos2>,
    pub emoji_query: String,
    pub emoji_group: usize,
}

pub use super::links::Tab as LinkTab;

pub struct Slash {
    pub block: u64,
    /// Position du « / » (caractères).
    pub start: usize,
    pub sel: usize,
    pub pos: Pos2,
    /// Zone du menu (un clic dedans ne le ferme pas).
    pub rect: Rect,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Cmd {
    Kind(Kind),
    Image,
    File,
    Today,
    Link(LinkTab),
}

pub const COMMANDS: [(&str, &str, &str, Cmd); 19] = [
    ("¶", "Texte", "texte paragraphe text", Cmd::Kind(Kind::Text)),
    ("H1", "Titre 1", "titre heading h1", Cmd::Kind(Kind::H1)),
    ("H2", "Titre 2", "titre heading h2", Cmd::Kind(Kind::H2)),
    ("H3", "Titre 3", "titre heading h3", Cmd::Kind(Kind::H3)),
    ("•", "Liste à puces", "liste puces bullet", Cmd::Kind(Kind::Bullet)),
    ("1.", "Liste numérotée", "liste numérotée numbered", Cmd::Kind(Kind::Numbered)),
    ("☑", "Case à cocher", "case cocher todo tâche checkbox", Cmd::Kind(Kind::Todo(false))),
    ("❝", "Citation", "citation quote", Cmd::Kind(Kind::Quote)),
    ("{}", "Code", "code bloc", Cmd::Kind(Kind::Code)),
    ("—", "Séparateur", "séparateur ligne divider", Cmd::Kind(Kind::Divider)),
    ("🖼", "Image", "image photo", Cmd::Image),
    ("📎", "Fichier joint", "fichier pièce jointe file", Cmd::File),
    ("📅", "Date du jour", "date aujourd'hui today", Cmd::Today),
    ("✉", "Mail", "mail courriel email gmail", Cmd::Link(LinkTab::Mail)),
    ("💬", "Message Google Chat", "message chat conversation discussion", Cmd::Link(LinkTab::Chat)),
    ("🗓", "Événement de l'agenda", "événement evenement agenda calendrier rendez-vous réunion", Cmd::Link(LinkTab::Event)),
    ("✅", "Tâche", "tâche tache kanban todo", Cmd::Link(LinkTab::Task)),
    ("📆", "Date", "date jour calendrier", Cmd::Link(LinkTab::Date)),
    ("📄", "Fichier Google Drive", "fichier drive document doc sheet", Cmd::Link(LinkTab::Drive)),
];

/// Les commandes de rattachement commencent ici (titre de section du menu).
const FIRST_LINK: usize = 13;

pub fn block_id(note: &str, b: u64) -> egui::Id {
    egui::Id::unique(("wt_note_block", note, b))
}

fn chars(s: &str) -> usize {
    s.chars().count()
}

fn byte(s: &str, c: usize) -> usize {
    s.char_indices().nth(c).map(|(b, _)| b).unwrap_or(s.len())
}

pub fn base_of(kind: Kind) -> Base {
    let b = Base { size: 15.5, bold: false, italic: false, color: theme::TEXT(), mono: false, strike: false };
    match kind {
        Kind::H1 => Base { size: 28.0, bold: true, ..b },
        Kind::H2 => Base { size: 22.0, bold: true, ..b },
        Kind::H3 => Base { size: 18.0, bold: true, ..b },
        Kind::Quote => Base { color: crate::theme::gray(0xC4), ..b },
        Kind::Code => Base { size: 13.5, mono: true, color: crate::theme::gray(0xDA), ..b },
        Kind::Todo(true) => Base { color: theme::TEXT_MUTED(), strike: true, ..b },
        _ => b,
    }
}

enum Op {
    Split(usize, usize),
    ExitCode(usize),
    MergeBack(usize),
    Convert(usize, Kind),
    Focus(usize, bool),
    Move(usize, bool),
    Remove(usize),
    ToggleTodo(usize),
    Explode(usize),
    Slash(usize, Cmd),
    Insert(usize, Vec<(Kind, String)>),
}

/// Rendu (et édition si `edit`) du corps de la note. Renvoie un lien cliqué.
pub fn body(ui: &mut egui::Ui, note: &mut Note, ed: &mut Ed, edit: bool) -> Option<String> {
    let ctx = ui.ctx().clone();
    let mut ops: Vec<Op> = Vec::new();
    let mut clicked_link = None;
    let mut changed = false;
    let note_id = note.id.clone();
    let width = ui.available_width();
    let single = note.blocks.len() == 1;
    let mut number = 0usize;

    for i in 0..note.blocks.len() {
        let kind = note.blocks[i].kind;
        number = if kind == Kind::Numbered { number + 1 } else { 0 };
        let top = match kind {
            Kind::H1 => 18.0,
            Kind::H2 => 14.0,
            Kind::H3 => 8.0,
            _ => 1.0,
        };
        ui.add_space(top);
        match kind {
            Kind::Divider => {
                let (r, resp) = ui.allocate_exact_size(vec2(width, 22.0), Sense::click());
                let y = r.center().y;
                ui.painter().line_segment([Pos2::new(r.left(), y), Pos2::new(r.right(), y)], Stroke::new(1.0, theme::BORDER()));
                if edit && resp.hovered() && delete_button(ui, r) {
                    ops.push(Op::Remove(i));
                }
            }
            Kind::Image => {
                let rel = note.blocks[i].text.clone();
                let key = format!("{note_id}/{rel}");
                let tex = ed
                    .textures
                    .entry(key.clone())
                    .or_insert_with(|| {
                        let bytes = note.image_bytes(&rel)?;
                        let img = decode(&bytes)?;
                        let size = vec2(img.size[0] as f32, img.size[1] as f32);
                        Some((ctx.load_texture(format!("note_{key}"), img, egui::TextureOptions::LINEAR), size))
                    })
                    .clone();
                match tex {
                    Some((t, size)) => {
                        let scale = (width / size.x).min(1.0);
                        let sz = size * scale;
                        let (r, resp) = ui.allocate_exact_size(vec2(width, sz.y + 8.0), Sense::click());
                        let img_rect = Rect::from_min_size(Pos2::new(r.left(), r.top() + 4.0), sz);
                        ui.painter().add(egui::epaint::RectShape::filled(img_rect, CornerRadius::same(6), Color32::WHITE).with_texture(t.id(), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0))));
                        if edit && (resp.hovered() || ui.rect_contains_pointer(img_rect)) && delete_button(ui, img_rect) {
                            ops.push(Op::Remove(i));
                        }
                        if resp.double_clicked() {
                            clicked_link = Some(rel.clone());
                        }
                    }
                    None => {
                        let (r, resp) = ui.allocate_exact_size(vec2(width, 40.0), Sense::click());
                        ui.painter().rect_stroke(r, CornerRadius::same(6), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
                        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, format!("Image introuvable : {rel}"), egui::FontId::proportional(12.0), theme::TEXT_MUTED());
                        if edit && resp.hovered() && delete_button(ui, r) {
                            ops.push(Op::Remove(i));
                        }
                    }
                }
            }
            _ => {
                let id = block_id(&note_id, note.blocks[i].id);
                if edit {
                    text_block(ui, &ctx, note, i, id, number, single, ed, &mut ops, &mut changed, &mut clicked_link);
                } else {
                    read_block(ui, &mut note.blocks[i], number, &mut ops, i, &mut clicked_link);
                }
            }
        }
    }

    // Clic sous le texte : curseur à la fin (nouveau paragraphe si le dernier n'est pas du texte).
    if edit {
        let rest = ui.available_size_before_wrap();
        let (_, resp) = ui.allocate_exact_size(vec2(width, rest.y.max(120.0)), Sense::click());
        if resp.clicked() {
            match note.blocks.last() {
                Some(b) if b.kind == Kind::Text => ed.focus = Some((b.id, chars(&b.text), chars(&b.text))),
                _ => ops.push(Op::Insert(note.blocks.len().saturating_sub(1), vec![(Kind::Text, String::new())])),
            }
        }
    } else {
        ui.add_space(80.0);
    }

    // Suggestions d'émojis (« :nom »).
    if let Some(e) = emoji_suggestions(ui, ed) {
        if let Some((bid, at, prefix, _)) = ed.emoji.take() {
            if let Some(b) = note.blocks.iter_mut().find(|b| b.id == bid) {
                let c = insert_emoji_code(&mut b.text, at, prefix.chars().count(), &e);
                ed.focus = Some((bid, c, c));
                changed = true;
            }
        }
    }
    // Sélecteur d'émojis de la barre d'outils.
    if let Some(e) = emoji_picker(ui, ed) {
        insert_text(note, ed, &e);
    }

    // Menu « / ».
    if let Some(cmd) = slash_menu(ui, note, ed) {
        if let Some(i) = ed.slash.as_ref().and_then(|s| note.blocks.iter().position(|b| b.id == s.block)) {
            ops.push(Op::Slash(i, cmd));
        }
    }

    if !ops.is_empty() || changed {
        apply(note, ed, ops);
        note.touch();
    }
    clicked_link
}

/// Bouton « × » dessiné par-dessus un bloc (sans réserver de place dans la mise en page).
fn delete_button(ui: &mut egui::Ui, r: Rect) -> bool {
    let b = Rect::from_min_size(r.right_top() + vec2(-32.0, 6.0), vec2(26.0, 26.0));
    let resp = ui.interact(b, ui.scope_id().with(("wt_note_del", r.min.x as i32, r.min.y as i32)), Sense::click()).on_hover_text("Supprimer");
    let p = ui.painter();
    p.circle_filled(b.center(), 13.0, Color32::from_black_alpha(if resp.hovered() { 240 } else { 190 }));
    let k = 4.5;
    let c = b.center();
    let st = Stroke::new(1.6, theme::TEXT());
    p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], st);
    p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], st);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

pub fn decode(bytes: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    let img = if img.width() > 1800 {
        let h = (img.height() as f32 * 1800.0 / img.width() as f32) as u32;
        image::imageops::resize(&img, 1800, h.max(1), image::imageops::FilterType::Triangle)
    } else {
        img
    };
    Some(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()))
}

/// Puce, numéro, case ou barre de citation à gauche du texte. Renvoie vrai si la case est cliquée.
fn gutter(ui: &mut egui::Ui, kind: Kind, number: usize, line_h: f32) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(26.0, line_h), if matches!(kind, Kind::Todo(_)) { Sense::click() } else { Sense::hover() });
    let p = ui.painter();
    let c = Pos2::new(r.left() + 9.0, r.center().y);
    match kind {
        Kind::Bullet => {
            p.circle_filled(c, 2.6, theme::TEXT());
        }
        Kind::Numbered => {
            p.text(Pos2::new(r.left() + 16.0, c.y), egui::Align2::RIGHT_CENTER, format!("{number}."), theme::editor_font(15.0, false), theme::TEXT_MUTED());
        }
        Kind::Todo(done) => {
            let b = Rect::from_center_size(c, vec2(15.0, 15.0));
            if done {
                p.rect_filled(b, CornerRadius::same(4), theme::TEXT());
                p.line(vec![b.left_center() + vec2(3.5, 0.0), b.center_bottom() + vec2(-1.0, -4.0), b.right_top() + vec2(-3.5, 4.0)], Stroke::new(1.8, theme::BG()));
            } else {
                let col = if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
                p.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.4, col), egui::StrokeKind::Inside);
            }
        }
        _ => {}
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

#[allow(clippy::too_many_arguments)]
fn text_block(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    note: &mut Note,
    i: usize,
    id: egui::Id,
    number: usize,
    single: bool,
    ed: &mut Ed,
    ops: &mut Vec<Op>,
    changed: &mut bool,
    clicked_link: &mut Option<String>,
) {
    let block = &note.blocks[i];
    let kind = block.kind;
    let bid = block.id;
    let len = chars(&block.text);

    // Focus / sélection demandés.
    if let Some((_, a, b)) = ed.focus.filter(|(f, ..)| *f == bid) {
        let mut st = TextEditState::load(ctx, id).unwrap_or_default();
        st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a.min(len)), CCursor::new(b.min(len)))));
        st.store(ctx, id);
        ctx.memory_mut(|m| m.request_focus(id));
        ed.focus = None;
    }

    let focused = ctx.memory(|m| m.has_focus(id));
    let slash_here = ed.slash.as_ref().is_some_and(|s| s.block == bid);
    // Suggestion d'émoji : Tab insère la première.
    if focused {
        if let Some((_, at, prefix, _)) = ed.emoji.clone().filter(|e| e.0 == bid) {
            if let Some(e) = emoji::suggestions(ctx, &prefix, 1).first() {
                if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Tab)) {
                    let c = insert_emoji_code(&mut note.blocks[i].text, at, prefix.chars().count(), e.as_str());
                    ed.focus = Some((bid, c, c));
                    ed.emoji = None;
                    *changed = true;
                }
            }
        }
    }
    // Touches gérées avant le champ.
    if focused && !slash_here {
        let cur = TextEditState::load(ctx, id).and_then(|s| s.cursor.char_range()).map(|r| (r.primary.index.0, r.secondary.index.0));
        if let Some((a, b)) = cur {
            let (lo, hi) = (a.min(b), a.max(b));
            let enter = ui.input_mut(|inp| !inp.modifiers.command && !inp.modifiers.alt && inp.consume_key(Modifiers::NONE, Key::Enter) || (inp.modifiers.shift && inp.consume_key(Modifiers::SHIFT, Key::Enter)));
            let ctrl_enter = ui.input_mut(|inp| inp.consume_key(Modifiers::COMMAND, Key::Enter));
            if kind == Kind::Code {
                if ctrl_enter || (enter && lo == len && note.blocks[i].text.ends_with('\n')) {
                    ops.push(Op::ExitCode(i));
                } else if enter {
                    // Retour à la ligne dans le code.
                    let text = &mut note.blocks[i].text;
                    let (bl, bh) = (byte(text, lo), byte(text, hi));
                    text.replace_range(bl..bh, "\n");
                    ed.focus = Some((bid, lo + 1, lo + 1));
                    *changed = true;
                }
            } else if enter || ctrl_enter {
                ops.push(Op::Split(i, lo));
            }
            let block = &note.blocks[i];
            if lo == 0 && hi == 0 && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Backspace)) {
                if kind != Kind::Text && !(kind == Kind::Code && !block.text.is_empty()) {
                    ops.push(Op::Convert(i, Kind::Text));
                } else if i > 0 {
                    ops.push(Op::MergeBack(i));
                }
            }
            if ui.input_mut(|inp| inp.consume_key(Modifiers::ALT, Key::ArrowUp)) {
                ops.push(Op::Move(i, true));
            } else if ui.input_mut(|inp| inp.consume_key(Modifiers::ALT, Key::ArrowDown)) {
                ops.push(Op::Move(i, false));
            } else if lo == 0 && hi == 0 && i > 0 && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowUp)) {
                ops.push(Op::Focus(i - 1, true));
            } else if lo == len && hi == len && i + 1 < note.blocks.len() && ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::ArrowDown)) {
                ops.push(Op::Focus(i + 1, false));
            }
            // Mise en forme au clavier.
            let shortcuts: [(Modifiers, Key, &str, &str); 6] = [
                (Modifiers::COMMAND, Key::B, "**", "**"),
                (Modifiers::COMMAND, Key::I, "*", "*"),
                (Modifiers::COMMAND, Key::U, "<u>", "</u>"),
                (Modifiers::COMMAND | Modifiers::SHIFT, Key::X, "~~", "~~"),
                (Modifiers::COMMAND, Key::E, "`", "`"),
                (Modifiers::COMMAND | Modifiers::SHIFT, Key::H, "==", "=="),
            ];
            for (m, k, open, close) in shortcuts {
                if kind != Kind::Code && ui.input_mut(|inp| inp.consume_key(m, k)) {
                    let (na, nb) = wrap(&mut note.blocks[i].text, lo, hi, open, close);
                    ed.focus = Some((bid, na, nb));
                    *changed = true;
                }
            }
            if kind != Kind::Code && ui.input_mut(|inp| inp.consume_key(Modifiers::COMMAND, Key::K)) {
                let (na, nb) = link(&mut note.blocks[i].text, lo, hi);
                ed.focus = Some((bid, na, nb));
                *changed = true;
            }
        }
    }

    let base = base_of(kind);
    let line_h = base.size * 1.5;
    let hint = match kind {
        Kind::H1 | Kind::H2 | Kind::H3 => kind.label(),
        Kind::Bullet | Kind::Numbered => "Liste",
        Kind::Todo(_) => "À faire",
        Kind::Quote => "Citation",
        Kind::Code => "Code",
        _ if focused || single => "Écris, ou tape « / » pour les commandes…",
        _ => "",
    };
    let width = ui.available_width();
    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_w: f32| ui.painter().layout_job(inline::job(buf.as_str(), base, true, wrap_w).0);

    let before = note.blocks[i].text.clone();
    let mut toggled = false;
    let output = if kind == Kind::Code {
        egui::Frame::new()
            .fill(theme::SURFACE())
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(egui::Margin::symmetric(14, 12))
            .show(ui, |ui| {
                ui.set_width(width - 28.0);
                egui::TextEdit::multiline(&mut note.blocks[i].text)
                    .id(id)
                    .frame(egui::Frame::NONE)
                    .desired_rows(1)
                    .desired_width(f32::INFINITY)
                    .hint_text(hint)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui)
            })
            .inner
    } else {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            let quote = kind == Kind::Quote;
            if matches!(kind, Kind::Bullet | Kind::Numbered | Kind::Todo(_)) {
                toggled = gutter(ui, kind, number, line_h);
            } else if quote {
                ui.add_space(16.0);
            }
            let out = egui::TextEdit::multiline(&mut note.blocks[i].text)
                .id(id)
                .frame(egui::Frame::NONE)
                .desired_rows(1)
                .desired_width(f32::INFINITY)
                .hint_text(egui::RichText::new(hint).color(theme::TEXT_FAINT()).font(inline::font(base.size, base.bold, false)))
                .return_key(None)
                .layouter(&mut layouter)
                .show(ui);
            if quote {
                let r = out.response.rect;
                ui.painter().line_segment([Pos2::new(r.left() - 13.0, r.top() + 2.0), Pos2::new(r.left() - 13.0, r.bottom() - 2.0)], Stroke::new(3.0, theme::TEXT_MUTED()));
            }
            out
        })
        .inner
    };
    if toggled {
        ops.push(Op::ToggleTodo(i));
    }

    let resp = &output.response;
    if resp.has_focus() {
        if let Some(r) = output.cursor_range {
            ed.cursor = Some((bid, r.primary.index.0, r.secondary.index.0));
        }
    }
    // Ctrl + clic sur un lien : ouverture.
    if resp.clicked() && ui.input(|inp| inp.modifiers.command) {
        if let Some(p) = resp.interact_pointer_pos() {
            let idx = output.galley.cursor_from_pos(p - output.galley_pos).index.0;
            let (_, links) = inline::job(&note.blocks[i].text, base, true, f32::INFINITY);
            if let Some((_, url)) = links.iter().find(|(r, _)| r.contains(&idx)) {
                *clicked_link = Some(url.clone());
            }
        }
    }
    // « :nom » en cours de frappe : suggestions sous le curseur.
    if output.response.has_focus() && kind != Kind::Code {
        let cursor = output.cursor_range.map(|r| r.primary.index.0).unwrap_or(0);
        match emoji::pending_code_at(&note.blocks[i].text, cursor) {
            Some((at, prefix)) => {
                let pos = output.galley_pos + output.galley.pos_from_cursor(CCursor::new(cursor)).left_bottom().to_vec2();
                ed.emoji = Some((bid, at, prefix, pos));
            }
            None => {
                if ed.emoji.as_ref().is_some_and(|e| e.0 == bid) {
                    ed.emoji = None;
                }
            }
        }
    }
    if note.blocks[i].text != before {
        *changed = true;
        // « :nom: » complet → émoji, comme dans Google Chat.
        if kind != Kind::Code {
            let cursor = output.cursor_range.map(|r| r.primary.index.0).unwrap_or(chars(&note.blocks[i].text));
            if let Some((t, c)) = emoji::replace_codes_at(&note.blocks[i].text, cursor) {
                note.blocks[i].text = t;
                ed.focus = Some((bid, c, c));
                ed.emoji = None;
            }
        }
        let text = note.blocks[i].text.clone();
        let cursor = output.cursor_range.map(|r| r.primary.index.0).unwrap_or(chars(&text));
        if kind != Kind::Code && text.contains('\n') {
            ops.push(Op::Explode(i));
        } else if let Some(k) = shortcut(kind, &text, cursor) {
            ops.push(Op::Convert(i, k.0));
            note.blocks[i].text = text.chars().skip(k.1).collect();
            ed.focus = Some((bid, 0, 0));
        } else if kind != Kind::Code && cursor > 0 && text.chars().nth(cursor - 1) == Some('/') && (cursor == 1 || text.chars().nth(cursor - 2).is_some_and(char::is_whitespace)) {
            let pos = output.galley_pos + output.galley.pos_from_cursor(CCursor::new(cursor)).left_bottom().to_vec2();
            ed.slash = Some(Slash { block: bid, start: cursor - 1, sel: 0, pos, rect: Rect::NOTHING });
        }
    }
    // Fermeture du menu « / » si le curseur en sort.
    if let Some(s) = ed.slash.as_ref().filter(|s| s.block == bid) {
        let cursor = output.cursor_range.map(|r| r.primary.index.0);
        let text = &note.blocks[i].text;
        let in_menu = ui.input(|i| i.pointer.interact_pos()).is_some_and(|p| s.rect.contains(p));
        let ok = (resp.has_focus() || in_menu)
            && cursor.is_some_and(|c| c > s.start && text.chars().nth(s.start) == Some('/'))
            && !slash_filter(text, s.start, cursor.unwrap_or(0)).contains(char::is_whitespace);
        if !ok {
            ed.slash = None;
        }
    }
}

fn read_block(ui: &mut egui::Ui, b: &mut Block, number: usize, ops: &mut Vec<Op>, i: usize, clicked_link: &mut Option<String>) {
    let base = base_of(b.kind);
    let width = ui.available_width();
    let draw = |ui: &mut egui::Ui, w: f32, clicked_link: &mut Option<String>| {
        let (job, links) = inline::job(&b.text, base, false, w);
        let galley = ui.painter().layout_job(job);
        let (r, resp) = ui.allocate_exact_size(vec2(w, galley.size().y.max(base.size * 1.5)), Sense::click());
        ui.painter().galley(r.min, galley.clone(), base.color);
        if let Some(p) = resp.hover_pos() {
            let idx = galley.cursor_from_pos(p - r.min).index.0;
            if let Some((_, url)) = links.iter().find(|(rg, _)| rg.contains(&idx)) {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                if resp.clicked() {
                    *clicked_link = Some(url.clone());
                }
                egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), egui::Id::unique("wt_note_link_tip"), egui::PopupAnchor::Pointer).show(|ui| {
                    ui.label(egui::RichText::new(url).small());
                });
            }
        }
        r
    };
    match b.kind {
        Kind::Code => {
            egui::Frame::new()
                .fill(theme::SURFACE())
                .stroke(Stroke::new(1.0, theme::BORDER()))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(14, 12))
                .show(ui, |ui| {
                    draw(ui, width - 28.0, clicked_link);
                });
        }
        Kind::Bullet | Kind::Numbered | Kind::Todo(_) | Kind::Quote => {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                if b.kind == Kind::Quote {
                    ui.add_space(16.0);
                } else if gutter(ui, b.kind, number, base.size * 1.5) {
                    ops.push(Op::ToggleTodo(i));
                }
                let r = draw(ui, ui.available_width(), clicked_link);
                if b.kind == Kind::Quote {
                    ui.painter().line_segment([Pos2::new(r.left() - 13.0, r.top() + 2.0), Pos2::new(r.left() - 13.0, r.bottom() - 2.0)], Stroke::new(3.0, theme::TEXT_MUTED()));
                }
            });
        }
        _ => {
            draw(ui, width, clicked_link);
        }
    }
}

fn slash_filter(text: &str, start: usize, cursor: usize) -> String {
    text.chars().skip(start + 1).take(cursor.saturating_sub(start + 1)).collect()
}

fn matches(filter: &str) -> Vec<usize> {
    let f = filter.to_lowercase();
    (0..COMMANDS.len()).filter(|&k| f.is_empty() || COMMANDS[k].1.to_lowercase().contains(&f) || COMMANDS[k].2.contains(&f)).collect()
}

/// Menu des commandes « / » sous le curseur ; renvoie la commande choisie.
fn slash_menu(ui: &mut egui::Ui, note: &Note, ed: &mut Ed) -> Option<Cmd> {
    let s = ed.slash.as_mut()?;
    let block = note.blocks.iter().find(|b| b.id == s.block)?;
    let id = block_id(&note.id, s.block);
    let cursor = TextEditState::load(ui.ctx(), id).and_then(|st| st.cursor.char_range()).map(|r| r.primary.index.0).unwrap_or(s.start + 1);
    let filter = slash_filter(&block.text, s.start, cursor);
    let list = matches(&filter);
    if list.is_empty() {
        if filter.chars().count() > 3 {
            ed.slash = None;
        }
        return None;
    }
    s.sel = s.sel.min(list.len() - 1);
    let mut chosen = None;
    ui.input_mut(|inp| {
        if inp.consume_key(Modifiers::NONE, Key::ArrowDown) {
            s.sel = (s.sel + 1) % list.len();
        }
        if inp.consume_key(Modifiers::NONE, Key::ArrowUp) {
            s.sel = (s.sel + list.len() - 1) % list.len();
        }
        if inp.consume_key(Modifiers::NONE, Key::Enter) || inp.consume_key(Modifiers::NONE, Key::Tab) {
            chosen = Some(COMMANDS[list[s.sel]].3);
        }
    });
    if ui.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Escape)) {
        ed.slash = None;
        return None;
    }
    let mut pick = None;
    // Taille fixe ; ouvert au-dessus du curseur s'il n'y a pas la place en dessous.
    let screen = ui.ctx().content_rect();
    let below = screen.bottom() - s.pos.y - 16.0;
    let above = s.pos.y - screen.top() - 40.0;
    let up = below < 220.0 && above > below;
    let max_h = (if up { above } else { below } - 20.0).clamp(120.0, 300.0);
    let (pivot, at) = if up { (egui::Align2::LEFT_BOTTOM, s.pos - vec2(0.0, 26.0)) } else { (egui::Align2::LEFT_TOP, s.pos + vec2(0.0, 4.0)) };
    let area = egui::Area::new(egui::Id::unique("wt_note_slash"))
        .order(egui::Order::Foreground)
        .pivot(pivot)
        .fixed_pos(at)
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(theme::SURFACE_HI())
                .stroke(Stroke::new(1.0, theme::BORDER()))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::same(6))
                .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) })
                .show(ui, |ui| {
                    ui.set_width(240.0);
                    egui::ScrollArea::vertical().max_height(max_h).auto_shrink([false, true]).show(ui, |ui| {
                    ui.set_width(232.0);
                    ui.spacing_mut().item_spacing.y = 1.0;
                    for (row, &k) in list.iter().enumerate() {
                        let (icon, label, _, cmd) = COMMANDS[k];
                        if row == 0 || (k >= FIRST_LINK && list[row - 1] < FIRST_LINK) {
                            if row > 0 {
                                ui.add_space(4.0);
                            }
                            ui.label(egui::RichText::new(if k >= FIRST_LINK { "RATTACHER" } else { "BLOCS" }).small().strong().color(theme::TEXT_MUTED()));
                        }
                        let (r, resp) = ui.allocate_exact_size(vec2(232.0, 28.0), Sense::click());
                        if row == s.sel {
                            ui.scroll_to_rect(r, None);
                        }
                        if row == s.sel || resp.hovered() {
                            ui.painter().rect_filled(r, CornerRadius::same(6), crate::theme::gray(0x2C));
                        }
                        let icon_r = Rect::from_min_size(r.min + vec2(4.0, 3.0), vec2(22.0, 22.0));
                        ui.painter().rect_stroke(icon_r, CornerRadius::same(5), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
                        ui.painter().text(icon_r.center(), egui::Align2::CENTER_CENTER, icon, egui::FontId::proportional(12.0), theme::TEXT());
                        ui.painter().text(Pos2::new(r.left() + 38.0, r.center().y), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.5), theme::TEXT());
                        if resp.clicked() {
                            pick = Some(cmd);
                        }
                    }
                    });
                });
        });
    s.rect = area.response.rect;
    // Un clic sur la souris dans le menu fait perdre le focus au champ : on garde la commande.
    chosen.or(pick)
}

/// Raccourcis Markdown en début de bloc : renvoie le type et la longueur du préfixe à retirer.
fn shortcut(kind: Kind, text: &str, cursor: usize) -> Option<(Kind, usize)> {
    let rules: &[(&str, Kind)] = match kind {
        Kind::Text => &[
            ("# ", Kind::H1),
            ("## ", Kind::H2),
            ("### ", Kind::H3),
            ("- ", Kind::Bullet),
            ("* ", Kind::Bullet),
            ("1. ", Kind::Numbered),
            ("[] ", Kind::Todo(false)),
            ("[ ] ", Kind::Todo(false)),
            ("> ", Kind::Quote),
            ("```", Kind::Code),
            ("---", Kind::Divider),
        ],
        Kind::Bullet => &[("[] ", Kind::Todo(false)), ("[ ] ", Kind::Todo(false))],
        _ => &[],
    };
    for (p, k) in rules {
        let n = chars(p);
        if text.starts_with(p) && cursor == n {
            return Some((*k, n));
        }
    }
    None
}

/// Entoure (ou libère) la sélection de marqueurs ; renvoie la nouvelle sélection.
pub fn wrap(text: &mut String, lo: usize, hi: usize, open: &str, close: &str) -> (usize, usize) {
    let (bl, bh) = (byte(text, lo), byte(text, hi));
    let (no, nc) = (chars(open), chars(close));
    if text[..bl].ends_with(open) && text[bh..].starts_with(close) {
        text.replace_range(bh..bh + close.len(), "");
        text.replace_range(bl - open.len()..bl, "");
        return (lo - no, hi - no);
    }
    let sel = &text[bl..bh];
    if sel.starts_with(open) && sel.ends_with(close) && sel.len() >= open.len() + close.len() {
        let inner = sel[open.len()..sel.len() - close.len()].to_string();
        text.replace_range(bl..bh, &inner);
        return (lo, hi - no - nc);
    }
    text.insert_str(bh, close);
    text.insert_str(bl, open);
    (lo + no, hi + no)
}

/// [sélection](https://) avec l'adresse sélectionnée.
pub fn link(text: &mut String, lo: usize, hi: usize) -> (usize, usize) {
    let (bl, bh) = (byte(text, lo), byte(text, hi));
    let label = if lo == hi { "lien".to_string() } else { text[bl..bh].to_string() };
    let n = chars(&label);
    text.replace_range(bl..bh, &format!("[{label}](https://)"));
    (lo + n + 3, lo + n + 11)
}

fn apply(note: &mut Note, ed: &mut Ed, ops: Vec<Op>) {
    // Une seule opération structurelle par image (les indices changent ensuite).
    for op in ops.into_iter().take(1) {
        match op {
            Op::Split(i, at) => {
                let b = &note.blocks[i];
                let kind = b.kind;
                if kind.continues() && b.text.is_empty() {
                    note.blocks[i].kind = Kind::Text;
                    ed.focus = Some((note.blocks[i].id, 0, 0));
                    continue;
                }
                if at == 0 && !b.text.is_empty() {
                    let nb = note.block(Kind::Text, "");
                    note.blocks.insert(i, nb);
                    ed.focus = Some((note.blocks[i + 1].id, 0, 0));
                    continue;
                }
                let at_b = byte(&b.text, at);
                let right = note.blocks[i].text.split_off(at_b);
                let nk = match kind {
                    Kind::Todo(_) => Kind::Todo(false),
                    k if k.continues() => k,
                    _ => Kind::Text,
                };
                let nb = note.block(nk, &right);
                ed.focus = Some((nb.id, 0, 0));
                note.blocks.insert(i + 1, nb);
            }
            Op::ExitCode(i) => {
                while note.blocks[i].text.ends_with('\n') {
                    note.blocks[i].text.pop();
                }
                let nb = note.block(Kind::Text, "");
                ed.focus = Some((nb.id, 0, 0));
                note.blocks.insert(i + 1, nb);
            }
            Op::MergeBack(i) => {
                let prev = i - 1;
                if matches!(note.blocks[prev].kind, Kind::Divider | Kind::Image) {
                    note.blocks.remove(prev);
                    ed.focus = Some((note.blocks[prev].id, 0, 0));
                } else {
                    let b = note.blocks.remove(i);
                    let caret = chars(&note.blocks[prev].text);
                    note.blocks[prev].text.push_str(&b.text);
                    ed.focus = Some((note.blocks[prev].id, caret, caret));
                }
            }
            Op::Convert(i, k) => {
                if k == Kind::Divider {
                    note.blocks[i].kind = Kind::Divider;
                    note.blocks[i].text.clear();
                    let nb = note.block(Kind::Text, "");
                    ed.focus = Some((nb.id, 0, 0));
                    note.blocks.insert(i + 1, nb);
                } else {
                    note.blocks[i].kind = k;
                    if ed.focus.is_none() {
                        ed.focus = Some((note.blocks[i].id, 0, 0));
                    }
                }
            }
            Op::Focus(i, end) => {
                let b = &note.blocks[i];
                if matches!(b.kind, Kind::Divider | Kind::Image) {
                    continue;
                }
                let n = if end { chars(&b.text) } else { 0 };
                ed.focus = Some((b.id, n, n));
            }
            Op::Move(i, up) => {
                let j = if up { i.checked_sub(1) } else { (i + 1 < note.blocks.len()).then_some(i + 1) };
                if let Some(j) = j {
                    note.blocks.swap(i, j);
                    if let Some((id, a, b)) = ed.cursor {
                        ed.focus = Some((id, a, b));
                    }
                }
            }
            Op::Remove(i) => {
                note.blocks.remove(i);
                if note.blocks.is_empty() {
                    let nb = note.block(Kind::Text, "");
                    note.blocks.push(nb);
                }
            }
            Op::ToggleTodo(i) => {
                if let Kind::Todo(d) = note.blocks[i].kind {
                    note.blocks[i].kind = Kind::Todo(!d);
                }
            }
            Op::Explode(i) => {
                let text = std::mem::take(&mut note.blocks[i].text);
                let mut lines = text.splitn(2, '\n');
                let first = lines.next().unwrap_or_default().to_string();
                let rest = lines.next().unwrap_or_default().to_string();
                let (k, t) = if note.blocks[i].kind == Kind::Text { notes_store::parse_line(&first) } else { (note.blocks[i].kind, first) };
                note.blocks[i].kind = k;
                note.blocks[i].text = t;
                let mut tmp = Note::new();
                tmp.set_markdown(&rest);
                let mut at = i + 1;
                let mut last = note.blocks[i].id;
                for b in tmp.blocks {
                    let nb = note.block(b.kind, &b.text);
                    last = nb.id;
                    note.blocks.insert(at, nb);
                    at += 1;
                }
                let n = note.blocks.iter().find(|b| b.id == last).map(|b| chars(&b.text)).unwrap_or(0);
                ed.focus = Some((last, n, n));
            }
            Op::Slash(i, cmd) => {
                let Some(s) = ed.slash.take() else { continue };
                let id = note.blocks[i].id;
                // Retire « /filtre ».
                let text = &mut note.blocks[i].text;
                let end = text.chars().skip(s.start).position(char::is_whitespace).map(|p| s.start + p).unwrap_or(chars(text));
                let (bs, be) = (byte(text, s.start), byte(text, end));
                text.replace_range(bs..be, "");
                let empty = text.trim().is_empty();
                match cmd {
                    Cmd::Today => {
                        let d = crate::tabs::today_long();
                        let text = &mut note.blocks[i].text;
                        text.insert_str(bs, &d);
                        let c = s.start + chars(&d);
                        ed.focus = Some((id, c, c));
                    }
                    Cmd::Kind(Kind::Divider) => {
                        if empty {
                            note.blocks[i].kind = Kind::Divider;
                            note.blocks[i].text.clear();
                        } else {
                            let d = note.block(Kind::Divider, "");
                            note.blocks.insert(i + 1, d);
                        }
                        let at = if empty { i + 1 } else { i + 2 };
                        let nb = note.block(Kind::Text, "");
                        ed.focus = Some((nb.id, 0, 0));
                        note.blocks.insert(at, nb);
                    }
                    Cmd::Kind(k) => {
                        note.blocks[i].kind = k;
                        ed.focus = Some((id, s.start, s.start));
                    }
                    Cmd::Link(tab) => {
                        ed.link_request = Some((tab, id, s.start));
                    }
                    Cmd::Image | Cmd::File => {
                        let at = if empty { i } else { i + 1 };
                        if empty {
                            note.blocks.remove(i);
                        }
                        insert_files(note, ed, at, cmd == Cmd::Image, None);
                    }
                }
            }
            Op::Insert(i, list) => {
                let mut at = (i + 1).min(note.blocks.len());
                for (k, t) in list {
                    let nb = note.block(k, &t);
                    ed.focus = Some((nb.id, chars(&t), chars(&t)));
                    note.blocks.insert(at, nb);
                    at += 1;
                }
            }
        }
    }
}

/// Insère des images / fichiers (sélecteur si `paths` est vide) à la position `at`.
pub fn insert_files(note: &mut Note, ed: &mut Ed, at: usize, images: bool, paths: Option<Vec<std::path::PathBuf>>) {
    let paths = paths.or_else(|| {
        let d = rfd::FileDialog::new();
        let d = if images { d.add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]) } else { d };
        d.pick_files()
    });
    let Some(paths) = paths else { return };
    let mut at = at.min(note.blocks.len());
    for p in paths {
        let is_img = p.extension().map(|e| e.to_string_lossy().to_lowercase()).is_some_and(|e| ["png", "jpg", "jpeg", "gif", "webp", "bmp"].contains(&e.as_str()));
        match note.add_image(&p) {
            Ok(rel) => {
                let b = if is_img {
                    note.block(Kind::Image, &rel)
                } else {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    note.block(Kind::Text, &format!("📎 [{}]({})", name.replace(['[', ']'], ""), rel.replace(' ', "%20")))
                };
                note.blocks.insert(at, b);
                at += 1;
            }
            Err(e) => ed.error = Some(e),
        }
    }
    // Un paragraphe vide après, pour continuer à écrire.
    if note.blocks.get(at).is_none_or(|b| b.kind != Kind::Text) {
        let nb = note.block(Kind::Text, "");
        note.blocks.insert(at, nb);
    }
    let id = note.blocks[at].id;
    ed.focus = Some((id, 0, 0));
    note.touch();
}

/// Applique un type au bloc courant (barre d'outils).
pub fn set_kind(note: &mut Note, ed: &mut Ed, k: Kind) {
    let i = current(note, ed);
    if k == Kind::Divider {
        let d = note.block(Kind::Divider, "");
        note.blocks.insert(i + 1, d);
        let nb = note.block(Kind::Text, "");
        ed.focus = Some((nb.id, 0, 0));
        note.blocks.insert(i + 2, nb);
    } else if let Some(b) = note.blocks.get_mut(i) {
        if matches!(b.kind, Kind::Divider | Kind::Image) {
            return;
        }
        b.kind = if b.kind == k && k != Kind::Text { Kind::Text } else { k };
        let (a, c) = ed.cursor.filter(|c| c.0 == b.id).map(|c| (c.1, c.2)).unwrap_or((chars(&b.text), chars(&b.text)));
        ed.focus = Some((b.id, a, c));
    }
    note.touch();
}

/// Index du bloc qui a (ou avait) le curseur.
pub fn current(note: &Note, ed: &Ed) -> usize {
    ed.cursor
        .and_then(|(id, ..)| note.blocks.iter().position(|b| b.id == id))
        .unwrap_or(note.blocks.len().saturating_sub(1))
}

/// Mise en forme depuis la barre d'outils.
pub fn format(note: &mut Note, ed: &mut Ed, open: &str, close: &str) {
    let Some((id, a, b)) = ed.cursor else { return };
    let Some(blk) = note.blocks.iter_mut().find(|x| x.id == id) else { return };
    if matches!(blk.kind, Kind::Code | Kind::Divider | Kind::Image) {
        return;
    }
    let (lo, hi) = (a.min(b), a.max(b));
    let (na, nb) = if open == "[" { link(&mut blk.text, lo, hi) } else { wrap(&mut blk.text, lo, hi, open, close) };
    ed.focus = Some((id, na, nb));
    ed.cursor = Some((id, na, nb));
    note.touch();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_toggles() {
        let mut t = "un mot ici".to_string();
        let (a, b) = wrap(&mut t, 3, 6, "**", "**");
        assert_eq!((t.as_str(), a, b), ("un **mot** ici", 5, 8));
        let (a, b) = wrap(&mut t, a, b, "**", "**");
        assert_eq!((t.as_str(), a, b), ("un mot ici", 3, 6));
        let mut t = "é".to_string();
        assert_eq!(link(&mut t, 0, 1), (4, 12));
        assert_eq!(t, "[é](https://)");
    }

    #[test]
    fn shortcuts() {
        assert_eq!(shortcut(Kind::Text, "## ", 3), Some((Kind::H2, 3)));
        assert_eq!(shortcut(Kind::Text, "- ", 2), Some((Kind::Bullet, 2)));
        assert_eq!(shortcut(Kind::Bullet, "[ ] ", 4), Some((Kind::Todo(false), 4)));
        assert_eq!(shortcut(Kind::Text, "a - ", 4), None);
    }
}

/// Insère une mention (lien vers un mail, un événement…) à la position donnée d'un bloc.
pub fn insert_mention(note: &mut Note, ed: &mut Ed, block: u64, pos: usize, label: &str, url: &str) {
    let label: String = label.chars().filter(|c| !matches!(c, '[' | ']' | '\n' | '\r')).take(80).collect();
    let mention = format!("[{label}]({url}) ");
    let idx = note.blocks.iter().position(|b| b.id == block && !matches!(b.kind, Kind::Divider | Kind::Image | Kind::Code));
    let (bid, end) = match idx {
        Some(i) => {
            let text = &mut note.blocks[i].text;
            let at = byte(text, pos.min(chars(text)));
            text.insert_str(at, &mention);
            (note.blocks[i].id, pos.min(chars(&note.blocks[i].text)) + chars(&mention))
        }
        None => {
            let nb = note.block(Kind::Text, &mention);
            let id = nb.id;
            note.blocks.push(nb);
            (id, chars(&mention))
        }
    };
    ed.focus = Some((bid, end, end));
    note.touch();
}

/// Remplace « :préfixe » (à la position `at`) par l'émoji ; renvoie le curseur après l'émoji.
fn insert_emoji_code(text: &mut String, at: usize, prefix_len: usize, e: &str) -> usize {
    let (a, b) = (byte(text, at), byte(text, at + 1 + prefix_len));
    let ins = format!("{e} ");
    text.replace_range(a..b, &ins);
    emoji::remember(e);
    at + chars(&ins)
}

/// Insère du texte à la position du curseur (bloc courant).
pub fn insert_text(note: &mut Note, ed: &mut Ed, t: &str) {
    let i = current(note, ed);
    let Some(b) = note.blocks.get_mut(i) else { return };
    if matches!(b.kind, Kind::Divider | Kind::Image) {
        return;
    }
    let (lo, hi) = ed.cursor.filter(|c| c.0 == b.id).map(|c| (c.1.min(c.2), c.1.max(c.2))).unwrap_or((chars(&b.text), chars(&b.text)));
    let (bl, bh) = (byte(&b.text, lo), byte(&b.text, hi));
    b.text.replace_range(bl..bh, t);
    let c = lo + chars(t);
    ed.focus = Some((b.id, c, c));
    ed.cursor = Some((b.id, c, c));
    emoji::remember(t);
    note.touch();
}

/// Bandeau de suggestions sous le curseur pendant la frappe de « :nom ».
fn emoji_suggestions(ui: &mut egui::Ui, ed: &Ed) -> Option<String> {
    let (_, _, prefix, pos) = ed.emoji.as_ref()?;
    let list = emoji::suggestions(ui.ctx(), prefix, 8);
    if list.is_empty() {
        return None;
    }
    let mut out = None;
    egui::Area::new(egui::Id::unique("wt_note_emoji_sugg")).order(egui::Order::Foreground).fixed_pos(*pos + vec2(0.0, 4.0)).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for e in &list {
                    if crate::emoji_img::button(ui, e.as_str(), 28.0).on_hover_text(format!(":{}:", e.shortcode().unwrap_or(e.name()))).clicked() {
                        out = Some(e.as_str().to_string());
                    }
                }
                ui.label(egui::RichText::new("Tab").small().color(theme::TEXT_FAINT()));
            });
        });
    });
    out
}

/// Sélecteur complet (même que Google Chat) ouvert depuis la barre d'outils.
fn emoji_picker(ui: &mut egui::Ui, ed: &mut Ed) -> Option<String> {
    let pos = ed.picker?;
    let recent = emoji::recent();
    let (mut q, mut g) = (std::mem::take(&mut ed.emoji_query), ed.emoji_group);
    let mut picked = None;
    let area = egui::Area::new(egui::Id::unique("wt_note_emoji_picker")).order(egui::Order::Foreground).fixed_pos(pos).constrain(true).show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            picked = emoji::picker(ui, &mut q, &mut g, &recent);
        });
    });
    ed.emoji_query = q;
    ed.emoji_group = g;
    let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !Rect::from_center_size(pos - vec2(0.0, 20.0), vec2(40.0, 40.0)).contains(p)));
    if picked.is_some() || outside || ui.input(|i| i.key_pressed(Key::Escape)) {
        ed.picker = None;
    }
    picked
}
