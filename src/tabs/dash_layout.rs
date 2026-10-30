//! Disposition du tableau de bord, par rangées. Chaque rangée occupe toujours toute la largeur :
//! ses panneaux se la partagent (parts en %, dont la somme fait 100 %), et ils ont tous la
//! hauteur de la rangée. Un panneau ne change jamais de rangée tout seul.
//!
//! - Glisser un panneau par sa barre de titre : le déposer dans une rangée (il y prend une part
//!   égale aux autres, qui rétrécissent), ou en haut d'une rangée / sous la dernière pour créer
//!   une nouvelle rangée. Près de sa place, il y est attiré (aimant) ; lâché, il s'y pose avec un
//!   léger rebond. Une rangée vidée disparaît, les autres panneaux de sa rangée d'origine
//!   s'élargissent pour la remplir.
//! - Glisser le coin en bas à droite : la largeur change la part du panneau, les autres panneaux
//!   de la rangée rétrécissent ou s'élargissent en proportion (les parts s'affichent en %, et
//!   s'aimantent à ¼, ⅓, ½, ⅔, ¾) ; la hauteur change celle de toute la rangée.
//!
//! La disposition est gardée dans les préférences (`Prefs::dashboard_rows`, et
//! `CustomTab::rows` pour les onglets personnalisés).
//!
//! Onglets personnalisés ([`show_free`]) : mêmes panneaux, mais choisis librement (galerie de
//! panneaux) ; chaque panneau s'y appelle `type#n` et se retire par sa pastille « × ».

use std::collections::HashMap;

use eframe::egui::{self, Color32, CornerRadius, FontId, Pos2, Rect, Sense, Stroke, Vec2, vec2};

use crate::prefs::{DashPanel, DashRow};
use crate::theme;

/// Hauteur d'un cran de rangée et espace entre deux panneaux.
pub const UNIT: f32 = 28.0;
pub const GAP: f32 = 12.0;
/// Hauteur de la barre de titre (zone où l'on attrape le panneau).
const GRIP_H: f32 = 44.0;
/// Coin de redimensionnement.
const CORNER: f32 = 18.0;
/// Distance (px) à partir de laquelle le panneau déplacé est attiré par sa place.
const MAGNET: f32 = 70.0;
/// Haut d'une rangée où déposer un panneau crée une nouvelle rangée au-dessus.
const NEW_ROW_BAND: f32 = 36.0;
/// Parts « rondes » qui aimantent la part pendant le redimensionnement, et leur portée.
const NICE: [f32; 5] = [0.25, 1.0 / 3.0, 0.5, 2.0 / 3.0, 0.75];
const NICE_REACH: f32 = 0.025;
/// En dessous de cette largeur, les panneaux sont empilés (un par ligne), sans modification.
const STACK_BELOW: f32 = 700.0;

/// Panneau connu : identifiant, largeur minimale (px) et hauteur minimale (crans).
pub struct PanelDef {
    pub id: &'static str,
    pub min_px: f32,
    pub min_h: u8,
    /// Toujours présent sur le tableau de bord (sinon : onglets personnalisés seulement).
    pub dashboard: bool,
}

const fn panel(id: &'static str, min_px: f32, min_h: u8, dashboard: bool) -> PanelDef {
    PanelDef { id, min_px, min_h, dashboard }
}

pub const PANELS: [PanelDef; 9] = [
    panel("links", 200.0, 5, true),
    panel("planning", 150.0, 4, true),
    panel("timeline", 150.0, 4, true),
    panel("mails", 150.0, 4, true),
    panel("pinned", 150.0, 4, true),
    panel("reminders", 180.0, 4, true),
    panel("tasks", 360.0, 6, true),
    panel("files", 240.0, 3, true),
    panel("note", 160.0, 3, false),
];

/// Type d'un panneau : « tasks » pour « tasks#3 » (onglets personnalisés).
pub fn kind(id: &str) -> &str {
    id.split('#').next().unwrap_or(id)
}

fn def(id: &str) -> Option<&'static PanelDef> {
    let k = kind(id);
    PANELS.iter().find(|p| p.id == k)
}

