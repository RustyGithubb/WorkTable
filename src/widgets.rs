//! Composants visuels partagés entre les onglets.

use eframe::egui::{self, CornerRadius, Margin, RichText, Stroke};

use crate::theme;

/// En-tête d'onglet, identique partout : sur-titre discret en capitales + grand titre (police
/// « Titres »), suivis d'éléments optionnels sur la même ligne (pastille, numéro de semaine…).
pub fn page_header(ui: &mut egui::Ui, overline: &str, title: &str) {
    page_header_with(ui, overline, title, |_| {});
}

pub fn page_header_with(ui: &mut egui::Ui, overline: &str, title: &str, extra: impl FnOnce(&mut egui::Ui)) {
    ui.label(RichText::new(overline.to_uppercase()).small().color(theme::TEXT_MUTED()).strong());
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.set_min_height(HEADER_TITLE_H);
        ui.label(RichText::new(title).font(theme::title_font(32.0)).color(theme::TEXT()));
        ui.add_space(4.0);
        extra(ui);
    });
    ui.add_space(18.0);
}

/// Information à côté du titre (non lus en terracotta, numéro de semaine en gris).
pub fn header_badge(ui: &mut egui::Ui, text: &str, accent: bool) {
    ui.label(RichText::new(text).size(14.0).color(if accent { theme::TODAY() } else { theme::TEXT_MUTED() }));
}

/// Hauteur de la ligne de titre (alignement identique d'un onglet à l'autre).
const HEADER_TITLE_H: f32 = 40.0;
/// Marges du contenu des onglets (voir `app.rs`) : haut et droite.
const CONTENT_TOP: f32 = 36.0;
const CONTENT_RIGHT: f32 = 32.0;

