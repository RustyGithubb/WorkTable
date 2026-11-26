//! Panneau Compte (clic sur la pastille Compte) : même boîte que la recherche Ctrl+Espace, sur
//! fond flouté ; fermé par Échap ou un clic à côté.

use eframe::egui::{self, Color32, CornerRadius, Key, Modifiers, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, vec2};

use crate::avatar::{Avatar, Source};
use crate::google::Account;
use crate::hyperplanning::HyperPlanning;
use crate::search::{Backdrop, overlay_frame};
use crate::tabs::messages::ChatState;
use crate::{theme, widgets};

#[derive(Default)]
pub struct AccountPanel {
    open: bool,
    bg: Backdrop,
    /// Import d'image raté.
    error: Option<String>,
    /// Remettre le défilement en haut (ouverture).
    scroll_top: bool,
    /// Formulaire d'ajout de compte affiché ; déconnexion à confirmer.
    adding: bool,
    confirm_logout: bool,
    /// Autres comptes déjà connectés sur ce PC (bulles derrière l'image du compte ouvert).
    others: Vec<Other>,
}

/// Rayon de l'image du compte (et des bulles des autres comptes) ; décalage de chaque bulle.
const AVATAR_R: f32 = 48.0;
const BUBBLE_SHIFT: f32 = 26.0;

/// Autre compte de ce poste : son image importée si c'est elle qu'il affiche, sinon ses initiales
/// (sa photo Google n'est chargée que quand son compte est ouvert).
struct Other {
    login: String,
    signed_in: bool,
    texture: Option<TextureHandle>,
}

impl Other {
    fn paint(&self, p: &egui::Painter, c: Pos2, r: f32) {
        match &self.texture {
            Some(t) => {
                let rect = Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r));
                let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
                p.add(egui::epaint::RectShape::filled(rect, CornerRadius::same(r.min(255.0) as u8), Color32::WHITE).with_texture(t.id(), uv));
            }
            None => crate::tabs::messages::initials_avatar(p, c, r, &self.login, false),
        }
    }
}

/// Comptes de ce poste autres que celui ouvert (sans les données « sans compte »).
fn load_others(ctx: &egui::Context) -> Vec<Other> {
    let current = crate::paths::profile();
    crate::sync::known_accounts()
        .into_iter()
        .filter(|(login, _)| login != current && login != crate::paths::LOCAL_PROFILE)
        .map(|(login, signed_in)| {
            let config = crate::paths::profile_root(&login).join("config");
            let image =
                std::fs::read_to_string(config.join("avatar.json")).ok().and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok()).is_some_and(|v| v["source"] == "image");
            let texture = image.then(|| image::open(config.join("avatar.png")).ok()).flatten().map(|img| {
                let img = img.to_rgba8();
                let ci = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
                ctx.load_texture(format!("account_other_{login}"), ci, egui::TextureOptions::LINEAR)
            });
            Other { login, signed_in, texture }
        })
        .collect()
}

/// Rangée centrée : sa largeur est retenue d'une image à l'autre pour la centrer.
fn centered_row(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    let id = ui.scope_id().with(id);
    let w: f32 = ui.ctx().data(|d| d.get_temp(id)).unwrap_or(0.0);
    ui.horizontal(|ui| {
        ui.add_space(((ui.available_width() - w) / 2.0).max(0.0));
        let start = ui.cursor().left();
        add(ui);
        let used = ui.min_rect().right() - start;
        if (used - w).abs() > 0.5 {
            ui.ctx().data_mut(|d| d.insert_temp(id, used));
            ui.ctx().request_repaint();
        }
    });
}

fn rule(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));
    ui.add_space(8.0);
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).strong().color(theme::TEXT()));
    ui.add_space(4.0);
}

/// Image de profil : photo Google, initiales ou image importée.
fn picture(ui: &mut egui::Ui, avatar: &mut Avatar, chat: &ChatState, error: &mut Option<String>) {
    section(ui, t!("Image de profil", "Profile picture"));
    ui.horizontal(|ui| {
        let mut options = vec![(Source::Google, t!("Photo Google", "Google photo")), (Source::Initial, t!("Initiales", "Initials"))];
        if avatar.has_image() {
            options.push((Source::Image, t!("Image importée", "Imported image")));
        }
        let mut source = avatar.source;
        if widgets::segmented(ui, "account_picture", &options, &mut source) {
            avatar.set_source(source);
        }
        if ui.button(t!("Importer…", "Import…")).clicked()
            && let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_file()
        {
            *error = avatar.import(&path).err();
        }
    });
    if avatar.source == Source::Google && !avatar.has_google_photo(chat) {
        ui.label(
            RichText::new(t!("Photo Google pas encore trouvée : initiales en attendant.", "Google photo not found yet: initials meanwhile.")).small().color(theme::TEXT_FAINT()),
        );
    }
    if let Some(e) = error {
        ui.label(RichText::new(e.as_str()).small().color(theme::TODAY()));
    }
}

