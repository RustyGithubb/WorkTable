//! Notifications : nouveau mail, nouveau message Google Chat, document partagé sur Drive,
//! rendez-vous dans quelques minutes. Toutes passent par un bandeau animé en haut au centre de
//! l'écran, façon « succès débloqué » Xbox (apparition élastique, barre de durée : 10 s pour un
//! mail, 5 s pour le reste), avec le nombre de notifications du même type en attente.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::Local;
use eframe::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke};

use crate::google::calendar::CalEvent;
use crate::google::chat::{self, ChatEvent, ChatMessage};
use crate::google::drive::{self, DriveEvent, DriveFile, Source};
use crate::google::gmail::Summary;
use crate::google::{Account, Event};
use crate::prefs::Prefs;
use crate::tabs::calendar::CalendarState;
use crate::tabs::messages::ChatState;
use crate::theme;

/// Bandeau « succès » : apparition, durée affichée (barre), disparition.
const ACH_IN: f32 = 0.65;
/// Durée d'affichage (barre) : 10 s pour un mail, 5 s pour le reste.
const ACH_HOLD_MAIL: f32 = 10.0;
const ACH_HOLD: f32 = 5.0;
/// Hauteur de la rangée du bouton « Ouvrir » (mails).
const OPEN_ROW: f32 = 30.0;
/// Images de préparation de la fenêtre du bandeau avant de l'afficher.
const WARMUP_FRAMES: u32 = 3;
const ACH_OUT: f32 = 0.5;
/// Bandeaux en attente au maximum (les plus anciens sont abandonnés).
const ACH_QUEUE: usize = 9;
/// Relève des fichiers partagés avec moi sur Drive.
const SHARED_EVERY: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub enum Target {
    Mail(String),
    Space(String),
    Drive(DriveFile),
    Event(CalEvent),
}

/// Bandeau animé.
struct Achievement {
    /// Petite ligne du haut (« Nouveau mail · Alice »).
    head: String,
    /// Ligne principale (objet, texte du message).
    text: String,
    target: Target,
    /// Aperçu du contenu (mail), affiché sous le titre sur quelques lignes.
    body: Option<String>,
    start: Option<Instant>,
    /// Images déjà calculées : la fenêtre reste invisible le temps d'être prête (pas d'artefact).
    frames: u32,
    /// Son joué à l'apparition (`None` : sons coupés).
    sound: Option<crate::sound::Sound>,
}

pub struct Notifier {
    achievements: VecDeque<Achievement>,
    /// Bordure Windows de la fenêtre du bandeau déjà retirée.
    border_done: bool,
    /// Forme découpée de la fenêtre du bandeau (pixels), pour ne la mettre à jour qu'au besoin.
    shape: Option<[i32; 4]>,
    started_ms: i64,
    known_mail: HashSet<String>,
    /// Dernière activité connue de chaque conversation (None : premier passage).
    chat_seen: Option<HashMap<String, i64>>,
    /// Conversations dont on attend les nouveaux messages : depuis quand.
    chat_pending: HashMap<String, i64>,
    shared_seen: Option<HashSet<String>>,
    last_shared: Option<Instant>,
    events_done: HashSet<String>,
    last_events: Option<Instant>,
}

impl Default for Notifier {
    fn default() -> Self {
        Notifier {
            achievements: VecDeque::new(),
            border_done: false,
            shape: None,
            started_ms: Local::now().timestamp_millis(),
            known_mail: HashSet::new(),
            chat_seen: None,
            chat_pending: HashMap::new(),
            shared_seen: None,
            last_shared: None,
            events_done: HashSet::new(),
            last_events: None,
        }
    }
}

fn short(s: &str, n: usize) -> String {
    let s = s.replace(['\n', '\r'], " ");
    if s.chars().count() > n { s.chars().take(n).collect::<String>() + "…" } else { s }
}

impl Notifier {
    /// Ajoute un bandeau à la file (joué l'un après l'autre).
    fn push(&mut self, prefs: &Prefs, head: String, text: String, target: Target) {
        self.push_full(prefs, head, text, None, target);
    }

    fn push_full(&mut self, prefs: &Prefs, head: String, text: String, body: Option<String>, target: Target) {
        let sound = (!prefs.notif_no_sound).then_some(match target {
            Target::Mail(_) | Target::Event(_) => crate::sound::Sound::Mail,
            Target::Space(_) | Target::Drive(_) => crate::sound::Sound::Message,
        });
        self.achievements.push_back(Achievement { head, text, target, body, start: None, frames: 0, sound });
        while self.achievements.len() > ACH_QUEUE {
            // On garde celui en cours et les plus récents.
            self.achievements.remove(1);
        }
    }