/// Heure en haut à droite, sur la ligne du titre de l'onglet et dans la même police, visible
/// dans tous les onglets. Un clic la fait disparaître (ressort élastique) ; un clic au même
/// endroit la fait revenir. Masquée, elle réapparaît en transparence quand la souris approche,
/// pour montrer où cliquer. Renvoie `true` si on a cliqué (bascule à faire).
pub fn clock(ctx: &egui::Context, window: egui::Rect, hidden: bool) -> bool {
    use std::sync::Mutex;
    /// Ressort (taille, vitesse), état précédent, fantôme affiché, souris sortie depuis le clic.
    struct State {
        scale: f32,
        vel: f32,
        hidden: bool,
        ghost: f32,
        armed: bool,
    }
    static STATE: Mutex<Option<State>> = Mutex::new(None);
    let now = chrono::Local::now();
    let text = now.format("%H:%M").to_string();
    // Même hauteur que le titre : sous le sur-titre (petit texte) puis centré dans sa ligne.
    let over = ctx.fonts_mut(|f| f.row_height(&egui::FontId::proportional(11.0)));
    let gap = ctx.global_style().spacing.item_spacing.y;
    let center_y = window.top() + CONTENT_TOP + over + gap + 2.0 + HEADER_TITLE_H / 2.0;
    let full = ctx.fonts_mut(|f| f.layout_no_wrap(text.clone(), theme::title_font(32.0), theme::TEXT())).size();
    let zone = egui::Rect::from_min_size(egui::pos2(window.right() - CONTENT_RIGHT - full.x, center_y - full.y / 2.0), full);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("wt_clock_zone"), zone));

    // Ressort : 1 = affichée, 0 = masquée, avec dépassement (élastique).
    let dt = ctx.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let mut guard = STATE.lock().unwrap();
    let st = guard.get_or_insert(State { scale: if hidden { 0.0 } else { 1.0 }, vel: 0.0, hidden, ghost: 0.0, armed: true });
    if st.hidden != hidden {
        // Petite impulsion au départ : elle gonfle un peu avant de se rétracter.
        if hidden {
            st.vel += 3.5;
        }
        st.hidden = hidden;
        // Après un clic, la souris doit d'abord quitter la zone pour que l'heure réagisse.
        st.armed = false;
        st.ghost = 0.0;
    }
    if !st.armed && pointer.is_none_or(|p| !zone.expand(6.0).contains(p)) {
        st.armed = true;
    }
    let target = if hidden { 0.0 } else { 1.0 };
    st.vel += (220.0 * (target - st.scale) - 12.0 * st.vel) * dt;
    st.scale += st.vel * dt;
    let mut moving = (st.scale - target).abs() > 0.002 || st.vel.abs() > 0.01;
    if !moving {
        st.scale = target;
        st.vel = 0.0;
    }
    // Fantôme quand elle est masquée : apparaît en douceur à l'approche de la souris (courbe
    // progressive selon la distance, puis lissage dans le temps).
    let goal = match pointer {
        Some(p) if hidden && st.armed => {
            let k = (1.0 - zone.distance_to_pos(p) / 160.0).clamp(0.0, 1.0);
            k * k * (3.0 - 2.0 * k) * k * 0.45
        }
        _ => 0.0,
    };
    st.ghost += (goal - st.ghost) * (1.0 - (-5.0 * dt).exp());
    if (goal - st.ghost).abs() > 0.003 {
        moving = true;
    } else {
        st.ghost = goal;
    }
    let (s, ghost, armed) = (st.scale, st.ghost, st.armed);
    drop(guard);

    let mut clicked = false;
    egui::Area::new(egui::Id::unique("wt_clock")).order(egui::Order::Middle).fixed_pos(zone.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(full, if armed { egui::Sense::click() } else { egui::Sense::hover() });
        let tip = if hidden { t!("Afficher l'heure", "Show the clock") } else { t!("Masquer l'heure", "Hide the clock") };
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        let base = if resp.hovered() && !hidden { theme::TEXT_MUTED() } else { theme::TEXT() };
        let (size, alpha) = if s > 0.02 { (32.0 * s.max(0.0), s.clamp(0.0, 1.0)) } else { (32.0, ghost) };
        if alpha > 0.0 && size > 1.0 {
            let g = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), theme::title_font(size), base.gamma_multiply(alpha)));
            // Ancrée à droite, centrée verticalement sur la ligne du titre.
            let pos = egui::pos2(r.right() - g.size().x, r.center().y - g.size().y / 2.0);
            ui.painter().with_clip_rect(ctx.content_rect()).galley(pos, g, base);
        }
        clicked = resp.clicked();
    });
    if moving {
        ctx.request_repaint();
    }
    // Mise à jour à la minute suivante.
    let secs = 60 - chrono::Timelike::second(&now) as u64;
    ctx.request_repaint_after(std::time::Duration::from_secs(secs.max(1)));
    clicked
}

