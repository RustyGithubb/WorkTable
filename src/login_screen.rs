//! Écran de connexion, à chaque lancement, avant que rien ne soit chargé (fond uni).
//!
//! - Compte retenu : image et nom du compte, un clic sur « Se connecter ».
//! - Compte non retenu : le mot de passe est redemandé, vérifié sur ce poste (empreinte Argon2id,
//!   jamais le mot de passe), même hors ligne.
//! - Pas de compte : formulaire de connexion du NAS (premier login avec code d'invitation), ou
//!   « Continuer sans compte ».
//!
//! Ensuite seulement : synchronisation de démarrage, compte Google, chargement de l'app. Le tableau
//! de bord apparaît par un fondu flou (capture floutée de la première image, qui s'efface).
//!
//! C'est un verrou d'interface : les fichiers de ce poste ne sont pas chiffrés par le mot de passe.

use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::App;
use eframe::egui::{self, Color32, ColorImage, CornerRadius, Key, Pos2, Rect, RichText, Sense, TextureHandle, vec2};

use crate::app::WorkTableApp;
use crate::theme;

/// Rayon de l'image du compte.
const AVATAR_R: f32 = 52.0;
/// Durée du fondu flou qui révèle l'app.
const REVEAL_S: f32 = 0.7;
/// Voile posé sur la première image de l'app (capturée puis floutée).
const VEIL: f32 = 0.5;

enum Stage {
    Login,
    /// Synchronisation de démarrage et compte Google, dans un fil à part.
    Opening(Receiver<()>),
    /// L'app est chargée : fondu flou par-dessus.
    Reveal,
    Done,
}

pub struct Gate {
    app: Option<WorkTableApp>,
    stage: Stage,
    remember: bool,
    password: String,
    error: Option<String>,
    /// Vérification locale du mot de passe en cours (Argon2id, quelques dixièmes de seconde).
    checking: Option<Receiver<Option<bool>>>,
    /// Connexion au serveur en cours (pas d'empreinte locale, ou session à refaire).
    online: bool,
    /// « Autre compte » : formulaire de connexion complet ; connexion lancée depuis ce formulaire.
    other: bool,
    form_watch: bool,
    avatar: Option<TextureHandle>,
    // Fondu flou.
    shot: Arc<Mutex<Option<ColorImage>>>,
    shot_asked: Option<Instant>,
    reveal_tex: Option<TextureHandle>,
    reveal_start: Option<Instant>,
}

impl Gate {
    pub fn new(ctx: &egui::Context) -> Self {
        // Apparence choisie (thème, langue, polices) dès l'écran de connexion.
        let mut prefs = crate::prefs::Prefs::load();
        if prefs.theme == theme::ThemeId::Retired {
            prefs.theme = theme::ThemeId::Monochrome;
        }
        crate::i18n::set(prefs.language);
        theme::set_fonts(ctx, &prefs.fonts);
        theme::set_theme(ctx, prefs.theme);
        crate::sync::set_ctx(ctx);
        let account = crate::sync::Account::load();
        let mut gate = Gate {
            app: None,
            stage: Stage::Login,
            remember: account.as_ref().is_none_or(|a| !a.forget),
            password: String::new(),
            error: None,
            checking: None,
            online: false,
            other: false,
            form_watch: false,
            avatar: load_avatar(ctx),
            shot: Default::default(),
            shot_asked: None,
            reveal_tex: None,
            reveal_start: None,
        };
        // Redémarrage juste après une connexion réussie (autre compte, sans compte) : on ouvre.
        if crate::sync::take_unlock_marker() {
            gate.open(true);
        }
        gate
    }

