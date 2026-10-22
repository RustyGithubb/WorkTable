//! Dessin de l'onglet Mail : dossiers + catégories + favoris | liste | lecture / rédaction.

use std::time::Duration;

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::{self, Align, Color32, CornerRadius, FontId, Layout, Painter, Pos2, Rect, RichText, Sense, Stroke, Vec2, vec2};
use eframe::epaint::text::{LayoutJob, TextWrapping};

use super::attachments::{self, Content};
use super::{DocTarget, INBOX, MailState, composer, contacts, directory, groups, inline};
use crate::google::compose::Draft;
use crate::google::html::{Align as HAlign, BodyBlock, Dim, Para};
use crate::google::{Account, Attachment, FullMessage, Label, Summary};
use crate::liquid::{self, Liquid};
use crate::prefs::GroupColor;
use crate::prefs::Prefs;
use crate::{tabs, theme, widgets};

const FOLDERS: [&str; 4] = [INBOX, "SENT", "SPAM", "TRASH"];

/// Ce que l'utilisateur a demandé pendant la frame ; appliqué après le dessin.
enum Action {
    Folder(String),
    Open(String),
    Star(Summary),
    Pin(Summary),
    /// (identifiant, true = mettre en spam / false = ce n'est pas un spam)
    Spam(String, bool),
    /// (identifiant, true = mettre à la corbeille / false = la restaurer)
    Trash(String, bool),
    Move(String, Label),
    StartCategory,
    CreateCategory(String),
    CancelCategory,
    Reply,
    ReplyAll,
    Forward,
    ToggleThread(String),
    Compose,
    Send,
    CancelDraft,
    Refresh,
    Attachment(Attachment),
    AttachmentExternal(Attachment),
    ClosePreview,
    Directory,
    Dir(directory::DirAction),
    FromFilter(String),
    ContactFavorite(String),
    WriteTo(String),
    ChatTo(String),
}

pub fn show(ui: &mut egui::Ui, mail: &mut MailState, acc: &mut Account, prefs: &mut Prefs) {
    let unread = mail.inbox_unread;
    widgets::page_header_with(ui, t!("Messagerie", "Mailbox"), "Gmail", |ui| {
        if unread > 0 {
            let badge = if crate::i18n::en() { format!("{unread} unread") } else { format!("{unread} non lu{}", if unread > 1 { "s" } else { "" }) };
            widgets::header_badge(ui, &badge, true);
        }
    });

    if acc.session().is_none() {
        connect_screen(ui, acc);
        return;
    }

    let mut actions = Vec::new();
    let email = acc.session().map(|s| s.email.clone()).unwrap_or_default();
    let list_full = ((ui.available_width() - 240.0) * 0.38).clamp(280.0, 420.0);
    // Liste des mails masquable (animée) pour agrandir la lecture.
    let list_shown = mail.directory || !prefs.mail_list_hidden;
    let list_w = list_full * liquid::animate(ui.ctx(), "mail_list_width", list_shown);
    let has_list = list_w >= 40.0;
    let fixed: Vec<f32> = if has_list { vec![240.0, list_w] } else { vec![240.0] };
    let mut toggle_list = false;
    let mut toggle_cats = false;
    mail.groups = prefs.groups.clone();
    mail.own = email.clone();
    mail.groups_open = !prefs.groups_collapsed;
    let cats_open = !prefs.categories_collapsed;
    let list_hidden = prefs.mail_list_hidden;

    let mut dir_actions = Vec::new();
    let acc_ref: &Account = acc;
    widgets::pane_columns(ui, &fixed, |i, ui| match (i, mail.directory, has_list) {
        (0, _, _) => sidebar(ui, mail, cats_open, &mut toggle_cats, &mut actions),
        (1, true, _) => directory::list(ui, mail, &mut dir_actions),
        (_, true, _) => directory::detail(ui, mail, &mut dir_actions),
        (1, false, true) => {
            if list_w > 160.0 {
                message_list(ui, mail, &mut actions)
            }
        }
        _ => reading_pane(ui, mail, acc_ref, &email, list_hidden, &mut toggle_list, &mut actions),
    });
    if toggle_list {
        prefs.mail_list_hidden = !prefs.mail_list_hidden;
    }
    if toggle_cats {
        prefs.categories_collapsed = !prefs.categories_collapsed;
    }
    if std::mem::take(&mut mail.toggle_groups) {
        prefs.groups_collapsed = !prefs.groups_collapsed;
    }
    for e in std::mem::take(&mut mail.group_edits) {
        groups::apply(&mut prefs.groups, e);
    }
    actions.extend(dir_actions.into_iter().map(Action::Dir));

    preview_modal(ui, mail, &mut actions);
    toast(ui, mail);

    // Suppr : mail ouvert à la corbeille (hors saisie de texte).
    if !ui.ctx().egui_wants_keyboard_input()
        && ui.input(|i| i.key_pressed(egui::Key::Delete))
        && let Some(s) = mail.open.as_ref().map(|m| &m.summary)
    {
        let in_trash = s.labels.iter().any(|l| l == "TRASH");
        actions.push(Action::Trash(s.id.clone(), !in_trash));
    }

    for a in actions {
        match a {
            Action::Folder(id) => {
                mail.directory = false;
                mail.select_folder(acc, &id);
            }
            Action::Directory => mail.directory = true,
            Action::FromFilter(e) => mail.show_from(acc, &e),
            Action::ContactFavorite(e) => mail.contacts.toggle_favorite(&e),
            Action::WriteTo(m) => mail.write_to(m),
            Action::ChatTo(e) => mail.chat_to = Some(e),
            Action::Dir(d) => match d {
                directory::DirAction::Select(e) => mail.select_contact(acc, &e),
                directory::DirAction::OpenMail(e, id) => {
                    mail.show_from(acc, &e);
                    mail.open_message(acc, &id);
                }
                directory::DirAction::Favorite(e) => mail.contacts.toggle_favorite(&e),
                directory::DirAction::ShowMails(e) => mail.show_from(acc, &e),
                directory::DirAction::ChatTo(e) => mail.chat_to = Some(e),
                directory::DirAction::WriteTo(e) => {
                    let mb = mail.contacts.get(&e).map(|c| c.mailbox()).unwrap_or(e);
                    mail.write_to(mb);
                }
                directory::DirAction::Remove(e) => {
                    mail.contacts.remove(&e);
                    mail.dir_selected = None;
                }
            },
            Action::Open(id) => mail.open_message(acc, &id),
            Action::Star(m) => mail.toggle_star(acc, &m),
            Action::Pin(m) => mail.toggle_pin(&m),
            Action::Spam(id, spam) => mail.mark_spam(acc, &id, spam),
            Action::Trash(id, trash) => mail.trash(acc, &id, trash),
            Action::Move(id, l) => mail.move_to(acc, &id, &l),
            Action::StartCategory => mail.new_category = Some(String::new()),
            Action::CreateCategory(n) => {
                mail.new_category = None;
                mail.create_category(acc, n);
            }
            Action::CancelCategory => mail.new_category = None,
            Action::Reply => {
                mail.draft = mail.open.as_ref().map(Draft::reply);
                mail.editor.reset();
            }
            Action::ReplyAll => {
                mail.draft = mail.reply_all(&email);
                mail.editor.reset();
            }
            Action::ToggleThread(id) => {
                if !mail.thread_expanded.remove(&id) {
                    mail.thread_expanded.insert(id);
                }
            }
            Action::Forward => {
                mail.draft = mail.open.as_ref().map(|m| Draft::forward(m, &long_date(m.summary.date)));
                mail.editor.reset();
            }
            Action::Compose => {
                mail.draft = Some(Draft::new());
                mail.editor.reset();
            }
            Action::Send => mail.send(acc),
            Action::CancelDraft => mail.draft = None,
            Action::Refresh => mail.refresh_folder(acc),
            Action::Attachment(a) => mail.activate_attachment(acc, &a),
            Action::AttachmentExternal(a) => mail.open_attachment_external(acc, &a),
            Action::ClosePreview => mail.attachments.preview = None,
        }
    }
}

// --- Écran de connexion ----------------------------------------------------

fn connect_screen(ui: &mut egui::Ui, acc: &mut Account) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, t!("Connecter Gmail", "Connect Gmail"), 0.0, |ui| {
                ui.label(
                    RichText::new(t!(
                        "Connecte ton compte Google pour recevoir, lire, répondre, \
                         transférer et ranger tes mails depuis WorkTable.",
                        "Connect your Google account to receive, read, reply to, \
                         forward and file your mails from WorkTable."
                    ))
                    .color(theme::TEXT_MUTED()),
                );
                ui.add_space(12.0);
                tabs::settings::google_account_form(ui, acc);
            });
        });
    });
}

// --- Colonne de gauche -----------------------------------------------------

