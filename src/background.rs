//! Arrière-plan : fermer la fenêtre la cache seulement. L'app continue de tourner (relève des
//! mails, notifications, rappels, graphs du Blueprint) et revient avec un raccourci global
//! (Ctrl+Maj+Espace par défaut, réglable), en relançant l'app ou en cliquant une notification.
//! « Quitter WorkTable » (Réglages) l'arrête pour de bon.
//!
//! Une seule instance tourne à la fois : relancer l'exe rouvre celle qui est en arrière-plan.
//! Hors Windows, la fermeture quitte l'app comme avant.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui::{self, Key};

use crate::prefs::Hotkey;

#[cfg(windows)]
mod win;

/// Argument passé à l'app relancée après une mise à jour : elle attend que l'ancienne se ferme.
pub const AFTER_UPDATE: &str = "--after-update";

static QUIT: AtomicBool = AtomicBool::new(false);
/// Un nouveau raccourci est en cours de saisie (Réglages) : l'actuel est suspendu.
static RECORDING: AtomicBool = AtomicBool::new(false);

pub fn set_recording(on: bool) {
    RECORDING.store(on, Ordering::Relaxed);
}

/// Ferme l'app pour de bon (sans passer en arrière-plan).
pub fn quit(ctx: &egui::Context) {
    QUIT.store(true, Ordering::Relaxed);
    ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Close);
}

/// Au lancement : `false` si une autre instance tourne déjà (elle vient d'être rouverte ;
/// celle-ci doit s'arrêter).
pub fn single_instance() -> bool {
    #[cfg(windows)]
    {
        let after_update = std::env::args().any(|a| a == AFTER_UPDATE);
        // En développement (`cargo run`), la nouvelle compilation remplace celle qui tournait
        // encore en arrière-plan ; sinon, l'instance déjà lancée est simplement rouverte.
        let mode = crate::paths::mode();
        win::single_instance(&format!("{mode:?}"), after_update, mode == crate::paths::Mode::Development)
    }
    #[cfg(not(windows))]
    true
}

/// Démarre l'écoute des raccourcis (ouvrir / cacher, quitter) et des demandes de réouverture.
pub fn start(ctx: &egui::Context, hotkey: &Hotkey, quit_key: &Hotkey) {
    #[cfg(windows)]
    win::start(ctx.clone(), code(hotkey), code(quit_key));
    #[cfg(not(windows))]
    let _ = (ctx, hotkey, quit_key);
}

/// À chaque image : fermeture → arrière-plan, raccourci modifié dans les réglages.
pub fn on_frame(ctx: &egui::Context, hotkey: &Hotkey, quit_key: &Hotkey) {
    static LAST: Mutex<Option<(Hotkey, Hotkey, bool)>> = Mutex::new(None);
    // Fermeture (✕, Alt+F4) ou raccourci : animation, puis fenêtre cachée.
    let close = ctx.input(|i| i.viewport().close_requested()) && !QUIT.load(Ordering::Relaxed) && can_hide();
    if close {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
    if close | CLOSE_REQ.swap(false, Ordering::Relaxed) {
        start_anim(false);
    }
    // Remis à zéro à chaque image : les Réglages le redemandent tant que la saisie est ouverte.
    let paused = RECORDING.swap(false, Ordering::Relaxed);
    let now = (hotkey.clone(), quit_key.clone(), paused);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    // Hors Windows (pas de raccourci global) : « quitter » quand l'app a le clavier (pas à
    // l'image où le raccourci vient d'être choisi dans les Réglages).
    #[cfg(not(windows))]
    if !paused && last.as_ref().is_some_and(|l| l.1 == *quit_key) && pressed(ctx, quit_key) {
        quit(ctx);
    }
    if last.as_ref() != Some(&now) {
        if last.is_some() {
            #[cfg(windows)]
            win::set_hotkeys(if paused { None } else { code(hotkey) }, if paused { None } else { code(quit_key) });
        }
        *last = Some(now);
    }
}

/// Raccourci tapé à cette image (dans l'app).
#[cfg_attr(windows, allow(dead_code))]
fn pressed(ctx: &egui::Context, h: &Hotkey) -> bool {
    let Some(key) = Key::from_name(&h.key) else { return false };
    ctx.input(|i| {
        i.events.iter().any(|e| matches!(e, egui::Event::Key { key: k, pressed: true, modifiers: m, .. } if *k == key && m.ctrl == h.ctrl && m.shift == h.shift && m.alt == h.alt))
    })
}

/// La fenêtre peut-elle être cachée sur ce système ?
fn can_hide() -> bool {
    #[cfg(windows)]
    {
        win::can_hide()
    }
    #[cfg(not(windows))]
    false
}

// --- Animation d'ouverture et de fermeture -----------------------------------------------------

/// Durée de l'animation (s).
const ANIM_S: f32 = 0.2;

/// Animation en cours : début, ouverture (sinon fermeture).
static ANIM: Mutex<Option<(std::time::Instant, bool)>> = Mutex::new(None);
/// Fermeture demandée par le raccourci (fil d'écoute) : animée à la prochaine image.
static CLOSE_REQ: AtomicBool = AtomicBool::new(false);

fn anim() -> std::sync::MutexGuard<'static, Option<(std::time::Instant, bool)>> {
    ANIM.lock().unwrap_or_else(|e| e.into_inner())
}

