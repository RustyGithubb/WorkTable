//! Barre de navigation verticale en forme de pilule, flottante à gauche.

use eframe::egui::{self, Color32, CornerRadius, Margin, Order, Painter, Pos2, Rect, Sense, Shadow, Stroke, vec2};

use std::sync::Mutex;

use crate::app::Tab;
use crate::liquid::Liquid;
use crate::search::Target;
use crate::theme;

/// Survol prolongé d'une icône avant d'ouvrir l'aperçu des notifications (secondes).
const PEEK_DELAY: f64 = 0.3;
/// Vitesse de l'animation de l'aperçu (1 = vitesse d'origine).
const PEEK_SPEED: f32 = 1.5;
/// Distance au-delà de laquelle l'aperçu se referme (points).
const PEEK_KEEP: f32 = 48.0;
const PEEK_W: f32 = 320.0;
const PEEK_MAX: usize = 6;

/// Notification en attente, affichée dans l'aperçu flottant d'un onglet.
pub struct PeekItem {
    pub icon: &'static str,
    pub title: String,
    pub text: String,
    /// Date (ms).
    pub when: i64,
    pub target: Target,
}

/// Résultat d'un clic dans la barre.
pub enum Nav {
    Tab(Tab),
    Go(Box<Target>),
}

#[derive(Default)]
struct PeekState {
    /// Icône survolée et depuis quand.
    hover: Option<(Tab, f64)>,
    /// Aperçu ouvert, position de l'icône, rectangle de l'aperçu (image précédente).
    open: Option<(Tab, Rect)>,
    panel: Option<Rect>,
    /// Début de l'animation d'ouverture (s).
    opened_at: f64,
    /// Fermeture animée en cours : onglet, icône, aperçu, début (s).
    closing: Option<(Tab, Rect, Rect, f64)>,
}

static PEEK: Mutex<Option<PeekState>> = Mutex::new(None);

const PILL_WIDTH: f32 = 56.0;
const BUTTON: f32 = 40.0;
const SCREEN_MARGIN: f32 = 16.0;