/// Bouton « Ne pas déranger » : pilule à gauche de l'heure, dont la pastille (tête de raton
/// laveur, qui se masque) glisse de gauche (notifications actives) à droite
/// (ne pas déranger) avec un rebond élastique. Renvoie le centre du bouton s'il a été cliqué
/// (origine de la vague de retour visuel).
pub fn do_not_disturb(ctx: &egui::Context, window: egui::Rect, on: bool) -> Option<egui::Pos2> {
    const W: f32 = 58.0;
    const H: f32 = 30.0;
    // À gauche de la zone de l'heure (même ligne), qu'elle soit affichée ou masquée.
    let clock = ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::unique("wt_clock_zone")));
    let (right, cy) = match clock {
        Some(z) => (z.left() - 18.0, z.center().y),
        None => (window.right() - CONTENT_RIGHT - 110.0, window.top() + CONTENT_TOP + 40.0),
    };
    let rect = egui::Rect::from_min_size(egui::pos2(right - W, cy - H / 2.0), egui::vec2(W, H));
    // Pastille liquide (0 = gauche, 1 = droite), en ease-in-out : le bord avant part le premier,
    // le bord arrière suit avec un léger retard, si bien que la pastille s'étire en chemin puis
    // se reforme en goutte à l'arrivée.
    const DURATION: f64 = 0.55;
    let id = egui::Id::unique("wt_dnd_knob");
    let now = ctx.input(|i| i.time);
    let target = if on { 1.0 } else { 0.0 };
    // (départ, arrivée, début de l'animation)
    // Premier affichage : la pastille est mémorisée à sa place, au repos. Sans cela, l'état
    // n'existerait pas encore au premier clic et serait recréé directement à l'arrivée (pas
    // d'animation).
    let (from, to, start) = ctx.data_mut(|d| *d.get_temp_mut_or_insert_with(id, || (target, target, f64::NEG_INFINITY)));
    let mut t = ((now - start) / DURATION) as f32;
    let (mut from, mut to) = (from, to);
    if to != target {
        // Nouveau clic (même en pleine course) : on repart de la position actuelle.
        let (lead, trail) = liquid_edges(from, to, t);
        (from, to, t) = ((lead + trail) / 2.0, target, 0.0);
        ctx.data_mut(|d| d.insert_temp(id, (from, to, now)));
    }
    let (lead, trail) = liquid_edges(from, to, t);
    let moving = t < 1.0;
    // Position moyenne : sert à l'icône et au fondu des couleurs.
    let x = (lead + trail) / 2.0;

    let mut clicked = None;
    egui::Area::new(egui::Id::unique("wt_dnd")).order(egui::Order::Middle).fixed_pos(rect.min).show(ctx, |ui| {
        let (r, resp) = ui.allocate_exact_size(rect.size(), egui::Sense::click());
        let tip = if on {
            t!("Ne pas déranger : activé (clic : réactiver les notifications)", "Do not disturb: on (click: turn notifications back on)")
        } else {
            t!("Notifications actives (clic : ne pas déranger)", "Notifications on (click: do not disturb)")
        };
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip);
        let p = ui.painter();
        let k = x.clamp(0.0, 1.0);
        // Fond : discret (actif) → couleur de sélection (ne pas déranger), en fondu.
        let off_bg = if resp.hovered() { theme::SURFACE_HI() } else { theme::SURFACE() };
        let fill = crate::sidebar::lerp_color(off_bg, theme::TEXT(), k);
        p.rect(r, CornerRadius::same(255), fill, Stroke::new(1.0, theme::BORDER().gamma_multiply(1.0 - k)), egui::StrokeKind::Inside);
        let d = H - 6.0;
        let travel = W - 6.0 - d;
        let left = r.left() + 3.0 + d / 2.0;
        let c = egui::pos2(left + x * travel, r.center().y);
        // Goutte étirée entre ses deux bords, un peu amincie quand elle s'allonge.
        let (a, b) = (left + lead.min(trail) * travel, left + lead.max(trail) * travel);
        let stretch = (b - a) / travel;
        let thick = d * (1.0 - 0.12 * stretch);
        let blob = egui::Rect::from_min_max(egui::pos2(a - thick / 2.0, c.y - thick / 2.0), egui::pos2(b + thick / 2.0, c.y + thick / 2.0));
        let knob = crate::sidebar::lerp_color(theme::SURFACE_HI(), theme::BG(), k);
        p.rect_filled(blob.translate(egui::vec2(0.0, 1.0)), CornerRadius::same(255), egui::Color32::from_black_alpha(60));
        p.rect_filled(blob, CornerRadius::same(255), knob);
        let icon = crate::sidebar::lerp_color(theme::TEXT_MUTED(), theme::TEXT(), k);
        raccoon(p, c, d * 0.56, icon, knob, k);
        if resp.clicked() {
            clicked = Some(r.center());
        }
    });
    if moving {
        ctx.request_repaint();
    }
    clicked
}