/// Fond Windows de la fenêtre déjà rendu transparent pour cette animation ?
static BACKDROP_DONE: AtomicBool = AtomicBool::new(false);

fn start_anim(opening: bool) {
    BACKDROP_DONE.store(false, Ordering::Relaxed);
    *anim() = Some((std::time::Instant::now(), opening));
}

/// Fermeture animée demandée hors du fil de l'interface (raccourci).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn request_close() {
    CLOSE_REQ.store(true, Ordering::Relaxed);
}

/// Fermeture animée en cours ?
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn closing() -> bool {
    CLOSE_REQ.load(Ordering::Relaxed) || anim().is_some_and(|(_, opening)| !opening)
}

/// La fenêtre vient de réapparaître : animation d'ouverture (annule une fermeture en cours).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn opened() {
    CLOSE_REQ.store(false, Ordering::Relaxed);
    start_anim(true);
}

/// Élastique en sortie : dépasse la cible puis s'y pose en oscillant.
fn elastic_out(p: f32) -> f32 {
    if p <= 0.0 {
        0.0
    } else if p >= 1.0 {
        1.0
    } else {
        2f32.powf(-10.0 * p) * ((10.0 * p - 0.75) * std::f32::consts::TAU / 3.0).sin() + 1.0
    }
}

/// Élan en entrée : recule un peu avant de partir.
fn back_in(p: f32) -> f32 {
    let c1 = 1.70158;
    (c1 + 1.0) * p * p * p - c1 * p * p
}

/// (échelle, opacité) de l'interface à l'avancement `p` (0 → 1).
fn frame_at(p: f32, opening: bool) -> (f32, f32) {
    if opening { (0.9 + 0.1 * elastic_out(p), (p / 0.4).min(1.0)) } else { (1.0 - 0.1 * back_in(p), (1.0 - p).powf(1.5)) }
}

/// En fin d'image (après tout le dessin) : applique l'animation à toute l'interface, et n'affiche
/// rien tant que la fenêtre est cachée (rien de périmé ne réapparaît à sa réouverture).
pub fn paint(ctx: &egui::Context) {
    let mut a = anim();
    let (scale, alpha) = match *a {
        Some((t0, opening)) => {
            let p = (t0.elapsed().as_secs_f32() / ANIM_S).min(1.0);
            if p >= 1.0 {
                *a = None;
                drop(a);
                if !opening {
                    hide();
                    clear(ctx);
                }
                return;
            }
            ctx.request_repaint();
            // Autour de l'interface réduite : transparent, pas le fond blanc de Windows. À
            // l'ouverture, à chaque image : Windows le repeint en blanc en réaffichant la fenêtre
            // (et en l'agrandissant).
            #[cfg(windows)]
            if opening || !BACKDROP_DONE.swap(true, Ordering::Relaxed) {
                win::clear_backdrop();
            }
            frame_at(p, opening)
        }
        None if hidden() => {
            drop(a);
            clear(ctx);
            return;
        }
        None => return,
    };
    drop(a);
    let c = ctx.content_rect().center().to_vec2();
    let t = egui::emath::TSTransform::from_translation(c) * egui::emath::TSTransform::from_scaling(scale) * egui::emath::TSTransform::from_translation(-c);
    for_each_layer(ctx, |list| {
        list.transform(t);
        for i in 0..list.next_idx().0 {
            list.mutate_shape(egui::layers::ShapeIdx(i), |cs| egui::epaint::shape_transform::adjust_colors(&mut cs.shape, move |col| *col = col.gamma_multiply(alpha)));
        }
    });
}

