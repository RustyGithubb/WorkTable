//! Panneau Compte (clic sur la pastille Compte) : même boîte que la recherche Ctrl+Espace, sur
//! fond flouté ; fermé par Échap ou un clic à côté.

use eframe::egui::{self, CornerRadius, Key, Modifiers, Pos2, RichText, Sense, Stroke, vec2};

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
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn toggle(&mut self, ctx: &egui::Context) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            self.error = None;
            self.scroll_top = true;
            self.bg.start(ctx);
        }
    }

    fn close(&mut self) {
        self.open = false;
        self.bg.clear();
    }

    pub fn show(&mut self, ctx: &egui::Context, avatar: &mut Avatar, chat: &ChatState, acc: &mut Account, hp: &mut HyperPlanning) {
        if !self.open {
            return;
        }
        let (k, mut close) = self.bg.show(ctx, "account_backdrop");
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            close = true;
        }

        let screen = ctx.content_rect();
        // Même largeur et même haut que la recherche Ctrl+Espace.
        let w = 640.0_f32.min(screen.width() - 40.0);
        let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
        egui::Area::new(egui::Id::unique("account_box")).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
            ui.multiply_opacity(k);
            overlay_frame().show(ui, |ui| {
                ui.set_width(w - 28.0);
                // En-tête : image du compte et nom.
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(56.0, 56.0), Sense::hover());
                    avatar.paint(ui.painter(), chat, r.center(), 28.0);
                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.add_space(6.0);
                        ui.label(RichText::new(t!("Compte", "Account")).small().color(theme::TEXT_FAINT()));
                        let name = if avatar.name.trim().is_empty() { t!("Non connecté", "Not signed in").to_string() } else { avatar.name.clone() };
                        ui.label(RichText::new(name).font(theme::title_font(18.0)).color(theme::TEXT()));
                    });
                });
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
        });
        // Le fond cliqué passe au premier plan de son niveau : la boîte reste au-dessus.
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("account_box")));
        if close {
            self.close();
        }
    }
}