    /// Mails reçus (chargement complet : `initial`, sans notification ; relève : nouveaux mails).
    pub fn on_mails(&mut self, list: &[Summary], prefs: &Prefs, initial: bool) {
        for m in list {
            let fresh = self.known_mail.insert(m.id.clone());
            let inbox = m.labels.iter().any(|l| l == "INBOX");
            if initial || !fresh || !inbox || !m.unread() || m.date < self.started_ms - 120_000 || prefs.notif_no_mail {
                continue;
            }
            let subject = if m.subject.is_empty() { "(sans objet)".to_string() } else { short(&m.subject, 120) };
            // Début du mail sous l'objet (aperçu fourni par Gmail).
            let body = Some(short(&m.snippet, 260)).filter(|b| !b.trim().is_empty());
            self.push_full(prefs, "Nouveau mail".into(), format!("{} - {subject}", m.sender_name()), body, Target::Mail(m.id.clone()));
        }
    }

    /// Liste des conversations mise à jour : celles qui ont bougé sont relues pour la notification.
    pub fn on_chat_spaces(&mut self, chat: &ChatState, acc: &Account, prefs: &Prefs) {
        let first = self.chat_seen.is_none();
        let seen = self.chat_seen.get_or_insert_with(HashMap::new);
        for sp in &chat.spaces {
            let prev = seen.insert(sp.name.clone(), sp.last_active);
            let Some(prev) = prev else { continue };
            if first || prefs.notif_no_chat || sp.last_active <= prev || sp.last_active < self.started_ms || chat.is_muted(&sp.name) {
                continue;
            }
            // Conversation ouverte à l'écran : pas de notification.
            if chat.open.as_deref() == Some(sp.name.as_str()) {
                continue;
            }
            self.chat_pending.insert(sp.name.clone(), prev);
            let name = sp.name.clone();
            acc.run(move |s| {
                let list = chat::messages(s, &name, Some(prev), None, 10).map(|r| r.0).unwrap_or_default();
                Ok(Event::Chat(ChatEvent::Messages { space: name, list, replace_from: None, older: false, more: false }))
            });
        }
    }

    /// Nouveaux messages d'une conversation attendue : notification du dernier (pas les miens).
    pub fn on_chat_messages(&mut self, space: &str, list: &[ChatMessage], chat: &ChatState, prefs: &Prefs) {
        let Some(since) = self.chat_pending.remove(space) else { return };
        let me = chat.me.as_deref();
        let new: Vec<&ChatMessage> = list.iter().filter(|m| m.created > since && Some(m.sender.as_str()) != me && !m.text.trim().is_empty()).collect();
        let Some(last) = new.last() else { return };
        let title = chat.space(space).map(|s| chat.title(s)).unwrap_or_else(|| "Google Chat".into());
        let who = chat.name_of(&last.sender);
        let more = if new.len() > 1 { format!(" (+{})", new.len() - 1) } else { String::new() };
        let head = if title == who { "Nouveau message".to_string() } else { format!("Nouveau message · {title}") };
        self.push(prefs, head, format!("{who} - {}{more}", short(&last.text, 120)), Target::Space(space.to_string()));
    }

    /// Fichiers « Partagés avec moi » : notification des nouveaux.
    pub fn on_shared(&mut self, files: &[DriveFile], prefs: &Prefs) {
        let first = self.shared_seen.is_none();
        let seen = self.shared_seen.get_or_insert_with(HashSet::new);
        let mut fresh = Vec::new();
        for f in files {
            if seen.insert(f.id.clone()) && !first && !f.is_folder() {
                fresh.push(f.clone());
            }
        }
        if prefs.notif_no_share {
            return;
        }
        for f in fresh.into_iter().take(3) {
            let who = if f.owner.is_empty() { "Quelqu'un".to_string() } else { f.owner.clone() };
            self.push(prefs, "Document partagé".into(), format!("{who} - {} ({})", f.name, drive::kind_label(&f)), Target::Drive(f));
        }
    }

