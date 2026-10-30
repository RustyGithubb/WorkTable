//! Onglet personnalisé : une galerie de panneaux, sur la même grille que le tableau de bord
//! (déplacer par la barre de titre, redimensionner par le coin, rangées qui remplissent toute la
//! largeur). Les panneaux se choisissent dans la galerie « + Ajouter un panneau » : ceux du
//! tableau de bord (mêmes données) et des blocs-notes propres à l'onglet ; la pastille « × »
//! (au survol) retire un panneau.

use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, vec2};

use super::dash_layout;
use super::dashboard::{self, DashAction, Deps};
use crate::prefs::Prefs;
use crate::{theme, widgets};

/// Panneau de la galerie : type, émoji, nom, description.
struct Kind {
    id: &'static str,
    emoji: &'static str,
    name: (&'static str, &'static str),
    desc: (&'static str, &'static str),
    /// Plusieurs fois dans un même onglet (sinon une seule).
    many: bool,
}

const KINDS: [Kind; 9] = [
    Kind { id: "note", emoji: "📝", name: ("Bloc-notes", "Notepad"), desc: ("Texte libre, propre à cet onglet", "Free text, specific to this tab"), many: true },
    Kind { id: "tasks", emoji: "✅", name: ("Tâches", "Tasks"), desc: ("Kanban à faire / en cours / archivées", "To do / in progress / archived board"), many: false },
    Kind { id: "reminders", emoji: "⏰", name: ("Rappels", "Reminders"), desc: ("Minuteurs et rappels à venir", "Upcoming timers and reminders"), many: false },
    Kind { id: "planning", emoji: "📅", name: ("Planning du jour", "Today's schedule"), desc: ("Événements de l'agenda aujourd'hui", "Today's calendar events"), many: false },
    Kind { id: "timeline", emoji: "🗓", name: ("Quadrillage", "Day grid"), desc: ("Un carré par jour de la période", "One square per day of the period"), many: false },
    Kind { id: "mails", emoji: "✉", name: ("Mails", "Mails"), desc: ("Non lus et épinglés", "Unread and pinned"), many: false },
    Kind { id: "pinned", emoji: "📌", name: ("Messages épinglés", "Pinned messages"), desc: ("Épinglés dans Google Chat", "Pinned in Google Chat"), many: false },
    Kind { id: "files", emoji: "📎", name: ("Fichiers reçus", "Received files"), desc: ("Derniers fichiers reçus par mail", "Latest files received by mail"), many: false },
    Kind { id: "links", emoji: "🔗", name: ("Liens importants", "Important links"), desc: ("Raccourcis vers tes sites", "Shortcuts to your sites"), many: false },
];

fn lang<'a>(fr_en: (&'a str, &'a str)) -> &'a str {
    if crate::i18n::en() { fr_en.1 } else { fr_en.0 }
}

pub fn show(ui: &mut egui::Ui, prefs: &mut Prefs, id: u64, d: &mut Deps) -> Option<DashAction> {
    let Some(i) = prefs.custom_tabs.iter().position(|t| t.id == id) else {
        widgets::page_header(ui, t!("Onglet personnalisé", "Custom tab"), t!("Onglet supprimé", "Deleted tab"));
        return None;
    };
    let mut out = None;
    let b = dashboard::prepare(d, prefs, ui);
    grid_bases(prefs, i);
    let mut rows = std::mem::take(&mut prefs.custom_tabs[i].rows);
    let mut notes = std::mem::take(&mut prefs.custom_tabs[i].notes);
    let tab = prefs.custom_tabs[i].clone();
    let present: Vec<String> = rows.iter().flat_map(|r| r.panels.iter().map(|p| p.id.clone())).collect();
    let mut add: Option<&'static str> = None;

    widgets::page_header_with(ui, t!("Onglet personnalisé", "Custom tab"), &tab.name, |ui| {
        let (r, _) = ui.allocate_exact_size(vec2(30.0, 30.0), Sense::hover());
        crate::gallery::paint_icon(ui.painter(), r, &tab);
        if !rows.is_empty() {
            ui.add_space(8.0);
            let button =
                ui.add(egui::Button::new(RichText::new(t!("+ Ajouter un panneau", "+ Add a panel")).strong()).corner_radius(CornerRadius::same(255)).min_size(vec2(0.0, 30.0)));
            egui::Popup::menu(&button).width(560.0).gap(8.0).show(|ui| {
                ui.add_space(4.0);
                ui.label(widgets::section_title(t!("Galerie de panneaux", "Panel gallery")));
                ui.add_space(8.0);
                if let Some(k) = catalog(ui, &present, 3) {
                    add = Some(k);
                    ui.close();
                }
            });
        }
    });

    let view: &Prefs = prefs;
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if rows.is_empty() {
            empty(ui, &present, &mut add);
            return;
        }
        let (_, removed) = dash_layout::show_free(ui, id, &mut rows, |pid, ui| match dash_layout::kind(pid) {
            "note" => note(ui, pid, notes.entry(pid.to_string()).or_default()),
            "timeline" => {
                // Quadrillage : mêmes dates que celui du tableau de bord, jours de sa propre base.
                if let Some(set) = tab.grid_sets.get(pid).and_then(|id| view.highlight_sets.iter().find(|s| s.id == *id)) {
                    dashboard::grid_panel(ui, &set.name, d, view, &b, &set.highlights, &mut out);
                }
            }
            k => dashboard::draw(k, ui, d, view, &b, &mut out),
        });
        if let Some(pid) = removed {
            notes.remove(&pid);
        }
        ui.add_space(24.0);
    });

    if let Some(k) = add {
        let width = ui.available_width() - 12.0;
        let n = present.iter().filter_map(|p| p.split('#').nth(1)?.parse::<u32>().ok()).max().unwrap_or(0) + 1;
        dash_layout::append(&mut rows, &format!("{k}#{n}"), width);
    }
    let t = &mut prefs.custom_tabs[i];
    t.rows = rows;
    t.notes = notes;
    d.tasks.save_if_needed();
    out
}

