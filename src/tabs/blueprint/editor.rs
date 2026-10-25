//! Nodes et fils du graph. Tout est dessiné à la taille de l'écran (positions, épaisseurs et
//! polices multipliées par le zoom) : le texte reste net à tous les niveaux de zoom.
//!
//! - Glisser une node (son en-tête ou son corps) : la déplacer, avec toute la sélection ; elle
//!   s'aimante à la grille.
//! - Clic : sélectionner ; Maj ou Ctrl + clic : ajouter ou retirer de la sélection.
//! - Glisser depuis une pin : tirer un fil ; le lâcher sur une pin compatible pour relier, ou
//!   dans le vide pour ouvrir la bibliothèque des nodes qui peuvent s'y brancher.
//! - Alt + clic sur une pin : rompre ses fils. Clic droit sur une node : menu.
//! - Suppr : supprimer la sélection ; Ctrl + D : la dupliquer ; Q : aligner la sélection sur la
//!   sortie d'exécution de la node la plus à gauche.

use std::collections::{HashMap, HashSet};

use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2, vec2};

use super::nodes::{self, NodeDef, PinDef, PinType, Widget};
use super::sim::Trace;
use super::store::{Graph, Node, View};

pub const HEADER_H: f32 = 28.0;
pub const PARAM_H: f32 = 26.0;
pub const ROW_H: f32 = 24.0;
/// Distance des pins au bord de la node (elles sont à l'intérieur, comme dans Unreal).
const PIN_INSET: f32 = 13.0;
/// Pas de la grille sur lequel les nodes s'aimantent (celui de la grille fine).
pub const SNAP: f32 = 16.0;
const PIN_R: f32 = 5.0;
/// Contour de la sélection (orange d'Unreal).
const SELECTED: Color32 = Color32::from_rgb(0xF5, 0xA6, 0x23);
const BODY: Color32 = Color32::from_rgba_premultiplied(0x18, 0x18, 0x18, 0xF2);
const LABEL: Color32 = Color32::from_rgb(0xC8, 0xC8, 0xC8);

/// Pin repérée par sa node, sa clé et son côté.
#[derive(Clone, Debug, PartialEq)]
pub struct PinRef {
    pub node: u64,
    pub key: &'static str,
    pub output: bool,
    pub ty: PinType,
}

/// Déplacement en cours : déplacement total du pointeur et position de départ des nodes.
struct Drag {
    total: Vec2,
    origin: HashMap<u64, [f32; 2]>,
}

/// État de l'éditeur (non enregistré).
#[derive(Default)]
pub struct EditorState {
    pub selected: HashSet<u64>,
    /// Fil en cours de tirage, depuis cette pin.
    pub wire: Option<PinRef>,
    drag: Option<Drag>,
}

/// Simulation affichée : étapes déjà jouées et avancement de la dernière (0 → 1).
pub struct SimView<'a> {
    pub trace: &'a Trace,
    pub shown: usize,
    pub frac: f32,
}

/// Fil lâché dans le vide : la bibliothèque s'ouvre là, filtrée sur cette pin.
pub struct Dropped {
    pub from: PinRef,
    pub screen: Pos2,
    pub graph: Pos2,
}