    /// À chaque image : relève Drive, rendez-vous proches, expiration des notifications.
    pub fn background(&mut self, ctx: &egui::Context, acc: &Account, cal: &CalendarState, prefs: &Prefs) {
        if acc.session().is_some() && !prefs.notif_no_share && self.last_shared.is_none_or(|t| t.elapsed() >= SHARED_EVERY) {
            self.last_shared = Some(Instant::now());
            acc.run(|s| {
                Ok(match drive::list(s, &Source::Shared, None) {
                    Ok((files, next)) => Event::Drive(DriveEvent::Listing { source: Source::Shared, files, next, append: false }),
                    Err(_) => Event::Chat(ChatEvent::Noop),
                })
            });
        }
        if !prefs.notif_no_event && self.last_events.is_none_or(|t| t.elapsed() >= Duration::from_secs(15)) {
            self.last_events = Some(Instant::now());
            let now = Local::now().timestamp_millis();
            let mins = if prefs.notif_event_minutes == 0 { 10 } else { prefs.notif_event_minutes.min(120) } as i64;
            let evs: Vec<CalEvent> = cal.events_between(prefs, now, now + mins * 60_000).into_iter().filter(|e| !e.all_day && e.start > now).cloned().collect();
            for e in evs {
                if self.events_done.insert(format!("{}/{}/{}", e.calendar, e.id, e.start)) {
                    let left = ((e.start - now) / 60_000).max(1);
                    let title = if e.title.is_empty() { "(sans titre)".to_string() } else { e.title.clone() };
                    let place = if e.location.is_empty() { String::new() } else { format!(" · {}", short(&e.location, 50)) };
                    let when = crate::tabs::calendar::hhmm(e.start);
                    self.push(prefs, format!("Rendez-vous dans {left} min"), format!("{title} - {when}{place}"), Target::Event(e));
                }
            }
            ctx.request_repaint_after(Duration::from_secs(15));
        }
        if !self.achievements.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    /// Bandeau de notification ; renvoie la cible cliquée.
    pub fn show(&mut self, ctx: &egui::Context, window: Rect) -> Option<Target> {
        self.achievement(ctx, window)
    }

    /// Bandeau « succès débloqué » en haut au centre de l'écran, dans une petite fenêtre
    /// transparente toujours au premier plan (par-dessus les autres applications).
    /// Renvoie la cible s'il est cliqué.
    fn achievement(&mut self, ctx: &egui::Context, window: Rect) -> Option<Target> {
        // Notifications du même type en attente (celle affichée comprise).
        let group = std::mem::discriminant(&self.achievements.front()?.target);
        let pending = self.achievements.iter().filter(|x| std::mem::discriminant(&x.target) == group).count();
        let a = self.achievements.front_mut()?;
        // Les premières images servent à créer et préparer la fenêtre (fond, découpe) sans
        // l'afficher ; l'animation ne démarre qu'ensuite.
        a.frames = a.frames.saturating_add(1);
        let ready = a.frames > WARMUP_FRAMES;
        let fresh = a.frames <= WARMUP_FRAMES + 3;
        let t = if ready {
            let start = *a.start.get_or_insert_with(|| {
                // Apparition du bandeau : petit son (différent pour un mail et un message).
                if let Some(s) = a.sound {
                    crate::sound::play(s);
                }
                Instant::now()
            });
            start.elapsed().as_secs_f32()
        } else {
            0.0
        };
        let hold = if matches!(a.target, Target::Mail(_)) { ACH_HOLD_MAIL } else { ACH_HOLD };
        let total = ACH_IN + hold + ACH_OUT;
        if t >= total {
            self.achievements.pop_front();
            if self.achievements.is_empty() {
                // La fenêtre se ferme : la suivante devra être retouchée à nouveau.
                self.border_done = false;
                self.shape = None;
            }
            ctx.request_repaint();
            return None;
        }
        ctx.request_repaint();
        let k = |a: f32, d: f32| ((t - a) / d).clamp(0.0, 1.0);
        let out_t = ACH_IN + hold;
        let anim = Anim {
            // Bandeau entier : apparaît de l'échelle 0 à 1 avec un rebond élastique, puis se
            // rétracte à la fin.
            scale: ease_out_elastic(k(0.0, ACH_IN)) * (1.0 - ease_in(k(total - 0.3, 0.3))),
            circle: 1.0,
            text: k(0.08, 0.25) * (1.0 - k(total - 0.35, 0.2)),
            bar: 1.0 - k(ACH_IN, hold),
            drop: 0.0,
            // L'icône de l'onglet joue son animation de clic à l'arrivée.
            icon_phase: k(0.15, 0.6),
            clickable: t < out_t,
            pending,
        };
        // Largeur finale d'après les textes.
        let (head_w, text_w) = ctx.fonts_mut(|f| {
            let mut w = |s: &str, font: egui::FontId| f.layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x;
            (w(&a.head, ach_font_title()), w(&a.text, ach_font()))
        });
        // Avec un aperçu du contenu : bandeau à la largeur maximale, plus haut pour ses lignes.
        let widest = if a.body.is_some() { ACH_TEXT_MAX } else { head_w.max(text_w).min(ACH_TEXT_MAX) };
        let full = (ACH_H / 2.0 + ACH_D / 2.0 + 16.0 + widest + 30.0).clamp(320.0, ACH_H / 2.0 + ACH_D / 2.0 + 16.0 + ACH_TEXT_MAX + 30.0);
        let mut body_h = match &a.body {
            Some(b) => {
                let g = ctx.fonts_mut(|f| f.layout_job(body_job(b, full, Color32::WHITE)));
                g.size().y + 4.0
            }
            None => 0.0,
        };
        // Mail : une rangée en bas pour le bouton « Ouvrir » (et la barre de durée à côté).
        if matches!(a.target, Target::Mail(_)) {
            body_h += OPEN_ROW;
        }

        let clicked = if ctx.embed_viewports() {
            // Pas de fenêtres multiples (certains systèmes) : en haut au centre de l'app.
            let cx = ctx.content_rect().center().x;
            let pos = Pos2::new(cx - full / 2.0 - ACH_MARGIN, window.top() + window.height() * ACH_TOP);
            let mut clicked = false;
            egui::Area::new(egui::Id::unique("wt_achievement")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
                clicked = scaled(ui, &anim, |ui| paint_achievement(ui, pos, full, body_h, a, &anim)).0;
            });
            clicked
        } else {
            // Écran où se trouve l'app (écrans côte à côte), en haut au centre.
            let (monitor, outer) = ctx.input(|i| (i.viewport().monitor_size, i.viewport().outer_rect));
            let mw = monitor.map(|m| m.x).filter(|w| *w > 0.0).unwrap_or(1920.0);
            let mh = monitor.map(|m| m.y).filter(|h| *h > 0.0).unwrap_or(1080.0);
            let left = outer.map(|r| (r.center().x / mw).floor() * mw).unwrap_or(0.0);
            // Place en plus autour pour le rebond élastique de l'apparition (dépassement de taille).
            let base = egui::vec2(full + 2.0 * ACH_MARGIN, ACH_H + body_h + 2.0 * ACH_MARGIN);
            let extra = base * (ACH_OVERSHOOT - 1.0);
            let size = base + extra;
            let builder = egui::ViewportBuilder::default()
                .with_title(ACH_TITLE)
                .with_decorations(false)
                .with_transparent(true)
                .with_has_shadow(false)
                .with_resizable(false)
                .with_taskbar(false)
                .with_active(false)
                .with_window_level(egui::WindowLevel::AlwaysOnTop)
                // Invisible pendant la préparation et à la toute fin (rien de résiduel à l'écran).
                .with_visible(ready && t < total - 0.03)
                .with_inner_size(size)
                .with_position(Pos2::new(left + ((mw - size.x) / 2.0).round(), (mh * ACH_TOP - extra.y / 2.0).round()));
            let (clicked, shape) = ctx.show_viewport_immediate(egui::ViewportId::from_hash_of("wt_achievement"), builder, |ui, _| {
                let origin = (extra / 2.0).to_pos2();
                let (clicked, shape) = scaled(ui, &anim, |ui| paint_achievement(ui, origin, full, body_h, a, &anim));
                crate::emoji_img::colorize_frame(ui.ctx());
                // Forme de la capsule en pixels de l'écran.
                let ppp = ui.ctx().pixels_per_point();
                let px = |r: Rect| [(r.left() * ppp).floor() as i32, (r.top() * ppp).floor() as i32, (r.right() * ppp).ceil() as i32, (r.bottom() * ppp).ceil() as i32];
                // Fenêtre transparente : on n'en garde que le bandeau et son ombre (les bords de la
                // fenêtre, où Windows peut tracer un liseré, sont retirés).
                let outer = Rect::from_min_size(Pos2::ZERO, size).shrink(3.0);
                (clicked, (px(shape), px(outer), ((ACH_RADIUS + ACH_MARGIN - 3.0) * 2.0 * ppp) as i32, (ACH_RADIUS * 2.0 * ppp) as i32))
            });
            let (shape, outer, outer_d, shape_d) = shape;
            if !self.border_done {
                self.border_done = strip_border(ACH_TITLE);
            }
            no_activate(ACH_TITLE);
            // La fenêtre est découpée à la forme exacte de la capsule : rien autour ne peut
            // apparaître, même si Windows n'applique pas la transparence.
            if CLIP_SHAPE.load(std::sync::atomic::Ordering::Relaxed) {
                if self.shape != Some(shape) && shape_window(ACH_TITLE, shape, shape_d) {
                    self.shape = Some(shape);
                }
            } else {
                // Seulement quand la fenêtre vient d'être créée ou change de taille (pas à chaque image).
                if fresh || self.shape != Some(outer) {
                    clear_backdrop(ACH_TITLE);
                }
                if self.shape != Some(outer) && shape_window(ACH_TITLE, outer, outer_d) {
                    self.shape = Some(outer);
                }
            }
            clicked
        };
        if clicked {
            // Ramène l'app au premier plan.
            if !bring_root_to_front() {
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            let target = a.target.clone();
            // Clic : on passe directement à la disparition.
            a.start = Some(Instant::now() - Duration::from_secs_f32(out_t));
            return Some(target);
        }
        None
    }
}

/// Découper la fenêtre du bandeau à sa forme (moteur sans vraie transparence : OpenGL).
static CLIP_SHAPE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_clip_shape(on: bool) {
    CLIP_SHAPE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Fenêtre principale de l'app (identifiant Windows), enregistrée au démarrage.
static ROOT_WINDOW: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

pub fn set_root_window(hwnd: isize) {
    ROOT_WINDOW.store(hwnd, std::sync::atomic::Ordering::Relaxed);
}

/// Remet la fenêtre principale au premier plan (renvoie `false` si c'est impossible ici).
///
/// La commande `Focus` d'egui simule un appui sur Alt pour forcer le passage au premier plan :
/// Windows ouvre alors le « mode menu » de la fenêtre et y dessine un instant sa barre de titre
/// (cachée). Le clic sur le bandeau vient d'être reçu par l'app, Windows l'autorise donc à
/// passer au premier plan directement, sans cette simulation.
#[cfg(windows)]
fn bring_root_to_front() -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow};
    let hwnd = ROOT_WINDOW.load(std::sync::atomic::Ordering::Relaxed);
    if hwnd == 0 {
        return false;
    }
    let hwnd = hwnd as windows_sys::Win32::Foundation::HWND;
    // SAFETY : fenêtre principale de ce processus, qui existe tant que l'app tourne.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd);
    }
    true
}

