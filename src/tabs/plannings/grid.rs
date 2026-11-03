//! Vues Jour, N jours et Semaine : même grille que l'onglet Calendrier (en-têtes de jours en
//! pastille, 24 h défilables, week-ends assombris, ligne « maintenant »).
//! Planning d'une salle : cliquer-glisser sur la grille pour demander un créneau.

use chrono::{Datelike, Duration, Local, NaiveDate, NaiveDateTime, Timelike};
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};

use super::{Kind, PlanningsState, View, book, end_of, layout, paint_seance};
use crate::hyperplanning::cache::{Seance, Snapshot};
use crate::theme;

const GUTTER: f32 = 54.0;
const HOUR_H: f32 = 46.0;
const HEADER_H: f32 = 46.0;
const NOW: Color32 = Color32::from_rgb(0xEB, 0x57, 0x57);

pub fn show(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)]) {
    let days = st.days();
    let width = ui.available_width();
    let col_w = ((width - GUTTER - 10.0) / days.len() as f32).max(40.0);
    let today = Local::now().date_naive();

    // --- En-tête des jours (clic : vue Jour) ----------------------------------------------
    let (head, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    for (i, day) in days.iter().enumerate() {
        let r = Rect::from_min_size(Pos2::new(head.left() + GUTTER + i as f32 * col_w, head.top()), vec2(col_w, HEADER_H));
        let resp =
            ui.interact(r, ui.scope_id().with(("hp_day_head", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(t!("Afficher ce jour", "Show this day"));
        let p = ui.painter();
        let label = crate::tabs::weekday_short(day.weekday()).to_uppercase();
        let is_today = *day == today;
        let day_g = p.layout_no_wrap(label, FontId::proportional(11.0), if is_today { theme::TODAY() } else { theme::TEXT_MUTED() });
        let num_g = p.layout_no_wrap(day.day().to_string(), theme::editor_font(15.0, true), Color32::WHITE);
        let pill_w = num_g.size().x.max(14.0) + 12.0;
        let total = day_g.size().x + 6.0 + pill_w;
        // Colonnes étroites : le jour repasse au-dessus du numéro.
        let cy = r.center().y;
        let (day_pos, pill) = if total <= col_w - 8.0 {
            let x0 = r.center().x - total / 2.0;
            (Pos2::new(x0, cy - day_g.size().y / 2.0), Rect::from_center_size(Pos2::new(x0 + day_g.size().x + 6.0 + pill_w / 2.0, cy), vec2(pill_w, 24.0)))
        } else {
            let x0 = r.center().x - day_g.size().x / 2.0;
            (Pos2::new(x0, r.top() + 4.0), Rect::from_center_size(Pos2::new(r.center().x, r.top() + 31.0), vec2(pill_w, 22.0)))
        };
        let num_color = if is_today {
            p.rect_filled(pill, CornerRadius::same(12), theme::TODAY());
            Color32::WHITE
        } else {
            if resp.hovered() {
                p.rect_filled(pill, CornerRadius::same(12), theme::SURFACE_HI());
            }
            theme::TEXT()
        };
        p.galley(day_pos, day_g, theme::TEXT_MUTED());
        p.text(pill.center(), Align2::CENTER_CENTER, day.day().to_string(), theme::editor_font(15.0, true), num_color);
        if resp.clicked() {
            st.view = View::Days(1);
            st.go_to(*day);
        }
    }
    ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));

    // --- Grille horaire (24 h) --------------------------------------------------------------
    let mut area = egui::ScrollArea::vertical().id_salt("hp_grid").auto_shrink([false, false]);
    if st.scroll_to_morning {
        // Début des cours affichés (sinon 8 h), une demi-heure au-dessus.
        let start = seances.iter().map(|(_, s)| s.start.hour()).min().unwrap_or(8) as f32;
        area = area.vertical_scroll_offset((start - 0.5).max(0.0) * HOUR_H);
        st.scroll_to_morning = false;
    }
    area.show(ui, |ui| {
        // Réservation : seulement sur le planning d'une salle.
        let room = st.selected.filter(|s| s.0 == Kind::Salle).map(|s| s.1);
        let (rect, bg) = ui.allocate_exact_size(vec2(width - 10.0, 24.0 * HOUR_H), if room.is_some() { Sense::click_and_drag() } else { Sense::hover() });
        let p = ui.painter().clone();
        let grid_left = rect.left() + GUTTER;
        let y_min = |m: u32| rect.top() + m as f32 / 60.0 * HOUR_H;
        let y_of = |t: NaiveDateTime| y_min(t.hour() * 60 + t.minute());
        let col_rect = |i: usize| Rect::from_min_size(Pos2::new(grid_left + i as f32 * col_w, rect.top()), vec2(col_w, rect.height()));

        // Fond : week-ends assombris, aujourd'hui légèrement éclairé.
        for (i, day) in days.iter().enumerate() {
            if *day == today {
                p.rect_filled(col_rect(i), CornerRadius::ZERO, theme::overlay(5));
            } else if day.weekday().num_days_from_monday() >= 5 {
                p.rect_filled(col_rect(i), CornerRadius::ZERO, Color32::from_black_alpha(40));
            }
        }
        for h in 1..24 {
            let y = rect.top() + h as f32 * HOUR_H;
            p.hline(grid_left..=rect.right(), y, Stroke::new(1.0, theme::overlay(14)));
            p.text(Pos2::new(grid_left - 8.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), FontId::proportional(10.5), theme::TEXT_FAINT());
        }
        for i in 0..=days.len() {
            p.vline(grid_left + i as f32 * col_w, rect.y_range(), Stroke::new(1.0, theme::overlay(12)));
        }

        // Sélection d'un créneau (cliquer-glisser, ou clic : une heure), par demi-heure.
        let min_of = |y: f32| (((y - rect.top()) / HOUR_H * 60.0).clamp(0.0, 24.0 * 60.0)) as u32;
        let day_of = |x: f32| (x >= grid_left).then(|| ((x - grid_left) / col_w) as usize).filter(|i| *i < days.len());
        let pointer = bg.interact_pointer_pos();
        if bg.drag_started()
            && let Some(pos) = pointer
            && let Some(i) = day_of(pos.x)
        {
            st.drag = Some((days[i], min_of(pos.y), min_of(pos.y)));
        }
        if bg.dragged()
            && let (Some(pos), Some(dr)) = (pointer, st.drag.as_mut())
        {
            dr.2 = min_of(pos.y);
        }
        let mut picked = None;
        if bg.drag_stopped() {
            picked = st.drag.take().map(|(day, a, b)| (day, a.min(b), a.max(b)));
        } else if bg.clicked()
            && let Some(pos) = pointer
            && let Some(i) = day_of(pos.x)
        {
            let m = min_of(pos.y) / 30 * 30;
            picked = Some((days[i], m, (m + 60).min(24 * 60)));
        }

        // Séances, jour par jour (chevauchements côte à côte).
        let now = Local::now().naive_local();
        for (i, day) in days.iter().enumerate() {
            let of_day: Vec<(usize, &Seance)> = seances.iter().enumerate().filter(|(_, (_, s))| s.start.date() == *day).map(|(j, (_, s))| (j, s)).collect();
            for (j, col, cols) in layout(&of_day) {
                let (cours, s) = &seances[j];
                let c = col_rect(i);
                let w = (c.width() - 8.0) / cols as f32;
                let r = Rect::from_min_max(
                    Pos2::new(c.left() + 2.0 + col as f32 * w, y_of(s.start) + 1.0),
                    Pos2::new(c.left() + 2.0 + (col + 1) as f32 * w - 2.0, (y_of(end_of(s)) - 1.0).max(y_of(s.start) + 18.0)),
                );
                let selected = st.detail == Some((*cours, s.start));
                let resp = ui.interact(r, ui.scope_id().with(("hp_seance", *cours, s.start)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
                paint_seance(&p, r, d, s, st.selected.map(|x| x.0), selected || resp.hovered(), end_of(s) < now);
                if resp.clicked() {
                    st.detail = if selected { None } else { Some((*cours, s.start)) };
                }
            }
        }

        // Ligne « maintenant ».
        if let Some(i) = days.iter().position(|day| *day == now.date()) {
            let x = grid_left + i as f32 * col_w;
            let y = y_of(now);
            p.hline(x..=x + col_w, y, Stroke::new(2.0, NOW));
            p.circle_filled(Pos2::new(x, y), 4.0, NOW);
        }

        // Créneau en cours de sélection.
        if let Some((day, a, b)) = st.drag
            && let Some(i) = days.iter().position(|x| *x == day)
        {
            let (s0, s1) = snap(a.min(b), a.max(b));
            let x = grid_left + i as f32 * col_w;
            let r = Rect::from_min_max(Pos2::new(x + 3.0, y_min(s0) + 1.0), Pos2::new(x + col_w - 3.0, y_min(s1) - 1.0));
            p.rect_filled(r, CornerRadius::same(6), theme::TODAY().gamma_multiply(0.35));
            p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.5, theme::TODAY()), StrokeKind::Inside);
            p.text(r.left_top() + vec2(8.0, 5.0), Align2::LEFT_TOP, format!("{} – {}", fmt_min(s0), fmt_min(s1)), theme::editor_font(12.0, true), theme::TEXT());
        }
        if let (Some((day, a, b)), Some(room)) = (picked, room) {
            let (s0, s1) = snap(a, b);
            pick_slot(ui.ctx(), st, seances, room, day, s0, s1);
        }
    });
}

/// Créneau aligné sur la demi-heure (au moins une demi-heure, avant minuit).
fn snap(a: u32, b: u32) -> (u32, u32) {
    let start = (a / 30 * 30).min(24 * 60 - 30);
    let end = b.div_ceil(30) * 30;
    (start, end.max(start + 30).min(24 * 60))
}

fn fmt_min(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Créneau choisi sur la grille d'une salle : ouvre le panneau de réservation (ou explique
/// pourquoi non).
fn pick_slot(ctx: &egui::Context, st: &mut PlanningsState, seances: &[(u32, Seance)], room: u32, day: NaiveDate, start: u32, end: u32) {
    let at = |m: u32| day.and_hms_opt(0, 0, 0).unwrap_or_default() + Duration::minutes(m as i64);
    let (from, to) = (at(start), at(end));
    let notice = if to <= Local::now().naive_local() {
        Some(t!("Créneau passé", "Slot in the past"))
    } else if seances.iter().any(|(_, s)| s.start < to && end_of(s) > from) {
        Some(t!("Créneau déjà occupé dans cette salle", "Slot already taken in this room"))
    } else {
        None
    };
    match notice {
        Some(n) => st.notice = Some((n.to_string(), std::time::Instant::now())),
        None => st.book.open(ctx, book::Slot { date: day, start_min: start, end_min: end, room }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slots_snap_to_half_hours() {
        assert_eq!(snap(545, 600), (540, 600));
        assert_eq!(snap(540, 540), (540, 570));
        assert_eq!(snap(1435, 1440), (1410, 1440));
    }
}
