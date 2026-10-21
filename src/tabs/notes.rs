//! Prise de notes : favoris et liste à gauche, éditeur / lecteur sans bord à droite.
//! Notes enregistrées en local (Markdown) — voir `notes_store`.

pub mod editor;
pub mod inline;
mod links;

use std::time::{Duration, Instant};

use chrono::{Local, NaiveDate, TimeZone};
use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::google::Account;
use crate::google::calendar::CalEvent;
use crate::google::drive::DriveFile;
use crate::google::gmail::Summary;
use crate::notes_store::{self, Kind, Note, NoteLink};
use crate::tabs::calendar::CalendarState;
use crate::tabs::mail::MailState;
use crate::tabs::messages::ChatState;
use crate::tasks::Tasks;
use crate::{theme, widgets};

const LIST_W: f32 = 270.0;
const COLUMN_W: f32 = 760.0;

pub enum NotesAction {
    Chat(String),
    Task,
    Event(CalEvent),
    Date(NaiveDate),
    Mail(String),
    Drive(DriveFile),
}

struct Password {
    id: String,
    a: String,
    b: String,
    error: Option<String>,
}

#[derive(Default)]
pub struct NotesState {
    loaded: bool,
    pub notes: Vec<Note>,
    selected: Option<String>,
    query: String,
    ed: editor::Ed,
    focus_title: bool,
    picker: Option<links::Picker>,
    password: Option<Password>,
    unlock: String,
    unlock_error: Option<String>,
    unlock_focus: bool,
    confirm_delete: Option<String>,
    notice: Option<(String, Instant)>,
    /// Mention à insérer après le choix dans le sélecteur (« / ») : bloc, position.
    mention: Option<(u64, usize)>,
    /// Focus à rendre au texte quelques images après la fermeture d'une fenêtre.
    refocus: Option<((u64, usize, usize), u8)>,
}

impl Drop for NotesState {
    fn drop(&mut self) {
        for n in self.notes.iter_mut().filter(|n| n.dirty) {
            let _ = n.save();
        }
    }
}

impl NotesState {
    fn ensure_loaded(&mut self) {
        if !self.loaded {
            self.loaded = true;
            self.notes = Note::load_all();
            self.selected = self.notes.first().map(|n| n.id.clone());
        }
    }

    /// Résultats Drive d'une recherche lancée par le sélecteur de liens.
    pub fn on_drive(&mut self, query: &str, files: Vec<DriveFile>) -> Option<Vec<DriveFile>> {
        match &mut self.picker {
            Some(p) if query.starts_with(links::PREFIX) => {
                p.on_drive(query, files);
                None
            }
            _ if query.starts_with(links::PREFIX) => None,
            _ => Some(files),
        }
    }

    pub fn on_mail(&mut self, query: &str, list: Vec<Summary>) -> Option<Vec<Summary>> {
        match &mut self.picker {
            Some(p) if query.starts_with(links::PREFIX) => {
                p.on_mail(query, list);
                None
            }
            _ if query.starts_with(links::PREFIX) => None,
            _ => Some(list),
        }
    }

    /// Sauvegarde automatique (une seconde après la dernière modification).
    pub fn background(&mut self, ctx: &egui::Context) {
        self.ensure_loaded();
        for n in self.notes.iter_mut().filter(|n| n.dirty) {
            match n.edited_at {
                Some(t) if t.elapsed() < Duration::from_millis(900) => ctx.request_repaint_after(Duration::from_millis(950) - t.elapsed()),
                _ => {
                    if let Err(e) = n.save() {
                        self.notice = Some((format!("Enregistrement impossible : {e}"), Instant::now()));
                    }
                }
            }
        }
    }

    pub fn select(&mut self, id: &str) {
        if self.selected.as_deref() == Some(id) {
            return;
        }
        // La note protégée qu'on quitte est reverrouillée.
        if let Some(prev) = self.selected.take() {
            if let Some(n) = self.notes.iter_mut().find(|n| n.id == prev) {
                if n.dirty && !n.locked() {
                    let _ = n.save();
                }
                n.relock();
            }
        }
        self.selected = Some(id.to_string());
        self.ed = editor::Ed::default();
        self.unlock.clear();
        self.unlock_error = None;
        self.unlock_focus = true;
    }

    pub fn new_note(&mut self) {
        self.ensure_loaded();
        let n = Note::new();
        let id = n.id.clone();
        self.notes.insert(0, n);
        self.select(&id);
        self.focus_title = true;
    }

    /// Entrées pour la recherche globale : (id, titre, extrait, modifiée, protégée).
    pub fn entries(&self) -> Vec<(String, String, String, i64, bool)> {
        self.notes.iter().map(|n| (n.id.clone(), n.display_title(), inline::strip(&n.plain_text()), n.updated, n.protected)).collect()
    }
}

fn ago(ms: i64) -> String {
    let Some(t) = Local.timestamp_millis_opt(ms).single() else { return String::new() };
    let now = Local::now();
    if t.date_naive() == now.date_naive() {
        format!("{}", t.format("%H:%M"))
    } else if (now.date_naive() - t.date_naive()).num_days() < 7 {
        crate::tabs::weekday_fr(chrono::Datelike::weekday(&t.date_naive())).chars().take(3).collect::<String>() + "."
    } else {
        t.format("%d/%m/%y").to_string()
    }
}

