//! Tâches du tableau de bord (petit Kanban) : À faire → En cours → Archivé.
//! Le bloc « Archivé » repart à zéro chaque jour : les tâches archivées les jours précédents
//! passent dans l'historique des archives. Sauvegarde locale : tasks.json.
//!
//! Rappels minutés : une tâche peut être rappelée à une heure donnée (après un délai, ou à une
//! date fixe) ; notification à l'échéance, pastille dans le calendrier.

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Todo,
    Doing,
    Done,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Todo, Status::Doing, Status::Done];

    pub fn label(self) -> &'static str {
        match self {
            Status::Todo => t!("À faire", "To do"),
            Status::Doing => t!("En cours", "In progress"),
            Status::Done => t!("Archivé", "Archived"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub title: String,
    pub status: Status,
    /// Création (ms).
    pub created: i64,
    /// Entrée dans l'état actuel (ms) : « en cours depuis… ».
    pub since: i64,
    /// Résumé sur plusieurs lignes (notes, étapes…), facultatif.
    #[serde(default)]
    pub summary: String,
}

/// Rappel minuté d'une tâche.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Timer {
    pub id: u64,
    /// Tâche rappelée (Kanban).
    pub task: u64,
    pub title: String,
    /// Échéance (ms).
    pub due: i64,
    pub created: i64,
    /// Notification déjà affichée.
    #[serde(default)]
    pub fired: bool,
}

#[derive(Default, Serialize, Deserialize)]
pub struct Tasks {
    pub list: Vec<Task>,
    /// Tâches archivées les jours précédents (plus récentes en premier).
    pub archive: Vec<Task>,
    next_id: u64,
    /// Rappels minutés (du plus proche au plus lointain).
    #[serde(default)]
    pub timers: Vec<Timer>,
    #[serde(default)]
    next_timer: u64,
    #[serde(skip)]
    dirty: bool,
}

/// Une tâche à faire depuis plus longtemps est rappelée (jours).
pub const TODO_REMIND_DAYS: i64 = 1;
/// Une tâche en cours depuis plus longtemps est rappelée (jours).
pub const DOING_REMIND_DAYS: i64 = 3;
const DAY_MS: i64 = 86_400_000;

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("tasks.json"))
}

fn now() -> i64 {
    Local::now().timestamp_millis()
}

fn day_of(ms: i64) -> chrono::NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

impl Tasks {
    pub fn load() -> Self {
        let mut t: Tasks = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        t.roll_over(now());
        t
    }

    pub fn save_if_needed(&mut self) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        if let Some(p) = path() {
            crate::paths::save_json(&p, self);
        }
    }

    /// Remise à zéro quotidienne du bloc « Archivé » (vers l'historique).
    pub fn roll_over(&mut self, at: i64) {
        let today = day_of(at);
        let (old, keep): (Vec<Task>, Vec<Task>) = std::mem::take(&mut self.list).into_iter().partition(|t| t.status == Status::Done && day_of(t.since) < today);
        self.list = keep;
        if !old.is_empty() {
            let mut old = old;
            old.sort_by_key(|t| -t.since);
            for t in old.into_iter().rev() {
                self.archive.insert(0, t);
            }
            self.archive.sort_by_key(|t| -t.since);
            self.dirty = true;
        }
    }

    pub fn add(&mut self, title: &str) {
        self.add_task(title);
    }

    /// Ajoute une tâche « À faire » ; renvoie son identifiant.
    pub fn add_task(&mut self, title: &str) -> Option<u64> {
        let title = title.trim();
        if title.is_empty() {
            return None;
        }
        self.next_id = self.next_id.max(self.list.iter().chain(&self.archive).map(|t| t.id).max().unwrap_or(0)) + 1;
        let n = now();
        self.list.push(Task { id: self.next_id, title: title.to_string(), status: Status::Todo, created: n, since: n, summary: String::new() });
        self.dirty = true;
        Some(self.next_id)
    }

    // --- Rappels minutés -------------------------------------------------------------------

    /// Programme le rappel d'une tâche à l'échéance `due` (ms) ; renvoie son identifiant.
    pub fn add_timer(&mut self, task: u64, due: i64) -> Option<u64> {
        let title = self.list.iter().find(|t| t.id == task)?.title.clone();
        self.next_timer = self.next_timer.max(self.timers.iter().map(|t| t.id).max().unwrap_or(0)) + 1;
        self.timers.push(Timer { id: self.next_timer, task, title, due, created: now(), fired: false });
        self.timers.sort_by_key(|t| t.due);
        self.dirty = true;
        Some(self.next_timer)
    }

    pub fn remove_timer(&mut self, id: u64) {
        self.timers.retain(|t| t.id != id);
        self.dirty = true;
    }

    /// Repousse un rappel de `minutes` à partir de maintenant.
    pub fn snooze(&mut self, id: u64, minutes: i64) {
        if let Some(t) = self.timers.iter_mut().find(|t| t.id == id) {
            t.due = now() + minutes * 60_000;
            t.fired = false;
            self.timers.sort_by_key(|t| t.due);
            self.dirty = true;
        }
    }

    /// Rappels arrivés à échéance (marqués comme notifiés).
    pub fn fire_due(&mut self, at: i64) -> Vec<Timer> {
        let mut out = Vec::new();
        for t in self.timers.iter_mut().filter(|t| !t.fired && t.due <= at) {
            t.fired = true;
            out.push(t.clone());
        }
        if !out.is_empty() {
            self.dirty = true;
        }
        out
    }

    pub fn set_status(&mut self, id: u64, status: Status) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id)
            && t.status != status
        {
            t.status = status;
            t.since = now();
            self.dirty = true;
        }
        // Tâche terminée : ses rappels n'ont plus lieu d'être.
        if status == Status::Done {
            self.timers.retain(|t| t.task != id);
        }
    }

    pub fn rename(&mut self, id: u64, title: &str) {
        if let Some(t) = self.list.iter_mut().find(|t| t.id == id) {
            t.title = title.trim().to_string();
            self.dirty = true;
        }
        for t in self.timers.iter_mut().filter(|t| t.task == id) {
            t.title = title.trim().to_string();
        }
    }

    /// Remplace le résumé d'une tâche (lignes vides de fin retirées).
    pub fn set_summary(&mut self, id: u64, summary: &str) {
        if let Some(t) = self.list.iter_mut().chain(self.archive.iter_mut()).find(|t| t.id == id) {
            let s = summary.trim_end().to_string();
            if t.summary != s {
                t.summary = s;
                self.dirty = true;
            }
        }
    }

    pub fn remove(&mut self, id: u64) {
        self.list.retain(|t| t.id != id);
        self.archive.retain(|t| t.id != id);
        self.timers.retain(|t| t.task != id);
        self.dirty = true;
    }

    /// Remet une tâche de l'historique dans « À faire ».
    pub fn restore(&mut self, id: u64) {
        if let Some(i) = self.archive.iter().position(|t| t.id == id) {
            let mut t = self.archive.remove(i);
            t.status = Status::Todo;
            t.since = now();
            self.list.push(t);
            self.dirty = true;
        }
    }

    pub fn column(&self, s: Status) -> Vec<&Task> {
        let mut v: Vec<&Task> = self.list.iter().filter(|t| t.status == s).collect();
        v.sort_by_key(|t| t.since);
        v
    }

    /// Tâches oubliées : à faire depuis les jours précédents, ou en cours depuis trop longtemps.
    pub fn reminders(&self, at: i64) -> Vec<(&Task, i64)> {
        let mut out: Vec<(&Task, i64)> = self
            .list
            .iter()
            .filter_map(|t| {
                let days = (at - t.since) / DAY_MS;
                match t.status {
                    Status::Todo if day_of(t.since) < day_of(at) && days >= TODO_REMIND_DAYS - 1 => Some((t, days.max(1))),
                    Status::Doing if days >= DOING_REMIND_DAYS => Some((t, days)),
                    _ => None,
                }
            })
            .collect();
        out.sort_by_key(|(t, d)| (-d, t.id as i64));
        out
    }
}

