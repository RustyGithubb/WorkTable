//! Gestion d'un groupe Google Chat (clic droit sur un groupe) : renommer, gérer les membres,
//! supprimer. Même panneau au centre que « Nouveau groupe » et la recherche globale.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Modifiers, Pos2, RichText, Sense, Stroke, vec2};

use super::ChatState;
use super::new_group::{avatar, magnifier, people_icon, rule};
use crate::google::chat::Member;
use crate::search::{Backdrop, overlay_frame};
use crate::tabs::mail::contacts::{Contacts, parse_mailbox};
use crate::theme;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Rename,
    Members,
    Delete,
}

/// Panneau ouvert (non enregistré).
pub struct Panel {
    pub space: String,
    pub mode: Mode,
    /// Nom du groupe à l'ouverture (titre du panneau).
    title: String,
    bg: Backdrop,
    name: String,
    query: String,
    selected: usize,
    /// Membres lus chez Google (`None` : en cours de lecture).
    pub members: Option<Result<Vec<Member>, String>>,
    /// Requête en cours (ajout, retrait, renommage, suppression).
    pub busy: bool,
    pub error: Option<String>,
    focus: bool,
}

/// Ce que l'utilisateur demande.
pub enum Request {
    Rename(String),
    Delete,
    Add(String),
    Remove(String),
    Close,
}

impl Panel {
    pub fn new(ctx: &egui::Context, space: &str, title: &str, mode: Mode) -> Self {
        let mut bg = Backdrop::default();
        bg.start(ctx);
        Panel {
            space: space.to_string(),
            mode,
            title: title.to_string(),
            bg,
            name: title.to_string(),
            query: String::new(),
            selected: 0,
            members: None,
            busy: false,
            error: None,
            focus: true,
        }
    }
}

struct Row {
    name: String,
    email: String,
    typed: bool,
}

pub fn show(ctx: &egui::Context, chat: &mut ChatState, book: &Contacts) -> Option<Request> {
    let mut panel = chat.manage.take()?;
    let out = body(ctx, &mut panel, chat, book);
    chat.manage = Some(panel);
    out
}