fn long(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|t| crate::tabs::mail::view::long_date(t.timestamp_millis())).unwrap_or_default()
}

/// Cadenas dessiné (fermé ou ouvert).
pub fn draw_lock(p: &egui::Painter, c: Pos2, s: f32, open: bool, color: Color32) {
    let body = Rect::from_center_size(c + vec2(0.0, s * 0.22), vec2(s * 0.8, s * 0.58));
    p.rect_filled(body, CornerRadius::same((s * 0.12) as u8), color);
    let r = s * 0.24;
    let cx = c.x + if open { s * 0.3 } else { 0.0 };
    let top = body.top();
    let pts: Vec<Pos2> = (0..=12)
        .map(|k| {
            let a = std::f32::consts::PI * (k as f32 / 12.0);
            Pos2::new(cx - r * a.cos(), top - s * 0.18 - r * a.sin())
        })
        .collect();
    let mut line = vec![Pos2::new(cx - r, top)];
    line.extend(pts);
    line.push(Pos2::new(cx + r, if open { top - s * 0.18 } else { top }));
    p.line(line, Stroke::new(s * 0.11, color));
}

fn draw_page(p: &egui::Painter, c: Pos2, s: f32, color: Color32) {
    let r = Rect::from_center_size(c, vec2(s * 0.72, s * 0.9));
    p.rect_stroke(r, CornerRadius::same(2), Stroke::new(1.2, color), egui::StrokeKind::Inside);
    for k in 0..3 {
        let y = r.top() + s * (0.26 + k as f32 * 0.18);
        p.line_segment([Pos2::new(r.left() + s * 0.16, y), Pos2::new(r.right() - s * 0.16, y)], Stroke::new(1.0, color));
    }
}

#[allow(clippy::too_many_arguments)]
pub fn show(ui: &mut egui::Ui, st: &mut NotesState, cal: &CalendarState, mail: &MailState, chat: &ChatState, tasks: &Tasks, acc: &Account) -> Option<NotesAction> {
    st.ensure_loaded();
    st.notes.sort_by_key(|n| -n.updated);
    let n = st.notes.len();
    widgets::page_header_with(ui, "Notes", "Prise de notes", |ui| {
        widgets::header_badge(ui, &format!("{n} note{}", if n > 1 { "s" } else { "" }), false);
    });
    let full = ui.available_rect_before_wrap();
    let list_rect = Rect::from_min_size(full.min, vec2(LIST_W, full.height()));
    let main_rect = Rect::from_min_max(Pos2::new(full.left() + LIST_W + 28.0, full.top()), full.max);
    let mut action = None;

    ui.scope_builder(egui::UiBuilder::new().max_rect(list_rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        widgets::pane(ui, |ui| list(ui, st));
    });
    ui.scope_builder(egui::UiBuilder::new().max_rect(main_rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
        action = main(ui, st, cal, mail, acc);
    });
    ui.allocate_rect(full, Sense::hover());

    dialogs(ui.ctx(), st, cal, mail, chat, tasks, acc);
    // La fenêtre modale bloque le focus encore une image ou deux après sa fermeture.
    if st.picker.is_none() {
        if let Some((f, n)) = st.refocus.take() {
            if n == 0 {
                st.ed.focus = Some(f);
            } else {
                st.refocus = Some((f, n - 1));
            }
            ui.ctx().request_repaint();
        }
    }
    if let Some((text, at)) = &st.notice {
        if at.elapsed() > Duration::from_secs(4) {
            st.notice = None;
        } else {
            let text = text.clone();
            egui::Area::new(egui::Id::unique("wt_notes_toast")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, -28.0)).show(ui.ctx(), |ui| {
                egui::Frame::new().fill(theme::SURFACE_HI()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(10)).inner_margin(Margin::symmetric(14, 9)).show(ui, |ui| {
                    ui.label(text);
                });
            });
        }
    }
    action
}

// --- Colonne de gauche -------------------------------------------------------------------

fn list(ui: &mut egui::Ui, st: &mut NotesState) {
    let new = ui.add_sized([ui.available_width(), 34.0], egui::Button::new(RichText::new("＋  Nouvelle note").strong()).corner_radius(CornerRadius::same(10)));
    if new.clicked() {
        st.new_note();
    }
    ui.add_space(4.0);
    ui.add(egui::TextEdit::singleline(&mut st.query).hint_text("🔍  Rechercher").desired_width(f32::INFINITY).margin(vec2(10.0, 6.0)));
    ui.add_space(10.0);

    let q = st.query.trim().to_lowercase();
    let matches = |n: &Note| q.is_empty() || n.title.to_lowercase().contains(&q) || n.plain_text().to_lowercase().contains(&q);
    let favs: Vec<usize> = (0..st.notes.len()).filter(|&i| st.notes[i].favorite && matches(&st.notes[i])).collect();
    let all: Vec<usize> = (0..st.notes.len()).filter(|&i| matches(&st.notes[i])).collect();

    let mut clicked = None;
    let mut ctx_action: Option<(String, &'static str)> = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        if !favs.is_empty() {
            section(ui, "FAVORIS", favs.len());
            for &i in &favs {
                note_row(ui, &st.notes[i], st.selected.as_deref() == Some(&st.notes[i].id), "fav", &mut clicked, &mut ctx_action);
            }
            ui.add_space(14.0);
        }
        section(ui, "NOTES", all.len());
        if all.is_empty() {
            widgets::empty_state(ui, if q.is_empty() { "Aucune note" } else { "Aucun résultat" }, if q.is_empty() { "Crée ta première note." } else { "" });
        }
        for &i in &all {
            note_row(ui, &st.notes[i], st.selected.as_deref() == Some(&st.notes[i].id), "all", &mut clicked, &mut ctx_action);
        }
    });
    if let Some(id) = clicked {
        st.select(&id);
    }
    if let Some((id, what)) = ctx_action {
        note_action(st, &id, what);
    }
}