/// Bords avant et arrière de la pastille liquide à l'instant `t` (0 → 1) d'un trajet de `from`
/// à `to` : chacun suit une courbe ease-in-out, le bord arrière avec un léger retard.
fn liquid_edges(from: f32, to: f32, t: f32) -> (f32, f32) {
    const LAG: f32 = 0.22;
    let ease = |t: f32| crate::prefs::Curve::EaseInOut.apply(t.clamp(0.0, 1.0));
    let lead = from + (to - from) * ease(t / (1.0 - LAG));
    let trail = from + (to - from) * ease((t - LAG) / (1.0 - LAG));
    (lead, trail)
}

/// Icône : tête de raton laveur. Normal : deux taches en larme autour des yeux ouverts ;
/// masqué (`masked` 0 → 1) : les taches se rejoignent en un loup et les yeux se ferment.
/// `bg` : couleur de la pastille (détails en creux).
fn raccoon(p: &egui::Painter, c: egui::Pos2, s: f32, color: egui::Color32, bg: egui::Color32, masked: f32) {
    use egui::{Pos2, Shape, pos2, vec2};
    let k = masked.clamp(0.0, 1.0);
    let r = s * 0.6;
    let h = c + vec2(0.0, s * 0.1);
    let at = |x: f32, y: f32| pos2(h.x + x * r, h.y + y * r);
    // Ellipse inclinée (taches autour des yeux).
    let tilted = |center: Pos2, a: f32, b: f32, angle: f32| -> Vec<Pos2> {
        (0..20)
            .map(|i| {
                let t = i as f32 / 20.0 * std::f32::consts::TAU;
                let (x, y) = (a * t.cos(), b * t.sin());
                pos2(center.x + x * angle.cos() - y * angle.sin(), center.y + x * angle.sin() + y * angle.cos())
            })
            .collect()
    };
    // Oreilles arrondies, intérieur en creux.
    for side in [-1.0f32, 1.0] {
        p.add(Shape::convex_polygon(vec![at(side * 0.95, -0.35), at(side * 0.8, -1.15), at(side * 0.25, -0.8)], color, Stroke::NONE));
        p.circle_filled(at(side * 0.75, -0.95), r * 0.2, color);
        p.add(Shape::convex_polygon(vec![at(side * 0.78, -0.55), at(side * 0.72, -0.92), at(side * 0.45, -0.72)], bg, Stroke::NONE));
    }
    // Tête large, touffes de poils pointues sur les joues, menton.
    let head = vec![at(-0.55, -0.82), at(0.55, -0.82), at(0.98, -0.3), at(1.32, 0.3), at(0.62, 0.72), at(0.0, 0.9), at(-0.62, 0.72), at(-1.32, 0.3), at(-0.98, -0.3)];
    p.add(Shape::convex_polygon(head, color, Stroke::NONE));
    // Taches des yeux (en larme, tombantes vers l'extérieur) ; masqué : réunies en un loup.
    let eye_y = -0.12;
    for side in [-1.0f32, 1.0] {
        let patch = tilted(at(side * 0.5, eye_y + 0.06), r * (0.42 + 0.1 * k), r * 0.26, side * (0.45 * (1.0 - k)));
        p.add(Shape::convex_polygon(patch, bg, Stroke::NONE));
    }
    if k > 0.01 {
        p.rect_filled(egui::Rect::from_center_size(at(0.0, eye_y + 0.04), vec2(r * 1.1 * k, r * 0.4)), CornerRadius::same(255), bg);
    }
    // Yeux : ouverts (ronds), puis fermés (traits) une fois masqué.
    for side in [-1.0f32, 1.0] {
        let e = at(side * 0.48, eye_y);
        if k < 0.5 {
            p.circle_filled(e, r * 0.14 * (1.0 - k), color);
        } else {
            p.line_segment([e - vec2(r * 0.16, 0.0), e + vec2(r * 0.16, 0.0)], Stroke::new((r * 0.1).max(1.0), color));
        }
    }
    // Museau : truffe en creux au bout.
    p.circle_filled(at(0.0, 0.55), r * 0.14, bg);
}

