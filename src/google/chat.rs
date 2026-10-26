//! Appels à l'API Google Chat v1 (authentification utilisateur) et à l'API People
//! (noms des membres : l'API Chat ne renvoie que des identifiants `users/…`).

use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

use super::Session;

const API: &str = "https://chat.googleapis.com/v1";
const UPLOAD: &str = "https://chat.googleapis.com/upload/v1";
const PEOPLE: &str = "https://people.googleapis.com/v1";
/// Taille maximale d'un fichier téléchargé depuis Chat.
const MAX_MEDIA: u64 = 50 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpaceKind {
    /// Message privé à deux.
    Dm,
    /// Discussion de groupe sans nom.
    Group,
    /// Espace nommé.
    Space,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Space {
    /// « spaces/AAAA… »
    pub name: String,
    pub display_name: String,
    pub kind: SpaceKind,
    /// Fils de discussion (réponses regroupées sous un message).
    pub threaded: bool,
    /// Dernière activité (ms).
    pub last_active: i64,
    pub members: u32,
    /// Message privé avec une application (bot).
    pub bot_dm: bool,
}

impl Space {
    pub fn id(&self) -> &str {
        self.name.trim_start_matches("spaces/")
    }

    /// Lien vers la conversation dans Google Chat (dernier recours).
    pub fn web_link(&self) -> String {
        let kind = if self.kind == SpaceKind::Dm { "dm" } else { "space" };
        format!("https://mail.google.com/chat/u/0/#chat/{kind}/{}", self.id())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatAttachment {
    pub name: String,
    pub mime: String,
    /// Fichier téléversé dans Chat (téléchargeable via l'API media).
    pub resource: Option<String>,
    /// Fichier Google Drive (ouvert dans le navigateur).
    pub drive_id: Option<String>,
    pub download_uri: Option<String>,
}

impl ChatAttachment {
    pub fn is_image(&self) -> bool {
        self.mime.starts_with("image/") && self.resource.is_some() && !self.is_gif()
    }
    /// GIF (animé) : choisi dans Chat (lien public) ou envoyé en fichier.
    pub fn is_gif(&self) -> bool {
        self.mime == "image/gif" && (self.resource.is_some() || self.download_uri.is_some())
    }
    pub fn key(&self) -> String {
        self.resource.clone().or_else(|| self.drive_id.clone()).or_else(|| self.download_uri.clone()).unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChatMessage {
    /// « spaces/X/messages/Y »
    pub name: String,
    pub space: String,
    /// « users/123… »
    pub sender: String,
    pub bot: bool,
    pub created: i64,
    pub edited: bool,
    pub text: String,
    pub thread: String,
    pub thread_reply: bool,
    pub attachments: Vec<ChatAttachment>,
    /// (emoji, nombre)
    pub reactions: Vec<(String, u32)>,
    /// Extrait du message cité, le cas échéant.
    pub quote: Option<String>,
}

/// Section de la barre latérale de Google Chat (dossier personnel ou section par défaut).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Section {
    /// Nom complet (« users/…/sections/… »).
    pub name: String,
    /// Identifiant court (« default-direct-messages », « default-spaces », ou id personnalisé).
    pub id: String,
    pub title: String,
    pub order: i64,
    /// Espaces rangés dans la section (« spaces/… »).
    pub spaces: Vec<String>,
    /// Élément de section de chaque espace (pour le déplacer) : espace → « users/…/items/… ».
    pub items: HashMap<String, String>,
}

#[derive(Clone, Debug)]
pub struct Person {
    /// « users/123… »
    pub id: String,
    pub name: String,
    pub email: String,
    /// Photo de profil (URL publique), absente si c'est l'image par défaut.
    pub photo: Option<String>,
}

pub enum ChatEvent {
    Me(String),
    Spaces(Vec<Space>),
    /// Messages d'un espace. `replace_from` : remplace tout ce qui est plus récent que cette date
    /// (rafraîchissement complet de la fenêtre récente) ; `None` = ajout / mise à jour.
    Messages {
        space: String,
        list: Vec<ChatMessage>,
        replace_from: Option<i64>,
        older: bool,
        more: bool,
    },
    ReadStates(Vec<(String, i64)>),
    /// Membres de chaque conversation demandée : (identifiant, nom affiché s'il est fourni) ;
    /// `None` : lecture impossible (à réessayer plus tard).
    Members(Vec<SpaceMembers>),
    People(Vec<Person>),
    /// Personnes que l'annuaire n'a pas pu lire (erreur, quota) : à redemander plus tard.
    PeopleFailed(Vec<String>),
    /// Message envoyé : remplace l'aperçu local `local`.
    Sent {
        space: String,
        msg: ChatMessage,
        local: String,
    },
    SendFailed {
        space: String,
        local: String,
        error: String,
    },
    /// Rien à faire (action confirmée par Google).
    Noop,
    Updated(ChatMessage),
    Deleted {
        space: String,
        name: String,
    },
    /// Réaction modifiée : le message est rechargé.
    Reacted(ChatMessage),
    Image {
        key: String,
        image: Result<std::sync::Arc<eframe::egui::ColorImage>, String>,
    },
    /// GIF trouvés sur GIPHY (sélecteur de GIF) pour cette recherche (vide : tendances).
    GifResults {
        query: String,
        result: Result<Vec<crate::prefs::GifItem>, String>,
    },
    /// GIF décodé (toutes ses images et leur durée).
    Gif {
        key: String,
        frames: Result<Vec<crate::tabs::messages::gif::Frame>, String>,
    },
    /// Fichier téléchargé, prêt à ouvrir avec l'application par défaut.
    File {
        path: std::path::PathBuf,
    },
    /// Conversation privée trouvée ou créée.
    Dm(Space),
    /// Sections de l'utilisateur (`Err` : indisponibles, ex. autorisation à accorder).
    Sections(Result<Vec<Section>, String>),
    /// Déplacement refusé : message, puis les sections sont relues.
    MoveFailed(String),
    /// Membres d'un groupe (panneau « Gérer les membres ») ; `Err` : lecture impossible.
    Memberships {
        space: String,
        list: Result<Vec<Member>, String>,
    },
    /// Membre ajouté ou retiré (`error` : refusé) : la liste est relue.
    MembersChanged {
        space: String,
        error: Option<String>,
    },
    SpaceRenamed(Space),
    SpaceDeleted(String),
    /// Renommage ou suppression refusés.
    ManageFailed(String),
    Error(String),
}

/// Adhésion d'une personne à un groupe.
#[derive(Clone, Debug, Default)]
pub struct Member {
    /// « spaces/…/members/… » (pour la retirer).
    pub membership: String,
    /// « users/… »
    pub user: String,
    pub name: String,
    /// Gestionnaire de l'espace.
    pub manager: bool,
}

// --- Conversions ----------------------------------------------------------------

pub fn ms(t: &str) -> i64 {
    DateTime::parse_from_rfc3339(t).map(|d| d.timestamp_millis()).unwrap_or(0)
}

pub fn rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339_opts(SecondsFormat::Micros, true)
}

fn parse_space(v: &Value) -> Space {
    let kind = match v["spaceType"].as_str().or(v["type"].as_str()) {
        Some("DIRECT_MESSAGE") | Some("DM") => SpaceKind::Dm,
        Some("GROUP_CHAT") => SpaceKind::Group,
        _ => SpaceKind::Space,
    };
    Space {
        name: v["name"].as_str().unwrap_or_default().to_string(),
        display_name: v["displayName"].as_str().unwrap_or_default().to_string(),
        kind,
        threaded: v["spaceThreadingState"].as_str() == Some("THREADED_MESSAGES"),
        last_active: v["lastActiveTime"].as_str().map(ms).unwrap_or(0),
        members: v["membershipCount"]["joinedDirectHumanUserCount"].as_u64().unwrap_or(0) as u32,
        bot_dm: v["singleUserBotDm"].as_bool().unwrap_or(false),
    }
}

pub fn parse_message(v: &Value) -> ChatMessage {
    let name = v["name"].as_str().unwrap_or_default().to_string();
    let space = name.split("/messages/").next().unwrap_or_default().to_string();
    let text = v["text"].as_str().filter(|t| !t.is_empty()).or(v["argumentText"].as_str().filter(|t| !t.is_empty())).or(v["fallbackText"].as_str()).unwrap_or_default().to_string();
    let attachments = v["attachment"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|x| ChatAttachment {
                    name: x["contentName"].as_str().unwrap_or("fichier").to_string(),
                    mime: x["contentType"].as_str().unwrap_or_default().to_string(),
                    resource: x["attachmentDataRef"]["resourceName"].as_str().map(str::to_string),
                    drive_id: x["driveDataRef"]["driveFileId"].as_str().map(str::to_string),
                    download_uri: x["downloadUri"].as_str().map(str::to_string),
                })
                .collect()
        })
        .unwrap_or_default();
    // GIF choisis dans Chat (bouton GIF) : lien public, affichés comme les pièces jointes.
    let mut attachments: Vec<ChatAttachment> = attachments;
    if let Some(gifs) = v["attachedGifs"].as_array() {
        attachments.extend(gifs.iter().filter_map(|g| g["uri"].as_str()).map(|u| ChatAttachment {
            name: "GIF".into(),
            mime: "image/gif".into(),
            download_uri: Some(u.to_string()),
            ..Default::default()
        }));
    }
    let reactions = v["emojiReactionSummaries"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let e = r["emoji"]["unicode"].as_str().or(r["emoji"]["customEmoji"]["emojiName"].as_str())?;
                    Some((e.to_string(), r["reactionCount"].as_u64().unwrap_or(1) as u32))
                })
                .collect()
        })
        .unwrap_or_default();
    let quote = v["quotedMessageMetadata"]["quotedMessageSnapshot"]["text"].as_str().map(str::to_string);
    ChatMessage {
        name,
        space,
        sender: v["sender"]["name"].as_str().unwrap_or_default().to_string(),
        bot: v["sender"]["type"].as_str() == Some("BOT"),
        created: v["createTime"].as_str().map(ms).unwrap_or(0),
        edited: v["lastUpdateTime"].as_str().is_some_and(|t| !t.is_empty()),
        text,
        thread: v["thread"]["name"].as_str().unwrap_or_default().to_string(),
        thread_reply: v["threadReply"].as_bool().unwrap_or(false),
        attachments,
        reactions,
        quote,
    }
}