    /// Connexion acceptée : synchronisation de démarrage (sauf si la connexion au serveur vient de
    /// la faire) et compte Google, puis chargement.
    fn open(&mut self, startup: bool) {
        self.password.clear();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            // Changements des autres postes appliqués avant le chargement des données.
            if startup {
                crate::sync::startup();
            }
            // Compte Google rattaché au compte NAS : repris d'un autre poste s'il manque ici.
            crate::google::account_sync::import_then_export();
            let _ = tx.send(());
        });
        self.stage = Stage::Opening(rx);
    }

    fn login_ui(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let account = crate::sync::Account::load();
        let configured = crate::sync::configured();
        let st = crate::sync::status();
        // Choix « retenir » repris par toute connexion faite depuis cet écran (formulaire compris).
        crate::sync::set_remember_new(self.remember);

        // Résultat de la vérification locale (choix « retenir » déjà enregistré par le fil).
        if let Some(rx) = &self.checking
            && let Ok(ok) = rx.try_recv()
        {
            self.checking = None;
            match ok {
                Some(true) => {
                    self.open(true);
                    return;
                }
                Some(false) => self.error = Some(t!("Mot de passe incorrect.", "Wrong password.").into()),
                // Pas d'empreinte sur ce poste : vérification par le serveur.
                None => self.connect_online(account.as_ref()),
            }
        }
        // Connexion au serveur terminée.
        if self.online && !st.connecting {
            self.online = false;
            match &st.connect_error {
                Some(e) => self.error = Some(e.clone()),
                // La connexion a déjà fait la synchronisation : pas de seconde en parallèle.
                None if crate::sync::configured() => {
                    crate::sync::set_remember(self.remember, None);
                    self.open(false);
                    return;
                }
                None => {}
            }
        }
        // Formulaire « Autre compte » avec le compte déjà ouvert : reconnexion sans redémarrage.
        if self.other && st.connecting {
            self.form_watch = true;
        }
        if self.form_watch && !st.connecting {
            self.form_watch = false;
            if st.connect_error.is_none() && st.switch_to.is_none() && st.recovery.is_none() && crate::sync::configured() {
                self.open(false);
                return;
            }
        }

        let busy = self.checking.is_some() || self.online || st.connecting;
        let w = 380.0_f32.min(ui.available_width() - 32.0);
        let top = (ui.available_height() * 0.16).max(40.0);
        ui.add_space(top);
        ui.vertical_centered(|ui| {
            ui.set_max_width(w);
            match (&account, self.other) {
                // Compte de ce poste : image, nom, mot de passe si besoin, un bouton.
                (Some(a), false) => {
                    let (rect, _) = ui.allocate_exact_size(vec2(2.0 * AVATAR_R, 2.0 * AVATAR_R), Sense::hover());
                    match &self.avatar {
                        Some(t) => {
                            let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
                            ui.painter().add(egui::epaint::RectShape::filled(rect, CornerRadius::same(AVATAR_R as u8), Color32::WHITE).with_texture(t.id(), uv));
                        }
                        None => crate::tabs::messages::initials_avatar(ui.painter(), rect.center(), AVATAR_R, &a.login, false),
                    }
                    ui.add_space(12.0);
                    ui.label(RichText::new(&a.login).font(theme::title_font(28.0)).color(theme::TEXT()));
                    ui.add_space(18.0);
                    // Session à refaire (déconnecté) : le serveur doit vérifier le mot de passe.
                    let need_password = !configured || a.forget || !self.remember;
                    if !configured {
                        ui.label(RichText::new(t!("Session terminée : reconnecte-toi.", "Session ended: sign in again.")).color(theme::TEXT_MUTED()));
                        ui.add_space(8.0);
                    }
                    let mut submit = false;
                    if need_password {
                        let resp = ui.add_enabled(
                            !busy,
                            egui::TextEdit::singleline(&mut self.password)
                                .password(true)
                                .hint_text(t!("Mot de passe", "Password"))
                                .desired_width(260.0)
                                .font(egui::TextStyle::Body),
                        );
                        if !busy && !resp.has_focus() && self.password.is_empty() && self.error.is_none() {
                            resp.request_focus();
                        }
                        submit |= resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
                        ui.add_space(8.0);
                    }
                    ui.add_enabled(!busy, egui::Checkbox::new(&mut self.remember, RichText::new(t!("Retenir mon compte", "Remember my account")).size(15.0)));
                    ui.add_space(12.0);
                    let ready = !busy && (!need_password || !self.password.is_empty());
                    submit |= ui.add_enabled(ready, egui::Button::new(RichText::new(t!("Se connecter", "Sign in")).size(17.0)).min_size(vec2(200.0, 36.0))).clicked();
                    if !need_password && !busy && ui.input(|i| i.key_pressed(Key::Enter)) {
                        submit = true;
                    }
                    if submit && ready {
                        self.error = None;
                        if !need_password {
                            crate::sync::set_remember(true, None);
                            self.open(true);
                            return;
                        }
                        if configured {
                            // Vérifié sur ce poste (même hors ligne), hors de l'interface ; bon mot
                            // de passe : choix « retenir » et empreinte renouvelée au passage.
                            let (tx, rx) = channel();
                            let pw = self.password.clone();
                            let remember = self.remember;
                            let c = ctx.clone();
                            std::thread::spawn(move || {
                                let ok = crate::sync::verify_password(&pw);
                                if ok == Some(true) {
                                    crate::sync::set_remember(remember, Some(&pw));
                                }
                                let _ = tx.send(ok);
                                c.request_repaint();
                            });
                            self.checking = Some(rx);
                        } else {
                            self.connect_online(Some(a));
                        }
                    }
                    if busy {
                        ui.add_space(8.0);
                        ui.add(egui::Spinner::new().size(16.0));
                    }
                }
                // Pas encore de compte, ou « Autre compte » : formulaire complet du NAS.
                _ => {
                    ui.label(RichText::new(t!("Bienvenue", "Welcome")).font(theme::title_font(28.0)).color(theme::TEXT()));
                    ui.add_space(6.0);
                    ui.label(RichText::new(t!("Connecte-toi à ton compte NAS.", "Sign in to your NAS account.")).color(theme::TEXT_MUTED()));
                    ui.add_space(16.0);
                    ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                        crate::sync::ui::add_account(ui);
                        ui.add_space(8.0);
                        ui.checkbox(&mut self.remember, t!("Retenir mon compte", "Remember my account"));
                    });
                }
            }
            if let Some(e) = &self.error {
                ui.add_space(10.0);
                ui.label(RichText::new(e.as_str()).color(theme::TODAY()));
            }
            ui.add_space(22.0);
            ui.horizontal(|ui| {
                // Liens, centrés à peu près (largeur fixe estimée).
                ui.add_space(((ui.available_width() - 330.0) / 2.0).max(0.0));
                if account.is_some() {
                    let label = if self.other { t!("← Mon compte", "← My account") } else { t!("Autre compte", "Another account") };
                    if ui.add_enabled(!busy, egui::Button::new(label).frame(false)).clicked() {
                        self.other = !self.other;
                        self.error = None;
                    }
                    ui.label(RichText::new("·").color(theme::TEXT_FAINT()));
                }
                // Données sans compte : ouvertes ici, ou après redémarrage si un compte est ouvert.
                // (Données sans compte qui portent un compte, reprise interrompue : pas de passe-droit.)
                let local = crate::paths::profile() == crate::paths::LOCAL_PROFILE;
                if !(local && account.is_some()) && ui.add_enabled(!busy, egui::Button::new(t!("Continuer sans compte", "Continue without an account")).frame(false)).clicked() {
                    if local {
                        self.open(true);
                    } else {
                        crate::sync::open_without_account(ctx);
                    }
                }
            });
        });
    }

    fn connect_online(&mut self, account: Option<&crate::sync::Account>) {
        let Some(a) = account else { return };
        self.online = true;
        crate::sync::connect(a.server.clone(), a.login.clone(), self.password.clone(), None);
    }

    /// Fondu flou par-dessus l'app qui vient d'être dessinée.
    fn reveal(&mut self, ctx: &egui::Context, screen: Rect) {
        let paint = |ctx: &egui::Context, f: &dyn Fn(&egui::Painter)| {
            egui::Area::new(egui::Id::unique("wt_login_reveal")).order(egui::Order::Tooltip).fixed_pos(screen.min).interactable(false).show(ctx, |ui| f(ui.painter()));
        };
        // 1. Première image de l'app, sous un voile : capturée puis floutée hors de l'interface.
        if self.shot_asked.is_none() {
            self.shot_asked = Some(Instant::now());
            let slot = self.shot.clone();
            let c = ctx.clone();
            ctx.request_screenshot(move |img| {
                std::thread::spawn(move || {
                    let blurred = crate::search::blur(&img, 6, 4);
                    if let Ok(mut s) = slot.lock() {
                        *s = Some(blurred);
                    }
                    c.request_repaint();
                });
            });
        }
        if let Some(img) = self.shot.lock().ok().and_then(|mut s| s.take()) {
            self.reveal_tex = Some(ctx.load_texture("wt_login_reveal", img, egui::TextureOptions::LINEAR));
            self.reveal_start = Some(Instant::now());
        }
        let veil = theme::BG().gamma_multiply(VEIL);
        match (&self.reveal_tex, self.reveal_start) {
            // 2. Capture floutée, qui s'efface et laisse l'app nette.
            (Some(t), Some(start)) => {
                let p = (start.elapsed().as_secs_f32() / REVEAL_S).min(1.0);
                let k = 1.0 - p * p * (3.0 - 2.0 * p);
                let id = t.id();
                paint(ctx, &|painter| {
                    painter.image(id, screen, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE.gamma_multiply(k));
                });
                if p >= 1.0 {
                    self.stage = Stage::Done;
                    self.reveal_tex = None;
                }
                ctx.request_repaint();
            }
            // En attendant le flou : voile (capture impossible au bout d'une seconde : on montre l'app).
            _ => {
                if self.shot_asked.is_some_and(|t| t.elapsed() > Duration::from_secs(1)) {
                    self.stage = Stage::Done;
                } else {
                    paint(ctx, &|painter| {
                        painter.rect_filled(screen, CornerRadius::ZERO, veil);
                    });
                    ctx.request_repaint();
                }
            }
        }
    }
}

