//! Appels à l'API Gmail v1 et conversion vers des types simples.

use base64::Engine;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;

use super::Session;
use super::html::{self, BodyBlock, ImageSrc};

const API: &str = "https://gmail.googleapis.com/gmail/v1/users/me";

/// base64url tolérant (Gmail renvoie parfois du padding, parfois non).
const B64: GeneralPurpose = GeneralPurpose::new(&base64::alphabet::URL_SAFE, GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent));

// --- Types exposés ---------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Label {
    pub id: String,
    pub name: String,
    pub user: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Summary {
    pub id: String,
    pub thread_id: String,
    pub from: String,
    pub subject: String,
    pub snippet: String,
    /// Horodatage en millisecondes (epoch).
    pub date: i64,
    pub labels: Vec<String>,
    /// Destinataires (To + Cc) : sert à repérer les adresses de groupe.
    pub recipients: String,
    /// En-tête List-Id (listes de diffusion / Google Groups).
    pub list_id: String,
}

impl Summary {
    pub fn unread(&self) -> bool {
        self.labels.iter().any(|l| l == "UNREAD")
    }
    pub fn starred(&self) -> bool {
        self.labels.iter().any(|l| l == "STARRED")
    }
    /// Nom affichable de l'expéditeur (« Jean Dupont » plutôt que l'adresse complète).
    pub fn sender_name(&self) -> &str {
        let f = self.from.trim();
        match f.split_once('<') {
            Some((name, _)) if !name.trim().is_empty() => name.trim().trim_matches('"'),
            Some((_, addr)) => addr.trim_end_matches('>'),
            None => f,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FullMessage {
    pub summary: Summary,
    pub to: String,
    pub cc: String,
    pub reply_to: String,
    pub message_id: String,
    pub references: String,
    pub body: String,
    /// Corps découpé en texte / images, dans l'ordre d'affichage.
    pub blocks: Vec<BodyBlock>,
    pub attachments: Vec<Attachment>,
}

impl FullMessage {
    /// Pièce jointe correspondant à une image intégrée (`cid:`).
    pub fn by_cid(&self, cid: &str) -> Option<&Attachment> {
        self.attachments.iter().find(|a| a.content_id.as_deref() == Some(cid))
    }

    /// Pièces jointes à lister (sans les images déjà affichées dans le corps).
    pub fn listed_attachments(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .iter()
            .filter(|a| !a.content_id.as_ref().is_some_and(|cid| self.blocks.iter().any(|b| matches!(b, BodyBlock::Image { src: ImageSrc::Cid(c), .. } if c == cid))))
    }
}

#[derive(Clone, Debug, Default)]
pub struct Attachment {
    pub message_id: String,
    /// Identifiant Gmail de la pièce jointe (absent si le contenu est intégré au message).
    pub attachment_id: Option<String>,
    pub filename: String,
    pub mime: String,
    pub size: u64,
    /// Contenu déjà présent dans le message (petites pièces jointes).
    pub inline_data: Option<Vec<u8>>,
    /// Content-ID (sans < >) : image référencée dans le HTML par `cid:`.
    pub content_id: Option<String>,
}

impl Attachment {
    /// Clé unique (message + nom + position) pour le cache et l'état d'aperçu.
    pub fn key(&self) -> String {
        format!("{}/{}/{}", self.message_id, self.attachment_id.as_deref().unwrap_or("-"), self.filename)
    }

    pub fn extension(&self) -> String {
        self.filename.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default()
    }
}

// --- Réponses brutes -------------------------------------------------------

#[derive(Deserialize)]
struct RawMessage {
    id: String,
    #[serde(rename = "threadId", default)]
    thread_id: String,
    #[serde(rename = "labelIds", default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    #[serde(rename = "internalDate", default)]
    internal_date: String,
    payload: Option<Part>,
}

#[derive(Deserialize)]
struct Part {
    #[serde(rename = "mimeType", default)]
    mime_type: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<Header>,
    body: Option<Body>,
    #[serde(default)]
    parts: Vec<Part>,
}

#[derive(Deserialize)]
struct Header {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct Body {
    data: Option<String>,
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
    #[serde(default)]
    size: u64,
}

impl RawMessage {
    fn header(&self, name: &str) -> String {
        self.payload.as_ref().and_then(|p| p.headers.iter().find(|h| h.name.eq_ignore_ascii_case(name))).map(|h| h.value.clone()).unwrap_or_default()
    }

    fn summary(&self) -> Summary {
        Summary {
            id: self.id.clone(),
            thread_id: self.thread_id.clone(),
            from: self.header("From"),
            subject: self.header("Subject"),
            snippet: decode_entities(&self.snippet),
            date: self.internal_date.parse().unwrap_or(0),
            labels: self.label_ids.clone(),
            recipients: [self.header("To"), self.header("Cc")].into_iter().filter(|h| !h.is_empty()).collect::<Vec<_>>().join(", "),
            list_id: self.header("List-Id"),
        }
    }
}

fn parse<T: for<'de> Deserialize<'de>>(v: serde_json::Value) -> Result<T, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

// --- Appels ----------------------------------------------------------------

/// Adresse du compte et identifiant d'historique courant.
pub fn profile(s: &Session) -> Result<(String, String), String> {
    let v = s.get(&format!("{API}/profile"), &[])?;
    Ok((v["emailAddress"].as_str().unwrap_or_default().to_string(), v["historyId"].as_str().unwrap_or_default().to_string()))
}

/// Mémorise le point de départ de la synchronisation incrémentale
/// (à appeler AVANT le chargement complet pour ne rien rater entre les deux).
pub fn mark_history(s: &Session) -> Result<(), String> {
    let (_, history) = profile(s)?;
    *s.history_id.lock().map_err(|_| "verrou")? = Some(history);
    Ok(())
}

pub fn labels(s: &Session) -> Result<Vec<Label>, String> {
    let v = s.get(&format!("{API}/labels"), &[])?;
    let mut out: Vec<Label> = v["labels"].as_array().map(|a| a.iter().map(label_from_json).collect()).unwrap_or_default();
    out.sort_by_key(|l| l.name.to_lowercase());
    Ok(out)
}

fn label_from_json(l: &serde_json::Value) -> Label {
    Label { id: l["id"].as_str().unwrap_or_default().to_string(), name: l["name"].as_str().unwrap_or_default().to_string(), user: l["type"].as_str() == Some("user") }
}

pub fn create_label(s: &Session, name: &str) -> Result<Label, String> {
    let v = s.post(&format!("{API}/labels"), json!({ "name": name, "labelListVisibility": "labelShow", "messageListVisibility": "show" }))?;
    // La réponse de création ne contient pas toujours le champ « type » :
    // un libellé créé ici est forcément une catégorie de l'utilisateur.
    let mut l = label_from_json(&v);
    l.user = true;
    if l.name.is_empty() {
        l.name = name.to_string();
    }
    Ok(l)
}

/// Les `max` derniers messages portant ce libellé.
/// 1 requête pour la liste + 1 par message jamais vu (les autres viennent du cache).
/// Un `label` commençant par `from:` est une recherche Gmail (mails d'une adresse).
pub fn list(s: &Session, label: &str, max: usize) -> Result<Vec<Summary>, String> {
    let max_s = max.to_string();
    let group_q;
    let filter = if let Some(q) = label.strip_prefix("search:") {
        ("q", q)
    } else if label.starts_with("from:") {
        ("q", label)
    } else if let Some(addr) = label.strip_prefix("group:") {
        group_q = group_query(addr);
        ("q", group_q.as_str())
    } else {
        ("labelIds", label)
    };
    let v = s.get(&format!("{API}/messages"), &[filter, ("maxResults", &max_s)])?;
    let ids: Vec<String> = v["messages"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();

    fetch_missing(s, &ids)?;
    let cache = s.cache.lock().map_err(|_| "verrou")?;
    let mut out: Vec<Summary> = ids.iter().filter_map(|id| cache.get(id).cloned()).collect();
    out.sort_by_key(|m| std::cmp::Reverse(m.date));
    Ok(out)
}

/// Recherche Gmail des mails passés par une adresse de groupe : destinataire (À, Cc, Cci), liste
/// de diffusion (`List-Id`) ou adresse de remise.
pub fn group_query(addr: &str) -> String {
    let a = addr.trim().to_lowercase();
    format!("{{to:{a} cc:{a} bcc:{a} list:{a} deliveredto:{a}}}")
}

/// Télécharge les métadonnées des messages absents du cache.
fn fetch_missing(s: &Session, ids: &[String]) -> Result<(), String> {
    let missing: Vec<&String> = {
        let cache = s.cache.lock().map_err(|_| "verrou")?;
        ids.iter().filter(|id| !cache.contains_key(*id)).collect()
    };
    if missing.is_empty() {
        return Ok(());
    }
    // Quelques threads ; le limiteur de la session plafonne de toute façon la concurrence.
    let chunk = missing.len().div_ceil(4).max(1);
    let results: Vec<Result<Vec<Summary>, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = missing.chunks(chunk).map(|c| scope.spawn(move || c.iter().map(|id| metadata(s, id)).collect())).collect();
        handles.into_iter().map(|h| h.join().unwrap_or_else(|_| Err("thread".into()))).collect()
    });
    let mut cache = s.cache.lock().map_err(|_| "verrou")?;
    for r in results {
        for m in r? {
            cache.insert(m.id.clone(), m);
        }
    }
    Ok(())
}

fn metadata(s: &Session, id: &str) -> Result<Summary, String> {
    let v = s.get(
        &format!("{API}/messages/{id}"),
        &[
            ("format", "metadata"),
            ("metadataHeaders", "From"),
            ("metadataHeaders", "Subject"),
            ("metadataHeaders", "To"),
            ("metadataHeaders", "Cc"),
            ("metadataHeaders", "List-Id"),
        ],
    )?;
    Ok(parse::<RawMessage>(v)?.summary())
}

pub enum HistoryResult {
    Delta {
        changed: Vec<Summary>,
        deleted: Vec<String>,
    },
    /// Point de départ expiré ou inconnu : recharger entièrement.
    Expired,
}

/// Synchronisation incrémentale : tout ce qui a changé depuis la dernière relève,
/// en général en UNE seule requête (2 unités de quota au lieu de ~700).
pub fn history(s: &Session) -> Result<HistoryResult, String> {
    let Some(start) = s.history_id.lock().map_err(|_| "verrou")?.clone() else {
        return Ok(HistoryResult::Expired);
    };

    let mut labels_of: HashMap<String, Vec<String>> = HashMap::new();
    let mut deleted: Vec<String> = Vec::new();
    let mut page: Option<String> = None;
    let mut latest = start.clone();

    loop {
        let mut q = vec![("startHistoryId", start.as_str()), ("maxResults", "500")];
        if let Some(p) = &page {
            q.push(("pageToken", p.as_str()));
        }
        let v = match s.get(&format!("{API}/history"), &q) {
            Ok(v) => v,
            Err(e) if e.starts_with("Erreur 404") || e.starts_with("Error 404") => return Ok(HistoryResult::Expired),
            Err(e) => return Err(e),
        };
        if let Some(h) = v["historyId"].as_str() {
            latest = h.to_string();
        }
        for rec in v["history"].as_array().into_iter().flatten() {
            for kind in ["messagesAdded", "labelsAdded", "labelsRemoved"] {
                for item in rec[kind].as_array().into_iter().flatten() {
                    let m = &item["message"];
                    if let Some(id) = m["id"].as_str() {
                        let labels = m["labelIds"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
                        labels_of.insert(id.to_string(), labels);
                    }
                }
            }
            for item in rec["messagesDeleted"].as_array().into_iter().flatten() {
                if let Some(id) = item["message"]["id"].as_str() {
                    labels_of.remove(id);
                    deleted.push(id.to_string());
                }
            }
        }
        page = v["nextPageToken"].as_str().map(str::to_string);
        if page.is_none() {
            break;
        }
    }

    // Nouveaux messages : on ne télécharge que ceux qu'on n'a jamais vus.
    let ids: Vec<String> = labels_of.keys().cloned().collect();
    fetch_missing(s, &ids)?;

    let mut changed = Vec::new();
    {
        let mut cache = s.cache.lock().map_err(|_| "verrou")?;
        for (id, labels) in labels_of {
            if let Some(m) = cache.get_mut(&id) {
                m.labels = labels;
                changed.push(m.clone());
            }
        }
        for id in &deleted {
            cache.remove(id);
        }
    }
    *s.history_id.lock().map_err(|_| "verrou")? = Some(latest);
    Ok(HistoryResult::Delta { changed, deleted })
}

pub fn get_full(s: &Session, id: &str) -> Result<FullMessage, String> {
    let raw: RawMessage = parse(s.get(&format!("{API}/messages/{id}"), &[("format", "full")])?)?;
    Ok(full_from_raw(raw))
}

/// Type de fichier partagé affiché au tableau de bord.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedKind {
    Pdf,
    Doc,
    Sheet,
    Form,
}

/// Fichier partagé par mail : PDF en pièce jointe, ou lien Google Docs / Sheets / Forms.
#[derive(Clone, Debug)]
pub struct SharedFile {
    pub kind: SharedKind,
    pub title: String,
    /// Lien Google (documents) ; `None` pour une pièce jointe PDF.
    pub url: Option<String>,
    pub attachment: Option<Attachment>,
}

/// Type et identifiant d'un lien Google Docs / Sheets / Forms.
pub fn google_doc(url: &str) -> Option<(SharedKind, String)> {
    let u = url.split(['?', '#']).next().unwrap_or(url);
    let rest = u.split("docs.google.com/").nth(1)?;
    let kind = if rest.starts_with("document/") {
        SharedKind::Doc
    } else if rest.starts_with("spreadsheets/") {
        SharedKind::Sheet
    } else if rest.starts_with("forms/") {
        SharedKind::Form
    } else {
        return None;
    };
    // …/d/<id>/… ou …/d/e/<id>/… (formulaire publié).
    let mut parts = rest.split('/').skip_while(|p| *p != "d").skip(1);
    let id = match parts.next()? {
        "e" => parts.next()?,
        id => id,
    };
    (id.len() > 10).then(|| (kind, id.to_string()))
}

/// Titre d'un document : texte du lien s'il est parlant, sinon le nom entre guillemets du sujet.
fn doc_title(link_text: &str, subject: &str, kind: SharedKind) -> String {
    let t = link_text.trim();
    let generic = t.len() < 3 || t.starts_with("http") || ["ouvrir", "open", "afficher", "view", "ici", "here", "lien", "link"].iter().any(|g| t.eq_ignore_ascii_case(g));
    if !generic {
        return t.to_string();
    }
    for (a, b) in [('“', '”'), ('"', '"'), ('«', '»')] {
        if let Some(i) = subject.find(a) {
            let rest = &subject[i + a.len_utf8()..];
            if let Some(j) = rest.find(b) {
                let inner = rest[..j].trim();
                if !inner.is_empty() {
                    return inner.to_string();
                }
            }
        }
    }
    if !subject.trim().is_empty() {
        return subject.trim().to_string();
    }
    match kind {
        SharedKind::Doc => "Google Docs".into(),
        SharedKind::Sheet => "Google Sheets".into(),
        SharedKind::Form => "Google Forms".into(),
        SharedKind::Pdf => "PDF".into(),
    }
}

/// Derniers fichiers partagés par mail : PDF joints et documents Google (Docs, Sheets, Forms).
pub fn recent_shared(s: &Session, n: usize) -> Result<Vec<(Summary, SharedFile)>, String> {
    let v = s.get(&format!("{API}/messages"), &[("q", "(filename:pdf OR \"docs.google.com\") -in:sent -in:chats -in:spam -in:trash"), ("maxResults", "40")])?;
    let mut out: Vec<(Summary, SharedFile)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in v["messages"].as_array().into_iter().flatten() {
        if out.len() >= n {
            break;
        }
        let Some(id) = m["id"].as_str() else { continue };
        let Ok(full) = get_full(s, id) else { continue };
        for a in full.listed_attachments() {
            if (a.mime == "application/pdf" || a.extension().eq_ignore_ascii_case("pdf")) && seen.insert(a.key()) {
                out.push((full.summary.clone(), SharedFile { kind: SharedKind::Pdf, title: a.filename.clone(), url: None, attachment: Some(a.clone()) }));
            }
        }
        for b in &full.blocks {
            let BodyBlock::Para(p) = b else { continue };
            for r in &p.runs {
                let Some(url) = &r.link else { continue };
                let Some((kind, doc)) = google_doc(url) else { continue };
                if seen.insert(doc) {
                    let title = doc_title(&r.text, &full.summary.subject, kind);
                    out.push((full.summary.clone(), SharedFile { kind, title, url: Some(url.clone()), attachment: None }));
                }
            }
        }
    }
    out.sort_by_key(|(m, _)| -m.date);
    out.truncate(n);
    Ok(out)
}

/// Conversation complète (tous les messages du fil, du plus ancien au plus récent).
pub fn get_thread(s: &Session, thread_id: &str) -> Result<Vec<FullMessage>, String> {
    let v = s.get(&format!("{API}/threads/{thread_id}"), &[("format", "full")])?;
    let raws: Vec<RawMessage> = parse(v["messages"].clone()).unwrap_or_default();
    let mut out: Vec<FullMessage> = raws.into_iter().map(full_from_raw).collect();
    out.sort_by_key(|m| m.summary.date);
    Ok(out)
}

/// Nombre de messages de chaque conversation (requête minimale, un fil à la fois).
pub fn thread_sizes(s: &Session, ids: &[String]) -> Result<Vec<(String, usize)>, String> {
    let mut out = Vec::new();
    for id in ids {
        // Un échec compte pour 1 : pas de nouvel essai en boucle.
        let n =
            s.get(&format!("{API}/threads/{id}"), &[("format", "minimal"), ("fields", "messages/id")]).ok().and_then(|v| v["messages"].as_array().map(|a| a.len())).unwrap_or(1);
        out.push((id.clone(), n));
    }
    Ok(out)
}

fn full_from_raw(raw: RawMessage) -> FullMessage {
    let mut plain = None;
    let mut html = None;
    let mut attachments = Vec::new();
    if let Some(p) = &raw.payload {
        find_bodies(p, &mut plain, &mut html);
        find_attachments(p, &raw.id, &mut attachments);
    }
    // Affichage : le HTML (mise en forme Gmail + images) quand il existe, sinon le texte
    // brut avec interprétation du Markdown. Le texte brut sert aussi à citer le message.
    let (body, blocks) = match (plain, html) {
        (plain, Some(h)) => {
            let text = plain.unwrap_or_else(|| html::to_text(&h));
            (text, html::from_html(&h))
        }
        (Some(t), None) => {
            let blocks = html::from_plain(&t);
            (t, blocks)
        }
        _ => (raw.snippet.clone(), html::from_plain(&raw.snippet)),
    };

    FullMessage {
        to: raw.header("To"),
        cc: raw.header("Cc"),
        reply_to: raw.header("Reply-To"),
        message_id: raw.header("Message-ID"),
        references: raw.header("References"),
        summary: raw.summary(),
        body: body.replace("\r\n", "\n"),
        blocks,
        attachments,
    }
}

fn find_bodies(p: &Part, plain: &mut Option<String>, html: &mut Option<String>) {
    let data = p.body.as_ref().and_then(|b| b.data.as_deref());
    // Une pièce jointe .txt / .html n'est pas le corps du message.
    if let (Some(d), true) = (data, p.filename.is_empty()) {
        let text = B64.decode(d).ok().map(|b| String::from_utf8_lossy(&b).into_owned());
        match p.mime_type.as_str() {
            "text/plain" if plain.is_none() => *plain = text,
            "text/html" if html.is_none() => *html = text,
            _ => {}
        }
    }
    for child in &p.parts {
        find_bodies(child, plain, html);
    }
}

fn find_attachments(p: &Part, message_id: &str, out: &mut Vec<Attachment>) {
    let content_id = p
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("Content-ID") || h.name.eq_ignore_ascii_case("X-Attachment-Id"))
        .map(|h| h.value.trim().trim_matches(['<', '>']).to_string());
    let inline_image = content_id.is_some() && p.mime_type.starts_with("image/");
    if !p.filename.is_empty() || inline_image {
        let body = p.body.as_ref();
        let filename = if p.filename.is_empty() { format!("image.{}", p.mime_type.trim_start_matches("image/")) } else { p.filename.clone() };
        out.push(Attachment {
            message_id: message_id.to_string(),
            attachment_id: body.and_then(|b| b.attachment_id.clone()),
            content_id,
            filename,
            mime: p.mime_type.clone(),
            size: body.map(|b| b.size).unwrap_or(0),
            inline_data: body.and_then(|b| b.data.as_deref()).and_then(|d| B64.decode(d).ok()),
        });
    }
    for child in &p.parts {
        find_attachments(child, message_id, out);
    }
}

/// Télécharge le contenu d'une pièce jointe.
pub fn attachment(s: &Session, a: &Attachment) -> Result<Vec<u8>, String> {
    if let Some(d) = &a.inline_data {
        return Ok(d.clone());
    }
    let id = a.attachment_id.as_deref().ok_or(t!("Pièce jointe sans contenu", "Attachment has no content"))?;
    let v = s.get(&format!("{API}/messages/{}/attachments/{id}", a.message_id), &[])?;
    let data = v["data"].as_str().ok_or(t!("Pièce jointe vide", "Empty attachment"))?;
    B64.decode(data).map_err(|e| tf!("Pièce jointe illisible : {e}", "Unreadable attachment: {e}"))
}

/// Ajoute / retire des libellés. Renvoie la nouvelle liste de libellés du message.
pub fn modify(s: &Session, id: &str, add: &[String], remove: &[String]) -> Result<Vec<String>, String> {
    let v = s.post(&format!("{API}/messages/{id}/modify"), json!({ "addLabelIds": add, "removeLabelIds": remove }))?;
    let labels = parse::<RawMessage>(v)?.label_ids;
    if let Some(m) = s.cache.lock().map_err(|_| "verrou")?.get_mut(id) {
        m.labels = labels.clone();
    }
    Ok(labels)
}

/// Met un message à la corbeille (`trash = true`) ou l'en sort ; renvoie ses libellés.
pub fn trash(s: &Session, id: &str, trash: bool) -> Result<Vec<String>, String> {
    let verb = if trash { "trash" } else { "untrash" };
    let v = s.post(&format!("{API}/messages/{id}/{verb}"), json!({}))?;
    let labels = parse::<RawMessage>(v)?.label_ids;
    if let Some(m) = s.cache.lock().map_err(|_| "verrou")?.get_mut(id) {
        m.labels = labels.clone();
    }
    Ok(labels)
}

/// Met toute une conversation à la corbeille (ou l'en sort) ; renvoie les libellés de chaque
/// message : `(identifiant, libellés)`.
pub fn trash_thread(s: &Session, thread_id: &str, trash: bool) -> Result<Vec<(String, Vec<String>)>, String> {
    let verb = if trash { "trash" } else { "untrash" };
    let v = s.post(&format!("{API}/threads/{thread_id}/{verb}"), json!({}))?;
    let mut out = Vec::new();
    for m in v["messages"].as_array().cloned().unwrap_or_default() {
        let id = m["id"].as_str().unwrap_or_default().to_string();
        let labels: Vec<String> = m["labelIds"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(str::to_string)).collect()).unwrap_or_default();
        if let Some(c) = s.cache.lock().map_err(|_| "verrou")?.get_mut(&id) {
            c.labels = labels.clone();
        }
        out.push((id, labels));
    }
    Ok(out)
}

/// Envoie un message MIME déjà construit.
/// Passe par le point d'entrée de téléversement (jusqu'à 35 Mo, contre ~5 Mo en JSON),
/// nécessaire pour les images et pièces jointes.
pub fn send(s: &Session, mime: &[u8], thread_id: Option<&str>) -> Result<(), String> {
    const UPLOAD: &str = "https://gmail.googleapis.com/upload/gmail/v1/users/me/messages/send?uploadType=multipart";
    let mut meta = json!({});
    if let Some(t) = thread_id {
        meta["threadId"] = json!(t);
    }
    let boundary = format!("worktable_{}", std::process::id());
    let mut body = Vec::with_capacity(mime.len() + 512);
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: message/rfc822\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(mime);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    s.post_bytes(UPLOAD, &format!("multipart/related; boundary={boundary}"), &body).map(|_| ())
}

// --- Brouillons ----------------------------------------------------------------

/// Crée (`id` absent) ou met à jour le brouillon Gmail ; renvoie son identifiant et celui de son
/// message. Il apparaît dans « Brouillons » de Gmail, sur tous les appareils.
pub fn save_draft(s: &Session, id: Option<&str>, mime: &[u8], thread_id: Option<&str>) -> Result<(String, String), String> {
    let mut msg = json!({ "raw": B64.encode(mime) });
    if let Some(t) = thread_id {
        msg["threadId"] = json!(t);
    }
    let v = match id {
        Some(id) => match s.send("PUT", &format!("{API}/drafts/{id}"), &[], Some(json!({ "id": id, "message": msg.clone() }))) {
            Ok(v) => v,
            // Supprimé entre-temps (dans Gmail) : recréé.
            Err(e) if e.contains("404") || e.to_lowercase().contains("not found") => s.post(&format!("{API}/drafts"), json!({ "message": msg }))?,
            Err(e) => return Err(e),
        },
        None => s.post(&format!("{API}/drafts"), json!({ "message": msg }))?,
    };
    let id = v["id"].as_str().ok_or_else(|| "brouillon sans identifiant".to_string())?;
    Ok((id.to_string(), v["message"]["id"].as_str().unwrap_or_default().to_string()))
}

/// Supprime un brouillon Gmail (déjà supprimé : rien à faire).
pub fn delete_draft(s: &Session, id: &str) -> Result<(), String> {
    match s.send("DELETE", &format!("{API}/drafts/{id}"), &[], None) {
        Ok(_) => Ok(()),
        Err(e) if e.contains("404") || e.to_lowercase().contains("not found") => Ok(()),
        Err(e) => Err(e),
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&#39;", "'").replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ").replace("&amp;", "&")
}

// --- Signatures --------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    /// Libellé affiché (« Gmail · adresse » ou « Mails envoyés »).
    pub label: String,
    pub email: String,
    /// HTML exact de la signature, tel que Gmail l'insère.
    pub html: String,
}

fn normalize(html: &str) -> String {
    html.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Signatures disponibles : celles des réglages Gmail (« Envoyer en tant que »),
/// sinon celle trouvée dans les derniers mails envoyés (bloc `gmail_signature`).
pub fn signatures(s: &Session) -> Result<Vec<Signature>, String> {
    let mut out: Vec<Signature> = Vec::new();
    if let Ok(v) = s.get(&format!("{API}/settings/sendAs"), &[]) {
        for a in v["sendAs"].as_array().into_iter().flatten() {
            let html = a["signature"].as_str().unwrap_or_default().trim().to_string();
            if html.is_empty() {
                continue;
            }
            let email = a["sendAsEmail"].as_str().unwrap_or_default().to_string();
            let primary = a["isPrimary"].as_bool().unwrap_or(false) || a["isDefault"].as_bool().unwrap_or(false);
            let sig = Signature { label: format!("Gmail · {email}"), email, html };
            if primary { out.insert(0, sig) } else { out.push(sig) }
        }
    }
    if out.is_empty()
        && let Some(sig) = signature_from_sent(s)?
    {
        out.push(sig);
    }
    Ok(out)
}

/// Cherche la signature dans les derniers mails envoyés (au plus 12 lus).
fn signature_from_sent(s: &Session) -> Result<Option<Signature>, String> {
    let v = s.get(&format!("{API}/messages"), &[("labelIds", "SENT"), ("maxResults", "12")])?;
    let ids: Vec<String> = v["messages"].as_array().map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).collect()).unwrap_or_default();
    for id in ids {
        let raw: RawMessage = parse(s.get(&format!("{API}/messages/{id}"), &[("format", "full")])?)?;
        let (mut plain, mut html) = (None, None);
        if let Some(p) = &raw.payload {
            find_bodies(p, &mut plain, &mut html);
        }
        if let Some(sig) = html.as_deref().and_then(extract_signature) {
            return Ok(Some(Signature { label: t!("Signature de tes mails envoyés", "Signature from your sent mails").into(), email: raw.header("From"), html: sig }));
        }
    }
    Ok(None)
}