fn sidebar(ui: &mut egui::Ui, mail: &mut MailState, cats_open: bool, toggle_cats: &mut bool, actions: &mut Vec<Action>) {
    let full_w = ui.available_width();
    if ui.add_sized([full_w, 32.0], egui::Button::new(t!("Nouveau message", "New message"))).clicked() {
        actions.push(Action::Compose);
    }
    ui.add_space(12.0);

    // Moitié haute : dossiers + catégories.
    let top_h = (ui.available_height() * 0.62).max(160.0);
    egui::ScrollArea::vertical().id_salt("mail_folders").max_height(top_h).auto_shrink([false, true]).show(ui, |ui| {
        // Une seule forme liquide pour dossiers, annuaire et catégories.
        let mut hl = Liquid::begin(ui, "mail_nav");
        section_title(ui, t!("Dossiers", "Folders"));
        for id in FOLDERS {
            if nav_row(ui, &mail.folder_name(id), !mail.directory && mail.folder == id, &mut hl).clicked() {
                actions.push(Action::Folder(id.into()));
            }
        }
        if nav_row(ui, &tf!("Annuaire · {}", "Directory · {}", mail.contacts.len()), mail.directory, &mut hl).clicked() {
            actions.push(Action::Directory);
        }

        ui.add_space(12.0);
        ui.horizontal(|ui| {
            // En-tête cliquable : replie / déplie les catégories.
            let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 30.0, 18.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if cats_open {
                t!("Replier les catégories", "Collapse categories")
            } else {
                t!("Déplier les catégories", "Expand categories")
            });
            let k = liquid::animate(ui.ctx(), "cats_chevron", cats_open);
            chevron(ui.painter(), Pos2::new(r.left() + 5.0, r.center().y), k, theme::TEXT_MUTED());
            let n = mail.labels.iter().filter(|l| l.user).count();
            let title = if cats_open { t!("CATÉGORIES", "CATEGORIES").to_string() } else { tf!("CATÉGORIES · {n}", "CATEGORIES · {n}") };
            ui.painter().text(
                Pos2::new(r.left() + 16.0, r.center().y),
                egui::Align2::LEFT_CENTER,
                title,
                FontId::proportional(11.0),
                if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() },
            );
            if resp.clicked() {
                *toggle_cats = true;
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("+").on_hover_text(t!("Nouvelle catégorie", "New category")).clicked() {
                    actions.push(Action::StartCategory);
                    if !cats_open {
                        *toggle_cats = true;
                    }
                }
            });
        });
        if let Some(name) = &mut mail.new_category {
            match name_input(ui, "new_category", name, t!("Nom de la catégorie", "Category name")) {
                Some(true) if !name.trim().is_empty() => actions.push(Action::CreateCategory(name.clone())),
                Some(_) => actions.push(Action::CancelCategory),
                None => {}
            }
        }
        let cats: Vec<Label> = mail.labels.iter().filter(|l| l.user).cloned().collect();
        let (folder, directory) = (mail.folder.clone(), mail.directory);
        liquid::collapse(ui, "mail_categories", cats_open, |ui| {
            if cats.is_empty() && mail.new_category.is_none() {
                ui.label(RichText::new(t!("Aucune catégorie", "No category")).small().color(theme::TEXT_FAINT()));
            }
            for l in &cats {
                if nav_row(ui, &l.name, !directory && folder == l.id, &mut hl).clicked() {
                    actions.push(Action::Folder(l.id.clone()));
                }
            }
        });
        hl.end(ui, SELECTED(), 6.0);

        ui.add_space(12.0);
        groups_section(ui, mail);
    });

    // Moitié basse : favoris, toujours visibles tant qu'ils restent en favoris.
    ui.add_space(8.0);
    ui.separator();
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
        draw_star(ui.painter(), r.center(), 6.0, true, theme::TEXT());
        section_title(ui, &tf!("Favoris · {}", "Starred · {}", mail.starred.len()));
    });
    egui::ScrollArea::vertical().id_salt("mail_starred").auto_shrink([false, false]).show(ui, |ui| {
        if mail.starred.is_empty() {
            ui.label(RichText::new(t!("Ajoute un mail en favori pour le garder ici", "Star a mail to keep it here")).small().color(theme::TEXT_FAINT()));
        }
        let open_id = mail.open.as_ref().map(|m| m.summary.id.as_str());
        let mut hl = Liquid::begin(ui, "mail_starred");
        for m in &mail.starred {
            let r = starred_row(ui, m, open_id == Some(m.id.as_str()), &mut hl);
            if let Some(g) = groups::group_of(m, &mail.groups) {
                let bar = Rect::from_min_size(r.rect.left_top() + vec2(0.0, 8.0), vec2(3.0, r.rect.height() - 16.0));
                ui.painter().rect_filled(bar, CornerRadius::same(2), groups::color32(g.color));
            }
            if r.clicked() {
                actions.push(Action::Open(m.id.clone()));
            }
        }
        hl.end(ui, SELECTED(), 6.0);
    });
}

/// Champ de saisie avec boutons « ✓ » / « ✕ ».
/// Renvoie `Some(true)` pour valider (Entrée ou ✓), `Some(false)` pour annuler (Échap ou ✕).
/// Le focus n'est donné qu'à l'ouverture : le reprendre à chaque image empêchait Entrée de valider.
fn name_input(ui: &mut egui::Ui, salt: &str, text: &mut String, hint: &str) -> Option<bool> {
    let id = egui::Id::unique(("name_input", salt));
    let mut out = None;
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 58.0).max(60.0);
        // Focus mémorisé avant l'affichage : Entrée fait perdre le focus au champ pendant cette image.
        let had_focus = ui.memory(|m| m.has_focus(id));
        let r = ui.add(egui::TextEdit::singleline(text).id(id).hint_text(hint).desired_width(w));
        let opened = id.with("opened");
        if !ui.data(|d| d.get_temp::<bool>(opened).unwrap_or(false)) {
            r.request_focus();
            ui.data_mut(|d| d.insert_temp(opened, true));
        }
        let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
        let focused = had_focus || r.lost_focus() || r.has_focus();
        if enter && focused {
            out = Some(true);
        } else if esc && focused {
            out = Some(false);
        }
        if ui.small_button("✓").on_hover_text(t!("Valider (Entrée)", "Confirm (Enter)")).clicked() {
            out = Some(true);
        }
        if ui.small_button("✕").on_hover_text(t!("Annuler (Échap)", "Cancel (Esc)")).clicked() {
            out = Some(false);
        }
        if out == Some(true) && text.trim().is_empty() {
            r.request_focus();
        }
    });
    if out.is_some() {
        ui.data_mut(|d| d.remove::<bool>(id.with("opened")));
    }
    out
}

fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).small().strong().color(theme::TEXT_MUTED()));
}

/// Section « Groupes » : adresses intermédiaires colorées + adresses détectées à colorer.
fn groups_section(ui: &mut egui::Ui, mail: &mut MailState) {
    let open = mail.groups_open;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width() - 30.0, 18.0), Sense::click());
        let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if open {
            t!("Replier les groupes", "Collapse groups")
        } else {
            t!("Déplier les groupes", "Expand groups")
        });
        let k = liquid::animate(ui.ctx(), "groups_chevron", open);
        chevron(ui.painter(), Pos2::new(r.left() + 5.0, r.center().y), k, theme::TEXT_MUTED());
        let title = if open { t!("GROUPES", "GROUPS").to_string() } else { tf!("GROUPES · {}", "GROUPS · {}", mail.groups.len()) };
        ui.painter().text(
            Pos2::new(r.left() + 16.0, r.center().y),
            egui::Align2::LEFT_CENTER,
            title,
            FontId::proportional(11.0),
            if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() },
        );
        if resp.clicked() {
            mail.toggle_groups = true;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("+").on_hover_text(t!("Colorer une adresse de groupe", "Colour a group address")).clicked() {
                mail.new_group = Some(String::new());
                if !open {
                    mail.toggle_groups = true;
                }
            }
        });
    });

    // Adresses intermédiaires repérées dans la boîte et pas encore colorées.
    let mut detected: Vec<String> = Vec::new();
    for m in &mail.messages {
        for a in groups::intermediates(m, &mail.own) {
            if !detected.contains(&a) && !mail.groups.iter().any(|g| g.address.eq_ignore_ascii_case(&a)) {
                detected.push(a);
            }
        }
    }

    let (groups_list, mut edits, mut new_group) = (mail.groups.clone(), Vec::new(), mail.new_group.take());
    liquid::collapse(ui, "mail_groups", open, |ui| {
        if let Some(addr) = &mut new_group {
            match name_input(ui, "new_group", addr, "adresse@groupe.fr") {
                Some(true) if addr.contains('@') => {
                    edits.push(groups::GroupEdit::Color(addr.trim().to_lowercase(), groups::next_color(&groups_list)));
                    new_group = None;
                }
                Some(true) => {} // adresse incomplète : on laisse la saisie ouverte
                Some(false) => new_group = None,
                None => {}
            }
        }
        if groups_list.is_empty() && detected.is_empty() && new_group.is_none() {
            ui.label(
                RichText::new(t!("Les mails reçus via une adresse de groupe peuvent être colorés ici", "Mails received through a group address can be coloured here"))
                    .small()
                    .color(theme::TEXT_FAINT()),
            );
        }
        for g in &groups_list {
            ui.horizontal(|ui| {
                ui.add_space(8.0);
                if let Some(e) = groups::swatch(ui, g, 12.0) {
                    edits.push(e);
                }
                ui.add_space(4.0);
                let w = ui.available_width();
                let gal = one_line(ui.painter(), &g.address, FontId::proportional(13.0), theme::TEXT_MUTED(), w);
                ui.label(gal).on_hover_text(&g.address);
            });
        }
        if !detected.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(t!("Détectés — cliquer pour colorer", "Detected — click to colour")).small().color(theme::TEXT_FAINT()));
            for a in detected.iter().take(6) {
                let gal = one_line(ui.painter(), &format!("+  {a}"), FontId::proportional(12.0), theme::TEXT_FAINT(), ui.available_width() - 8.0);
                if ui.add(egui::Button::new(gal).frame(false)).on_hover_text(t!("Associer une couleur à cette adresse", "Assign a colour to this address")).clicked() {
                    edits.push(groups::GroupEdit::Color(a.clone(), groups::next_color(&groups_list)));
                }
            }
        }
    });
    mail.new_group = new_group;
    mail.group_edits.extend(edits);
}