/// Ajoute un panneau en bas (dans la dernière rangée si elle a de la place, sinon dans une
/// nouvelle rangée) : onglets personnalisés.
pub fn append(rows: &mut Vec<DashRow>, id: &str, width: f32) {
    let min_h = def(id).map_or(4, |d| d.min_h);
    // Largeur confortable (un peu plus que le minimum du redimensionnement).
    let min_px = |id: &str| def(id).map_or(200.0, |d| (d.min_px * 1.35).max(240.0));
    // Place à parts égales pour tous, chacun au moins à sa largeur minimale.
    let fits = |r: &DashRow| {
        let n = r.panels.len() as f32 + 1.0;
        let each = (width - GAP * (n - 1.0)) / n;
        r.panels.len() < 3 && r.panels.iter().map(|p| min_px(&p.id)).chain([min_px(id)]).all(|m| m <= each)
    };
    match rows.last() {
        Some(r) if fits(r) => {
            let row = rows.len() - 1;
            let at = r.panels.len();
            insert(rows, id, min_h.max(8), Drop::Into { row, at });
        }
        _ => insert(rows, id, min_h.max(8), Drop::NewRow { row: rows.len() }),
    }
}

fn row(h: u8, panels: &[(&str, f32)]) -> DashRow {
    DashRow { h, panels: panels.iter().map(|(id, s)| DashPanel { id: id.to_string(), share: *s }).collect() }
}

pub fn default_layout() -> Vec<DashRow> {
    vec![
        row(9, &[("links", 3.0 / 12.0), ("planning", 2.0 / 12.0), ("timeline", 2.0 / 12.0), ("mails", 3.0 / 12.0), ("pinned", 2.0 / 12.0)]),
        row(10, &[("reminders", 3.0 / 12.0), ("tasks", 9.0 / 12.0)]),
        row(8, &[("files", 1.0)]),
    ]
}

/// Parts ramenées à une somme de 1 (parts nulles ou invalides : égales).
fn rebalance(r: &mut DashRow) {
    for p in r.panels.iter_mut() {
        if !p.share.is_finite() || p.share <= 0.0 {
            p.share = 0.0;
        }
    }
    let total: f32 = r.panels.iter().map(|p| p.share).sum();
    let n = r.panels.len() as f32;
    for p in r.panels.iter_mut() {
        p.share = if total > 1e-4 { p.share / total } else { 1.0 / n };
    }
}

/// Disposition enregistrée remise en ordre : panneaux inconnus ou en double retirés, panneaux
/// manquants ajoutés dans une rangée en bas, rangées vides retirées, parts ramenées à 100 %,
/// hauteurs au moins égales aux minimums. Renvoie `true` si quelque chose a changé.
#[cfg(test)]
fn normalize(rows: &mut Vec<DashRow>) -> bool {
    normalize_with(rows, true)
}

/// `dashboard` : disposition du tableau de bord (tous ses panneaux, une fois chacun) ; sinon,
/// onglet personnalisé (panneaux libres, éventuellement aucun).
fn normalize_with(rows: &mut Vec<DashRow>, dashboard: bool) -> bool {
    let before = rows.clone();
    if rows.is_empty() && dashboard {
        *rows = default_layout();
    }
    let mut seen = std::collections::HashSet::new();
    for r in rows.iter_mut() {
        r.panels.retain(|p| def(&p.id).is_some_and(|d| !dashboard || (d.dashboard && kind(&p.id) == p.id)) && seen.insert(p.id.clone()));
    }
    let missing: Vec<&str> = if dashboard { PANELS.iter().filter(|p| p.dashboard).map(|p| p.id).filter(|id| !seen.contains(*id)).collect() } else { Vec::new() };
    if !missing.is_empty() {
        let share = 1.0 / missing.len() as f32;
        rows.push(DashRow { h: 8, panels: missing.iter().map(|id| DashPanel { id: id.to_string(), share }).collect() });
    }
    rows.retain(|r| !r.panels.is_empty());
    for r in rows.iter_mut() {
        rebalance(r);
        let min_h = r.panels.iter().filter_map(|p| def(&p.id)).map(|d| d.min_h).max().unwrap_or(1);
        r.h = r.h.clamp(min_h, 40);
    }
    *rows != before
}

/// Retire un panneau ; les autres panneaux de sa rangée s'élargissent (mêmes proportions), une
/// rangée vide disparaît. Renvoie la hauteur de sa rangée.
fn take(rows: &mut Vec<DashRow>, id: &str) -> Option<u8> {
    let (ri, pi) = rows.iter().enumerate().find_map(|(ri, r)| r.panels.iter().position(|p| p.id == id).map(|pi| (ri, pi)))?;
    let h = rows[ri].h;
    rows[ri].panels.remove(pi);
    if rows[ri].panels.is_empty() {
        rows.remove(ri);
    } else {
        rebalance(&mut rows[ri]);
    }
    Some(h)
}