#[cfg(not(windows))]
fn bring_root_to_front() -> bool {
    false
}

/// Titre (invisible) de la fenêtre du bandeau, qui sert à la retrouver sous Windows.
const ACH_TITLE: &str = "WorkTable — notification";

/// Windows 11 dessine un liseré clair et arrondit les coins des fenêtres sans cadre :
/// on retire l'un et l'autre (renvoie `false` tant que la fenêtre n'existe pas encore).
#[cfg(windows)]
fn strip_border(title: &str) -> bool {
    use windows_sys::Win32::Graphics::Dwm::{DWMWA_BORDER_COLOR, DWMWA_COLOR_NONE, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DwmSetWindowAttribute};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY : appels Win32 sur une fenêtre de ce processus, avec des pointeurs valides
    // le temps de l'appel.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        let none: u32 = DWMWA_COLOR_NONE;
        DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR as u32, (&none as *const u32).cast(), 4);
        let corner: i32 = DWMWCP_DONOTROUND;
        DwmSetWindowAttribute(hwnd, DWMWA_WINDOW_CORNER_PREFERENCE as u32, (&corner as *const i32).cast(), 4);
    }
    true
}

#[cfg(not(windows))]
fn strip_border(_title: &str) -> bool {
    true
}

/// Le bandeau ne devient jamais la fenêtre active : sinon, au clic sur « Ouvrir », Windows
/// dessinait sa barre de titre (cachée mais toujours présente, winit la garde aux fenêtres sans
/// cadre) dans la marge transparente au-dessus du bandeau. L'apparence n'est pas modifiée.
#[cfg(windows)]
fn no_activate(title: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY : fenêtre de ce processus, pointeurs valides le temps des appels.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if !hwnd.is_null() {
            hook_window(hwnd);
        }
    }
}

