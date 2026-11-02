//! Client de l'espace web HyperPlanning.net (espace Enseignants), pour ce que le service web
//! SOAP ne permet pas : les demandes de réservation de salle.
//!
//! API **non officielle** (celle qu'utilise le site, comme pour PRONOTE) : elle peut changer à
//! chaque version d'HYPERPLANNING.
//!
//! - Requêtes `POST <base>/appelfonction/<espace>/<session>/<n° d'ordre chiffré>`, corps JSON
//!   `{session, numeroOrdre, nom, donneesSec}` : `donneesSec` = JSON compressé (deflate brut
//!   du texte en hexadécimal) puis chiffré en AES-128-CBC (clé et IV : MD5 de la clé et de
//!   l'IV de session).
//! - Ouverture : la page donne le n° de session et la clé publique RSA ; `FonctionParametres`
//!   envoie l'IV de session chiffré en RSA ; `Identification` renvoie un défi, résolu avec
//!   `identifiant + SHA256(alea + mot de passe)` ; `Authentification` renvoie la clé AES.
//! - Le n° d'ordre avance de 2 à chaque échange (le serveur répond avec n + 1).
//! - Demande de réservation ([`Session::request_room`]) : `SaisieCours` (création d'un cours
//!   « Réservation de salles » avec une demande de salle), comme la recherche de salles libres
//!   du site ; un administrateur la valide ou la refuse.

use aes::cipher::{BlockDecryptMut, BlockEncryptMut, KeyIvInit, block_padding::Pkcs7};
use base64::Engine;
use chrono::{Datelike, NaiveDate};
use md5::{Digest, Md5};
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::time::Duration;

type Enc = cbc::Encryptor<aes::Aes128>;
type Dec = cbc::Decryptor<aes::Aes128>;

/// AES-CBC du protocole : clé et IV = MD5 de la clé et de l'IV de session (IV vide : zéros).
fn aes_keys(key: &[u8], iv: &[u8]) -> ([u8; 16], [u8; 16]) {
    let k: [u8; 16] = Md5::digest(key).into();
    let v: [u8; 16] = if iv.is_empty() { [0; 16] } else { Md5::digest(iv).into() };
    (k, v)
}

pub fn aes_encrypt(plain: &[u8], key: &[u8], iv: &[u8]) -> String {
    let (k, v) = aes_keys(key, iv);
    hex::encode(Enc::new(&k.into(), &v.into()).encrypt_padded_vec_mut::<Pkcs7>(plain))
}

pub fn aes_decrypt(hex_data: &str, key: &[u8], iv: &[u8]) -> Result<Vec<u8>, String> {
    let (k, v) = aes_keys(key, iv);
    let data = hex::decode(hex_data.trim()).map_err(|e| e.to_string())?;
    Dec::new(&k.into(), &v.into())
        .decrypt_padded_vec_mut::<Pkcs7>(&data)
        .map_err(|_| t!("Déchiffrement impossible (clé de session)", "Decryption failed (session key)").to_string())
}

/// Compression du protocole : texte en hexadécimal, puis deflate brut (niveau 6).
fn compress(text: &str) -> Vec<u8> {
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    let _ = z.write_all(hex::encode(text.as_bytes()).as_bytes());
    z.finish().unwrap_or_default()
}

fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Chiffrement RSA PKCS#1 v1.5 (clé publique de la page, en hexadécimal), base64 en lignes de
/// 64 caractères comme le site.
fn rsa_encrypt(data: &[u8], modulus_hex: &str, exponent_hex: &str) -> Result<String, String> {
    use num_bigint::BigUint;
    let n = BigUint::parse_bytes(modulus_hex.as_bytes(), 16).ok_or("clé RSA invalide")?;
    let e = BigUint::parse_bytes(exponent_hex.as_bytes(), 16).ok_or("clé RSA invalide")?;
    let k = n.to_bytes_be().len();
    if data.len() + 11 > k {
        return Err("clé RSA trop courte".into());
    }
    // 00 02 | octets aléatoires non nuls | 00 | données
    let mut pad = vec![0u8; k - data.len() - 3];
    getrandom::fill(&mut pad).map_err(|e| e.to_string())?;
    for b in pad.iter_mut() {
        while *b == 0 {
            let mut one = [0u8; 1];
            getrandom::fill(&mut one).map_err(|e| e.to_string())?;
            *b = one[0];
        }
    }
    let mut m = vec![0u8, 2];
    m.extend_from_slice(&pad);
    m.push(0);
    m.extend_from_slice(data);
    let c = BigUint::from_bytes_be(&m).modpow(&e, &n).to_bytes_be();
    let mut out = vec![0u8; k - c.len()];
    out.extend_from_slice(&c);
    let b64 = base64::engine::general_purpose::STANDARD.encode(out);
    Ok(b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect::<Vec<_>>().join("\r\n"))
}