/// Rien à l'écran (fenêtre cachée).
fn clear(ctx: &egui::Context) {
    for_each_layer(ctx, |list| {
        for i in 0..list.next_idx().0 {
            list.reset_shape(egui::layers::ShapeIdx(i));
        }
    });
}

fn for_each_layer(ctx: &egui::Context, mut f: impl FnMut(&mut egui::layers::PaintList)) {
    let mut ids: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
    ids.push(egui::LayerId::background());
    ids.sort_by_key(|l| (l.order as u8, l.id.value()));
    ids.dedup();
    ctx.graphics_mut(|g| {
        for id in ids {
            if let Some(list) = g.get_mut(id) {
                f(list);
            }
        }
    });
}

fn hidden() -> bool {
    #[cfg(windows)]
    {
        win::is_hidden()
    }
    #[cfg(not(windows))]
    false
}

/// Cache la fenêtre (l'app continue) ; `false` si ce n'est pas possible sur ce système.
pub fn hide() -> bool {
    #[cfg(windows)]
    {
        win::hide()
    }
    #[cfg(not(windows))]
    false
}

/// Raccourci actif (0 : ouvrir / cacher, 1 : quitter) ? `None` : pas encore essayé ou sans objet.
pub fn hotkey_ok(i: usize) -> Option<bool> {
    #[cfg(windows)]
    {
        win::hotkey_ok(i)
    }
    #[cfg(not(windows))]
    {
        let _ = i;
        None
    }
}

/// Raccourci « quitter WorkTable » par défaut : Ctrl+Maj+K.
pub fn default_quit_key() -> Hotkey {
    Hotkey { ctrl: true, shift: true, alt: false, key: "K".into() }
}

/// Rouvre la fenêtre au premier plan ; `false` si ce n'est pas possible sur ce système.
pub fn show() -> bool {
    #[cfg(windows)]
    {
        win::show()
    }
    #[cfg(not(windows))]
    false
}

// --- Raccourci ----------------------------------------------------------------------------

/// Touches utilisables et leur code de touche virtuelle Windows.
const KEYS: &[(Key, u32)] = &[
    (Key::Space, 0x20),
    (Key::Enter, 0x0D),
    (Key::Tab, 0x09),
    (Key::Backspace, 0x08),
    (Key::Insert, 0x2D),
    (Key::Delete, 0x2E),
    (Key::Home, 0x24),
    (Key::End, 0x23),
    (Key::PageUp, 0x21),
    (Key::PageDown, 0x22),
    (Key::ArrowLeft, 0x25),
    (Key::ArrowUp, 0x26),
    (Key::ArrowRight, 0x27),
    (Key::ArrowDown, 0x28),
    (Key::Num0, 0x30),
    (Key::Num1, 0x31),
    (Key::Num2, 0x32),
    (Key::Num3, 0x33),
    (Key::Num4, 0x34),
    (Key::Num5, 0x35),
    (Key::Num6, 0x36),
    (Key::Num7, 0x37),
    (Key::Num8, 0x38),
    (Key::Num9, 0x39),
    (Key::A, 0x41),
    (Key::B, 0x42),
    (Key::C, 0x43),
    (Key::D, 0x44),
    (Key::E, 0x45),
    (Key::F, 0x46),
    (Key::G, 0x47),
    (Key::H, 0x48),
    (Key::I, 0x49),
    (Key::J, 0x4A),
    (Key::K, 0x4B),
    (Key::L, 0x4C),
    (Key::M, 0x4D),
    (Key::N, 0x4E),
    (Key::O, 0x4F),
    (Key::P, 0x50),
    (Key::Q, 0x51),
    (Key::R, 0x52),
    (Key::S, 0x53),
    (Key::T, 0x54),
    (Key::U, 0x55),
    (Key::V, 0x56),
    (Key::W, 0x57),
    (Key::X, 0x58),
    (Key::Y, 0x59),
    (Key::Z, 0x5A),
    (Key::F1, 0x70),
    (Key::F2, 0x71),
    (Key::F3, 0x72),
    (Key::F4, 0x73),
    (Key::F5, 0x74),
    (Key::F6, 0x75),
    (Key::F7, 0x76),
    (Key::F8, 0x77),
    (Key::F9, 0x78),
    (Key::F10, 0x79),
    (Key::F11, 0x7A),
    (Key::F12, 0x7B),
];

