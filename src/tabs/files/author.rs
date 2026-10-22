//! Fiche « Auteur » d'un fichier Drive (panneau de détail) : qui l'a créé, et tout ce que
//! WorkTable sait déjà de cette personne — annuaire, derniers mails, conversation Google Chat,
//! rendez-vous en commun, ses autres fichiers.

use chrono::Local;
use eframe::egui::{self, Align2, CornerRadius, FontId, Pos2, Rect, RichText, Sense, vec2};

use crate::google::calendar::CalEvent;
use crate::google::chat::SpaceKind;
use crate::google::drive::DriveFile;
use crate::tabs::calendar::CalendarState;
use crate::tabs::mail::MailState;
use crate::tabs::mail::contacts::parse_mailbox;
use crate::tabs::mail::view::{one_line, short_date};
use crate::tabs::messages::ChatState;
use crate::theme;

/// Ce que les autres onglets savent déjà (lecture seule).
pub struct Links<'a> {
    pub mail: &'a MailState,
    pub chat: &'a ChatState,
    pub cal: &'a CalendarState,
}

/// Action demandée depuis la fiche, à traiter par l'application (changement d'onglet).
pub enum DriveAction {
    OpenMail(String),
    /// Mails reçus de cette adresse.
    MailsFrom(String),
    WriteMail(String),
    /// Conversation privée Google Chat (ouverte ou créée) avec cette adresse.
    ChatWith(String),
    OpenSpace(String),
    OpenEvent(CalEvent),
}

/// Ce qui est connu d'une personne, d'après son adresse.
pub struct Known {
    pub name: String,
    pub email: String,
    /// Annuaire : mails échangés, date du dernier.
    pub contact: Option<(u32, i64)>,
    /// Derniers mails reçus d'elle : (identifiant, objet, date).
    pub mails: Vec<(String, String, i64)>,
    /// Conversation privée : (espace, dernier message, date).
    pub dm: Option<(String, String, i64)>,
    /// Rendez-vous en commun (prochains d'abord, sinon le plus récent).
    pub events: Vec<CalEvent>,
}

/// Rassemble ce que WorkTable sait d'une adresse.
pub fn known(email: &str, fallback_name: &str, links: &Links) -> Known {
    let email = email.trim().to_lowercase();
    let contact = links.mail.contacts.get(&email);
    // Derniers mails reçus de cette adresse (un par fil).
    let mut seen = std::collections::HashSet::new();
    let mut mails: Vec<(String, String, i64)> = links
        .mail
        .messages
        .iter()
        .filter(|m| parse_mailbox(&m.from).is_some_and(|(_, a)| a.eq_ignore_ascii_case(&email)))
        .filter(|m| seen.insert(m.thread_id.clone()))
        .map(|m| (m.id.clone(), m.subject.clone(), m.date))
        .collect();
    mails.sort_by_key(|m| std::cmp::Reverse(m.2));
    mails.truncate(3);
    // Google Chat : la personne (par son adresse), puis le message privé avec elle.
    let person = links.chat.people.values().find(|p| p.email.eq_ignore_ascii_case(&email));
    let dm = person.and_then(|p| {
        let sp = links.chat.spaces.iter().find(|s| s.kind == SpaceKind::Dm && links.chat.members.get(&s.name).is_some_and(|m| m.contains(&p.id)))?;
        let (text, at) = links.chat.msgs.get(&sp.name).and_then(|l| l.last()).map(|m| (m.text.replace(['\n', '\r'], " "), m.created)).unwrap_or((String::new(), sp.last_active));
        Some((sp.name.clone(), text, at))
    });
    // Agenda : rendez-vous où elle est invitée.
    let now = Local::now().timestamp_millis();
    let mut ids = std::collections::HashSet::new();
    let mut all: Vec<&CalEvent> = links
        .cal
        .events
        .values()
        .flatten()
        .filter(|e| e.attendees.iter().any(|a| a.eq_ignore_ascii_case(&email)))
        .filter(|e| ids.insert((e.calendar.clone(), e.id.clone(), e.start)))
        .collect();
    all.sort_by_key(|e| e.start);
    let mut events: Vec<CalEvent> = all.iter().filter(|e| e.end > now).take(2).map(|e| (*e).clone()).collect();
    if events.is_empty()
        && let Some(e) = all.last()
    {
        events.push((*e).clone());
    }
    let name = [contact.map(|c| c.display_name().to_string()), person.map(|p| p.name.clone()), Some(fallback_name.to_string())]
        .into_iter()
        .flatten()
        .find(|n| !n.trim().is_empty() && !n.eq_ignore_ascii_case(&email))
        .unwrap_or_else(|| email.clone());
    Known { name, email, contact: contact.map(|c| (c.count, c.last_seen)), mails, dm, events }
}