/// Procédure d'origine des fenêtres winit, et fenêtre du bandeau déjà interceptée.
#[cfg(all(windows, target_pointer_width = "64"))]
static OLD_PROC: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
#[cfg(all(windows, target_pointer_width = "64"))]
static HOOKED: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// Intercepte le message qui rendrait la fenêtre du bandeau active au clic (le clic lui-même
/// est bien reçu) ; tout le reste va à winit.
#[cfg(all(windows, target_pointer_width = "64"))]
unsafe fn hook_window(hwnd: windows_sys::Win32::Foundation::HWND) {
    use std::sync::atomic::Ordering::SeqCst;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, GWLP_WNDPROC, GetWindowLongPtrW, MA_NOACTIVATE, SetWindowLongPtrW, WM_MOUSEACTIVATE, WM_NCACTIVATE, WNDPROC,
    };

    unsafe extern "system" fn banner_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        // SAFETY : `OLD_PROC` est la procédure d'origine de la fenêtre (même taille qu'un pointeur
        // de fonction, 0 = aucune).
        unsafe {
            let old: WNDPROC = std::mem::transmute::<isize, WNDPROC>(OLD_PROC.load(SeqCst));
            if old.is_none() {
                return DefWindowProcW(hwnd, msg, wp, lp);
            }
            match msg {
                // Un clic ne rend pas le bandeau actif.
                WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
                // Par sécurité, si la fenêtre change d'état malgré tout : sans redessiner le cadre.
                WM_NCACTIVATE => CallWindowProcW(old, hwnd, msg, wp, -1),
                _ => CallWindowProcW(old, hwnd, msg, wp, lp),
            }
        }
    }

    if HOOKED.load(SeqCst) == hwnd as isize {
        return;
    }
    let f: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT = banner_proc;
    let ours = f as usize as isize;
    // SAFETY : fenêtre de ce processus ; la procédure d'origine est gardée avant le changement.
    unsafe {
        let current = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        if current != ours {
            OLD_PROC.store(current, SeqCst);
            SetWindowLongPtrW(hwnd, GWLP_WNDPROC, ours);
        }
    }
    HOOKED.store(hwnd as isize, SeqCst);
}