/// Bloc signature d'un mail écrit avec Gmail (`<div class="gmail_signature">…</div>`).
pub fn extract_signature(html: &str) -> Option<String> {
    let doc = scraper::Html::parse_fragment(html);
    let sel = scraper::Selector::parse(".gmail_signature").ok()?;
    doc.select(&sel).map(|e| e.html()).find(|h| !normalize(&super::html::to_text(h)).is_empty())
}

#[cfg(test)]
mod signature_tests {
    use super::*;

    #[test]
    fn finds_gmail_signature_block() {
        let html = r##"<div dir="ltr">Bonjour,<div>Voici le rendu.</div><br clear="all"><div><br></div><span class="gmail_signature_prefix">-- </span><br><div dir="ltr" class="gmail_signature" data-smartmail="gmail_signature"><div dir="ltr"><b>Dorian Noirel</b><div>Étudiant Game Design · <font color="#888888">ARTFX</font></div><img src="https://ci3.googleusercontent.com/logo.png" width="96" height="40"></div></div></div>"##;
        let sig = extract_signature(html).unwrap();
        assert!(sig.starts_with("<div") && sig.contains("gmail_signature"), "{sig}");
        assert!(sig.contains("ARTFX") && sig.contains("<img"));
        assert!(!sig.contains("Voici le rendu"));
        assert!(extract_signature("<p>Pas de signature</p>").is_none());
    }
}

#[cfg(test)]
mod shared_tests {
    use super::*;

    #[test]
    fn google_doc_links() {
        let (k, id) = google_doc("https://docs.google.com/document/d/1AbCdEfGhIjKlMnOp/edit?usp=sharing").unwrap();
        assert_eq!((k, id.as_str()), (SharedKind::Doc, "1AbCdEfGhIjKlMnOp"));
        assert_eq!(google_doc("https://docs.google.com/forms/d/e/1FAIpQLSeXyz123456/viewform").map(|x| x.0), Some(SharedKind::Form));
        assert_eq!(google_doc("https://docs.google.com/spreadsheets/d/1ZzYyXxWwVv/edit").map(|x| x.0), Some(SharedKind::Sheet));
        assert!(google_doc("https://drive.google.com/file/d/123/view").is_none());
        assert_eq!(doc_title("Ouvrir", "Document partagé avec vous : « Brief GMTK »", SharedKind::Doc), "Brief GMTK");
    }
}
