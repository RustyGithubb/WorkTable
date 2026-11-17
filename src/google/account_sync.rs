//! Compte Google rattaché au compte NAS : identifiants OAuth et jeton de connexion, retrouvés sur
//! chaque poste. Copie dans `config\google_account.bin` (synchronisée), **chiffrée** avec une clé
//! tirée de la clé maître du compte NAS (illisible sur le disque comme sur le serveur). Sur chaque
//! poste, `secrets\` reste la référence pour se connecter : la copie ne fait que le compléter.

use std::path::PathBuf;

use nas_sync_client::MasterKey;
use serde::{Deserialize, Serialize};

/// Clé dérivée de la clé maître, réservée à ce fichier.
const KEY: &str = "worktable-google";
/// Nom chiffré avec le contenu (contrôle à la lecture).
const NAME: &str = "google_account";

#[derive(Default, PartialEq, Serialize, Deserialize)]
struct Blob {
    client_id: String,
    client_secret: String,
    refresh_token: String,
}

fn path() -> PathBuf {
    crate::paths::config().join("google_account.bin")
}

fn read(master: &MasterKey) -> Option<Blob> {
    let data = std::fs::read(path()).ok()?;
    let (name, json) = master.app(KEY).decrypt(&data).ok()?;
    if name != NAME {
        return None;
    }
    serde_json::from_slice(&json).ok()
}

/// Met à jour la copie chiffrée avec les identifiants et le jeton de ce poste (sans compte NAS ou
/// sans identifiants : rien). Réécrite seulement si elle change.
pub fn export() {
    let Some(master) = crate::sync::master_key() else { return };
    if !super::Config::saved() {
        return;
    }
    let cfg = super::Config::load();
    let blob = Blob { client_id: cfg.client_id, client_secret: cfg.client_secret, refresh_token: super::auth::stored_refresh_token().unwrap_or_default() };
    if read(&master).as_ref() == Some(&blob) {
        return;
    }
    let Ok(json) = serde_json::to_vec(&blob) else { return };
    let _ = crate::paths::write_atomic(&path(), &master.app(KEY).encrypt(NAME, &json));
}

/// Au lancement (après la réception des changements) : ce qui manque ici est repris de la copie
/// venue d'un autre poste (identifiants, connexion), puis la copie est mise à jour.
pub fn import_then_export() {
    let Some(master) = crate::sync::master_key() else { return };
    if let Some(b) = read(&master) {
        if !super::Config::saved() && !b.client_id.is_empty() {
            let _ = super::Config { client_id: b.client_id, client_secret: b.client_secret }.save();
        }
        if super::auth::stored_refresh_token().is_none() && !b.refresh_token.is_empty() {
            let _ = super::auth::store_refresh_token(&b.refresh_token);
        }
    }
    export();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_is_encrypted_and_bound_to_the_account() {
        let master = MasterKey([7; 32]);
        let blob = Blob { client_id: "id".into(), client_secret: "secret".into(), refresh_token: "jeton".into() };
        let data = master.app(KEY).encrypt(NAME, &serde_json::to_vec(&blob).unwrap());
        assert!(!data.windows(5).any(|w| w == b"jeton"), "contenu en clair");
        let (name, json) = master.app(KEY).decrypt(&data).unwrap();
        assert_eq!(name, NAME);
        assert!(serde_json::from_slice::<Blob>(&json).unwrap() == blob);
        // Autre compte : illisible.
        assert!(MasterKey([8; 32]).app(KEY).decrypt(&data).is_err());
    }
}
