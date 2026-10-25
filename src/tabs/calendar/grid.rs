//! Vue Jour / N jours / Semaine : grille heure par heure.
//!
//! - glisser sur une zone libre : crée un événement sur la plage (pas de 15 min) ;
//! - clic sur une zone libre : événement d'une heure ;
//! - glisser un événement : le déplace (même jour ou autre jour) ;
//! - glisser le bas d'un événement : change sa durée ;
//! - clic sur un événement : ouvre le panneau de détail.

use chrono::{Datelike, Local, NaiveDate, Timelike};
use eframe::egui::{self, Align2, Color32, CornerRadius, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, vec2};

use super::{CalendarState, DAY_MS, Drag, View, at, color32, date_of, day_start, hhmm, minutes_of, snap};
use crate::google::Account;
use crate::google::calendar::CalEvent;
use crate::prefs::Prefs;
use crate::theme;

const GUTTER: f32 = 54.0;
const HOUR_H: f32 = 46.0;
const HEADER_H: f32 = 46.0;
const ALLDAY_LANE: f32 = 22.0;
/// Rouge de la ligne « maintenant » (comme dans Notion Calendar).
const NOW: Color32 = Color32::from_rgb(0xEB, 0x57, 0x57);
pub fn show(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &Prefs, acc: &Account) {
    let days = cal.days();
    let n = days.len() as f32;
    let width = ui.available_width();
    let col_w = ((width - GUTTER - 10.0) / n).max(40.0);
    let today = Local::now().date_naive();

    // --- En-tête des jours -----------------------------------------------------------
    let (head, _) = ui.allocate_exact_size(vec2(width, HEADER_H), Sense::hover());
    for (i, d) in days.iter().enumerate() {
        let x = head.left() + GUTTER + i as f32 * col_w;
        let r = Rect::from_min_size(Pos2::new(x, head.top()), vec2(col_w, HEADER_H));
        let holiday = crate::tabs::timeline::holiday_on(&prefs.timeline, *d);
        let tip = match holiday.as_deref() {
            Some("") => t!("Vacances · clic : afficher ce jour", "Holidays · click: show this day").to_string(),
            Some(name) => tf!("Vacances : {name} · clic : afficher ce jour", "Holidays: {name} · click: show this day"),
            None => t!("Afficher ce jour (clic droit : vacances)", "Show this day (right-click: holidays)").to_string(),
        };
        let resp = ui.interact(r, ui.scope_id().with(("day_head", i)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tip);
        resp.context_menu(|ui| crate::tabs::timeline::day_menu(ui, &prefs.timeline, *d));
        let p = ui.painter();
        // Jour et numéro côte à côte : « MAR. 20 » (aujourd'hui en terracotta).
        let label = crate::tabs::weekday_short(d.weekday()).to_uppercase();
        let is_today = *d == today;
        let day_g = p.layout_no_wrap(label, FontId::proportional(11.0), if is_today { theme::TODAY() } else { theme::TEXT_MUTED() });
        let num_g = p.layout_no_wrap(d.day().to_string(), theme::editor_font(15.0, true), Color32::WHITE);
        let pill_w = num_g.size().x.max(14.0) + 12.0;
        let total = day_g.size().x + 6.0 + pill_w;
        // Colonnes étroites : le jour repasse au-dessus du numéro.
        let side_by_side = total <= col_w - 8.0;
        let cy = r.center().y;
        let (_x0, day_pos, pill) = if side_by_side {
            let x0 = r.center().x - total / 2.0;
            let pill = Rect::from_center_size(Pos2::new(x0 + day_g.size().x + 6.0 + pill_w / 2.0, cy), vec2(pill_w, 24.0));
            (x0, Pos2::new(x0, cy - day_g.size().y / 2.0), pill)
        } else {
            let x0 = r.center().x - day_g.size().x / 2.0;
            let pill = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 31.0), vec2(pill_w, 22.0));
            (x0, Pos2::new(x0, r.top() + 4.0), pill)
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
        p.text(pill.center(), Align2::CENTER_CENTER, d.day().to_string(), theme::editor_font(15.0, true), num_color);
        if resp.clicked() {
            cal.view = View::Days(1);
            cal.go_to(*d);
        }
    }

    // --- Journée entière -------------------------------------------------------------
    all_day_row(ui, cal, prefs, &days, col_w);
    ui.painter().hline(ui.min_rect().x_range(), ui.cursor().top(), Stroke::new(1.0, theme::BORDER()));

    // --- Grille horaire ------------------------------------------------------------------
    let mut area = egui::ScrollArea::vertical().id_salt("cal_grid").auto_shrink([false, false]);
    if cal.scroll_to_now {
        let now_min = (Local::now().hour() * 60) as f32;
        area = area.vertical_scroll_offset(((now_min / 60.0) - 2.0).max(0.0) * HOUR_H);
        cal.scroll_to_now = false;
    }
    area.show(ui, |ui| {
        let (rect, resp) = ui.allocate_exact_size(vec2(width - 10.0, 24.0 * HOUR_H), Sense::click_and_drag());
        let p = ui.painter().clone();
        let grid_left = rect.left() + GUTTER;
        let y_of = |min: i32| rect.top() + min as f32 / 60.0 * HOUR_H;
        let col_rect = |i: usize| Rect::from_min_size(Pos2::new(grid_left + i as f32 * col_w, rect.top()), vec2(col_w, rect.height()));

        // Fond : week-ends et vacances assombris, aujourd'hui légèrement éclairé.
        for (i, d) in days.iter().enumerate() {
            let wd = d.weekday().num_days_from_monday();
            if *d == today {
                p.rect_filled(col_rect(i), CornerRadius::ZERO, crate::theme::overlay(5));
            } else if wd >= 5 || crate::tabs::timeline::holiday_on(&prefs.timeline, *d).is_some() {
                p.rect_filled(col_rect(i), CornerRadius::ZERO, Color32::from_black_alpha(40));
            }
        }
        // Lignes des heures (pas de ligne aux demi-heures).
        for h in 1..24 {
            let y = y_of(h * 60);
            p.hline(grid_left..=rect.right(), y, Stroke::new(1.0, crate::theme::overlay(14)));
            p.text(Pos2::new(grid_left - 8.0, y), Align2::RIGHT_CENTER, format!("{h:02}:00"), FontId::proportional(10.5), theme::TEXT_FAINT());
        }
        for i in 0..=days.len() {
            let x = grid_left + i as f32 * col_w;
            p.vline(x, rect.y_range(), Stroke::new(1.0, crate::theme::overlay(12)));
        }

        // Événements, jour par jour (les événements sur plusieurs jours sont découpés).
        let mut hits: Vec<(Rect, CalEvent, bool)> = Vec::new(); // (zone, événement, fin dans ce jour)
        for (i, d) in days.iter().enumerate() {
            let (ds, de) = (day_start(*d), day_start(*d) + DAY_MS);
            let evs: Vec<CalEvent> = cal.events_between(prefs, ds, de).into_iter().filter(|e| !e.all_day).cloned().collect();
            for (e, col, cols) in layout_columns(&evs, ds, de) {
                let (s, en) = (e.start.max(ds), e.end.min(de));
                let (m0, m1) = (((s - ds) / 60_000) as i32, ((en - ds) / 60_000) as i32);
                let c = col_rect(i);
                let inner_w = c.width() - 8.0;
                let w = inner_w / cols as f32;
                let r = Rect::from_min_max(
                    Pos2::new(c.left() + 2.0 + col as f32 * w, y_of(m0) + 1.0),
                    Pos2::new(c.left() + 2.0 + (col + 1) as f32 * w - 2.0, y_of(m1).max(y_of(m0) + 18.0) - 1.0),
                );
                let dragged = matches!(&cal.drag, Some(Drag::Move { orig, .. } | Drag::Resize { orig, .. }) if orig.id == e.id);
                let selected = cal.selected.as_ref().is_some_and(|(sc, si)| *sc == e.calendar && *si == e.id);
                paint_event(&p, r, &e, cal.color_of(&e), selected, dragged, e.end < Local::now().timestamp_millis());
                hits.push((r, e.clone(), e.end <= de));
            }
        }

        // Nouvel événement en cours de saisie : visible dans la grille jusqu'à l'enregistrement.
        if let (None, Some(ed)) = (&cal.drag, &cal.editor)
            && ed.original.is_none()
            && !ed.ev.all_day
        {
            let ev = ed.build().unwrap_or_else(|_| ed.ev.clone());
            if let Some(i) = days.iter().position(|d| *d == date_of(ev.start)) {
                let c = col_rect(i);
                let (a, b) = (minutes_of(ev.start), if date_of(ev.end) == date_of(ev.start) { minutes_of(ev.end) } else { 24 * 60 });
                let r = Rect::from_min_max(Pos2::new(c.left() + 2.0, y_of(a) + 1.0), Pos2::new(c.right() - 6.0, y_of(b).max(y_of(a) + 18.0) - 1.0));
                let col = color32(ev.color.unwrap_or_else(|| cal.color_of(&ev)));
                p.rect(r, CornerRadius::same(6), col.gamma_multiply(0.45), Stroke::new(1.5, theme::TEXT()), StrokeKind::Inside);
                let title = if ev.title.trim().is_empty() { t!("(Nouvel événement)", "(New event)").to_string() } else { ev.title.clone() };
                let tp = p.with_clip_rect(r.shrink(1.0).intersect(p.clip_rect()));
                tp.text(r.left_top() + vec2(8.0, 5.0), Align2::LEFT_TOP, title, theme::editor_font(12.0, true), theme::TEXT());
                tp.text(r.left_top() + vec2(8.0, 21.0), Align2::LEFT_TOP, format!("{} – {}", fmt_min(a), fmt_min(b)), FontId::proportional(11.0), theme::TEXT_MUTED());
            }
        }

        // Fantôme pendant le geste en cours.
        if let Some(drag) = &cal.drag {
            let ghost = |day: NaiveDate, a: i32, b: i32| -> Option<Rect> {
                let i = days.iter().position(|d| *d == day)?;
                let c = col_rect(i);
                Some(Rect::from_min_max(Pos2::new(c.left() + 2.0, y_of(a.min(b))), Pos2::new(c.right() - 6.0, y_of(a.max(b)).max(y_of(a.min(b)) + 14.0))))
            };
            let (r, label, color) = match drag {
                Drag::Create { day, from, to } => {
                    let (a, b) = (*from.min(to), (*from.max(to)).max(from.min(to) + 15));
                    (ghost(*day, a, b), format!("{} – {}", fmt_min(a), fmt_min(b)), theme::TEXT())
                }
                Drag::Move { orig, day, start, .. } => {
                    let dur = ((orig.end - orig.start) / 60_000) as i32;
                    (ghost(*day, *start, (*start + dur).min(24 * 60)), format!("{} – {}", fmt_min(*start), fmt_min(*start + dur)), color32(cal.color_of(orig)))
                }
                Drag::Resize { orig, end } => {
                    let s = minutes_of(orig.start);
                    (ghost(date_of(orig.start), s, *end), format!("{} – {}", fmt_min(s), fmt_min(*end)), color32(cal.color_of(orig)))
                }
            };
            if let Some(r) = r {
                p.rect(r, CornerRadius::same(6), color.gamma_multiply(0.30), Stroke::new(1.5, color), StrokeKind::Inside);
                p.text(r.left_top() + vec2(8.0, 5.0), Align2::LEFT_TOP, label, FontId::proportional(11.5), theme::TEXT());
            }
        }

        // Ligne « maintenant ».
        if let Some(i) = days.iter().position(|d| *d == today) {
            let now = Local::now();
            let y = y_of((now.hour() * 60 + now.minute()) as i32);
            p.hline(grid_left..=rect.right(), y, Stroke::new(1.0, NOW.gamma_multiply(0.35)));
            let c = col_rect(i);
            p.hline(c.x_range(), y, Stroke::new(2.0, NOW));
            p.circle_filled(Pos2::new(c.left(), y), 4.5, NOW);
            let t = now.format("%H:%M").to_string();
            let tr = Rect::from_center_size(Pos2::new(grid_left - 26.0, y), vec2(42.0, 16.0));
            p.rect_filled(tr, CornerRadius::same(8), NOW);
            p.text(tr.center(), Align2::CENTER_CENTER, t, FontId::proportional(10.5), Color32::WHITE);
        }

        // Rappels minutés des tâches : barre à l'heure prévue et pastille ⏰ (clic = la tâche).
        for tm in &cal.timers {
            let Some(i) = days.iter().position(|d| *d == date_of(tm.due)) else { continue };
            let c = col_rect(i);
            let y = y_of(minutes_of(tm.due));
            let color = timer_color(tm);
            let dash = 5.0;
            let mut x = c.left() + 2.0;
            while x < c.right() - 6.0 {
                p.hline(x..=(x + dash).min(c.right() - 6.0), y, Stroke::new(1.5, color));
                x += dash * 1.8;
            }
            let label = format!("{} {}", hhmm(tm.due), tm.title);
            let g = p.layout_no_wrap(label, FontId::proportional(10.5), timer_ink());
            let w = (g.size().x + 30.0).min(c.width() - 10.0).max(30.0);
            let pill = Rect::from_min_size(Pos2::new(c.left() + 3.0, y - 9.0), vec2(w, 18.0));
            let r = ui.interact(pill, ui.scope_id().with(("wt_timer_pill", tm.id)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand).on_hover_text(tf!(
                "Rappel : {} — ouvrir la tâche dans le tableau de bord",
                "Reminder: {} — open the task in the dashboard",
                tm.title
            ));
            let fill = if r.hovered() { color } else { color.gamma_multiply(0.92) };
            p.rect_filled(pill, CornerRadius::same(9), fill);
            crate::widgets::alarm_icon(&p, pill.left_center() + vec2(11.0, 0.0), 13.0, timer_ink(), 1.0);
            p.with_clip_rect(pill.shrink(2.0).intersect(p.clip_rect())).galley(Pos2::new(pill.left() + 21.0, pill.center().y - g.size().y / 2.0), g, timer_ink());
            if r.clicked() {
                cal.open_timer = Some(tm.id);
            }
        }

        // Clic droit sur un événement : fiches contact des invités.
        let menu_id = ui.scope_id().with("wt_event_menu");
        if resp.secondary_clicked() {
            let e = resp.interact_pointer_pos().and_then(|pos| hits.iter().rev().find(|(r, ..)| r.contains(pos))).map(|(_, e, _)| e.attendees.clone());
            ui.ctx().data_mut(|d| d.insert_temp(menu_id, e));
        }
        resp.context_menu(|ui| match ui.ctx().data(|d| d.get_temp::<Option<Vec<String>>>(menu_id)).flatten() {
            Some(guests) => crate::people::guests_menu(ui, &guests),
            None => ui.close(),
        });
        interact(ui, cal, prefs, acc, &resp, rect, &days, col_w, &hits);
    });
}

/// Couleur des pastilles de rappel : ambre, ou couleur « aujourd'hui » une fois l'heure passée.
pub fn timer_color(t: &crate::tasks::Timer) -> Color32 {
    if t.due <= Local::now().timestamp_millis() { theme::TODAY() } else { Color32::from_rgb(0xF6, 0xBF, 0x26) }
}

/// Texte et logo sur une pastille de rappel.
pub fn timer_ink() -> Color32 {
    Color32::from_rgb(0x1C, 0x1C, 0x1C)
}

/// Disposition côte à côte des événements qui se chevauchent : (événement, colonne, nb colonnes).
fn layout_columns(evs: &[CalEvent], ds: i64, de: i64) -> Vec<(CalEvent, usize, usize)> {
    let mut sorted: Vec<&CalEvent> = evs.iter().collect();
    sorted.sort_by_key(|e| (e.start.max(ds), -(e.end.min(de))));
    let mut out = Vec::new();
    let mut cluster: Vec<(CalEvent, usize)> = Vec::new();
    let mut col_ends: Vec<i64> = Vec::new();
    let mut cluster_end = i64::MIN;
    let flush = |cluster: &mut Vec<(CalEvent, usize)>, col_ends: &mut Vec<i64>, out: &mut Vec<(CalEvent, usize, usize)>| {
        let cols = col_ends.len().max(1);
        out.extend(cluster.drain(..).map(|(e, c)| (e, c, cols)));
        col_ends.clear();
    };
    for e in sorted {
        let (s, en) = (e.start.max(ds), e.end.min(de).max(e.start.max(ds) + 15 * 60_000));
        if s >= cluster_end && !cluster.is_empty() {
            flush(&mut cluster, &mut col_ends, &mut out);
        }
        let col = col_ends.iter().position(|end| *end <= s).unwrap_or_else(|| {
            col_ends.push(0);
            col_ends.len() - 1
        });
        col_ends[col] = en;
        cluster_end = cluster_end.max(en);
        cluster.push((e.clone(), col));
    }
    flush(&mut cluster, &mut col_ends, &mut out);
    out
}

fn fmt_min(m: i32) -> String {
    let m = m.clamp(0, 24 * 60);
    format!("{:02}:{:02}", m / 60, m % 60)
}

fn paint_event(p: &egui::Painter, r: Rect, e: &CalEvent, color: [u8; 3], selected: bool, dragged: bool, past: bool) {
    let c = color32(color);
    let mut alpha = if selected { 0.6 } else { 0.36 };
    if past {
        alpha *= 0.65;
    }
    if dragged {
        alpha *= 0.4;
    }
    p.rect_filled(r, CornerRadius::same(6), c.gamma_multiply(alpha));
    p.rect_filled(Rect::from_min_size(r.min, vec2(3.0, r.height())), CornerRadius { nw: 6, sw: 6, ne: 0, se: 0 }, c);
    if selected {
        p.rect_stroke(r, CornerRadius::same(6), Stroke::new(1.5, theme::TEXT()), StrokeKind::Inside);
    }
    let tp = p.with_clip_rect(r.shrink(1.0).intersect(p.clip_rect()));
    let text = if past { crate::theme::gray(0xB0) } else { theme::TEXT() };
    let title = if e.title.is_empty() { t!("(Sans titre)", "(Untitled)") } else { &e.title };
    let x = r.left() + 8.0;
    if r.height() < 30.0 {
        // Événement court : titre et heure sur une ligne.
        tp.text(Pos2::new(x, r.center().y), Align2::LEFT_CENTER, format!("{title}  {}", hhmm(e.start)), theme::editor_font(11.5, true), text);
        return;
    }
    tp.text(Pos2::new(x, r.top() + 5.0), Align2::LEFT_TOP, title, theme::editor_font(12.0, true), text);
    tp.text(Pos2::new(x, r.top() + 21.0), Align2::LEFT_TOP, format!("{} – {}", hhmm(e.start), hhmm(e.end)), FontId::proportional(11.0), text.gamma_multiply(0.75));
    if r.height() > 52.0 && !e.location.is_empty() {
        tp.text(Pos2::new(x, r.top() + 36.0), Align2::LEFT_TOP, &e.location, FontId::proportional(11.0), text.gamma_multiply(0.6));
    }
}

#[allow(clippy::too_many_arguments)]
fn interact(
    ui: &egui::Ui,
    cal: &mut CalendarState,
    prefs: &Prefs,
    acc: &Account,
    resp: &egui::Response,
    rect: Rect,
    days: &[NaiveDate],
    col_w: f32,
    hits: &[(Rect, CalEvent, bool)],
) {
    let grid_left = rect.left() + GUTTER;
    let locate = |pos: Pos2| -> Option<(NaiveDate, i32)> {
        if pos.x < grid_left {
            return None;
        }
        let i = (((pos.x - grid_left) / col_w).floor() as usize).min(days.len() - 1);
        let m = (((pos.y - rect.top()) / HOUR_H) * 60.0) as i32;
        Some((days[i], m.clamp(0, 24 * 60)))
    };
    // Déplacement / durée à la souris : seulement pour l'événement en cours de modification
    // (bouton « Modifier » du panneau) ; sinon un clic l'affiche, sans rien changer.
    let writable = |e: &CalEvent| {
        cal.calendar(&e.calendar).is_some_and(|c| c.writable)
            && cal.editor.as_ref().is_some_and(|ed| ed.editing && ed.original.as_ref().is_some_and(|o| o.id == e.id && o.calendar == e.calendar))
    };
    let hit_at = |pos: Pos2| hits.iter().rev().find(|(r, ..)| r.contains(pos));

    // Curseur : poignée de durée en bas d'un événement, main sur un événement.
    if let Some(pos) = resp.hover_pos()
        && let Some((r, e, ends_here)) = hit_at(pos)
    {
        if writable(e) && *ends_here && pos.y > r.bottom() - 6.0 {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeVertical);
        } else if writable(e) {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        } else {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
    }

    if resp.drag_started() {
        let origin = ui.input(|i| i.pointer.press_origin()).unwrap_or_default();
        cal.drag = match (hit_at(origin), locate(origin)) {
            (Some((r, e, ends_here)), Some((day, m))) if writable(e) => {
                if *ends_here && origin.y > r.bottom() - 6.0 {
                    Some(Drag::Resize { orig: e.clone(), end: minutes_of(e.end) })
                } else {
                    let grab = if date_of(e.start) == day { m - minutes_of(e.start) } else { 0 };
                    Some(Drag::Move { orig: e.clone(), grab, day: date_of(e.start), start: minutes_of(e.start) })
                }
            }
            (Some(_), _) => None, // agenda en lecture seule
            (None, Some((day, m))) => {
                let a = snap(m, 15).min(24 * 60 - 15);
                Some(Drag::Create { day, from: a, to: a + 15 })
            }
            _ => None,
        };
    }
    if resp.dragged() {
        if let (Some(pos), Some(drag)) = (resp.interact_pointer_pos(), cal.drag.as_mut())
            && let Some((day, m)) = locate(pos)
        {
            match drag {
                Drag::Create { day: d0, to, .. } => {
                    if day == *d0 {
                        *to = snap(m, 15);
                    }
                }
                Drag::Move { orig, grab, day: d, start } => {
                    let dur = ((orig.end - orig.start) / 60_000) as i32;
                    *d = day;
                    *start = snap(m - *grab, 15).clamp(0, (24 * 60 - dur).max(0));
                }
                Drag::Resize { orig, end } => {
                    *end = snap(m, 15).max(minutes_of(orig.start) + 15);
                }
            }
        }
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if resp.drag_stopped() {
        match cal.drag.take() {
            Some(Drag::Create { day, from, to }) => {
                let (mut a, mut b) = (from.min(to), from.max(to));
                if b - a < 15 {
                    b = a + 60;
                }
                if b > 24 * 60 {
                    a -= b - 24 * 60;
                    b = 24 * 60;
                }
                cal.new_event(prefs, at(day, a), at(day, b), false);
            }
            Some(Drag::Move { orig, day, start, .. }) => {
                let mut e = orig.clone();
                let dur = e.end - e.start;
                e.start = at(day, start);
                e.end = e.start + dur;
                if e.start != orig.start {
                    cal.commit_drag(acc, e);
                }
            }
            Some(Drag::Resize { orig, end }) => {
                let mut e = orig.clone();
                e.end = at(date_of(orig.start), end);
                if e.end != orig.end && e.end > e.start {
                    cal.commit_drag(acc, e);
                }
            }
            None => {}
        }
    }
    if resp.clicked()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        match hit_at(pos) {
            Some((_, e, _)) => cal.open_event(e),
            None => {
                if let Some((day, m)) = locate(pos) {
                    let a = snap(m - 15, 30).min(23 * 60);
                    cal.new_event(prefs, at(day, a), at(day, a + 60), false);
                }
            }
        }
    }
}

/// Bandeau « journée entière » (événements sans heure, éventuellement sur plusieurs jours).
fn all_day_row(ui: &mut egui::Ui, cal: &mut CalendarState, prefs: &Prefs, days: &[NaiveDate], col_w: f32) {
    let (from, to) = (day_start(days[0]), day_start(*days.last().unwrap()) + DAY_MS);
    let evs: Vec<CalEvent> = cal.events_between(prefs, from, to).into_iter().filter(|e| e.all_day).cloned().collect();
    // Couloirs : un événement occupe ses colonnes sur un couloir libre.
    let mut lanes: Vec<Vec<(usize, usize)>> = Vec::new();
    let mut placed: Vec<(CalEvent, usize, usize, usize)> = Vec::new();
    for e in evs {
        let a = days.iter().position(|d| day_start(*d) + DAY_MS > e.start).unwrap_or(0);
        let b = days.iter().rposition(|d| day_start(*d) < e.end).unwrap_or(days.len() - 1);
        let lane = lanes.iter().position(|l| l.iter().all(|(x, y)| b < *x || a > *y)).unwrap_or_else(|| {
            lanes.push(Vec::new());
            lanes.len() - 1
        });
        lanes[lane].push((a, b));
        placed.push((e, a, b, lane));
    }
    let shown_lanes = lanes.len().clamp(1, 3);
    let h = shown_lanes as f32 * ALLDAY_LANE + 8.0;
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, h), Sense::click());
    let p = ui.painter().clone();
    p.text(Pos2::new(rect.left() + GUTTER - 8.0, rect.top() + 13.0), Align2::RIGHT_CENTER, t!("Journée", "All day"), FontId::proportional(10.0), theme::TEXT_FAINT());
    let grid_left = rect.left() + GUTTER;
    let mut clicked_event = None;
    for (e, a, b, lane) in &placed {
        if *lane >= 3 {
            continue;
        }
        let r = Rect::from_min_max(
            Pos2::new(grid_left + *a as f32 * col_w + 2.0, rect.top() + 4.0 + *lane as f32 * ALLDAY_LANE),
            Pos2::new(grid_left + (*b + 1) as f32 * col_w - 6.0, rect.top() + 4.0 + (*lane + 1) as f32 * ALLDAY_LANE - 3.0),
        );
        let c = color32(cal.color_of(e));
        let selected = cal.selected.as_ref().is_some_and(|(sc, si)| *sc == e.calendar && *si == e.id);
        p.rect_filled(r, CornerRadius::same(5), c.gamma_multiply(if selected { 0.7 } else { 0.45 }));
        p.with_clip_rect(r).text(r.left_center() + vec2(8.0, 0.0), Align2::LEFT_CENTER, &e.title, theme::editor_font(11.5, true), theme::TEXT());
        let er = ui.interact(r, ui.scope_id().with(("allday", &e.id)), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
        er.context_menu(|ui| crate::people::guests_menu(ui, &e.attendees));
        if er.clicked() {
            clicked_event = Some(e.clone());
        }
    }
    let hidden = placed.iter().filter(|(_, _, _, l)| *l >= 3).count();
    if hidden > 0 {
        p.text(Pos2::new(rect.right() - 8.0, rect.bottom() - 4.0), Align2::RIGHT_BOTTOM, format!("+{hidden}"), FontId::proportional(10.5), theme::TEXT_MUTED());
    }
    if let Some(e) = clicked_event {
        cal.open_event(&e);
    } else if resp.double_clicked() {
        // Double-clic : nouvel événement « journée entière » ce jour-là.
        if let Some(pos) = resp.interact_pointer_pos()
            && pos.x > grid_left
        {
            let i = (((pos.x - grid_left) / col_w) as usize).min(days.len() - 1);
            cal.new_event(prefs, day_start(days[i]), day_start(days[i]) + DAY_MS, true);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(id: &str, a: i32, b: i32) -> CalEvent {
        CalEvent { id: id.into(), start: a as i64 * 60_000, end: b as i64 * 60_000, ..Default::default() }
    }

    #[test]
    fn overlapping_events_share_columns() {
        let evs = vec![ev("a", 600, 660), ev("b", 630, 720), ev("c", 700, 760), ev("d", 800, 860)];
        let out = layout_columns(&evs, 0, DAY_MS);
        let find = |id: &str| out.iter().find(|(e, ..)| e.id == id).map(|(_, c, n)| (*c, *n)).unwrap();
        assert_eq!(find("a"), (0, 2));
        assert_eq!(find("b"), (1, 2));
        assert_eq!(find("c"), (0, 2), "réutilise la colonne libérée par a");
        assert_eq!(find("d"), (0, 1), "groupe séparé");
    }
}
