//! Barre de navigation verticale en forme de pilule, flottante à gauche.

use eframe::egui::{
    self, Color32, CornerRadius, Margin, Order, Painter, Pos2, Rect, Sense, Shadow, Stroke, vec2,
};

use crate::app::Tab;
use crate::liquid::Liquid;
use crate::theme;

const PILL_WIDTH: f32 = 56.0;
const BUTTON: f32 = 40.0;
const SCREEN_MARGIN: f32 = 16.0;

/// Affiche la barre et renvoie l'onglet cliqué, le cas échéant.
pub fn show(ctx: &egui::Context, window: Rect, current: Tab, badges: &[(Tab, usize)]) -> Option<Tab> {
    // Hauteur : logo + séparateur + boutons.
    let n = Tab::ALL.len() as f32;
    let inner_h = BUTTON + 17.0 + n * BUTTON + (n - 1.0) * 8.0;
    let pill_h = inner_h + 16.0;
    // Centrée, mais sans jamais chevaucher le bouton Réglages en bas.
    let max_y = window.bottom() - 2.0 * SCREEN_MARGIN - (BUTTON + 16.0) - pill_h;
    let y = (window.center().y - pill_h / 2.0)
        .min(max_y)
        .max(window.top() + SCREEN_MARGIN);
    let pos = Pos2::new(window.left() + SCREEN_MARGIN, y);

    let mut clicked = None;

    egui::Area::new(egui::Id::unique("worktable_sidebar"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            pill_frame()
                .show(ui, |ui| {
                    ui.set_width(BUTTON);
                    ui.spacing_mut().item_spacing.y = 8.0;

                    // Logo
                    // Logo
                    let (logo, _) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::hover());
                    ui.painter().text(
                        logo.center(),
                        egui::Align2::CENTER_CENTER,
                        "W",
                        egui::FontId::proportional(18.0),
                        theme::TEXT(),
                    );

                    // Séparateur
                    let (sep, _) = ui.allocate_exact_size(vec2(BUTTON, 1.0), Sense::hover());
                    ui.painter().hline(
                        sep.x_range().shrink(8.0),
                        sep.center().y,
                        Stroke::new(1.0, theme::BORDER()),
                    );

                    // La pastille blanche de sélection s'écoule d'un onglet à l'autre.
                    let mut hl = Liquid::begin(ui, "sidebar_nav");
                    let mut dots = Vec::new();
                    for tab in Tab::ALL {
                        let r = nav_button(ui, tab, tab == current, &mut hl);
                        if r.clicked() {
                            clicked = Some(tab);
                        }
                        if let Some((_, n)) = badges.iter().find(|(t, n)| *t == tab && *n > 0) {
                            dots.push((r.rect, *n));
                        }
                    }
                    hl.end(ui, theme::TEXT(), BUTTON / 2.0);
                    // Pastilles « non lus » (dessinées au-dessus de la sélection).
                    for (rect, n) in dots {
                        badge(ui.painter(), rect, n);
                    }
                });
        });

    clicked
}

/// Petite pastille terracotta avec le nombre de non lus, en haut à droite de l'icône.
fn badge(p: &Painter, rect: Rect, n: usize) {
    let text = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = p.layout_no_wrap(text, egui::FontId::proportional(9.5), Color32::WHITE);
    let w = (g.size().x + 8.0).max(15.0);
    let r = Rect::from_center_size(rect.right_top() + vec2(-6.0, 7.0), vec2(w, 15.0));
    p.rect(r, CornerRadius::same(8), theme::TODAY(), Stroke::new(1.5, theme::SIDEBAR_BG()), egui::StrokeKind::Outside);
    p.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
}

fn nav_button(ui: &mut egui::Ui, tab: Tab, active: bool, hl: &mut Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tab.label());
    if active {
        hl.target(rect, tab.label());
    }
    let phase = click_phase(ui, tab.label(), resp.clicked());
    let zoom = magnify(ui, tab.label(), rect);
    paint_button(ui, rect, &resp, hl.coverage(rect), zoom, phase, |p, r, c, a| draw_icon(p, r, tab, c, a));
    resp
}

