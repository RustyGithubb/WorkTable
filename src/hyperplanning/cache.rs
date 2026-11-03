//! Copie locale chiffrée des données HyperPlanning (référentiels et séances).
//!
//! - Fichier `cache\hyperplanning.wtc`, chiffré (AES-256-GCM). Sa clé, tirée au hasard, est dans
//!   `secrets\hyperplanning_cache.key`, protégée par Windows (DPAPI) comme celle de Google Chat.
//! - Relue au lancement : plannings consultables sans requête, même hors ligne.
//! - Liée au compte (adresse + identifiant) : un autre compte repart d'une copie vide.

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

const MAGIC: &[u8; 4] = b"WTH1";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Salle {
    pub name: String,
    pub capacity: u32,
    /// Groupe de salles (réunit plusieurs salles).
    pub is_group: bool,
    /// Groupe de salles dont elle fait partie.
    pub group: Option<u32>,
}

/// Séance placée d'un cours.
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub struct Seance {
    /// Début, heure locale de l'école (le serveur ne donne pas de fuseau).
    pub start: NaiveDateTime,
    pub minutes: u32,
    pub matiere: u32,
    pub type_cours: String,
    pub enseignants: Vec<u32>,
    pub salles: Vec<u32>,
    pub promotions: Vec<u32>,
    pub tdoptions: Vec<u32>,
    pub regroupements: Vec<u32>,
}

/// Catégorie d'une salle ou d'une promotion (famille › rubrique HYPERPLANNING).
#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Debug)]
pub struct Category {
    pub famille: String,
    pub rubrique: String,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Cours {
    pub memo: String,
    pub seances: Vec<Seance>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    /// Compte auquel appartient la copie (adresse + identifiant).
    pub account: String,
    pub matieres: BTreeMap<u32, String>,
    /// « NOM Prénom ».
    pub enseignants: BTreeMap<u32, String>,
    pub salles: BTreeMap<u32, Salle>,
    pub promotions: BTreeMap<u32, String>,
    pub tdoptions: BTreeMap<u32, String>,
    pub regroupements: BTreeMap<u32, String>,
    pub cours: BTreeMap<u32, Cours>,
    /// Catégories des salles et des promotions (vides : listes non rangées).
    #[serde(default)]
    pub salle_cats: BTreeMap<u32, Vec<Category>>,
    #[serde(default)]
    pub promo_cats: BTreeMap<u32, Vec<Category>>,
    /// Période couverte (année de la base).
    pub window: Option<(NaiveDate, NaiveDate)>,
    /// Dernière lecture des référentiels (horodatage Unix).
    pub refs_at: i64,
    /// Dernière synchronisation réussie (horodatage Unix) et son jour (pour les modifications).
    pub synced_at: i64,
    pub synced_day: Option<NaiveDate>,
}

impl Snapshot {
    pub fn seance_count(&self) -> usize {
        self.cours.values().map(|c| c.seances.len()).sum()
    }
}

pub fn account_id(url: &str, login: &str) -> String {
    format!("{}|{}", url.trim(), login.trim().to_lowercase())
}

fn file() -> PathBuf {
    crate::paths::cache().join("hyperplanning.wtc")
}

fn key_file() -> PathBuf {
    crate::paths::secrets().join("hyperplanning_cache.key")
}

fn key(create: bool) -> Option<[u8; 32]> {
    use crate::tabs::messages::store::{protect, unprotect};
    if let Ok(raw) = std::fs::read(key_file())
        && let Some(k) = unprotect(&raw).and_then(|k| <[u8; 32]>::try_from(k.as_slice()).ok())
    {
        return Some(k);
    }
    if !create {
        return None;
    }
    let mut k = [0u8; 32];
    getrandom::fill(&mut k).ok()?;
    crate::paths::write_atomic(&key_file(), &protect(&k)?).ok()?;
    Some(k)
}

pub fn save(snap: &Snapshot) -> Result<(), String> {
    let plain = serde_json::to_vec(snap).map_err(|e| e.to_string())?;
    let k = key(true).ok_or("clé indisponible")?;
    let (nonce, ct) = crate::notes_store::encrypt(&k, &plain)?;
    let mut out = Vec::with_capacity(MAGIC.len() + nonce.len() + ct.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    crate::paths::write_atomic(&file(), &out).map_err(|e| e.to_string())
}

pub fn load() -> Option<Snapshot> {
    let raw = std::fs::read(file()).ok()?;
    let rest = raw.strip_prefix(MAGIC)?;
    if rest.len() < 12 {
        return None;
    }
    let plain = crate::notes_store::decrypt(&key(false)?, &rest[..12], &rest[12..]).ok()?;
    serde_json::from_slice(&plain).ok()
}

/// Efface la copie locale et sa clé.
pub fn clear() {
    let _ = std::fs::remove_file(file());
    let _ = std::fs::remove_file(key_file());
}