// --- Appels -------------------------------------------------------------------------

/// Identifiant Chat de l'utilisateur connecté (« users/<sub> »).
/// `None` si Google ne le fournit pas : il est alors déduit des membres des conversations.
pub fn me(s: &Session) -> Option<String> {
    let token = s.access_token().ok()?;
    let v = s.get("https://oauth2.googleapis.com/tokeninfo", &[("access_token", token.as_str())]).ok()?;
    v["sub"].as_str().or(v["user_id"].as_str()).map(|id| format!("users/{id}"))
}

/// Tous les espaces dont l'utilisateur est membre (une requête par page de 1000).
pub fn spaces(s: &Session) -> Result<Vec<Space>, String> {
    let mut out = Vec::new();
    let mut token = String::new();
    for _ in 0..5 {
        let mut q = vec![("pageSize", "1000")];
        if !token.is_empty() {
            q.push(("pageToken", token.as_str()));
        }
        let v = s.get(&format!("{API}/spaces"), &q)?;
        out.extend(v["spaces"].as_array().into_iter().flatten().map(parse_space));
        match v["nextPageToken"].as_str() {
            Some(t) if !t.is_empty() => token = t.to_string(),
            _ => break,
        }
    }
    out.sort_by_key(|sp| -sp.last_active);
    Ok(out)
}

