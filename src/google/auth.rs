//! OAuth 2.0 Google pour application de bureau :
//! redirection loopback (127.0.0.1) + PKCE, jeton de rafraîchissement dans le coffre
//! d'identifiants du système (Gestionnaire d'identification Windows).

use std::io::{Read, Write};
use std::net::TcpListener;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::config::Config;

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
/// Gmail (lire, envoyer, libellés, favoris — pas de suppression définitive),
/// réglages Gmail en lecture (signatures) et Google Agenda (lecture / écriture).
pub const SCOPES: &str = "https://www.googleapis.com/auth/gmail.modify \
https://www.googleapis.com/auth/gmail.settings.basic \
https://www.googleapis.com/auth/calendar \
https://www.googleapis.com/auth/chat.spaces.readonly \
https://www.googleapis.com/auth/chat.spaces.create \
https://www.googleapis.com/auth/chat.messages \
https://www.googleapis.com/auth/chat.memberships.readonly \
https://www.googleapis.com/auth/chat.users.readstate \
https://www.googleapis.com/auth/chat.users.sections \
https://www.googleapis.com/auth/directory.readonly \
https://www.googleapis.com/auth/drive.readonly";

const KEYRING_SERVICE: &str = "WorkTable";
const KEYRING_USER: &str = "google-refresh-token";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    expires_in: u64,
    refresh_token: Option<String>,
}

pub struct Tokens {
    pub access: String,
    pub expires_at: Instant,
    pub refresh: String,
}

// --- Coffre d'identifiants -------------------------------------------------

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| e.to_string())
}

/// Jeton gardé par les anciennes versions dans le Gestionnaire d'identification Windows
/// (repris une fois dans `data\secrets\` par `paths::migrate`).
pub fn keyring_refresh_token() -> Option<String> {
    entry().ok()?.get_password().ok().filter(|t| !t.trim().is_empty())
}

/// Jeton de connexion : fichier `data\secrets\google_refresh_token` (dossier transportable).
fn token_path() -> std::path::PathBuf {
    crate::paths::secrets().join("google_refresh_token")
}

pub fn stored_refresh_token() -> Option<String> {
    std::fs::read_to_string(token_path()).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn store_refresh_token(token: &str) -> Result<(), String> {
    crate::paths::write_atomic(&token_path(), token.as_bytes()).map_err(|e| e.to_string())
}

pub fn forget_refresh_token() {
    let _ = std::fs::remove_file(token_path());
    if let Ok(e) = entry() {
        let _ = e.delete_credential();
    }
}

// --- Flux de connexion -----------------------------------------------------

fn random_string(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("générateur aléatoire indisponible");
    URL_SAFE_NO_PAD.encode(buf)
}

/// Ouvre le navigateur sur la page de consentement Google et attend le retour.
pub fn login(agent: &ureq::Agent, cfg: &Config) -> Result<Tokens, String> {
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");

    let verifier = random_string(48);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    let state = random_string(16);

    let url = format!(
        "{AUTH_URL}?client_id={}&redirect_uri={}&response_type=code&scope={}\
         &code_challenge={challenge}&code_challenge_method=S256\
         &access_type=offline&prompt=consent&include_granted_scopes=true&state={state}",
        urlencoding::encode(cfg.client_id.trim()),
        urlencoding::encode(&redirect),
        urlencoding::encode(SCOPES),
    );
    webbrowser::open(&url).map_err(|e| format!("Impossible d'ouvrir le navigateur : {e}"))?;

    let code = wait_for_code(&listener, &state)?;

    let resp: TokenResponse = agent
        .post(TOKEN_URL)
        .send_form(&[
            ("code", code.as_str()),
            ("client_id", cfg.client_id.trim()),
            ("client_secret", cfg.client_secret.trim()),
            ("redirect_uri", redirect.as_str()),
            ("grant_type", "authorization_code"),
            ("code_verifier", verifier.as_str()),
        ])
        .map_err(super::http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;

    let refresh = resp.refresh_token.ok_or("Google n'a pas renvoyé de jeton de rafraîchissement")?;
    store_refresh_token(&refresh)?;

    Ok(Tokens { access: resp.access_token, expires_at: Instant::now() + Duration::from_secs(resp.expires_in.saturating_sub(60)), refresh })
}

/// Attend (5 min max) la redirection du navigateur et renvoie le code d'autorisation.
fn wait_for_code(listener: &TcpListener, state: &str) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(300);

    while Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(150));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

        let mut buf = [0u8; 4096];
        let n = stream.read(&mut buf).unwrap_or(0);
        let request = String::from_utf8_lossy(&buf[..n]);
        let path = request.split_whitespace().nth(1).unwrap_or("");
        let query = path.split_once('?').map(|(_, q)| q).unwrap_or("");

        let param = |key: &str| {
            query.split('&').find_map(|kv| {
                let (k, v) = kv.split_once('=')?;
                (k == key).then(|| urlencoding::decode(v).map(|s| s.into_owned()).ok())?
            })
        };

        let (ok, result) = if let Some(err) = param("error") {
            (false, Err(format!("Connexion refusée : {err}")))
        } else if let Some(code) = param("code") {
            if param("state").as_deref() == Some(state) { (true, Ok(code)) } else { (false, Err("Réponse OAuth invalide (state)".to_string())) }
        } else {
            // Requête parasite (favicon…) : on ignore.
            let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
            continue;
        };

        let msg = if ok { "Connexion réussie. Tu peux fermer cet onglet et revenir sur WorkTable." } else { "La connexion a échoué. Reviens sur WorkTable pour réessayer." };
        let body = format!(
            "<!doctype html><meta charset=utf-8><title>WorkTable</title>\
             <body style=\"background:#0e0e0e;color:#ededed;font-family:sans-serif;\
             display:grid;place-items:center;height:100vh;margin:0\"><p>{msg}</p></body>"
        );
        let _ = write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
        return result;
    }
    Err("Délai de connexion dépassé".into())
}

/// Obtient un nouveau jeton d'accès à partir du jeton de rafraîchissement.
pub fn refresh(agent: &ureq::Agent, cfg: &Config, refresh: &str) -> Result<Tokens, String> {
    let resp: TokenResponse = agent
        .post(TOKEN_URL)
        .send_form(&[("refresh_token", refresh), ("client_id", cfg.client_id.trim()), ("client_secret", cfg.client_secret.trim()), ("grant_type", "refresh_token")])
        .map_err(super::http_err)?
        .into_json()
        .map_err(|e| e.to_string())?;

    Ok(Tokens {
        access: resp.access_token,
        expires_at: Instant::now() + Duration::from_secs(resp.expires_in.saturating_sub(60)),
        refresh: resp.refresh_token.unwrap_or_else(|| refresh.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;

    #[test]
    fn loopback_receives_code() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let browser = std::thread::spawn(move || {
            // Requête parasite puis vraie redirection, comme un navigateur.
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /favicon.ico HTTP/1.1\r\n\r\n").unwrap();
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.write_all(b"GET /?state=xyz&code=4%2F0Abc&scope=gmail HTTP/1.1\r\n\r\n").unwrap();
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
            out
        });
        assert_eq!(wait_for_code(&listener, "xyz").unwrap(), "4/0Abc");
        assert!(browser.join().unwrap().contains("Connexion réussie"));
    }
}
