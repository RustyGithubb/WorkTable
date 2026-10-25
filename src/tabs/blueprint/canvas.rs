//! Espace de travail du Blueprint, à la manière d'Unreal Engine 5 : grille infinie,
//! caméra et zoom.
//!
//! - Clic droit maintenu (ou molette maintenue) : déplacer la vue.
//! - Clic droit simple : bibliothèque de nodes à l'endroit du clic.
//! - Molette : zoom autour du curseur. `Origine` : recentrer la vue.
//! - Glisser au clic gauche dans le vide : rectangle de sélection.
//! - Nodes et fils : voir `editor`.

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, PointerButton, Pos2, Rect, Sense, Stroke, vec2};

use super::editor::{self, EditorState, SimView};
use super::library::Library;
use super::store::{Graph, View};

/// Couleurs fixes du graph (inspirées d'Unreal Engine 5, indépendantes du thème de l'app).
pub const BG: Color32 = Color32::from_rgb(0x26, 0x26, 0x26);
const GRID_MINOR: Color32 = Color32::from_rgb(0x2E, 0x2E, 0x2E);
const GRID_MAJOR: Color32 = Color32::from_rgb(0x17, 0x17, 0x17);
const WATERMARK: Color32 = Color32::from_rgb(0x34, 0x34, 0x34);
/// Cadre et bandeau pendant une simulation (comme le « Play In Editor » d'Unreal).
pub const SIM: Color32 = Color32::from_rgb(0xF2, 0xA6, 0x2C);

/// Pas de la grille fine et de la grille principale (unités du graph).
const STEP: f32 = 16.0;
const MAJOR: f32 = 128.0;
pub const ZOOM_MIN: f32 = 0.2;
pub const ZOOM_MAX: f32 = 2.0;

/// État d'interface du graph ouvert (non enregistré).
#[derive(Default)]
pub struct Canvas {
    pub library: Option<Library>,
    /// Rectangle de sélection en cours (début, en coordonnées du graph).
    marquee: Option<Pos2>,
    /// Vue en cours de déplacement (clic droit ou molette maintenus).
    panning: bool,
    pub ed: EditorState,
}

/// Ce que l'affichage du graph a modifié.
#[derive(Default)]
pub struct Changes {
    /// Caméra (déplacement, zoom) : enregistrée sans changer la date de modification.
    pub view: bool,
    /// Nodes ou fils.
    pub graph: bool,
}

/// Graph → écran.
pub fn to_screen(center: Pos2, v: &View, g: Pos2) -> Pos2 {
    center + (g - Pos2::from(v.pan)) * v.zoom
}

/// Écran → graph.
pub fn to_graph(center: Pos2, v: &View, s: Pos2) -> Pos2 {
    Pos2::from(v.pan) + (s - center) / v.zoom
}

/// Zoom de `factor` en gardant fixe le point du graph sous `anchor` (écran).
fn zoom_at(v: &mut View, center: Pos2, anchor: Pos2, factor: f32) {
    let before = to_graph(center, v, anchor);
    v.zoom = (v.zoom * factor).clamp(ZOOM_MIN, ZOOM_MAX);
    let after = to_graph(center, v, anchor);
    v.pan = (Pos2::from(v.pan) + (before - after)).into();
}