#[cfg(all(windows, not(target_pointer_width = "64")))]
unsafe fn hook_window(_hwnd: windows_sys::Win32::Foundation::HWND) {}

#[cfg(not(windows))]
fn no_activate(_title: &str) {}

/// Découpe la fenêtre du bandeau en capsule (rectangle aux bouts arrondis), en pixels.
#[cfg(windows)]
fn shape_window(title: &str, [x0, y0, x1, y1]: [i32; 4], round: i32) -> bool {
    use windows_sys::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY : fenêtre de ce processus ; la région appartient ensuite à Windows (pas de libération).
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            return false;
        }
        let d = round.min(y1 - y0).max(1);
        let rgn = CreateRoundRectRgn(x0, y0, x1 + 1, y1 + 1, d, d);
        if rgn.is_null() {
            return false;
        }
        SetWindowRgn(hwnd, rgn, 1) != 0
    }
}

/// Fond Windows de la fenêtre du bandeau (sous l'image affichée par DirectX) rempli de noir
/// transparent : sans cela il apparaît en blanc autour de la capsule.
#[cfg(windows)]
fn clear_backdrop(title: &str) {
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{BLACK_BRUSH, FillRect, GetDC, GetStockObject, ReleaseDC};
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, GetClientRect};
    let wide: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    // SAFETY : fenêtre de ce processus ; le contexte de dessin est rendu aussitôt.
    unsafe {
        let hwnd = FindWindowW(std::ptr::null(), wide.as_ptr());
        if hwnd.is_null() {
            return;
        }
        let mut rc = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        if GetClientRect(hwnd, &mut rc) == 0 {
            return;
        }
        let dc = GetDC(hwnd);
        if dc.is_null() {
            return;
        }
        FillRect(dc, &rc, GetStockObject(BLACK_BRUSH));
        ReleaseDC(hwnd, dc);
    }
}

#[cfg(not(windows))]
fn clear_backdrop(_title: &str) {}

#[cfg(not(windows))]
fn shape_window(_title: &str, _shape: [i32; 4], _round: i32) -> bool {
    true
}

/// Hauteur de la capsule, diamètre de la pastille (centrée dans le bout gauche, avec une marge), marge autour (ombre), largeur maximale du texte.
const ACH_H: f32 = 58.0;
/// Arrondi des coins du bandeau.
const ACH_RADIUS: f32 = 10.0;
/// Distance au haut de l'écran (part de sa hauteur).
const ACH_TOP: f32 = 0.05;
/// Taille maximale atteinte pendant le rebond d'apparition (marge prévue dans la fenêtre).
const ACH_OVERSHOOT: f32 = 1.16;
const ACH_D: f32 = 38.0;
const ACH_MARGIN: f32 = 22.0;
const ACH_TEXT_MAX: f32 = 400.0;

fn ach_font() -> egui::FontId {
    egui::FontId::new(14.0, egui::FontFamily::Name(theme::ACHIEVEMENT_FONT.into()))
}

/// Titre du bandeau : même police que les titres d'onglets.
fn ach_font_title() -> egui::FontId {
    theme::title_font(15.0)
}

/// État de l'animation du bandeau à cet instant (0 → 1).
struct Anim {
    circle: f32,
    text: f32,
    bar: f32,
    drop: f32,
    clickable: bool,
    /// Échelle du bandeau entier (apparition élastique).
    scale: f32,
    /// Avancement de l'animation de l'icône d'onglet (1 = au repos).
    icon_phase: f32,
    /// Nombre de notifications du même type en attente (pastille si plus d'une).
    pending: usize,
}