/// Messages d'un espace, du plus ancien au plus récent.
/// `after` : uniquement ceux créés après (relève) ; `before` : plus anciens (historique).
pub fn messages(s: &Session, space: &str, after: Option<i64>, before: Option<i64>, page: u32) -> Result<(Vec<ChatMessage>, bool), String> {
    let mut filter = Vec::new();
    if let Some(a) = after {
        filter.push(format!("createTime > \"{}\"", rfc3339(a)));
    }
    if let Some(b) = before {
        filter.push(format!("createTime < \"{}\"", rfc3339(b)));
    }
    let filter = filter.join(" AND ");
    let page = page.to_string();
    let mut q = vec![("pageSize", page.as_str()), ("orderBy", "createTime desc")];
    if !filter.is_empty() {
        q.push(("filter", filter.as_str()));
    }
    let v = s.get(&format!("{API}/{space}/messages"), &q)?;
    let mut list: Vec<ChatMessage> = v["messages"].as_array().into_iter().flatten().map(parse_message).collect();
    let more = v["nextPageToken"].as_str().is_some_and(|t| !t.is_empty());
    list.reverse();
    Ok((list, more))
}

/// Date de dernière lecture de plusieurs espaces (une requête par espace, à la suite).
pub fn read_states(s: &Session, spaces: &[String]) -> Result<Vec<(String, i64)>, String> {
    let mut out = Vec::new();
    for sp in spaces {
        let id = sp.trim_start_matches("spaces/");
        match s.get(&format!("{API}/users/me/spaces/{id}/spaceReadState"), &[]) {
            Ok(v) => out.push((sp.clone(), v["lastReadTime"].as_str().map(ms).unwrap_or(0))),
            Err(e) if out.is_empty() && is_setup_error(&e) => return Err(e),
            Err(_) => {}
        }
    }
    Ok(out)
}