fn section(ui: &mut egui::Ui, title: &str, n: usize) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).small().strong().color(theme::TEXT_MUTED()));
        ui.label(RichText::new(n.to_string()).small().color(theme::TEXT_FAINT()));
    });
    ui.add_space(4.0);
}

fn note_row(ui: &mut egui::Ui, n: &Note, selected: bool, salt: &str, clicked: &mut Option<String>, act: &mut Option<(String, &'static str)>) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 32.0), Sense::hover());
    let id = egui::Id::unique(("wt_note_row", salt, &n.id));
    let resp = ui.interact(r, id, Sense::click());
    // Survol de toute la ligne (y compris l'étoile, qui passe devant la ligne).
    let over = ui.rect_contains_pointer(r);
    if selected {
        ui.painter().rect_filled(r, CornerRadius::same(8), theme::SURFACE_HI());
    } else if over {
        ui.painter().rect_filled(r, CornerRadius::same(8), crate::theme::gray(0x1B));
    }
    let p = ui.painter();
    let ic = Pos2::new(r.left() + 16.0, r.center().y);
    let col = if selected { theme::TEXT() } else { theme::TEXT_MUTED() };
    if n.protected {
        draw_lock(p, ic, 13.0, !n.locked(), col);
    } else {
        draw_page(p, ic, 14.0, col);
    }
    let star_r = Rect::from_center_size(Pos2::new(r.right() - 16.0, r.center().y), vec2(22.0, 22.0));
    let date = ago(n.updated);
    let dg = p.layout_no_wrap(date, egui::FontId::proportional(11.0), theme::TEXT_FAINT());
    let right = if over || n.favorite { star_r.left() - 4.0 } else { r.right() - 10.0 - dg.size().x - 6.0 };
    let title = crate::tabs::mail::view::one_line(p, &n.display_title(), egui::FontId::proportional(13.5), if selected { theme::TEXT() } else { crate::theme::gray(0xCC) }, right - r.left() - 34.0);
    p.galley(Pos2::new(r.left() + 32.0, r.center().y - title.size().y / 2.0), title, theme::TEXT());
    if over || n.favorite {
        let s = ui.interact(star_r, id.with("star"), Sense::click()).on_hover_text(if n.favorite { "Retirer des favoris" } else { "Ajouter aux favoris" });
        let c = if n.favorite { theme::TODAY() } else if s.hovered() { theme::TEXT() } else { theme::TEXT_FAINT() };
        crate::tabs::mail::view::draw_star(ui.painter(), star_r.center(), 6.5, n.favorite, c);
        if s.clicked() {
            *act = Some((n.id.clone(), "favorite"));
        }
    } else {
        ui.painter().galley(Pos2::new(r.right() - 10.0 - dg.size().x, r.center().y - dg.size().y / 2.0), dg, theme::TEXT_FAINT());
    }
    if resp.clicked() {
        *clicked = Some(n.id.clone());
    }
    resp.context_menu(|ui| {
        if ui.button(if n.favorite { "☆  Retirer des favoris" } else { "★  Ajouter aux favoris" }).clicked() {
            *act = Some((n.id.clone(), "favorite"));
        }
        if !n.locked() && ui.button(if n.readonly { "✏  Débloquer l'éditeur" } else { "🔏  Bloquer l'éditeur" }).clicked() {
            *act = Some((n.id.clone(), "readonly"));
        }
        if !n.protected && ui.button("🔒  Protéger par mot de passe…").clicked() {
            *act = Some((n.id.clone(), "protect"));
        }
        ui.separator();
        if ui.button(RichText::new("🗑  Supprimer").color(theme::TODAY())).clicked() {
            *act = Some((n.id.clone(), "delete"));
        }
    });
}

fn note_action(st: &mut NotesState, id: &str, what: &str) {
    match what {
        "delete" => st.confirm_delete = Some(id.to_string()),
        "protect" => st.password = Some(Password { id: id.to_string(), a: String::new(), b: String::new(), error: None }),
        _ => {
            if let Some(n) = st.notes.iter_mut().find(|n| n.id == id) {
                match what {
                    "favorite" => n.favorite = !n.favorite,
                    "readonly" => n.readonly = !n.readonly,
                    _ => {}
                }
                // Changement d'état : enregistré sans changer la date de modification.
                n.dirty = true;
                n.edited_at = None;
            }
        }
    }
}

// --- Éditeur / lecteur -------------------------------------------------------------------

fn main(ui: &mut egui::Ui, st: &mut NotesState, cal: &CalendarState, mail: &MailState, acc: &Account) -> Option<NotesAction> {
    let _ = (cal, mail, acc);
    let Some(idx) = st.selected.as_ref().and_then(|id| st.notes.iter().position(|n| &n.id == id)) else {
        ui.add_space(ui.available_height() * 0.3);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("Aucune note ouverte").size(18.0).color(theme::TEXT_MUTED()));
            ui.add_space(8.0);
            if ui.button("＋  Nouvelle note").clicked() {
                st.new_note();
            }
        });
        return None;
    };
    let mut action = None;
    let locked = st.notes[idx].locked();
    let edit = !locked && !st.notes[idx].readonly;

    // Barre du haut : mise en forme (édition) ou état, puis actions de la note.
    let mut bar_act: Option<&'static str> = None;
    ui.horizontal(|ui| {
        ui.set_height(32.0);
        let note = &mut st.notes[idx];
        let msg = if locked {
            "Note protégée — contenu chiffré"
        } else if note.readonly {
            "Lecture seule"
        } else if note.dirty {
            "Enregistrement…"
        } else {
            "Enregistrée sur cet ordinateur"
        };
        ui.label(RichText::new(msg).small().color(theme::TEXT_MUTED()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.menu_button(RichText::new("⋯").size(18.0), |ui| {
                if ui.button("📂  Ouvrir le dossier des notes").clicked() {
                    if let Some(d) = notes_store::dir() {
                        let _ = std::fs::create_dir_all(&d);
                        let _ = open::that(d);
                    }
                }
                if ui.button("🗑  Supprimer la note").clicked() {
                    bar_act = Some("delete");
                }
            });
            if !locked {
                // Mot de passe.
                if note.protected {
                    ui.menu_button("🔓", |ui| {
                        if ui.button("🔒  Verrouiller maintenant").clicked() {
                            bar_act = Some("relock");
                        }
                        if ui.button("Retirer le mot de passe").clicked() {
                            bar_act = Some("unprotect");
                        }
                    })
                    .response
                    .on_hover_text("Note protégée (déverrouillée)");
                } else if ui.add(egui::Button::new("🔒").frame_when_inactive(false)).on_hover_text("Protéger par mot de passe").clicked() {
                    bar_act = Some("protect");
                }
                if !note.readonly && ui.add(egui::Button::new("🔗 Rattacher").frame_when_inactive(false)).on_hover_text("Lier un événement, une date, un mail ou un fichier Drive").clicked() {
                    bar_act = Some("link");
                }
                // Bloquer / débloquer l'éditeur.
                let mut mode = note.readonly;
                if widgets::segmented(ui, "note_mode", &[(true, "Lecture"), (false, "Édition")], &mut mode) {
                    bar_act = Some("readonly");
                }
            }
            let (sr, sresp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
            let sresp = sresp.on_hover_text(if note.favorite { "Retirer des favoris" } else { "Ajouter aux favoris" });
            if sresp.hovered() {
                ui.painter().rect_filled(sr, CornerRadius::same(6), theme::SURFACE_HI());
            }
            crate::tabs::mail::view::draw_star(ui.painter(), sr.center(), 7.5, note.favorite, if note.favorite { theme::TODAY() } else { theme::TEXT_MUTED() });
            if sresp.clicked() {
                bar_act = Some("favorite");
            }
        });
    });
    let id = st.notes[idx].id.clone();
    match bar_act {
        Some("link") => {
            st.picker = Some(links::Picker::new(links::Tab::Event));
            st.mention = None;
        }
        Some("relock") => {
            st.notes[idx].relock();
            st.unlock_focus = true;
        }
        Some("unprotect") => {
            if let Err(e) = st.notes[idx].unprotect() {
                st.notice = Some((e, Instant::now()));
            } else {
                st.notice = Some(("Mot de passe retiré".into(), Instant::now()));
            }
        }
        Some(what) => note_action(st, &id, what),
        None => {}
    }
    if edit {
        ui.horizontal(|ui| {
            ui.set_height(32.0);
            toolbar(ui, &mut st.notes[idx], &mut st.ed);
        });
    }
    ui.add_space(2.0);
    let sep = ui.available_rect_before_wrap();
    ui.painter().line_segment([sep.left_top(), sep.right_top()], Stroke::new(1.0, theme::BORDER()));
    ui.add_space(4.0);

    // Dépôt de fichiers sur l'éditeur.
    if edit {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let dropped: Vec<std::path::PathBuf> = ui.ctx().input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect());
        let area = ui.available_rect_before_wrap();
        if hovering {
            ui.painter().rect_stroke(area.shrink(4.0), CornerRadius::same(12), Stroke::new(1.5, theme::TEXT_MUTED()), egui::StrokeKind::Inside);
            ui.painter().text(area.center_top() + vec2(0.0, 30.0), egui::Align2::CENTER_CENTER, "Dépose pour insérer dans la note", egui::FontId::proportional(14.0), theme::TEXT());
        }
        if !dropped.is_empty() {
            let note = &mut st.notes[idx];
            let at = editor::current(note, &st.ed) + 1;
            editor::insert_files(note, &mut st.ed, at, true, Some(dropped));
        }
    }

    let width = ui.available_width();
    let col_w = width.min(COLUMN_W);
    let pad = ((width - col_w) / 2.0).max(0.0);
    egui::ScrollArea::vertical().id_salt(("wt_note_scroll", &id)).auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal_top(|ui| {
            ui.add_space(pad);
            ui.vertical(|ui| {
                ui.set_width(col_w);
                ui.add_space(26.0);
                action = page(ui, st, idx, edit, locked);
            });
        });
    });

    if let Some(e) = st.ed.error.take() {
        st.notice = Some((e, Instant::now()));
    }
    action
}