/// Fond de sélection (forme liquide) des listes.
#[allow(non_snake_case)]
fn SELECTED() -> Color32 {
    crate::theme::gray(0x2A)
}
/// Survol : blanc translucide, pour rester visible par-dessus la forme liquide.
const HOVER: Color32 = Color32::from_rgba_premultiplied(7, 7, 7, 7);

/// Chevron de repli : `k` = 0 (replié, ▸) → 1 (déplié, ▾).
fn chevron(p: &Painter, c: Pos2, k: f32, color: Color32) {
    let a = k * std::f32::consts::FRAC_PI_2;
    let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
    p.add(egui::Shape::line(vec![rot(-2.0, -4.0), rot(2.5, 0.0), rot(-2.0, 4.0)], Stroke::new(1.5, color)));
}

fn nav_row(ui: &mut egui::Ui, text: &str, active: bool, hl: &mut Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if active {
        hl.target(rect, text);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let color = if active { theme::TEXT() } else { theme::TEXT_MUTED() };
    let g = one_line(p, text, FontId::proportional(14.0), color, rect.width() - 20.0);
    p.galley(rect.left_center() + vec2(10.0, -g.size().y / 2.0), g, color);
    resp
}

fn starred_row(ui: &mut egui::Ui, m: &Summary, active: bool, hl: &mut Liquid) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    if active {
        hl.target(rect, &m.id);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let w = rect.width() - 20.0;
    let subject = if m.subject.is_empty() { t!("(sans objet)", "(no subject)") } else { &m.subject };
    let g1 = one_line(p, subject, FontId::proportional(13.0), theme::TEXT(), w);
    let g2 = one_line(p, m.sender_name(), FontId::proportional(11.0), theme::TEXT_MUTED(), w);
    p.galley(rect.left_top() + vec2(10.0, 6.0), g1, theme::TEXT());
    p.galley(rect.left_top() + vec2(10.0, 25.0), g2, theme::TEXT_MUTED());
    resp
}

// --- Liste des messages ----------------------------------------------------

fn message_list(ui: &mut egui::Ui, mail: &MailState, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(mail.folder_name(&mail.folder)).strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if mail.folder.starts_with("from:") && ui.small_button("×").on_hover_text(t!("Retirer le filtre", "Remove the filter")).clicked() {
                actions.push(Action::Folder(INBOX.into()));
            }
            if ui.small_button(t!("Actualiser", "Refresh")).clicked() {
                actions.push(Action::Refresh);
            }
            if mail.loading {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
    });
    ui.add_space(8.0);

    let open_id = mail.open.as_ref().map(|m| m.summary.id.as_str());
    // Mails épinglés : accrochés en haut de la boîte de réception, hors défilement.
    let show_pins = mail.folder == INBOX && !mail.pinned.list.is_empty();
    if show_pins {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(12.0, 12.0), Sense::hover());
            draw_pin(ui.painter(), r.center(), 6.0, true, theme::TEXT());
            ui.label(RichText::new(tf!("ÉPINGLÉS · {}/{}", "PINNED · {}/{}", mail.pinned.list.len(), super::pinned::MAX)).small().strong().color(theme::TEXT_MUTED()));
        });
        ui.add_space(2.0);
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut hl = Liquid::begin(ui, "mail_pinned");
        for m in &mail.pinned.list {
            // Version la plus récente si le mail est dans la liste chargée.
            let m = mail.messages.iter().find(|x| x.id == m.id).unwrap_or(m);
            let group = groups::group_of(m, &mail.groups);
            let n = mail.thread_sizes.get(&m.thread_id).copied().unwrap_or(1);
            message_row(ui, m, open_id == Some(m.id.as_str()), true, n, &mut hl, group, actions);
        }
        hl.end(ui, SELECTED(), 8.0);
        ui.add_space(4.0);
        let y = ui.cursor().top();
        let x = ui.max_rect().x_range();
        ui.painter().hline(x, y, Stroke::new(1.0, theme::BORDER()));
        ui.add_space(6.0);
    }

    if mail.messages.is_empty() {
        if mail.loading {
            for r in [0.85, 0.6, 0.75, 0.5] {
                widgets::ghost_row(ui, r);
                widgets::ghost_row(ui, r * 0.6);
                ui.add_space(10.0);
            }
        } else {
            widgets::empty_state(ui, t!("Aucun message", "No messages"), t!("Ce dossier est vide", "This folder is empty"));
        }
        return;
    }

    egui::ScrollArea::vertical().id_salt("mail_list").auto_shrink([false, false]).show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let mut hl = Liquid::begin(ui, "mail_list");
        // Une ligne par conversation (le message le plus récent), comme Gmail.
        let mut threads_seen = std::collections::HashSet::new();
        let pinned_threads: std::collections::HashSet<&str> = if show_pins { mail.pinned.list.iter().map(|m| m.thread_id.as_str()).collect() } else { Default::default() };
        for m in &mail.messages {
            let pinned = mail.pinned.contains(&m.id);
            if (pinned && show_pins) || (!m.thread_id.is_empty() && pinned_threads.contains(m.thread_id.as_str())) {
                continue; // déjà affiché en haut
            }
            if !m.thread_id.is_empty() && !threads_seen.insert(m.thread_id.as_str()) {
                continue;
            }
            let group = groups::group_of(m, &mail.groups);
            let n = mail.thread_sizes.get(&m.thread_id).copied().unwrap_or(1);
            message_row(ui, m, open_id == Some(m.id.as_str()), pinned, n, &mut hl, group, actions);
        }
        hl.end(ui, SELECTED(), 8.0);
    });
}