pub fn mark_read(s: &Session, space: &str, at: i64) -> Result<(), String> {
    let id = space.trim_start_matches("spaces/");
    s.send("PATCH", &format!("{API}/users/me/spaces/{id}/spaceReadState"), &[("updateMask", "lastReadTime")], Some(json!({ "lastReadTime": rfc3339(at) })))?;
    Ok(())
}

/// Membres (humains) de plusieurs espaces.
/// Membres d'une conversation : (espace, `Some` liste de (identifiant, nom affiché) ou `None` si
/// la lecture a échoué).
pub type SpaceMembers = (String, Option<Vec<(String, String)>>);

pub fn members(s: &Session, spaces: &[String]) -> Result<Vec<SpaceMembers>, String> {
    let mut out = Vec::new();
    for sp in spaces {
        let Ok(v) = s.get(&format!("{API}/{sp}/members"), &[("pageSize", "100")]) else {
            out.push((sp.clone(), None));
            continue;
        };
        let users = v["memberships"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|m| {
                let id = m["member"]["name"].as_str()?;
                Some((id.to_string(), m["member"]["displayName"].as_str().unwrap_or_default().to_string()))
            })
            .collect();
        out.push((sp.clone(), Some(users)));
    }
    Ok(out)
}

/// Noms et adresses (annuaire de l'organisation) à partir des identifiants « users/… ».
pub fn people(s: &Session, ids: &[String]) -> Result<Vec<Person>, String> {
    let mut out = Vec::new();
    for chunk in ids.chunks(50) {
        let names: Vec<String> = chunk.iter().map(|id| format!("people/{}", id.trim_start_matches("users/"))).collect();
        let mut q: Vec<(&str, &str)> = names.iter().map(|n| ("resourceNames", n.as_str())).collect();
        q.push(("personFields", "names,emailAddresses,photos"));
        q.push(("sources", "READ_SOURCE_TYPE_PROFILE"));
        q.push(("sources", "READ_SOURCE_TYPE_DOMAIN_CONTACT"));
        q.push(("sources", "READ_SOURCE_TYPE_CONTACT"));
        let v = s.get(&format!("{PEOPLE}/people:batchGet"), &q)?;
        for r in v["responses"].as_array().into_iter().flatten() {
            let id = r["requestedResourceName"].as_str().unwrap_or_default().replace("people/", "users/");
            let p = &r["person"];
            let name = p["names"][0]["displayName"].as_str().unwrap_or_default().to_string();
            let email = p["emailAddresses"][0]["value"].as_str().unwrap_or_default().to_string();
            let photo = p["photos"].as_array().and_then(|a| a.iter().find(|x| !x["default"].as_bool().unwrap_or(false))).and_then(|x| x["url"].as_str()).map(|u| photo_size(u, 96));
            // Introuvable : on le note quand même pour ne pas redemander à chaque fois.
            out.push(Person { id, name, email, photo });
        }
    }
    Ok(out)
}

