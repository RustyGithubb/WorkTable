//! Bibliothèque de nodes (clic droit dans le graph, ou fil lâché dans le vide), à la manière du
//! menu « All Actions » d'Unreal Engine : barre de recherche, catégories repliables.
//!
//! « Sensible au contexte » : quand on a tiré un fil, seules les nodes qui peuvent s'y brancher
//! sont proposées, et la node choisie est reliée au fil.
//! Clavier : taper pour chercher, ↑ ↓ pour choisir, Entrée pour poser la node.

use eframe::egui::{self, Color32, CornerRadius, Margin, Pos2, RichText, Stroke, vec2};

use super::editor::PinRef;
use super::nodes::{self, Cat, NodeDef};

const W: f32 = 340.0;
const H: f32 = 420.0;
const FILL: Color32 = Color32::from_rgb(0x1A, 0x1A, 0x1A);
const LINE: Color32 = Color32::from_rgb(0x3A, 0x3A, 0x3A);

pub struct Library {
    /// Position du clic (écran) et point du graph où poser la node choisie.
    pub screen: Pos2,
    pub graph: Pos2,
    /// Fil tiré depuis cette pin (la node choisie y sera reliée).
    pub from: Option<PinRef>,
    query: String,
    /// « Sensible au contexte » : ne montrer que les nodes branchables sur le fil.
    context: bool,
    open: [bool; Cat::ALL.len()],
    /// Ligne choisie au clavier (parmi les nodes affichées).
    cursor: usize,
}

impl Library {
    pub fn new(screen: Pos2, graph: Pos2, from: Option<PinRef>) -> Self {
        Self { screen, graph, from, query: String::new(), context: true, open: [true; Cat::ALL.len()], cursor: 0 }
    }

    /// La node peut-elle se brancher sur le fil tiré ?
    fn fits(&self, d: &'static NodeDef) -> bool {
        match &self.from {
            Some(f) if self.context => compatible_pin(d, f).is_some(),
            _ => true,
        }
    }
}

/// Première pin de `d` qui peut recevoir (ou fournir) le fil tiré depuis `from`.
pub fn compatible_pin(d: &'static NodeDef, from: &PinRef) -> Option<&'static str> {
    if from.output { d.wired_inputs().find(|p| p.ty.accepts(from.ty)).map(|p| p.key) } else { d.outputs.iter().find(|p| from.ty.accepts(p.ty)).map(|p| p.key) }
}

fn matches(d: &NodeDef, q: &str) -> bool {
    q.is_empty() || [d.fr, d.en, d.desc_fr, d.desc_en].iter().any(|s| s.to_lowercase().contains(q))
}

/// Résultat d'une image : garder la bibliothèque ouverte ? node choisie ?
pub struct Out {
    pub keep: bool,
    pub pick: Option<&'static str>,
}

pub fn show(ctx: &egui::Context, lib: &mut Library) -> Out {
    // Reste dans la fenêtre, même ouverte près d'un bord.
    let screen = ctx.content_rect();
    let pos = Pos2::new(lib.screen.x.min(screen.right() - W - 8.0), lib.screen.y.min(screen.bottom() - H - 8.0)).max(screen.min + vec2(8.0, 8.0));
    let q = lib.query.trim().to_lowercase();
    // Nodes affichées, dans l'ordre des catégories.
    let visible: Vec<&'static NodeDef> = Cat::ALL.iter().flat_map(|c| nodes::ALL.iter().filter(move |d| d.cat == *c)).filter(|d| lib.fits(d) && matches(d, &q)).collect();
    let (up, down, enter) = ctx.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.key_pressed(egui::Key::Enter)));
    if down && !visible.is_empty() {
        lib.cursor = (lib.cursor + 1) % visible.len();
    }
    if up && !visible.is_empty() {
        lib.cursor = (lib.cursor + visible.len() - 1) % visible.len();
    }
    lib.cursor = lib.cursor.min(visible.len().saturating_sub(1));
    let mut pick = if enter { visible.get(lib.cursor).map(|d| d.key) } else { None };

    let area = egui::Area::new(egui::Id::unique("wt_bp_library")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new()
            .fill(FILL)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(8))
            .inner_margin(Margin::same(10))
            .shadow(egui::Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(160) })
            .show(ui, |ui| {
                ui.set_width(W - 20.0);
                ui.set_max_height(H - 20.0);
                ui.horizontal(|ui| {
                    let title = if lib.from.is_some() && lib.context { t!("Nodes branchables ici", "Nodes that fit here") } else { t!("Toutes les nodes", "All nodes") };
                    ui.label(RichText::new(title).strong().color(Color32::from_gray(0xE0)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.add_enabled_ui(lib.from.is_some(), |ui| {
                            ui.checkbox(&mut lib.context, RichText::new(t!("Sensible au contexte", "Context sensitive")).small().color(Color32::from_gray(0xB0)))
                                .on_disabled_hover_text(t!("Tire un fil depuis une pin pour filtrer les nodes", "Drag a wire from a pin to filter nodes"));
                        });
                    });
                });
                ui.add_space(6.0);
                let search = ui.add(
                    egui::TextEdit::singleline(&mut lib.query)
                        .hint_text(t!("🔍  Rechercher une node", "🔍  Search for a node"))
                        .desired_width(f32::INFINITY)
                        .margin(vec2(8.0, 5.0)),
                );
                if search.changed() {
                    lib.cursor = 0;
                }
                // Le clavier reste à la recherche tant que la bibliothèque est ouverte (comme la
                // recherche globale) : on tape directement le nom d'une node.
                search.request_focus();
                ui.add_space(6.0);
                egui::ScrollArea::vertical().id_salt("wt_bp_library_list").auto_shrink([false, true]).show(ui, |ui| {
                    if visible.is_empty() {
                        ui.add_space(12.0);
                        ui.vertical_centered(|ui| {
                            let msg = if q.is_empty() {
                                t!("Aucune node ne peut se brancher sur ce fil.", "No node fits this wire.").to_string()
                            } else {
                                tf!("Aucune node ne correspond à « {} »", "No node matches “{}”", lib.query.trim())
                            };
                            ui.label(RichText::new(msg).color(Color32::from_gray(0x90)));
                        });
                        return;
                    }
                    let mut row = 0;
                    for (ci, cat) in Cat::ALL.iter().enumerate() {
                        let defs: Vec<&&NodeDef> = visible.iter().filter(|d| d.cat == *cat).collect();
                        if defs.is_empty() {
                            continue;
                        }
                        // Une recherche déplie tout.
                        let open = lib.open[ci] || !q.is_empty();
                        if category(ui, *cat, defs.len(), open) {
                            lib.open[ci] = !lib.open[ci];
                        }
                        for d in defs {
                            if open {
                                let r = item(ui, d, row == lib.cursor);
                                if r.clicked() {
                                    pick = Some(d.key);
                                }
                                if r.hovered() {
                                    lib.cursor = row;
                                }
                            }
                            row += 1;
                        }
                        ui.add_space(2.0);
                    }
                });
            });
    });
    let rect = area.response.rect;
    let clicked_outside = ctx.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p)));
    let escape = ctx.input(|i| i.key_pressed(egui::Key::Escape));
    Out { keep: pick.is_none() && !(clicked_outside || escape), pick }
}

