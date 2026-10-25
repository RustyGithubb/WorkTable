//! Création d'un groupe Google Chat, dans un panneau au centre de l'écran comme la recherche
//! globale (Ctrl+Espace) : fond flouté, type de groupe, nom, membres cherchés dans l'annuaire
//! (pilules), navigation au clavier.
//!
//! - ↑ ↓ : choisir une personne ; Entrée : l'ajouter (ou l'adresse tapée) ;
//! - Retour arrière dans le champ vide : retirer la dernière personne ;
//! - Ctrl+Entrée : créer ; Échap ou clic à côté : fermer.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, vec2};

use super::{ChatState, NewGroup};
use crate::search::{Backdrop, overlay_frame};
use crate::tabs::mail::contacts::{Contacts, parse_mailbox};
use crate::{theme, widgets};

/// État du panneau (non enregistré).
#[derive(Default)]
pub struct Panel {
    bg: Backdrop,
    query: String,
    selected: usize,
    /// Personnes ajoutées : (nom, adresse).
    picked: Vec<(String, String)>,
    /// Focus à donner au premier affichage.
    focus: bool,
    /// Focus à donner au champ des personnes à l'image suivante (après Entrée dans le nom).
    focus_query: bool,
}

/// Ce que l'utilisateur a demandé.
pub enum Outcome {
    Create,
    Close,
}

/// Ouvre le panneau (bouton « Nouveau groupe »).
pub fn open(ctx: &egui::Context, chat: &mut ChatState) {
    chat.new_group = Some(NewGroup { space: true, ..Default::default() });
    chat.group_panel = Panel { focus: true, ..Default::default() };
    chat.group_panel.bg.start(ctx);
}

/// Personne proposée dans la liste.
struct Row {
    name: String,
    email: String,
    /// Adresse tapée, absente de l'annuaire.
    typed: bool,
}

const NAME_ID: &str = "wt_new_group_name";
const QUERY_ID: &str = "wt_new_group_query";