pub fn send(s: &Session, space: &str, text: &str, thread: Option<&str>, attachments: Vec<Value>) -> Result<ChatMessage, String> {
    let mut body = json!({ "text": text });
    if let Some(t) = thread {
        body["thread"] = json!({ "name": t });
    }
    if !attachments.is_empty() {
        body["attachment"] = Value::Array(attachments);
    }
    let url = format!("{API}/{space}/messages");
    let v = if thread.is_some() { s.send("POST", &url, &[("messageReplyOption", "REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD")], Some(body))? } else { s.post(&url, body)? };
    Ok(parse_message(&v))
}

/// Téléverse un fichier dans l'espace ; renvoie la pièce jointe à joindre au message.
pub fn upload(s: &Session, space: &str, filename: &str, mime: &str, data: &[u8]) -> Result<Value, String> {
    let boundary = "worktable_chat_upload";
    let mut body = Vec::with_capacity(data.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
    body.extend_from_slice(json!({ "filename": filename }).to_string().as_bytes());
    body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: {mime}\r\n\r\n").as_bytes());
    body.extend_from_slice(data);
    body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());
    let v = s.post_bytes(&format!("{UPLOAD}/{space}/attachments:upload?uploadType=multipart"), &format!("multipart/related; boundary={boundary}"), &body)?;
    Ok(json!({ "attachmentDataRef": v["attachmentDataRef"].clone() }))
}

pub fn edit(s: &Session, name: &str, text: &str) -> Result<ChatMessage, String> {
    let v = s.send("PATCH", &format!("{API}/{name}"), &[("updateMask", "text")], Some(json!({ "text": text })))?;
    Ok(parse_message(&v))
}

pub fn delete(s: &Session, name: &str) -> Result<(), String> {
    s.send("DELETE", &format!("{API}/{name}"), &[], None)?;
    Ok(())
}

pub fn get_message(s: &Session, name: &str) -> Result<ChatMessage, String> {
    Ok(parse_message(&s.get(&format!("{API}/{name}"), &[])?))
}

/// Ajoute (`add`) ou retire une réaction en un minimum d'appels.
/// Renvoie `true` si la réaction est finalement présente.
pub fn set_reaction(s: &Session, name: &str, emoji: &str, add: bool, me: Option<&str>) -> Result<bool, String> {
    if add {
        match s.post(&format!("{API}/{name}/reactions"), json!({ "emoji": { "unicode": emoji } })) {
            Ok(_) => return Ok(true),
            // Déjà mise (depuis un autre appareil) : c'est un retrait.
            Err(e) if e.contains("409") || e.to_uppercase().contains("ALREADY") => {}
            Err(e) => return Err(e),
        }
    }
    let me = me.ok_or(t!("identifiant inconnu", "unknown identifier"))?;
    let filter = format!("emoji.unicode = \"{emoji}\" AND user.name = \"{me}\"");
    let v = s.get(&format!("{API}/{name}/reactions"), &[("filter", filter.as_str())])?;
    for r in v["reactions"].as_array().into_iter().flatten() {
        if let Some(rn) = r["name"].as_str() {
            s.send("DELETE", &format!("{API}/{rn}"), &[], None)?;
        }
    }
    Ok(false)
}

/// Conversation privée avec une adresse : retrouvée si elle existe, sinon créée.
pub fn open_dm(s: &Session, email: &str) -> Result<Space, String> {
    let user = format!("users/{email}");
    match s.get(&format!("{API}/spaces:findDirectMessage"), &[("name", user.as_str())]) {
        Ok(v) => Ok(parse_space(&v)),
        Err(e) if e.contains("404") => {
            let body = json!({
                "space": { "spaceType": "DIRECT_MESSAGE" },
                "memberships": [{ "member": { "name": user, "type": "HUMAN" } }],
            });
            Ok(parse_space(&s.post(&format!("{API}/spaces:setup"), body)?))
        }
        Err(e) => Err(e),
    }
}