/// Affiche la barre et renvoie l'onglet cliqué (ou la notification choisie dans un aperçu).
/// Survol d'une icône pendant 1 s avec des notifications en attente : petit panneau flottant à
/// droite, avec un raccourci vers chacune.
pub fn show(ctx: &egui::Context, window: Rect, current: Tab, badges: &[(Tab, usize)], peeks: &[(Tab, Vec<PeekItem>)]) -> Option<Nav> {
    // Hauteur : logo + séparateur + boutons.
    let n = Tab::ALL.len() as f32;
    let inner_h = BUTTON + 17.0 + n * BUTTON + (n - 1.0) * 8.0;
    let pill_h = inner_h + 16.0;
    // Centrée, mais sans jamais chevaucher la pilule Blueprint + Réglages en bas.
    let max_y = window.bottom() - 2.0 * SCREEN_MARGIN - BOTTOM_H - pill_h;
    let y = (window.center().y - pill_h / 2.0).min(max_y).max(window.top() + SCREEN_MARGIN);
    let pos = Pos2::new(window.left() + SCREEN_MARGIN, y);

    let mut clicked = None;
    let mut hovered: Option<(Tab, Rect)> = None;

    egui::Area::new(egui::Id::unique("worktable_sidebar")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        pill_frame().show(ui, |ui| {
            ui.set_width(BUTTON);
            ui.spacing_mut().item_spacing.y = 8.0;

            // Logo
            let (logo, logo_resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
            let logo_resp = logo_resp.on_hover_text(t!("Clic droit : onglets personnalisés", "Right-click: custom tabs"));
            spinning_logo(ui, logo, logo_resp.clicked());
            // Clic droit : galerie des onglets personnalisés, à droite de la barre.
            if logo_resp.secondary_clicked() {
                crate::gallery::toggle(ui.ctx(), Pos2::new(logo.right() + 26.0, logo.top()));
            }
            // Onglet personnalisé ouvert ou galerie affichée : anneau autour du logo.
            if matches!(current, Tab::Custom(_)) || crate::gallery::is_open() {
                ui.painter().circle_stroke(logo.center(), BUTTON / 2.0, Stroke::new(1.5, theme::TEXT_MUTED()));
            }

            // Séparateur
            let (sep, _) = ui.allocate_exact_size(vec2(BUTTON, 1.0), Sense::hover());
            ui.painter().hline(sep.x_range().shrink(8.0), sep.center().y, Stroke::new(1.0, theme::BORDER()));

            // La pastille blanche de sélection s'écoule d'un onglet à l'autre.
            let mut hl = Liquid::begin(ui, "sidebar_nav");
            let mut dots = Vec::new();
            let peek_open = PEEK.lock().unwrap().as_ref().and_then(|p| p.open.map(|o| o.0));
            for tab in Tab::ALL {
                let r = nav_button(ui, tab, tab == current, &mut hl, peek_open != Some(tab));
                if r.clicked() {
                    clicked = Some(tab);
                }
                if r.hovered() {
                    hovered = Some((tab, r.rect));
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

    let go = peek(ctx, pos.x + PILL_WIDTH + 10.0, hovered, clicked, peeks);
    match (go, clicked) {
        (Some(t), _) => Some(Nav::Go(Box::new(t))),
        (None, Some(t)) => Some(Nav::Tab(t)),
        _ => None,
    }
}

/// Ressort amorti (0 → 1 avec dépassement) : effet élastique.
fn spring(t: f32, omega: f32, zeta: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    let wd = omega * (1.0 - zeta * zeta).sqrt();
    1.0 - (-zeta * omega * t).exp() * ((wd * t).cos() + zeta * omega / wd * (wd * t).sin())
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Gère l'aperçu flottant : ouverture après un survol prolongé, fermeture quand la souris
/// s'éloigne ou qu'un autre onglet est choisi. Renvoie la notification cliquée.
fn peek(ctx: &egui::Context, x: f32, hovered: Option<(Tab, Rect)>, clicked: Option<Tab>, peeks: &[(Tab, Vec<PeekItem>)]) -> Option<Target> {
    let now = ctx.input(|i| i.time);
    let pointer = ctx.input(|i| i.pointer.hover_pos());
    let items = |t: Tab| peeks.iter().find(|(p, v)| *p == t && !v.is_empty()).map(|(_, v)| v);
    let mut guard = PEEK.lock().unwrap();
    let st = guard.get_or_insert_with(PeekState::default);

    // Survol prolongé d'une icône qui a des notifications en attente.
    match hovered {
        Some((tab, rect)) if items(tab).is_some() => {
            let since = match st.hover {
                Some((t, s)) if t == tab => s,
                _ => now,
            };
            st.hover = Some((tab, since));
            if st.open.map(|o| o.0) != Some(tab) {
                if now - since >= PEEK_DELAY {
                    st.open = Some((tab, rect));
                    st.panel = None;
                    st.opened_at = now;
                    st.closing = None;
                } else {
                    ctx.request_repaint_after(std::time::Duration::from_secs_f64(PEEK_DELAY - (now - since) + 0.01));
                }
            }
        }
        _ => st.hover = None,
    }
    // Fermeture : autre onglet cliqué, Échap, souris trop loin ou plus de notifications.
    let mut close = false;
    if let Some((tab, anchor)) = st.open {
        let far = match pointer {
            None => true,
            Some(p) => !st.panel.map_or(anchor, |r| r.union(anchor)).expand(PEEK_KEEP).contains(p),
        };
        let esc = ctx.input(|i| i.key_pressed(egui::Key::Escape));
        close = clicked.is_some_and(|c| c != tab) || far || esc || items(tab).is_none();
    }

    let mut go = None;
    if let Some((tab, anchor)) = st.open.filter(|_| !close)
        && let Some(list) = items(tab)
    {
        let t = (now - st.opened_at) as f32 * PEEK_SPEED;
        let (panel, clicked_item) = draw_peek(ctx, x, anchor, tab, list, st.panel, Grow::Open(t));
        st.panel = Some(panel);
        go = clicked_item;
        if t < 1.0 {
            ctx.request_repaint();
        }
    }
    if close || go.is_some() {
        if let (Some((tab, anchor)), Some(panel)) = (st.open, st.panel) {
            st.closing = Some((tab, anchor, panel, now));
        }
        st.open = None;
        st.panel = None;
        st.hover = None;
    }
    // Fermeture : l'aperçu se rétracte dans l'icône.
    if let Some((tab, anchor, panel, start)) = st.closing {
        let t = (now - start) as f32 * PEEK_SPEED / 0.18;
        match items(tab) {
            Some(list) if t < 1.0 && st.open.is_none() => {
                draw_peek(ctx, x, anchor, tab, list, Some(panel), Grow::Close(t));
                ctx.request_repaint();
            }
            _ => st.closing = None,
        }
    }
    go
}

enum Grow {
    /// Secondes depuis l'ouverture.
    Open(f32),
    /// Avancement de la fermeture (0 → 1).
    Close(f32),
}

/// Dessine l'aperçu. À l'ouverture, une goutte part de l'icône et s'étire jusqu'à la taille
/// du panneau avec un rebond élastique (largeur d'abord, hauteur ensuite), reliée à l'icône
/// par un pont « liquide » qui s'amincit puis se résorbe en petite pointe.
fn draw_peek(ctx: &egui::Context, x: f32, anchor: Rect, tab: Tab, list: &[PeekItem], last: Option<Rect>, grow: Grow) -> (Rect, Option<Target>) {
    let top = anchor.top() - 10.0;
    // Taille finale : mesurée à l'image précédente, estimée la première fois.
    let n = list.len().min(PEEK_MAX) as f32;
    let est = vec2(PEEK_W, 16.0 + 22.0 + n * 48.0 + if list.len() > PEEK_MAX { 20.0 } else { 0.0 } + 36.0);
    let full = Rect::from_min_size(Pos2::new(x, top), last.map_or(est, |r| r.size()));

    let (kw, kh, neck, content) = match grow {
        Grow::Open(t) => (spring(t, 17.0, 0.42), spring(t - 0.05, 15.0, 0.38), 1.0 - ((t - 0.12) / 0.4).clamp(0.0, 1.0), ((t - 0.16) / 0.18).clamp(0.0, 1.0)),
        Grow::Close(t) => {
            let k = 1.0 - t * t;
            (k, k * k, 0.0, (1.0 - t * 2.5).max(0.0))
        }
    };
    // Goutte : du cercle de l'icône au panneau.
    let seed = Rect::from_center_size(Pos2::new(x + 14.0, anchor.center().y), vec2(28.0, 28.0));
    let blob = Rect::from_min_max(
        Pos2::new(lerp(seed.left(), full.left(), kw.min(1.0)), lerp(seed.top(), full.top(), kh)),
        Pos2::new(lerp(seed.right(), full.right(), kw), lerp(seed.bottom(), full.bottom(), kh)),
    );
    let radius = lerp(14.0, 16.0, kw.clamp(0.0, 1.0)).min(blob.height() / 2.0).min(blob.width() / 2.0).max(0.0);

    let mut go = None;
    let area = egui::Area::new(egui::Id::unique("wt_sidebar_peek")).order(Order::Foreground).fixed_pos(full.min).constrain(false).show(ctx, |ui| {
        let p = ui.painter().with_clip_rect(ctx.content_rect());
        // Pont liquide entre l'icône et la goutte (épais au départ, il s'amincit et disparaît).
        let fill = theme::SIDEBAR_BG();
        let cy = anchor.center().y.clamp(blob.top() + radius, (blob.bottom() - radius).max(blob.top() + radius));
        let x0 = anchor.right() - 6.0;
        let x1 = blob.left() + radius.min(12.0);
        if neck > 0.01 && x1 > x0 {
            let mut mesh = egui::Mesh::default();
            let steps = 16;
            for i in 0..=steps {
                let u = i as f32 / steps as f32;
                // Plus étroit au milieu (aspect de goutte qui s'étire).
                let h = lerp(13.0 * neck, 3.0 + 13.0 * neck, (2.0 * u - 1.0).powi(2));
                let px = lerp(x0, x1, u);
                mesh.colored_vertex(Pos2::new(px, cy - h), fill);
                mesh.colored_vertex(Pos2::new(px, cy + h), fill);
            }
            for i in 0..steps as u32 {
                let k = i * 2;
                mesh.add_triangle(k, k + 1, k + 2);
                mesh.add_triangle(k + 1, k + 3, k + 2);
            }
            p.add(mesh);
        }
        let round = CornerRadius::same(radius as u8);
        p.add(Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha((110.0 * content.max(kw.clamp(0.0, 1.0) * 0.6)) as u8) }.as_shape(blob, round));
        p.rect(blob, round, fill, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        // Petite pointe vers l'icône, une fois le pont résorbé.
        if neck <= 0.01 && content > 0.0 {
            let ty = anchor.center().y.clamp(blob.top() + 18.0, blob.bottom() - 18.0);
            let (a, b, c) = (Pos2::new(blob.left() + 1.0, ty - 7.0), Pos2::new(blob.left() - 6.0, ty), Pos2::new(blob.left() + 1.0, ty + 7.0));
            p.add(egui::Shape::convex_polygon(vec![a, b, c], fill, Stroke::NONE));
            let edge = Stroke::new(1.0, theme::BORDER().gamma_multiply(content));
            p.line_segment([a, b], edge);
            p.line_segment([b, c], edge);
        }

        // Contenu, coupé par la goutte, en fondu.
        ui.set_clip_rect(blob.shrink(1.0));
        ui.set_opacity(content);
        egui::Frame::new().inner_margin(Margin::same(8)).show(ui, |ui| {
            ui.set_width(PEEK_W - 16.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tab.label().to_uppercase()).small().strong().color(theme::TEXT_MUTED()));
                ui.label(egui::RichText::new(tf!("{} en attente", "{} pending", list.len())).small().color(theme::TEXT_MUTED()));
            });
            ui.add_space(4.0);
            for item in list.iter().take(PEEK_MAX) {
                if peek_row(ui, item).clicked() && content > 0.5 {
                    go = Some(item.target.clone());
                }
            }
            if list.len() > PEEK_MAX {
                ui.add_space(2.0);
                ui.horizontal(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new(tf!("+ {} autres", "+ {} more", list.len() - PEEK_MAX)).small().color(theme::TEXT_FAINT()));
                });
            }
            ui.add_space(4.0);
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(10), theme::SURFACE_HI());
            }
            ui.painter().text(
                r.left_center() + vec2(10.0, 0.0),
                egui::Align2::LEFT_CENTER,
                tf!("Ouvrir {} →", "Open {} →", tab.label()),
                egui::FontId::proportional(13.0),
                if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() },
            );
            if resp.clicked() && content > 0.5 {
                go = Some(Target::Tab(tab));
            }
        });
    });
    (Rect::from_min_size(full.min, area.response.rect.size()), go)
}