/// Dessine le graph dans `rect`, ses nodes et ses fils, et gère la caméra.
/// `home` : place normale du graph (le nom, l'aide et le bandeau de simulation s'y placent) ;
/// `center` : point de l'écran qui porte `view.pan`, fixe pendant l'agrandissement en plein
/// écran et à l'ouverture du journal : le contenu du graph ne bouge pas.
/// `sim` : simulation en cours ; `full` : avancement du plein écran (0 → 1 : coins de plus en
/// plus droits, sans bordure) ; `grid` : opacité de la grille (0 → 1).
#[allow(clippy::too_many_arguments)]
pub fn show(ui: &mut egui::Ui, rect: Rect, home: Rect, center: Pos2, g: &mut Graph, cv: &mut Canvas, sim: Option<&SimView>, full: f32, grid: f32) -> Changes {
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique(("wt_bp_canvas", &g.id));
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let before = g.view;
    // Le pointeur est-il sur le graph lui-même (fond ou nodes), pas sur un panneau par-dessus ?
    let ours = |p: Pos2| rect.contains(p) && ctx.layer_id_at(p).is_none_or(|l| l == ui.layer_id());

    // Déplacer la vue : clic droit ou molette maintenus, commencés sur le graph (même sur une node).
    let (held, origin, delta, decided) =
        ctx.input(|i| (i.pointer.secondary_down() || i.pointer.middle_down(), i.pointer.press_origin(), i.pointer.delta(), i.pointer.is_decidedly_dragging()));
    if held && origin.is_some_and(ours) && (decided || cv.panning) {
        cv.panning = true;
        g.view.pan = (Pos2::from(g.view.pan) - delta / g.view.zoom).into();
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
        cv.library = None;
    }
    if !held {
        cv.panning = false;
    }
    // Clic droit simple dans le vide : bibliothèque de nodes.
    if resp.secondary_clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        cv.library = Some(Library::new(p, to_graph(center, &g.view, p), None));
    }
    if resp.clicked() {
        cv.library = None;
        cv.ed.selected.clear();
    }
    // Zoom à la molette (ou au pavé tactile), autour du curseur.
    if let Some(p) = ctx.pointer_hover_pos()
        && ours(p)
    {
        let (scroll, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
        let factor = (scroll * 0.0015).exp() * pinch;
        if (factor - 1.0).abs() > 1e-4 {
            zoom_at(&mut g.view, center, p, factor);
        }
    }
    // Rectangle de sélection (clic gauche glissé dans le vide) : sélectionne les nodes touchées.
    if resp.drag_started_by(PointerButton::Primary)
        && let Some(p) = resp.interact_pointer_pos()
    {
        cv.marquee = Some(to_graph(center, &g.view, p));
        cv.library = None;
    }
    if let (Some(start), Some(now)) = (cv.marquee, ctx.pointer_interact_pos()) {
        let area = Rect::from_two_pos(start, to_graph(center, &g.view, now));
        cv.ed.selected = editor::node_rects(g).into_iter().filter(|(_, r)| r.intersects(area)).map(|(id, _)| id).collect();
    }
    if !resp.dragged_by(PointerButton::Primary) {
        cv.marquee = None;
    }
    // Origine : recentrer (si aucun champ de texte n'a le clavier).
    if ctx.pointer_hover_pos().is_some_and(ours) && !ctx.egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Home)) {
        g.view = View::default();
    }

    paint(ui, rect, home, center, g, full, grid);
    let (graph, dropped) = editor::show(ui, rect, center, g, &mut cv.ed, sim);
    if let Some(d) = dropped {
        cv.library = Some(Library::new(d.screen, d.graph, Some(d.from)));
    }
    overlay(ui, rect, home, center, g, cv, sim.is_some(), full);
    Changes { view: g.view != before, graph }
}

#[allow(clippy::too_many_arguments)]
fn paint(ui: &egui::Ui, rect: Rect, home: Rect, center: Pos2, g: &Graph, full: f32, grid: f32) {
    let p = ui.painter_at(rect);
    let v = &g.view;
    let round = CornerRadius::same((10.0 * (1.0 - full)).round() as u8);
    p.rect_filled(rect, round, BG);

    // Grille : fine (masquée quand elle devient trop serrée), puis principale par-dessus.
    let lo = to_graph(center, v, rect.min);
    let hi = to_graph(center, v, rect.max);
    let inner = rect.shrink(1.0);
    let lines = |step: f32, color: Color32| {
        if step * v.zoom < 7.0 {
            return;
        }
        let mut x = (lo.x / step).floor() * step;
        while x <= hi.x {
            let sx = to_screen(center, v, Pos2::new(x, 0.0)).x;
            p.vline(sx, inner.y_range(), Stroke::new(1.0, color));
            x += step;
        }
        let mut y = (lo.y / step).floor() * step;
        while y <= hi.y {
            let sy = to_screen(center, v, Pos2::new(0.0, y)).y;
            p.hline(inner.x_range(), sy, Stroke::new(1.0, color));
            y += step;
        }
    };
    // Opacité réglable : les lignes se fondent dans le fond.
    let fade = |c: Color32| crate::sidebar::lerp_color(BG, c, grid.clamp(0.0, 1.0));
    lines(STEP, fade(GRID_MINOR));
    lines(MAJOR, fade(GRID_MAJOR));

    // Filigrane en bas à droite : le nom du graph, en capitales (comme « BLUEPRINT » dans
    // Unreal), réduit pour tenir dans un peu plus de la moitié de la largeur.
    let name = g.name.trim().to_uppercase();
    let name = if name.is_empty() { "BLUEPRINT".to_string() } else { name };
    let mut mark = p.layout_no_wrap(name.clone(), FontId::proportional(44.0), WATERMARK);
    let max_w = home.width() * 0.55;
    if mark.size().x > max_w {
        let size = (44.0 * max_w / mark.size().x).max(18.0);
        mark = p.layout_no_wrap(name, FontId::proportional(size), WATERMARK);
    }
    p.galley(home.right_bottom() - mark.size() - vec2(22.0, 14.0), mark, WATERMARK);

    // Graph vide : aide au centre.
    if g.nodes.is_empty() {
        let c = center;
        p.text(
            c - vec2(0.0, 10.0),
            Align2::CENTER_CENTER,
            t!("Clic droit pour ajouter une node", "Right-click to add a node"),
            FontId::proportional(16.0),
            Color32::from_gray(0x9A),
        );
        p.text(
            c + vec2(0.0, 14.0),
            Align2::CENTER_CENTER,
            t!("Clic droit maintenu : déplacer la vue  ·  Molette : zoom  ·  Origine : recentrer", "Hold right-click: move the view  ·  Wheel: zoom  ·  Home: recentre"),
            FontId::proportional(12.0),
            Color32::from_gray(0x6A),
        );
    }

    if full < 1.0 {
        p.rect_stroke(rect, round, Stroke::new(1.0, crate::theme::BORDER().gamma_multiply(1.0 - full)), egui::StrokeKind::Inside);
    }
}