// --- Effets : animation au clic, loupe façon Dock macOS --------------------------

/// Durée de l'animation de clic d'une icône (50 % plus rapide qu'à l'origine : 0,65 s / 1,5).
const CLICK_ANIM: f64 = 0.43;
/// Durée de la transition de survol (loupe) : 0,12 s / 1,5.
const HOVER_ANIM: f32 = 0.08;

/// Avancement (0 → 1) de l'animation de clic de l'icône `key` ; 1 = au repos.
fn click_phase(ui: &egui::Ui, key: &str, clicked: bool) -> f32 {
    let id = egui::Id::unique(("wt_icon_click", key));
    let now = ui.input(|i| i.time);
    if clicked {
        ui.ctx().data_mut(|d| d.insert_temp(id, now));
    }
    let Some(start) = ui.ctx().data(|d| d.get_temp::<f64>(id)) else { return 1.0 };
    let a = ((now - start) / CLICK_ANIM) as f32;
    if a < 1.0 {
        ui.ctx().request_repaint();
        a.max(0.0)
    } else {
        1.0
    }
}

/// Loupe façon Dock macOS, limitée à l'icône survolée (les voisines ne bougent pas).
fn magnify(ui: &egui::Ui, key: &str, rect: Rect) -> f32 {
    const MAX: f32 = 0.42; // +42 % au maximum
    let hovered = ui.ctx().pointer_hover_pos().is_some_and(|p| rect.contains(p));
    let target = if hovered { MAX } else { 0.0 };
    1.0 + ui.ctx().animate_value_with_time(egui::Id::unique(("wt_icon_zoom", key)), target, HOVER_ANIM)
}

/// Bouton rond « Réglages », ancré en bas à gauche, au premier plan sur tous les onglets.
/// Renvoie `true` s'il a été cliqué.
pub fn settings_button(ctx: &egui::Context, window: Rect, active: bool) -> bool {
    let size = BUTTON + 16.0; // même diamètre que la pilule
    let pos = Pos2::new(
        window.left() + SCREEN_MARGIN,
        window.bottom() - SCREEN_MARGIN - size,
    );
    let mut clicked = false;
    egui::Area::new(egui::Id::unique("worktable_settings"))
        .order(Order::Foreground)
        .fixed_pos(pos)
        .show(ctx, |ui| {
            pill_frame().show(ui, |ui| {
                let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
                let resp = resp
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(Tab::Settings.label());
                let mut hl = Liquid::begin(ui, "sidebar_settings");
                if active {
                    hl.target(rect, "settings");
                }
                let phase = click_phase(ui, "settings", resp.clicked());
                let zoom = magnify(ui, "settings", rect);
                paint_button(ui, rect, &resp, hl.coverage(rect), zoom, phase, draw_gear);
                hl.end(ui, theme::TEXT(), BUTTON / 2.0);
                clicked = resp.clicked();
            });
        });
    clicked
}

fn pill_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::SIDEBAR_BG())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same((PILL_WIDTH / 2.0) as u8))
        .inner_margin(Margin::same(8))
        .shadow(Shadow {
            offset: [0, 6],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(140),
        })
}

fn paint_button(
    ui: &egui::Ui,
    rect: Rect,
    resp: &egui::Response,
    coverage: f32,
    zoom: f32,
    phase: f32,
    icon: impl FnOnce(&Painter, Rect, Color32, f32),
) {
    // `coverage` : part du bouton recouverte par la pastille liquide (l'icône s'inverse).
    let base = if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
    let fg = lerp_color(base, theme::BG(), coverage);
    let painter = ui.painter();
    if resp.hovered() && coverage < 0.5 {
        painter.rect_filled(rect, CornerRadius::same((BUTTON / 2.0) as u8), theme::SURFACE_HI());
    }
    // Petit « enfoncement » au clic, puis loupe : l'icône grossit sur place, centrée dans son bouton.
    let press = 1.0 - 0.09 * (std::f32::consts::PI * phase.min(1.0)).sin().max(0.0) * (1.0 - phase).max(0.0).sqrt();
    let size = 18.0 * zoom * press;
    let center = rect.center();
    icon(painter, Rect::from_center_size(center, vec2(size, size)), fg, phase);
}

pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

/// Lissage « ease in-out » pour les animations d'icônes.
fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Enveloppe aller-retour : 0 → 1 → 0.
fn bump(t: f32) -> f32 {
    (std::f32::consts::PI * t.clamp(0.0, 1.0)).sin()
}

/// Engrenage : au clic, il tourne d'un quart de tour.
fn draw_gear(p: &Painter, r: Rect, c: Color32, phase: f32) {
    let center = r.center();
    let rad = r.width() / 2.0;
    let spin = ease(phase) * std::f32::consts::FRAC_PI_8;
    p.circle_stroke(center, rad * 0.62, Stroke::new(1.6, c));
    p.circle_stroke(center, rad * 0.24, Stroke::new(1.6, c));
    for i in 0..8 {
        let a = spin + i as f32 * std::f32::consts::TAU / 8.0;
        let d = vec2(a.cos(), a.sin());
        p.line_segment([center + d * rad * 0.62, center + d * rad], Stroke::new(2.6, c));
    }
}

/// Icônes vectorielles minimalistes (pas de dépendance à une police d'icônes).
/// Icônes vectorielles ; `phase` (0 → 1) anime l'icône juste après un clic :
/// tableau de bord = les tuiles s'écartent, calendrier = saut + page qui se tourne,
/// mail = l'enveloppe s'ouvre et la lettre sort, notes = les lignes se réécrivent.
fn draw_icon(p: &Painter, r: Rect, tab: Tab, c: Color32, phase: f32) {
    let w = (r.width() / 18.0).max(0.5);
    let s = Stroke::new(1.6 * w.sqrt(), c);
    let k = CornerRadius::same(2);
    match tab {
        Tab::Settings => draw_gear(p, r, c, phase),
        Tab::Dashboard => {
            let g = 3.0 * w;
            let side = (r.width() - g) / 2.0;
            for (n, (i, j)) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)].into_iter().enumerate() {
                // Écartement en diagonale, légèrement décalé d'une tuile à l'autre.
                let spread = bump((phase - n as f32 * 0.08) / 0.75) * 1.6 * w;
                let dir = vec2(i * 2.0 - 1.0, j * 2.0 - 1.0);
                let min = r.min + vec2(i * (side + g), j * (side + g)) + dir * spread;
                p.rect_stroke(Rect::from_min_size(min, vec2(side, side)), k, s, egui::StrokeKind::Inside);
            }
        }
        Tab::Calendar => {
            // Page de calendrier façon Notion Calendar : bandeau plein en haut, numéro du jour en gras.
            let hop = -bump(phase) * 1.8 * w;
            let r = r.translate(vec2(0.0, hop));
            let page = r.shrink2(vec2(0.5 * w, 0.0));
            let round = CornerRadius::same((3.5 * w) as u8);
            let band_h = page.height() * 0.30;
            let band = Rect::from_min_size(page.min, vec2(page.width(), band_h));
            p.rect_filled(band, CornerRadius { nw: round.nw, ne: round.ne, sw: 0, se: 0 }, c);
            p.rect_stroke(page, round, s, egui::StrokeKind::Inside);
            // Le chiffre « tourne » légèrement au clic (fondu bref).
            let alpha = 1.0 - 0.6 * bump(phase);
            let day = chrono::Datelike::day(&chrono::Local::now()).to_string();
            let body_c = Pos2::new(page.center().x, band.bottom() + (page.bottom() - band.bottom()) / 2.0 + 0.2 * w);
            p.text(
                body_c,
                egui::Align2::CENTER_CENTER,
                day,
                theme::editor_font(page.height() * 0.48, true),
                c.gamma_multiply(alpha),
            );
        }
        Tab::Mail => {
            // « M » encadré : cadre arrondi, M centré ; au clic, le creux du M s'enfonce un peu.
            let frame = r.shrink(0.5 * w);
            p.rect_stroke(frame, CornerRadius::same((4.0 * w) as u8), s, egui::StrokeKind::Inside);
            let m = frame.shrink2(vec2(4.2 * w, 4.6 * w));
            let dip = bump(phase) * 1.2 * w;
            let thick = Stroke::new(2.0 * w.sqrt(), c);
            p.add(egui::Shape::line(
                vec![
                    m.left_bottom(),
                    m.left_top(),
                    Pos2::new(m.center().x, m.center().y + 1.0 * w + dip),
                    m.right_top(),
                    m.right_bottom(),
                ],
                thick,
            ));
        }
        Tab::Messages => {
            // Bulle ovale pleine dans un cadre arrondi (à la manière de l'app Messages d'Apple) ;
            // au clic, la bulle « pop » légèrement.
            let frame = r.shrink(0.5 * w);
            p.rect_stroke(frame, CornerRadius::same((4.0 * w) as u8), s, egui::StrokeKind::Inside);
            let pop = 1.0 + 0.08 * bump(phase);
            let center = frame.center() + vec2(0.0, -0.6 * w);
            let radius = vec2(5.4 * w, 4.3 * w) * pop;
            p.add(egui::Shape::ellipse_filled(center, radius, c));
            // Petite queue en bas à gauche.
            let base = center + vec2(-radius.x * 0.55, radius.y * 0.62);
            p.add(egui::Shape::convex_polygon(
                vec![base + vec2(-0.6 * w, -1.2 * w), base + vec2(2.4 * w, 0.2 * w), base + vec2(-1.8 * w, 2.0 * w) * pop],
                c,
                Stroke::NONE,
            ));
        }
        Tab::Files => {
            // Dossier ; au clic, le rabat avant s'entrouvre.
            let tab_w = r.width() * 0.42;
            let back = Rect::from_min_max(r.min + vec2(0.0, 3.0 * w), r.max - vec2(0.0, 1.0 * w));
            let tab_top = back.top() - 2.5 * w;
            p.add(egui::Shape::line(
                vec![
                    Pos2::new(back.left(), back.top() + 2.0 * w),
                    Pos2::new(back.left(), tab_top),
                    Pos2::new(back.left() + tab_w - 2.0 * w, tab_top),
                    Pos2::new(back.left() + tab_w, back.top()),
                    Pos2::new(back.right(), back.top()),
                    Pos2::new(back.right(), back.top() + 3.0 * w),
                ],
                s,
            ));
            let lift = bump(phase) * 1.6 * w;
            let front = Rect::from_min_max(Pos2::new(back.left(), back.top() + 3.5 * w - lift), back.max);
            p.rect_stroke(front, CornerRadius::same(2), s, egui::StrokeKind::Inside);
        }
        Tab::Notes => {
            let body = Rect::from_center_size(r.center(), vec2(r.width() * 0.8, r.height()));
            p.rect_stroke(body, k, s, egui::StrokeKind::Inside);
            for i in 1..=3 {
                let y = body.top() + i as f32 * body.height() / 4.0;
                let full = if i == 3 { body.center().x } else { body.right() - 4.0 * w };
                let x0 = body.left() + 4.0 * w;
                // Les lignes s'effacent puis se réécrivent l'une après l'autre.
                let grow = if phase >= 1.0 { 1.0 } else { ease((phase - 0.15 * (i - 1) as f32) / 0.55) };
                if grow > 0.0 {
                    p.hline(x0..=x0 + (full - x0) * grow, y, s);
                }
            }
        }
    }
}