/// Vague de retour visuel (bascule « Ne pas déranger ») : origine et heure de départ.
pub fn start_wave(ctx: &egui::Context, origin: egui::Pos2) {
    let now = ctx.input(|i| i.time);
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("wt_dnd_wave"), (origin, now)));
}

/// Décalage d'un panneau (de rectangle `r`) pendant la vague : il vibre quand le front de
/// l'onde l'atteint (plus tard s'il est loin de l'icône), dans la direction de l'onde.
pub fn wave_offset(ctx: &egui::Context, r: egui::Rect) -> egui::Vec2 {
    const SPEED: f32 = 1800.0; // px/s
    const DURATION: f32 = 0.55;
    let Some((origin, start)) = ctx.data(|d| d.get_temp::<(egui::Pos2, f64)>(egui::Id::unique("wt_dnd_wave"))) else { return egui::Vec2::ZERO };
    let elapsed = (ctx.input(|i| i.time) - start) as f32;
    let reach = r.distance_to_pos(origin) / SPEED;
    let t = elapsed - reach;
    if elapsed > reach + DURATION + 0.1 {
        // Onde terminée pour ce panneau (les plus lointains la reçoivent en dernier).
        return egui::Vec2::ZERO;
    }
    ctx.request_repaint();
    if t <= 0.0 || t >= DURATION {
        return egui::Vec2::ZERO;
    }
    let dir = (r.center() - origin).normalized();
    // Amplitude plus faible pour les panneaux éloignés, oscillation amortie.
    let fade = (1.0 - r.distance_to_pos(origin) / 2400.0).clamp(0.35, 1.0);
    let a = 7.0 * fade * (-t * 7.0).exp() * (t * 38.0).sin();
    dir * a
}

/// Tremblement des panneaux pendant que le logo de la barre tourne : amplitude et fréquence
/// croissent avec la vitesse de la roue ; `seed` décale chaque panneau (ils ne tremblent pas en
/// bloc).
pub fn shake_offset(ctx: &egui::Context, seed: f32) -> egui::Vec2 {
    let k = crate::sidebar::spin_intensity();
    if k < 0.04 {
        return egui::Vec2::ZERO;
    }
    // Plus la roue tourne vite, plus c'est fort (progression plus que proportionnelle, plafonnée).
    let amp = (5.0 * k.powf(1.35)).min(16.0);
    let freq = 26.0 + 14.0 * k.min(3.0);
    let t = ctx.input(|i| i.time) as f32;
    let x = (t * freq + seed * 1.7).sin() + 0.5 * (t * freq * 1.9 + seed * 3.1).sin();
    let y = (t * freq * 1.3 + seed * 2.3).cos() + 0.5 * (t * freq * 2.3 + seed * 0.7).sin();
    ctx.request_repaint();
    egui::vec2(x, y) * amp / 1.5
}

/// Affiche `body` décalé de `offset` (effet visuel seulement), sans surcoût quand il est nul.
pub fn shifted<R>(ui: &mut egui::Ui, offset: egui::Vec2, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    if offset == egui::Vec2::ZERO { body(ui) } else { ui.with_visual_transform(egui::emath::TSTransform::from_translation(offset), body).inner }
}

/// Titre de section (cartes, colonnes) : police « Titres ».
pub fn section_title(text: &str) -> RichText {
    RichText::new(text).font(theme::title_font(15.0)).color(theme::TEXT())
}

/// Carte de contenu avec bordure fine.
pub fn card<R>(ui: &mut egui::Ui, title: &str, min_height: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let jolt = shake_offset(ui.ctx(), ui.cursor().top() * 0.013 + title.len() as f32);
    shifted(ui, jolt, |ui| card_inner(ui, title, min_height, body))
}