#[allow(clippy::too_many_arguments)]
fn message_row(ui: &mut egui::Ui, m: &Summary, active: bool, pinned: bool, count: usize, hl: &mut Liquid, group: Option<&GroupColor>, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::click());
    let star_rect = Rect::from_center_size(rect.right_bottom() + vec2(-16.0, -16.0), vec2(22.0, 22.0));
    let star = ui.interact(star_rect, resp.id.with("star"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if m.starred() {
        t!("Retirer des favoris", "Remove from starred")
    } else {
        t!("Ajouter aux favoris", "Add to starred")
    });

    let pin_rect = star_rect.translate(vec2(-24.0, 0.0));
    let pin = ui.interact(pin_rect, resp.id.with("pin"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if pinned {
        t!("Décrocher", "Unpin")
    } else {
        t!("Épingler en haut de la boîte (3 max)", "Pin to the top of the inbox (3 max)")
    });

    if star.clicked() {
        actions.push(Action::Star(m.clone()));
    } else if pin.clicked() {
        actions.push(Action::Pin(m.clone()));
    } else if resp.clicked() {
        actions.push(Action::Open(m.id.clone()));
    }
    // Clic droit : actions rapides.
    let is_spam = m.labels.iter().any(|l| l == "SPAM");
    resp.context_menu(|ui| {
        ui.set_min_width(190.0);
        if ui.button(t!("Ouvrir", "Open")).clicked() {
            actions.push(Action::Open(m.id.clone()));
        }
        let fav = if m.starred() { t!("Retirer des favoris", "Remove from starred") } else { t!("Ajouter aux favoris", "Add to starred") };
        if ui.button(fav).clicked() {
            actions.push(Action::Star(m.clone()));
        }
        if ui.button(if pinned { t!("Décrocher", "Unpin") } else { t!("Épingler en haut", "Pin to top") }).clicked() {
            actions.push(Action::Pin(m.clone()));
        }
        ui.separator();
        let label = if is_spam { t!("Ce n'est pas un spam", "Not spam") } else { t!("Signaler comme spam", "Report as spam") };
        if ui.button(label).clicked() {
            actions.push(Action::Spam(m.id.clone(), !is_spam));
        }
        let in_trash = m.labels.iter().any(|l| l == "TRASH");
        if ui.button(if in_trash { t!("Restaurer de la corbeille", "Restore from trash") } else { t!("Mettre à la corbeille", "Move to trash") }).clicked() {
            actions.push(Action::Trash(m.id.clone(), !in_trash));
        }
    });

    // Épingle, étoile et nom du groupe : seulement au survol de la ligne.
    let hover = ui.rect_contains_pointer(rect);
    let p = ui.painter();
    // Mail épinglé : fond plus clair.
    if pinned {
        p.rect_filled(rect, CornerRadius::same(8), crate::theme::overlay(16));
    }
    if active {
        hl.target(rect, &m.id);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), HOVER);
    }

    // Mail reçu via une adresse de groupe colorée : teinte + liseré + nom du groupe.
    let mut tag_w = 0.0;
    if let Some(g) = group {
        let c = groups::color32(g.color);
        p.rect_filled(rect, CornerRadius::same(8), c.gamma_multiply(0.10));
        let bar = Rect::from_min_size(rect.left_top() + vec2(0.0, 8.0), vec2(3.0, rect.height() - 16.0));
        p.rect_filled(bar, CornerRadius::same(2), c);
        if hover {
            let name = g.address.split('@').next().unwrap_or(&g.address);
            let tag = one_line(p, name, FontId::proportional(10.5), c, 110.0);
            tag_w = tag.size().x + 8.0;
            p.galley(Pos2::new(rect.right() - 56.0 - tag.size().x, rect.top() + 46.0), tag, c);
        }
    }

    let unread = m.unread();
    let left = rect.left() + 18.0;
    if unread {
        p.circle_filled(Pos2::new(rect.left() + 8.0, rect.top() + 15.0), 3.0, theme::TEXT());
    }

    let date = short_date(m.date);
    let date_g = one_line(p, &date, FontId::proportional(11.0), theme::TEXT_MUTED(), 120.0);
    let date_w = date_g.size().x;
    p.galley(Pos2::new(rect.right() - 10.0 - date_w, rect.top() + 9.0), date_g, theme::TEXT_MUTED());

    let name_color = if unread { theme::TEXT() } else { theme::TEXT_MUTED() };
    let name_font = FontId::proportional(14.0);
    // Pastille : nombre de messages de la conversation.
    let badge = (count > 1).then(|| one_line(p, &count.to_string(), FontId::proportional(10.5), theme::TEXT(), 40.0));
    let badge_w = badge.as_ref().map_or(0.0, |g| g.size().x + 18.0);
    let name_g = one_line(p, m.sender_name(), name_font, name_color, rect.right() - left - date_w - 24.0 - badge_w);
    let name_end = left + name_g.size().x;
    p.galley(Pos2::new(left, rect.top() + 6.0), name_g, name_color);
    if let Some(g) = badge {
        let r = Rect::from_min_size(Pos2::new(name_end + 8.0, rect.top() + 8.0), vec2(g.size().x + 10.0, 16.0));
        p.rect_filled(r, CornerRadius::same(8), theme::SURFACE_HI());
        p.rect_stroke(r, CornerRadius::same(8), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        p.galley(r.center() - g.size() / 2.0, g, theme::TEXT());
    }

    let w = rect.right() - left - 64.0;
    let subject = if m.subject.is_empty() { t!("(sans objet)", "(no subject)") } else { &m.subject };
    let subj_color = if unread { theme::TEXT() } else { crate::theme::gray(0xBB) };
    let g = one_line(p, subject, FontId::proportional(13.0), subj_color, w);
    p.galley(Pos2::new(left, rect.top() + 26.0), g, subj_color);
    let g = one_line(p, &m.snippet, FontId::proportional(12.0), theme::TEXT_FAINT(), w - tag_w);
    p.galley(Pos2::new(left, rect.top() + 44.0), g, theme::TEXT_FAINT());

    let star_color = if m.starred() {
        theme::TEXT()
    } else if star.hovered() {
        theme::TEXT_MUTED()
    } else {
        theme::TEXT_FAINT()
    };
    if !hover {
        return;
    }
    draw_star(p, star_rect.center(), 7.0, m.starred(), star_color);
    let pin_color = if pinned {
        theme::TEXT()
    } else if pin.hovered() {
        theme::TEXT_MUTED()
    } else {
        theme::TEXT_FAINT()
    };
    draw_pin(p, pin_rect.center(), 7.0, pinned, pin_color);
}

// --- Lecture / rédaction ---------------------------------------------------