/// Ligne de l'aperçu : icône, titre et heure, texte sur une ligne.
fn peek_row(ui: &mut egui::Ui, item: &PeekItem) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(r, CornerRadius::same(10), theme::SURFACE_HI());
    }
    // Même pastille que les bandeaux de notification (couleurs de la sélection de la barre).
    let c = Pos2::new(r.left() + 22.0, r.center().y);
    p.circle_filled(c, 15.0, theme::TEXT());
    // Icône de l'onglet, comme dans le bandeau de notification.
    let tab = if item.icon == "✉️" { Tab::Mail } else { Tab::Messages };
    draw_icon(p, Rect::from_center_size(c, vec2(16.0, 16.0)), tab, theme::BG(), 1.0);
    let x = r.left() + 46.0;
    let when = when_label(item.when);
    let wg = p.layout_no_wrap(when, egui::FontId::proportional(11.0), theme::TEXT_FAINT());
    let right = r.right() - 10.0;
    let one_line = |text: &str, font: egui::FontId, color: Color32, max: f32| {
        let mut j = egui::text::LayoutJob::simple_singleline(text.to_string(), font, color);
        j.wrap = egui::text::TextWrapping { max_width: max, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        ui.fonts_mut(|f| f.layout_job(j))
    };
    let title = one_line(&item.title, theme::title_font(13.5), theme::TEXT(), right - x - wg.size().x - 8.0);
    let text = one_line(&item.text, egui::FontId::proportional(12.5), theme::TEXT_MUTED(), right - x);
    p.galley(Pos2::new(x, r.top() + 6.0), title, theme::TEXT());
    p.galley(Pos2::new(right - wg.size().x, r.top() + 8.0), wg, theme::TEXT_FAINT());
    p.galley(Pos2::new(x, r.top() + 25.0), text, theme::TEXT_MUTED());
    resp
}

