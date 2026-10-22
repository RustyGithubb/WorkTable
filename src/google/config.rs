//! Identifiants OAuth de l'application (client « Application de bureau » Google Cloud).
//! Stockés en JSON dans `secrets\google.json` du dossier des données (voir `paths`). Un
//! `google.json` livré à côté de l'exécutable (par l'installeur) sert de valeur par défaut.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Config {
    pub client_id: String,
    pub client_secret: String,
}

impl Config {
    pub fn is_complete(&self) -> bool {
        !self.client_id.trim().is_empty() && !self.client_secret.trim().is_empty()
    }

    fn path() -> Option<PathBuf> {
        Some(crate::paths::secrets().join("google.json"))
    }

    pub fn load() -> Self {
        let read = |p: PathBuf| std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str::<Config>(&s).ok());
        Self::path().and_then(read).filter(Config::is_complete).or_else(|| read(crate::paths::install_dir().join("google.json"))).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = Self::path().ok_or(t!("Dossier de configuration introuvable", "Configuration folder not found"))?;
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::paths::write_atomic(&p, json.as_bytes()).map_err(|e| e.to_string())
    }
}