fn reading_pane(ui: &mut egui::Ui, mail: &mut MailState, acc: &Account, email: &str, list_hidden: bool, toggle_list: &mut bool, actions: &mut Vec<Action>) {
    // Bouton pour masquer / réafficher la liste des mails.
    ui.horizontal(|ui| {
        let (label, tip) = if list_hidden {
            (t!("›  Afficher la liste", "›  Show the list"), t!("Réafficher la liste des mails", "Show the mail list again"))
        } else {
            (t!("‹  Masquer la liste", "‹  Hide the list"), t!("Masquer la liste pour agrandir la lecture", "Hide the list to widen the reading pane"))
        };
        if ui.add(egui::Button::new(RichText::new(label).small().color(theme::TEXT_MUTED())).frame(false)).on_hover_text(tip).clicked() {
            *toggle_list = true;
        }
    });
    if let Some(d) = &mut mail.draft {
        // Aperçu fidèle de la signature (mise en forme, couleurs, images hébergées).
        let sig_blocks = d.signature.as_deref().map(crate::google::html::from_html).unwrap_or_default();
        mail.inline.request_blocks(acc, "signature", &sig_blocks, |_| None);
        let inline_imgs = &mut mail.inline;
        let mut preview = |ui: &mut egui::Ui| {
            let ctx = ui.ctx().clone();
            for (i, b) in sig_blocks.iter().enumerate() {
                match b {
                    BodyBlock::Para(p) => {
                        if let (Some(url), _) = rich_para(ui, p) {
                            let _ = webbrowser::open(&url);
                        }
                    }
                    BodyBlock::Image { src, alt, width, height, max_width, align, link } => {
                        let key = inline::key("signature", i, src);
                        let spec = ImageSpec { width: *width, height: *height, max_width: *max_width, align: *align };
                        body_image(ui, inline_imgs.get(&ctx, &key), alt, spec, link.as_deref());
                    }
                }
            }
        };
        match composer::show(ui, d, &mut mail.editor, mail.sending, &mail.contacts, &mail.signatures, &mut preview) {
            Some(composer::Out::Send) => actions.push(Action::Send),
            Some(composer::Out::Cancel) => actions.push(Action::CancelDraft),
            Some(composer::Out::Error(e)) => mail.notice = Some(super::Notice { text: e, error: true, at: std::time::Instant::now() }),
            None => {}
        }
        return;
    }
    if mail.opening.is_some() {
        ui.add_space(ui.available_height() * 0.42);
        ui.vertical_centered(|ui| ui.add(egui::Spinner::new()));
        return;
    }
    let Some(m) = &mail.open else {
        ui.add_space(ui.available_height() * 0.42);
        widgets::empty_state(ui, t!("Aucun message sélectionné", "No message selected"), t!("Choisis un mail dans la liste", "Choose a mail in the list"));
        return;
    };
    let s = &m.summary;

    ui.add(egui::Label::new(RichText::new(if s.subject.is_empty() { t!("(sans objet)", "(no subject)") } else { &s.subject }).font(theme::title_font(20.0))).wrap());
    ui.add_space(4.0);
    // Conversation : messages plus anciens au-dessus, plus récents (ex. ta réponse) en dessous.
    let (earlier, later): (Vec<&FullMessage>, Vec<&FullMessage>) = mail.thread.iter().partition(|t| t.summary.date <= s.date);
    let latest = later.last().map(|t| t.summary.id.clone());
    if !mail.thread.is_empty() {
        ui.label(RichText::new(tf!("CONVERSATION · {} MESSAGES", "CONVERSATION · {} MESSAGES", mail.thread.len() + 1)).small().strong().color(theme::TEXT_MUTED()));
    }
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("mail_body").auto_shrink([false, false]).show(ui, |ui| {
        for t in &earlier {
            let open = mail.thread_expanded.contains(&t.summary.id);
            if thread_card(ui, t, open, &mut mail.inline, &mail.attachments, email, actions) {
                actions.push(Action::ToggleThread(t.summary.id.clone()));
            }
        }
        if !earlier.is_empty() {
            ui.add_space(6.0);
        }
        ui.horizontal(|ui| {
            sender_menu(ui, &mail.contacts, &s.from, actions);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(long_date(s.date)).small().color(theme::TEXT_MUTED()));
            });
        });
        if !m.to.is_empty() {
            ui.add(egui::Label::new(RichText::new(tf!("À : {}", "To: {}", m.to)).small().color(theme::TEXT_MUTED())).wrap());
        }
        if !m.cc.is_empty() {
            ui.add(egui::Label::new(RichText::new(tf!("Cc : {}", "Cc: {}", m.cc)).small().color(theme::TEXT_MUTED())).wrap());
        }
        // Reçu via une adresse intermédiaire (groupe / liste) : couleur réglable.
        let via = groups::intermediates(s, &mail.own);
        if !via.is_empty() {
            ui.add_space(4.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(t!("Reçu via", "Received via")).small().color(theme::TEXT_MUTED()));
                for a in via.iter().take(4) {
                    match mail.groups.iter().find(|g| g.address.eq_ignore_ascii_case(a)) {
                        Some(g) => {
                            if let Some(e) = groups::swatch(ui, g, 11.0) {
                                mail.group_edits.push(e);
                            }
                            ui.label(RichText::new(a).small().color(groups::color32(g.color)));
                        }
                        None => {
                            ui.label(RichText::new(a).small().color(theme::TEXT_MUTED()));
                            if ui
                                .small_button(t!("Colorer", "Colour"))
                                .on_hover_text(t!("Colorer les mails reçus via cette adresse", "Colour mails received through this address"))
                                .clicked()
                            {
                                mail.group_edits.push(groups::GroupEdit::Color(a.clone(), groups::next_color(&mail.groups)));
                            }
                        }
                    }
                }
            });
        }

        ui.add_space(10.0);
        ui.horizontal_wrapped(|ui| {
            ui.menu_button(t!("Action  ▾", "Action  ▾"), |ui| {
                ui.set_min_width(170.0);
                if ui.button(t!("Répondre", "Reply")).clicked() {
                    actions.push(Action::Reply);
                    ui.close();
                }
                if ui.button(t!("Répondre à tous", "Reply all")).clicked() {
                    actions.push(Action::ReplyAll);
                    ui.close();
                }
                if ui.button(t!("Transférer", "Forward")).clicked() {
                    actions.push(Action::Forward);
                    ui.close();
                }
            });
            let is_spam = s.labels.iter().any(|l| l == "SPAM");
            let (spam_label, spam_tip) = if is_spam {
                (t!("Pas un spam", "Not spam"), t!("Remettre ce mail dans la boîte de réception", "Move this mail back to the inbox"))
            } else {
                ("Spam", t!("Signaler comme spam : le mail part dans le dossier Spam", "Report as spam: the mail goes to the Spam folder"))
            };
            if ui.button(spam_label).on_hover_text(spam_tip).clicked() {
                actions.push(Action::Spam(s.id.clone(), !is_spam));
            }
            let in_trash = s.labels.iter().any(|l| l == "TRASH");
            let (trash_label, trash_tip) = if in_trash {
                (t!("Restaurer", "Restore"), t!("Sortir ce mail de la corbeille (retour dans la boîte de réception)", "Take this mail out of the trash (back to the inbox)"))
            } else {
                (t!("🗑 Corbeille", "🗑 Trash"), t!("Mettre ce mail à la corbeille (Suppr)", "Move this mail to trash (Del)"))
            };
            if ui.button(trash_label).on_hover_text(trash_tip).clicked() {
                actions.push(Action::Trash(s.id.clone(), !in_trash));
            }
            let fav = if s.starred() { t!("Retirer des favoris", "Remove from starred") } else { t!("Ajouter aux favoris", "Add to starred") };
            if icon_button(ui, fav, |p, c, col| draw_star(p, c, 8.0, s.starred(), col), s.starred()).clicked() {
                actions.push(Action::Star(s.clone()));
            }
            let pinned = mail.pinned.contains(&s.id);
            let pin = if pinned { t!("Décrocher", "Unpin") } else { t!("Épingler en haut de la boîte (3 max)", "Pin to the top of the inbox (3 max)") };
            if icon_button(ui, pin, |p, c, col| draw_pin(p, c, 8.0, pinned, col), pinned).clicked() {
                actions.push(Action::Pin(s.clone()));
            }
            ui.menu_button(t!("Ranger dans…", "Move to…"), |ui| {
                ui.set_min_width(200.0);
                let cats: Vec<&Label> = mail.labels.iter().filter(|l| l.user).collect();
                if cats.is_empty() {
                    ui.label(
                        RichText::new(t!("Crée d'abord une catégorie (bouton + à gauche)", "Create a category first (+ button on the left)")).small().color(theme::TEXT_MUTED()),
                    );
                }
                for l in cats {
                    let already = s.labels.contains(&l.id);
                    if ui.add_enabled(!already, egui::Button::new(&l.name)).clicked() {
                        actions.push(Action::Move(s.id.clone(), l.clone()));
                        ui.close();
                    }
                }
            });
        });

        // Catégories déjà attribuées.
        let tags: Vec<&Label> = mail.labels.iter().filter(|l| l.user && s.labels.contains(&l.id)).collect();
        if !tags.is_empty() {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                for l in tags {
                    chip(ui, &l.name);
                }
            });
        }

        // Pastilles « documents » : une par pièce jointe et par lien de téléchargement ;
        // un clic fait défiler jusqu'au document et le met en surbrillance.
        let docs = documents(m);
        if !docs.is_empty() {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                for d in &docs {
                    if doc_pill(ui, d).clicked() {
                        mail.jump = Some(d.target.clone());
                    }
                }
            });
        }

        ui.add_space(10.0);
        ui.separator();
        ui.add_space(6.0);
        {
            let ctx = ui.ctx().clone();
            for (i, block) in m.blocks.iter().enumerate() {
                match block {
                    BodyBlock::Para(p) => {
                        let (url, rect) = rich_para(ui, p);
                        if let Some(url) = url {
                            let _ = webbrowser::open(&url);
                        }
                        if rect.is_positive() {
                            jump_target(ui, &mut mail.jump, &mut mail.flash, DocTarget::Block(i), rect, theme::LINK());
                        }
                    }
                    BodyBlock::Image { src, alt, width, height, max_width, align, link } => {
                        let key = inline::key(&m.summary.id, i, src);
                        let spec = ImageSpec { width: *width, height: *height, max_width: *max_width, align: *align };
                        if body_image(ui, mail.inline.get(&ctx, &key), alt, spec, link.as_deref())
                            && let Some(url) = link
                        {
                            let _ = webbrowser::open(url);
                        }
                    }
                }
                ui.add_space(6.0);
            }

            // Pièces jointes en bas du mail (aperçu, ouverture, ou lien Gmail en dernier recours).
            let listed: Vec<&Attachment> = m.listed_attachments().collect();
            if !listed.is_empty() {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.label(RichText::new(tf!("PIÈCES JOINTES · {}", "ATTACHMENTS · {}", listed.len())).small().strong().color(theme::TEXT_MUTED()));
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for a in listed {
                        let rect = attachment_chip(ui, a, &mail.attachments, email, actions);
                        jump_target(ui, &mut mail.jump, &mut mail.flash, DocTarget::Attachment(a.key()), rect, theme::TEXT());
                    }
                });
                ui.add_space(12.0);
            }
        }
        for t in &later {
            let open = mail.thread_expanded.contains(&t.summary.id) != (latest.as_deref() == Some(t.summary.id.as_str()));
            if thread_card(ui, t, open, &mut mail.inline, &mail.attachments, email, actions) {
                actions.push(Action::ToggleThread(t.summary.id.clone()));
            }
        }
    });
}

/// Message d'une conversation : replié (expéditeur, date, aperçu) ou déplié (corps complet).
/// Renvoie `true` si l'en-tête a été cliqué (plier / déplier).
fn thread_card(
    ui: &mut egui::Ui,
    t: &FullMessage,
    open: bool,
    inline_imgs: &mut inline::InlineImages,
    atts: &attachments::Attachments,
    email: &str,
    actions: &mut Vec<Action>,
) -> bool {
    let mut toggled = false;
    egui::Frame::new()
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(8))
        .fill(if open { Color32::TRANSPARENT } else { crate::theme::overlay(4) })
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let s = &t.summary;
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if open { 20.0 } else { 38.0 }), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(if open { t!("Replier", "Collapse") } else { t!("Déplier", "Expand") });
            toggled = resp.clicked();
            let p = ui.painter();
            let date = short_date(s.date);
            let dg = one_line(p, &date, FontId::proportional(11.0), theme::TEXT_MUTED(), 140.0);
            let dw = dg.size().x;
            p.galley(Pos2::new(rect.right() - dw, rect.top() + 2.0), dg, theme::TEXT_MUTED());
            let ng = one_line(p, s.sender_name(), theme::editor_font(13.5, true), theme::TEXT(), rect.width() - dw - 12.0);
            p.galley(rect.left_top(), ng, theme::TEXT());
            if !open {
                let g = one_line(p, &s.snippet, FontId::proportional(12.0), theme::TEXT_FAINT(), rect.width());
                p.galley(rect.left_top() + vec2(0.0, 20.0), g, theme::TEXT_FAINT());
                return;
            }
            if !t.to.is_empty() {
                ui.add(egui::Label::new(RichText::new(tf!("À : {}", "To: {}", t.to)).small().color(theme::TEXT_MUTED())).wrap());
            }
            ui.add_space(6.0);
            let ctx = ui.ctx().clone();
            for (i, block) in t.blocks.iter().enumerate() {
                match block {
                    BodyBlock::Para(p) => {
                        if let (Some(url), _) = rich_para(ui, p) {
                            let _ = webbrowser::open(&url);
                        }
                    }
                    BodyBlock::Image { src, alt, width, height, max_width, align, link } => {
                        let key = inline::key(&s.id, i, src);
                        let spec = ImageSpec { width: *width, height: *height, max_width: *max_width, align: *align };
                        if body_image(ui, inline_imgs.get(&ctx, &key), alt, spec, link.as_deref())
                            && let Some(url) = link
                        {
                            let _ = webbrowser::open(url);
                        }
                    }
                }
                ui.add_space(4.0);
            }
            let listed: Vec<&Attachment> = t.listed_attachments().collect();
            if !listed.is_empty() {
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    for a in listed {
                        attachment_chip(ui, a, atts, email, actions);
                    }
                });
            }
        });
    ui.add_space(6.0);
    toggled
}