/// Sections (dossiers) de l'utilisateur dans Google Chat, avec leurs espaces, dans son ordre.
pub fn sections(s: &Session) -> Result<Vec<Section>, String> {
    let mut out: Vec<Section> = Vec::new();
    let mut token = String::new();
    for _ in 0..5 {
        let mut q = vec![("pageSize", "100")];
        if !token.is_empty() {
            q.push(("pageToken", token.as_str()));
        }
        let v = s.get(&format!("{API}/users/me/sections"), &q)?;
        for x in v["sections"].as_array().into_iter().flatten() {
            let name = x["name"].as_str().unwrap_or_default();
            let id = name.rsplit('/').next().unwrap_or_default().to_string();
            let custom = x["type"].as_str() == Some("CUSTOM_SECTION") || !id.starts_with("default-");
            let title = match (custom, id.as_str()) {
                (true, _) => x["displayName"].as_str().unwrap_or("Section").to_string(),
                (false, "default-direct-messages") => t!("Messages privés", "Direct messages").into(),
                (false, "default-spaces") => t!("Espaces", "Spaces").into(),
                (false, "default-apps") => t!("Applications", "Apps").into(),
                (false, other) => x["displayName"].as_str().filter(|d| !d.is_empty()).unwrap_or(other).to_string(),
            };
            out.push(Section { name: name.to_string(), id, title, order: x["sortOrder"].as_i64().unwrap_or(i64::MAX), spaces: Vec::new(), items: HashMap::new() });
        }
        match v["nextPageToken"].as_str() {
            Some(t) if !t.is_empty() => token = t.to_string(),
            _ => break,
        }
    }
    // Contenu de chaque section (nom exact renvoyé par Google).
    for sec in out.iter_mut() {
        let mut token = String::new();
        for _ in 0..20 {
            let mut q = vec![("pageSize", "100")];
            if !token.is_empty() {
                q.push(("pageToken", token.as_str()));
            }
            let v = s.get(&format!("{API}/{}/items", sec.name), &q)?;
            for it in v["sectionItems"].as_array().into_iter().flatten() {
                let space = it["space"].as_str().or(it["space"]["name"].as_str());
                if let Some(space) = space {
                    sec.spaces.push(space.to_string());
                    if let Some(item) = it["name"].as_str() {
                        sec.items.insert(space.to_string(), item.to_string());
                    }
                }
            }
            match v["nextPageToken"].as_str() {
                Some(t) if !t.is_empty() => token = t.to_string(),
                _ => break,
            }
        }
    }
    out.sort_by_key(|x| x.order);
    Ok(out)
}

pub fn media(s: &Session, resource: &str) -> Result<Vec<u8>, String> {
    s.get_bytes(&format!("{API}/media/{resource}"), &[("alt", "media")], MAX_MEDIA)
}

/// Déplace une conversation dans une autre section (`target` : nom complet de la section).
pub fn move_to_section(s: &Session, space: &str, item: Option<&str>, target: &str) -> Result<(), String> {
    let item = match item {
        Some(i) => i.to_string(),
        None => {
            let filter = format!("space = {space}");
            let v = s.get(&format!("{API}/users/me/sections/-/items"), &[("filter", filter.as_str())])?;
            v["sectionItems"][0]["name"].as_str().map(str::to_string).ok_or(t!("Conversation introuvable dans tes sections", "Conversation not found in your sections"))?
        }
    };
    s.post(&format!("{API}/{item}:move"), json!({ "targetSection": target }))?;
    Ok(())
}

/// Crée un espace nommé, ou une discussion de groupe si `name` est vide (2 membres minimum).
pub fn create_space(s: &Session, name: &str, emails: &[String]) -> Result<Space, String> {
    let members: Vec<Value> = emails.iter().map(|e| json!({ "member": { "name": format!("users/{e}"), "type": "HUMAN" } })).collect();
    let space = if name.trim().is_empty() { json!({ "spaceType": "GROUP_CHAT" }) } else { json!({ "spaceType": "SPACE", "displayName": name.trim() }) };
    Ok(parse_space(&s.post(&format!("{API}/spaces:setup"), json!({ "space": space, "memberships": members }))?))
}

/// Membres d'un groupe, avec leur adhésion (pour les retirer) et leur rôle.
pub fn memberships(s: &Session, space: &str) -> Result<Vec<Member>, String> {
    let v = s.get(&format!("{API}/{space}/members"), &[("pageSize", "200")])?;
    Ok(v["memberships"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["member"]["type"].as_str() != Some("BOT"))
        .filter_map(|m| {
            Some(Member {
                membership: m["name"].as_str()?.to_string(),
                user: m["member"]["name"].as_str()?.to_string(),
                name: m["member"]["displayName"].as_str().unwrap_or_default().to_string(),
                manager: m["role"].as_str() == Some("ROLE_MANAGER"),
            })
        })
        .collect())
}

