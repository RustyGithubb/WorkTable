//! Panneau de droite : un événement existant s'affiche d'abord en lecture (rien n'est
//! modifiable) ; le bouton « Modifier » ouvre l'édition (titre, dates, heures, journée
//! entière, agenda, couleur, invités avec suggestions de l'annuaire, lieu, Google Meet,
//! description) ; enregistrer, annuler, supprimer, ouvrir dans Google Agenda.

use eframe::egui::{self, Align, Color32, Key, Layout, RichText, Sense, Stroke, vec2};

use super::{CalendarState, Editor, color32};
use crate::google::Account;
use crate::google::calendar::{CalEvent, EVENT_COLORS};
use crate::tabs::mail::contacts::Contacts;
use crate::tabs::mail::recipients;
use crate::theme;

pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, acc: &Account, book: &Contacts, prefs: &mut crate::prefs::Prefs) {
    // Événement existant : affichage en lecture, modification seulement après « Modifier ».
    if let Some(ed) = &cal.editor
        && !ed.editing
    {
        let ev = ed.ev.clone();
        let highlighted = crate::tabs::timeline::is_highlighted(&prefs.timeline, &ev);
        match view(ui, cal, &ev, book, highlighted) {
            Some(ViewAction::Highlight) => crate::tabs::timeline::toggle(&mut prefs.timeline, &ev),
            Some(ViewAction::Edit) => {
                if let Some(ed) = &mut cal.editor {
                    ed.editing = true;
                    ed.focus_title = true;
                }
            }
            Some(ViewAction::Close) => {
                cal.editor = None;
                cal.selected = None;
            }
            None => {}
        }
        return;
    }
    let cals = cal.calendars.clone();
    let Some(ed) = cal.editor.as_mut() else { return };
    let writable = cals.iter().find(|c| c.id == ed.ev.calendar).is_none_or(|c| c.writable);
    let is_new = ed.original.is_none();
    let mut close = false;
    let mut cancel = false;
    let mut save = false;
    let mut delete = false;

    ui.horizontal(|ui| {
        ui.label(RichText::new(if is_new { t!("NOUVEL ÉVÉNEMENT", "NEW EVENT") } else { t!("MODIFIER L'ÉVÉNEMENT", "EDIT EVENT") }).small().strong().color(theme::TEXT_MUTED()));
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
            let out = egui::TextEdit::singleline(&mut ed.ev.title)
                .hint_text(t!("Ajouter un titre", "Add a title"))
                .font(theme::title_font(19.0))
                .frame(egui::Frame::NONE)
                .desired_width(f32::INFINITY)
                .show(ui);
            crate::spell::check(ui, &out, &mut ed.ev.title);
            let title = out.response.response;
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
            let guests = recipients::field(ui, t!("Invités", "Guests"), &mut ed.attendees, "cal_attendees", t!("Nom ou adresse…", "Name or address…"), book, &mut ed.suggest);
            // Clic droit : fiche contact de chaque invité.
            let list: Vec<String> =
                crate::tabs::mail::contacts::split_list(&ed.attendees).iter().filter_map(|m| crate::tabs::mail::contacts::parse_mailbox(m).map(|(_, a)| a)).collect();
            if !list.is_empty() {
                guests.context_menu(|ui| crate::people::guests_menu(ui, &list));
            }
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
            let out = egui::TextEdit::multiline(&mut ed.ev.description)
                .desired_rows(4)
                .desired_width(f32::INFINITY)
                .hint_text(t!("Ajouter une description", "Add a description"))
                .show(ui);
            crate::spell::check(ui, &out, &mut ed.ev.description);
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
            // Annuler la modification d'un événement existant : retour à l'affichage.
            if is_new || !writable {
                close = true;
            } else {
                cancel = true;
            }
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
    } else if cancel {
        if let Some(ed) = &mut cal.editor
            && let Some(o) = ed.original.clone()
        {
            *ed = Editor::new(o.clone(), Some(o));
        }
    } else if delete {
        cal.delete_editor(acc);
    } else if save {
        cal.save_editor(acc);
    }
}

pub enum ViewAction {
    Edit,
    Close,
    /// Mettre en évidence dans le quadrillage du tableau de bord (ou retirer).
    Highlight,
}

/// Affichage en lecture d'un événement (aucun champ modifiable).
pub fn view(ui: &mut egui::Ui, cal: &CalendarState, e: &CalEvent, book: &Contacts, highlighted: bool) -> Option<ViewAction> {
    let writable = cal.calendar(&e.calendar).is_none_or(|c| c.writable);
    let mut act = None;
    ui.horizontal(|ui| {
        ui.label(RichText::new(t!("ÉVÉNEMENT", "EVENT")).small().strong().color(theme::TEXT_MUTED()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("×").on_hover_text(t!("Fermer (Échap)", "Close (Esc)")).clicked() {
                act = Some(ViewAction::Close);
            }
        });
    });
    ui.add_space(6.0);
    ui.set_max_width(ui.available_width());
    egui::ScrollArea::vertical().id_salt("cal_view").auto_shrink([false, false]).max_height(ui.available_height() - 48.0).show(ui, |ui| {
        // Titre, avec la couleur de l'événement.
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(14.0, 26.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 6.0, color32(cal.color_of(e)));
            let title = if e.title.trim().is_empty() { t!("(Sans titre)", "(Untitled)") } else { &e.title };
            ui.add(egui::Label::new(RichText::new(title).font(theme::title_font(19.0)).color(theme::TEXT())).wrap().selectable(true));
        });
        ui.add_space(4.0);
        ui.label(RichText::new(when(e)).color(theme::TEXT_MUTED()));
        // Mise en évidence : le jour est coloré dans le quadrillage du tableau de bord.
        ui.add_space(6.0);
        let accent = crate::tabs::timeline::ACCENT;
        let (label, tip) = if highlighted {
            (
                t!("Mis en évidence dans le quadrillage · retirer", "Highlighted in the day grid · remove"),
                t!("Le jour n'est plus coloré dans le quadrillage du tableau de bord", "The day is no longer coloured in the dashboard day grid"),
            )
        } else {
            (
                t!("Mettre en évidence dans le quadrillage", "Highlight in the day grid"),
                t!("Colore ce jour dans le quadrillage du tableau de bord", "Colours this day in the dashboard day grid"),
            )
        };
        let button = egui::Button::new(RichText::new(label).small().color(if highlighted { Color32::WHITE } else { accent }))
            .fill(if highlighted { accent.gamma_multiply(0.8) } else { accent.gamma_multiply(0.15) })
            .stroke(egui::Stroke::new(1.0, accent.gamma_multiply(0.7)))
            .corner_radius(egui::CornerRadius::same(255));
        if ui.add(button).on_hover_text(tip).clicked() {
            act = Some(ViewAction::Highlight);
        }
        ui.add_space(10.0);
        if let Some(c) = cal.calendar(&e.calendar) {
            info_row(ui, t!("Agenda", "Calendar"), |ui| {
                let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                ui.painter().circle_filled(r.center(), 5.0, color32(c.color));
                ui.label(&c.name);
            });
        }
        if !e.location.is_empty() {
            info_row(ui, t!("Lieu", "Location"), |ui| {
                ui.add(egui::Label::new(&e.location).wrap().selectable(true));
            });
        }
        if let Some(link) = &e.meet_link {
            ui.add_space(4.0);
            if ui.button(t!("Rejoindre Google Meet", "Join Google Meet")).clicked() {
                let _ = webbrowser::open(link);
            }
        }
        // Invités : clic = fiche contact ; clic droit = menu.
        if !e.attendees.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new(tf!("INVITÉS · {}", "GUESTS · {}", e.attendees.len())).small().color(theme::TEXT_FAINT()));
            ui.add_space(2.0);
            for a in &e.attendees {
                let name = book.get(a).map(|c| c.display_name().to_string()).filter(|n| !n.eq_ignore_ascii_case(a));
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if name.is_some() { 34.0 } else { 24.0 }), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Fiche contact", "Contact card"));
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, egui::CornerRadius::same(6), theme::overlay(7));
                }
                let initial: String = name.as_deref().unwrap_or(a).chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
                let c = egui::pos2(rect.left() + 13.0, rect.center().y);
                p.circle_filled(c, 10.0, theme::SURFACE_HI());
                p.text(c, egui::Align2::CENTER_CENTER, initial, egui::FontId::proportional(11.0), theme::TEXT());
                let x = rect.left() + 30.0;
                match &name {
                    Some(n) => {
                        p.text(egui::pos2(x, rect.top() + 3.0), egui::Align2::LEFT_TOP, n, egui::FontId::proportional(13.0), theme::TEXT());
                        p.text(egui::pos2(x, rect.top() + 19.0), egui::Align2::LEFT_TOP, a, egui::FontId::proportional(11.0), theme::TEXT_FAINT());
                    }
                    None => {
                        p.text(egui::pos2(x, rect.center().y), egui::Align2::LEFT_CENTER, a, egui::FontId::proportional(12.5), theme::TEXT());
                    }
                }
                if resp.clicked() {
                    crate::people::open(ui.ctx(), a, name.as_deref().unwrap_or(""));
                }
                resp.context_menu(|ui| crate::people::menu_item(ui, a, name.as_deref().unwrap_or("")));
            }
        }
        if !e.description.trim().is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new("DESCRIPTION").small().color(theme::TEXT_FAINT()));
            ui.add(egui::Label::new(e.description.trim()).wrap().selectable(true));
        }
        if let Some(link) = &e.html_link {
            ui.add_space(10.0);
            if ui.add(egui::Button::new(RichText::new(t!("Ouvrir dans Google Agenda ↗", "Open in Google Calendar ↗")).small().color(theme::TEXT_MUTED())).frame(false)).clicked()
            {
                let _ = webbrowser::open(link);
            }
        }
    });
    ui.separator();
    ui.horizontal(|ui| {
        if writable {
            if ui.button(t!("✏  Modifier", "✏  Edit")).on_hover_text(t!("Modifier l'événement (E)", "Edit the event (E)")).clicked() {
                act = Some(ViewAction::Edit);
            }
        } else {
            ui.label(RichText::new(t!("Agenda en lecture seule", "Read-only calendar")).small().color(theme::TEXT_FAINT()));
        }
        if ui.button(t!("Fermer", "Close")).clicked() {
            act = Some(ViewAction::Close);
        }
    });
    if writable && act.is_none() && ui.input(|i| i.key_pressed(Key::E) && i.modifiers.is_none()) && !ui.ctx().egui_wants_keyboard_input() {
        act = Some(ViewAction::Edit);
    }
    act
}

/// Date et heures d'un événement, en toutes lettres.
fn when(e: &CalEvent) -> String {
    use super::{date_of, hhmm};
    let d0 = date_of(e.start);
    if e.all_day {
        let d1 = date_of((e.end - 86_400_000).max(e.start));
        return if d1 == d0 {
            tf!("{} · toute la journée", "{} · all day", crate::tabs::long_date(d0))
        } else {
            format!("{} – {}", crate::tabs::long_date(d0), crate::tabs::long_date(d1))
        };
    }
    let d1 = date_of(e.end);
    if d1 == d0 {
        format!("{} · {} – {}", crate::tabs::long_date(d0), hhmm(e.start), hhmm(e.end))
    } else {
        format!("{} {} → {} {}", crate::tabs::long_date(d0), hhmm(e.start), crate::tabs::long_date(d1), hhmm(e.end))
    }
}

fn info_row(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui)) {
    ui.horizontal_top(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(62.0, 20.0), Sense::hover());
        ui.painter().text(r.left_center(), egui::Align2::LEFT_CENTER, label, egui::FontId::proportional(13.0), theme::TEXT_MUTED());
        add(ui);
    });
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
