//! Salles libres sur un créneau, calculées dans la copie locale (aucune requête).
//!
//! Une salle est occupée par ses propres séances, par celles de son groupe de salles, et un
//! groupe de salles l'est dès qu'une de ses salles l'est.
//!
//! « Demander » : demande de réservation envoyée à l'espace web HyperPlanning.net, après
//! confirmation (motif facultatif) ; un administrateur la valide ou la refuse.

use chrono::{Duration, Local, NaiveDate, NaiveDateTime, Timelike};
use eframe::egui::{self, RichText};
use std::collections::{BTreeMap, BTreeSet};

use crate::hyperplanning::RequestStatus;
use crate::hyperplanning::cache::Snapshot;
use crate::{theme, widgets};

/// Pas des horaires proposés (minutes) et plage de la journée.
const STEP: u32 = 30;
const DAY_START: u32 = 7 * 60;
const DAY_END: u32 = 22 * 60;

#[derive(Clone, Debug, PartialEq)]
pub struct FreeRoom {
    pub key: u32,
    pub name: String,
    pub capacity: u32,
    /// Début de la prochaine occupation le même jour (après le créneau).
    pub until: Option<NaiveDateTime>,
}

type Key = (NaiveDate, u32, u32, u32, bool, i64);

pub struct FreeState {
    date: Option<NaiveDate>,
    /// Créneau en minutes depuis minuit.
    start: u32,
    end: u32,
    min_capacity: u32,
    groups: bool,
    filter: String,
    cache: Option<(Key, Vec<FreeRoom>)>,
    /// Salle dont le formulaire de demande est ouvert, et le motif saisi.
    asking: Option<u32>,
    comment: String,
}

/// Action demandée depuis la liste des salles libres.
pub enum Action {
    /// Voir le planning de la salle.
    Open(u32),
    /// Envoyer une demande de réservation (clé de la salle, demande).
    Request(u32, crate::hyperplanning::web::RoomRequest),
}

impl Default for FreeState {
    fn default() -> Self {
        // Créneau par défaut : la prochaine demi-heure, pour une heure.
        let now = Local::now().time();
        let m = (now.hour() * 60 + now.minute()).div_ceil(STEP) * STEP;
        let start = m.clamp(DAY_START, DAY_END - 60);
        Self { date: None, start, end: start + 60, min_capacity: 0, groups: false, filter: String::new(), cache: None, asking: None, comment: String::new() }
    }
}

impl FreeState {
    /// Jour choisi (aujourd'hui par défaut).
    pub fn date(&self) -> NaiveDate {
        self.date.unwrap_or_else(|| Local::now().date_naive())
    }
}

fn at(d: NaiveDate, min: u32) -> NaiveDateTime {
    d.and_hms_opt(0, 0, 0).unwrap_or_default() + Duration::minutes(min as i64)
}

/// Salles occupées par une séance qui utilise `salles` (groupes et salles des groupes compris).
fn occupied(salles: &[u32], members: &BTreeMap<u32, Vec<u32>>, d: &Snapshot) -> BTreeSet<u32> {
    let mut out = BTreeSet::new();
    for r in salles {
        out.insert(*r);
        if let Some(m) = members.get(r) {
            out.extend(m.iter().copied());
        }
        if let Some(g) = d.salles.get(r).and_then(|s| s.group) {
            out.insert(g);
        }
    }
    out
}

/// Salles libres sur [`from`, `to`[ (triées par nom).
pub fn free_rooms(d: &Snapshot, from: NaiveDateTime, to: NaiveDateTime, min_capacity: u32, groups: bool) -> Vec<FreeRoom> {
    let mut members: BTreeMap<u32, Vec<u32>> = BTreeMap::new();
    for (k, s) in &d.salles {
        if let Some(g) = s.group {
            members.entry(g).or_default().push(*k);
        }
    }
    let day_end = at(from.date(), 24 * 60);
    let mut busy = BTreeSet::new();
    let mut next: BTreeMap<u32, NaiveDateTime> = BTreeMap::new();
    for s in d.cours.values().flat_map(|c| c.seances.iter()) {
        let end = s.start + Duration::minutes(s.minutes as i64);
        if s.salles.is_empty() || end <= from || s.start >= day_end {
            continue;
        }
        let rooms = occupied(&s.salles, &members, d);
        if s.start < to {
            busy.extend(rooms);
        } else {
            for r in rooms {
                let e = next.entry(r).or_insert(s.start);
                *e = (*e).min(s.start);
            }
        }
    }
    let mut out: Vec<FreeRoom> = d
        .salles
        .iter()
        .filter(|(k, s)| !busy.contains(k) && s.capacity >= min_capacity && (groups || !s.is_group) && !s.name.trim().is_empty())
        .map(|(k, s)| FreeRoom { key: *k, name: s.name.clone(), capacity: s.capacity, until: next.get(k).copied() })
        .collect();
    out.sort_by_key(|r| r.name.to_lowercase());
    out
}

fn hhmm(min: u32) -> String {
    format!("{:02}:{:02}", min / 60, min % 60)
}