/// Heure si c'est aujourd'hui, sinon date courte.
fn when_label(ms: i64) -> String {
    use chrono::TimeZone;
    let Some(t) = chrono::Local.timestamp_millis_opt(ms).single() else { return String::new() };
    if t.date_naive() == chrono::Local::now().date_naive() { t.format("%H:%M").to_string() } else { t.format("%d/%m").to_string() }
}

/// Rotation du logo : chaque clic l'accélère et le fait grossir (effet élastique), puis il
/// freine doucement et revient à sa position et à sa taille d'origine.
#[derive(Default)]
struct Spin {
    angle: f32,
    vel: f32,
    scale: f32,
    scale_vel: f32,
}

static SPIN: Mutex<Option<Spin>> = Mutex::new(None);

/// Intensité de la rotation du logo (0 = immobile ; ~1 après quelques clics, plus au-delà) :
/// les panneaux tremblent d'autant plus fort que la roue tourne vite.
pub fn spin_intensity() -> f32 {
    SPIN.lock().ok().and_then(|g| g.as_ref().map(|s| s.vel.abs() / 30.0)).unwrap_or(0.0)
}

fn spinning_logo(ui: &mut egui::Ui, cell: Rect, clicked: bool) {
    use std::f32::consts::TAU;
    let dt = ui.input(|i| i.stable_dt).clamp(0.0, 1.0 / 30.0);
    let mut guard = SPIN.lock().unwrap();
    let s = guard.get_or_insert_with(|| Spin { scale: 1.0, ..Default::default() });
    if clicked {
        // Plus on clique, plus il tourne vite (impulsion croissante) et plus il grossit.
        s.vel += 8.0 + s.vel * 0.4;
        s.scale_vel += 3.0;
    }
    // Freinage doux, puis retour à l'endroit (tour complet le plus proche) quand il ralentit.
    s.vel *= (-dt).exp();
    let target = (s.angle / TAU).round() * TAU;
    if s.vel.abs() < 3.0 {
        s.vel += (25.0 * (target - s.angle) - 7.0 * s.vel) * dt;
    }
    s.angle += s.vel * dt;
    // Taille : suit la vitesse, avec un ressort qui rebondit (élastique).
    let goal = 1.0 + (s.vel.abs() / 25.0).min(0.7);
    s.scale_vel += (160.0 * (goal - s.scale) - 10.0 * s.scale_vel) * dt;
    s.scale += s.scale_vel * dt;
    let resting = s.vel.abs() < 0.02 && (s.angle - target).abs() < 0.002 && (s.scale - 1.0).abs() < 0.001 && s.scale_vel.abs() < 0.01;
    if resting {
        *s = Spin { scale: 1.0, ..Default::default() };
        paint_logo(ui.painter(), Rect::from_center_size(cell.center(), vec2(28.0, 28.0)), theme::TEXT());
        return;
    }
    ui.ctx().request_repaint();
    let size = 28.0 * s.scale;
    let tex = logo_glyph(ui.ctx());
    let mut mesh = egui::Mesh::with_texture(tex.id());
    mesh.add_rect_with_uv(Rect::from_center_size(cell.center(), vec2(size, size)), Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), theme::TEXT());
    mesh.rotate(egui::emath::Rot2::from_angle(s.angle), cell.center());
    // Au-dessus du reste de la barre pendant qu'il est agrandi.
    ui.ctx().layer_painter(egui::LayerId::new(Order::Tooltip, egui::Id::unique("wt_logo_spin"))).add(mesh);
}

