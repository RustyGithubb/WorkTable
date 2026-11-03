//! Panneau de demande de réservation (même style que la recherche Ctrl+Espace) : ouvert après
//! un cliquer-glisser sur la grille du planning d'une salle (seul planning où l'on réserve).
//! Envoi à l'espace web après confirmation ; un administrateur valide ou refuse.

use chrono::NaiveDate;
use eframe::egui::{self, Key, Modifiers, Pos2, RichText, Stroke};

use crate::hyperplanning::RequestStatus;
use crate::hyperplanning::cache::Snapshot;
use crate::hyperplanning::web::RoomRequest;
use crate::search::{Backdrop, overlay_frame};
use crate::theme;

/// Créneau choisi sur la grille (minutes depuis minuit).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Slot {
    pub date: NaiveDate,
    pub start_min: u32,
    pub end_min: u32,
    /// Salle du planning affiché.
    pub room: u32,
}

#[derive(Default)]
pub struct BookPanel {
    slot: Option<Slot>,
    comment: String,
    bg: Backdrop,
    /// La demande en cours a été lancée depuis ce panneau (affichage de son résultat).
    sent: bool,
}

fn hhmm(min: u32) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

impl BookPanel {
    pub fn is_open(&self) -> bool {
        self.slot.is_some()
    }

    pub fn open(&mut self, ctx: &egui::Context, slot: Slot) {
        self.slot = Some(slot);
        self.comment.clear();
        self.sent = false;
        self.bg.start(ctx);
    }

    fn close(&mut self) {
        self.slot = None;
        self.bg.clear();
    }

    /// Affiche le panneau ; renvoie la demande à envoyer (clé de la salle, demande).
    pub fn show(&mut self, ctx: &egui::Context, d: &Snapshot, status: &RequestStatus) -> Option<(u32, RoomRequest)> {
        let slot = self.slot?;
        let (k, mut close) = self.bg.show(ctx, "hp_book_backdrop");
        let sending = matches!(status, RequestStatus::Sending(_));
        let done = self.sent && matches!(status, RequestStatus::Sent(_));
        let mut send = false;
        ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::Escape) {
                close = true;
            }
            if i.consume_key(Modifiers::NONE, Key::Enter) && !sending {
                if done {
                    close = true;
                } else {
                    send = true;
                }
            }
        });

        let screen = ctx.content_rect();
        let w = 520.0_f32.min(screen.width() - 40.0);
        let top = screen.top() + screen.height() * 0.16 - (1.0 - k) * 12.0;
        let box_id = egui::Id::unique("hp_book_box");
        egui::Area::new(box_id).order(egui::Order::Tooltip).fixed_pos(Pos2::new(screen.center().x - w / 2.0, top)).show(ctx, |ui| {
            ui.multiply_opacity(k);
            overlay_frame().show(ui, |ui| {
                ui.set_width(w - 28.0);
                ui.label(RichText::new(t!("Demande de réservation", "Booking request")).font(theme::title_font(18.0)).color(theme::TEXT()));
                ui.add_space(8.0);
                ui.painter().hline(ui.max_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));
                ui.add_space(8.0);

                let row = |ui: &mut egui::Ui, label: &str, value: String| {
                    ui.horizontal(|ui| {
                        ui.add_sized([70.0, 20.0], egui::Label::new(RichText::new(label).small().color(theme::TEXT_FAINT())));
                        ui.label(RichText::new(value).font(theme::editor_font(14.0, true)).color(theme::TEXT()));
                    });
                };
                row(ui, t!("Date", "Date"), crate::tabs::long_date(slot.date));
                row(ui, t!("Horaire", "Time"), format!("{} – {}", hhmm(slot.start_min), hhmm(slot.end_min)));
                let room = d.salles.get(&slot.room);
                let cap = room.map(|s| s.capacity).filter(|c| *c > 0).map(|c| tf!("  ·  {c} places", "  ·  {c} seats")).unwrap_or_default();
                row(ui, t!("Salle", "Room"), format!("{}{cap}", room.map(|s| s.name.as_str()).unwrap_or("?")));
                ui.add_space(8.0);
                let te = ui.add_enabled(
                    !sending && !done,
                    egui::TextEdit::singleline(&mut self.comment)
                        .hint_text(t!("Commentaire (motif, nombre de personnes…)", "Comment (reason, number of people…)"))
                        .desired_width(f32::INFINITY),
                );
                if !sending && !done && !te.has_focus() && ui.memory(|m| m.focused().is_none()) {
                    te.request_focus();
                }
                ui.add_space(10.0);

                ui.horizontal(|ui| {
                    if done {
                        ui.label(RichText::new(t!("✓ Demande envoyée — en attente de validation", "✓ Request sent — awaiting approval")).color(theme::TODAY()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button(t!("Fermer", "Close")).clicked() {
                                close = true;
                            }
                        });
                        return;
                    }
                    let ready = !sending;
                    if ui.add_enabled(ready, egui::Button::new(RichText::new(t!("Envoyer la demande", "Send the request")).strong())).clicked() {
                        send = true;
                    }
                    if ui.add_enabled(!sending, egui::Button::new(t!("Annuler", "Cancel"))).clicked() {
                        close = true;
                    }
                    if sending {
                        ui.add(egui::Spinner::new().size(14.0));
                        ui.label(RichText::new(t!("Envoi…", "Sending…")).small());
                    }
                });
                if self.sent
                    && let RequestStatus::Failed(_, e) = status
                {
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(tf!("Demande non envoyée : {e}", "Request not sent: {e}")).small().color(theme::TEXT())).wrap());
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(t!(
                        "Entrée envoyer · Échap fermer · validée ou refusée par un administrateur",
                        "Enter send · Esc close · approved or refused by an administrator"
                    ))
                    .small()
                    .color(theme::TEXT_FAINT()),
                );
            });
        });
        ctx.move_to_top(egui::LayerId::new(egui::Order::Tooltip, box_id));

        let mut out = None;
        if send
            && !done
            && !sending
            && let Some(s) = d.salles.get(&slot.room)
        {
            out = Some((slot.room, RoomRequest { room: s.name.clone(), date: slot.date, start_min: slot.start_min, end_min: slot.end_min, comment: self.comment.clone() }));
            self.sent = true;
        }
        // Pendant l'envoi, le panneau reste ouvert (résultat affiché).
        if close && !sending {
            self.close();
        }
        out
    }
}