fn page(ui: &mut egui::Ui, st: &mut NotesState, idx: usize, edit: bool, locked: bool) -> Option<NotesAction> {
    let mut action = None;
    let note = &mut st.notes[idx];
    // Titre.
    if edit {
        let id = egui::Id::unique(("wt_note_title", &note.id));
        let r = ui.add(
            egui::TextEdit::multiline(&mut note.title)
                .id(id)
                .frame(egui::Frame::NONE)
                .font(theme::title_font(34.0))
                .hint_text(RichText::new("Sans titre").color(theme::TEXT_FAINT()).font(theme::title_font(34.0)))
                .desired_rows(1)
                .return_key(None)
                .desired_width(f32::INFINITY),
        );
        if st.focus_title {
            r.request_focus();
            st.focus_title = false;
        }
        if r.changed() {
            note.title = note.title.replace('\n', " ");
            note.touch();
        }
        if r.has_focus() && ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) || i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown)) {
            if let Some(b) = note.blocks.iter().find(|b| !matches!(b.kind, Kind::Divider | Kind::Image)) {
                st.ed.focus = Some((b.id, 0, 0));
            }
        }
    } else {
        ui.label(RichText::new(note.display_title()).font(theme::title_font(34.0)).color(if note.title.is_empty() { theme::TEXT_FAINT() } else { theme::TEXT() }));
    }
    ui.add_space(4.0);
    ui.label(RichText::new(format!("Modifiée {} · créée {}", long(note.updated), long(note.created))).small().color(theme::TEXT_FAINT()));
    ui.add_space(10.0);

    if locked {
        locked_view(ui, st, idx);
        return None;
    }

    // Liens rattachés.
    if !note.links.is_empty() {
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            for (k, l) in note.links.iter().enumerate() {
                let (open, del) = chip(ui, l, edit);
                if open {
                    action = Some(link_action(l));
                }
                if del {
                    remove = Some(k);
                }
            }
            if edit && ui.add(egui::Button::new(RichText::new("＋").color(theme::TEXT_MUTED())).frame_when_inactive(false)).on_hover_text("Rattacher").clicked() {
                st.picker = Some(links::Picker::new(links::Tab::Event));
            }
        });
        if let Some(k) = remove {
            note.links.remove(k);
            note.touch();
        }
        ui.add_space(14.0);
    }

    let note = &mut st.notes[idx];
    if let Some(url) = editor::body(ui, note, &mut st.ed, edit) {
        match url.strip_prefix("wt:") {
            Some(key) => action = mention_action(note, key),
            None => open_url(note, &url),
        }
    }
    // « / » → Mail, Message, Événement, Tâche… : ouvre le sélecteur, la mention suivra.
    if let Some((tab, block, pos)) = st.ed.link_request.take() {
        st.picker = Some(links::Picker::new(tab));
        st.mention = Some((block, pos));
    }
    action
}