pub fn show(ctx: &egui::Context, chat: &mut ChatState, book: &Contacts) -> Option<Outcome> {
    let g = chat.new_group.as_mut()?;
    let p = &mut chat.group_panel;
    let screen = ctx.content_rect();
    let (k, backdrop_click) = p.bg.show(ctx, "wt_new_group_backdrop");
    let mut out = backdrop_click.then_some(Outcome::Close);

    // Propositions : l'annuaire (sans les personnes déjà ajoutées), et l'adresse tapée si elle
    // n'y est pas.
    let q = p.query.trim().to_string();
    let mut rows: Vec<Row> = book
        .search(&q, 30)
        .into_iter()
        .filter(|c| !p.picked.iter().any(|(_, e)| e.eq_ignore_ascii_case(&c.email)))
        .take(7)
        .map(|c| Row { name: c.name.clone(), email: c.email.clone(), typed: false })
        .collect();
    if let Some((name, email)) = parse_mailbox(&q).filter(|(_, e)| e.contains('@'))
        && !rows.iter().any(|r| r.email.eq_ignore_ascii_case(&email))
        && !p.picked.iter().any(|(_, e)| e.eq_ignore_ascii_case(&email))
    {
        rows.insert(0, Row { name, email, typed: true });
    }
    p.selected = p.selected.min(rows.len().saturating_sub(1));

    let name_id = egui::Id::unique(NAME_ID);
    let query_id = egui::Id::unique(QUERY_ID);
    let in_query = ctx.memory(|m| m.has_focus(query_id));
    let mut add: Option<usize> = None;
    let mut pop = false;
    ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::Escape) {
            out = Some(Outcome::Close);
        }
        if i.consume_key(Modifiers::COMMAND, Key::Enter) {
            out = Some(Outcome::Create);
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
            if p.query.is_empty() && i.key_pressed(Key::Backspace) {
                pop = true;
            }
        }
    });

    // Validation (affichée en bas, et le bouton « Créer » reste grisé tant qu'elle échoue).
    let problem = if g.space && g.name.trim().is_empty() {
        Some(t!("Donne un nom à l'espace", "Give the space a name"))
    } else if g.space && p.picked.is_empty() {
        Some(t!("Ajoute au moins une personne", "Add at least one person"))
    } else if !g.space && p.picked.len() < 2 {
        Some(t!("Une discussion de groupe demande au moins deux personnes", "A group chat needs at least two people"))
    } else {
        None
    };

    let w = 600.0_f32.min(screen.width() - 40.0);
    let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
    let mut remove: Option<usize> = None;
    let mut create_click = false;
    egui::Area::new(egui::Id::unique("wt_new_group_box")).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
        ui.multiply_opacity(k);
        overlay_frame().show(ui, |ui| {
            ui.set_width(w - 28.0);
            // En-tête : icône, titre, type de groupe.
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                people_icon(ui.painter(), r.center(), theme::TEXT_MUTED());
                ui.label(RichText::new(t!("Nouveau groupe", "New group")).size(17.0).strong().color(theme::TEXT()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let mut space = g.space;
                    if widgets::segmented(ui, "wt_new_group_kind", &[(false, t!("Discussion de groupe", "Group chat")), (true, t!("Espace", "Space"))], &mut space) {
                        g.space = space;
                    }
                });
            });
            ui.label(
                RichText::new(if g.space {
                    t!("Espace : un groupe nommé, que d'autres peuvent rejoindre.", "Space: a named group that others can join.")
                } else {
                    t!("Discussion de groupe : une conversation sans nom, entre les personnes choisies.", "Group chat: an unnamed conversation between the chosen people.")
                })
                .small()
                .color(theme::TEXT_FAINT()),
            );
            ui.add_space(8.0);
            // Nom de l'espace.
            if g.space {
                let te = ui.add(
                    egui::TextEdit::singleline(&mut g.name)
                        .id(name_id)
                        .hint_text(t!("Nom de l'espace", "Space name"))
                        .font(FontId::proportional(18.0))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                if p.focus {
                    te.request_focus();
                    p.focus = false;
                }
                // Entrée dans le nom : on passe aux membres.
                if te.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    p.focus_query = true;
                }
                ui.add_space(6.0);
                rule(ui);
            }
            // Personnes ajoutées (pilules) puis champ de recherche.
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for (i, (name, email)) in p.picked.iter().enumerate() {
                    if pill(ui, if name.is_empty() { email } else { name }, email) {
                        remove = Some(i);
                    }
                }
            });
            if !p.picked.is_empty() {
                ui.add_space(6.0);
            }
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
                if p.focus || std::mem::take(&mut p.focus_query) {
                    te.request_focus();
                    p.focus = false;
                }
                if te.changed() {
                    p.selected = 0;
                }
            });
            ui.add_space(6.0);
            rule(ui);
            ui.add_space(4.0);
            // Propositions.
            if rows.is_empty() {
                ui.add_space(10.0);
                ui.vertical_centered(|ui| {
                    let (head, sub) = if q.is_empty() {
                        (t!("Qui ajouter ?", "Who to add?"), t!("Tape un nom de l'annuaire ou une adresse complète.", "Type a name from the directory or a full address."))
                    } else {
                        (t!("Personne ne correspond", "Nobody matches"), t!("Tape une adresse complète pour l'ajouter quand même.", "Type a full address to add it anyway."))
                    };
                    ui.label(RichText::new(head).color(theme::TEXT_MUTED()));
                    ui.label(RichText::new(sub).small().color(theme::TEXT_FAINT()));
                });
                ui.add_space(10.0);
            } else {
                ui.label(RichText::new(if q.is_empty() { t!("SUGGESTIONS", "SUGGESTIONS") } else { t!("ANNUAIRE", "DIRECTORY") }).small().strong().color(theme::TEXT_FAINT()));
                egui::ScrollArea::vertical().max_height(screen.height() * 0.4).auto_shrink([false, true]).show(ui, |ui| {
                    for (i, row) in rows.iter().enumerate() {
                        let sel = i == p.selected;
                        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                            p.selected = i;
                        }
                        let pa = ui.painter();
                        if sel {
                            pa.rect_filled(r, CornerRadius::same(8), theme::overlay(14));
                            ui.scroll_to_rect(r, None);
                        }
                        let title = if row.typed {
                            tf!("Ajouter « {} »", "Add “{}”", row.email)
                        } else if row.name.is_empty() {
                            row.email.clone()
                        } else {
                            row.name.clone()
                        };
                        avatar(pa, Pos2::new(r.left() + 18.0, r.center().y), &title, row.typed);
                        let left = r.left() + 40.0;
                        let g1 = crate::tabs::mail::view::one_line(pa, &title, theme::editor_font(14.0, sel), theme::TEXT(), r.right() - left - 80.0);
                        let sub = if row.typed { t!("Adresse hors de l'annuaire", "Address not in the directory").to_string() } else { row.email.clone() };
                        let g2 = crate::tabs::mail::view::one_line(pa, &sub, FontId::proportional(11.5), theme::TEXT_MUTED(), r.right() - left - 80.0);
                        pa.galley(Pos2::new(left, r.top() + 3.0), g1, theme::TEXT());
                        pa.galley(Pos2::new(left, r.top() + 22.0), g2, theme::TEXT_MUTED());
                        if sel {
                            pa.text(Pos2::new(r.right() - 10.0, r.center().y), Align2::RIGHT_CENTER, t!("Entrée ↵", "Enter ↵"), FontId::proportional(11.0), theme::TEXT_FAINT());
                        }
                        if resp.clicked() {
                            add = Some(i);
                        }
                    }
                });
            }
            ui.add_space(8.0);
            // Pied : aide clavier, problème éventuel, créer.
            ui.horizontal(|ui| {
                let n = p.picked.len();
                let count = if crate::i18n::en() { format!("{n} {}", if n == 1 { "person" } else { "people" }) } else { format!("{n} personne{}", if n > 1 { "s" } else { "" }) };
                ui.label(RichText::new(count).small().strong().color(theme::TEXT_MUTED()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let create = egui::Button::new(RichText::new(t!("Créer", "Create")).strong()).corner_radius(CornerRadius::same(8)).min_size(vec2(84.0, 30.0));
                    let resp = ui.add_enabled(problem.is_none(), create);
                    let resp = match problem {
                        Some(why) => resp.on_disabled_hover_text(why),
                        None => resp.on_hover_text("Ctrl+Entrée"),
                    };
                    if resp.clicked() {
                        create_click = true;
                    }
                    if ui.button(t!("Annuler", "Cancel")).clicked() {
                        out = Some(Outcome::Close);
                    }
                    if let Some(why) = problem {
                        ui.label(RichText::new(why).small().color(theme::TEXT_FAINT()));
                    }
                });
            });
            ui.add_space(2.0);
            ui.label(
                RichText::new(t!(
                    "↑ ↓ choisir · Entrée ajouter · Retour arrière retirer · Ctrl+Entrée créer · Échap fermer",
                    "↑ ↓ choose · Enter add · Backspace remove · Ctrl+Enter create · Esc close"
                ))
                .small()
                .color(theme::TEXT_FAINT()),
            );
        });
    });
    // Comme la recherche globale : la boîte reste au-dessus de son fond.
    ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("wt_new_group_box")));

    if let Some(i) = add
        && let Some(r) = rows.get(i)
    {
        p.picked.push((if r.typed { String::new() } else { r.name.clone() }, r.email.clone()));
        p.query.clear();
        p.selected = 0;
        p.focus_query = true;
    }
    if let Some(i) = remove {
        p.picked.remove(i);
    } else if pop {
        p.picked.pop();
    }
    // Membres au format attendu par la création (« Nom <adresse>, … »).
    g.members = p.picked.iter().map(|(n, e)| if n.is_empty() { e.clone() } else { format!("{n} <{e}>") }).collect::<Vec<_>>().join(", ");
    if create_click || (matches!(out, Some(Outcome::Create)) && problem.is_none()) {
        return Some(Outcome::Create);
    }
    match out {
        Some(Outcome::Create) => None,
        o => o,
    }
}