/// Fiche de l'auteur d'un fichier. `own_files` : ouvrir la liste de ses fichiers Drive.
pub fn card(ui: &mut egui::Ui, f: &DriveFile, links: &Links, out: &mut Option<DriveAction>, own_files: &mut bool) {
    // Drives partagés : pas de propriétaire, l'auteur affiché est la dernière personne à l'avoir modifié.
    let shared_drive = f.owner_email.is_empty() && f.owner.is_empty();
    let (name, email) = if shared_drive { (&f.modified_by, &f.modified_by_email) } else { (&f.owner, &f.owner_email) };
    if name.is_empty() && email.is_empty() {
        return;
    }
    let k = known(email, name, links);
    ui.add_space(14.0);
    ui.separator();
    ui.add_space(8.0);
    ui.label(RichText::new(t!("AUTEUR", "AUTHOR")).small().strong().color(theme::TEXT_MUTED()));
    ui.add_space(6.0);
    // En-tête : initiale, nom, adresse.
    let head = ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 17.0, theme::SURFACE_HI());
        let initial: String = k.name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
        ui.painter().text(r.center(), Align2::CENTER_CENTER, initial, theme::title_font(15.0), theme::TEXT());
        ui.vertical(|ui| {
            ui.add(egui::Label::new(RichText::new(&k.name).strong().color(theme::TEXT())).truncate());
            if !k.email.is_empty() {
                ui.add(egui::Label::new(RichText::new(&k.email).small().color(theme::TEXT_MUTED())).truncate().selectable(true));
            }
        });
    });
    if !k.email.is_empty() {
        ui.interact(head.response.rect, ui.scope_id().with("wt_author_head"), Sense::click())
            .on_hover_text(t!("Clic droit : fiche contact", "Right-click: contact card"))
            .context_menu(|ui| crate::people::menu_item(ui, &k.email, &k.name));
    }
    if shared_drive {
        ui.label(RichText::new(t!("Drive partagé : dernière personne à l'avoir modifié", "Shared drive: last person who modified it")).small().color(theme::TEXT_FAINT()));
    } else if !f.modified_by.is_empty() && !f.modified_by_email.eq_ignore_ascii_case(&k.email) {
        ui.label(RichText::new(tf!("Dernière modification par {}", "Last modified by {}", f.modified_by)).small().color(theme::TEXT_FAINT()));
    }
    if k.email.is_empty() {
        return;
    }
    ui.add_space(6.0);
    let line = match k.contact {
        Some((n, last)) if last > 0 => tf!("Annuaire · {n} mail(s) échangé(s), dernier le {}", "Directory · {n} mail(s) exchanged, last on {}", short_date(last)),
        Some((n, _)) => tf!("Annuaire · {n} mail(s) échangé(s)", "Directory · {n} mail(s) exchanged"),
        None => t!("Pas encore dans l'annuaire", "Not in the directory yet").to_string(),
    };
    ui.label(RichText::new(line).small().color(theme::TEXT_MUTED()));

    // Derniers mails.
    if !k.mails.is_empty() {
        section(ui, t!("Ses derniers mails", "Their latest mails"));
        for (id, subject, date) in &k.mails {
            let subject = if subject.is_empty() { t!("(sans objet)", "(no subject)") } else { subject };
            if link_row(ui, "✉", subject, &short_date(*date)).on_hover_text(t!("Ouvrir le mail", "Open the mail")).clicked() {
                *out = Some(DriveAction::OpenMail(id.clone()));
            }
        }
    }
    // Conversation Google Chat.
    if let Some((space, text, at)) = &k.dm {
        section(ui, "Google Chat");
        let text = if text.is_empty() { t!("Conversation privée", "Direct conversation") } else { text };
        if link_row(ui, "💬", text, &short_date(*at)).on_hover_text(t!("Ouvrir la conversation", "Open the conversation")).clicked() {
            *out = Some(DriveAction::OpenSpace(space.clone()));
        }
    }
    // Rendez-vous en commun.
    if !k.events.is_empty() {
        section(ui, t!("Rendez-vous en commun", "Shared events"));
        for e in &k.events {
            let title = if e.title.is_empty() { t!("(sans titre)", "(untitled)") } else { &e.title };
            let when = format!("{} {}", short_date(e.start), if e.all_day { String::new() } else { crate::tabs::calendar::hhmm(e.start) });
            if link_row(ui, "📅", title, when.trim()).on_hover_text(t!("Ouvrir dans le calendrier", "Open in the calendar")).clicked() {
                *out = Some(DriveAction::OpenEvent(e.clone()));
            }
        }
    }
    // Actions.
    ui.add_space(10.0);
    let w = ui.available_width();
    let half = (w - ui.spacing().item_spacing.x) / 2.0;
    if ui
        .add_sized([w, 28.0], egui::Button::new(t!("👤  Fiche contact", "👤  Contact card")))
        .on_hover_text(t!(
            "Tout ce qui est relié à cette personne : mails, conversations, fichiers, rendez-vous",
            "Everything linked to this person: mails, conversations, files, events"
        ))
        .clicked()
    {
        crate::people::open(ui.ctx(), &k.email, &k.name);
    }
    ui.horizontal(|ui| {
        if ui.add_sized([half, 28.0], egui::Button::new(t!("✉  Écrire un mail", "✉  Write a mail"))).clicked() {
            *out = Some(DriveAction::WriteMail(k.email.clone()));
        }
        if ui.add_sized([half, 28.0], egui::Button::new(t!("Ses mails", "Their mails"))).clicked() {
            *out = Some(DriveAction::MailsFrom(k.email.clone()));
        }
    });
    ui.horizontal(|ui| {
        let chat = if k.dm.is_some() { t!("💬  Ouvrir le Chat", "💬  Open the Chat") } else { t!("💬  Écrire sur Chat", "💬  Write on Chat") };
        if ui.add_sized([half, 28.0], egui::Button::new(chat)).clicked() {
            *out = Some(match &k.dm {
                Some((space, _, _)) => DriveAction::OpenSpace(space.clone()),
                None => DriveAction::ChatWith(k.email.clone()),
            });
        }
        // « Ses fichiers » : fichiers dont la personne est propriétaire (pas dans un Drive partagé).
        if !shared_drive
            && ui
                .add_sized([half, 28.0], egui::Button::new(t!("📁  Ses fichiers", "📁  Their files")))
                .on_hover_text(t!("Fichiers Drive dont cette personne est propriétaire", "Drive files owned by this person"))
                .clicked()
        {
            *own_files = true;
        }
    });
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(8.0);
    ui.label(RichText::new(title.to_uppercase()).small().color(theme::TEXT_FAINT()));
    ui.add_space(2.0);
}