/// (modificateurs Windows `MOD_*`, touche virtuelle) ; `None` sans touche reconnue.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn code(h: &Hotkey) -> Option<(u32, u32)> {
    let key = Key::from_name(&h.key)?;
    let vk = KEYS.iter().find(|(k, _)| *k == key)?.1;
    // MOD_ALT 1, MOD_CONTROL 2, MOD_SHIFT 4.
    let mods = u32::from(h.alt) | u32::from(h.ctrl) << 1 | u32::from(h.shift) << 2;
    Some((mods, vk))
}

/// Raccourci tapé (événement clavier) → [`Hotkey`] ; il faut Ctrl ou Alt pour ne pas capturer
/// une touche ordinaire dans toutes les applications.
pub fn from_event(key: Key, m: egui::Modifiers) -> Option<Hotkey> {
    (KEYS.iter().any(|(k, _)| *k == key) && (m.ctrl || m.alt)).then(|| Hotkey { ctrl: m.ctrl, shift: m.shift, alt: m.alt, key: key.name().to_string() })
}

/// Saisie d'un raccourci dans les Réglages, lue directement au clavier : sous Windows, les
/// combinaisons avec Ctrl ou Alt n'arrivent pas toujours jusqu'à la fenêtre. `Some(None)` :
/// Échap (annuler). `start` : première image de la saisie (touches déjà enfoncées ignorées).
#[cfg_attr(not(windows), allow(unused_variables))]
pub fn poll_typed(start: bool) -> Option<Option<Hotkey>> {
    #[cfg(windows)]
    {
        const ESC: u32 = 0x1B;
        static PREV: Mutex<Vec<u32>> = Mutex::new(Vec::new());
        let down: Vec<u32> = KEYS.iter().map(|(_, vk)| *vk).chain([ESC]).filter(|vk| win::down(*vk)).collect();
        let mut prev = PREV.lock().unwrap_or_else(|e| e.into_inner());
        let fresh: Vec<u32> = if start { Vec::new() } else { down.iter().filter(|v| !prev.contains(v)).copied().collect() };
        *prev = down;
        if fresh.contains(&ESC) {
            return Some(None);
        }
        let key = fresh.iter().find_map(|vk| KEYS.iter().find(|(_, v)| v == vk).map(|(k, _)| *k))?;
        // VK_CONTROL, VK_SHIFT, VK_MENU (Alt).
        let m = egui::Modifiers { ctrl: win::down(0x11), shift: win::down(0x10), alt: win::down(0x12), ..Default::default() };
        from_event(key, m).map(Some)
    }
    #[cfg(not(windows))]
    None
}

/// « Ctrl + Maj + Espace ».
pub fn label(h: &Hotkey) -> String {
    let Some(key) = Key::from_name(&h.key) else {
        return t!("Aucun", "None").to_string();
    };
    let mut parts: Vec<String> = Vec::new();
    if h.ctrl {
        parts.push("Ctrl".into());
    }
    if h.alt {
        parts.push("Alt".into());
    }
    if h.shift {
        parts.push(t!("Maj", "Shift").into());
    }
    parts.push(match key {
        Key::Space => t!("Espace", "Space").to_string(),
        Key::Enter => t!("Entrée", "Enter").to_string(),
        k => k.symbol_or_name().to_string(),
    });
    parts.join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn animation_starts_and_ends_at_rest() {
        assert_eq!(frame_at(1.0, true), (1.0, 1.0));
        assert_eq!(frame_at(0.0, false), (1.0, 1.0));
        let (s, a) = frame_at(0.0, true);
        assert!(s < 1.0 && a == 0.0);
        // Élastique : dépasse la taille normale en chemin.
        assert!((1..10).any(|k| frame_at(k as f32 / 10.0, true).0 > 1.0));
        assert!(frame_at(0.99, false).1 < 0.01);
    }

    #[test]
    fn default_hotkey_is_ctrl_shift_space() {
        let h = Hotkey::default();
        assert_eq!(code(&h), Some((2 | 4, 0x20)));
        let m = egui::Modifiers { ctrl: true, shift: true, command: true, ..Default::default() };
        assert_eq!(from_event(Key::Space, m), Some(h));
        // Sans Ctrl ni Alt : refusé.
        assert_eq!(from_event(Key::K, egui::Modifiers::SHIFT), None);
        let k = from_event(Key::K, egui::Modifiers::ALT).unwrap();
        assert_eq!(code(&k), Some((1, 0x4B)));
    }
}
