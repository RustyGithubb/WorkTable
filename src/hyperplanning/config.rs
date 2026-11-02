//! Identifiants HyperPlanning : `secrets\hyperplanning.json` du dossier des données.
//! Le mot de passe y est protégé par Windows (DPAPI) : seule la session Windows qui l'a
//! enregistré peut le relire (à ressaisir sur un autre poste).

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Adresse du service web de l'école, proposée par défaut.
pub const DEFAULT_URL: &str = "http://hyperplanning.artfx.fr:8086/hpsw";
/// Espace web HyperPlanning.net (demandes de réservation de salle), proposé par défaut.
pub const DEFAULT_WEB_URL: &str = "https://hpp.artfx.fr/enseignant";

fn default_web_url() -> String {
    DEFAULT_WEB_URL.into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub url: String,
    pub login: String,
    /// Espace web HyperPlanning.net, ouvert pour les demandes de réservation.
    #[serde(default = "default_web_url")]
    pub web_url: String,
    /// Mot de passe protégé (DPAPI), en base64.
    #[serde(default)]
    password: String,
    /// Identifiant de l'espace web (vide : celui du service web).
    #[serde(default)]
    pub web_login: String,
    /// Mot de passe de l'espace web, protégé comme `password` (vide : celui du service web).
    #[serde(default)]
    web_password: String,
}

impl Default for Config {
    fn default() -> Self {
        Self { url: DEFAULT_URL.into(), login: String::new(), web_url: default_web_url(), password: String::new(), web_login: String::new(), web_password: String::new() }
    }
}

impl Config {
    fn path() -> PathBuf {
        crate::paths::secrets().join("hyperplanning.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        crate::paths::write_atomic(&Self::path(), json.as_bytes()).map_err(|e| e.to_string())
    }

    pub fn clear() {
        let _ = std::fs::remove_file(Self::path());
    }

    pub fn has_password(&self) -> bool {
        !self.password.is_empty()
    }

    pub fn is_complete(&self) -> bool {
        !self.url.trim().is_empty() && !self.login.trim().is_empty() && self.has_password()
    }

    pub fn set_password(&mut self, plain: &str) -> Result<(), String> {
        self.password = seal(plain)?;
        Ok(())
    }

    /// Mot de passe en clair (le temps d'un appel), s'il est enregistré et relisible.
    pub fn password(&self) -> Option<String> {
        unseal(&self.password)
    }

    pub fn has_web_password(&self) -> bool {
        !self.web_password.is_empty()
    }

    pub fn set_web_password(&mut self, plain: &str) -> Result<(), String> {
        self.web_password = seal(plain)?;
        Ok(())
    }

    /// Identifiants de l'espace web : les siens, sinon ceux du service web.
    pub fn web_credentials(&self) -> Option<(String, String)> {
        let login = if self.web_login.trim().is_empty() { self.login.trim() } else { self.web_login.trim() };
        let password = if self.has_web_password() { unseal(&self.web_password)? } else { self.password()? };
        (!login.is_empty()).then(|| (login.to_string(), password))
    }
}

fn seal(plain: &str) -> Result<String, String> {
    let sealed = crate::tabs::messages::store::protect(plain.as_bytes()).ok_or(t!("Protection du mot de passe impossible", "Could not protect the password"))?;
    Ok(base64::engine::general_purpose::STANDARD.encode(sealed))
}

fn unseal(stored: &str) -> Option<String> {
    let sealed = base64::engine::general_purpose::STANDARD.decode(stored).ok()?;
    String::from_utf8(crate::tabs::messages::store::unprotect(&sealed)?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_round_trip() {
        let mut c = Config::default();
        assert!(!c.is_complete());
        c.login = "moi".into();
        c.set_password("s3cret").unwrap();
        assert!(c.is_complete());
        assert_eq!(c.password().as_deref(), Some("s3cret"));
        let back: Config = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(back.password().as_deref(), Some("s3cret"));
    }
}