/// En-tête de catégorie repliable ; renvoie `true` s'il a été cliqué.
fn category(ui: &mut egui::Ui, cat: Cat, n: usize, open: bool) -> bool {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if r.hovered() {
        p.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(0x26));
    }
    let y = rect.center().y;
    // Triangle dessiné (les caractères ▾ ▸ manquent dans certaines polices).
    let c = egui::pos2(rect.left() + 10.0, y);
    let tri = if open { vec![c + vec2(-4.0, -2.0), c + vec2(4.0, -2.0), c + vec2(0.0, 3.0)] } else { vec![c + vec2(-2.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-2.0, 4.0)] };
    p.add(egui::Shape::convex_polygon(tri, Color32::from_gray(0x90), Stroke::NONE));
    // Pastille de couleur de la catégorie (celle de l'en-tête de ses nodes).
    p.rect_filled(egui::Rect::from_center_size(egui::pos2(rect.left() + 26.0, y), vec2(9.0, 9.0)), CornerRadius::same(2), cat.color());
    p.text(egui::pos2(rect.left() + 38.0, y), egui::Align2::LEFT_CENTER, cat.label(), egui::FontId::proportional(13.5), Color32::from_gray(0xD8));
    p.text(egui::pos2(rect.right() - 8.0, y), egui::Align2::RIGHT_CENTER, n.to_string(), egui::FontId::proportional(11.0), Color32::from_gray(0x70));
    r.clicked()
}

fn item(ui: &mut egui::Ui, d: &NodeDef, active: bool) -> egui::Response {
    let (rect, r) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), egui::Sense::click());
    let r = r.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(d.desc());
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(4), Color32::from_rgb(0x24, 0x3A, 0x5C));
    }
    // Logo du type de node, dans la couleur de sa catégorie (éclaircie pour rester lisible).
    let col = crate::sidebar::lerp_color(d.cat.color(), Color32::WHITE, 0.35);
    super::editor::node_icon(p, egui::pos2(rect.left() + 46.0, rect.center().y), 13.0, d.key, col);
    p.text(egui::pos2(rect.left() + 58.0, rect.center().y), egui::Align2::LEFT_CENTER, d.title(), egui::FontId::proportional(13.0), Color32::from_gray(0xE8));
    r
}