/// Où déposer un panneau.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Drop {
    /// Dans la rangée `row`, à la place `at`.
    Into { row: usize, at: usize },
    /// Nouvelle rangée, insérée à l'indice `row`.
    NewRow { row: usize },
}

/// Ajoute un panneau : dans une rangée, il prend une part égale (1 / nombre de panneaux) et les
/// autres rétrécissent en proportion ; dans une nouvelle rangée, il prend toute la largeur.
fn insert(rows: &mut Vec<DashRow>, id: &str, h: u8, at: Drop) {
    match at {
        Drop::Into { row, at } if row < rows.len() => {
            let r = &mut rows[row];
            let n = r.panels.len() as f32;
            for p in r.panels.iter_mut() {
                p.share *= n / (n + 1.0);
            }
            r.panels.insert(at.min(r.panels.len()), DashPanel { id: id.to_string(), share: 1.0 / (n + 1.0) });
            rebalance(r);
            r.h = r.h.max(def(id).map_or(1, |d| d.min_h));
        }
        Drop::Into { .. } => rows.push(DashRow { h, panels: vec![DashPanel { id: id.to_string(), share: 1.0 }] }),
        Drop::NewRow { row } => rows.insert(row.min(rows.len()), DashRow { h, panels: vec![DashPanel { id: id.to_string(), share: 1.0 }] }),
    }
}

/// Place visée par le pointeur `p`, d'après les rangées affichées (`bands` : haut et bas de
/// chaque rangée ; `rects` : panneaux de chaque rangée).
fn drop_target(bands: &[(f32, f32)], rects: &[Vec<Rect>], p: Pos2) -> Drop {
    let Some(last) = bands.last() else { return Drop::NewRow { row: 0 } };
    if p.y > last.1 + GAP / 2.0 {
        return Drop::NewRow { row: bands.len() };
    }
    // Rangée sous le pointeur (dans l'espace entre deux rangées : celle du dessous).
    let ri = bands.iter().position(|(_, bottom)| p.y <= bottom + GAP / 2.0).unwrap_or(bands.len() - 1);
    if p.y < bands[ri].0 + NEW_ROW_BAND.min((bands[ri].1 - bands[ri].0) * 0.25) {
        return Drop::NewRow { row: ri };
    }
    let at = rects[ri].iter().position(|r| r.center().x > p.x).unwrap_or(rects[ri].len());
    Drop::Into { row: ri, at }
}

/// Rectangles de chaque panneau (par rangée) et haut / bas de chaque rangée.
fn layout(rows: &[DashRow], origin: Pos2, width: f32, stacked: bool) -> (Vec<Vec<Rect>>, Vec<(f32, f32)>) {
    let px_h = |h: u8| h as f32 * UNIT + (h.max(1) - 1) as f32 * GAP;
    let mut y = origin.y;
    let mut rects = Vec::new();
    let mut bands = Vec::new();
    for r in rows {
        let h = px_h(r.h);
        if stacked {
            // Écran étroit : un panneau par ligne, toute la largeur.
            let mut list = Vec::new();
            let top = y;
            for _ in &r.panels {
                list.push(Rect::from_min_size(Pos2::new(origin.x, y), vec2(width, h)));
                y += h + GAP;
            }
            bands.push((top, y - GAP));
            rects.push(list);
            continue;
        }
        let usable = width - GAP * (r.panels.len() as f32 - 1.0);
        let mut x = origin.x;
        let list: Vec<Rect> = r
            .panels
            .iter()
            .map(|p| {
                let w = (p.share * usable).max(1.0);
                let rect = Rect::from_min_size(Pos2::new(x, y), vec2(w, h));
                x += w + GAP;
                rect
            })
            .collect();
        bands.push((y, y + h));
        rects.push(list);
        y += h + GAP;
    }
    (rects, bands)
}