/// Personne ajoutée : pilule avec une croix ; renvoie vrai si la croix est cliquée.
fn pill(ui: &mut egui::Ui, label: &str, tip: &str) -> bool {
    let font = FontId::proportional(13.0);
    let g = ui.painter().layout_no_wrap(label.to_string(), font, theme::TEXT());
    let size = vec2(g.size().x + 48.0, 26.0);
    let (r, resp) = ui.allocate_exact_size(size, Sense::hover());
    let resp = resp.on_hover_text(tip);
    let p = ui.painter();
    p.rect(r, CornerRadius::same(255), theme::overlay(16), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
    avatar(p, Pos2::new(r.left() + 13.0, r.center().y), label, false);
    p.galley(Pos2::new(r.left() + 24.0, r.center().y - g.size().y / 2.0), g, theme::TEXT());
    let x = Rect::from_center_size(Pos2::new(r.right() - 12.0, r.center().y), vec2(16.0, 16.0));
    let xr = ui.interact(x, resp.id.with("x"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let ink = if xr.hovered() { theme::TEXT() } else { theme::TEXT_FAINT() };
    let (c, k) = (x.center(), 3.0);
    let s = Stroke::new(1.4, ink);
    ui.painter().line_segment([c + vec2(-k, -k), c + vec2(k, k)], s);
    ui.painter().line_segment([c + vec2(k, -k), c + vec2(-k, k)], s);
    xr.clicked()
}

/// Rond avec l'initiale (ou « + » pour une adresse tapée).
pub(super) fn avatar(p: &egui::Painter, c: Pos2, name: &str, typed: bool) {
    let r = if typed { 12.0 } else { 9.5 };
    p.circle_filled(c, r, theme::overlay(30));
    if typed {
        let s = Stroke::new(1.6, theme::TEXT());
        p.hline(c.x - 5.0..=c.x + 5.0, c.y, s);
        p.vline(c.x, c.y - 5.0..=c.y + 5.0, s);
    } else {
        let initial = name.chars().find(|ch| ch.is_alphanumeric()).map(|ch| ch.to_uppercase().to_string()).unwrap_or_default();
        p.text(c, Align2::CENTER_CENTER, initial, FontId::proportional(r * 1.05), theme::TEXT());
    }
}

pub(super) fn rule(ui: &mut egui::Ui) {
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));
}

pub(super) fn magnifier(p: &egui::Painter, c: Pos2, color: Color32) {
    p.circle_stroke(c + vec2(-2.0, -2.0), 6.0, Stroke::new(1.8, color));
    p.line_segment([c + vec2(2.5, 2.5), c + vec2(7.0, 7.0)], Stroke::new(2.0, color));
}

/// Deux silhouettes.
pub(super) fn people_icon(p: &egui::Painter, c: Pos2, color: Color32) {
    let s = Stroke::new(1.6, color);
    for (dx, k) in [(-3.5, 1.0), (4.0, 0.85)] {
        let h = c + vec2(dx, -3.5 * k);
        p.circle_stroke(h, 3.2 * k, s);
        let b = c + vec2(dx, 6.5);
        p.add(egui::Shape::line(
            (0..=10)
                .map(|i| {
                    let a = std::f32::consts::PI * (1.0 + i as f32 / 10.0);
                    b + vec2(a.cos() * 5.5 * k, a.sin() * 5.0 * k)
                })
                .collect(),
            s,
        ));
    }
}