/// « aujourd'hui », « hier », « il y a 3 jours ».
pub fn ago(days: i64) -> String {
    match days {
        0 => t!("aujourd'hui", "today").into(),
        1 => t!("depuis hier", "since yesterday").into(),
        n => tf!("depuis {n} jours", "for {n} days"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_resets_every_day() {
        let mut t = Tasks::default();
        t.add("Rendu");
        let id = t.list[0].id;
        t.set_status(id, Status::Done);
        t.roll_over(now());
        assert_eq!(t.column(Status::Done).len(), 1, "archivée aujourd'hui : reste visible");
        t.roll_over(now() + 2 * DAY_MS);
        assert!(t.column(Status::Done).is_empty());
        assert_eq!(t.archive.len(), 1);
        t.restore(id);
        assert_eq!(t.column(Status::Todo).len(), 1);
    }

    #[test]
    fn timers_fire_once_and_follow_their_task() {
        let mut t = Tasks::default();
        let id = t.add_task("Envoyer le rendu").unwrap();
        let n = now();
        let a = t.add_timer(id, n + 60_000).unwrap();
        t.add_timer(id, n - 1000);
        assert!(t.add_timer(999, n).is_none(), "tâche inconnue");
        assert_eq!(t.timers[0].due, n - 1000, "le plus proche d'abord");
        let fired = t.fire_due(n);
        assert_eq!(fired.len(), 1);
        assert!(t.fire_due(n).is_empty(), "une seule notification");
        assert_eq!(t.fire_due(n + 120_000).len(), 1);
        t.snooze(a, 10);
        assert!(!t.timers.iter().find(|x| x.id == a).unwrap().fired);
        t.rename(id, "Envoyer le rendu final");
        assert!(t.timers.iter().all(|x| x.title == "Envoyer le rendu final"));
        t.set_status(id, Status::Done);
        assert!(t.timers.is_empty(), "tâche archivée : rappels retirés");
    }

    #[test]
    fn summary_is_kept_and_saved() {
        let mut t = Tasks::default();
        let id = t.add_task("Rendu").unwrap();
        t.set_summary(id, "1. Exporter\n2. Envoyer\n\n");
        assert_eq!(t.list[0].summary, "1. Exporter\n2. Envoyer");
        let json = serde_json::to_string(&t).unwrap();
        let back: Tasks = serde_json::from_str(&json).unwrap();
        assert_eq!(back.list[0].summary, t.list[0].summary);
        // Anciennes tâches sans résumé : relues sans erreur.
        let old: Task = serde_json::from_str(r#"{"id":1,"title":"x","status":"Todo","created":0,"since":0}"#).unwrap();
        assert!(old.summary.is_empty());
    }

    #[test]
    fn reminders_for_old_tasks() {
        let mut t = Tasks::default();
        t.add("Ancienne");
        t.add("Récente");
        t.list[0].since -= 2 * DAY_MS;
        t.add("Longue");
        let id = t.list[2].id;
        t.set_status(id, Status::Doing);
        t.list[2].since -= 4 * DAY_MS;
        let r = t.reminders(now());
        let names: Vec<&str> = r.iter().map(|(x, _)| x.title.as_str()).collect();
        assert_eq!(names, ["Longue", "Ancienne"]);
    }
}