/// Valeur typée du protocole (`{"_T": …, "V": …}`) ou valeur simple.
pub fn plain(v: &Value) -> &Value {
    match v {
        Value::Object(o) if o.contains_key("_T") && o.contains_key("V") => &o["V"],
        _ => v,
    }
}

fn str_of(v: &Value, key: &str) -> String {
    match plain(&v[key]) {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Paramètre de `Start({…})` dans la page : `a:0`, `i:'5451591'`, `MR:'…'`.
fn start_param(start: &str, key: &str) -> Option<String> {
    let pat = format!("{key}:");
    let mut from = 0;
    while let Some(p) = start[from..].find(&pat) {
        let at = from + p;
        let before = start[..at].chars().last();
        from = at + pat.len();
        if !matches!(before, Some('{' | ',' | ' ')) {
            continue;
        }
        let rest = start[from..].trim_start();
        return Some(if let Some(r) = rest.strip_prefix('\'') { r.split('\'').next()?.to_string() } else { rest.split([',', '}']).next()?.trim().to_string() });
    }
    None
}

/// Navigateur annoncé au site (celui de Windows le plus courant).
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// Objet passé à `Start ({…})` dans la page (`onload="try { Start ({a:0,…}) }"`).
fn start_object(html: &str) -> Option<&str> {
    let mut from = 0;
    while let Some(p) = html[from..].find("Start") {
        let rest = html[from + p + 5..].trim_start();
        from += p + 5;
        if let Some(r) = rest.strip_prefix('(') {
            let r = r.trim_start();
            if r.starts_with('{') {
                return r.find('}').map(|j| &r[..=j]);
            }
        }
    }
    None
}

/// Message d'une erreur d'accès (`Acces` dans la réponse d'authentification).
fn acces_message(code: i64, msg: &Value) -> String {
    let detail = str_of(msg, "message");
    match code {
        1 => t!("Identifiant ou mot de passe incorrect", "Wrong login or password").into(),
        2 => t!("Accès refusé à l'espace Enseignants pour ce compte", "Access to the Teachers space refused for this account").into(),
        _ if !detail.is_empty() => detail,
        _ => tf!("Connexion refusée (code {code})", "Connection refused (code {code})"),
    }
}

/// Demande de réservation : salle (nom HYPERPLANNING), jour, créneau en minutes, motif.
#[derive(Clone)]
pub struct RoomRequest {
    pub room: String,
    pub date: NaiveDate,
    pub start_min: u32,
    pub end_min: u32,
    pub comment: String,
}

/// Grille horaire du site : `ListeHeures` = places successives (« 08h00 » → « 08h30 »).
#[derive(Debug, PartialEq)]
struct Grid {
    start_min: u32,
    step_min: u32,
    places_per_day: u32,
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(['h', 'H', ':'])?;
    Some(h.trim().parse::<u32>().ok()? * 60 + m.trim().parse::<u32>().unwrap_or(0))
}

impl Grid {
    fn from_heures(heures: &[Value]) -> Option<Self> {
        let first = heures.first()?;
        let start = parse_hhmm(&str_of(first, "Debut"))?;
        let end = parse_hhmm(&str_of(first, "Fin")).filter(|e| *e > start)?;
        Some(Self { start_min: start, step_min: end - start, places_per_day: heures.len() as u32 })
    }

    /// (n° de semaine HYPERPLANNING, place dans la semaine, durée en places).
    fn place(&self, first_monday: NaiveDate, date: NaiveDate, start: u32, end: u32) -> Result<(i64, u32, u32), String> {
        let grid_end = self.start_min + self.step_min * self.places_per_day;
        if end <= start || start < self.start_min || end > grid_end || (start - self.start_min) % self.step_min != 0 || (end - start) % self.step_min != 0 {
            return Err(tf!(
                "Créneau hors de la grille de l'espace web ({} – {}, par pas de {} min)",
                "Time slot outside the web space grid ({} – {}, {} min steps)",
                fmt_min(self.start_min),
                fmt_min(grid_end),
                self.step_min
            ));
        }
        let wd = date.weekday().num_days_from_monday();
        let monday = date - chrono::Duration::days(wd as i64);
        let week = (monday - first_monday).num_days().div_euclid(7) + 1;
        if week < 1 {
            return Err(t!("Jour hors de l'année HYPERPLANNING", "Day outside the HYPERPLANNING year").into());
        }
        Ok((week, wd * self.places_per_day + (start - self.start_min) / self.step_min, (end - start) / self.step_min))
    }
}

fn fmt_min(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// Date du protocole : `17/08/2026 0:0:0` ou `2026-08-17…` (éventuellement typée).
fn parse_date(v: &Value) -> Option<NaiveDate> {
    let s = plain(v).as_str()?.trim();
    let head = s.split([' ', 'T']).next()?;
    NaiveDate::parse_from_str(head, "%d/%m/%Y").or_else(|_| NaiveDate::parse_from_str(head, "%Y-%m-%d")).ok()
}

/// Première valeur trouvée sous la clé `key`, à n'importe quelle profondeur.
fn find_key<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    match v {
        Value::Object(o) => o.get(key).or_else(|| o.values().find_map(|x| find_key(x, key))),
        Value::Array(a) => a.iter().find_map(|x| find_key(x, key)),
        _ => None,
    }
}

/// Élément `{N, G, L}` sous l'une des clés `keys` (à n'importe quelle profondeur).
fn find_element(v: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|k| find_key(v, k)).map(plain).filter(|e| e.get("N").is_some()).map(|e| json!({"N": e["N"], "G": plain(&e["G"]).as_i64().unwrap_or(0), "L": e["L"]}))
}

pub struct Session {
    agent: ureq::Agent,
    /// Dossier de la page (`https://hpp.artfx.fr/`), où se trouve `appelfonction`.
    base: String,
    espace: String,
    session: u64,
    key: Vec<u8>,
    iv: Vec<u8>,
    ordre: u64,
    crypt: bool,
    compression: bool,
    /// Nom de l'utilisateur connecté.
    pub user: String,
    /// Utilisateur connecté tel que le site le désigne (`{N, G, L}`), pour la signature des requêtes.
    me: Option<Value>,
}

impl Session {
    /// Ouvre une session et s'authentifie (4 requêtes).
    pub fn connect(page_url: &str, login: &str, password: &str) -> Result<Self, String> {
        // Le site sert une autre page aux clients qu'il ne reconnaît pas comme navigateurs.
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(60)).user_agent(USER_AGENT).build();
        let url = page_url.trim();
        let html = agent
            .get(url)
            .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .set("Accept-Language", "fr-FR,fr;q=0.9")
            .call()
            .map_err(|e| tf!("Espace web injoignable : {e}", "Web space unreachable: {e}"))?
            .into_string()
            .map_err(|e| e.to_string())?;
        let start = start_object(&html).ok_or_else(|| {
            // Titre de la page reçue, pour comprendre ce que le serveur a renvoyé.
            let title = html.split("<title>").nth(1).and_then(|t| t.split("</title>").next()).unwrap_or("").trim();
            let page = if title.is_empty() { html.chars().take(120).collect::<String>().replace(['\r', '\n'], " ") } else { title.to_string() };
            tf!("Page de l'espace web non reconnue (reçu : « {page} »)", "Web space page not recognised (received: “{page}”)")
        })?;
        let session: u64 = start_param(start, "h").or_else(|| start_param(start, "i")).and_then(|s| s.parse().ok()).ok_or("n° de session introuvable")?;
        let espace = start_param(start, "a").unwrap_or_else(|| "0".into());
        let (mr, er) = (start_param(start, "MR").ok_or("clé RSA introuvable")?, start_param(start, "ER").ok_or("clé RSA introuvable")?);
        let base = match url.split('?').next().unwrap_or(url).rfind('/') {
            Some(i) => url[..=i].to_string(),
            None => format!("{url}/"),
        };
        let mut s = Self {
            agent,
            base,
            espace,
            session,
            key: Vec::new(),
            iv: Vec::new(),
            ordre: 1,
            crypt: start_param(start, "sCrA").as_deref() != Some("true"),
            compression: start_param(start, "sCoA").as_deref() != Some("true"),
            user: String::new(),
            me: None,
        };

        // IV de session envoyé en RSA ; la requête elle-même part encore avec l'IV vide.
        let mut iv = vec![0u8; 16];
        getrandom::fill(&mut iv).map_err(|e| e.to_string())?;
        let uuid = rsa_encrypt(&iv, &mr, &er)?;
        s.send("FonctionParametres", Some(json!({"ModeJeton": false, "Uuid": uuid, "identifiantNav": null})), None, Some(iv))?;

        let id = s.call(
            "Identification",
            json!({
                "genreConnexion": 0, "genreEspace": s.espace.parse::<i64>().unwrap_or(0), "identifiant": login, "pourENT": false,
                "enConnexionAuto": false, "demandeConnexionAuto": false, "enConnexionAppliMobile": false, "demandeConnexionAppliMobile": false,
                "demandeConnexionAppliMobileJeton": false, "uuidAppliMobile": "", "loginTokenSAV": ""
            }),
        )?;
        let d = &id["donnees"];
        let lower = |v: &Value| plain(v).as_i64().unwrap_or(0) > 0;
        let login_c = if lower(&d["modeCompLog"]) { login.to_lowercase() } else { login.to_string() };
        let pass_c = if lower(&d["modeCompMdp"]) { password.to_lowercase() } else { password.to_string() };
        let hash = hex::encode_upper(sha2::Sha256::new().chain_update(str_of(d, "alea").as_bytes()).chain_update(pass_c.as_bytes()).finalize());
        let user_key = format!("{login_c}{hash}").into_bytes();
        // Défi : déchiffré, un caractère sur deux, rechiffré.
        let raw = aes_decrypt(&str_of(d, "challenge"), &user_key, &s.iv).map_err(|_| t!("Identifiant ou mot de passe incorrect", "Wrong login or password").to_string())?;
        let raw = String::from_utf8(raw).map_err(|_| t!("Identifiant ou mot de passe incorrect", "Wrong login or password").to_string())?;
        let solved: String = raw.chars().step_by(2).collect();
        let challenge = aes_encrypt(solved.as_bytes(), &user_key, &s.iv);

        let mut auth = json!({
            "genreConnexion": 0, "identifiant": login_c, "pourENT": false, "nomRessource": "",
            "enConnexionAuto": false, "demandeConnexionAuto": false, "enConnexionAppliMobile": false, "demandeConnexionAppliMobile": false,
            "demandeConnexionAppliMobileJeton": false, "uuidAppliMobile": "", "loginTokenSAV": "", "challenge": challenge
        });
        if !d["utilisateur"].is_null() {
            auth["ressourceInternet"] = d["utilisateur"].clone();
        }
        let r = s.call("Authentification", auth)?;
        let a = &r["donnees"];
        if let Some(code) = plain(&a["Acces"]).as_i64() {
            return Err(acces_message(code, &a["AccesMessage"]));
        }
        if !a["actionsDoubleAuth"].is_null() {
            return Err(t!("Double authentification demandée : non gérée par WorkTable", "Two-factor authentication required: not supported by WorkTable").into());
        }
        let cle = str_of(a, "cle");
        if cle.is_empty() {
            return Err(t!("Authentification refusée", "Authentication refused").into());
        }
        // Nouvelle clé : liste d'octets « 12,34,… » chiffrée avec la clé de l'utilisateur.
        let list = String::from_utf8(aes_decrypt(&cle, &user_key, &s.iv)?).map_err(|e| e.to_string())?;
        s.key = list.split(',').filter_map(|x| x.trim().parse::<u8>().ok()).collect();
        s.me = find_element(a, &["ressource", "utilisateur", "Ressource", "Utilisateur", "membre"]);
        s.user = str_of(a, "libelleUtil");
        if s.user.is_empty() {
            s.user = login.to_string();
        }
        Ok(s)
    }

    /// Requête sans signature (données seules) ; renvoie la racine de la réponse déchiffrée.
    pub fn call(&mut self, nom: &str, donnees: Value) -> Result<Value, String> {
        self.send(nom, Some(donnees), None, None)
    }

    /// Requête avec signature (`_Signature_` : onglet, ressource…), pour les pages après connexion.
    fn call_signed(&mut self, nom: &str, donnees: Option<Value>, onglet: &str) -> Result<Value, String> {
        let liste: Vec<Value> = self.me.iter().cloned().collect();
        self.send(nom, donnees, Some(json!({"Onglet": onglet, "listeRecherche": liste})), None)
    }

    /// Envoie une demande de réservation de salle (3 requêtes). `first_monday` : premier lundi
    /// de l'année HYPERPLANNING (repli si le site ne le donne pas). Renvoie le nom de la salle.
    pub fn request_room(&mut self, r: &RoomRequest, first_monday: Option<NaiveDate>) -> Result<String, String> {
        // Grille du site : places de 30 min à partir de la première heure (« 08h00 »).
        let p = self.call_signed("DemandeParametreUtilisateur", None, "ACCUEIL")?;
        let heures = p["donnees"]["Horaire"]["ListeHeures"].as_array().cloned().unwrap_or_default();
        let grid = Grid::from_heures(&heures).ok_or(t!("Grille horaire de l'espace web introuvable", "Web space time grid not found"))?;
        let first_monday =
            find_key(&p, "PremierLundi").and_then(parse_date).or(first_monday).ok_or(t!("Début de l'année HYPERPLANNING inconnu", "HYPERPLANNING year start unknown"))?;
        let (week, place, duree) = grid.place(first_monday, r.date, r.start_min, r.end_min)?;
        let domaine = json!({"_T": 8, "V": format!("[{week}]")});

        // Salles libres du site sur ce créneau : identifiant de la salle et matière par défaut.
        let f =
            self.call_signed("FonctionSallesLibres", Some(json!({"avecRecherche": true, "duree": duree, "domaine": domaine, "avecSallesDemandees": true})), "SALLE.SALLESLIBRES")?;
        let d = &f["donnees"];
        let wanted = r.room.trim().to_lowercase();
        let salle = plain(&d["listeSalles"])
            .as_array()
            .and_then(|l| l.iter().find(|s| str_of(s, "L").trim().to_lowercase() == wanted))
            .ok_or_else(|| tf!("Salle « {} » introuvable dans l'espace web (pas réservable ?)", "Room “{}” not found in the web space (not bookable?)", r.room))?;
        let salle = json!({"N": salle["N"], "G": plain(&salle["G"]).as_i64().unwrap_or(3), "L": salle["L"]});
        let m = plain(&d["matiereParDefaut"]);
        if m["N"].is_null() {
            return Err(t!("Matière « Réservation de salles » introuvable", "“Room booking” subject not found").into());
        }
        let matiere = json!({"N": m["N"], "G": 0, "L": m["L"], "E": 1});

        let saisie = json!({
            "genreSaisie": 8, "place": place, "duree": duree, "etatCours": 3,
            "domaine": domaine, "domaineAffichage": domaine, "matiere": matiere, "listeRessources": [],
            "listeDemandes": [{
                "listeSalles": [salle], "commentaire": r.comment.trim(),
                "periode": {"N": 0, "G": 0}, "site": {"N": 0, "G": 0}, "capacite": 0, "listeSousFamilles": []
            }]
        });
        let rep = self.call_signed("SaisieCours", Some(saisie), "SALLE.SALLESLIBRES")?;
        let rapport = &rep["RapportSaisie"];
        let messages: Vec<String> = rapport["_messagesErreur_"]
            .as_array()
            .map(|l| l.iter().map(|m| plain(m).as_str().unwrap_or_default().to_string()).filter(|m| !m.is_empty()).collect())
            .unwrap_or_default();
        if !messages.is_empty() {
            return Err(messages.join(" — "));
        }
        if rapport["_erreurSaisie_"].as_bool() == Some(true) {
            return Err(t!("Demande refusée par l'espace web", "Request refused by the web space").into());
        }
        Ok(str_of(&salle, "L"))
    }

    /// `next_iv` : IV à utiliser à partir de la réponse (ouverture de session).
    fn send(&mut self, nom: &str, donnees: Option<Value>, signature: Option<Value>, next_iv: Option<Vec<u8>>) -> Result<Value, String> {
        let mut root = serde_json::Map::new();
        if let Some(d) = donnees {
            root.insert("donnees".into(), d);
        }
        if let Some(sig) = signature {
            root.insert("_Signature_".into(), sig);
        }
        let text = Value::Object(root).to_string();
        let payload = if !self.crypt {
            Value::String(text)
        } else {
            let bytes = if self.compression { compress(&text) } else { text.into_bytes() };
            Value::String(aes_encrypt(&bytes, &self.key, &self.iv))
        };
        let ordre = aes_encrypt(self.ordre.to_string().as_bytes(), &self.key, &self.iv);
        if let Some(iv) = next_iv {
            self.iv = iv;
        }
        let body = json!({"session": self.session, "numeroOrdre": ordre, "nom": nom, "donneesSec": payload}).to_string();
        let url = format!("{}appelfonction/{}/{}/{}", self.base, self.espace, self.session, ordre);
        let resp = self.agent.post(&url).set("Content-Type", "application/json").send_string(&body).map_err(|e| tf!("Espace web : {e}", "Web space: {e}"))?;
        let j: Value = serde_json::from_str(&resp.into_string().map_err(|e| e.to_string())?)
            .map_err(|_| t!("Réponse illisible de l'espace web", "Unreadable web space response").to_string())?;
        if !j["Erreur"].is_null() {
            let e = &j["Erreur"];
            let msg = [str_of(e, "Titre"), str_of(e, "Message")].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" — ");
            return Err(if msg.is_empty() { t!("Session refusée par l'espace web", "Session refused by the web space").into() } else { msg });
        }
        let back = String::from_utf8(aes_decrypt(&str_of(&j, "numeroOrdre"), &self.key, &self.iv)?).unwrap_or_default();
        if back.trim().parse::<u64>().ok() != Some(self.ordre + 1) {
            return Err(t!("Session interrompue (n° d'ordre)", "Session interrupted (sequence number)").into());
        }
        self.ordre += 2;
        let sec = str_of(&j, "donneesSec");
        let text = if !self.crypt {
            sec
        } else {
            let raw = aes_decrypt(&sec, &self.key, &self.iv)?;
            let raw = if self.compression { inflate(&raw)? } else { raw };
            String::from_utf8(raw).map_err(|e| e.to_string())?
        };
        let root: Value = serde_json::from_str(&text).map_err(|_| t!("Réponse illisible de l'espace web", "Unreadable web space response").to_string())?;
        if let Some(true) = root["_Signature_"]["Erreur"].as_bool() {
            let m = str_of(&root["_Signature_"], "MessageErreur");
            return Err(if m.is_empty() { tf!("{nom} refusé par le serveur", "{nom} refused by the server") } else { m });
        }
        Ok(root)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Vecteurs calculés dans la page de l'espace web (bibliothèques forge et pako du site).
    #[test]
    fn aes_matches_site() {
        let iv = hex::decode("000102030405060708090a0b0c0d0e0f").unwrap();
        assert_eq!(aes_encrypt(b"1", b"", b""), "3fa959b13967e0ef176069e01e23c8d7");
        assert_eq!(aes_encrypt(b"3", b"", &iv), "18e4c3eb97be7e4cf58efbbe3c4a505e");
        let raw = aes_decrypt("09fa6c6d0d36b38935eb973cd1fd4becab9710f2dd24ec2e6d447474ffdd326b211b943fca6f0752729e445532286bfd", b"cle", &iv).unwrap();
        let hexed = String::from_utf8(inflate(&raw).unwrap()).unwrap();
        assert_eq!(String::from_utf8(hex::decode(hexed).unwrap()).unwrap(), r#"{"donnees":{"x":"é€"}}"#);
        // Notre compression se relit de la même façon.
        let ours = String::from_utf8(inflate(&compress("abc")).unwrap()).unwrap();
        assert_eq!(ours, hex::encode("abc"));
    }

    #[test]
    fn password_hash_matches_site() {
        let h = hex::encode_upper(sha2::Sha256::new().chain_update(b"ALEA").chain_update("Mdp€".as_bytes()).finalize());
        assert_eq!(h, "45F3FBBE006094AC6CBCC3812A70127BF75FCCE09FF421070DBA7939B5D7ED68");
    }

    #[test]
    fn grid_places_match_site() {
        // Grille de l'école : 24 places de 30 min à partir de 8 h ; premier lundi 17/08/2026.
        let heures: Vec<Value> = (0..24).map(|i| json!({"Debut": fmt_min(480 + 30 * i).replace(':', "h"), "Fin": fmt_min(510 + 30 * i).replace(':', "h")})).collect();
        let g = Grid::from_heures(&heures).unwrap();
        assert_eq!(g, Grid { start_min: 480, step_min: 30, places_per_day: 24 });
        let first = NaiveDate::from_ymd_opt(2026, 8, 17).unwrap();
        // Demande relevée sur le site : mardi 3 novembre 2026, 9 h – 9 h 30 → semaine 12, place 26, durée 1.
        assert_eq!(g.place(first, NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 540, 570), Ok((12, 26, 1)));
        assert!(g.place(first, NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 7 * 60, 8 * 60).is_err());
        assert!(g.place(first, NaiveDate::from_ymd_opt(2026, 11, 3).unwrap(), 545, 600).is_err());
        assert_eq!(parse_date(&json!({"_T": 7, "V": "17/08/2026 0:0:0"})), Some(first));
    }

    #[test]
    fn start_object_is_found() {
        let html = r#"<script>onloadStart=1</script><body onload="try { Start ({a:0,b:0,c:'ACCUEIL',i:'5699684',MR:'AB',ER:'01'}) } catch (e) {}">"#;
        assert_eq!(start_object(html), Some("{a:0,b:0,c:'ACCUEIL',i:'5699684',MR:'AB',ER:'01'}"));
        assert_eq!(start_object("<html>Navigateur non supporté</html>"), None);
    }

    #[test]
    fn start_params_are_read() {
        let s = "{a:0,b:0,c:'ACCUEIL',i:'5451591',MR:'B9AB',ER:'010001',poll:true}";
        assert_eq!(start_param(s, "a").as_deref(), Some("0"));
        assert_eq!(start_param(s, "i").as_deref(), Some("5451591"));
        assert_eq!(start_param(s, "MR").as_deref(), Some("B9AB"));
        assert_eq!(start_param(s, "R"), None);
        assert_eq!(start_param(s, "h"), None);
    }

    #[test]
    fn rsa_pads_to_key_size() {
        // Clé de test 512 bits (e = 65537) : sortie de 64 octets.
        let n = "c3a3d4f0f2b7a1b8e5f0a2c4d6e8f0a1b3c5d7e9f1a3b5c7d9e1f3a5b7c9d1e3c3a3d4f0f2b7a1b8e5f0a2c4d6e8f0a1b3c5d7e9f1a3b5c7d9e1f3a5b7c9d1e3";
        let out = rsa_encrypt(&[7u8; 16], n, "010001").unwrap();
        let bytes = base64::engine::general_purpose::STANDARD.decode(out.replace("\r\n", "")).unwrap();
        assert_eq!(bytes.len(), 64);
    }
}
