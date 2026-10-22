//! Mails épinglés (« highlight ») : 3 au maximum, accrochés en haut de la boîte de réception
//! tant qu'ils ne sont pas décrochés. Sauvegardés localement dans pinned.json.

use std::path::PathBuf;

use crate::google::Summary;

pub const MAX: usize = 3;

#[derive(Default)]
pub struct Pinned {
    pub list: Vec<Summary>,
}

fn path() -> Option<PathBuf> {
    Some(crate::paths::config().join("pinned.json"))
}

impl Pinned {
    pub fn load() -> Self {
        let list = path().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Pinned { list }
    }

    fn save(&self) {
        if let Some(p) = path() {
            crate::paths::save_json(&p, &self.list);
        }
    }

    pub fn contains(&self, id: &str) -> bool {
        self.list.iter().any(|m| m.id == id)
    }

    /// Épingle ou décroche. `Err(())` si la limite de 3 est atteinte.
    pub fn toggle(&mut self, m: &Summary) -> Result<bool, ()> {
        if self.contains(&m.id) {
            self.list.retain(|x| x.id != m.id);
            self.save();
            return Ok(false);
        }
        if self.list.len() >= MAX {
            return Err(());
        }
        self.list.insert(0, m.clone());
        self.save();
        Ok(true)
    }

    /// Garde la copie épinglée à jour (lu / favori…).
    pub fn update(&mut self, m: &Summary) {
        if let Some(x) = self.list.iter_mut().find(|x| x.id == m.id) {
            *x = m.clone();
            self.save();
        }
    }

    pub fn set_labels(&mut self, id: &str, labels: &[String]) {
        if let Some(x) = self.list.iter_mut().find(|x| x.id == id) {
            x.labels = labels.to_vec();
            self.save();
        }
    }

    pub fn remove(&mut self, id: &str) {
        if self.contains(id) {
            self.list.retain(|x| x.id != id);
            self.save();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limited_to_three() {
        let mut p = Pinned::default();
        for i in 0..3 {
            let m = Summary { id: i.to_string(), ..Default::default() };
            p.list.insert(0, m);
        }
        let m = Summary { id: "x".into(), ..Default::default() };
        assert!(p.list.len() == MAX && !p.contains("x"));
        assert!(p.list.len() >= MAX && p.toggle(&m).is_err());
    }
}