fn card_inner<R>(ui: &mut egui::Ui, title: &str, min_height: f32, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::SURFACE())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(20))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(min_height);
            if !title.is_empty() {
                ui.label(section_title(title));
                ui.add_space(10.0);
            }
            body(ui)
        })
        .inner
}

/// Panneau plein (utilisé pour les colonnes Mail / Notes).
pub fn pane<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let jolt = shake_offset(ui.ctx(), ui.max_rect().left() * 0.011);
    shifted(ui, jolt, |ui| pane_inner(ui, body))
}

fn pane_inner<R>(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(theme::SURFACE())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            body(ui)
        })
        .inner
}

/// État vide : message centré, en gris.
pub fn empty_state(ui: &mut egui::Ui, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(RichText::new(title).color(theme::TEXT_MUTED()));
        ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT()));
    });
}

/// Ligne fantôme (placeholder de liste).
pub fn ghost_row(ui: &mut egui::Ui, width_ratio: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 14.0), egui::Sense::hover());
    let bar = egui::Rect::from_min_size(rect.min, egui::vec2(w * width_ratio, 10.0));
    ui.painter().rect_filled(bar, CornerRadius::same(5), theme::SURFACE_HI());
}

/// Découpe l'espace restant en colonnes de largeurs fixes (la dernière prend le reste)
/// et affiche un panneau dans chacune, en disposition verticale.
pub fn pane_columns(ui: &mut egui::Ui, fixed: &[f32], mut body: impl FnMut(usize, &mut egui::Ui)) {
    let full = ui.available_rect_before_wrap();
    let gap = ui.spacing().item_spacing.x;
    let mut x = full.left();
    for i in 0..=fixed.len() {
        let w = fixed.get(i).copied().unwrap_or(full.right() - x);
        let rect = egui::Rect::from_min_size(egui::pos2(x, full.top()), egui::vec2(w, full.height()));
        ui.scope_builder(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)), |ui| pane(ui, |ui| body(i, ui)));
        x += w + gap;
    }
    ui.allocate_rect(full, egui::Sense::hover());
}

/// Logo des rappels minutés : réveil (deux cloches, cadran, aiguilles, pieds).
/// `ring` (0..1) : il sonne (oscille) puis se pose ; 1 = au repos.
pub fn alarm_icon(p: &egui::Painter, c: egui::Pos2, size: f32, color: egui::Color32, ring: f32) {
    let ring = ring.clamp(0.0, 1.0);
    let angle = (ring * 30.0).sin() * 0.32 * (1.0 - ring);
    let (s, co) = angle.sin_cos();
    let at = |x: f32, y: f32| c + egui::vec2(x * co - y * s, x * s + y * co) * size;
    let w = (size * 0.085).max(1.2);
    let stroke = egui::Stroke::new(w, color);
    let center = at(0.0, 0.06);
    p.circle_stroke(center, size * 0.34, stroke);
    // Aiguilles (10 h 10).
    p.line_segment([center, at(0.0, -0.14)], stroke);
    p.line_segment([center, at(0.13, 0.13)], stroke);
    // Cloches.
    for side in [-1.0, 1.0] {
        let pts: Vec<egui::Pos2> = (0..=8)
            .map(|i| {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 8.0);
                at(side * 0.30 + 0.13 * a.cos(), -0.34 + 0.13 * a.sin() + 0.04)
            })
            .collect();
        p.add(egui::Shape::convex_polygon(pts, color, egui::Stroke::NONE));
    }
    // Pieds.
    p.line_segment([at(-0.2, 0.33), at(-0.29, 0.45)], stroke);
    p.line_segment([at(0.2, 0.33), at(0.29, 0.45)], stroke);
}

/// Avancement (0..1, avec rebond élastique) de l'ouverture d'une colonne : même animation que
/// les blocs Groupes / Personnes de Google Chat.
pub fn reveal(ctx: &egui::Context, salt: &str, open: bool) -> f32 {
    ctx.animate_bool_with_time_and_easing(egui::Id::unique(salt), open, 0.28, |t| crate::prefs::Curve::Elastic.apply(t))
}