/// Image importée du compte ouvert (si c'est elle qu'il affiche) ; sinon ses initiales.
fn load_avatar(ctx: &egui::Context) -> Option<TextureHandle> {
    let config = crate::paths::config();
    let image = std::fs::read_to_string(config.join("avatar.json")).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).is_some_and(|v| v["source"] == "image");
    let img = image.then(|| image::open(config.join("avatar.png")).ok()).flatten()?.to_rgba8();
    let ci = ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
    Some(ctx.load_texture("wt_login_avatar", ci, egui::TextureOptions::LINEAR))
}

impl App for Gate {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Some(app) = &mut self.app {
            app.on_exit(gl);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let screen = ui.max_rect();
        // Données chargées : l'app est créée (dans le fil de l'interface).
        if let Stage::Opening(rx) = &self.stage
            && !matches!(rx.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty))
        {
            self.app = Some(WorkTableApp::new(&ctx));
            self.stage = Stage::Reveal;
        }
        if let Some(app) = &mut self.app {
            app.ui(ui, frame);
            if matches!(self.stage, Stage::Reveal) {
                self.reveal(&ctx, screen);
            }
            return;
        }
        // Avant la connexion : fond uni, rien de chargé.
        ui.painter().rect_filled(screen, CornerRadius::ZERO, theme::BG());
        crate::titlebar::show(&ctx, screen);
        match self.stage {
            Stage::Login => self.login_ui(&ctx, ui),
            _ => {
                ui.add_space(ui.available_height() * 0.42);
                ui.vertical_centered(|ui| {
                    ui.add(egui::Spinner::new().size(22.0));
                    ui.add_space(8.0);
                    ui.label(RichText::new(t!("Ouverture…", "Opening…")).color(theme::TEXT_MUTED()));
                });
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }
    }
}