/// Clic sur une mention dans le texte (`wt:<clé>`).
fn mention_action(note: &Note, key: &str) -> Option<NotesAction> {
    if let Some(l) = note.links.iter().find(|l| l.key() == key) {
        return Some(link_action(l));
    }
    // Lien détaché des pastilles : ce que la clé suffit à retrouver.
    let (kind, id) = key.split_once(':')?;
    match kind {
        "date" => NaiveDate::parse_from_str(id, "%Y-%m-%d").ok().map(NotesAction::Date),
        "mail" => Some(NotesAction::Mail(id.to_string())),
        "task" => Some(NotesAction::Task),
        "chat" => Some(NotesAction::Chat(id.split("/messages/").next().unwrap_or(id).to_string())),
        _ => None,
    }
}

fn link_action(l: &NoteLink) -> NotesAction {
    match l {
        NoteLink::Chat { space, .. } => NotesAction::Chat(space.clone()),
        NoteLink::Task { .. } => NotesAction::Task,
        NoteLink::Event { calendar, id, title, start } => {
            NotesAction::Event(CalEvent { id: id.clone(), calendar: calendar.clone(), title: title.clone(), start: *start, end: *start + 3_600_000, ..Default::default() })
        }
        NoteLink::Date { date } => NotesAction::Date(NaiveDate::parse_from_str(date, "%Y-%m-%d").unwrap_or_else(|_| Local::now().date_naive())),
        NoteLink::Mail { id, .. } => NotesAction::Mail(id.clone()),
        NoteLink::Drive { id, name, mime, url } => NotesAction::Drive(DriveFile { id: id.clone(), name: name.clone(), mime: mime.clone(), web_link: url.clone(), ..Default::default() }),
    }
}