/// Largeur d'une colonne qui s'anime (0 quand elle est presque fermée).
pub fn reveal_width(full: f32, k: f32) -> f32 {
    if full * k < 24.0 { 0.0 } else { full * k }
}

/// Colonne qui s'anime en largeur : contenu à sa taille normale, rogné, en fondu.
pub fn animated_column(ui: &mut egui::Ui, w: f32, full: f32, k: f32, body: impl FnOnce(&mut egui::Ui)) {
    let inner = ui.available_rect_before_wrap();
    let margins = w - inner.width();
    let content = egui::Rect::from_min_size(inner.min, egui::vec2((full - margins).max(inner.width()), inner.height()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(*ui.layout()));
    child.set_clip_rect(inner.expand(2.0).intersect(ui.clip_rect()));
    child.multiply_opacity(k.clamp(0.0, 1.0));
    body(&mut child);
    ui.allocate_rect(inner, egui::Sense::hover());
}

/// Ligne « libellé + champ texte » occupant toute la largeur disponible.
pub fn field(ui: &mut egui::Ui, label: &str, label_w: f32, edit: egui::TextEdit) -> egui::Response {
    ui.horizontal(|ui| {
        ui.add_sized([label_w, 24.0], egui::Label::new(RichText::new(label).color(theme::TEXT_MUTED())));
        ui.add(edit.desired_width(ui.available_width()))
    })
    .inner
}

/// Sélecteur segmenté : la pastille de sélection glisse (animation liquide) d'une option à l'autre.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, salt: &str, options: &[(T, &str)], value: &mut T) -> bool {
    let mut changed = false;
    egui::Frame::new().fill(theme::BG()).stroke(egui::Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(255)).inner_margin(egui::Margin::same(3)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let mut hl = crate::liquid::Liquid::begin(ui, ("segmented", salt));
                for (i, (opt, label)) in options.iter().enumerate() {
                    let galley = ui.painter().layout_no_wrap(label.to_string(), egui::FontId::proportional(13.0), egui::Color32::PLACEHOLDER);
                    let size = egui::vec2(galley.size().x + 24.0, 26.0);
                    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    if *opt == *value {
                        hl.target(rect, i);
                    }
                    let cov = hl.coverage(rect);
                    let base = if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
                    let color = crate::sidebar::lerp_color(base, theme::BG(), cov);
                    ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
                    if resp.clicked() && *opt != *value {
                        *value = *opt;
                        changed = true;
                    }
                }
                hl.end(ui, theme::TEXT(), 13.0);
            });
        },
    );
    changed
}

#[cfg(test)]
mod tests {
    use super::liquid_edges;

    #[test]
    fn liquid_knob_stretches_then_settles() {
        assert_eq!(liquid_edges(0.0, 1.0, 0.0), (0.0, 0.0), "au départ : une goutte ronde");
        assert_eq!(liquid_edges(0.0, 1.0, 1.0), (1.0, 1.0), "à l'arrivée : une goutte ronde");
        let (lead, trail) = liquid_edges(0.0, 1.0, 0.5);
        assert!(lead > trail + 0.2, "étirée en chemin ({lead} / {trail})");
        // Ease-in-out : départ et arrivée en douceur, plus rapide au milieu.
        let step = |a: f32, b: f32| liquid_edges(0.0, 1.0, b).0 - liquid_edges(0.0, 1.0, a).0;
        assert!(step(0.0, 0.1) < step(0.35, 0.45));
        assert!(step(0.68, 0.78) < step(0.35, 0.45));
        // Retour : même mouvement dans l'autre sens.
        let (lead, trail) = liquid_edges(1.0, 0.0, 0.5);
        assert!(lead < trail);
    }
}