fn body(ctx: &egui::Context, p: &mut Panel, chat: &ChatState, book: &Contacts) -> Option<Request> {
    let screen = ctx.content_rect();
    let (k, backdrop_click) = p.bg.show(ctx, "wt_manage_backdrop");
    let mut out = backdrop_click.then_some(Request::Close);
    let query_id = egui::Id::unique("wt_manage_query");
    let name_id = egui::Id::unique("wt_manage_name");

    // Membres actuels (adresses, pour ne pas les reproposer).
    let current: Vec<(Member, String, String)> = match &p.members {
        Some(Ok(list)) => list
            .iter()
            .map(|m| {
                let name = if m.name.is_empty() { chat.name_of(&m.user) } else { m.name.clone() };
                (m.clone(), name, chat.email_of(&m.user).unwrap_or_default())
            })
            .collect(),
        _ => Vec::new(),
    };
    let q = p.query.trim().to_string();
    let mut rows: Vec<Row> = Vec::new();
    if p.mode == Mode::Members && !q.is_empty() {
        rows = book
            .search(&q, 30)
            .into_iter()
            .filter(|c| !current.iter().any(|(_, _, e)| e.eq_ignore_ascii_case(&c.email)))
            .take(6)
            .map(|c| Row { name: c.name.clone(), email: c.email.clone(), typed: false })
            .collect();
        if let Some((name, email)) = parse_mailbox(&q).filter(|(_, e)| e.contains('@'))
            && !rows.iter().any(|r| r.email.eq_ignore_ascii_case(&email))
            && !current.iter().any(|(_, _, e)| e.eq_ignore_ascii_case(&email))
        {
            rows.insert(0, Row { name, email, typed: true });
        }
    }
    p.selected = p.selected.min(rows.len().saturating_sub(1));

    let in_query = ctx.memory(|m| m.has_focus(query_id));
    let mut add: Option<usize> = None;
    let mut confirm = false;
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            out = Some(Request::Close);
        }
        if in_query {
            if i.consume_key(Modifiers::NONE, Key::ArrowDown) && !rows.is_empty() {
                p.selected = (p.selected + 1) % rows.len();
            }
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) && !rows.is_empty() {
                p.selected = (p.selected + rows.len() - 1) % rows.len();
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) && !rows.is_empty() {
                add = Some(p.selected);
            }
        } else if p.mode == Mode::Delete && i.consume_key(Modifiers::NONE, Key::Enter) {
            confirm = true;
        }
    });

    let w = 560.0_f32.min(screen.width() - 40.0);
    let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
    let mut remove: Option<String> = None;
    egui::Area::new(egui::Id::unique("wt_manage_box")).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
        ui.multiply_opacity(k);
        overlay_frame().show(ui, |ui| {
            ui.set_width(w - 28.0);
            let heading = match p.mode {
                Mode::Rename => t!("Renommer le groupe", "Rename the group"),
                Mode::Members => t!("Membres du groupe", "Group members"),
                Mode::Delete => t!("Supprimer le groupe", "Delete the group"),
            };
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                people_icon(ui.painter(), r.center(), theme::TEXT_MUTED());
                ui.label(RichText::new(heading).size(17.0).strong().color(theme::TEXT()));
                if p.busy {
                    ui.add(egui::Spinner::new().size(14.0));
                }
            });
            ui.label(RichText::new(&p.title).small().color(theme::TEXT_FAINT()));
            ui.add_space(8.0);
            match p.mode {
                Mode::Rename => {
                    let te = ui.add(
                        egui::TextEdit::singleline(&mut p.name)
                            .id(name_id)
                            .hint_text(t!("Nouveau nom", "New name"))
                            .font(FontId::proportional(18.0))
                            .frame(egui::Frame::NONE)
                            .desired_width(f32::INFINITY),
                    );
                    if std::mem::take(&mut p.focus) {
                        te.request_focus();
                    }
                    if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                        confirm = true;
                    }
                    ui.add_space(6.0);
                    rule(ui);
                }
                Mode::Delete => {
                    ui.label(
                        RichText::new(t!(
                            "Le groupe, ses messages et ses fichiers seront supprimés définitivement, pour tous ses membres. Seul un gestionnaire du groupe peut le faire.",
                            "The group, its messages and its files will be permanently deleted, for all its members. Only a group manager can do this."
                        ))
                        .color(theme::TEXT_MUTED()),
                    );
                }
                Mode::Members => members(ui, p, chat, &current, &rows, query_id, &mut add, &mut remove),
            }
            if let Some(e) = &p.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).small().color(theme::TODAY()));
            }
            ui.add_space(10.0);
            // Pied : boutons.
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match p.mode {
                        Mode::Rename => {
                            let ok = !p.name.trim().is_empty() && p.name.trim() != p.title.trim() && !p.busy;
                            if ui.add_enabled(ok, primary(t!("Renommer", "Rename"), None)).clicked() {
                                confirm = true;
                            }
                        }
                        Mode::Delete => {
                            if ui.add_enabled(!p.busy, primary(t!("Supprimer", "Delete"), Some(theme::TODAY()))).clicked() {
                                confirm = true;
                            }
                        }
                        Mode::Members => {}
                    }
                    let close = if p.mode == Mode::Members { t!("Fermer", "Close") } else { t!("Annuler", "Cancel") };
                    if ui.button(close).clicked() {
                        out = Some(Request::Close);
                    }
                });
            });
            ui.add_space(2.0);
            let help = match p.mode {
                Mode::Rename => t!("Entrée renommer · Échap fermer", "Enter rename · Esc close"),
                Mode::Delete => t!("Entrée supprimer · Échap annuler", "Enter delete · Esc cancel"),
                Mode::Members => t!("↑ ↓ choisir · Entrée ajouter · Échap fermer", "↑ ↓ choose · Enter add · Esc close"),
            };
            ui.label(RichText::new(help).small().color(theme::TEXT_FAINT()));
        });
    });
    ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("wt_manage_box")));

    if let Some(i) = add
        && let Some(r) = rows.get(i)
        && !p.busy
    {
        p.query.clear();
        p.selected = 0;
        p.focus = true;
        return Some(Request::Add(r.email.clone()));
    }
    if let Some(m) = remove
        && !p.busy
    {
        return Some(Request::Remove(m));
    }
    if confirm && !p.busy {
        match p.mode {
            Mode::Rename if !p.name.trim().is_empty() && p.name.trim() != p.title.trim() => return Some(Request::Rename(p.name.trim().to_string())),
            Mode::Delete => return Some(Request::Delete),
            _ => {}
        }
    }
    out
}