/// Logo en blanc sur fond transparent (teinté à l'affichage), pour le faire tourner.
fn logo_glyph(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::unique("wt_logo_glyph");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(include_bytes!("../assets/logo-glyph.png")).map(|i| i.to_rgba8()).unwrap_or_else(|_| image::RgbaImage::new(1, 1));
    let color = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    let opts = egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR };
    let t = ctx.load_texture("wt_logo_glyph", color, opts);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}

/// Logo de l'app, dessiné en vectoriel (net à toutes les tailles) dans la couleur du texte :
/// équerres aux coins, cadre arrondi, grille 3 × 3. Même tracé que `assets/logo.png`.
pub fn paint_logo(p: &Painter, r: Rect, color: Color32) {
    let k = r.width() / 100.0;
    let at = |x: f32, y: f32| r.min + vec2(x * k, y * k);
    let w = 5.0 * k;
    let stroke = Stroke::new(w, color);
    for (x0, y0, dx, dy) in [(8.0, 8.0, 1.0, 1.0), (92.0, 8.0, -1.0, 1.0), (8.0, 92.0, 1.0, -1.0), (92.0, 92.0, -1.0, -1.0)] {
        let pts = [at(x0, y0 + 14.0 * dy), at(x0, y0), at(x0 + 14.0 * dx, y0)];
        p.line_segment([pts[0], pts[1]], stroke);
        p.line_segment([pts[1], pts[2]], stroke);
        for q in pts {
            p.circle_filled(q, w / 2.0, color);
        }
    }
    p.rect_stroke(Rect::from_min_max(at(18.0, 18.0), at(82.0, 82.0)), CornerRadius::same((10.0 * k) as u8), stroke, egui::StrokeKind::Middle);
    let t = (40.0 - 2.0 * 4.0) / 3.0;
    for i in 0..3 {
        for j in 0..3 {
            let (x, y) = (30.0 + j as f32 * (t + 4.0), 30.0 + i as f32 * (t + 4.0));
            p.rect_filled(Rect::from_min_max(at(x, y), at(x + t, y + t)), CornerRadius::same((2.5 * k).round() as u8), color);
        }
    }
}