fn time_combo(ui: &mut egui::Ui, salt: &str, value: &mut u32, range: std::ops::RangeInclusive<u32>) {
    egui::ComboBox::from_id_salt(salt).selected_text(hhmm(*value)).width(80.0).show_ui(ui, |ui| {
        for m in range.step_by(STEP as usize) {
            ui.selectable_value(value, m, hhmm(m));
        }
    });
}

/// Formulaire (colonne de gauche).
pub fn form(ui: &mut egui::Ui, st: &mut FreeState) {
    ui.spacing_mut().item_spacing.y = 8.0;
    ui.label(RichText::new(t!("Jour", "Day")).small().color(theme::TEXT_FAINT()));
    let day = st.date();
    ui.horizontal(|ui| {
        if ui.button("‹").clicked() {
            st.date = Some(day - Duration::days(1));
        }
        ui.label(crate::tabs::long_date(day));
        if ui.button("›").clicked() {
            st.date = Some(day + Duration::days(1));
        }
    });
    if st.date.is_some() && ui.small_button(t!("Aujourd'hui", "Today")).clicked() {
        st.date = None;
    }
    ui.add_space(4.0);
    ui.label(RichText::new(t!("Créneau", "Time slot")).small().color(theme::TEXT_FAINT()));
    ui.horizontal(|ui| {
        ui.label(t!("De", "From"));
        time_combo(ui, "hp_free_start", &mut st.start, DAY_START..=DAY_END - STEP);
        ui.label(t!("à", "to"));
        time_combo(ui, "hp_free_end", &mut st.end, DAY_START + STEP..=DAY_END);
    });
    // Début déplacé après la fin : la fin suit (même durée minimale d'un pas).
    if st.end <= st.start {
        st.end = (st.start + 60).min(DAY_END);
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.label(t!("Capacité min.", "Min. capacity"));
        ui.add(egui::DragValue::new(&mut st.min_capacity).range(0..=1000).suffix(t!(" places", " seats")));
    });
    ui.checkbox(&mut st.groups, t!("Inclure les groupes de salles", "Include room groups"));
    ui.add(egui::TextEdit::singleline(&mut st.filter).hint_text(t!("Filtrer les salles…", "Filter rooms…")).desired_width(f32::INFINITY));
}

