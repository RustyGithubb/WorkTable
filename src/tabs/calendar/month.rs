//! Vue Mois : 6 semaines ; jusqu'à 3 événements par jour, puis « +N ».
//! Clic sur un événement = détail ; clic sur le numéro = vue Jour ; double-clic = nouvel événement.

use chrono::{Datelike, Local};
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, vec2};

use super::{CalendarState, DAY_MS, View, at, color32, day_start, hhmm};
use crate::prefs::Prefs;
use crate::theme;

const WEEKDAYS: [&str; 7] = ["LUN.", "MAR.", "MER.", "JEU.", "VEN.", "SAM.", "DIM."];

pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &Prefs) {
    let days = cal.days();
    let size = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    let head = 22.0;
    let col_w = rect.width() / 7.0;
    let row_h = (rect.height() - head) / 6.0;
    let today = Local::now().date_naive();
    let month = cal.anchor.month();

    for (i, w) in WEEKDAYS.iter().enumerate() {
        p.text(Pos2::new(rect.left() + col_w * (i as f32 + 0.5), rect.top() + 9.0), Align2::CENTER_CENTER, *w, FontId::proportional(10.5), theme::TEXT_MUTED());
    }
    let mut open = None;
    let mut go_day = None;
    let mut create = None;
    for (k, d) in days.iter().enumerate() {
        let (c, r) = ((k % 7) as f32, (k / 7) as f32);
        let cell = Rect::from_min_size(Pos2::new(rect.left() + c * col_w, rect.top() + head + r * row_h), vec2(col_w, row_h));
        let resp = ui.interact(cell, ui.scope_id().with(("month_cell", k)), Sense::click());
        if d.month() != month {
            p.rect_filled(cell, CornerRadius::ZERO, Color32::from_black_alpha(45));
        }
        p.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, crate::theme::overlay(12)), egui::StrokeKind::Inside);

        // Numéro du jour (aujourd'hui en pastille blanche) : clic = vue Jour.
        let num = Rect::from_center_size(cell.left_top() + vec2(16.0, 14.0), vec2(24.0, 22.0));
        let num_resp = ui.interact(num, ui.scope_id().with(("month_num", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let color = if *d == today {
            p.circle_filled(num.center(), 11.0, theme::TODAY());
            Color32::WHITE
        } else if d.month() == month {
            theme::TEXT()
        } else {
            theme::TEXT_FAINT()
        };
        p.text(num.center(), Align2::CENTER_CENTER, d.day().to_string(), FontId::proportional(12.0), color);
        if num_resp.clicked() {
            go_day = Some(*d);
        }

        let (ds, de) = (day_start(*d), day_start(*d) + DAY_MS);
        let evs: Vec<_> = cal.events_between(prefs, ds, de).into_iter().cloned().collect();
        let max = (((row_h - 30.0) / 19.0).floor() as usize).max(1);
        for (j, e) in evs.iter().take(if evs.len() > max { max - 1 } else { max }).enumerate() {
            let er = Rect::from_min_size(cell.left_top() + vec2(4.0, 28.0 + j as f32 * 19.0), vec2(col_w - 8.0, 17.0));
            let col = color32(cal.color_of(e));
            let selected = cal.selected.as_ref().is_some_and(|(sc, si)| *sc == e.calendar && *si == e.id);
            let er_resp = ui.interact(er, ui.scope_id().with(("month_ev", k, j)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            if e.all_day || selected {
                p.rect_filled(er, CornerRadius::same(4), col.gamma_multiply(if selected { 0.7 } else { 0.45 }));
            } else if er_resp.hovered() {
                p.rect_filled(er, CornerRadius::same(4), crate::theme::overlay(10));
            }
            let tp = p.with_clip_rect(er);
            let mut x = er.left() + 6.0;
            if !e.all_day {
                tp.circle_filled(Pos2::new(x + 2.0, er.center().y), 3.0, col);
                x += 9.0;
                let t = hhmm(e.start);
                tp.text(Pos2::new(x, er.center().y), Align2::LEFT_CENTER, &t, FontId::proportional(10.5), theme::TEXT_MUTED());
                x += 34.0;
            }
            tp.text(Pos2::new(x, er.center().y), Align2::LEFT_CENTER, &e.title, theme::editor_font(11.0, e.all_day), theme::TEXT());
            if er_resp.clicked() {
                open = Some(e.clone());
            }
        }
        if evs.len() > max {
            let more = evs.len() - (max - 1);
            let mr = Rect::from_min_size(cell.left_top() + vec2(8.0, 28.0 + (max - 1) as f32 * 19.0), vec2(col_w - 16.0, 17.0));
            p.text(mr.left_center(), Align2::LEFT_CENTER, tf!("+{more} autres", "+{more} more"), FontId::proportional(10.5), theme::TEXT_MUTED());
            if ui.interact(mr, ui.scope_id().with(("month_more", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).clicked() {
                go_day = Some(*d);
            }
        }
        if resp.double_clicked() {
            create = Some(*d);
        }
    }
    if let Some(e) = open {
        cal.open_event(&e);
    }
    if let Some(d) = go_day {
        cal.view = View::Days(1);
        cal.go_to(d);
        cal.scroll_to_now = true;
    }
    if let Some(d) = create {
        cal.new_event(prefs, at(d, 9 * 60), at(d, 10 * 60), false);
    }
}
