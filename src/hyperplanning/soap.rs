//! Client SOAP 1.2 minimal (RPC / literal) du service web HyperPlanning.
//!
//! - Une interface par point d'accès : `IHpSvcWCours` → `<base>/Cours`.
//! - Authentification HTTP Basic (401 : identifiants non transmis, 403 : refusés).
//! - **Lecture seule** : toute opération qui modifie la base est refusée avant l'envoi.

use base64::Engine;
use std::time::Duration;

const NS: &str = "http://www.indexeducation.com/frahtm/HpSvcW.html";
const ACTION: &str = "urn:HpSvcW2022";

/// Préfixes des opérations qui modifient la base HyperPlanning : jamais envoyées.
const WRITE_PREFIXES: &[&str] = &[
    "Modifier",
    "Creer",
    "Supprimer",
    "Ajouter",
    "Enlever",
    "Affecter",
    "Desaffecter",
    "Remplacer",
    "Fusionner",
    "Annuler",
    "Restaurer",
    "Placer",
    "Deplacer",
    "Depositionner",
    "Dupliquer",
    "Importer",
    "Definir",
    "Activer",
    "Desactiver",
    "Envoyer",
    "Saisir",
    "Renommer",
    "Valider",
    "Copier",
    "Associer",
    "Dissocier",
    "Reinitialiser",
    "Rendre",
    "Autoriser",
    "NePas",
    "Publier",
    "Vider",
    "Retirer",
    "AccederOuCreer",
];

pub fn is_write_operation(op: &str) -> bool {
    WRITE_PREFIXES.iter().any(|p| op.starts_with(p))
}

/// Adresse de base du service (`http://serveur:port/hpsw`), quelle que soit la page collée
/// (`…/hpsw/fr`, `…/hpsw/wsdl`, adresse sans `/hpsw`…).
pub fn normalize_base(url: &str) -> String {
    let mut u = url.trim().trim_end_matches('/').to_string();
    if !u.is_empty() && !u.contains("://") {
        u = format!("http://{u}");
    }
    if let Some(i) = u.find("/hpsw") {
        u.truncate(i + "/hpsw".len());
    } else if !u.is_empty() {
        u.push_str("/hpsw");
    }
    u
}

/// Échappe un texte pour le XML.
pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Paramètre tableau : `<hp:THpSvcWCleCours>1</hp:THpSvcWCleCours>…`.
pub fn arg_list<T: ToString>(item: &str, values: &[T]) -> String {
    values.iter().map(|v| format!("<hp:{item}>{}</hp:{item}>", escape(&v.to_string()))).collect()
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Enveloppe SOAP de l'appel `op` avec ses paramètres (contenu XML déjà échappé).
fn envelope(op: &str, params: &[(&str, String)]) -> String {
    let args: String = params.iter().map(|(n, v)| format!("<{n}>{v}</{n}>")).collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><soap:Envelope xmlns:soap="http://www.w3.org/2003/05/soap-envelope" xmlns:hp="{NS}"><soap:Body><hp:{op}>{args}</hp:{op}></soap:Body></soap:Envelope>"#
    )
}

/// Contenu texte de tous les éléments de nom local `local` (préfixe ignoré), dans l'ordre.
/// Un élément vide (`<x/>`) donne une chaîne vide.
pub fn elements(xml: &str, local: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt + 1..];
        let end = rest.find(['>', ' ', '/', '\t', '\r', '\n']).unwrap_or(rest.len());
        let name = &rest[..end];
        let bare = name.rsplit(':').next().unwrap_or(name);
        if bare != local || name.starts_with('/') {
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        if rest[..gt].ends_with('/') {
            out.push(String::new());
            rest = &rest[gt + 1..];
            continue;
        }
        let body = &rest[gt + 1..];
        let close = format!("</{name}>");
        let Some(c) = body.find(&close) else { break };
        out.push(unescape(&body[..c]));
        rest = &body[c + close.len()..];
    }
    out
}

/// Message d'une erreur SOAP (« Fault »), s'il y en a une.
fn fault(xml: &str) -> Option<String> {
    if !xml.contains("Fault") {
        return None;
    }
    let text = elements(xml, "Text").into_iter().chain(elements(xml, "faultstring")).find(|t| !t.trim().is_empty());
    Some(text.unwrap_or_else(|| t!("Erreur du serveur HyperPlanning", "HyperPlanning server error").into()))
}

#[derive(Clone)]
pub struct Client {
    base: String,
    auth: String,
    agent: ureq::Agent,
    /// Nombre de requêtes envoyées (suivi de la consommation).
    calls: std::sync::Arc<std::sync::atomic::AtomicU32>,
}

