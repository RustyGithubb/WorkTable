//! Brouillons, comme dans Gmail : un mail en cours d'écriture n'est jamais perdu.
//!
//! - Enregistrement automatique 2 s après la dernière modification : sur l'ordinateur
//!   (`data\drafts.json`, relu au lancement) et dans les « Brouillons » de Gmail (dès qu'il y a un
//!   destinataire valide), donc aussi sur le téléphone.
//! - Ouvrir un autre mail, écrire un nouveau message ou répondre ailleurs range le brouillon en
//!   cours dans la barre des brouillons (en bas à droite, comme les fenêtres réduites de Gmail) ;
//!   un clic le rouvre. « Fermer » range aussi ; seul « Supprimer le brouillon » l'efface.
//! - Un brouillon jamais modifié (réponse ouverte puis abandonnée) n'est pas gardé.
//! - Envoyé : le brouillon Gmail est supprimé.

use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

use crate::google::compose::{Block, Draft};
use crate::google::{Account, Event, gmail};

/// Délai après la dernière frappe avant d'enregistrer.
const IDLE: Duration = Duration::from_secs(2);
/// Brouillons rangés gardés au plus.
const MAX_PARKED: usize = 20;

#[derive(Default)]
pub struct Drafts {
    /// Brouillons rangés (le plus récent en premier).
    pub parked: Vec<Draft>,
    /// Empreinte du contenu à la création (brouillon jamais modifié), au dernier enregistrement.
    pristine: HashMap<u64, u64>,
    saved: HashMap<u64, u64>,
    /// Dernière empreinte vue du brouillon ouvert, et moment de son dernier changement.
    seen: Option<(u64, u64, Instant)>,
    /// Enregistrements Gmail en cours ; à faire (brouillons rangés) ; brouillons Gmail à supprimer.
    in_flight: HashSet<u64>,
    to_push: Vec<u64>,
    to_delete: Vec<String>,
    loaded: bool,
}

/// Empreinte du contenu (sans relire les données des fichiers à chaque image).
pub fn fingerprint(d: &Draft) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (&d.to, &d.cc, &d.subject, &d.signature).hash(&mut h);
    for b in &d.blocks {
        match b {
            Block::Para(p) => (0u8, &p.text, p.size as u8, p.align as u8).hash(&mut h),
            Block::Image(i) => (1u8, i.id, i.data.len()).hash(&mut h),
        }
    }
    for f in &d.files {
        (&f.name, f.data.len()).hash(&mut h);
    }
    h.finish()
}

/// Rien d'écrit : ni destinataire, ni objet, ni texte, ni fichier.
fn is_empty(d: &Draft) -> bool {
    d.to.trim().is_empty()
        && d.cc.trim().is_empty()
        && d.subject.trim().is_empty()
        && d.files.is_empty()
        && d.blocks.iter().all(|b| matches!(b, Block::Para(p) if p.text.trim().is_empty()))
}

fn path() -> std::path::PathBuf {
    crate::paths::data().join("drafts.json")
}

impl Drafts {
    /// Brouillons enregistrés sur l'ordinateur (une fois, au premier usage).
    fn load(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Ok(text) = std::fs::read_to_string(path()) else { return };
        let mut list: Vec<Draft> = serde_json::from_str(&text).unwrap_or_default();
        for d in &mut list {
            restore_previews(d);
            let fp = fingerprint(d);
            self.saved.insert(d.local_id, fp);
        }
        self.parked = list;
    }

    /// Nouveau brouillon : son contenu de départ (citation, signature…) ne compte pas comme écrit.
    pub fn started(&mut self, d: &Draft) {
        self.pristine.insert(d.local_id, fingerprint(d));
    }

    /// Le brouillon a-t-il quelque chose à garder ?
    pub fn worth_keeping(&self, d: &Draft) -> bool {
        !is_empty(d) && self.pristine.get(&d.local_id) != Some(&fingerprint(d))
    }

    /// Range le brouillon ouvert (gardé s'il contient quelque chose).
    pub fn park(&mut self, d: Draft) {
        self.load();
        if !self.worth_keeping(&d) {
            self.to_delete.extend(d.gmail_id.clone());
            return;
        }
        self.parked.retain(|p| p.local_id != d.local_id);
        let fp = fingerprint(&d);
        if self.saved.get(&d.local_id) != Some(&fp) {
            self.to_push.push(d.local_id);
            self.saved.insert(d.local_id, fp);
        }
        self.parked.insert(0, d);
        self.parked.truncate(MAX_PARKED);
        self.seen = None;
        self.persist(None);
    }

    /// Ressort un brouillon rangé (il redevient le brouillon ouvert).
    pub fn take(&mut self, local_id: u64) -> Option<Draft> {
        let i = self.parked.iter().position(|d| d.local_id == local_id)?;
        let d = self.parked.remove(i);
        self.seen = None;
        Some(d)
    }

    /// Brouillon rangé correspondant à ce message Gmail (ouvert depuis « Brouillons »).
    pub fn find_by_message(&self, message_id: &str) -> Option<u64> {
        self.parked.iter().find(|d| d.gmail_message_id.as_deref() == Some(message_id)).map(|d| d.local_id)
    }

    /// Supprime un brouillon (ici et dans Gmail).
    pub fn discard(&mut self, d: &Draft, current: Option<&Draft>) {
        self.parked.retain(|p| p.local_id != d.local_id);
        self.saved.remove(&d.local_id);
        self.pristine.remove(&d.local_id);
        self.to_push.retain(|i| *i != d.local_id);
        self.to_delete.extend(d.gmail_id.clone());
        self.persist(current.filter(|c| c.local_id != d.local_id));
    }