/// Ajoute une personne (adresse) au groupe.
pub fn add_member(s: &Session, space: &str, email: &str) -> Result<(), String> {
    s.post(&format!("{API}/{space}/members"), json!({ "member": { "name": format!("users/{email}"), "type": "HUMAN" } }))?;
    Ok(())
}

/// Retire une personne du groupe (`membership` : « spaces/…/members/… »).
pub fn remove_member(s: &Session, membership: &str) -> Result<(), String> {
    s.send("DELETE", &format!("{API}/{membership}"), &[], None)?;
    Ok(())
}

/// Renomme un espace.
pub fn rename_space(s: &Session, space: &str, name: &str) -> Result<Space, String> {
    let v = s.send("PATCH", &format!("{API}/{space}"), &[("updateMask", "displayName")], Some(json!({ "displayName": name.trim() })))?;
    Ok(parse_space(&v))
}

/// Supprime un espace, ses messages et ses fichiers, pour tous ses membres.
pub fn delete_space(s: &Session, space: &str) -> Result<(), String> {
    s.send("DELETE", &format!("{API}/{space}"), &[], None)?;
    Ok(())
}

/// Demande une taille donnée à l'hébergement des photos Google (« …=s100 » → « …=s96 »).
fn photo_size(url: &str, px: u32) -> String {
    match url.rfind('=') {
        Some(i) if url[i + 1..].starts_with('s') => format!("{}=s{px}-c", &url[..i]),
        _ => url.to_string(),
    }
}

/// Télécharge une photo de profil (adresse publique, sans jeton).
pub fn fetch_photo(agent: &ureq::Agent, url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let r = agent.get(url).call().map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    r.into_reader().take(2 * 1024 * 1024).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

/// Erreurs de configuration (autorisation manquante, API désactivée, application Chat non configurée).
pub fn is_setup_error(e: &str) -> bool {
    let l = e.to_lowercase();
    l.contains("insufficient") || l.contains("scope") || l.contains("has not been used") || l.contains("disabled") || l.contains("chat app not found")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_message() {
        let v = json!({
            "name": "spaces/AAA/messages/BBB",
            "sender": { "name": "users/123", "type": "HUMAN" },
            "createTime": "2026-10-20T10:00:00.123456Z",
            "text": "Salut *toi*",
            "thread": { "name": "spaces/AAA/threads/T" },
            "threadReply": true,
            "attachment": [{ "contentName": "a.png", "contentType": "image/png", "attachmentDataRef": { "resourceName": "R" } }],
            "emojiReactionSummaries": [{ "emoji": { "unicode": "👍" }, "reactionCount": 2 }]
        });
        let m = parse_message(&v);
        assert_eq!(m.space, "spaces/AAA");
        assert_eq!(m.sender, "users/123");
        assert!(m.thread_reply && m.attachments[0].is_image());
        assert_eq!(m.reactions, vec![("👍".to_string(), 2)]);
        assert_eq!(rfc3339(m.created), "2026-10-20T10:00:00.123000Z");
    }

    #[test]
    fn parses_attached_gifs() {
        let v = json!({
            "name": "spaces/AAA/messages/C",
            "attachedGifs": [{ "uri": "https://media.tenor.com/x/cat.gif" }],
            "attachment": [{ "contentName": "b.gif", "contentType": "image/gif", "attachmentDataRef": { "resourceName": "G" } }]
        });
        let m = parse_message(&v);
        assert_eq!(m.attachments.len(), 2);
        assert!(m.attachments.iter().all(|a| a.is_gif() && !a.is_image()));
        assert_eq!(m.attachments[1].key(), "https://media.tenor.com/x/cat.gif");
    }

    #[test]
    fn web_links() {
        let sp = parse_space(&json!({ "name": "spaces/XYZ", "spaceType": "DIRECT_MESSAGE" }));
        assert_eq!(sp.web_link(), "https://mail.google.com/chat/u/0/#chat/dm/XYZ");
    }
}