/// Bouton rond avec une icône dessinée (étoile, punaise…).
fn icon_button(ui: &mut egui::Ui, tip: &str, icon: impl FnOnce(&Painter, Pos2, Color32), on: bool) -> egui::Response {
    let h = ui.spacing().interact_size.y.max(24.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(h + 6.0, h), Sense::click());
    let resp = resp.on_hover_text(tip).on_hover_cursor(egui::CursorIcon::PointingHand);
    let p = ui.painter();
    let bg = if resp.hovered() { theme::SURFACE_HI() } else { crate::theme::overlay(6) };
    p.rect_filled(rect, CornerRadius::same(6), bg);
    let col = if on || resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() };
    icon(p, rect.center(), col);
    resp
}

/// Document repéré dans un mail (pièce jointe ou lien de téléchargement).
struct Doc {
    target: DocTarget,
    label: String,
    tag: String,
    link: bool,
}

fn documents(m: &crate::google::FullMessage) -> Vec<Doc> {
    let mut out: Vec<Doc> =
        m.listed_attachments().map(|a| Doc { target: DocTarget::Attachment(a.key()), label: a.filename.clone(), tag: a.extension().to_uppercase(), link: false }).collect();
    let mut seen = std::collections::HashSet::new();
    for (i, b) in m.blocks.iter().enumerate() {
        let BodyBlock::Para(p) = b else { continue };
        for r in p.runs.iter().filter(|r| r.download) {
            let Some(url) = &r.link else { continue };
            if !seen.insert(url.clone()) {
                continue;
            }
            out.push(Doc { target: DocTarget::Block(i), label: link_label(&r.text, url), tag: link_tag(url), link: true });
        }
    }
    out
}

/// Nom lisible d'un lien : son texte, ou le nom du fichier dans l'adresse.
fn link_label(text: &str, url: &str) -> String {
    let t = text.trim();
    let looks_like_url = t.starts_with("http") || t.starts_with("www.");
    let label = if !t.is_empty() && !looks_like_url {
        t.to_string()
    } else {
        let path = url.split(['?', '#']).next().unwrap_or(url);
        let last = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path);
        let host = url.split("//").nth(1).and_then(|h| h.split('/').next()).unwrap_or(url);
        if last.contains('.') && !last.contains(host) { urlencoding::decode(last).map(|s| s.into_owned()).unwrap_or_else(|_| last.to_string()) } else { host.to_string() }
    };
    let mut chars = label.chars();
    let short: String = chars.by_ref().take(32).collect();
    if chars.next().is_some() { format!("{short}…") } else { short }
}

fn link_tag(url: &str) -> String {
    let path = url.to_ascii_lowercase();
    let path = path.split(['?', '#']).next().unwrap_or("");
    match path.rsplit('/').next().and_then(|s| s.rsplit_once('.')).map(|(_, e)| e) {
        Some(e) if e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()) => e.to_uppercase(),
        _ => t!("LIEN", "LINK").into(),
    }
}

fn doc_pill(ui: &mut egui::Ui, d: &Doc) -> egui::Response {
    let color = if d.link { theme::LINK() } else { theme::TEXT() };
    let mut job = egui::text::LayoutJob::default();
    job.append(&format!("{}  ", d.tag), 0.0, egui::TextFormat::simple(FontId::proportional(10.0), theme::TEXT_MUTED()));
    job.append(&d.label, 0.0, egui::TextFormat::simple(FontId::proportional(12.5), color));
    ui.add(
        egui::Button::new(job)
            .fill(theme::SURFACE_HI())
            .stroke(Stroke::new(1.0, if d.link { theme::LINK().gamma_multiply(0.45) } else { theme::BORDER() }))
            .corner_radius(CornerRadius::same(255)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
    .on_hover_text(if d.link { t!("Aller au lien de téléchargement", "Go to the download link") } else { t!("Aller à la pièce jointe", "Go to the attachment") })
}

/// Défile vers l'élément s'il est la cible demandée, puis le met en surbrillance qui s'estompe.
fn jump_target(ui: &mut egui::Ui, jump: &mut Option<DocTarget>, flash: &mut Option<(DocTarget, std::time::Instant)>, me: DocTarget, rect: Rect, color: Color32) {
    if jump.as_ref() == Some(&me) {
        ui.scroll_to_rect(rect.expand(24.0), Some(Align::Center));
        *flash = Some((me.clone(), std::time::Instant::now()));
        *jump = None;
    }
    const FLASH: f32 = 1.8;
    if let Some((t, at)) = flash.as_ref().filter(|(t, _)| *t == me) {
        let _ = t;
        let e = at.elapsed().as_secs_f32();
        if e < FLASH {
            // Deux pulsations qui s'estompent.
            let k = (1.0 - e / FLASH) * (0.6 + 0.4 * (e * 7.0).cos().abs());
            let r = rect.expand(5.0);
            ui.painter().rect(r, CornerRadius::same(8), color.gamma_multiply(0.10 * k), Stroke::new(1.5, color.gamma_multiply(k)), egui::StrokeKind::Outside);
            ui.ctx().request_repaint();
        } else {
            *flash = None;
        }
    }
}

/// Expéditeur cliquable : voir ses mails, favori de l'annuaire, écrire.
fn sender_menu(ui: &mut egui::Ui, book: &contacts::Contacts, from: &str, actions: &mut Vec<Action>) {
    let Some((_, email)) = contacts::parse_mailbox(from) else {
        ui.label(RichText::new(from).strong());
        return;
    };
    let contact = book.get(&email);
    let favorite = contact.is_some_and(|c| c.favorite);
    let text = if favorite { format!("★ {from}") } else { from.to_string() };
    let btn = egui::Button::new(RichText::new(text).strong()).frame(false);
    let r = ui.add(btn).on_hover_text(t!("Options de l'annuaire", "Directory options")).on_hover_cursor(egui::CursorIcon::PointingHand);
    egui::Popup::menu(&r).show(|ui| {
        ui.set_min_width(220.0);
        if ui.button(t!("Voir tous ses mails", "See all their mails")).clicked() {
            actions.push(Action::FromFilter(email.clone()));
        }
        if ui.button(t!("Écrire à cette adresse", "Write to this address")).clicked() {
            actions.push(Action::WriteTo(contact.map(|c| c.mailbox()).unwrap_or(email.clone())));
        }
        if ui.button(t!("Écrire sur Google Chat", "Write on Google Chat")).clicked() {
            actions.push(Action::ChatTo(email.clone()));
        }
        let fav = if favorite { t!("Retirer des favoris de l'annuaire", "Remove from directory favourites") } else { t!("Favori dans l'annuaire", "Favourite in the directory") };
        if ui.button(fav).clicked() {
            actions.push(Action::ContactFavorite(email.clone()));
        }
        if ui.button(t!("Ouvrir dans l'annuaire", "Open in the directory")).clicked() {
            actions.push(Action::Dir(directory::DirAction::Select(email.clone())));
            actions.push(Action::Directory);
        }
    });
}

/// Paragraphe mis en forme (gras, italique, souligné, barré, taille, alignement,
/// liens cliquables, citations, listes). Renvoie l'URL d'un lien cliqué.
pub(crate) fn rich_para(ui: &mut egui::Ui, p: &Para) -> (Option<String>, Rect) {
    const BASE: f32 = 14.0;
    if p.runs.is_empty() {
        ui.add_space(BASE * 0.6);
        return (None, Rect::NOTHING);
    }
    let quote_w = p.quote as f32 * 14.0;
    let indent = quote_w + p.indent as f32 * 18.0;
    // 14 px de marge pour la barre de défilement.
    let width = (ui.available_width() - indent - 14.0).max(60.0);
    let text_color = if p.quote > 0 { theme::TEXT_MUTED() } else { theme::TEXT() };

    let mut job = LayoutJob::default();
    let mut links: Vec<(usize, usize, String)> = Vec::new();
    // Émojis : glyphe transparent dans le texte, image couleur dessinée par-dessus.
    let mut emojis_at: Vec<(usize, usize, String, f32)> = Vec::new();
    let mut chars = 0usize;
    for r in &p.runs {
        let size = BASE * r.scale;
        let font = if r.code { FontId::monospace(size * 0.92) } else { theme::editor_font(size, r.bold) };
        let color = if r.download {
            theme::LINK()
        } else if r.link.is_some() {
            Color32::WHITE
        } else if let Some(c) = r.color.map(readable) {
            c
        } else {
            text_color
        };
        let mut f = egui::TextFormat::simple(font, color);
        f.italics = r.italic;
        if r.underline || r.download {
            f.underline = Stroke::new(1.0, color);
        }
        if r.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        if r.code {
            f.background = theme::SURFACE_HI();
        }
        let n = r.text.chars().count();
        if let Some(l) = &r.link {
            links.push((chars, chars + n, l.clone()));
        }
        let found = if r.code { Vec::new() } else { crate::emoji_img::find(&r.text) };
        if found.is_empty() {
            job.append(&r.text, 0.0, f);
        } else {
            let cs: Vec<char> = r.text.chars().collect();
            let mut at = 0;
            for (i, len, e) in found {
                if i > at {
                    job.append(&cs[at..i].iter().collect::<String>(), 0.0, f.clone());
                }
                let mut fe = f.clone();
                fe.color = Color32::TRANSPARENT;
                fe.underline = Stroke::NONE;
                // Un peu d'air : l'image couleur est plus large que le glyphe monochrome.
                fe.extra_letter_spacing = size * 0.3;
                job.append(&cs[i..i + len].iter().collect::<String>(), 0.0, fe);
                emojis_at.push((chars + i, len, e, size));
                at = i + len;
            }
            if at < cs.len() {
                job.append(&cs[at..].iter().collect::<String>(), 0.0, f);
            }
        }
        chars += n;
    }
    job.wrap.max_width = width;
    job.halign = match p.align {
        HAlign::Center => Align::Center,
        HAlign::Right => Align::Max,
        _ => Align::Min,
    };
    job.justify = p.align == HAlign::Justify;
    let galley = ui.painter().layout_job(job);

    // Centrage / alignement à droite dans la largeur disponible.
    let gw = galley.size().x;
    let offset = match p.align {
        HAlign::Center => (width - gw) / 2.0,
        HAlign::Right => width - gw,
        _ => 0.0,
    }
    .max(0.0);

    let mut clicked = None;
    let mut text_rect = Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.add_space(indent + offset);
        let sense = if links.is_empty() { Sense::hover() } else { Sense::click() };
        let resp = ui.add(egui::Label::new(galley.clone()).selectable(true).sense(sense));
        text_rect = resp.rect;
        let galley_pos = match galley.job.halign {
            Align::Min => resp.rect.left_top(),
            Align::Center => resp.rect.center_top(),
            Align::Max => resp.rect.right_top(),
        };
        let link_at = |pos: Pos2| {
            let c = galley.cursor_from_pos(pos - galley_pos).index.0;
            links.iter().find(|(a, b, _)| c >= *a && c < *b).map(|(_, _, u)| u.clone())
        };
        if let Some(pos) = resp.hover_pos()
            && let Some(url) = link_at(pos)
        {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            resp.clone().on_hover_text(&url);
            if resp.clicked() {
                clicked = Some(url);
            }
        }
        // Émojis en couleur (style 3D), centrés sur leur emplacement dans le texte.
        for (i, len, e, size) in &emojis_at {
            let a = galley.pos_from_cursor(egui::text::CCursor::new(*i));
            let b = galley.pos_from_cursor(egui::text::CCursor::new(i + len));
            let cx = if (b.top() - a.top()).abs() < 1.0 { (a.left() + b.left()) / 2.0 } else { a.left() + size * 0.65 };
            let c = galley_pos + vec2(cx, a.center().y);
            let s = size * 1.18;
            crate::emoji_img::paint(ui.painter(), Rect::from_center_size(c, vec2(s, s)), e, text_color);
        }
        // Barres de citation.
        for q in 0..p.quote {
            let x = resp.rect.left() - indent - offset + q as f32 * 14.0 + 3.0;
            ui.painter().vline(x, resp.rect.y_range(), Stroke::new(2.0, theme::BORDER()));
        }
    });
    (clicked, text_rect)
}

