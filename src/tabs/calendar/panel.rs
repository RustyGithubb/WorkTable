//! Panneau de droite : détail / édition d'un événement (titre, dates, heures, journée
//! entière, agenda, couleur, invités avec suggestions de l'annuaire, lieu, Google Meet,
//! description) ; enregistrer, supprimer, ouvrir dans Google Agenda.

use eframe::egui::{self, Align, Color32, Key, Layout, RichText, Sense, Stroke, vec2};

use super::{CalendarState, color32};
use crate::google::Account;
use crate::google::calendar::EVENT_COLORS;
use crate::tabs::mail::contacts::Contacts;
use crate::tabs::mail::recipients;
use crate::theme;

pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, acc: &Account, book: &Contacts) {
    let cals = cal.calendars.clone();
    let Some(ed) = cal.editor.as_mut() else { return };
    let writable = cals.iter().find(|c| c.id == ed.ev.calendar).is_none_or(|c| c.writable);
    let is_new = ed.original.is_none();
    let mut close = false;
    let mut save = false;
    let mut delete = false;

    ui.horizontal(|ui| {
        ui.label(RichText::new(if is_new { t!("NOUVEL ÉVÉNEMENT", "NEW EVENT") } else { t!("ÉVÉNEMENT", "EVENT") }).small().strong().color(theme::TEXT_MUTED()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("×").on_hover_text(t!("Fermer (Échap)", "Close (Esc)")).clicked() {
                close = true;
            }
        });
    });
    ui.add_space(6.0);

    ui.set_max_width(ui.available_width());
    egui::ScrollArea::vertical().id_salt("cal_panel").auto_shrink([false, false]).max_height(ui.available_height() - 48.0).show(ui, |ui| {
        ui.add_enabled_ui(writable && !ed.saving, |ui| {
            // Titre
            let title = ui.add(
                egui::TextEdit::singleline(&mut ed.ev.title)
                    .hint_text(t!("Ajouter un titre", "Add a title"))
                    .font(theme::title_font(19.0))
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY),
            );
            if ed.focus_title {
                title.request_focus();
                ed.focus_title = false;
            }
            if !writable {
                ui.label(RichText::new(t!("Agenda en lecture seule", "Read-only calendar")).small().color(theme::TEXT_FAINT()));
            }
            ui.add_space(10.0);

            // Dates et heures
            ui.checkbox(&mut ed.ev.all_day, t!("Toute la journée", "All day"));
            ui.add_space(4.0);
            let all_day = ed.ev.all_day;
            row(ui, t!("Début", "Start"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.date).desired_width(96.0).hint_text("jj/mm/aaaa"));
                if !all_day {
                    ui.add(egui::TextEdit::singleline(&mut ed.start_time).desired_width(60.0).hint_text("hh:mm"));
                }
            });
            row(ui, t!("Fin", "End"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.end_date).desired_width(96.0).hint_text("jj/mm/aaaa"));
                if !all_day {
                    ui.add(egui::TextEdit::singleline(&mut ed.end_time).desired_width(60.0).hint_text("hh:mm"));
                }
            });
            if !all_day {
                row(ui, t!("Durée", "Duration"), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (label, min) in [("15 min", 15), ("30 min", 30), ("1 h", 60), ("2 h", 120)] {
                        if ui.small_button(label).clicked()
                            && let Ok(e) = ed.build()
                        {
                            let end = e.start + min * 60_000;
                            ed.end_date = super::date_of(end).format("%d/%m/%Y").to_string();
                            ed.end_time = super::hhmm(end);
                        }
                    }
                });
            }
            ui.add_space(8.0);

            // Agenda
            row(ui, t!("Agenda", "Calendar"), |ui| {
                let current = cals.iter().find(|c| c.id == ed.ev.calendar);
                egui::ComboBox::from_id_salt("cal_pick").selected_text(current.map(|c| c.name.as_str()).unwrap_or("—")).width(ui.available_width() - 8.0).show_ui(ui, |ui| {
                    for c in cals.iter().filter(|c| c.writable) {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 5.0, color32(c.color));
                            if ui.selectable_label(ed.ev.calendar == c.id, &c.name).clicked() {
                                ed.ev.calendar = c.id.clone();
                            }
                        });
                    }
                });
            });

            // Couleur de l'événement (sinon celle de l'agenda)
            row(ui, t!("Couleur", "Colour"), |ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let cal_color = cals.iter().find(|c| c.id == ed.ev.calendar).map(|c| c.color).unwrap_or([0x88, 0x88, 0x88]);
                if swatch(ui, color32(cal_color), ed.ev.color.is_none(), t!("Couleur de l'agenda", "Calendar colour")) {
                    ed.ev.color = None;
                }
                for c in EVENT_COLORS {
                    if swatch(ui, color32(c), ed.ev.color == Some(c), "") {
                        ed.ev.color = Some(c);
                    }
                }
            });
            ui.add_space(8.0);

            // Invités : suggestions de l'annuaire (Tab / Entrée pour ajouter).
            recipients::field(ui, t!("Invités", "Guests"), &mut ed.attendees, "cal_attendees", t!("Nom ou adresse…", "Name or address…"), book, &mut ed.suggest);
            row(ui, t!("Lieu", "Location"), |ui| {
                ui.add(egui::TextEdit::singleline(&mut ed.ev.location).hint_text(t!("Ajouter un lieu", "Add a location")).desired_width(f32::INFINITY));
            });
            ui.add_space(6.0);

            // Visioconférence
            match ed.ev.meet_link.clone() {
                Some(link) => {
                    ui.horizontal(|ui| {
                        if ui.button(t!("Rejoindre Google Meet", "Join Google Meet")).clicked() {
                            let _ = webbrowser::open(&link);
                        }
                        ui.add(egui::Label::new(RichText::new(&link).small().color(theme::LINK())).truncate());
                    });
                }
                None => {
                    ui.checkbox(&mut ed.add_meet, t!("Ajouter une visioconférence Google Meet", "Add Google Meet video conferencing"));
                }
            }
            ui.add_space(8.0);

            ui.label(RichText::new("Description").small().color(theme::TEXT_MUTED()));
            ui.add(egui::TextEdit::multiline(&mut ed.ev.description).desired_rows(4).desired_width(f32::INFINITY).hint_text(t!("Ajouter une description", "Add a description")));
        });

        if let Some(e) = &ed.error {
            ui.add_space(6.0);
            ui.add(egui::Label::new(RichText::new(e).small().color(Color32::from_rgb(0xEB, 0x57, 0x57))).wrap());
        }
        if let Some(link) = ed.original.as_ref().and_then(|o| o.html_link.clone()) {
            ui.add_space(8.0);
            if ui.add(egui::Button::new(RichText::new(t!("Ouvrir dans Google Agenda ↗", "Open in Google Calendar ↗")).small().color(theme::TEXT_MUTED())).frame(false)).clicked()
            {
                let _ = webbrowser::open(&link);
            }
        }
    });

    // Boutons
    ui.separator();
    ui.horizontal(|ui| {
        if writable {
            let label = if ed.saving { t!("Enregistrement…", "Saving…") } else { t!("Enregistrer", "Save") };
            if ui.add_enabled(!ed.saving, egui::Button::new(label)).on_hover_text(t!("Ctrl + Entrée", "Ctrl + Enter")).clicked() {
                save = true;
            }
        }
        if ui.button(if writable { t!("Annuler", "Cancel") } else { t!("Fermer", "Close") }).clicked() {
            close = true;
        }
        if ed.saving {
            ui.add(egui::Spinner::new().size(14.0));
        }
        if !is_new && writable {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let label = if ed.confirm_delete { t!("Confirmer la suppression", "Confirm deletion") } else { t!("Supprimer", "Delete") };
                let btn = egui::Button::new(RichText::new(label).color(if ed.confirm_delete { theme::BG() } else { theme::TEXT_MUTED() })).fill(if ed.confirm_delete {
                    Color32::from_rgb(0xEB, 0x57, 0x57)
                } else {
                    Color32::TRANSPARENT
                });
                if ui.add(btn).clicked() {
                    if ed.confirm_delete {
                        delete = true;
                    } else {
                        ed.confirm_delete = true;
                    }
                }
            });
        }
    });
    if writable && ui.input(|i| i.modifiers.command && i.key_pressed(Key::Enter)) {
        save = true;
    }

    if close {
        cal.editor = None;
        cal.selected = None;
    } else if delete {
        cal.delete_editor(acc);
    } else if save {
        cal.save_editor(acc);
    }
}

fn row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([52.0, 22.0], egui::Label::new(RichText::new(label).color(theme::TEXT_MUTED())));
        add(ui);
    });
}

fn swatch(ui: &mut egui::Ui, c: Color32, selected: bool, tip: &str) -> bool {
    let (r, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let resp = if tip.is_empty() { resp } else { resp.on_hover_text(tip) };
    ui.painter().circle_filled(r.center(), 7.0, c);
    if selected {
        ui.painter().circle_stroke(r.center(), 9.0, Stroke::new(1.5, theme::TEXT()));
    }
    resp.clicked()
}