/// Chaque quadrillage de l'onglet `i` a sa base de jours mis en évidence, distincte de celle du
/// tableau de bord : créée (au nom de l'onglet) s'il n'en a pas, ou si elle n'existe pas ici
/// (onglet importé).
fn grid_bases(prefs: &mut Prefs, i: usize) {
    let grids: Vec<String> = prefs.custom_tabs[i].rows.iter().flat_map(|r| r.panels.iter()).filter(|p| dash_layout::kind(&p.id) == "timeline").map(|p| p.id.clone()).collect();
    for pid in grids {
        let known = prefs.custom_tabs[i].grid_sets.get(&pid).is_some_and(|id| prefs.highlight_sets.iter().any(|s| s.id == *id));
        if !known {
            let name = prefs.custom_tabs[i].name.clone();
            let id = super::timeline::new_base(prefs, &name);
            prefs.custom_tabs[i].grid_sets.insert(pid, id);
        }
    }
}

/// Onglet sans panneau : la galerie, au centre.
fn empty(ui: &mut egui::Ui, present: &[String], add: &mut Option<&'static str>) {
    let w = ui.available_width().min(760.0);
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        ui.vertical(|ui| {
            ui.set_width(w);
            let top = ui.cursor().min;
            ui.add_space(28.0);
            ui.vertical_centered(|ui| {
                ui.label(widgets::section_title(t!("Compose ton onglet", "Build your tab")));
                ui.add_space(4.0);
                ui.label(
                    RichText::new(t!(
                        "Choisis des panneaux : tu pourras ensuite les déplacer par leur titre et les redimensionner par leur coin, comme sur le tableau de bord.",
                        "Pick panels: then move them by their title and resize them by their corner, like on the dashboard."
                    ))
                    .small()
                    .color(theme::TEXT_FAINT()),
                );
            });
            ui.add_space(18.0);
            ui.horizontal(|ui| {
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.set_width(w - 48.0);
                    *add = catalog(ui, present, 3).or(*add);
                });
            });
            ui.add_space(24.0);
            let frame = Rect::from_min_max(top, Pos2::new(top.x + w, ui.cursor().min.y));
            dash_layout::dashed(ui.painter(), frame, theme::TEXT_FAINT());
        });
    });
}

/// Cartes des types de panneaux (`cols` par ligne) ; renvoie le type cliqué.
fn catalog(ui: &mut egui::Ui, present: &[String], cols: usize) -> Option<&'static str> {
    let mut picked = None;
    let gap = 10.0;
    let w = ((ui.available_width() - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let h = if w < 200.0 { 92.0 } else { 76.0 };
    for chunk in KINDS.chunks(cols) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for k in chunk {
                let taken = !k.many && present.iter().any(|p| dash_layout::kind(p) == k.id);
                let (r, resp) = ui.allocate_exact_size(vec2(w, h), if taken { Sense::hover() } else { Sense::click() });
                let hovered = resp.hovered() && !taken;
                let p = ui.painter();
                p.rect(
                    r,
                    CornerRadius::same(12),
                    if hovered { theme::overlay(10) } else { theme::SURFACE() },
                    Stroke::new(1.0, if hovered { crate::tabs::timeline::ACCENT } else { theme::BORDER() }),
                    egui::StrokeKind::Inside,
                );
                let alpha = if taken { 0.4 } else { 1.0 };
                let icon = Rect::from_min_size(r.min + vec2(14.0, 16.0), vec2(26.0, 26.0));
                crate::emoji_img::paint(p, icon, k.emoji, theme::TEXT());
                let tx = icon.right() + 12.0;
                let max_w = r.right() - tx - 10.0;
                let name = p.layout(lang(k.name).to_string(), theme::title_font(14.0), theme::TEXT().gamma_multiply(alpha), max_w);
                let name_h = name.size().y;
                p.galley(Pos2::new(tx, r.top() + 14.0), name, Color32::WHITE);
                let sub = if taken { t!("Déjà dans l'onglet", "Already in this tab") } else { lang(k.desc) };
                let desc = p.layout(sub.to_string(), egui::FontId::proportional(11.5), theme::TEXT_FAINT().gamma_multiply(alpha), max_w);
                p.galley(Pos2::new(tx, r.top() + 18.0 + name_h), desc, Color32::WHITE);
                if resp.on_hover_cursor(if taken { egui::CursorIcon::Default } else { egui::CursorIcon::PointingHand }).clicked() && !taken {
                    picked = Some(k.id);
                }
            }
        });
        ui.add_space(gap);
    }
    picked
}

/// Bloc-notes : texte libre (vérifié par le correcteur), enregistré avec l'onglet.
fn note(ui: &mut egui::Ui, pid: &str, text: &mut String) {
    dashboard::panel(ui, t!("Bloc-notes", "Notepad"), "", |ui| {
        egui::ScrollArea::vertical().id_salt(("wt_custom_note", pid)).auto_shrink(false).show(ui, |ui| {
            let out = egui::TextEdit::multiline(text)
                .id_salt(("wt_custom_note_edit", pid))
                .frame(egui::Frame::NONE)
                .hint_text(t!("Écris ici…", "Write here…"))
                .desired_width(f32::INFINITY)
                .desired_rows(6)
                .show(ui);
            crate::spell::check(ui, &out, text);
        });
    });
}