/// Nouvelles parts d'une rangée quand le panneau `pi` veut la part `want` : les autres gardent
/// leurs proportions de départ (`start`) et ne passent pas sous leur part minimale (`mins`).
fn reshare(start: &[f32], mins: &[f32], pi: usize, want: f32) -> Vec<f32> {
    let others: f32 = start.iter().enumerate().filter(|(i, _)| *i != pi).map(|(_, s)| s).sum();
    let others_min: f32 = mins.iter().enumerate().filter(|(i, _)| *i != pi).map(|(_, m)| m).sum();
    let lo = mins[pi].min(1.0);
    let hi = (1.0 - others_min).max(lo);
    let mut s = want.clamp(lo, hi);
    // Aimant : parts « rondes ».
    if let Some(n) = NICE.iter().find(|n| (s - **n).abs() < NICE_REACH) {
        s = n.clamp(lo, hi);
    }
    let rest = (1.0 - s).max(0.0);
    // Les autres se partagent le reste en proportion, sans passer sous leur minimum.
    let mut out: Vec<f32> = start.iter().map(|x| if others > 1e-4 { x / others * rest } else { 0.0 }).collect();
    out[pi] = s;
    for _ in 0..4 {
        let short: f32 = (0..out.len()).filter(|&i| i != pi && out[i] < mins[i]).map(|i| mins[i] - out[i]).sum();
        if short <= 1e-5 {
            break;
        }
        let free: f32 = (0..out.len()).filter(|&i| i != pi && out[i] > mins[i]).map(|i| out[i] - mins[i]).sum();
        for i in 0..out.len() {
            if i == pi {
                continue;
            }
            if out[i] < mins[i] {
                out[i] = mins[i];
            } else if free > 1e-5 {
                out[i] -= (out[i] - mins[i]) / free * short;
            }
        }
    }
    let total: f32 = out.iter().sum();
    out.iter().map(|x| x / total).collect()
}

/// Position animée d'un panneau (ressort : léger dépassement, puis il se pose).
#[derive(Clone, Copy, Default)]
struct Spring {
    pos: Vec2,
    vel: Vec2,
    size: Vec2,
    vsize: Vec2,
}

impl Spring {
    /// Avance d'un pas vers `target` ; renvoie `true` s'il bouge encore.
    fn step(&mut self, target: Rect, dt: f32) -> bool {
        // Raideur et amortissement : ζ ≈ 0,7 (un petit rebond, « aimanté »).
        const K: f32 = 260.0;
        const C: f32 = 22.0;
        let (tp, ts) = (target.min.to_vec2(), target.size());
        self.vel += (K * (tp - self.pos) - C * self.vel) * dt;
        self.pos += self.vel * dt;
        self.vsize += (K * (ts - self.size) - C * self.vsize) * dt;
        self.size += self.vsize * dt;
        let still = (tp - self.pos).length() < 0.3 && self.vel.length() < 0.3 && (ts - self.size).length() < 0.3 && self.vsize.length() < 0.3;
        if still {
            *self = Spring { pos: tp, size: ts, ..Default::default() };
        }
        !still
    }

    fn rect(&self) -> Rect {
        Rect::from_min_size(self.pos.to_pos2(), self.size)
    }
}

#[derive(Clone)]
struct Drag {
    id: String,
    /// Pointeur − coin du panneau au moment de l'attraper.
    grab: Vec2,
    /// Disposition sans le panneau déplacé (base des essais de place) et hauteur de sa rangée.
    base: Vec<DashRow>,
    h: u8,
}

#[derive(Clone)]
struct Resize {
    id: String,
    /// Taille (px), pointeur et parts de la rangée au début.
    size: Vec2,
    from: Pos2,
    shares: Vec<f32>,
}

#[derive(Clone, Default)]
struct Board {
    springs: HashMap<String, Spring>,
    drag: Option<Drag>,
    resize: Option<Resize>,
    /// Origine à l'image précédente (défilement : les ressorts suivent sans animer).
    origin: Option<Pos2>,
}

/// Dessine les panneaux des rangées `rows` ; `body(id, ui)` dessine le contenu d'un panneau dans
/// `ui` (à sa taille). Renvoie `true` si la disposition a changé.
pub fn show(ui: &mut egui::Ui, rows: &mut Vec<DashRow>, body: impl FnMut(&str, &mut egui::Ui)) -> bool {
    show_inner(ui, rows, None, body).0
}