/// Emplacement des éléments d'une node, en coordonnées du graph.
pub struct Layout {
    pub rect: Rect,
    pub params: Vec<(&'static PinDef, Rect)>,
    pub inputs: Vec<(&'static PinDef, Pos2, Rect)>,
    pub outputs: Vec<(&'static PinDef, Pos2, Rect)>,
}

/// Le champ est-il affiché (certains ne servent que pour un choix précis, ex. « Minutes avant ») ?
pub fn shown(p: &PinDef, n: &Node) -> bool {
    match p.show {
        None => true,
        Some((key, values)) => values.contains(&n.text(key).as_str()),
    }
}

// --- Mots-clés en pilules -------------------------------------------------------------

/// Mots-clés enregistrés (séparés par des virgules).
pub fn keywords(s: &str) -> Vec<String> {
    s.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()
}

const PILL_H: f32 = 18.0;
const PILL_GAP: f32 = 4.0;
const KW_INPUT_W: f32 = 84.0;

/// Largeur d'une pilule (texte + croix), en unités du graph.
fn pill_w(word: &str) -> f32 {
    word.chars().count() as f32 * 6.8 + 26.0
}

/// Place les pilules dans une zone de largeur `w`, à la suite, en passant à la ligne quand il
/// le faut ; le champ d'ajout vient après. Renvoie les rectangles (relatifs au coin de la zone)
/// et la hauteur totale.
fn keyword_flow(words: &[String], w: f32) -> (Vec<Rect>, Rect, f32) {
    let (mut x, mut y) = (0.0, 0.0);
    let mut place = |width: f32| {
        let width = width.min(w);
        if x > 0.0 && x + width > w {
            x = 0.0;
            y += PILL_H + PILL_GAP;
        }
        let r = Rect::from_min_size(Pos2::new(x, y), vec2(width, PILL_H));
        x += width + PILL_GAP;
        r
    };
    let pills: Vec<Rect> = words.iter().map(|k| place(pill_w(k))).collect();
    let input = place(KW_INPUT_W);
    (pills, input, input.bottom())
}

/// Hauteur de la ligne d'un réglage (les mots-clés peuvent prendre plusieurs lignes).
fn param_h(p: &PinDef, n: &Node, w: f32) -> f32 {
    match p.widget {
        Widget::Keywords(..) => {
            let (.., h) = keyword_flow(&keywords(&n.text(p.key)), field_w(w));
            (h + 8.0).max(PARAM_H)
        }
        _ => PARAM_H,
    }
}

/// Largeur du champ d'un réglage (à droite de son libellé).
fn field_w(node_w: f32) -> f32 {
    node_w * 0.6 - 10.0
}

pub fn layout(n: &Node, d: &'static NodeDef) -> Layout {
    let (x, top, w) = (n.pos[0], n.pos[1], d.width);
    let mut y = top + HEADER_H + 4.0;
    let params: Vec<_> = d
        .params()
        .filter(|p| shown(p, n))
        .map(|p| {
            let h = param_h(p, n, w);
            let r = Rect::from_min_size(Pos2::new(x, y), vec2(w, h));
            y += h;
            (p, r)
        })
        .collect();
    if !params.is_empty() {
        y += 4.0;
    }
    let ins: Vec<_> = d.wired_inputs().filter(|p| shown(p, n)).collect();
    let rows = ins.len().max(d.outputs.len());
    let row = |i: usize| Rect::from_min_size(Pos2::new(x, y + i as f32 * ROW_H), vec2(w, ROW_H));
    let inputs = ins.iter().enumerate().map(|(i, p)| (*p, Pos2::new(x + PIN_INSET, row(i).center().y), row(i))).collect();
    let outputs = d.outputs.iter().enumerate().map(|(i, p)| (p, Pos2::new(x + w - PIN_INSET, row(i).center().y), row(i))).collect();
    let h = y + rows as f32 * ROW_H + 6.0 - top;
    Layout { rect: Rect::from_min_size(Pos2::new(x, top), vec2(w, h)), params, inputs, outputs }
}

/// Rectangle de chaque node (rectangle de sélection, recherche de la pin sous le pointeur).
pub fn node_rects(g: &Graph) -> Vec<(u64, Rect)> {
    g.nodes.iter().filter_map(|n| Some((n.id, layout(n, n.def()?).rect))).collect()
}

fn pin_pos(g: &Graph, node: u64, key: &str, output: bool) -> Option<Pos2> {
    let n = g.node(node)?;
    let l = layout(n, n.def()?);
    let list = if output { &l.outputs } else { &l.inputs };
    list.iter().find(|(p, ..)| p.key == key).map(|(_, pos, _)| *pos)
}

/// Pin la plus proche du point `p` (graph), à moins de `reach`.
fn pin_at(g: &Graph, p: Pos2, reach: f32) -> Option<PinRef> {
    let mut best: Option<(f32, PinRef)> = None;
    for n in &g.nodes {
        let Some(d) = n.def() else { continue };
        let l = layout(n, d);
        let all = l.inputs.iter().map(|x| (x, false)).chain(l.outputs.iter().map(|x| (x, true)));
        for ((pd, pos, _), output) in all {
            let dist = pos.distance(p);
            if dist < reach && best.as_ref().is_none_or(|(b, _)| dist < *b) {
                best = Some((dist, PinRef { node: n.id, key: pd.key, output, ty: pd.ty }));
            }
        }
    }
    best.map(|b| b.1)
}

/// Relie deux pins dans le bon sens (sortie → entrée), si c'est possible.
pub fn connect(g: &mut Graph, a: &PinRef, b: &PinRef) -> bool {
    match (a.output, b.output) {
        (true, false) => g.connect(a.node, a.key, b.node, b.key),
        (false, true) => g.connect(b.node, b.key, a.node, a.key),
        _ => false,
    }
}

/// Valeur aimantée à la grille.
pub fn snap(v: f32) -> f32 {
    (v / SNAP).round() * SNAP
}

/// Aligne les nodes `ids` à la même hauteur : la sortie d'exécution de la node la plus à gauche
/// sert de référence, et chaque autre node place son entrée d'exécution (à défaut sa première
/// pin) sur cette ligne.
pub fn align(g: &mut Graph, ids: &HashSet<u64>) -> bool {
    let pins = |n: &Node, out_first: bool| -> Option<f32> {
        let l = layout(n, n.def()?);
        let exec = |list: &[(&'static PinDef, Pos2, Rect)]| list.iter().find(|p| p.0.ty == PinType::Exec).map(|p| p.1.y);
        let first = |list: &[(&'static PinDef, Pos2, Rect)]| list.first().map(|p| p.1.y);
        if out_first {
            exec(&l.outputs).or_else(|| exec(&l.inputs)).or_else(|| first(&l.outputs)).or_else(|| first(&l.inputs))
        } else {
            exec(&l.inputs).or_else(|| exec(&l.outputs)).or_else(|| first(&l.inputs)).or_else(|| first(&l.outputs))
        }
    };
    let Some(reference) = g.nodes.iter().filter(|n| ids.contains(&n.id)).min_by(|a, b| a.pos[0].total_cmp(&b.pos[0])) else { return false };
    let (ref_id, Some(line)) = (reference.id, pins(reference, true)) else { return false };
    let mut changed = false;
    let moves: Vec<(u64, f32)> = g.nodes.iter().filter(|n| ids.contains(&n.id) && n.id != ref_id).filter_map(|n| Some((n.id, line - pins(n, false)?))).collect();
    for (id, dy) in moves {
        if let Some(n) = g.node_mut(id)
            && dy.abs() > 0.01
        {
            n.pos[1] += dy;
            changed = true;
        }
    }
    changed
}

/// Caméra : passage du graph à l'écran (positions, longueurs, polices).
#[derive(Clone, Copy)]
struct Cam {
    center: Pos2,
    pan: Vec2,
    z: f32,
}

impl Cam {
    fn new(center: Pos2, v: &View) -> Self {
        Self { center, pan: Vec2::from(v.pan), z: v.zoom }
    }
    fn p(&self, g: Pos2) -> Pos2 {
        self.center + (g.to_vec2() - self.pan) * self.z
    }
    fn r(&self, r: Rect) -> Rect {
        Rect::from_min_max(self.p(r.min), self.p(r.max))
    }
    fn s(&self, v: f32) -> f32 {
        v * self.z
    }
    fn v(&self, x: f32, y: f32) -> Vec2 {
        vec2(x, y) * self.z
    }
    fn graph_pos(&self, s: Pos2) -> Pos2 {
        (self.pan + (s - self.center) / self.z).to_pos2()
    }
    /// Police à la taille de l'écran, arrondie au demi-point (moins de tailles différentes à
    /// préparer pour egui, sans perte de netteté).
    fn font(&self, size: f32) -> FontId {
        FontId::proportional(((size * self.z) * 2.0).round().max(2.0) / 2.0)
    }
    fn radius(&self, r: f32) -> CornerRadius {
        CornerRadius::same(self.s(r).round().clamp(0.0, 255.0) as u8)
    }
}

/// Dessine les fils et les nodes ; renvoie (graph modifié, fil lâché dans le vide).
pub fn show(ui: &mut egui::Ui, rect: Rect, center: Pos2, g: &mut Graph, ed: &mut EditorState, sim: Option<&SimView>) -> (bool, Option<Dropped>) {
    let ctx = ui.ctx().clone();
    let cam = Cam::new(center, &g.view);
    // Même calque que le fond du graph, par-dessus lui (créées après, les nodes reçoivent les
    // clics en premier ; un calque à part masquerait le fond pour egui).
    let mut nui = ui.new_child(egui::UiBuilder::new().max_rect(rect).sense(Sense::empty()));
    nui.set_clip_rect(rect.intersect(ui.clip_rect()));
    // Champs, cases et listes aux couleurs sombres d'Unreal et à l'échelle du zoom.
    style_widgets(&mut nui, cam.z);
    let pointer = ctx.pointer_latest_pos();
    let pointer_g = pointer.map(|p| cam.graph_pos(p));
    let mut changed = false;

    // Étapes jouées par la simulation.
    let played: &[super::sim::Step] = sim.map(|s| &s.trace.steps[..s.shown.min(s.trace.steps.len())]).unwrap_or(&[]);
    let current = played.last();
    let lit_nodes: HashSet<u64> = played.iter().map(|s| s.node).collect();
    let lit_links: HashSet<usize> = played.iter().filter_map(|s| s.link).collect();

    // 1. Fils (sous les nodes).
    let p = nui.painter().clone();
    for (i, l) in g.links.iter().enumerate() {
        let (Some(a), Some(b)) = (pin_pos(g, l.from, &l.out, true), pin_pos(g, l.to, &l.input, false)) else { continue };
        let ty = g.node(l.from).and_then(|n| n.def()).and_then(|d| d.output(&l.out)).map(|p| p.ty).unwrap_or(PinType::Exec);
        let lit = lit_links.contains(&i);
        let pulse = current.and_then(|s| s.link).filter(|&c| c == i).map(|_| sim.map(|s| s.frac).unwrap_or(1.0));
        wire(&p, &cam, cam.p(a), cam.p(b), ty, lit, pulse);
    }
    // Fil en cours de tirage.
    if let (Some(w), Some(ptr)) = (&ed.wire, pointer)
        && let Some(from) = pin_pos(g, w.node, w.key, w.output)
    {
        let from = cam.p(from);
        let (a, b) = if w.output { (from, ptr) } else { (ptr, from) };
        wire(&p, &cam, a, b, w.ty, false, None);
    }

    // 2. Nodes.
    let shift = ctx.input(|i| i.modifiers.shift || i.modifiers.command);
    let alt = ctx.input(|i| i.modifiers.alt);
    let mut drag_start = false;
    let mut dragging = false;
    let mut menu: Option<(u64, NodeMenu)> = None;
    let mut select_click: Option<u64> = None;
    let mut front: Option<u64> = None;
    let mut wire_start: Option<PinRef> = None;
    let mut break_pin: Option<PinRef> = None;
    let ids: Vec<u64> = g.nodes.iter().map(|n| n.id).collect();
    for id in ids {
        let Some(idx) = g.nodes.iter().position(|n| n.id == id) else { continue };
        let Some(def) = g.nodes[idx].def() else { continue };
        let lay = layout(&g.nodes[idx], def);
        let srect = cam.r(lay.rect);
        // Hors de la vue : rien à dessiner.
        if !nui.clip_rect().intersects(srect.expand(20.0)) {
            continue;
        }
        let body = nui.interact(srect, egui::Id::unique(("wt_bp_node", &g.id, id)), Sense::click_and_drag());
        if body.drag_started_by(egui::PointerButton::Primary) || body.clicked() {
            front = Some(id);
        }
        if body.drag_started_by(egui::PointerButton::Primary) {
            if !ed.selected.contains(&id) {
                if !shift {
                    ed.selected.clear();
                }
                ed.selected.insert(id);
            }
            drag_start = true;
        }
        if body.dragged_by(egui::PointerButton::Primary) {
            dragging = true;
        }
        if body.clicked() {
            select_click = Some(id);
        }
        body.context_menu(|ui| {
            if super::menu_item(ui, t!("Dupliquer", "Duplicate"), None, super::draw_copy).clicked() {
                menu = Some((id, NodeMenu::Duplicate));
            }
            if ui.button(t!("Rompre tous les fils", "Break all links")).clicked() {
                menu = Some((id, NodeMenu::BreakAll));
            }
            ui.separator();
            if super::menu_item(ui, t!("Supprimer", "Delete"), Some(crate::theme::TODAY()), super::draw_trash).clicked() {
                menu = Some((id, NodeMenu::Delete));
            }
        });
        let state = NodeLook { selected: ed.selected.contains(&id), lit: lit_nodes.contains(&id), current: current.is_some_and(|s| s.node == id) };
        paint_node(&p, &cam, def, &lay, &g.nodes[idx], g, state);
        // Infobulle : la description de la node, au survol de l'en-tête.
        let header = Rect::from_min_size(lay.rect.min, vec2(lay.rect.width(), HEADER_H));
        if body.hovered() && pointer_g.is_some_and(|q| header.contains(q)) && ed.wire.is_none() {
            body.clone().on_hover_text(def.desc());
        }

        // Champs (au-dessus du corps de la node pour les clics).
        let node = &mut g.nodes[idx];
        for (pd, r) in &lay.params {
            let r = cam.r(*r);
            if matches!(pd.widget, Widget::Bool(_)) {
                // Case à cocher, puis son libellé (comme dans Unreal).
                let field = Rect::from_min_size(Pos2::new(r.left() + cam.s(10.0), r.center().y - cam.s(9.0)), cam.v(18.0, 18.0));
                changed |= field_widget(&mut nui, &cam, field, node, pd, &g.id);
                p.text(Pos2::new(r.left() + cam.s(36.0), r.center().y), Align2::LEFT_CENTER, pd.label(), cam.font(12.0), LABEL);
                continue;
            }
            let label_w = r.width() * 0.4;
            // Libellé coupé avant le champ s'il est trop long (en haut de la ligne si elle est haute).
            let label_y = r.top() + cam.s(PARAM_H / 2.0);
            let clip = Rect::from_min_max(r.min, Pos2::new(r.left() + label_w - cam.s(4.0), r.bottom()));
            p.with_clip_rect(clip.intersect(p.clip_rect())).text(Pos2::new(r.left() + cam.s(10.0), label_y), Align2::LEFT_CENTER, pd.label(), cam.font(12.0), LABEL);
            if matches!(pd.widget, Widget::Keywords(..)) {
                let field = Rect::from_min_max(Pos2::new(r.left() + label_w, r.top() + cam.s(4.0)), Pos2::new(r.right() - cam.s(10.0), r.bottom() - cam.s(4.0)));
                changed |= keywords_widget(&mut nui, &cam, field, node, pd, &g.id);
                continue;
            }
            let field = Rect::from_min_max(Pos2::new(r.left() + label_w, r.top() + cam.s(2.0)), Pos2::new(r.right() - cam.s(10.0), r.bottom() - cam.s(2.0)));
            changed |= field_widget(&mut nui, &cam, field, node, pd, &g.id);
        }
        let has_out_row = |i: usize| i < def.outputs.len() && !def.outputs[i].fr.is_empty();
        let linked_in: Vec<bool> = lay.inputs.iter().map(|(pd, ..)| g.links.iter().any(|l| l.to == id && l.input == pd.key)).collect();
        let node = &mut g.nodes[idx];
        for (i, (pd, pos, r)) in lay.inputs.iter().enumerate() {
            let r = cam.r(*r);
            let linked = linked_in[i];
            let mut x = r.left() + cam.s(PIN_INSET + 12.0);
            if !pd.label().is_empty() {
                let gal = p.layout_no_wrap(pd.label().to_string(), cam.font(12.0), LABEL);
                let w = gal.size().x;
                p.galley(Pos2::new(x, r.center().y - gal.size().y / 2.0), gal, LABEL);
                x += w + cam.s(8.0);
            }
            if !linked && pd.ty != PinType::Exec && !matches!(pd.widget, Widget::None) {
                let right = if has_out_row(i) { r.left() + r.width() * 0.6 } else { r.right() - cam.s(10.0) };
                if right - x > cam.s(24.0) {
                    let field = if matches!(pd.widget, Widget::Bool(_)) {
                        Rect::from_min_size(Pos2::new(x, r.center().y - cam.s(9.0)), cam.v(18.0, 18.0))
                    } else {
                        Rect::from_min_max(Pos2::new(x, r.top() + cam.s(2.0)), Pos2::new(right, r.bottom() - cam.s(2.0)))
                    };
                    changed |= field_widget(&mut nui, &cam, field, node, pd, &g.id);
                }
            }
            if let Some(pr) = pin_widget(&mut nui, &cam, cam.p(*pos), PinRef { node: id, key: pd.key, output: false, ty: pd.ty }, &g.id) {
                match pr {
                    PinAction::Start(r) if alt => break_pin = Some(r),
                    PinAction::Start(r) => wire_start = Some(r),
                    PinAction::Break(r) => break_pin = Some(r),
                }
            }
        }
        for (pd, pos, r) in &lay.outputs {
            let r = cam.r(*r);
            if !pd.label().is_empty() {
                p.text(Pos2::new(r.right() - cam.s(PIN_INSET + 12.0), r.center().y), Align2::RIGHT_CENTER, pd.label(), cam.font(12.0), LABEL);
            }
            if let Some(pr) = pin_widget(&mut nui, &cam, cam.p(*pos), PinRef { node: id, key: pd.key, output: true, ty: pd.ty }, &g.id) {
                match pr {
                    PinAction::Start(r) if alt => break_pin = Some(r),
                    PinAction::Start(r) => wire_start = Some(r),
                    PinAction::Break(r) => break_pin = Some(r),
                }
            }
        }
    }

    // Déplacement de la sélection, aimanté à la grille : chaque node part de sa position de
    // départ plus le déplacement total du pointeur, arrondi au pas de la grille.
    if drag_start {
        ed.drag = Some(Drag { total: Vec2::ZERO, origin: g.nodes.iter().filter(|n| ed.selected.contains(&n.id)).map(|n| (n.id, n.pos)).collect() });
    }
    if dragging && let Some(d) = &mut ed.drag {
        d.total += ctx.input(|i| i.pointer.delta()) / cam.z;
        for n in g.nodes.iter_mut() {
            if let Some(o) = d.origin.get(&n.id) {
                let to = [snap(o[0] + d.total.x), snap(o[1] + d.total.y)];
                if to != n.pos {
                    n.pos = to;
                    changed = true;
                }
            }
        }
    } else if !dragging {
        ed.drag = None;
    }
    // La node cliquée ou déplacée passe au premier plan (dessinée en dernier).
    if let Some(id) = front
        && let Some(i) = g.nodes.iter().position(|n| n.id == id)
        && i + 1 < g.nodes.len()
    {
        let n = g.nodes.remove(i);
        g.nodes.push(n);
        changed = true;
    }
    if let Some(id) = select_click {
        if shift {
            if !ed.selected.remove(&id) {
                ed.selected.insert(id);
            }
        } else {
            ed.selected.clear();
            ed.selected.insert(id);
        }
    }
    if let Some(r) = break_pin {
        g.break_links(r.node, r.key, r.output);
        changed = true;
    } else if let Some(r) = wire_start {
        ed.wire = Some(r);
    }
    match menu {
        Some((id, NodeMenu::Delete)) => {
            let ids: Vec<u64> = if ed.selected.contains(&id) { ed.selected.iter().copied().collect() } else { vec![id] };
            g.remove_nodes(&ids);
            ed.selected.retain(|s| !ids.contains(s));
            changed = true;
        }
        Some((id, NodeMenu::Duplicate)) => {
            let ids: Vec<u64> = if ed.selected.contains(&id) { ed.selected.iter().copied().collect() } else { vec![id] };
            ed.selected = g.duplicate_nodes(&ids).into_iter().collect();
            changed = true;
        }
        Some((id, NodeMenu::BreakAll)) => {
            g.links.retain(|l| l.from != id && l.to != id);
            changed = true;
        }
        None => {}
    }

    // Fil lâché : sur une pin compatible, ou dans le vide.
    let mut dropped = None;
    if ed.wire.is_some() && !ctx.input(|i| i.pointer.primary_down()) {
        let w = ed.wire.take().expect("fil en cours");
        if let (Some(ptr), Some(gp)) = (pointer, pointer_g) {
            match pin_at(g, gp, 14.0) {
                Some(target) => changed |= connect(g, &w, &target),
                None if !node_rects(g).iter().any(|(_, r)| r.contains(gp)) && rect.contains(ptr) => {
                    dropped = Some(Dropped { from: w, screen: ptr, graph: gp });
                }
                None => {}
            }
        }
    }

    // Raccourcis clavier (quand aucun champ n'a le clavier).
    if !ctx.egui_wants_keyboard_input() && !ed.selected.is_empty() {
        let (del, dup, q) = ctx.input(|i| {
            (
                i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace),
                i.modifiers.command && i.key_pressed(egui::Key::D),
                !i.modifiers.any() && i.key_pressed(egui::Key::Q),
            )
        });
        if del {
            let ids: Vec<u64> = ed.selected.drain().collect();
            g.remove_nodes(&ids);
            changed = true;
        } else if dup {
            let ids: Vec<u64> = ed.selected.iter().copied().collect();
            ed.selected = g.duplicate_nodes(&ids).into_iter().collect();
            changed = true;
        } else if q {
            changed |= align(g, &ed.selected);
        }
    }
    ed.selected.retain(|id| g.node(*id).is_some());
    (changed, dropped)
}

enum NodeMenu {
    Duplicate,
    BreakAll,
    Delete,
}

enum PinAction {
    Start(PinRef),
    Break(PinRef),
}

#[derive(Clone, Copy)]
struct NodeLook {
    selected: bool,
    lit: bool,
    current: bool,
}

/// Couleurs sombres d'Unreal, et tailles (texte, cases, marges) à l'échelle du zoom.
fn style_widgets(ui: &mut egui::Ui, z: f32) {
    let st = ui.style_mut();
    let px = |v: f32| ((v * z) * 2.0).round().max(2.0) / 2.0;
    for (ts, size) in [(egui::TextStyle::Body, 12.0), (egui::TextStyle::Button, 12.0), (egui::TextStyle::Small, 10.0), (egui::TextStyle::Monospace, 12.0)] {
        st.text_styles.insert(ts, FontId::proportional(px(size)));
    }
    st.spacing.interact_size = vec2(40.0, 20.0) * z;
    st.spacing.icon_width = 14.0 * z;
    st.spacing.icon_width_inner = 8.0 * z;
    st.spacing.icon_spacing = 4.0 * z;
    st.spacing.button_padding = vec2(6.0, 2.0) * z;
    st.spacing.item_spacing = vec2(4.0, 4.0) * z;
    let v = &mut st.visuals;
    v.dark_mode = true;
    v.override_text_color = Some(Color32::from_gray(0xE0));
    v.extreme_bg_color = Color32::from_gray(0x0E);
    v.text_edit_bg_color = Some(Color32::from_gray(0x0E));
    let k = CornerRadius::same((3.0 * z).round().clamp(1.0, 255.0) as u8);
    for (w, fill) in [(&mut v.widgets.inactive, 0x2A), (&mut v.widgets.hovered, 0x36), (&mut v.widgets.active, 0x40), (&mut v.widgets.open, 0x36)] {
        w.bg_fill = Color32::from_gray(fill);
        w.weak_bg_fill = Color32::from_gray(fill);
        w.bg_stroke = Stroke::new(1.0, Color32::from_gray(0x44));
        w.fg_stroke = Stroke::new((1.0 * z).max(0.6), Color32::from_gray(0xE0));
        w.corner_radius = k;
    }
    v.selection.bg_fill = Color32::from_rgb(0x2A, 0x5C, 0xA8);
}

/// Les menus déroulants s'ouvrent à taille normale, même quand le graph est dézoomé.
fn normal_style(ui: &mut egui::Ui) {
    let global = ui.ctx().global_style();
    ui.set_style(global);
}

/// Champ d'une entrée ou d'un réglage. Renvoie `true` si la valeur a changé.
fn field_widget(ui: &mut egui::Ui, cam: &Cam, r: Rect, node: &mut Node, pd: &PinDef, graph: &str) -> bool {
    let key = pd.key;
    let id = egui::Id::unique(("wt_bp_field", graph, node.id, key));
    let set = |node: &mut Node, v: serde_json::Value| {
        node.values.insert(key.to_string(), v);
        true
    };
    match pd.widget {
        Widget::None => false,
        Widget::Keywords(..) => keywords_widget(ui, cam, r, node, pd, graph),
        Widget::Text(hint_fr, hint_en) => {
            let mut s = node.text(key);
            let edit = egui::TextEdit::singleline(&mut s).id(id).hint_text(t!(hint_fr, hint_en)).font(cam.font(12.0)).margin(cam.v(5.0, 2.0)).desired_width(r.width());
            if ui.put(r, edit).changed() { set(node, s.into()) } else { false }
        }
        Widget::Bool(_) => {
            let mut b = node.flag(key);
            if ui.put(r, egui::Checkbox::without_text(&mut b)).changed() { set(node, b.into()) } else { false }
        }
        Widget::Int(_, lo, hi) => {
            let mut v = node.int(key);
            let r = Rect::from_min_size(r.min, vec2(r.width().min(cam.s(80.0)), r.height()));
            if ui.put(r, egui::DragValue::new(&mut v).range(lo..=hi)).changed() { set(node, v.into()) } else { false }
        }
        Widget::Choice(opts) => {
            let cur = node.text(key);
            let label = opts.iter().find(|o| o.0 == cur).map(|o| t!(o.1, o.2)).unwrap_or(t!(opts[0].1, opts[0].2));
            let mut chosen = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                egui::ComboBox::from_id_salt(id).width(r.width() - cam.s(8.0)).selected_text(egui::RichText::new(label).font(cam.font(12.0))).show_ui(ui, |ui| {
                    normal_style(ui);
                    for (k, fr, en) in opts {
                        if ui.selectable_label(cur == *k, t!(*fr, *en)).clicked() {
                            chosen = Some(*k);
                        }
                    }
                });
            });
            match chosen {
                Some(k) if k != cur => set(node, k.into()),
                _ => false,
            }
        }
        Widget::Icon => {
            let cur = node.text(key);
            // Aperçu du logo à gauche de la liste.
            let ic = Rect::from_center_size(Pos2::new(r.left() + cam.s(9.0), r.center().y), cam.v(14.0, 14.0));
            draw_logo(ui.painter(), ic, &cur, Color32::from_gray(0xE0));
            let r = Rect::from_min_max(Pos2::new(r.left() + cam.s(24.0), r.top()), r.max);
            let label = nodes::ICONS.iter().find(|o| o.0 == cur).map(|o| t!(o.1, o.2)).unwrap_or("Mail");
            let mut chosen = None;
            ui.scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                egui::ComboBox::from_id_salt(id).width(r.width() - cam.s(8.0)).selected_text(egui::RichText::new(label).font(cam.font(12.0))).show_ui(ui, |ui| {
                    normal_style(ui);
                    for (k, fr, en) in nodes::ICONS {
                        if ui.selectable_label(cur == k, t!(fr, en)).clicked() {
                            chosen = Some(k);
                        }
                    }
                });
            });
            match chosen {
                Some(k) if k != cur => set(node, k.into()),
                _ => false,
            }
        }
    }
}

/// Mots-clés en pilules : chaque mot validé (Entrée ou virgule) devient une pilule, retirée
/// par sa croix ; le champ d'ajout suit la dernière pilule.
fn keywords_widget(ui: &mut egui::Ui, cam: &Cam, r: Rect, node: &mut Node, pd: &PinDef, graph: &str) -> bool {
    let Widget::Keywords(hint_fr, hint_en) = pd.widget else { return false };
    let key = pd.key;
    let mut words = keywords(&node.text(key));
    let (pills, input, _) = keyword_flow(&words, r.width() / cam.z);
    let at = |q: Rect| Rect::from_min_size(r.min + q.min.to_vec2() * cam.z, q.size() * cam.z);
    let p = ui.painter().clone();
    let mut remove = None;
    for (i, (w, q)) in words.iter().zip(&pills).enumerate() {
        let pr = at(*q);
        p.rect(pr, CornerRadius::same(255), Color32::from_rgb(0x3A, 0x2A, 0x36), Stroke::new(1.0, Color32::from_rgb(0x7A, 0x3E, 0x68)), egui::StrokeKind::Inside);
        let text_clip = Rect::from_min_max(pr.min, Pos2::new(pr.right() - cam.s(16.0), pr.bottom()));
        p.with_clip_rect(text_clip.intersect(p.clip_rect())).text(
            Pos2::new(pr.left() + cam.s(8.0), pr.center().y),
            Align2::LEFT_CENTER,
            w,
            cam.font(11.5),
            Color32::from_gray(0xF0),
        );
        // Croix : retire le mot.
        let x = Rect::from_center_size(Pos2::new(pr.right() - cam.s(9.0), pr.center().y), cam.v(14.0, 14.0));
        let resp = ui.interact(x, egui::Id::unique(("wt_bp_kw_x", graph, node.id, key, i)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let ink = if resp.hovered() { Color32::WHITE } else { Color32::from_gray(0xB0) };
        let (c, k) = (x.center(), cam.s(2.8));
        p.line_segment([c + vec2(-k, -k), c + vec2(k, k)], Stroke::new(cam.s(1.3), ink));
        p.line_segment([c + vec2(k, -k), c + vec2(-k, k)], Stroke::new(cam.s(1.3), ink));
        if resp.clicked() {
            remove = Some(i);
        }
    }
    let mut changed = false;
    if let Some(i) = remove {
        words.remove(i);
        changed = true;
    }
    // Champ d'ajout : le texte en cours de saisie reste en mémoire d'egui.
    let id = egui::Id::unique(("wt_bp_kw_input", graph, node.id, key));
    let mut draft: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
    let hint = if words.is_empty() { t!(hint_fr, hint_en) } else { "+" };
    let edit = egui::TextEdit::singleline(&mut draft).id(id).hint_text(hint).font(cam.font(11.5)).margin(cam.v(5.0, 1.0)).desired_width(at(input).width());
    let resp = ui.put(at(input), edit).on_hover_text(t!(
        "Entrée ou virgule : ajouter le mot-clé. Le mot entier doit figurer dans le texte (« ren » ne trouve pas « rendez-vous »).",
        "Enter or comma: add the keyword. The whole word must appear in the text (“meet” doesn't match “meeting”)."
    ));
    let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
    if draft.contains(',') || enter || (resp.lost_focus() && !draft.trim().is_empty()) {
        for k in keywords(&draft) {
            if !words.iter().any(|w| w.eq_ignore_ascii_case(&k)) {
                words.push(k);
                changed = true;
            }
        }
        draft.clear();
        // Entrée : on garde la main pour taper le mot suivant.
        if enter {
            resp.request_focus();
        }
    } else if (resp.has_focus() && draft.is_empty() && ui.input(|i| i.key_pressed(egui::Key::Backspace))) && words.pop().is_some() {
        changed = true;
    }
    ui.data_mut(|d| d.insert_temp(id, draft));
    if changed {
        node.values.insert(key.to_string(), words.join(", ").into());
    }
    changed
}

/// Onglet dont l'icône sert de logo de notification.
/// Onglet dont le logo représente `key` (`None` : rappel, logo du réveil).
pub fn icon_tab(key: &str) -> Option<crate::app::Tab> {
    use crate::app::Tab;
    Some(match key {
        "messages" => Tab::Messages,
        "calendar" => Tab::Calendar,
        "notes" => Tab::Notes,
        "dashboard" => Tab::Dashboard,
        "files" => Tab::Files,
        "blueprint" => Tab::Blueprint,
        "reminder" => return None,
        _ => Tab::Mail,
    })
}

/// Dessine le logo `key` (celui d'un onglet, ou le réveil des rappels).
fn draw_logo(p: &egui::Painter, r: Rect, key: &str, col: Color32) {
    match icon_tab(key) {
        Some(tab) => crate::sidebar::draw_icon(p, r, tab, col, 1.0),
        None => crate::widgets::alarm_icon(p, r.center(), r.width() * 1.1, col, 1.0),
    }
}

/// Zone cliquable d'une pin : tirer un fil, ou le rompre (Alt + clic).
fn pin_widget(ui: &mut egui::Ui, cam: &Cam, pos: Pos2, pin: PinRef, graph: &str) -> Option<PinAction> {
    let r = Rect::from_center_size(pos, cam.v(18.0, 18.0));
    let resp = ui.interact(r, egui::Id::unique(("wt_bp_pin", graph, pin.node, pin.key, pin.output)), Sense::click_and_drag());
    let resp = resp.on_hover_cursor(egui::CursorIcon::Crosshair).on_hover_text(pin.ty.label());
    if resp.hovered() {
        ui.painter().circle_filled(pos, cam.s(PIN_R + 4.0), Color32::from_white_alpha(18));
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        return Some(PinAction::Start(pin));
    }
    if resp.clicked() && ui.input(|i| i.modifiers.alt) {
        return Some(PinAction::Break(pin));
    }
    None
}

fn paint_node(p: &egui::Painter, cam: &Cam, d: &NodeDef, l: &Layout, n: &Node, g: &Graph, look: NodeLook) {
    let r = cam.r(l.rect);
    let k = cam.radius(8.0);
    // Ombre portée.
    p.rect_filled(r.translate(cam.v(0.0, 4.0)).expand(cam.s(2.0)), cam.radius(10.0), Color32::from_black_alpha(70));
    p.rect_filled(r, k, BODY);
    // En-tête uni, de la couleur de la catégorie.
    let header = Rect::from_min_size(r.min, vec2(r.width(), cam.s(HEADER_H)));
    p.rect_filled(header, CornerRadius { nw: k.nw, ne: k.ne, sw: 0, se: 0 }, d.cat.color());
    p.hline(header.x_range(), header.bottom(), Stroke::new(1.0, Color32::from_black_alpha(120)));
    // Logo du type de node, puis le titre.
    node_icon(p, Pos2::new(header.left() + cam.s(15.0), header.center().y), cam.s(14.0), d.key, Color32::WHITE);
    p.text(Pos2::new(header.left() + cam.s(30.0), header.center().y), Align2::LEFT_CENTER, d.title(), cam.font(13.5), Color32::WHITE);
    // Séparateur sous les réglages.
    if let Some((_, last)) = l.params.last() {
        let y = cam.p(last.max).y + cam.s(2.0);
        p.hline(r.x_range().shrink(cam.s(8.0)), y, Stroke::new(1.0, Color32::from_white_alpha(14)));
    }
    // Pins.
    for (pd, pos, _) in &l.inputs {
        pin_shape(p, cam, cam.p(*pos), pd.ty, g.is_linked(n.id, pd.key, false));
    }
    for (pd, pos, _) in &l.outputs {
        pin_shape(p, cam, cam.p(*pos), pd.ty, g.is_linked(n.id, pd.key, true));
    }
    // Contour : simulation (orange), sélection, sinon discret.
    let sim = super::canvas::SIM;
    if look.current {
        p.rect_stroke(r.expand(cam.s(3.0)), cam.radius(11.0), Stroke::new(cam.s(3.0), sim.gamma_multiply(0.5)), egui::StrokeKind::Outside);
        p.rect_stroke(r, k, Stroke::new(cam.s(2.0), sim), egui::StrokeKind::Outside);
    } else if look.lit {
        p.rect_stroke(r, k, Stroke::new(cam.s(2.0), sim.gamma_multiply(0.7)), egui::StrokeKind::Outside);
    } else if look.selected {
        p.rect_stroke(r, k, Stroke::new(cam.s(2.0), SELECTED), egui::StrokeKind::Outside);
    } else {
        p.rect_stroke(r, k, Stroke::new(1.0, Color32::from_black_alpha(200)), egui::StrokeKind::Outside);
    }
    if look.selected && (look.lit || look.current) {
        p.rect_stroke(r.expand(cam.s(3.0)), cam.radius(11.0), Stroke::new(cam.s(1.0), SELECTED), egui::StrokeKind::Outside);
    }
}

/// Logo d'un type de node, dessiné (centre `c`, côté `s`).
pub fn node_icon(p: &egui::Painter, c: Pos2, s: f32, key: &str, col: Color32) {
    use crate::app::Tab;
    let u = s / 14.0;
    let st = Stroke::new((1.5 * u).max(0.8), col);
    let v = |x: f32, y: f32| c + vec2(x, y) * u;
    let tab = |t: Tab| crate::sidebar::draw_icon(p, Rect::from_center_size(c, Vec2::splat(s)), t, col, 1.0);
    let text = |t: &str| {
        p.text(c, Align2::CENTER_CENTER, t, FontId::monospace((12.0 * u * 2.0).round() / 2.0), col);
    };
    match key {
        "ev_mail" => tab(Tab::Mail),
        "ev_chat" => tab(Tab::Messages),
        "ev_event" | "act_event" => tab(Tab::Calendar),
        "act_note" => tab(Tab::Notes),
        "act_group" => tab(Tab::Files),
        // Rappels : le réveil du tableau de bord (barré pour la suppression, flèche pour reporter).
        "ev_reminder" | "act_reminder" => crate::widgets::alarm_icon(p, c, s * 1.1, col, 1.0),
        "act_snooze" => {
            crate::widgets::alarm_icon(p, c + vec2(-1.5, 0.0) * u, s * 0.95, col, 1.0);
            p.add(egui::Shape::line(vec![v(3.5, 3.0), v(6.5, 3.0), v(3.5, 6.5), v(6.5, 6.5)], Stroke::new((1.2 * u).max(0.7), col)));
        }
        "act_unremind" => {
            crate::widgets::alarm_icon(p, c, s * 1.0, col, 1.0);
            p.line_segment([v(-6.0, -6.0), v(6.0, 6.0)], st);
        }
        "act_notify" => {
            // Cloche.
            let pts = vec![v(-5.0, 3.5), v(-4.0, 2.0), v(-4.0, -1.5), v(-2.5, -4.5), v(0.0, -5.5), v(2.5, -4.5), v(4.0, -1.5), v(4.0, 2.0), v(5.0, 3.5)];
            p.add(egui::Shape::closed_line(pts, st));
            p.circle_filled(v(0.0, 5.5), 1.4 * u, col);
        }
        "act_task" => {
            p.rect_stroke(Rect::from_center_size(c, Vec2::splat(11.0 * u)), CornerRadius::same((2.0 * u) as u8), st, egui::StrokeKind::Inside);
            p.add(egui::Shape::line(vec![v(-3.0, 0.0), v(-1.0, 2.5), v(3.5, -2.5)], st));
        }
        "act_pin" => {
            // Punaise : tête ronde, aiguille.
            p.circle_stroke(v(1.5, -2.5), 3.2 * u, st);
            p.line_segment([v(-0.5, -0.5), v(-5.0, 5.0)], st);
        }
        "act_star" => crate::tabs::mail::view::draw_star(p, c, 6.0 * u, false, col),
        "act_trash" => {
            p.hline(v(-5.0, 0.0).x..=v(5.0, 0.0).x, v(0.0, -3.5).y, st);
            p.hline(v(-1.5, 0.0).x..=v(1.5, 0.0).x, v(0.0, -5.5).y, st);
            p.rect_stroke(Rect::from_min_max(v(-3.5, -3.5), v(3.5, 5.5)), CornerRadius::same(u as u8), st, egui::StrokeKind::Inside);
        }
        "act_category" => {
            // Étiquette : pointe à gauche, trou.
            p.add(egui::Shape::closed_line(vec![v(-6.0, 0.0), v(-2.0, -4.5), v(6.0, -4.5), v(6.0, 4.5), v(-2.0, 4.5)], st));
            p.circle_filled(v(-1.5, 0.0), 1.3 * u, col);
        }
        "pu_break" => {
            // Une entrée qui s'éclate en trois sorties.
            p.line_segment([v(-6.0, 0.0), v(-1.5, 0.0)], st);
            for y in [-4.5, 0.0, 4.5] {
                p.line_segment([v(-1.5, 0.0), v(4.0, y)], st);
                p.circle_filled(v(5.0, y), 1.3 * u, col);
            }
        }
        "fl_branch" => {
            // Fourche : une entrée, deux sorties.
            p.line_segment([v(-6.0, 0.0), v(-1.0, 0.0)], st);
            p.add(egui::Shape::line(vec![v(5.5, -4.0), v(1.0, -4.0), v(-1.0, 0.0), v(1.0, 4.0), v(5.5, 4.0)], st));
        }
        "fl_if" => {
            p.add(egui::Shape::closed_line(vec![v(0.0, -6.0), v(6.0, 0.0), v(0.0, 6.0), v(-6.0, 0.0)], st));
            p.line_segment([v(-2.5, 0.0), v(2.5, 0.0)], st);
        }
        "fl_foreach" => {
            // Flèche en boucle.
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let a = 0.6 + i as f32 / 16.0 * 5.0;
                    v(a.cos() * 5.0, a.sin() * 5.0)
                })
                .collect();
            let end = *pts.last().unwrap_or(&c);
            p.add(egui::Shape::line(pts, st));
            p.add(egui::Shape::convex_polygon(vec![end + vec2(2.5, -2.5) * u, end + vec2(2.5, 2.5) * u, end + vec2(-1.5, 0.5) * u], col, Stroke::NONE));
        }
        "fl_sequence" => {
            for y in [-4.0, 0.0, 4.0] {
                p.circle_filled(v(-4.5, y), 1.2 * u, col);
                p.line_segment([v(-2.0, y), v(5.5, y)], st);
            }
        }
        "pu_contains" => {
            p.circle_stroke(v(-1.0, -1.0), 4.0 * u, st);
            p.line_segment([v(2.0, 2.0), v(5.5, 5.5)], st);
        }
        "act_setvar" => text("x="),
        "act_addvar" => text("+1"),
        "pu_getvar" => text("x"),
        "pu_compare" => text("<>"),
        "pu_math" => {
            // Plus et moins.
            p.line_segment([v(-6.0, -2.5), v(-1.0, -2.5)], st);
            p.line_segment([v(-3.5, -5.0), v(-3.5, 0.0)], st);
            p.line_segment([v(1.0, 3.5), v(6.0, 3.5)], st);
        }
        "pu_now" => {
            // Horloge.
            p.circle_stroke(c, 6.0 * u, st);
            p.line_segment([c, v(0.0, -3.8)], st);
            p.line_segment([c, v(2.8, 1.2)], st);
        }
        "fl_once" => {
            p.circle_stroke(c, 6.0 * u, st);
            p.text(c, Align2::CENTER_CENTER, "1", FontId::proportional((10.0 * u * 2.0).round() / 2.0), col);
        }
        "fl_delay" => {
            // Sablier.
            p.add(egui::Shape::closed_line(vec![v(-4.5, -6.0), v(4.5, -6.0), v(-4.5, 6.0), v(4.5, 6.0)], st));
        }
        "pu_not" => text("!"),
        "pu_and" => text("&"),
        "pu_or" => text("|"),
        "pu_join" => text("+"),
        _ => {
            p.circle_stroke(c, 4.0 * u, st);
        }
    }
}

/// Forme d'une pin : flèche pour l'exécution, carré pour une liste, rond sinon ; pleine si reliée.
fn pin_shape(p: &egui::Painter, cam: &Cam, c: Pos2, ty: PinType, linked: bool) {
    let col = ty.color();
    let w = cam.s(1.6).max(0.8);
    match ty {
        PinType::Exec => {
            let pts = vec![c + cam.v(-5.0, -5.5), c + cam.v(1.0, -5.5), c + cam.v(6.0, 0.0), c + cam.v(1.0, 5.5), c + cam.v(-5.0, 5.5)];
            if linked {
                p.add(egui::Shape::convex_polygon(pts, col, Stroke::NONE));
            } else {
                p.add(egui::Shape::closed_line(pts, Stroke::new(w, col)));
            }
        }
        PinType::List => {
            let r = Rect::from_center_size(c, cam.v(9.0, 9.0));
            if linked {
                p.rect_filled(r, CornerRadius::same(1), col);
            } else {
                p.rect_stroke(r, CornerRadius::same(1), Stroke::new(w, col), egui::StrokeKind::Inside);
            }
        }
        _ => {
            if linked {
                p.circle_filled(c, cam.s(PIN_R), col);
            } else {
                p.circle_stroke(c, cam.s(PIN_R - 0.5), Stroke::new(w, col));
            }
        }
    }
}

/// Fil en courbe de Bézier horizontale, entre deux points de l'écran. `lit` : parcouru par la
/// simulation ; `pulse` : impulsion en cours sur ce fil (0 → 1).
fn wire(p: &egui::Painter, cam: &Cam, a: Pos2, b: Pos2, ty: PinType, lit: bool, pulse: Option<f32>) {
    let dx = ((b.x - a.x).abs() * 0.5).max(cam.s(50.0));
    let pts = [a, a + vec2(dx, 0.0), b - vec2(dx, 0.0), b];
    let base = ty.color();
    let width = cam.s(if ty == PinType::Exec { 2.6 } else { 2.0 }).max(1.0);
    let sim = super::canvas::SIM;
    let (col, w) = if lit { (sim, width + cam.s(1.0)) } else { (if ty == PinType::Exec { Color32::from_gray(0xE8) } else { base }, width) };
    if lit {
        p.add(egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(w + cam.s(5.0), sim.gamma_multiply(0.18))));
    }
    let shape = egui::epaint::CubicBezierShape::from_points_stroke(pts, false, Color32::TRANSPARENT, Stroke::new(w, col));
    let dot = pulse.map(|t| shape.sample(t.clamp(0.0, 1.0)));
    p.add(shape);
    if let Some(dot) = dot {
        p.circle_filled(dot, cam.s(5.0), Color32::WHITE);
        p.circle_filled(dot, cam.s(9.0), sim.gamma_multiply(0.35));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_inside_the_node() {
        let mut g = Graph::new("t");
        let id = g.add_node("act_notify", [100.0, 50.0]);
        let n = g.node(id).unwrap();
        let w = n.def().unwrap().width;
        let l = layout(n, n.def().unwrap());
        assert!(l.inputs[0].1.x > 100.0 && l.outputs[0].1.x < 100.0 + w);
        assert!(l.rect.height() > HEADER_H);
        let hit = pin_at(&g, l.outputs[0].1 + vec2(3.0, 2.0), 14.0).unwrap();
        assert!(hit.output && hit.key == "then");
    }

    #[test]
    fn snaps_to_the_grid() {
        assert_eq!(snap(7.0), 0.0);
        assert_eq!(snap(9.0), 16.0);
        assert_eq!(snap(-25.0), -32.0);
    }

    #[test]
    fn q_aligns_on_the_leftmost_exec_output() {
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let a = g.add_node("act_notify", [400.0, 137.0]);
        let b = g.add_node("fl_branch", [800.0, -50.0]);
        let sel: HashSet<u64> = [ev, a, b].into_iter().collect();
        assert!(align(&mut g, &sel));
        let y = |id: u64, key: &str, out: bool| pin_pos(&g, id, key, out).unwrap().y;
        let line = y(ev, "then", true);
        assert!((y(a, "exec", false) - line).abs() < 0.01);
        assert!((y(b, "exec", false) - line).abs() < 0.01);
        // La node de référence ne bouge pas.
        assert_eq!(g.node(ev).unwrap().pos, [0.0, 0.0]);
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;

    #[test]
    fn fields_follow_the_choice() {
        let mut g = Graph::new("t");
        let r = g.add_node("act_reminder", [0.0, 0.0]);
        let keys = |g: &Graph| -> Vec<&str> {
            let n = g.node(r).unwrap();
            layout(n, n.def().unwrap()).inputs.iter().map(|p| p.0.key).collect()
        };
        assert!(keys(&g).contains(&"in_minutes") && !keys(&g).contains(&"at"));
        g.node_mut(r).unwrap().values.insert("when".into(), "at".into());
        assert!(!keys(&g).contains(&"in_minutes") && keys(&g).contains(&"at"));
    }

    #[test]
    fn keyword_pills_wrap() {
        let words: Vec<String> = ["urgent", "facture", "réunion", "client important"].iter().map(|s| s.to_string()).collect();
        let (pills, input, h) = keyword_flow(&words, 150.0);
        assert_eq!(pills.len(), 4);
        assert!(pills.iter().all(|r| r.right() <= 150.0 + 0.01));
        assert!(h > PILL_H * 2.0);
        assert!(input.top() >= pills[3].top());
    }
}