impl Client {
    pub fn new(url: &str, login: &str, password: &str) -> Self {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{login}:{password}"));
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(10)).timeout(Duration::from_secs(120)).build();
        Self { base: normalize_base(url), auth: format!("Basic {token}"), agent, calls: Default::default() }
    }

    /// Appelle l'opération `op` de l'interface `iface` (« Cours », « Salles », « Admin »…) et
    /// renvoie le corps XML de la réponse.
    pub fn call(&self, iface: &str, op: &str, params: &[(&str, String)]) -> Result<String, String> {
        if is_write_operation(op) {
            return Err(tf!("Opération d'écriture refusée : {op}", "Write operation refused: {op}"));
        }
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ct = format!(r#"application/soap+xml; charset=utf-8; action="{ACTION}:IHpSvcW{iface}#{op}""#);
        let resp = self.agent.post(&format!("{}/{iface}", self.base)).set("Content-Type", &ct).set("Authorization", &self.auth).send_string(&envelope(op, params));
        match resp {
            Ok(r) => {
                let body = r.into_string().map_err(|e| e.to_string())?;
                fault(&body).map_or(Ok(body), Err)
            }
            Err(ureq::Error::Status(401, _)) => Err(t!("Identifiants non transmis au serveur (401)", "Credentials not sent to the server (401)").into()),
            Err(ureq::Error::Status(403, _)) => Err(t!("Identifiant ou mot de passe refusé (403)", "Login or password refused (403)").into()),
            Err(ureq::Error::Status(code, r)) => {
                let body = r.into_string().unwrap_or_default();
                Err(fault(&body).unwrap_or_else(|| tf!("Erreur HTTP {code}", "HTTP error {code}")))
            }
            Err(ureq::Error::Transport(t)) => Err(tf!("Serveur injoignable : {t}", "Server unreachable: {t}")),
        }
    }

    /// Valeur de retour (`return`) d'un appel qui renvoie une valeur simple.
    pub fn call_value(&self, iface: &str, op: &str, params: &[(&str, String)]) -> Result<String, String> {
        let body = self.call(iface, op, params)?;
        elements(&body, "return").into_iter().next().ok_or_else(|| t!("Réponse inattendue du serveur", "Unexpected server response").into())
    }

    /// Nombre de requêtes envoyées par ce client.
    pub fn calls(&self) -> u32 {
        self.calls.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Version du serveur (sert de test de connexion).
    pub fn version(&self) -> Result<String, String> {
        self.call_value("Admin", "Version", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_is_normalized() {
        assert_eq!(normalize_base("http://hp.ecole.fr:8086/hpsw/fr"), "http://hp.ecole.fr:8086/hpsw");
        assert_eq!(normalize_base(" http://hp.ecole.fr:8086/hpsw/wsdl/IHpSvcWCours/ "), "http://hp.ecole.fr:8086/hpsw");
        assert_eq!(normalize_base("hp.ecole.fr:8086"), "http://hp.ecole.fr:8086/hpsw");
        assert_eq!(normalize_base(""), "");
    }

    #[test]
    fn writes_are_refused() {
        assert!(is_write_operation("CreerCoursFixe"));
        assert!(is_write_operation("ModifierPlaceCours"));
        assert!(is_write_operation("SupprimerCours"));
        assert!(!is_write_operation("TousLesCoursEntre2Dates"));
        assert!(!is_write_operation("DetailDesSeancesPlaceesTableauDeCours"));
        let c = Client::new("http://127.0.0.1:9/hpsw", "a", "b");
        assert!(c.call("Cours", "SupprimerCours", &[]).unwrap_err().contains("SupprimerCours"));
    }

    #[test]
    fn envelope_and_parsing() {
        let e = envelope("CoursSalle", &[("ASalle", "12".into()), ("ATableau", "<hp:THpSvcWCleCours>1</hp:THpSvcWCleCours>".into())]);
        assert!(e.contains("<hp:CoursSalle><ASalle>12</ASalle><ATableau><hp:THpSvcWCleCours>1</hp:THpSvcWCleCours>"));
        let r = r#"<env:Envelope><env:Body><ns1:Rep><return><ns2:THpSvcWCleCours>3</ns2:THpSvcWCleCours><ns2:THpSvcWCleCours/><THpSvcWCleCours>R&amp;D</THpSvcWCleCours></return></ns1:Rep></env:Body></env:Envelope>"#;
        assert_eq!(elements(r, "THpSvcWCleCours"), vec!["3", "", "R&D"]);
        assert_eq!(elements(r, "Rep").len(), 1);
        let f = r#"<env:Envelope><env:Body><env:Fault><env:Reason><env:Text xml:lang="fr">Clé invalide</env:Text></env:Reason></env:Fault></env:Body></env:Envelope>"#;
        assert_eq!(fault(f).as_deref(), Some("Clé invalide"));
    }
}