/// Onglet personnalisé `key` : panneaux libres, chacun avec sa pastille « × » (au survol).
/// Renvoie si la disposition a changé et le panneau à retirer.
pub fn show_free(ui: &mut egui::Ui, key: u64, rows: &mut Vec<DashRow>, body: impl FnMut(&str, &mut egui::Ui)) -> (bool, Option<String>) {
    show_inner(ui, rows, Some(key), body)
}

fn show_inner(ui: &mut egui::Ui, rows: &mut Vec<DashRow>, free: Option<u64>, mut body: impl FnMut(&str, &mut egui::Ui)) -> (bool, Option<String>) {
    let ctx = ui.ctx().clone();
    let mut changed = normalize_with(rows, free.is_none());
    let mut removed = None;
    let width = ui.available_width() - 12.0;
    let stacked = width < STACK_BELOW;
    let origin = ui.cursor().min;

    let board_id = egui::Id::unique(("wt_dash_board", free));
    let mut board: Board = ctx.data(|d| d.get_temp(board_id)).unwrap_or_default();
    // Défilement : tout se décale d'un bloc, sans animation.
    if let Some(prev) = board.origin
        && prev != origin
    {
        let d = origin - prev;
        for s in board.springs.values_mut() {
            s.pos += d;
        }
    }
    board.origin = Some(origin);
    let pointer = ctx.pointer_latest_pos();
    let down = ctx.input(|i| i.pointer.primary_down());

    // Déplacement : la place visée est calculée sur la disposition sans le panneau déplacé.
    let mut drop_line: Option<f32> = None;
    if let Some(d) = board.drag.clone() {
        if !down {
            // Lâché : il part de là où il est (le ressort le pose à sa place).
            board.drag = None;
        } else if let Some(p) = pointer {
            let (b_rects, b_bands) = layout(&d.base, origin, width, false);
            let target = drop_target(&b_bands, &b_rects, p);
            let mut next = d.base.clone();
            insert(&mut next, &d.id, d.h, target);
            normalize_with(&mut next, free.is_none());
            if let Drop::NewRow { row } = target {
                drop_line = Some(if row < b_bands.len() { b_bands[row].0 - GAP / 2.0 } else { b_bands.last().map_or(origin.y, |b| b.1 + GAP / 2.0) });
            }
            if next != *rows {
                *rows = next;
                changed = true;
            }
        }
    }
    let (mut rects, mut bands) = layout(rows, origin, width, stacked);
    // Redimensionnement : part de la largeur (les voisins en proportion) et hauteur de la rangée.
    if let Some(rz) = board.resize.clone() {
        if !down {
            board.resize = None;
        } else if let Some(p) = pointer
            && let Some((ri, pi)) = rows.iter().enumerate().find_map(|(ri, r)| r.panels.iter().position(|x| x.id == rz.id).map(|pi| (ri, pi)))
        {
            let want = rz.size + (p - rz.from);
            let r = &mut rows[ri];
            let n = r.panels.len();
            if n > 1 && rz.shares.len() == n {
                let usable = width - GAP * (n as f32 - 1.0);
                let mins: Vec<f32> = r.panels.iter().map(|x| def(&x.id).map_or(0.1, |d| d.min_px / usable)).collect();
                let shares = reshare(&rz.shares, &mins, pi, want.x / usable);
                for (x, s) in r.panels.iter_mut().zip(shares) {
                    x.share = s;
                }
            }
            let min_h = r.panels.iter().filter_map(|x| def(&x.id)).map(|d| d.min_h).max().unwrap_or(1);
            r.h = (((want.y + GAP) / (UNIT + GAP)).round() as i32).clamp(min_h as i32, 40) as u8;
            (rects, bands) = layout(rows, origin, width, stacked);
            changed = true;
        }
    }
    let bottom = bands.last().map_or(origin.y, |b| b.1);
    // Pendant un déplacement : de la place en dessous pour créer une rangée à la fin.
    let extra = if board.drag.is_some() { 140.0 } else { 0.0 };
    ui.allocate_rect(Rect::from_min_max(origin, Pos2::new(origin.x + width, bottom + extra)), Sense::hover());

    // Ressorts vers les places (le panneau déplacé suit la souris, attiré par sa place).
    let dt = ctx.input(|i| i.stable_dt).clamp(1.0 / 240.0, 1.0 / 30.0);
    let mut moving = false;
    let placed: Vec<(String, Rect, usize)> =
        rows.iter().enumerate().flat_map(|(ri, r)| r.panels.iter().enumerate().map(move |(pi, p)| (p.id.clone(), ri, pi))).map(|(id, ri, pi)| (id, rects[ri][pi], ri)).collect();
    for (id, target, _) in &placed {
        let sp = board.springs.entry(id.clone()).or_insert(Spring { pos: target.min.to_vec2(), size: target.size(), ..Default::default() });
        match (&board.drag, pointer) {
            (Some(d), Some(p)) if &d.id == id => {
                let free = p - d.grab;
                let dist = (free - target.min).length();
                let pull = if dist < MAGNET { (1.0 - dist / MAGNET).powi(2) * 0.85 } else { 0.0 };
                sp.pos = free.to_vec2() + (target.min.to_vec2() - free.to_vec2()) * pull;
                sp.vel = Vec2::ZERO;
                sp.size = target.size();
                moving = true;
            }
            _ => moving |= sp.step(*target, dt),
        }
    }
    board.springs.retain(|id, _| placed.iter().any(|(p, ..)| p == id));
    if moving || board.drag.is_some() || board.resize.is_some() {
        ctx.request_repaint();
    }

    // Rangée en cours de redimensionnement : parts affichées en %.
    let resizing_row = board.resize.as_ref().and_then(|z| placed.iter().find(|(id, ..)| *id == z.id).map(|(_, _, ri)| *ri));
    let dragged = board.drag.as_ref().map(|d| d.id.clone());
    for (id, target, ri) in &placed {
        let r = board.springs[id].rect();
        if dragged.as_deref() == Some(id.as_str()) {
            // Place visée : contour pointillé (ou le trait d'une nouvelle rangée).
            if drop_line.is_none() {
                dashed(ui.painter(), *target, theme::TEXT_FAINT());
            }
            continue;
        }
        // Barre de titre : poignée de déplacement (sous le contenu : ses boutons restent cliquables).
        if !stacked {
            let grip = ui.interact(Rect::from_min_size(r.min, vec2(r.width(), GRIP_H)), egui::Id::unique(("wt_dash_grip", id.as_str())), Sense::drag());
            let grip = grip.on_hover_cursor(egui::CursorIcon::Grab);
            if grip.drag_started()
                && let Some(p) = pointer
            {
                let mut base = rows.clone();
                let h = take(&mut base, id).unwrap_or(8);
                board.drag = Some(Drag { id: id.clone(), grab: p - r.min, base, h });
            }
        }
        let off = crate::widgets::wave_offset(&ctx, r) + crate::widgets::shake_offset(&ctx, r.left() * 0.011 + r.top() * 0.007);
        ui.scope_builder(egui::UiBuilder::new().max_rect(r.translate(off)).layout(egui::Layout::top_down(egui::Align::Min)), |ui| {
            // Textes non sélectionnables : glisser sur un titre déplace le panneau.
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_min_size(r.size());
            ui.set_max_size(r.size());
            body(id, ui);
        });
        // Onglet personnalisé : pastille « × » au coin (au survol) pour retirer le panneau.
        if free.is_some() && board.drag.is_none() && board.resize.is_none() && ui.rect_contains_pointer(r) {
            let c = Pos2::new(r.right() - 18.0, r.top() + 18.0);
            let hit = Rect::from_center_size(c, vec2(22.0, 22.0));
            let x = ui.interact(hit, egui::Id::unique(("wt_dash_remove", id.as_str())), Sense::click_and_drag()).on_hover_text(t!("Retirer ce panneau", "Remove this panel"));
            let p = ui.painter();
            p.circle_filled(c, 10.0, if x.hovered() { crate::tabs::timeline::ACCENT } else { theme::SURFACE() });
            p.circle_stroke(c, 10.0, Stroke::new(1.0, if x.hovered() { crate::tabs::timeline::ACCENT } else { theme::BORDER() }));
            let ink = if x.hovered() { Color32::WHITE } else { theme::TEXT_MUTED() };
            for d in [vec2(3.5, 3.5), vec2(3.5, -3.5)] {
                p.line_segment([c - d, c + d], Stroke::new(1.5, ink));
            }
            if x.clicked() {
                removed = Some(id.clone());
            }
        }
        if stacked {
            continue;
        }
        // Coin de redimensionnement (par-dessus le contenu).
        let corner = Rect::from_min_size(r.max - vec2(CORNER, CORNER), vec2(CORNER, CORNER));
        let cr = ui.interact(corner, egui::Id::unique(("wt_dash_resize", id.as_str())), Sense::drag()).on_hover_cursor(egui::CursorIcon::ResizeNwSe);
        let active = board.resize.as_ref().is_some_and(|z| &z.id == id);
        if cr.hovered() || active || ui.rect_contains_pointer(r) {
            let ink = if cr.hovered() || active { theme::TEXT() } else { theme::TEXT_FAINT() };
            grip_lines(ui.painter(), corner, ink);
        }
        if cr.drag_started()
            && let Some(p) = pointer
            && let Some(rr) = rows.iter().find(|x| x.panels.iter().any(|q| &q.id == id))
        {
            board.resize = Some(Resize { id: id.clone(), size: r.size(), from: p, shares: rr.panels.iter().map(|q| q.share).collect() });
        }
        if resizing_row == Some(*ri)
            && let Some(share) = rows[*ri].panels.iter().find(|q| &q.id == id).map(|q| q.share)
        {
            percent_chip(ui.painter(), r, share, active);
        }
    }
    if let Some(y) = drop_line {
        // Nouvelle rangée : trait à l'endroit où elle sera créée.
        let p = ui.painter();
        p.hline(origin.x..=origin.x + width, y, Stroke::new(3.0, crate::tabs::timeline::ACCENT));
        p.circle_filled(Pos2::new(origin.x, y), 5.0, crate::tabs::timeline::ACCENT);
    }
    if let Some(id) = dragged
        && let Some(sp) = board.springs.get(&id)
    {
        let r = sp.rect();
        egui::Area::new(egui::Id::unique("wt_dash_floating")).order(egui::Order::Foreground).fixed_pos(r.min).interactable(false).show(&ctx, |ui| {
            let shadow = egui::Shadow { offset: [0, 14], blur: 36, spread: 0, color: Color32::from_black_alpha(150) };
            ui.painter().add(shadow.as_shape(r, CornerRadius::same(12)));
            ui.style_mut().interaction.selectable_labels = false;
            ui.set_min_size(r.size());
            ui.set_max_size(r.size());
            body(&id, ui);
        });
        ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    }
    ctx.data_mut(|d| d.insert_temp(board_id, board));
    if let Some(id) = &removed {
        take(rows, id);
        changed = true;
    }
    (changed, removed)
}