    /// Brouillon envoyé : supprimé dans Gmail, oublié ici.
    pub fn sent(&mut self, d: &Draft) {
        self.discard(d, None);
    }

    /// À chaque image : enregistre le brouillon ouvert 2 s après la dernière modification, envoie
    /// à Gmail les brouillons rangés et les suppressions en attente.
    pub fn track(&mut self, acc: &Account, from: &str, current: Option<&Draft>) {
        self.load();
        if acc.session().is_some() {
            for id in std::mem::take(&mut self.to_delete) {
                acc.run(move |s| gmail::delete_draft(s, &id).map(|_| Event::Chat(crate::google::chat::ChatEvent::Noop)));
            }
            for local in std::mem::take(&mut self.to_push) {
                if let Some(d) = self.parked.iter().find(|d| d.local_id == local).cloned() {
                    self.push_gmail(acc, from, &d);
                }
            }
        }
        let Some(d) = current else {
            self.seen = None;
            return;
        };
        let fp = fingerprint(d);
        match self.seen {
            Some((id, f, _)) if id == d.local_id && f == fp => {}
            _ => self.seen = Some((d.local_id, fp, Instant::now())),
        }
        let Some((_, _, at)) = self.seen else { return };
        if at.elapsed() < IDLE || self.saved.get(&d.local_id) == Some(&fp) || !self.worth_keeping(d) {
            return;
        }
        self.saved.insert(d.local_id, fp);
        self.persist(Some(d));
        self.push_gmail(acc, from, d);
    }

    /// Enregistrement Gmail (seulement avec un destinataire valide : sans, le brouillon reste sur
    /// l'ordinateur).
    fn push_gmail(&mut self, acc: &Account, from: &str, d: &Draft) {
        if from.is_empty() || acc.session().is_none() {
            return;
        }
        if self.in_flight.contains(&d.local_id) {
            // Un enregistrement est déjà parti : celui-ci suivra (empreinte oubliée).
            self.saved.remove(&d.local_id);
            return;
        }
        let Ok(mime) = d.to_mime(from) else { return };
        self.in_flight.insert(d.local_id);
        let (local_id, id, thread) = (d.local_id, d.gmail_id.clone(), d.thread_id.clone());
        acc.run(move |s| {
            let result = gmail::save_draft(s, id.as_deref(), &mime, thread.as_deref());
            Ok(Event::DraftSaved { local_id, result })
        });
    }

    /// Réponse de Gmail : identifiants du brouillon (à garder pour les mises à jour).
    pub fn on_saved(&mut self, local_id: u64, result: &Result<(String, String), String>, current: Option<&mut Draft>) {
        self.in_flight.remove(&local_id);
        let Ok((id, message)) = result else {
            // Échec : nouvel essai à la prochaine modification.
            self.saved.remove(&local_id);
            return;
        };
        let now = chrono::Local::now().timestamp_millis();
        let mut found = false;
        for d in current.into_iter().chain(self.parked.iter_mut()) {
            if d.local_id == local_id {
                d.gmail_id = Some(id.clone());
                d.gmail_message_id = Some(message.clone());
                d.saved_at = Some(now);
                found = true;
            }
        }
        // Supprimé ou envoyé pendant l'enregistrement : le brouillon Gmail créé est retiré.
        if !found {
            self.to_delete.push(id.clone());
        }
    }

    /// État à afficher dans l'éditeur : « Enregistrement… », « Brouillon enregistré »…
    pub fn label(&self, d: &Draft) -> Option<String> {
        if self.in_flight.contains(&d.local_id) {
            return Some(t!("Enregistrement…", "Saving…").into());
        }
        let up_to_date = self.saved.get(&d.local_id) == Some(&fingerprint(d));
        match d.saved_at {
            Some(ms) if up_to_date => Some(tf!("Brouillon enregistré · {}", "Draft saved · {}", crate::tabs::calendar::hhmm(ms))),
            _ if up_to_date => Some(t!("Brouillon enregistré sur l'ordinateur", "Draft saved on this computer").into()),
            _ => None,
        }
    }

    /// Écrit les brouillons sur l'ordinateur (le brouillon ouvert compris).
    pub fn persist(&mut self, current: Option<&Draft>) {
        self.load();
        let mut all: Vec<&Draft> = Vec::new();
        if let Some(c) = current.filter(|c| self.worth_keeping(c)) {
            all.push(c);
        }
        all.extend(self.parked.iter().filter(|p| current.is_none_or(|c| c.local_id != p.local_id)));
        crate::paths::save_json(&path(), &all);
    }
}

/// Aperçus des images intégrées (non enregistrés) recalculés.
fn restore_previews(d: &mut Draft) {
    for b in &mut d.blocks {
        if let Block::Image(i) = b
            && i.preview.is_none()
        {
            i.preview = super::inline::decode(&i.data).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn untouched_drafts_are_not_kept() {
        let mut ds = Drafts { loaded: true, ..Default::default() };
        let mut d = Draft::new();
        ds.started(&d);
        assert!(!ds.worth_keeping(&d));
        d.subject = "Bonjour".into();
        assert!(ds.worth_keeping(&d));
        // Relu tel quel depuis le disque (base64 des fichiers compris).
        d.files.push(crate::google::compose::FileData { name: "a.txt".into(), mime: "text/plain".into(), data: std::sync::Arc::new(b"abc".to_vec()) });
        let back: Draft = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
        assert_eq!(fingerprint(&back), fingerprint(&d));
        assert_eq!(back.local_id, d.local_id);
    }
}