/// Pastille d'un lien : (ouvrir, retirer).
fn chip(ui: &mut egui::Ui, l: &NoteLink, edit: bool) -> (bool, bool) {
    let (icon, label, sub) = match l {
        NoteLink::Event { title, start, .. } => ("📅", title.clone(), long(*start)),
        NoteLink::Date { date } => ("📆", links::date_label(date), String::new()),
        NoteLink::Mail { subject, from, .. } => ("✉", subject.clone(), from.clone()),
        NoteLink::Drive { name, mime, .. } => (links::file_icon(mime), name.clone(), String::new()),
        NoteLink::Chat { title, text, .. } => ("💬", title.clone(), text.clone()),
        NoteLink::Task { title, .. } => ("✅", title.clone(), String::new()),
    };
    let mut cross = None;
    let resp = egui::Frame::new()
        .fill(theme::SURFACE())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(255))
        .inner_margin(Margin { left: 10, right: if edit { 4 } else { 12 }, top: 4, bottom: 4 })
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.label(RichText::new(icon).size(13.0));
            let short: String = if label.chars().count() > 38 { label.chars().take(36).collect::<String>() + "…" } else { label.clone() };
            ui.label(RichText::new(short).size(12.5));
            if edit {
                // Place de la croix ; elle est rendue cliquable après la pastille (par-dessus).
                cross = Some(ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover()).0);
            }
        })
        .response;
    let resp = ui.interact(resp.rect, egui::Id::unique(("wt_note_chip", format!("{l:?}"))), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    // Croix « Détacher » : interagie en dernier pour passer devant la pastille.
    let mut del = false;
    if let Some(x) = cross {
        let xr = ui.interact(x, egui::Id::unique(("wt_note_chip_x", format!("{l:?}"))), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Détacher de la note");
        let p = ui.painter();
        if xr.hovered() {
            p.circle_filled(x.center(), 9.0, theme::SURFACE_HI());
        }
        let c = x.center();
        let k = 3.5;
        let st = Stroke::new(1.4, if xr.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() });
        p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], st);
        p.line_segment([c + vec2(-k, k), c + vec2(k, -k)], st);
        del = xr.clicked();
    }
    let tip = match l {
        NoteLink::Event { .. } => "Ouvrir dans le calendrier",
        NoteLink::Date { .. } => "Afficher ce jour dans le calendrier",
        NoteLink::Mail { .. } => "Ouvrir le mail",
        NoteLink::Drive { .. } => "Afficher dans Fichiers",
        NoteLink::Chat { .. } => "Ouvrir la conversation",
        NoteLink::Task { .. } => "Voir dans le Tableau de bord",
    };
    let resp = if sub.is_empty() { resp.on_hover_text(tip) } else { resp.on_hover_text(format!("{label}\n{sub}\n{tip}")) };
    (resp.clicked() && !del, del)
}

/// Ouvre un lien du texte : site web, fichier joint (local ou d'une note protégée).
fn open_url(note: &Note, url: &str) {
    let rel = url.replace("%20", " ");
    if rel.starts_with("attachments/") {
        if let Some(bytes) = note.images.get(&rel) {
            // Note protégée : copie temporaire en clair, le temps de l'ouvrir.
            let name = rel.rsplit('/').next().unwrap_or("fichier");
            let p = std::env::temp_dir().join(name);
            if std::fs::write(&p, bytes).is_ok() {
                let _ = open::that(p);
            }
        } else if let Some(d) = notes_store::dir() {
            let _ = open::that(d.join(rel));
        }
    } else if url.starts_with("http://") || url.starts_with("https://") || url.starts_with("mailto:") {
        let _ = webbrowser::open(url);
    } else {
        let _ = open::that(url);
    }
}