fn size(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.1} Go", b as f64 / (1u64 << 30) as f64) } else { crate::tabs::mail::attachments::human_size(b.max(1)) }
}

/// Poids total sur le serveur et PC synchronisés.
fn sync_summary(ui: &mut egui::Ui) {
    if !crate::sync::configured() {
        return;
    }
    let st = crate::sync::status();

    rule(ui);
    section(ui, t!("Poids total", "Total size"));
    if st.quota > 0 {
        let frac = (st.used as f32 / st.quota as f32).clamp(0.0, 1.0);
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 6.0), Sense::hover());
        let p = ui.painter();
        p.rect_filled(r, CornerRadius::same(3), theme::SURFACE_HI());
        let fill = egui::Rect::from_min_size(r.min, vec2(r.width() * frac, r.height()));
        p.rect_filled(fill, CornerRadius::same(3), if frac > 0.9 { theme::TODAY() } else { theme::TEXT() });
        ui.add_space(4.0);
        ui.label(RichText::new(tf!("{} sur {} · {} fichiers", "{} of {} · {} files", size(st.used), size(st.quota), st.files)).small().color(theme::TEXT_FAINT()));
    } else {
        ui.label(RichText::new(t!("En attente de la première synchronisation", "Waiting for the first sync")).small().color(theme::TEXT_FAINT()));
    }

    rule(ui);
    section(ui, t!("PC synchronisés", "Synced computers"));
    for d in &st.devices {
        ui.horizontal(|ui| {
            let me = d.id == st.device_id;
            ui.label(if me { tf!("{} (ce PC)", "{} (this computer)", d.name) } else { d.name.clone() });
            ui.label(RichText::new(crate::sync::ui::when(d.last_seen as i64 * 1000)).small().color(theme::TEXT_FAINT()));
            if !me {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t!("Révoquer", "Revoke")).clicked() {
                        crate::sync::revoke(d.id);
                    }
                });
            }
        });
    }
}

impl AccountPanel {
    /// Ouvert et pas en train de disparaître.
    pub fn is_open(&self) -> bool {
        self.open && !self.bg.is_closing()
    }

    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.is_open() {
            self.bg.close(ctx);
        } else {
            self.open = true;
            self.error = None;
            self.scroll_top = true;
            self.adding = false;
            self.confirm_logout = false;
            self.others = load_others(ctx);
            self.bg.start(ctx);
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, avatar: &mut Avatar, chat: &ChatState, acc: &mut Account, hp: &mut HyperPlanning) {
        if !self.open {
            return;
        }
        let (k, mut close) = self.bg.show(ctx, "account_backdrop");
        let closing = self.bg.is_closing();
        if !closing && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }

        let screen = ctx.content_rect();
        // Même largeur et même haut que la recherche Ctrl+Espace.
        let w = 640.0_f32.min(screen.width() - 40.0);
        let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
        // Pendant la disparition, la boîte ne réagit plus aux clics.
        egui::Area::new(egui::Id::unique("account_box")).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).interactable(!closing).show(
            ctx,
            |ui| {
                ui.multiply_opacity(k);
                overlay_frame().show(ui, |ui| {
                    ui.set_width(w - 28.0);
                    // En-tête centré : image du compte (autres comptes en bulles derrière), nom, actions.
                    let mut open_other = None;
                    ui.vertical_centered(|ui| {
                        let size = vec2(2.0 * AVATAR_R, 2.0 * AVATAR_R);
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        // Du plus loin au plus proche : la bulle la plus proche passe devant.
                        for (i, o) in self.others.iter().enumerate().rev() {
                            let c = rect.center() + vec2(BUBBLE_SHIFT * (i + 1) as f32, 0.0);
                            let resp = ui.interact(Rect::from_center_size(c, size), ui.scope_id().with(("account_other", i)), Sense::click());
                            let mut p = ui.painter().clone();
                            p.set_opacity(if resp.hovered() { 0.7 } else { 0.4 });
                            o.paint(&p, c, AVATAR_R);
                            let tip = if o.signed_in { tf!("Ouvrir {}", "Open {}", o.login) } else { tf!("Ouvrir {} (à reconnecter)", "Open {} (sign in again)", o.login) };
                            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(tip).clicked() {
                                open_other = Some(o.login.clone());
                            }
                        }
                        // L'image du compte ouvert, au-dessus, prend les clics de la partie recouverte.
                        let _ = ui.interact(rect, ui.scope_id().with("account_current"), Sense::click());
                        avatar.paint(ui.painter(), chat, rect.center(), AVATAR_R);
                        ui.add_space(10.0);
                        let name = if avatar.name.trim().is_empty() { t!("Non connecté", "Not signed in").to_string() } else { avatar.name.clone() };
                        ui.label(RichText::new(name).font(theme::title_font(26.0)).color(theme::TEXT()));
                    });
                    // Pas connecté : le formulaire « Compte NAS » plus bas sert à ajouter un compte.
                    if crate::sync::configured() {
                        ui.add_space(10.0);
                        centered_row(ui, "account_actions", |ui| {
                            if ui.selectable_label(self.adding, RichText::new(t!("Connecter un nouveau compte", "Connect a new account")).size(15.0)).clicked() {
                                self.adding = !self.adding;
                                self.scroll_top = true;
                            }
                            ui.add_space(8.0);
                            if ui.add_enabled(!self.confirm_logout, egui::Button::new(RichText::new(t!("Se déconnecter", "Sign out")).size(15.0))).clicked() {
                                self.confirm_logout = true;
                            }
                        });
                        if self.confirm_logout {
                            ui.add_space(6.0);
                            ui.vertical_centered(|ui| {
                                ui.label(RichText::new(t!(
                                    "Ce PC ne synchronisera plus ce compte (ses fichiers restent). Sûr ?",
                                    "This computer will stop syncing this account (its files stay). Sure?"
                                )));
                            });
                            centered_row(ui, "account_logout_confirm", |ui| {
                                if ui.button(t!("Se déconnecter", "Sign out")).clicked() {
                                    crate::sync::disconnect();
                                    self.confirm_logout = false;
                                }
                                if ui.button(t!("Annuler", "Cancel")).clicked() {
                                    self.confirm_logout = false;
                                }
                            });
                        }
                    } else {
                        self.adding = false;
                        self.confirm_logout = false;
                    }
                    if let Some(login) = open_other {
                        crate::sync::switch_to(ui.ctx(), &login);
                    }
                    rule(ui);
                    // Étiré vers le bas : même écart sous le panneau qu'au-dessus (comme Ctrl+Espace),
                    // quelle que soit la hauteur de la fenêtre (14 px : marge du cadre).
                    let bottom = screen.bottom() - screen.height() * 0.16;
                    let h = (bottom - 14.0 - ui.cursor().top()).max(120.0);
                    let mut scroll = egui::ScrollArea::vertical().min_scrolled_height(h).max_height(h).auto_shrink([false, false]);
                    // Réouverture : de nouveau en haut.
                    if std::mem::take(&mut self.scroll_top) {
                        scroll = scroll.vertical_scroll_offset(0.0);
                    }
                    scroll.show(ui, |ui| {
                        if self.adding {
                            crate::sync::ui::add_account(ui);
                            rule(ui);
                        }
                        picture(ui, avatar, chat, &mut self.error);
                        rule(ui);
                        section(ui, t!("Compte NAS", "NAS account"));
                        crate::sync::ui::form(ui);
                        sync_summary(ui);
                        rule(ui);
                        section(ui, t!("Compte Google", "Google account"));
                        ui.label(
                            RichText::new(t!("Gmail (Calendar, Docs et Sheets utiliseront ce même compte)", "Gmail (Calendar, Docs and Sheets use this same account)"))
                                .small()
                                .color(theme::TEXT_FAINT()),
                        );
                        ui.add_space(8.0);
                        crate::tabs::settings::google_account_form(ui, acc);
                        rule(ui);
                        section(ui, "HyperPlanning");
                        crate::tabs::settings::hyperplanning_form(ui, hp);
                    });
                });
            },
        );
        // Le fond cliqué passe au premier plan de son niveau : la boîte reste au-dessus.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("account_box")));
        if close {
            self.bg.close(ctx);
        }
        // Fin de la disparition : le panneau est retiré.
        if self.bg.is_closed() {
            self.open = false;
            self.bg.clear();
        }
    }
}