/// Image à sa place dans le corps du mail (réduite à la largeur du panneau si besoin).
#[derive(Clone, Copy)]
struct ImageSpec {
    width: Option<Dim>,
    height: Option<Dim>,
    max_width: Option<Dim>,
    align: HAlign,
}

/// Taille d'affichage d'une image, comme dans Gmail : dimensions imposées par l'expéditeur
/// (pixels ou %), proportions conservées si une seule est donnée, `max-width` respecté,
/// et jamais plus large que le panneau de lecture.
fn image_size(natural: Vec2, spec: ImageSpec, avail: f32) -> Vec2 {
    let (w0, h0) = (natural.x.max(1.0), natural.y.max(1.0));
    let w = spec.width.map(|d| d.resolve(avail));
    let h = spec.height.and_then(|d| match d {
        Dim::Px(v) => Some(v),
        Dim::Percent(_) => None, // % de hauteur : ignoré, comme dans un mail
    });
    let (mut w, mut h) = match (w, h) {
        (Some(w), Some(h)) => (w, h),
        (Some(w), None) => (w, w * h0 / w0),
        (None, Some(h)) => (h * w0 / h0, h),
        (None, None) => (w0, h0),
    };
    let max = spec.max_width.map(|d| d.resolve(avail)).unwrap_or(avail).min(avail);
    if w > max {
        h *= max / w;
        w = max;
    }
    vec2(w, h)
}

/// Couleur imposée par l'expéditeur, rendue lisible sur le fond sombre :
/// le noir / gris très foncé (texte « normal » sur fond blanc) devient la couleur de texte,
/// les couleurs sombres sont éclaircies en gardant leur teinte.
fn readable(c: [u8; 3]) -> Color32 {
    let lum = (0.2126 * c[0] as f32 + 0.7152 * c[1] as f32 + 0.0722 * c[2] as f32) / 255.0;
    if lum < 0.18 && (c[0].max(c[1]).max(c[2]) as i32 - c[0].min(c[1]).min(c[2]) as i32) < 40 {
        return theme::TEXT(); // noir / anthracite
    }
    if lum < 0.35 {
        let k = 0.35 / lum.max(0.05);
        let l = |x: u8| ((x as f32 * k).min(255.0)) as u8;
        return Color32::from_rgb(l(c[0]).max(60), l(c[1]).max(60), l(c[2]).max(60));
    }
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Image à sa place dans le corps du mail. Renvoie `true` si elle a été cliquée (lien).
fn body_image(ui: &mut egui::Ui, state: Option<&inline::ImgState>, alt: &str, spec: ImageSpec, link: Option<&str>) -> bool {
    let avail = (ui.available_width() - 14.0).max(40.0);
    let layout = match spec.align {
        HAlign::Center => Layout::top_down(Align::Center),
        HAlign::Right => Layout::top_down(Align::Max),
        _ => Layout::top_down(Align::Min),
    };
    let mut clicked = false;
    ui.allocate_ui_with_layout(vec2(avail, 0.0), layout, |ui| match state {
        Some(inline::ImgState::Ready { image, texture: Some(tex) }) => {
            let size = image_size(vec2(image.size[0] as f32, image.size[1] as f32), spec, avail);
            let mut r = ui.add(egui::Image::new((tex.id(), size)).sense(Sense::click()));
            if let Some(url) = link {
                r = r.on_hover_text(url).on_hover_cursor(egui::CursorIcon::PointingHand);
                clicked = r.clicked();
            } else {
                // Image sans lien : en grand dans la visionneuse.
                r = r.on_hover_cursor(egui::CursorIcon::ZoomIn);
                if r.clicked() {
                    let name = if alt.trim().is_empty() { "Image".to_string() } else { alt.to_string() };
                    crate::lightbox::open(ui.ctx(), format!("mail:{:?}", tex.id()), tex.clone(), name, crate::lightbox::OpenWith::None);
                }
            }
        }
        Some(inline::ImgState::Loading) | Some(inline::ImgState::Ready { .. }) => {
            // Emplacement réservé à la taille annoncée, pour éviter les sauts de mise en page.
            let size = if spec.width.is_some() || spec.height.is_some() { image_size(vec2(320.0, 120.0), spec, avail) } else { vec2(avail.min(320.0), 120.0) };
            let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
            ui.painter().rect_filled(rect, CornerRadius::same(8), theme::SURFACE_HI());
            egui::Spinner::new().size(16.0).paint_at(ui, Rect::from_center_size(rect.center(), vec2(16.0, 16.0)));
        }
        _ => {
            let label = if alt.is_empty() { t!("[image non disponible]", "[image unavailable]").to_string() } else { tf!("[image : {alt}]", "[image: {alt}]") };
            ui.label(RichText::new(label).small().color(theme::TEXT_FAINT()));
        }
    });
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(w: Option<Dim>, h: Option<Dim>, mw: Option<Dim>) -> ImageSpec {
        ImageSpec { width: w, height: h, max_width: mw, align: HAlign::Left }
    }

    #[test]
    fn image_sizes_like_gmail() {
        let nat = vec2(1200.0, 600.0);
        // Taille naturelle, réduite à la largeur du panneau.
        assert_eq!(image_size(nat, spec(None, None, None), 800.0), vec2(800.0, 400.0));
        assert_eq!(image_size(vec2(200.0, 100.0), spec(None, None, None), 800.0), vec2(200.0, 100.0));
        // Largeur imposée : proportions conservées.
        assert_eq!(image_size(nat, spec(Some(Dim::Px(300.0)), None, None), 800.0), vec2(300.0, 150.0));
        // Largeur + hauteur imposées : respectées telles quelles.
        assert_eq!(image_size(nat, spec(Some(Dim::Px(300.0)), Some(Dim::Px(300.0)), None), 800.0), vec2(300.0, 300.0));
        // Pourcentage + max-width.
        assert_eq!(image_size(nat, spec(Some(Dim::Percent(100.0)), None, Some(Dim::Px(500.0))), 800.0), vec2(500.0, 250.0));
        assert_eq!(image_size(nat, spec(Some(Dim::Percent(50.0)), None, None), 800.0), vec2(400.0, 200.0));
    }
}

// --- Pièces jointes --------------------------------------------------------

fn attachment_chip(ui: &mut egui::Ui, a: &Attachment, atts: &attachments::Attachments, email: &str, actions: &mut Vec<Action>) -> Rect {
    let key = a.key();
    let ext = a.extension().to_uppercase();
    let mut text = egui::text::LayoutJob::default();
    if !ext.is_empty() {
        text.append(&format!("{ext}  "), 0.0, egui::TextFormat::simple(FontId::proportional(10.0), theme::TEXT_MUTED()));
    }
    text.append(&a.filename, 0.0, egui::TextFormat::simple(FontId::proportional(13.0), theme::TEXT()));
    let size = attachments::human_size(a.size);
    if !size.is_empty() {
        text.append(&format!("  {size}"), 0.0, egui::TextFormat::simple(FontId::proportional(11.0), theme::TEXT_FAINT()));
    }

    let resp = ui
        .add(egui::Button::new(text).fill(theme::SURFACE_HI()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(255)))
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(t!("Aperçu", "Preview"));
    if resp.clicked() {
        actions.push(Action::Attachment(a.clone()));
    }
    if atts.pending.contains(&key) {
        ui.add(egui::Spinner::new().size(12.0));
    }

    // Dernier recours : uniquement si l'aperçu et l'ouverture locale ont échoué.
    if let Some(reason) = atts.blocked.get(&key) {
        let pill = ui
            .add(egui::Button::new(RichText::new("Voir dans Gmail").small().color(theme::BG())).fill(theme::TEXT()).corner_radius(CornerRadius::same(255)))
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("{reason}\nOuvre le message dans Gmail pour visualiser le document."));
        if pill.clicked() {
            let _ = webbrowser::open(&attachments::gmail_link(email, &a.message_id));
        }
    }
    resp.rect
}