/// Icône de la fenêtre (logo monochrome sur fond sombre arrondi).
pub const LOGO_PNG: &[u8] = include_bytes!("../assets/logo.png");

/// Petite pastille terracotta avec le nombre de non lus, en haut à droite de l'icône.
fn badge(p: &Painter, rect: Rect, n: usize) {
    let text = if n > 99 { "99+".to_string() } else { n.to_string() };
    let g = p.layout_no_wrap(text, egui::FontId::proportional(9.5), Color32::WHITE);
    let w = (g.size().x + 8.0).max(15.0);
    let r = Rect::from_center_size(rect.right_top() + vec2(-6.0, 7.0), vec2(w, 15.0));
    p.rect(r, CornerRadius::same(8), theme::TODAY(), Stroke::new(1.5, theme::SIDEBAR_BG()), egui::StrokeKind::Outside);
    p.galley(r.center() - g.size() / 2.0, g, Color32::WHITE);
}

/// Pastille sans nombre (mise à jour disponible), même couleur que celle des non lus.
fn update_dot(p: &Painter, rect: Rect) {
    p.circle(rect.right_top() + vec2(-5.0, 5.0), 5.0, theme::TODAY(), Stroke::new(1.5, theme::SIDEBAR_BG()));
}

fn nav_button(ui: &mut egui::Ui, tab: Tab, active: bool, hl: &mut Liquid, tip: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(BUTTON, BUTTON), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    // Pas d'infobulle quand l'aperçu des notifications de cet onglet est ouvert.
    let resp = if tip { resp.on_hover_text(tab.label()) } else { resp };
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

/// Onglets de la pilule du bas, de haut en bas.
const BOTTOM: [Tab; 2] = [Tab::Blueprint, Tab::Settings];
/// Hauteur de la pilule du bas (boutons + espacement + marges).
const BOTTOM_H: f32 = BOTTOM.len() as f32 * BUTTON + (BOTTOM.len() as f32 - 1.0) * 8.0 + 16.0;

/// Pilule Blueprint + Réglages, ancrée en bas à gauche, au premier plan sur tous les onglets.
/// La sélection liquide passe de l'un à l'autre. `dot` : pastille sur Réglages (mise à jour
/// disponible). Renvoie l'onglet cliqué.
pub fn bottom_buttons(ctx: &egui::Context, window: Rect, current: Tab, dot: bool) -> Option<Tab> {
    let pos = Pos2::new(window.left() + SCREEN_MARGIN, window.bottom() - SCREEN_MARGIN - BOTTOM_H);
    let mut clicked = None;
    egui::Area::new(egui::Id::unique("worktable_settings")).order(Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        pill_frame().show(ui, |ui| {
            ui.set_width(BUTTON);
            ui.spacing_mut().item_spacing.y = 8.0;
            let mut hl = Liquid::begin(ui, "sidebar_settings");
            let mut dot_rect = None;
            for tab in BOTTOM {
                let r = nav_button(ui, tab, tab == current, &mut hl, true);
                if r.clicked() {
                    clicked = Some(tab);
                }
                if tab == Tab::Settings && dot {
                    dot_rect = Some(r.rect);
                }
            }
            hl.end(ui, theme::TEXT(), BUTTON / 2.0);
            if let Some(r) = dot_rect {
                update_dot(ui.painter(), r);
            }
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
        .shadow(Shadow { offset: [0, 6], blur: 24, spread: 0, color: Color32::from_black_alpha(140) })
}

fn paint_button(ui: &egui::Ui, rect: Rect, resp: &egui::Response, coverage: f32, zoom: f32, phase: f32, icon: impl FnOnce(&Painter, Rect, Color32, f32)) {
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

/// Blueprint : deux nodes (en-tête plein, comme dans Unreal) reliées par un fil ; au clic, une
/// impulsion d'exécution parcourt le fil.
fn draw_blueprint(p: &Painter, r: Rect, c: Color32, phase: f32, w: f32) {
    let s = Stroke::new(1.6 * w.sqrt(), c);
    let (nw, nh) = (r.width() * 0.42, r.height() * 0.36);
    let a = Rect::from_min_size(r.min + vec2(0.0, 0.5 * w), vec2(nw, nh));
    let b = Rect::from_min_size(Pos2::new(r.right() - nw, r.bottom() - nh - 0.5 * w), vec2(nw, nh));
    let k = (2.5 * w) as u8;
    for n in [a, b] {
        p.rect_stroke(n, CornerRadius::same(k), s, egui::StrokeKind::Inside);
        p.rect_filled(Rect::from_min_size(n.min, vec2(n.width(), n.height() * 0.4)), CornerRadius { nw: k, ne: k, sw: 0, se: 0 }, c);
    }
    let p0 = Pos2::new(a.right(), a.bottom() - nh * 0.28);
    let p3 = Pos2::new(b.left(), b.bottom() - nh * 0.28);
    let dx = (p3.x - p0.x).abs().max(4.0 * w);
    let wire = egui::epaint::CubicBezierShape::from_points_stroke([p0, p0 + vec2(dx, 0.0), p3 - vec2(dx, 0.0), p3], false, Color32::TRANSPARENT, s);
    if phase < 1.0 {
        p.circle_filled(wire.sample(ease(phase)), 1.9 * w, c);
    }
    p.add(wire);
    p.circle_filled(p0, 1.2 * w, c);
    p.circle_filled(p3, 1.2 * w, c);
}

/// Icônes vectorielles minimalistes (pas de dépendance à une police d'icônes).
/// Icônes vectorielles ; `phase` (0 → 1) anime l'icône juste après un clic :
/// tableau de bord = les tuiles s'écartent, calendrier = saut + page qui se tourne,
/// mail = l'enveloppe s'ouvre et la lettre sort, notes = les lignes se réécrivent.
/// Icône d'un onglet (`phase` : animation de clic, 1 = au repos). Sert aussi aux notifications.
pub(crate) fn draw_icon(p: &Painter, r: Rect, tab: Tab, c: Color32, phase: f32) {
    let w = (r.width() / 18.0).max(0.5);
    let s = Stroke::new(1.6 * w.sqrt(), c);
    let k = CornerRadius::same(2);
    match tab {
        Tab::Settings => draw_gear(p, r, c, phase),
        Tab::Custom(_) => crate::sidebar::paint_logo(p, r, c),
        Tab::Blueprint => draw_blueprint(p, r, c, phase, w),
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
            p.text(body_c, egui::Align2::CENTER_CENTER, day, theme::editor_font(page.height() * 0.48, true), c.gamma_multiply(alpha));
        }
        Tab::Mail => {
            // « M » encadré : cadre arrondi, M centré ; au clic, le creux du M s'enfonce un peu.
            let frame = r.shrink(0.5 * w);
            p.rect_stroke(frame, CornerRadius::same((4.0 * w) as u8), s, egui::StrokeKind::Inside);
            let m = frame.shrink2(vec2(4.2 * w, 4.6 * w));
            let dip = bump(phase) * 1.2 * w;
            let thick = Stroke::new(2.0 * w.sqrt(), c);
            p.add(egui::Shape::line(vec![m.left_bottom(), m.left_top(), Pos2::new(m.center().x, m.center().y + 1.0 * w + dip), m.right_top(), m.right_bottom()], thick));
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
            p.add(egui::Shape::convex_polygon(vec![base + vec2(-0.6 * w, -1.2 * w), base + vec2(2.4 * w, 0.2 * w), base + vec2(-1.8 * w, 2.0 * w) * pop], c, Stroke::NONE));
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
