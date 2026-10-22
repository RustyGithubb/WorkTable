pub mod calendar;
pub mod dashboard;
pub mod files;
pub mod mail;
pub mod messages;
pub mod notes;
pub mod settings;

use chrono::{Datelike, Weekday};

pub const WEEKDAYS_FR: [&str; 7] = ["lundi", "mardi", "mercredi", "jeudi", "vendredi", "samedi", "dimanche"];
pub const WEEKDAYS_EN: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
pub const MONTHS_FR: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
pub const MONTHS_EN: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/// Nom du jour dans la langue de l'interface (« lundi », « Monday »).
pub fn weekday_name(d: Weekday) -> &'static str {
    t!(WEEKDAYS_FR, WEEKDAYS_EN)[d.num_days_from_monday() as usize]
}

/// Jour abrégé (« lun. », « Mon »).
pub fn weekday_short(d: Weekday) -> &'static str {
    const FR: [&str; 7] = ["lun.", "mar.", "mer.", "jeu.", "ven.", "sam.", "dim."];
    const EN: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    t!(FR, EN)[d.num_days_from_monday() as usize]
}

/// Nom du mois (1 à 12) dans la langue de l'interface.
pub fn month_name(m: u32) -> &'static str {
    t!(MONTHS_FR, MONTHS_EN)[(m as usize).saturating_sub(1) % 12]
}

/// Mois abrégé (« janv. », « Jan »).
pub fn month_short(m: u32) -> &'static str {
    const FR: [&str; 12] = ["janv.", "févr.", "mars", "avr.", "mai", "juin", "juil.", "août", "sept.", "oct.", "nov.", "déc."];
    const EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    t!(FR, EN)[(m as usize).saturating_sub(1) % 12]
}

/// Date longue : « jeudi 22 octobre 2026 » / « Thursday 22 October 2026 ».
pub fn long_date(d: chrono::NaiveDate) -> String {
    format!("{} {} {} {}", weekday_name(d.weekday()), d.day(), month_name(d.month()), d.year())
}

pub fn today_long() -> String {
    long_date(chrono::Local::now().date_naive())
}