fn preview_modal(ui: &mut egui::Ui, mail: &mut MailState, actions: &mut Vec<Action>) {
    let Some(p) = &mut mail.attachments.preview else { return };
    let ctx = ui.ctx().clone();
    let screen = ctx.content_rect();
    let max = vec2(screen.width() * 0.8, screen.height() * 0.8);
    let texture = attachments::texture_for(&ctx, &mut p.content, &p.att.key());

    let resp = egui::Modal::new(egui::Id::unique("attachment_preview"))
        .backdrop_color(Color32::from_black_alpha(190))
        .frame(egui::Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(12)).inner_margin(egui::Margin::same(16)))
        .show(&ctx, |ui| {
            ui.set_max_size(max);
            ui.horizontal(|ui| {
                ui.label(RichText::new(&p.att.filename).strong());
                ui.label(RichText::new(attachments::human_size(p.att.size)).small().color(theme::TEXT_MUTED()));
                ui.add_space(24.0);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button(t!("Fermer", "Close")).clicked() {
                        actions.push(Action::ClosePreview);
                    }
                    if ui.button("Ouvrir avec l'application").clicked() {
                        actions.push(Action::AttachmentExternal(p.att.clone()));
                    }
                });
            });
            ui.add_space(10.0);
            match (&p.content, texture) {
                (Content::Image { image, .. }, Some(tex)) => {
                    let [w, h] = image.size;
                    let avail = vec2(max.x, max.y - 60.0);
                    let scale = (avail.x / w as f32).min(avail.y / h as f32).min(1.0);
                    ui.vertical_centered(|ui| {
                        let r = ui.add(egui::Image::new((tex.id(), vec2(w as f32 * scale, h as f32 * scale))).sense(Sense::click()));
                        if r.on_hover_text(t!("Agrandir", "Enlarge")).on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                            crate::lightbox::open(ui.ctx(), format!("mail-att:{:?}", tex.id()), tex.clone(), p.att.filename.clone(), crate::lightbox::OpenWith::None);
                        }
                    });
                }
                (Content::Text(t), _) => {
                    ui.set_min_width(max.x.min(760.0));
                    egui::ScrollArea::both().max_height(max.y - 60.0).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(t).monospace()).selectable(true));
                    });
                }
                _ => {
                    ui.set_min_size(vec2(320.0, 160.0));
                    ui.centered_and_justified(|ui| ui.add(egui::Spinner::new()));
                }
            }
        });
    if resp.should_close() {
        actions.push(Action::ClosePreview);
    }
}

// --- Petits éléments -------------------------------------------------------

fn chip(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new().stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(255)).inner_margin(egui::Margin::symmetric(10, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).small().color(theme::TEXT_MUTED()));
    });
}

fn toast(ui: &mut egui::Ui, mail: &mut MailState) {
    let Some(n) = &mail.notice else { return };
    let ttl = if n.error { 8.0 } else { 3.0 };
    if n.at.elapsed().as_secs_f32() > ttl {
        mail.notice = None;
        return;
    }
    ui.ctx().request_repaint_after(Duration::from_millis(250));
    let screen = ui.ctx().content_rect();
    egui::Area::new(egui::Id::unique("mail_toast")).order(egui::Order::Foreground).anchor(egui::Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0)).show(ui.ctx(), |ui| {
        ui.set_max_width((screen.width() * 0.4).max(260.0));
        egui::Frame::new()
            .fill(if n.error { theme::TEXT() } else { theme::SURFACE_HI() })
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(egui::Margin::symmetric(14, 10))
            .show(ui, |ui| {
                let c = if n.error { theme::BG() } else { theme::TEXT() };
                ui.add(egui::Label::new(RichText::new(&n.text).color(c)).wrap());
            });
    });
}

/// Texte sur une ligne, tronqué avec « … ».
pub(crate) fn one_line(p: &Painter, text: &str, font: FontId, color: Color32, max_w: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = LayoutJob::simple_singleline(text.replace('\n', " "), font, color);
    job.wrap = TextWrapping { max_width: max_w.max(10.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    p.layout_job(job)
}

/// Punaise (pleine ou contour), même gabarit que l'étoile.
pub(crate) fn draw_pin(p: &Painter, c: Pos2, r: f32, filled: bool, color: Color32) {
    let s = r / 7.0;
    let pt = |x: f32, y: f32| c + vec2(x * s, y * s);
    let stroke = Stroke::new(1.2, color);
    // Tête, corps évasé, collerette, puis pointe.
    let head = vec![pt(-3.0, -7.0), pt(3.0, -7.0), pt(2.0, -5.0), pt(-2.0, -5.0)];
    let body = vec![pt(-2.0, -5.0), pt(2.0, -5.0), pt(3.0, -1.0), pt(-3.0, -1.0)];
    let collar = vec![pt(-5.0, -1.0), pt(5.0, -1.0), pt(5.0, 1.0), pt(-5.0, 1.0)];
    for poly in [head, body, collar] {
        if filled {
            p.add(egui::Shape::convex_polygon(poly, color, Stroke::NONE));
        } else {
            p.add(egui::Shape::closed_line(poly, stroke));
        }
    }
    p.line_segment([pt(0.0, 1.0), pt(0.0, 7.5)], Stroke::new(1.4, color));
}

/// Étoile à 5 branches (pleine ou contour).
pub(crate) fn draw_star(p: &Painter, c: Pos2, r: f32, filled: bool, color: Color32) {
    let pts: Vec<Pos2> = (0..10)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / 5.0;
            let rad = if i % 2 == 0 { r } else { r * 0.45 };
            c + vec2(a.cos(), a.sin()) * rad
        })
        .collect();
    if filled {
        // Forme non convexe : pentagone central + 5 pointes.
        let inner: Vec<Pos2> = pts.iter().skip(1).step_by(2).copied().collect();
        p.add(egui::Shape::convex_polygon(inner.clone(), color, Stroke::NONE));
        for i in 0..5 {
            let tip = pts[i * 2];
            let a = pts[(i * 2 + 9) % 10];
            let b = pts[i * 2 + 1];
            p.add(egui::Shape::convex_polygon(vec![a, tip, b], color, Stroke::NONE));
        }
    } else {
        p.add(egui::Shape::closed_line(pts, Stroke::new(1.2, color)));
    }
}

// --- Dates -----------------------------------------------------------------

fn to_local(ms: i64) -> Option<chrono::DateTime<Local>> {
    Local.timestamp_millis_opt(ms).single()
}

pub(crate) fn short_date(ms: i64) -> String {
    let Some(d) = to_local(ms) else { return String::new() };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if d.year() == now.year() {
        format!("{} {}", d.day(), tabs::month_short(d.month()))
    } else {
        d.format("%d/%m/%Y").to_string()
    }
}

pub(crate) fn long_date(ms: i64) -> String {
    let Some(d) = to_local(ms) else { return String::new() };
    format!("{} {} {} {}, {}", tabs::weekday_name(d.weekday()), d.day(), tabs::month_name(d.month()), d.year(), d.format("%H:%M"))
}