/// Part d'un panneau (en %), au centre de son bas, pendant le redimensionnement de sa rangée.
fn percent_chip(p: &egui::Painter, r: Rect, share: f32, active: bool) {
    let text = format!("{} %", (share * 100.0).round());
    let g = p.layout_no_wrap(text, FontId::proportional(12.0), Color32::WHITE);
    let chip = Rect::from_center_size(Pos2::new(r.center().x, r.bottom() - 24.0), g.size() + vec2(16.0, 8.0));
    let fill = if active { crate::tabs::timeline::ACCENT } else { Color32::from_black_alpha(190) };
    p.rect_filled(chip, CornerRadius::same(255), fill);
    p.galley(chip.center() - g.size() / 2.0, g, Color32::WHITE);
}

/// Contour pointillé arrondi (place visée).
pub(crate) fn dashed(p: &egui::Painter, r: Rect, color: Color32) {
    let r = r.shrink(1.0);
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.rect_filled(r, CornerRadius::same(12), theme::overlay(5));
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(1.2, color), 6.0, 5.0));
}

/// Trois traits en diagonale (coin de redimensionnement).
fn grip_lines(p: &egui::Painter, r: Rect, color: Color32) {
    let c = r.right_bottom() - vec2(5.0, 5.0);
    for k in [4.0, 8.0, 12.0] {
        p.line_segment([c - vec2(k, 0.0), c - vec2(0.0, k)], Stroke::new(1.2, color));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total(r: &DashRow) -> f32 {
        r.panels.iter().map(|p| p.share).sum()
    }

    #[test]
    fn rows_always_fill_the_width() {
        let mut rows = default_layout();
        assert!(!normalize(&mut rows));
        for r in &rows {
            assert!((total(r) - 1.0).abs() < 1e-4);
        }
        // Retirer un panneau : les autres de sa rangée s'élargissent.
        let h = take(&mut rows, "pinned").unwrap();
        assert_eq!(h, 9);
        assert_eq!(rows[0].panels.len(), 4);
        assert!((total(&rows[0]) - 1.0).abs() < 1e-4);
        // L'ajouter dans la 2e rangée : part égale, les autres rétrécissent.
        insert(&mut rows, "pinned", h, Drop::Into { row: 1, at: 1 });
        assert_eq!(rows[1].panels.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(), ["reminders", "pinned", "tasks"]);
        assert!((rows[1].panels[1].share - 1.0 / 3.0).abs() < 1e-4);
        assert!((total(&rows[1]) - 1.0).abs() < 1e-4);
        // Nouvelle rangée : toute la largeur.
        take(&mut rows, "files");
        insert(&mut rows, "files", 8, Drop::NewRow { row: 0 });
        assert_eq!(rows[0].panels[0].id, "files");
        assert_eq!(rows[0].panels[0].share, 1.0);
        // Une rangée vidée disparaît.
        take(&mut rows, "files");
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn resizing_keeps_100_percent() {
        let mins = [0.1, 0.1, 0.1];
        // Le premier passe de 50 % à 70 % : les deux autres (25 % / 25 %) se partagent 30 %.
        let s = reshare(&[0.5, 0.25, 0.25], &mins, 0, 0.7);
        assert!((s.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        assert!((s[0] - 0.7).abs() < 1e-4 && (s[1] - 0.15).abs() < 1e-4);
        // Aimant : 0,49 → ½.
        assert!((reshare(&[0.5, 0.25, 0.25], &mins, 0, 0.49)[0] - 0.5).abs() < 1e-4);
        // Les autres ne passent pas sous leur minimum.
        let s = reshare(&[0.4, 0.3, 0.3], &mins, 0, 0.95);
        assert!(s[1] >= 0.1 - 1e-4 && s[2] >= 0.1 - 1e-4);
        assert!((s.iter().sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn free_layouts() {
        // Onglet personnalisé : vide reste vide, plusieurs instances d'un même type.
        let mut rows: Vec<DashRow> = Vec::new();
        assert!(!normalize_with(&mut rows, false));
        for id in ["note#1", "tasks#2", "note#3", "files#4"] {
            append(&mut rows, id, 1600.0);
        }
        assert!(!normalize_with(&mut rows, false));
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].panels.len(), 3);
        assert_eq!(rows[0].h, 8);
        take(&mut rows, "files#4");
        assert_eq!(rows.len(), 1);
        // Le tableau de bord ignore ces panneaux.
        let mut dash = rows.clone();
        normalize(&mut dash);
        assert!(dash.iter().all(|r| r.panels.iter().all(|p| !p.id.contains('#'))));
    }

    #[test]
    fn drop_targets() {
        let r = |x: f32, y: f32| Rect::from_min_size(Pos2::new(x, y), vec2(100.0, 200.0));
        let rects = vec![vec![r(0.0, 0.0), r(110.0, 0.0)], vec![r(0.0, 212.0)]];
        let bands = vec![(0.0, 200.0), (212.0, 412.0)];
        assert_eq!(drop_target(&bands, &rects, Pos2::new(10.0, 100.0)), Drop::Into { row: 0, at: 0 });
        assert_eq!(drop_target(&bands, &rects, Pos2::new(400.0, 100.0)), Drop::Into { row: 0, at: 2 });
        // Haut d'une rangée : nouvelle rangée au-dessus ; sous la dernière : à la fin.
        assert_eq!(drop_target(&bands, &rects, Pos2::new(10.0, 220.0)), Drop::NewRow { row: 1 });
        assert_eq!(drop_target(&bands, &rects, Pos2::new(10.0, 600.0)), Drop::NewRow { row: 2 });
        assert_eq!(drop_target(&bands, &rects, Pos2::new(10.0, 300.0)), Drop::Into { row: 1, at: 0 });
    }

    #[test]
    fn layout_is_repaired() {
        let mut rows = vec![row(1, &[("tasks", 3.0), ("inconnu", 1.0), ("tasks", 1.0)])];
        assert!(normalize(&mut rows));
        assert_eq!(rows[0].panels.len(), 1);
        assert_eq!(rows[0].panels[0].share, 1.0);
        assert_eq!(rows[0].h, 6);
        // Les panneaux manquants arrivent dans une rangée en bas.
        assert_eq!(rows.iter().map(|r| r.panels.len()).sum::<usize>(), PANELS.iter().filter(|p| p.dashboard).count());
        assert!(!normalize(&mut rows));
    }
}
