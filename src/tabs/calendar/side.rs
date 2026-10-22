//! Colonne de gauche : nouvel événement, mini-calendrier, agendas (cocher pour afficher).

use chrono::{Datelike, Duration as CDuration, Local, NaiveDate, Timelike};
use eframe::egui::{self, Align2, Color32, CornerRadius, FontId, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, vec2};

use super::{CalendarState, View, at, color32, monday, month_title, snap};
use crate::prefs::Prefs;
use crate::theme;

pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &mut Prefs) {
    let w = ui.available_width();
    if ui.add_sized([w, 32.0], egui::Button::new(t!("+  Nouvel événement", "+  New event"))).on_hover_text("C").clicked() {
        let now = Local::now();
        let d = cal.days().into_iter().find(|d| *d == now.date_naive()).unwrap_or(cal.anchor);
        let m = snap((now.hour() * 60 + now.minute()) as i32 + 15, 30).min(23 * 60);
        cal.new_event(prefs, at(d, m), at(d, m + 60), false);
    }
    ui.add_space(14.0);
    mini_month(ui, cal);
    ui.add_space(16.0);

    egui::ScrollArea::vertical().id_salt("cal_list").auto_shrink([false, false]).show(ui, |ui| {
        let (mine, others): (Vec<_>, Vec<_>) = cal.calendars.iter().cloned().partition(|c| c.writable);
        if cal.calendars.is_empty() {
            ui.label(
                RichText::new(if cal.loading_calendars { t!("Chargement des agendas…", "Loading calendars…") } else { t!("Aucun agenda", "No calendar") })
                    .small()
                    .color(theme::TEXT_FAINT()),
            );
        }
        for (title, list) in [(t!("MES AGENDAS", "MY CALENDARS"), mine), (t!("AUTRES AGENDAS", "OTHER CALENDARS"), others)] {
            if list.is_empty() {
                continue;
            }
            ui.label(RichText::new(title).small().strong().color(theme::TEXT_MUTED()));
            ui.add_space(4.0);
            for c in list {
                let visible = !prefs.hidden_calendars.contains(&c.id);
                let r = calendar_row(ui, &c.name, color32(c.color), visible, c.primary);
                if r.clicked() {
                    if visible {
                        prefs.hidden_calendars.push(c.id.clone());
                    } else {
                        prefs.hidden_calendars.retain(|x| *x != c.id);
                    }
                }
                r.context_menu(|ui| {
                    if ui.button(t!("Afficher uniquement cet agenda", "Show only this calendar")).clicked() {
                        prefs.hidden_calendars = cal.calendars.iter().filter(|x| x.id != c.id).map(|x| x.id.clone()).collect();
                        ui.close();
                    }
                    if ui.button(t!("Afficher tous les agendas", "Show all calendars")).clicked() {
                        prefs.hidden_calendars.clear();
                        ui.close();
                    }
                });
            }
            ui.add_space(12.0);
        }
        ui.add_space(8.0);
        ui.label(
            RichText::new(t!(
                "Raccourcis : T aujourd'hui · J/K suivant/précédent · D/W/M jour/semaine/mois · 1-7 nombre de jours · C nouvel événement · S panneau · Échap fermer",
                "Shortcuts: T today · J/K next/previous · D/W/M day/week/month · 1-7 number of days · C new event · S panel · Esc close"
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
    });
}

fn calendar_row(ui: &mut egui::Ui, name: &str, color: Color32, visible: bool, primary: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if visible {
        t!("Masquer (clic droit : afficher uniquement)", "Hide (right-click: show only this one)")
    } else {
        t!("Afficher", "Show")
    });
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), crate::theme::overlay(7));
    }
    // Case à cocher à la couleur de l'agenda.
    let b = Rect::from_center_size(Pos2::new(rect.left() + 14.0, rect.center().y), vec2(14.0, 14.0));
    if visible {
        p.rect_filled(b, CornerRadius::same(4), color);
        p.add(egui::Shape::line(vec![b.left_center() + vec2(3.0, 0.0), b.center_bottom() + vec2(-1.0, -3.5), b.right_top() + vec2(-3.0, 3.5)], Stroke::new(1.6, theme::BG())));
    } else {
        // Agenda masqué : case et nom nettement estompés.
        p.rect_stroke(b, CornerRadius::same(4), Stroke::new(1.5, color.gamma_multiply(0.35)), StrokeKind::Inside);
    }
    let label = if primary { tf!("{name}  ·  principal", "{name}  ·  primary") } else { name.to_string() };
    let tc = if visible { theme::TEXT() } else { theme::TEXT_FAINT().gamma_multiply(0.75) };
    let g = p.layout_job({
        let mut j = egui::text::LayoutJob::simple_singleline(label, FontId::proportional(13.0), tc);
        j.wrap = egui::text::TextWrapping { max_width: rect.width() - 36.0, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        j
    });
    p.galley(Pos2::new(rect.left() + 30.0, rect.center().y - g.size().y / 2.0), g, tc);
    resp
}