fn toolbar(ui: &mut egui::Ui, note: &mut Note, ed: &mut editor::Ed) {
    ui.spacing_mut().item_spacing.x = 2.0;
    let cur = note.blocks.get(editor::current(note, ed)).map(|b| b.kind).unwrap_or(Kind::Text);
    let label = match cur {
        Kind::Divider | Kind::Image => "Texte",
        k => k.label(),
    };
    ui.menu_button(format!("{label}  ⏷"), |ui| {
        for k in [Kind::Text, Kind::H1, Kind::H2, Kind::H3, Kind::Bullet, Kind::Numbered, Kind::Todo(false), Kind::Quote, Kind::Code] {
            if ui.selectable_label(cur == k || (matches!(cur, Kind::Todo(_)) && matches!(k, Kind::Todo(_))), k.label()).clicked() {
                editor::set_kind(note, ed, k);
                ui.close();
            }
        }
    });
    sep(ui);
    let tb = |ui: &mut egui::Ui, text: RichText, tip: &str| ui.add(egui::Button::new(text).frame_when_inactive(false).min_size(vec2(28.0, 28.0))).on_hover_text(tip).clicked();
    if tb(ui, RichText::new("B").strong(), "Gras (Ctrl+B)") {
        editor::format(note, ed, "**", "**");
    }
    if tb(ui, RichText::new("I").italics(), "Italique (Ctrl+I)") {
        editor::format(note, ed, "*", "*");
    }
    if tb(ui, RichText::new("U").underline(), "Souligné (Ctrl+U)") {
        editor::format(note, ed, "<u>", "</u>");
    }
    if tb(ui, RichText::new("S").strikethrough(), "Barré (Ctrl+Maj+X)") {
        editor::format(note, ed, "~~", "~~");
    }
    if tb(ui, RichText::new("</>").monospace().size(12.0), "Code (Ctrl+E)") {
        editor::format(note, ed, "`", "`");
    }
    if tb(ui, RichText::new("H").background_color(theme::TODAY().gamma_multiply(0.45)), "Surligner (Ctrl+Maj+H)") {
        editor::format(note, ed, "==", "==");
    }
    if tb(ui, RichText::new("🔗"), "Lien (Ctrl+K)") {
        editor::format(note, ed, "[", "");
    }
    let er = ui.add(egui::Button::new(RichText::new("🙂")).frame_when_inactive(false).min_size(vec2(28.0, 28.0))).on_hover_text("Émojis (ou tape « :nom: »)");
    if er.clicked() {
        ed.picker = if ed.picker.is_some() { None } else { Some(er.rect.left_bottom() + vec2(0.0, 4.0)) };
    }
    sep(ui);
    for (icon, tip, k) in [("•", "Liste à puces", Kind::Bullet), ("1.", "Liste numérotée", Kind::Numbered), ("☑", "Case à cocher", Kind::Todo(false)), ("❝", "Citation", Kind::Quote), ("{ }", "Bloc de code", Kind::Code), ("—", "Séparateur", Kind::Divider)] {
        if tb(ui, RichText::new(icon), tip) {
            editor::set_kind(note, ed, k);
        }
    }
    sep(ui);
    let at = editor::current(note, ed) + 1;
    if tb(ui, RichText::new("🖼"), "Insérer une image (ou glisse-la dans la note)") {
        editor::insert_files(note, ed, at, true, None);
    }
    if tb(ui, RichText::new("📎"), "Joindre un fichier") {
        editor::insert_files(note, ed, at, false, None);
    }
}

fn sep(ui: &mut egui::Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(9.0, 20.0), Sense::hover());
    ui.painter().line_segment([r.center_top(), r.center_bottom()], Stroke::new(1.0, theme::BORDER()));
}

/// Note verrouillée : texte chiffré flouté et champ du mot de passe.
fn locked_view(ui: &mut egui::Ui, st: &mut NotesState, idx: usize) {
    let w = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(w, 340.0), Sense::hover());
    let p = ui.painter_at(r);
    // Faux paragraphes + données chiffrées, dessinés plusieurs fois décalés : effet de flou.
    let cipher = st.notes[idx].cipher_preview(900);
    let mut job = egui::text::LayoutJob::simple(cipher, egui::FontId::monospace(13.0), Color32::WHITE, w);
    job.wrap.max_width = w;
    let galley = ui.fonts_mut(|f| f.layout_job(job));
    let offsets = [-3.5f32, -1.8, 0.0, 1.8, 3.5];
    for dx in offsets {
        for dy in offsets {
            p.galley_with_override_text_color(r.min + vec2(dx, dy), galley.clone(), crate::theme::overlay(7));
        }
    }
    let mut y = r.top() + 6.0;
    for (k, ratio) in [0.92f32, 0.81, 0.97, 0.6, 0.0, 0.88, 0.94, 0.72, 0.0, 0.85, 0.5].iter().enumerate() {
        if *ratio > 0.0 {
            let bar = Rect::from_min_size(Pos2::new(r.left(), y), vec2(w * ratio, 12.0));
            for g in 0..4 {
                p.rect_filled(bar.expand(g as f32 * 2.0), CornerRadius::same(8), crate::theme::overlay(if k % 3 == 0 { 6 } else { 4 }));
            }
        }
        y += 28.0;
    }

    // Carte de déverrouillage au centre.
    let card = Rect::from_center_size(r.center(), vec2(320.0, 190.0));
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        egui::Frame::new()
            .fill(theme::SURFACE())
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(14))
            .inner_margin(Margin::same(18))
            .shadow(egui::Shadow { offset: [0, 8], blur: 28, spread: 0, color: Color32::from_black_alpha(150) })
            .show(ui, |ui| {
                ui.set_width(284.0);
                ui.vertical_centered(|ui| {
                    let (lr, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    draw_lock(ui.painter(), lr.center(), 22.0, false, theme::TEXT());
                    ui.label(RichText::new("Note protégée").strong());
                    ui.add_space(4.0);
                    let field = ui.add(egui::TextEdit::singleline(&mut st.unlock).password(true).hint_text("Mot de passe").desired_width(240.0).margin(vec2(10.0, 7.0)));
                    if st.unlock_focus {
                        field.request_focus();
                        st.unlock_focus = false;
                    }
                    let enter = field.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if let Some(e) = &st.unlock_error {
                        ui.label(RichText::new(e).small().color(theme::TODAY()));
                    }
                    ui.add_space(2.0);
                    if ui.add_sized([240.0, 30.0], egui::Button::new("Déverrouiller")).clicked() || enter {
                        match st.notes[idx].unlock(&st.unlock) {
                            Ok(()) => {
                                st.unlock.clear();
                                st.unlock_error = None;
                                st.ed = editor::Ed::default();
                            }
                            Err(e) => {
                                st.unlock.clear();
                                st.unlock_error = Some(e);
                                st.unlock_focus = true;
                            }
                        }
                    }
                });
            });
    });
}