/// Dessine le bandeau (pastille + capsule façon succès Xbox 360) à partir de `origin`.
/// Renvoie le clic et la forme visible (capsule et pastille) pour découper la fenêtre.
fn paint_achievement(ui: &mut egui::Ui, origin: Pos2, full: f32, extra: f32, a: &Achievement, an: &Anim) -> (bool, Rect) {
    let area = Rect::from_min_size(origin, egui::vec2(full + 2.0 * ACH_MARGIN, ACH_H + extra + 2.0 * ACH_MARGIN));
    // Centre de l'icône, dans la bande du haut (titre et détail à sa droite).
    let c = Pos2::new(origin.x + ACH_MARGIN + ACH_H / 2.0, origin.y + ACH_MARGIN + ACH_H / 2.0 - an.drop * 50.0);
    let h = ACH_H + extra;
    let r = Rect::from_min_size(Pos2::new(c.x - ACH_H / 2.0, c.y - ACH_H / 2.0), egui::vec2(full, h));
    ui.allocate_rect(area, Sense::hover());
    let hit = ui.interact(r.union(Rect::from_center_size(c, egui::vec2(ACH_D, ACH_D))), egui::Id::unique("wt_achievement_hit"), Sense::click());
    let hit = hit.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter().with_clip_rect(area);
    let mut open_clicked = false;

    if h > 1.0 {
        // Rectangle aux coins arrondis, comme une fenêtre Windows.
        let round = CornerRadius::same(ACH_RADIUS as u8);
        // Ombre portée douce, sous la capsule.
        // Ombre qui épouse le bandeau : même forme et mêmes coins, légèrement décalée vers le bas,
        // et assez courte pour tenir dans la marge de la fenêtre (sinon elle serait coupée net).
        p.add(egui::Shadow { offset: [0, 3], blur: 12, spread: 1, color: Color32::from_black_alpha(150) }.as_shape(r, round));
        // Capsule unie, aux couleurs des panneaux de l'app (fond et liseré du thème).
        // Mêmes fond et liseré que la barre de navigation de gauche.
        p.rect_filled(r, round, theme::SIDEBAR_BG());
        p.rect_stroke(r, round, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        // Texte blanc sur deux lignes, coupé par la capsule pendant qu'elle se déploie.
        let job = |s: &str, font: egui::FontId, color: Color32| {
            let mut j = egui::text::LayoutJob::simple_singleline(s.to_string(), font, color.gamma_multiply(an.text));
            j.wrap = egui::text::TextWrapping { max_width: ACH_TEXT_MAX, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
            j
        };
        // Titre en gras, détail en regular (comme les succès Xbox One).
        let head = p.layout_job(job(&a.head, ach_font_title(), theme::TEXT()));
        // Détail en gris, comme les icônes non sélectionnées de la barre.
        let text = p.layout_job(job(&a.text, ach_font(), theme::TEXT_MUTED()));
        let clip = p.with_clip_rect(r.shrink(2.0).intersect(area));
        let x = c.x + ACH_D / 2.0 + 16.0;
        clip.galley(Pos2::new(x, c.y - 19.0), head, Color32::WHITE);
        clip.galley(Pos2::new(x, c.y - 1.0), text, Color32::WHITE);
        // Aperçu du contenu du mail, sous le titre.
        if let Some(b) = &a.body {
            let g = p.layout_job(body_job(b, full, theme::TEXT_FAINT().gamma_multiply(an.text)));
            clip.galley(Pos2::new(x, c.y + 20.0), g, Color32::WHITE);
        }
        // Mail : bouton « Ouvrir » en bas à droite, dans une pilule aux couleurs de la sélection
        // de la barre de gauche ; la barre de durée s'arrête avant lui, à sa hauteur.
        let mut bar_right = c.x - ACH_H / 2.0 + full - 28.0;
        let mut bar_y = r.bottom() - 6.0;
        if matches!(a.target, Target::Mail(_)) {
            let label = p.layout_no_wrap("Ouvrir".into(), egui::FontId::new(13.0, egui::FontFamily::Name(theme::ACHIEVEMENT_FONT_BOLD.into())), theme::BG());
            let size = egui::vec2(label.size().x + 28.0, 24.0);
            let pill = Rect::from_min_size(Pos2::new(r.right() - 12.0 - size.x, r.bottom() - 10.0 - size.y), size);
            let resp = ui.interact(pill, egui::Id::unique("wt_achievement_open"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            // Survol : le bouton grossit légèrement et sa couleur glisse en douceur (interpolation).
            let k = ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique("wt_achievement_open_hover"), resp.hovered(), 0.16, egui::emath::easing::cubic_out);
            let fill = crate::sidebar::lerp_color(theme::TEXT(), theme::TEXT_MUTED(), k);
            let grown = Rect::from_center_size(pill.center(), pill.size() * (1.0 + 0.08 * k));
            clip.rect_filled(grown, CornerRadius::same((grown.height() / 2.0) as u8), fill.gamma_multiply(an.text));
            let label = if k > 0.001 {
                p.layout_no_wrap("Ouvrir".into(), egui::FontId::new(13.0 * (1.0 + 0.08 * k), egui::FontFamily::Name(theme::ACHIEVEMENT_FONT_BOLD.into())), theme::BG())
            } else {
                label
            };
            clip.galley(grown.center() - label.size() / 2.0, label, theme::BG());
            open_clicked = resp.clicked();
            bar_right = pill.left() - 14.0;
            bar_y = pill.center().y + 1.0;
        }
        // Durée restante : fine barre (même couleur que le fond des icônes de la barre de gauche).
        let track = Rect::from_min_max(Pos2::new(x, bar_y - 1.0), Pos2::new(bar_right, bar_y + 1.0));
        clip.rect_filled(track, CornerRadius::same(1), theme::BORDER().gamma_multiply(an.text));
        let fill = Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * an.bar, track.bottom()));
        clip.rect_filled(fill, CornerRadius::same(1), theme::TEXT().gamma_multiply(an.text));
    }

    // Pastille pleine aux couleurs de la sélection de la barre de gauche (fond = couleur du
    // texte du thème, pictogramme = couleur du fond) : suit le thème choisi dans les réglages.
    let rad = ACH_D / 2.0 * an.circle;
    if rad > 0.5 {
        // Icône seule, sans rond derrière, dans la couleur du texte.
        let ink = theme::TEXT();
        // Exactement l'icône de l'onglet d'où vient la notification (comme dans la barre de gauche).
        let tab = match a.target {
            Target::Mail(_) => crate::app::Tab::Mail,
            Target::Space(_) => crate::app::Tab::Messages,
            Target::Drive(_) => crate::app::Tab::Files,
            Target::Event(_) => crate::app::Tab::Calendar,
        };
        let icon = Rect::from_center_size(c, egui::vec2(rad * 1.3, rad * 1.3));
        crate::sidebar::draw_icon(&p, icon, tab, ink, an.icon_phase);
        // Pastille du nombre de notifications du même type en attente (comme dans la barre).
        if an.pending > 1 && an.circle > 0.9 {
            let n = if an.pending > 9 { "9+".to_string() } else { an.pending.to_string() };
            let g = p.layout_no_wrap(n, egui::FontId::new(10.5, egui::FontFamily::Name(theme::ACHIEVEMENT_FONT_BOLD.into())), Color32::WHITE);
            let w = (g.size().x + 9.0).max(17.0);
            let b = Rect::from_center_size(c + egui::vec2(rad * 0.95, -rad * 0.85), egui::vec2(w, 17.0));
            p.rect(b, CornerRadius::same(10), theme::TODAY(), Stroke::new(2.0, theme::SIDEBAR_BG()), egui::StrokeKind::Outside);
            p.galley(b.center() - g.size() / 2.0, g, Color32::WHITE);
        }
    }
    let shape = r.union(Rect::from_center_size(c, egui::vec2(2.0 * rad.max(0.0), 2.0 * rad.max(0.0)))).intersect(area);
    ((hit.clicked() || open_clicked) && an.clickable, shape)
}

/// Aperçu du contenu : 3 lignes au plus, à la largeur du texte du bandeau.
fn body_job(text: &str, full: f32, color: Color32) -> egui::text::LayoutJob {
    let width = full - (ACH_H / 2.0 + ACH_D / 2.0 + 16.0) - 28.0;
    let mut j = egui::text::LayoutJob::simple(text.to_string(), egui::FontId::new(13.0, egui::FontFamily::Name(theme::ACHIEVEMENT_FONT.into())), color, width);
    j.wrap = egui::text::TextWrapping { max_width: width, max_rows: 3, break_anywhere: false, overflow_character: Some('…') };
    j
}

/// Mise à l'échelle de tout ce que dessine `f` autour du centre de la zone (apparition).
fn scaled<R>(ui: &mut egui::Ui, an: &Anim, f: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let center = ui.max_rect().center();
    let ctx = ui.ctx().clone();
    let inner = egui::Area::new(egui::Id::unique("wt_achievement_layer")).order(egui::Order::Foreground).fixed_pos(ui.max_rect().min).constrain(false).show(&ctx, |ui| {
        ui.set_min_size(ui.max_rect().size());
        f(ui)
    });
    let k = an.scale.max(0.001);
    let t = egui::emath::TSTransform::from_translation(center.to_vec2() * (1.0 - k)) * egui::emath::TSTransform::from_scaling(k);
    ctx.set_transform_layer(inner.response.layer_id, t);
    inner.inner
}

/// Rebond élastique (dépasse puis se stabilise).
fn ease_out_elastic(x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    // Dépasse d'environ 12 %, revient un peu en dessous, puis se pose.
    1.0 - (-8.0 * x).exp() * (12.0 * x).cos()
}

fn ease_in(x: f32) -> f32 {
    x * x
}