/// Mini-calendrier du mois : clic sur un jour = s'y rendre ; la plage affichée est surlignée.
fn mini_month(ui: &mut egui::Ui, cal: &mut CalendarState) {
    let m = cal.mini_month;
    ui.horizontal(|ui| {
        ui.label(RichText::new(month_title(m)).strong());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("›").clicked() {
                cal.mini_month = add_months(m, 1);
            }
            if ui.small_button("‹").clicked() {
                cal.mini_month = add_months(m, -1);
            }
        });
    });
    ui.add_space(4.0);

    let w = ui.available_width();
    let cell = (w / 7.0).floor();
    let (rect, _) = ui.allocate_exact_size(vec2(w, cell * 0.75 + cell * 6.0 * 0.86), Sense::hover());
    let p = ui.painter().clone();
    for (i, d) in ["L", "M", "M", "J", "V", "S", "D"].iter().enumerate() {
        p.text(Pos2::new(rect.left() + cell * (i as f32 + 0.5), rect.top() + cell * 0.35), Align2::CENTER_CENTER, *d, FontId::proportional(10.5), theme::TEXT_FAINT());
    }
    let today = Local::now().date_naive();
    let shown: Vec<NaiveDate> = match cal.view {
        View::Month => Vec::new(),
        View::Days(_) => cal.days(),
    };
    let start = monday(m);
    let row_h = cell * 0.86;
    for k in 0..42 {
        let d = start + CDuration::days(k);
        let (col, row) = ((k % 7) as f32, (k / 7) as f32);
        let r = Rect::from_min_size(Pos2::new(rect.left() + col * cell, rect.top() + cell * 0.75 + row * row_h), vec2(cell, row_h));
        let resp = ui.interact(r, ui.scope_id().with(("mini", k)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        if shown.contains(&d) {
            p.rect_filled(r.shrink2(vec2(0.0, 2.0)), CornerRadius::same(5), crate::theme::overlay(12));
        }
        let in_month = d.month() == m.month();
        let color = if d == today {
            p.circle_filled(r.center(), row_h * 0.42, theme::TODAY());
            Color32::WHITE
        } else if resp.hovered() {
            p.circle_filled(r.center(), row_h * 0.42, theme::SURFACE_HI());
            theme::TEXT()
        } else if in_month {
            crate::theme::gray(0xCC)
        } else {
            theme::TEXT_FAINT()
        };
        p.text(r.center(), Align2::CENTER_CENTER, d.day().to_string(), FontId::proportional(11.5), color);
        if resp.clicked() {
            cal.go_to(d);
            cal.scroll_to_now = false;
        }
    }
}

fn add_months(d: NaiveDate, n: i32) -> NaiveDate {
    let m0 = d.month0() as i32 + n;
    let y = d.year() + m0.div_euclid(12);
    NaiveDate::from_ymd_opt(y, m0.rem_euclid(12) as u32 + 1, 1).unwrap_or(d)
}
