//! Vue Mois (comme dans l'onglet Calendrier) et année scolaire de la colonne de gauche.

use chrono::{Datelike, Duration, Local, NaiveDate};
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, RichText, Sense, Stroke, vec2};

use super::{PlanningsState, View, hhmm, matiere, monday_of, teacher_color};
use crate::hyperplanning::cache::{Seance, Snapshot};
use crate::theme;

const WEEKDAYS_FR: [&str; 7] = ["LUN.", "MAR.", "MER.", "JEU.", "VEN.", "SAM.", "DIM."];
const WEEKDAYS_EN: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];

/// Grille de 6 semaines : séances de chaque jour (couleur du prof), « +n » au-delà ; clic sur le
/// numéro du jour : vue Jour ; clic sur une séance : détail.
pub fn show(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot, seances: &[(u32, Seance)]) {
    let days = st.days();
    let size = ui.available_size();
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter().clone();
    let head = 22.0;
    let col_w = rect.width() / 7.0;
    let row_h = (rect.height() - head) / 6.0;
    let today = Local::now().date_naive();
    let month = st.anchor.month();
    let now = Local::now().naive_local();

    for (i, w) in t!(WEEKDAYS_FR, WEEKDAYS_EN).iter().enumerate() {
        p.text(Pos2::new(rect.left() + col_w * (i as f32 + 0.5), rect.top() + 9.0), Align2::CENTER_CENTER, *w, FontId::proportional(10.5), theme::TEXT_MUTED());
    }
    let mut go_day = None;
    for (k, day) in days.iter().enumerate() {
        let (c, r) = ((k % 7) as f32, (k / 7) as f32);
        let cell = Rect::from_min_size(Pos2::new(rect.left() + c * col_w, rect.top() + head + r * row_h), vec2(col_w, row_h));
        if day.month() != month {
            p.rect_filled(cell, CornerRadius::ZERO, Color32::from_black_alpha(45));
        }
        p.rect_stroke(cell, CornerRadius::ZERO, Stroke::new(1.0, theme::overlay(12)), egui::StrokeKind::Inside);

        // Numéro du jour (aujourd'hui en pastille) : clic = vue Jour.
        let num = Rect::from_center_size(cell.left_top() + vec2(16.0, 14.0), vec2(24.0, 22.0));
        let num_resp = ui.interact(num, ui.scope_id().with(("hp_month_num", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        let color = if *day == today {
            p.circle_filled(num.center(), 11.0, theme::TODAY());
            Color32::WHITE
        } else if day.month() == month {
            theme::TEXT()
        } else {
            theme::TEXT_FAINT()
        };
        p.text(num.center(), Align2::CENTER_CENTER, day.day().to_string(), FontId::proportional(12.0), color);
        if num_resp.clicked() {
            go_day = Some(*day);
        }

        let of_day: Vec<&(u32, Seance)> = seances.iter().filter(|(_, s)| s.start.date() == *day).collect();
        let max = (((row_h - 30.0) / 19.0).floor() as usize).max(1);
        let shown = if of_day.len() > max { max - 1 } else { max };
        for (j, (cours, s)) in of_day.iter().take(shown).enumerate() {
            let er = Rect::from_min_size(cell.left_top() + vec2(4.0, 28.0 + j as f32 * 19.0), vec2(col_w - 8.0, 17.0));
            let selected = st.detail == Some((*cours, s.start));
            let resp = ui.interact(er, ui.scope_id().with(("hp_month_ev", k, j)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            let col = s.enseignants.first().map(|t| teacher_color(*t)).unwrap_or(theme::TEXT_MUTED());
            if selected {
                p.rect_filled(er, CornerRadius::same(4), col.gamma_multiply(0.45));
            } else if resp.hovered() {
                p.rect_filled(er, CornerRadius::same(4), theme::overlay(10));
            }
            let past = super::end_of(s) < now;
            let ink = if past { theme::TEXT_MUTED() } else { theme::TEXT() };
            let tp = p.with_clip_rect(er);
            p.circle_filled(Pos2::new(er.left() + 7.0, er.center().y), 3.5, if past { col.gamma_multiply(0.55) } else { col });
            tp.text(Pos2::new(er.left() + 15.0, er.center().y), Align2::LEFT_CENTER, format!("{}  {}", hhmm(s.start), matiere(d, s)), FontId::proportional(11.0), ink);
            if resp.clicked() {
                st.detail = if selected { None } else { Some((*cours, s.start)) };
            }
        }
        if of_day.len() > shown {
            let y = cell.top() + 28.0 + shown as f32 * 19.0 + 8.0;
            let more = Rect::from_min_size(Pos2::new(cell.left() + 4.0, y - 8.0), vec2(col_w - 8.0, 17.0));
            let resp = ui.interact(more, ui.scope_id().with(("hp_month_more", k)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
            p.text(Pos2::new(cell.left() + 10.0, y), Align2::LEFT_CENTER, tf!("+{} autres", "+{} more", of_day.len() - shown), FontId::proportional(10.5), theme::TEXT_MUTED());
            if resp.clicked() {
                go_day = Some(*day);
            }
        }
    }
    if let Some(day) = go_day {
        st.view = View::Days(1);
        st.go_to(day);
        st.scroll_to_morning = true;
    }
}

/// Année scolaire de la colonne de gauche : une ligne par mois, une pastille par semaine
/// (numéro ISO, comme le badge « Semaine N ») ; clic : afficher cette semaine.
pub fn school_year(ui: &mut egui::Ui, st: &mut PlanningsState, d: &Snapshot) {
    let today = Local::now().date_naive();
    // Début d'année HYPERPLANNING (cache) ; sinon du 1er septembre au 31 août.
    let start = d.window.map(|(s, _)| s).unwrap_or_else(|| {
        let y = if today.month() >= 9 { today.year() } else { today.year() - 1 };
        NaiveDate::from_ymd_opt(y, 9, 1).unwrap_or(today)
    });
    let end = start.with_year(start.year() + 1).unwrap_or(start + Duration::days(365));
    ui.label(RichText::new(tf!("Année {}–{}", "Year {}–{}", start.year(), end.year())).strong());
    ui.add_space(6.0);

    // Semaines (lundis) groupées par mois du lundi.
    let mut months: Vec<(NaiveDate, Vec<NaiveDate>)> = Vec::new();
    let mut monday = monday_of(start);
    while monday < end {
        match months.last_mut() {
            Some((m, weeks)) if m.month() == monday.month() && m.year() == monday.year() => weeks.push(monday),
            _ => months.push((monday, vec![monday])),
        }
        monday += Duration::days(7);
    }

    // Semaines affichées dans la grille (vue Mois : celles du mois affiché).
    let shown: Vec<NaiveDate> = match st.view {
        View::Month => st.days().into_iter().filter(|x| x.month() == st.anchor.month()).collect(),
        View::Days(_) => st.days(),
    };
    let label_w = 42.0;
    let w = ui.available_width();
    let chip_w = ((w - label_w) / 5.0).floor();
    let row_h = 24.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, row_h * months.len() as f32), Sense::hover());
    let p = ui.painter().clone();
    let mut go = None;
    for (r, (m, weeks)) in months.iter().enumerate() {
        let y = rect.top() + r as f32 * row_h;
        p.text(Pos2::new(rect.left(), y + row_h / 2.0), Align2::LEFT_CENTER, crate::tabs::month_short(m.month()).to_uppercase(), FontId::proportional(10.0), theme::TEXT_FAINT());
        for (c, mon) in weeks.iter().enumerate() {
            let cell = Rect::from_min_size(Pos2::new(rect.left() + label_w + c as f32 * chip_w, y), vec2(chip_w, row_h)).shrink2(vec2(2.0, 2.0));
            let resp = ui.interact(cell, ui.scope_id().with(("hp_year_week", *mon)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tf!(
                "Semaine du {}",
                "Week of {}",
                crate::tabs::long_date(*mon)
            ));
            let current = (0..7).any(|k| *mon + Duration::days(k) == today);
            let visible = (0..7).any(|k| shown.contains(&(*mon + Duration::days(k))));
            let color = if current {
                p.rect_filled(cell, CornerRadius::same(6), theme::TODAY());
                Color32::WHITE
            } else {
                if visible {
                    p.rect_filled(cell, CornerRadius::same(6), theme::overlay(16));
                } else if resp.hovered() {
                    p.rect_filled(cell, CornerRadius::same(6), theme::SURFACE_HI());
                }
                if *mon + Duration::days(6) < today { theme::TEXT_FAINT() } else { theme::gray(0xCC) }
            };
            if visible && current {
                p.rect_stroke(cell, CornerRadius::same(6), Stroke::new(1.5, Color32::WHITE), egui::StrokeKind::Inside);
            }
            p.text(cell.center(), Align2::CENTER_CENTER, mon.iso_week().week().to_string(), FontId::proportional(11.5), color);
            if resp.clicked() {
                go = Some(*mon);
            }
        }
    }
    if let Some(mon) = go {
        if st.view == View::Month {
            st.view = View::Days(7);
        }
        st.go_to(mon);
        st.scroll_to_morning = true;
    }
}