/// Résultats (colonne de droite) ; renvoie la salle dont on veut voir le planning.
/// `web_url` : espace web HyperPlanning.net (demandes de réservation) ; `status` : demande en cours.
pub fn results(ui: &mut egui::Ui, st: &mut FreeState, d: &Snapshot, web_url: &str, status: &RequestStatus) -> Option<Action> {
    let day = st.date();
    if st.end <= st.start {
        widgets::empty_state(ui, t!("Créneau invalide", "Invalid time slot"), t!("L'heure de fin doit suivre l'heure de début.", "The end time must be after the start time."));
        return None;
    }
    let key = (day, st.start, st.end, st.min_capacity, st.groups, d.synced_at);
    if st.cache.as_ref().map(|c| c.0) != Some(key) {
        st.cache = Some((key, free_rooms(d, at(day, st.start), at(day, st.end), st.min_capacity, st.groups)));
    }
    let rooms = &st.cache.as_ref()?.1;
    let needle = st.filter.trim().to_lowercase();
    let shown: Vec<&FreeRoom> = rooms.iter().filter(|r| needle.is_empty() || r.name.to_lowercase().contains(&needle)).collect();

    let when = format!("{} {} · {} – {}", crate::tabs::weekday_short(chrono::Datelike::weekday(&day)), chrono::Datelike::day(&day), hhmm(st.start), hhmm(st.end));
    ui.horizontal(|ui| {
        ui.label(widgets::section_title(&tf!("{} salle(s) libre(s)", "{} free room(s)", shown.len())));
        ui.label(RichText::new(when).color(theme::TEXT_MUTED()));
    });
    if d.window.is_some_and(|(a, b)| day < a || day >= b) {
        ui.label(RichText::new(t!("Jour hors de l'année synchronisée : résultat non fiable.", "Day outside the synced year: result not reliable.")).small().color(theme::TODAY()));
    }
    ui.add_space(6.0);
    if shown.is_empty() {
        widgets::empty_state(ui, t!("Aucune salle libre", "No free room"), t!("Essaie un autre créneau ou une capacité plus faible.", "Try another slot or a lower capacity."));
        return None;
    }
    let mut action = None;
    let sending = matches!(status, RequestStatus::Sending(_));
    egui::ScrollArea::vertical().id_salt("hp_free").auto_shrink([false, false]).show(ui, |ui| {
        for (i, r) in shown.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(&r.name).strong());
                    let until = match r.until {
                        Some(t) => tf!("Libre jusqu'à {}", "Free until {}", t.format("%H:%M")),
                        None => t!("Libre le reste de la journée", "Free for the rest of the day").into(),
                    };
                    let cap = if r.capacity > 0 { tf!("{} places · ", "{} seats · ", r.capacity) } else { String::new() };
                    ui.label(RichText::new(format!("{cap}{until}")).small().color(theme::TEXT_FAINT()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let open_form = st.asking == Some(r.key);
                    if ui
                        .add_enabled(!sending, egui::Button::selectable(open_form, t!("Demander", "Request")))
                        .on_hover_text(t!("Demander la réservation de cette salle", "Request this room"))
                        .clicked()
                    {
                        st.asking = if open_form { None } else { Some(r.key) };
                    }
                    if ui.button(t!("Planning", "Timetable")).on_hover_text(t!("Voir le planning de la salle", "Show the room's timetable")).clicked() {
                        action = Some(Action::Open(r.key));
                    }
                });
            });
            // Formulaire de demande : récapitulatif, motif, confirmation.
            if st.asking == Some(r.key) {
                egui::Frame::new().fill(theme::SURFACE_HI()).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(tf!("Réserver {} — {}, de {} à {}", "Book {} — {}, from {} to {}", r.name, crate::tabs::long_date(day), hhmm(st.start), hhmm(st.end)));
                    ui.add(egui::TextEdit::singleline(&mut st.comment).hint_text(t!("Motif (facultatif)", "Reason (optional)")).desired_width(f32::INFINITY));
                    ui.horizontal(|ui| {
                        let send = ui.add_enabled(!sending, egui::Button::new(RichText::new(t!("Envoyer la demande", "Send the request")).strong()));
                        if send.clicked() {
                            let req = crate::hyperplanning::web::RoomRequest { room: r.name.clone(), date: day, start_min: st.start, end_min: st.end, comment: st.comment.clone() };
                            action = Some(Action::Request(r.key, req));
                        }
                        if ui.button(t!("Annuler", "Cancel")).clicked() {
                            st.asking = None;
                        }
                        if !web_url.trim().is_empty() && ui.link(t!("ou faire la demande sur l'espace web", "or request it on the web space")).clicked() {
                            let _ = open::that_detached(web_url.trim());
                        }
                    });
                    ui.label(
                        RichText::new(t!("La demande est validée ou refusée par un administrateur.", "The request is approved or refused by an administrator."))
                            .small()
                            .color(theme::TEXT_FAINT()),
                    );
                });
            }
            match status {
                RequestStatus::Sending(k) if *k == r.key => {
                    ui.horizontal(|ui| {
                        ui.add(egui::Spinner::new().size(12.0));
                        ui.label(RichText::new(t!("Envoi de la demande…", "Sending the request…")).small());
                    });
                }
                RequestStatus::Sent(k) if *k == r.key => {
                    ui.label(RichText::new(t!("✓ Demande envoyée — en attente de validation", "✓ Request sent — awaiting approval")).small().color(theme::TODAY()));
                }
                RequestStatus::Failed(k, e) if *k == r.key => {
                    ui.add(egui::Label::new(RichText::new(tf!("Demande non envoyée : {e}", "Request not sent: {e}")).small().color(theme::TEXT())).wrap());
                }
                _ => {}
            }
        }
    });
    if let Some(Action::Request(..)) = action {
        st.asking = None;
        st.comment.clear();
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hyperplanning::cache::{Cours, Salle, Seance};

    fn seance(h: u32, min: u32, salles: Vec<u32>) -> Seance {
        Seance {
            start: NaiveDate::from_ymd_opt(2026, 11, 2).unwrap().and_hms_opt(h, 0, 0).unwrap(),
            minutes: min,
            matiere: 0,
            type_cours: String::new(),
            enseignants: vec![],
            salles,
            promotions: vec![],
            tdoptions: vec![],
            regroupements: vec![],
        }
    }

    #[test]
    fn busy_rooms_and_groups_are_excluded() {
        let mut d = Snapshot::default();
        let salle = |n: &str, cap, is_group, group| Salle { name: n.into(), capacity: cap, is_group, group };
        d.salles.insert(1, salle("A", 20, false, Some(9)));
        d.salles.insert(2, salle("B", 30, false, Some(9)));
        d.salles.insert(3, salle("C", 10, false, None));
        d.salles.insert(4, salle("D", 40, false, None));
        d.salles.insert(9, salle("A+B", 50, true, None));
        // A occupée 9 h – 11 h (donc son groupe aussi) ; D occupée à 14 h.
        d.cours.insert(1, Cours { memo: String::new(), seances: vec![seance(9, 120, vec![1]), seance(14, 60, vec![4])] });
        let day = NaiveDate::from_ymd_opt(2026, 11, 2).unwrap();
        let free = free_rooms(&d, at(day, 10 * 60), at(day, 11 * 60), 0, true);
        let names: Vec<&str> = free.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["B", "C", "D"]);
        assert_eq!(free.iter().find(|r| r.name == "D").unwrap().until.unwrap().hour(), 14);
        // Groupe occupé par son contenu ; capacité minimale.
        d.cours.insert(2, Cours { memo: String::new(), seances: vec![seance(10, 60, vec![9])] });
        let free = free_rooms(&d, at(day, 10 * 60), at(day, 11 * 60), 25, true);
        let names: Vec<&str> = free.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["D"]);
    }
}