/// Par-dessus les nodes : rectangle de sélection, cadre et bandeau de simulation.
#[allow(clippy::too_many_arguments)]
fn overlay(ui: &egui::Ui, rect: Rect, home: Rect, center: Pos2, g: &Graph, cv: &Canvas, simulating: bool, full: f32) {
    let v = &g.view;
    let round = CornerRadius::same((10.0 * (1.0 - full)).round() as u8);
    let p = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Middle, egui::Id::unique(("wt_bp_overlay", &g.id)))).with_clip_rect(rect);
    if let (Some(start), Some(now)) = (cv.marquee, ui.ctx().pointer_interact_pos()) {
        let r = Rect::from_two_pos(to_screen(center, v, start), now);
        p.rect(r, CornerRadius::ZERO, Color32::from_rgba_unmultiplied(0x6A, 0xA8, 0xFF, 0x18), Stroke::new(1.0, Color32::from_rgb(0x6A, 0xA8, 0xFF)), egui::StrokeKind::Inside);
    }
    if simulating {
        p.rect_stroke(rect, round, Stroke::new(2.5, SIM), egui::StrokeKind::Inside);
        let label = p.layout_no_wrap(t!("SIMULATION", "SIMULATING").into(), FontId::proportional(12.0), Color32::BLACK);
        let tag = Rect::from_min_size(home.center_top() + vec2(-label.size().x / 2.0 - 10.0, 0.0), label.size() + vec2(20.0, 8.0));
        let tag_round = CornerRadius { nw: (6.0 * full) as u8, ne: (6.0 * full) as u8, sw: 6, se: 6 };
        p.rect_filled(tag, tag_round, SIM);
        p.galley(tag.min + vec2(10.0, 4.0), label, Color32::BLACK);
    }
}

/// Vue recentrée sur l'origine, zoom 100 %.
pub fn recenter(g: &mut Graph) {
    g.view = View::default();
}

/// Zoom par bouton (+ / −), autour du centre de la vue.
pub fn zoom_step(g: &mut Graph, factor: f32) {
    let c = Pos2::ZERO;
    zoom_at(&mut g.view, c, c, factor);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_graph_roundtrip() {
        let v = View { pan: [120.0, -40.0], zoom: 0.5 };
        let c = Pos2::new(400.0, 300.0);
        let g = Pos2::new(37.0, 81.0);
        let back = to_graph(c, &v, to_screen(c, &v, g));
        assert!((back - g).length() < 1e-3);
    }

    #[test]
    fn zoom_keeps_point_under_cursor() {
        let mut v = View::default();
        let c = Pos2::new(400.0, 300.0);
        let cursor = Pos2::new(650.0, 120.0);
        let under = to_graph(c, &v, cursor);
        zoom_at(&mut v, c, cursor, 1.7);
        assert!((to_graph(c, &v, cursor) - under).length() < 1e-3);
        zoom_at(&mut v, c, cursor, 100.0);
        assert_eq!(v.zoom, ZOOM_MAX);
    }
}