// --- Fenêtres ----------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
fn dialogs(ctx: &egui::Context, st: &mut NotesState, cal: &CalendarState, mail: &MailState, chat: &ChatState, tasks: &Tasks, acc: &Account) {
    // Rattacher.
    if let Some(p) = &mut st.picker {
        let mut close = false;
        if let Some(link) = links::show(ctx, p, cal, mail, chat, tasks, acc, &mut close) {
            let mention = st.mention.take();
            let id = st.selected.clone();
            if let Some(n) = id.and_then(|id| st.notes.iter_mut().find(|n| n.id == id)) {
                if let Some((block, pos)) = mention {
                    let label = match &link {
                        NoteLink::Date { date } => format!("📆 {}", links::date_label(date)),
                        l => l.mention(),
                    };
                    editor::insert_mention(n, &mut st.ed, block, pos, &label, &format!("wt:{}", link.key()));
                    st.refocus = st.ed.focus.take().map(|f| (f, 3));
                }
                if !n.links.iter().any(|l| l.key() == link.key()) {
                    n.links.push(link);
                }
                n.touch();
            }
            close = true;
        }
        if close {
            st.picker = None;
            // Sélecteur fermé sans choix : retour au texte, au même endroit.
            if let Some((block, pos)) = st.mention.take() {
                st.refocus = Some(((block, pos, pos), 3));
            }
        }
    }

    // Suppression.
    if let Some(id) = st.confirm_delete.clone() {
        let title = st.notes.iter().find(|n| n.id == id).map(|n| n.display_title()).unwrap_or_default();
        let mut close = false;
        let r = egui::Modal::new(egui::Id::unique("wt_note_delete")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("Supprimer la note ?").size(16.0).strong());
            ui.add_space(4.0);
            ui.label(RichText::new(format!("« {title} » sera supprimée définitivement de ton ordinateur, avec ses images.")).color(theme::TEXT_MUTED()));
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Supprimer").color(theme::TODAY())).clicked() {
                    if let Some(k) = st.notes.iter().position(|n| n.id == id) {
                        let n = st.notes.remove(k);
                        n.delete();
                        if st.selected.as_deref() == Some(&id) {
                            st.selected = st.notes.first().map(|n| n.id.clone());
                            st.ed = editor::Ed::default();
                        }
                    }
                    close = true;
                }
                if ui.button("Annuler").clicked() {
                    close = true;
                }
            });
        });
        if close || r.should_close() {
            st.confirm_delete = None;
        }
    }

    // Mot de passe.
    if let Some(pw) = &mut st.password {
        let mut close = false;
        let mut submit = false;
        let r = egui::Modal::new(egui::Id::unique("wt_note_password")).show(ctx, |ui| {
            ui.set_width(360.0);
            ui.label(RichText::new("Protéger la note par mot de passe").size(16.0).strong());
            ui.add_space(4.0);
            ui.label(RichText::new("Le contenu, les liens et les images sont chiffrés (AES-256). Sans le mot de passe, la note reste illisible : il est impossible de le récupérer.").small().color(theme::TEXT_MUTED()));
            ui.add_space(10.0);
            let a = ui.add(egui::TextEdit::singleline(&mut pw.a).password(true).hint_text("Mot de passe").desired_width(f32::INFINITY));
            if pw.a.is_empty() && pw.b.is_empty() && pw.error.is_none() && !a.has_focus() {
                a.request_focus();
            }
            let b = ui.add(egui::TextEdit::singleline(&mut pw.b).password(true).hint_text("Confirmation").desired_width(f32::INFINITY));
            submit |= b.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if let Some(e) = &pw.error {
                ui.label(RichText::new(e).small().color(theme::TODAY()));
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                submit |= ui.button("🔒  Protéger").clicked();
                if ui.button("Annuler").clicked() {
                    close = true;
                }
            });
        });
        if submit {
            if pw.a != pw.b {
                pw.error = Some("Les deux mots de passe sont différents".into());
            } else {
                let id = pw.id.clone();
                let pass = pw.a.clone();
                match st.notes.iter_mut().find(|n| n.id == id) {
                    Some(n) if n.locked() => pw.error = Some("Déverrouille d'abord la note".into()),
                    Some(n) => match n.protect(&pass) {
                        Ok(()) => {
                            close = true;
                            st.notice = Some(("Note protégée — verrouillée quand tu la quittes".into(), Instant::now()));
                        }
                        Err(e) => pw.error = Some(e),
                    },
                    None => close = true,
                }
            }
        }
        if close || r.should_close() {
            st.password = None;
        }
    }
}