/// Ligne cliquable : icône, texte sur une ligne, date à droite.
fn link_row(ui: &mut egui::Ui, icon: &str, text: &str, when: &str) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), theme::overlay(7));
    }
    crate::emoji_img::paint(p, Rect::from_center_size(Pos2::new(rect.left() + 12.0, rect.center().y), vec2(13.0, 13.0)), icon, theme::TEXT_MUTED());
    let dg = one_line(p, when, FontId::proportional(10.5), theme::TEXT_FAINT(), 90.0);
    let dw = dg.size().x;
    p.galley(Pos2::new(rect.right() - 6.0 - dw, rect.center().y - dg.size().y / 2.0), dg, theme::TEXT_FAINT());
    let g = one_line(p, text, FontId::proportional(12.5), theme::TEXT(), rect.width() - 40.0 - dw);
    p.galley(Pos2::new(rect.left() + 26.0, rect.center().y - g.size().y / 2.0), g, theme::TEXT());
    resp
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::Summary;
    use crate::google::chat::{ChatMessage, Person, Space};

    #[test]
    fn connects_author_with_mail_chat_and_calendar() {
        let email = "lea.auteur@example.invalid";
        let mut mail = MailState::default();
        for (i, (thread, date)) in [("t1", 10), ("t1", 20), ("t2", 30)].iter().enumerate() {
            mail.messages.push(Summary {
                id: format!("m{i}"),
                thread_id: thread.to_string(),
                from: format!("Léa Martin <{}>", email.to_uppercase()),
                subject: format!("Sujet {i}"),
                date: *date,
                ..Default::default()
            });
        }
        let mut chat = ChatState::default();
        chat.people.insert("users/7".into(), Person { id: "users/7".into(), name: "Léa M.".into(), email: email.into(), photo: None });
        chat.spaces.push(Space { name: "spaces/dm".into(), display_name: String::new(), kind: SpaceKind::Dm, threaded: false, last_active: 5, members: 2, bot_dm: false });
        chat.members.insert("spaces/dm".into(), vec!["users/me".into(), "users/7".into()]);
        chat.msgs.insert("spaces/dm".into(), vec![ChatMessage { text: "Tu as vu le doc ?".into(), created: 40, ..Default::default() }]);
        let mut cal = CalendarState::default();
        let soon = Local::now().timestamp_millis() + 3_600_000;
        cal.events.insert("k".into(), vec![CalEvent { id: "e1".into(), title: "Revue".into(), start: soon, end: soon + 1, attendees: vec![email.into()], ..Default::default() }]);
        let k = known(email, "", &Links { mail: &mail, chat: &chat, cal: &cal });
        assert_eq!(k.name, "Léa M.", "nom repris de Google Chat (pas dans l'annuaire)");
        assert_eq!(k.mails.len(), 2, "un mail par fil");
        assert_eq!(k.mails[0].0, "m2", "le plus récent d'abord");
        assert_eq!(k.dm.as_ref().map(|d| d.1.as_str()), Some("Tu as vu le doc ?"));
        assert_eq!(k.events.len(), 1);
        // Adresse inconnue : rien de relié, le nom retombe sur l'adresse.
        let none = known("inconnu@example.invalid", "", &Links { mail: &mail, chat: &chat, cal: &cal });
        assert!(none.mails.is_empty() && none.dm.is_none() && none.events.is_empty());
        assert_eq!(none.name, "inconnu@example.invalid");
    }
}