/// Liste des membres (retirer) et champ d'ajout (annuaire ou adresse).
#[allow(clippy::too_many_arguments)]
fn members(
    ui: &mut egui::Ui,
    p: &mut Panel,
    chat: &ChatState,
    current: &[(Member, String, String)],
    rows: &[Row],
    query_id: egui::Id,
    add: &mut Option<usize>,
    remove: &mut Option<String>,
) {
    let screen = ui.ctx().content_rect();
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
        magnifier(ui.painter(), r.center(), theme::TEXT_MUTED());
        let te = ui.add(
            egui::TextEdit::singleline(&mut p.query)
                .id(query_id)
                .hint_text(t!("Ajouter une personne : nom ou adresse", "Add a person: name or address"))
                .font(FontId::proportional(15.0))
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY),
        );
        if std::mem::take(&mut p.focus) {
            te.request_focus();
        }
        if te.changed() {
            p.selected = 0;
        }
    });
    ui.add_space(6.0);
    rule(ui);
    ui.add_space(4.0);
    egui::ScrollArea::vertical().max_height(screen.height() * 0.45).auto_shrink([false, true]).show(ui, |ui| {
        // Propositions d'ajout.
        if !rows.is_empty() {
            ui.label(RichText::new(t!("AJOUTER", "ADD")).small().strong().color(theme::TEXT_FAINT()));
            for (i, row) in rows.iter().enumerate() {
                let sel = i == p.selected;
                let title = if row.typed {
                    tf!("Ajouter « {} »", "Add “{}”", row.email)
                } else if row.name.is_empty() {
                    row.email.clone()
                } else {
                    row.name.clone()
                };
                let sub = if row.typed { t!("Adresse hors de l'annuaire", "Address not in the directory").to_string() } else { row.email.clone() };
                let resp = person_row(ui, &title, &sub, row.typed, sel, sel.then_some(t!("Entrée ↵", "Enter ↵")));
                if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                    p.selected = i;
                }
                if resp.clicked() {
                    *add = Some(i);
                }
            }
            ui.add_space(6.0);
        }
        // Membres actuels.
        match &p.members {
            None => {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(RichText::new(t!("Lecture des membres…", "Reading the members…")).color(theme::TEXT_MUTED()));
                });
            }
            Some(Err(e)) => {
                ui.label(RichText::new(tf!("Membres illisibles : {e}", "Members unreadable: {e}")).small().color(theme::TODAY()));
            }
            Some(Ok(_)) => {
                ui.label(RichText::new(tf!("MEMBRES · {}", "MEMBERS · {}", current.len())).small().strong().color(theme::TEXT_FAINT()));
                for (m, name, email) in current {
                    let me = chat.me.as_deref() == Some(m.user.as_str());
                    let mut sub = if email.is_empty() { String::new() } else { email.clone() };
                    if m.manager {
                        sub = if sub.is_empty() { t!("Gestionnaire", "Manager").to_string() } else { tf!("{sub} · gestionnaire", "{sub} · manager") };
                    }
                    let title = if me { tf!("{name} (vous)", "{name} (you)") } else { name.clone() };
                    let resp = person_row(ui, &title, &sub, false, false, None);
                    // Retirer : croix à droite, au survol (pas soi-même).
                    if !me && (resp.hovered() || ui.rect_contains_pointer(resp.rect)) {
                        let x = egui::Rect::from_center_size(Pos2::new(resp.rect.right() - 18.0, resp.rect.center().y), vec2(26.0, 26.0));
                        let xr = ui
                            .interact(x, resp.id.with("remove"), Sense::click())
                            .on_hover_cursor(egui::CursorIcon::PointingHand)
                            .on_hover_text(t!("Retirer du groupe", "Remove from the group"));
                        let ink = if xr.hovered() { theme::TODAY() } else { theme::TEXT_MUTED() };
                        let (c, k) = (x.center(), 4.0);
                        ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], Stroke::new(1.6, ink));
                        ui.painter().line_segment([c + vec2(k, -k), c + vec2(-k, k)], Stroke::new(1.6, ink));
                        if xr.clicked() {
                            *remove = Some(m.membership.clone());
                        }
                    }
                }
            }
        }
    });
}

/// Ligne d'une personne : initiale, nom, détail.
fn person_row(ui: &mut egui::Ui, title: &str, sub: &str, typed: bool, sel: bool, hint: Option<&str>) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let pa = ui.painter();
    if sel {
        pa.rect_filled(r, CornerRadius::same(8), theme::overlay(14));
        ui.scroll_to_rect(r, None);
    } else if resp.hovered() {
        pa.rect_filled(r, CornerRadius::same(8), theme::overlay(8));
    }
    avatar(pa, Pos2::new(r.left() + 18.0, r.center().y), title, typed);
    let left = r.left() + 40.0;
    let g1 = crate::tabs::mail::view::one_line(pa, title, theme::editor_font(14.0, sel), theme::TEXT(), r.right() - left - 80.0);
    let g2 = crate::tabs::mail::view::one_line(pa, sub, FontId::proportional(11.5), theme::TEXT_MUTED(), r.right() - left - 80.0);
    pa.galley(Pos2::new(left, r.top() + 3.0), g1, theme::TEXT());
    pa.galley(Pos2::new(left, r.top() + 22.0), g2, theme::TEXT_MUTED());
    if let Some(h) = hint {
        pa.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, h, FontId::proportional(11.0), theme::TEXT_FAINT());
    }
    resp
}

/// Bouton principal (en couleur pour une action destructrice).
fn primary(label: &str, color: Option<Color32>) -> egui::Button<'static> {
    let text = RichText::new(label.to_string()).strong();
    let text = match color {
        Some(c) => text.color(c),
        None => text,
    };
    egui::Button::new(text).corner_radius(CornerRadius::same(8)).min_size(vec2(96.0, 30.0))
}
