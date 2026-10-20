//! Interface de l'onglet Messages (Google Chat) : conversations | discussion | fil.

use std::time::Duration;

use chrono::{Datelike, Local, TimeZone};
use eframe::egui::text::{CCursor, CCursorRange};
use eframe::egui::text_edit::TextEditState;
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Key, Layout, Modifiers, Pos2, Rect, RichText, Sense, Stroke, vec2,
};

use super::{ChatState, PendingFile, draft_key, emoji, format};
use crate::google::chat::{ChatAttachment, ChatMessage, Space, SpaceKind};
use crate::google::{Account, compose};
use crate::liquid::{self, Liquid};
use crate::prefs::Prefs;
use crate::tabs::mail::contacts::Contacts;
use crate::tabs::mail::inline::ImgState;
use crate::tabs::mail::recipients;
use crate::tabs::mail::view::{draw_star, one_line, rich_para, short_date};
use crate::{tabs, theme, widgets};

const SIDE_W: f32 = 260.0;
const PEOPLE_W: f32 = 260.0;
const THREAD_W: f32 = 360.0;
#[allow(non_snake_case)]
fn SELECTED() -> Color32 {
    crate::theme::gray(0x2A)
}
const HOVER: Color32 = Color32::from_rgba_premultiplied(7, 7, 7, 7);
const AVATAR: f32 = 32.0;
const QUICK: [&str; 5] = ["👍", "❤️", "😂", "🎉", "👀"];

enum Action {
    Select(String),
    Older(String),
    Send(bool),
    React(ChatMessage, String),
    OpenThread(String),
    CloseThread,
    StartEdit(ChatMessage),
    SaveEdit,
    CancelEdit,
    AskDelete(Option<String>),
    Delete(ChatMessage),
    OpenAttachment(ChatAttachment),
    ViewImage(ChatAttachment),
    Image(ChatAttachment),
    Favorite(String),
    OpenDm(String),
    MailTo(String),
    ToggleSide,
    RetrySend(String),
    DiscardFailed(String),
    ReactPicker(ChatMessage, Pos2),
    MoveSpace(String, String),
    CreateGroup,
    TogglePeople,
    Mute(String),
    PickIcon(String),
    ClearIcon(String),
    PickFiles,
    RemoveFile(usize),
}

pub fn show(ui: &mut egui::Ui, chat: &mut ChatState, acc: &mut Account, prefs: &mut Prefs, book: &Contacts) {
    header(ui, chat);

    if acc.session().is_none() {
        setup_card(ui, "Google Chat", "Connecte ton compte Google pour retrouver tes conversations Google Chat.", |ui| {
            tabs::settings::google_account_form(ui, acc);
        });
        return;
    }
    if chat.need_consent {
        setup_card(
            ui,
            "Autoriser Google Chat",
            "WorkTable n'a pas encore l'autorisation de lire et d'envoyer tes messages Google Chat. \
             Reconnecte-toi pour l'accorder (une seule fois).",
            |ui| {
                if ui.button("Autoriser l'accès à Google Chat").clicked() {
                    chat.retry();
                    acc.connect();
                }
            },
        );
        return;
    }
    if chat.api_disabled || chat.app_missing {
        let text = if chat.app_missing {
            "L'API Google Chat est activée mais l'application Chat n'est pas configurée dans ton projet \
             Google Cloud (onglet « Configuration » : nom, avatar, description). Renseigne-la puis réessaie."
        } else {
            "Les API Google Chat et People doivent être activées dans ton projet Google Cloud. \
             Active-les (boutons ci-dessous), attends une minute, puis réessaie."
        };
        setup_card(ui, "Configurer Google Chat", text, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button("API Google Chat").clicked() {
                    let _ = webbrowser::open("https://console.cloud.google.com/apis/library/chat.googleapis.com");
                }
                if ui.button("Configuration de l'app Chat").clicked() {
                    let _ = webbrowser::open("https://console.cloud.google.com/apis/api/chat.googleapis.com/hangouts-chat");
                }
                if ui.button("API People").clicked() {
                    let _ = webbrowser::open("https://console.cloud.google.com/apis/library/people.googleapis.com");
                }
                if ui.button("Réessayer").clicked() {
                    chat.retry();
                }
            });
        });
        return;
    }

    let mut actions = Vec::new();
    prepare_images(chat, ui.ctx());
    react_picker(ui, chat, &mut actions);
    shortcuts(ui, chat, &mut actions);
    dropped_files(ui, chat);

    // Blocs « Groupes » et « Personnes » masquables, avec le rebond élastique du calendrier.
    let anim = |ui: &egui::Ui, salt: &str, on: bool| {
        ui.ctx().animate_bool_with_time_and_easing(egui::Id::unique(salt), on, 0.28, |t| crate::prefs::Curve::Elastic.apply(t))
    };
    let kg = anim(ui, "wt_chat_side", !prefs.chat_side_hidden);
    let kp = anim(ui, "wt_chat_people", !prefs.chat_people_hidden);
    let groups_w = if SIDE_W * kg < 24.0 { 0.0 } else { SIDE_W * kg };
    let people_w = if PEOPLE_W * kp < 24.0 { 0.0 } else { PEOPLE_W * kp };
    let thread_w = if chat.thread.is_some() { THREAD_W } else { 0.0 };
    let gap = ui.spacing().item_spacing.x;
    let used: f32 = [groups_w, people_w, thread_w].iter().filter(|w| **w > 0.0).map(|w| w + gap).sum();
    let center_w = (ui.available_width() - used).max(240.0);
    // Colonnes affichées, dans l'ordre : 0 groupes, 1 personnes, 2 discussion, 3 fil.
    let mut cols: Vec<(usize, f32)> = Vec::new();
    if groups_w > 0.0 {
        cols.push((0, groups_w));
    }
    if people_w > 0.0 {
        cols.push((1, people_w));
    }
    cols.push((2, center_w));
    if thread_w > 0.0 {
        cols.push((3, thread_w));
    }
    let fixed: Vec<f32> = cols.iter().take(cols.len() - 1).map(|c| c.1).collect();
    let acc_ref: &Account = acc;
    widgets::pane_columns(ui, &fixed, |i, ui| match cols[i].0 {
        0 => animated_column(ui, groups_w, SIDE_W, kg, |ui| groups_block(ui, chat, prefs, book, &mut actions)),
        1 => animated_column(ui, people_w, PEOPLE_W, kp, |ui| people_block(ui, chat, prefs, book, &mut actions)),
        2 => center(ui, chat, acc_ref, prefs, &mut actions),
        _ => thread_panel(ui, chat, &mut actions),
    });
    toast(ui, chat);
    if let Some(a) = image_viewer(ui.ctx(), chat) {
        chat.open_attachment(acc, &a);
    }

    for a in actions {
        match a {
            Action::Select(s) => chat.select(acc, &s),
            Action::Older(s) => chat.load_older(acc, &s),
            Action::Send(in_thread) => chat.send(acc, in_thread),
            Action::React(m, e) => chat.react(acc, &m, &e),
            Action::OpenThread(t) => chat.thread = Some(t),
            Action::CloseThread => chat.thread = None,
            Action::StartEdit(m) => chat.editing = Some((m.name.clone(), m.text.clone())),
            Action::SaveEdit => chat.save_edit(acc),
            Action::CancelEdit => chat.editing = None,
            Action::AskDelete(n) => ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::unique("wt_chat_confirm"), n);
            }),
            Action::Delete(m) => {
                ui.ctx().data_mut(|d| d.insert_temp::<Option<String>>(egui::Id::unique("wt_chat_confirm"), None));
                chat.delete(acc, &m);
            }
            Action::OpenAttachment(a) => chat.open_attachment(acc, &a),
            Action::ViewImage(a) => chat.viewer = Some((a, 1.0)),
            Action::Image(a) => chat.request_image(acc, &a),
            Action::Favorite(s) => {
                if !prefs.chat_favorites.contains(&s) {
                    prefs.chat_favorites.push(s);
                } else {
                    prefs.chat_favorites.retain(|x| *x != s);
                }
            }
            Action::OpenDm(e) => {
                chat.new_dm = None;
                chat.people_query.clear();
                chat.open_dm(acc, &e);
            }
            Action::MailTo(e) => chat.mail_to = Some(e),
            Action::ToggleSide => prefs.chat_side_hidden = !prefs.chat_side_hidden,
            Action::ReactPicker(m, pos) => {
                chat.emoji_query.clear();
                chat.react_picker = Some((m, pos));
            }
            Action::MoveSpace(space, sec) => chat.move_space(acc, &space, &sec),
            Action::CreateGroup => chat.create_group(acc),
            Action::RetrySend(n) => chat.retry_send(acc, &n),
            Action::DiscardFailed(n) => chat.discard_failed(&n),
            Action::TogglePeople => prefs.chat_people_hidden = !prefs.chat_people_hidden,
            Action::Mute(s) => {
                if prefs.chat_muted.contains(&s) {
                    prefs.chat_muted.retain(|x| *x != s);
                    chat.info("Notifications réactivées");
                } else {
                    prefs.chat_muted.push(s);
                    chat.info("Conversation rendue muette");
                }
            }
            Action::PickIcon(s) => {
                if let Some(path) = rfd::FileDialog::new().add_filter("Images", &["png", "jpg", "jpeg", "gif", "webp", "bmp"]).pick_file() {
                    chat.set_icon(acc, &s, &path);
                }
            }
            Action::ClearIcon(s) => chat.clear_icon(&s),
            Action::PickFiles => {
                if let Some(paths) = rfd::FileDialog::new().pick_files() {
                    for p in paths {
                        add_file(chat, &p);
                    }
                }
            }
            Action::RemoveFile(i) => {
                if i < chat.files.len() {
                    chat.files.remove(i);
                }
            }
        }
    }
}

/// Sélecteur complet de réactions, ouvert depuis la barre de survol d'un message.
fn react_picker(ui: &mut egui::Ui, chat: &mut ChatState, actions: &mut Vec<Action>) {
    let Some((m, pos)) = chat.react_picker.clone() else { return };
    let recent = chat.recent_emojis.clone();
    let (mut q, mut g) = (std::mem::take(&mut chat.emoji_query), chat.emoji_group);
    let mut picked = None;
    let area = egui::Area::new(egui::Id::unique("wt_chat_react_picker"))
        .order(egui::Order::Foreground)
        .fixed_pos(pos + vec2(-300.0, 4.0))
        .constrain(true)
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.label(RichText::new("RÉAGIR").small().strong().color(theme::TEXT_MUTED()));
                picked = emoji::picker(ui, &mut q, &mut g, &recent);
            });
        });
    chat.emoji_query = q;
    chat.emoji_group = g;
    let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p)));
    if let Some(e) = picked {
        remember_emoji(chat, &e);
        actions.push(Action::React(m, e));
        chat.react_picker = None;
    } else if outside || ui.input(|i| i.key_pressed(Key::Escape)) {
        chat.react_picker = None;
    }
}

/// Colonne qui s'anime en largeur : contenu à sa taille normale, rogné, en fondu.
fn animated_column(ui: &mut egui::Ui, w: f32, full: f32, k: f32, body: impl FnOnce(&mut egui::Ui)) {
    let inner = ui.available_rect_before_wrap();
    let margins = w - inner.width();
    let content = Rect::from_min_size(inner.min, vec2((full - margins).max(inner.width()), inner.height()));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(content).layout(*ui.layout()));
    child.set_clip_rect(inner.expand(2.0).intersect(ui.clip_rect()));
    child.multiply_opacity(k.clamp(0.0, 1.0));
    body(&mut child);
    ui.allocate_rect(inner, Sense::hover());
}

/// Boutons d'affichage des blocs Groupes / Personnes.
fn block_toggles(ui: &mut egui::Ui, prefs: &Prefs, actions: &mut Vec<Action>) {
    let g = if prefs.chat_side_hidden { "Groupes ›" } else { "‹ Groupes" };
    if ui.add(egui::Button::new(RichText::new(g).small()).selected(!prefs.chat_side_hidden)).on_hover_text("Afficher / masquer les groupes").clicked() {
        actions.push(Action::ToggleSide);
    }
    let p = if prefs.chat_people_hidden { "Personnes ›" } else { "‹ Personnes" };
    if ui.add(egui::Button::new(RichText::new(p).small()).selected(!prefs.chat_people_hidden)).on_hover_text("Afficher / masquer l'annuaire des personnes").clicked() {
        actions.push(Action::TogglePeople);
    }
}

fn header(ui: &mut egui::Ui, chat: &ChatState) {
    let n = chat.unread_count();
    widgets::page_header_with(ui, "Messages", "Google Chat", |ui| {
        if n > 0 {
            widgets::header_badge(ui, &format!("{n} non lu{}", if n > 1 { "s" } else { "" }), true);
        }
    });
}

fn setup_card(ui: &mut egui::Ui, title: &str, text: &str, body: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        // Aligné à gauche, comme le contenu des autres onglets.
        let w = 600.0_f32.min(ui.available_width());
        ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
            widgets::card(ui, title, 0.0, |ui| {
                ui.label(RichText::new(text).color(theme::TEXT_MUTED()));
                ui.add_space(12.0);
                body(ui);
            });
        });
    });
}

// --- Colonne des conversations ----------------------------------------------------------

/// Bloc « Groupes » : espaces et discussions de groupe, rangés selon les sections Google Chat.
fn groups_block(ui: &mut egui::Ui, chat: &mut ChatState, prefs: &Prefs, book: &Contacts, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(widgets::section_title("Groupes"));
        ui.label(RichText::new(chat.spaces.iter().filter(|s| !is_person(s)).count().to_string()).small().color(theme::TEXT_FAINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("+").on_hover_text("Créer un groupe").clicked() {
                chat.new_group = if chat.new_group.is_some() { None } else { Some(super::NewGroup { space: true, ..Default::default() }) };
            }
        });
    });
    ui.add_space(8.0);
    if chat.new_group.is_some() {
        new_group_form(ui, chat, book, actions);
    }
    ui.add(egui::TextEdit::singleline(&mut chat.query).hint_text("Rechercher un groupe").desired_width(f32::INFINITY));
    ui.add_space(10.0);

    if chat.spaces.is_empty() {
        if chat.loading {
            for r in [0.8, 0.6, 0.7, 0.5] {
                widgets::ghost_row(ui, r);
                ui.add_space(8.0);
            }
        } else {
            widgets::empty_state(ui, "Aucun groupe", "Tes espaces Google Chat apparaîtront ici");
        }
        return;
    }

    let q = crate::tabs::mail::contacts::fold(chat.query.trim());
    let matches = |chat: &ChatState, s: &Space| q.is_empty() || crate::tabs::mail::contacts::fold(&chat.title(s)).contains(&q);
    let fav = |s: &Space| prefs.chat_favorites.contains(&s.name);
    let groupish = |s: &Space| !is_person(s);
    let favs: Vec<Space> = chat.spaces.iter().filter(|s| groupish(s) && fav(s) && matches(chat, s)).cloned().collect();

    // Catégories : celles de Google Chat (noms et ordre de l'utilisateur) quand elles sont
    // disponibles ; sinon Discussions de groupe / Espaces. Messages privés : bloc « Personnes ».
    let mut groups: Vec<(String, String, Vec<Space>)> = Vec::new();
    if chat.sections.is_empty() {
        let g = chat.spaces.iter().filter(|s| s.kind == SpaceKind::Group && !fav(s) && matches(chat, s)).cloned().collect();
        let rooms = chat.spaces.iter().filter(|s| s.kind == SpaceKind::Space && !fav(s) && matches(chat, s)).cloned().collect();
        let apps = chat.spaces.iter().filter(|s| s.bot_dm && !fav(s) && matches(chat, s)).cloned().collect();
        groups.push(("group".into(), "Discussions de groupe".into(), g));
        groups.push(("rooms".into(), "Espaces".into(), rooms));
        groups.push(("apps".into(), "Applications".into(), apps));
    } else {
        let placed: std::collections::HashSet<&str> = chat.sections.iter().flat_map(|x| x.spaces.iter().map(String::as_str)).collect();
        for sec in &chat.sections {
            let mut list: Vec<Space> = chat
                .spaces
                .iter()
                .filter(|s| groupish(s) && !fav(s) && matches(chat, s))
                .filter(|s| {
                    sec.spaces.contains(&s.name)
                        // Conversations non rangées : dans la section par défaut correspondante.
                        || (!placed.contains(s.name.as_str())
                            && match sec.id.as_str() {
                                "default-direct-messages" => s.kind == SpaceKind::Group,
                                "default-spaces" => s.kind == SpaceKind::Space,
                                "default-apps" => s.bot_dm,
                                _ => false,
                            })
                })
                .cloned()
                .collect();
            list.sort_by_key(|s| -s.last_active);
            let title = if sec.id == "default-direct-messages" { "Discussions de groupe".to_string() } else { sec.title.clone() };
            groups.push((format!("sec_{}", sec.id), title, list));
        }
    }

    egui::ScrollArea::vertical().id_salt("chat_spaces").auto_shrink([false, false]).show(ui, |ui| {
        let mut hl = Liquid::begin(ui, "chat_nav");
        let mut first = true;
        if !favs.is_empty() {
            first = false;
            let open = section_header(ui, chat, "fav", "Favoris", &favs);
            liquid::collapse(ui, "chat_fav", open, |ui| {
                for s in &favs {
                    space_row(ui, chat, s, true, false, &mut hl, actions);
                }
            });
        }
        // Glisser-déposer d'un groupe d'une section à l'autre (sections Google Chat uniquement).
        let can_drag = !chat.sections.is_empty();
        let dragging = egui::DragAndDrop::payload::<DragSpace>(ui.ctx());
        let mut zones: Vec<(String, Rect)> = Vec::new();
        for (key, title, list) in &groups {
            // Section vide : affichée seulement pendant un glisser, comme cible.
            if list.is_empty() && !(dragging.is_some() && key.starts_with("sec_")) {
                continue;
            }
            if !first {
                section_rule(ui);
            }
            first = false;
            let top = ui.cursor().top();
            let open = section_header(ui, chat, key, title, list);
            liquid::collapse(ui, &format!("chat_{key}"), open, |ui| {
                for s in list {
                    space_row(ui, chat, s, false, can_drag, &mut hl, actions);
                }
            });
            if list.is_empty() {
                ui.add_space(24.0);
            }
            let zone = Rect::from_min_max(Pos2::new(ui.max_rect().left(), top - 4.0), Pos2::new(ui.max_rect().right(), ui.cursor().top() + 4.0));
            if let Some(id) = key.strip_prefix("sec_") {
                zones.push((id.to_string(), zone));
            }
        }
        hl.end(ui, SELECTED(), 8.0);
        if let Some(d) = dragging {
            let pos = ui.ctx().pointer_latest_pos();
            for (id, zone) in &zones {
                if pos.is_some_and(|p| zone.contains(p)) {
                    ui.painter().rect_stroke(*zone, CornerRadius::same(8), Stroke::new(1.5, theme::TODAY()), egui::StrokeKind::Inside);
                    if ui.input(|i| i.pointer.any_released()) {
                        let from = chat.sections.iter().find(|x| x.spaces.contains(&d.0)).map(|x| x.id.clone());
                        if from.as_deref() != Some(id.as_str()) {
                            actions.push(Action::MoveSpace(d.0.clone(), id.clone()));
                        }
                        egui::DragAndDrop::clear_payload(ui.ctx());
                    }
                }
            }
            // Étiquette qui suit le pointeur.
            if let (Some(p), Some(sp)) = (pos, chat.space(&d.0)) {
                let title = chat.title(sp);
                egui::Area::new(egui::Id::unique("wt_chat_drag_ghost")).order(egui::Order::Tooltip).fixed_pos(p + vec2(12.0, 8.0)).interactable(false).show(ui.ctx(), |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(RichText::new(format!("↕ {title}")).strong());
                    });
                });
            }
        }
        if let Some(e) = chat.sections_error.clone() {
            let low = e.to_lowercase();
            let hint = if low.contains("scope") || low.contains("insufficient") || low.contains("permission") {
                "Tes sections Google Chat n'ont pas pu être lues : reconnecte-toi (Réglages › Google) pour accorder l'accès aux sections."
            } else {
                "Tes sections Google Chat n'ont pas pu être lues (disposition par défaut affichée)."
            };
            section_rule(ui);
            ui.label(RichText::new(hint).small().color(theme::TEXT_MUTED()));
            ui.add(egui::Label::new(RichText::new(format!("Détail : {e}")).small().color(theme::TEXT_FAINT())).selectable(true).wrap());
            if ui.small_button("Réessayer").clicked() {
                chat.retry_sections();
            }
        }
    });
}

/// Création d'un groupe : espace nommé ou discussion de groupe, membres depuis l'annuaire.
fn new_group_form(ui: &mut egui::Ui, chat: &mut ChatState, book: &Contacts, actions: &mut Vec<Action>) {
    let mut cancel = false;
    egui::Frame::new().fill(theme::BG()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
        let Some(g) = chat.new_group.as_mut() else { return };
        ui.label(RichText::new("NOUVEAU GROUPE").small().strong().color(theme::TEXT_MUTED()));
        ui.horizontal(|ui| {
            ui.selectable_value(&mut g.space, true, "Espace").on_hover_text("Groupe nommé, ouvert à d'autres membres");
            ui.selectable_value(&mut g.space, false, "Discussion de groupe").on_hover_text("Conversation sans nom (2 personnes minimum)");
        });
        if g.space {
            ui.add(egui::TextEdit::singleline(&mut g.name).hint_text("Nom de l'espace").desired_width(f32::INFINITY));
        }
        recipients::field(ui, "", &mut g.members, "chat_new_group", "Membres (annuaire)", book, &mut chat.suggest);
        ui.horizontal(|ui| {
            if ui.button("Créer").clicked() {
                actions.push(Action::CreateGroup);
            }
            cancel = ui.button("Annuler").clicked();
        });
    });
    if cancel {
        chat.new_group = None;
    }
    ui.add_space(10.0);
}

/// Message privé avec une personne (pas une application).
fn is_person(s: &Space) -> bool {
    s.kind == SpaceKind::Dm && !s.bot_dm
}

/// Trait de séparation entre deux sections.
fn section_rule(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    ui.painter().hline(r.x_range().shrink(4.0), r.center().y, Stroke::new(1.0, theme::BORDER()));
    ui.add_space(8.0);
}

/// Bloc « Personnes » : grand annuaire de toutes les conversations privées ouvertes.
fn people_block(ui: &mut egui::Ui, chat: &mut ChatState, prefs: &Prefs, book: &Contacts, actions: &mut Vec<Action>) {
    let people: Vec<Space> = chat.spaces.iter().filter(|s| is_person(s)).cloned().collect();
    ui.horizontal(|ui| {
        ui.label(widgets::section_title("Personnes"));
        ui.label(RichText::new(people.len().to_string()).small().color(theme::TEXT_FAINT()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button("+").on_hover_text("Nouvelle conversation privée").clicked() {
                chat.new_dm = if chat.new_dm.is_some() { None } else { Some(String::new()) };
            }
        });
    });
    ui.add_space(8.0);
    if let Some(addr) = &mut chat.new_dm {
        recipients::field(ui, "", addr, "chat_new_dm", "Nom ou adresse (annuaire)", book, &mut chat.suggest);
        let email = crate::tabs::mail::contacts::parse_mailbox(addr.trim().trim_end_matches(','))
            .map(|(_, e)| e)
            .unwrap_or_else(|| addr.trim().trim_end_matches(',').to_string());
        let mut cancel = false;
        ui.horizontal(|ui| {
            if ui.add_enabled(email.contains('@'), egui::Button::new("Ouvrir")).clicked() {
                actions.push(Action::OpenDm(email.clone()));
            }
            cancel = ui.button("Annuler").clicked();
        });
        if cancel {
            chat.new_dm = None;
        }
        ui.add_space(8.0);
    }
    let search = ui.add(egui::TextEdit::singleline(&mut chat.people_query).hint_text("Rechercher une personne (nom ou adresse)").desired_width(f32::INFINITY));
    let enter = search.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter));
    ui.add_space(10.0);

    let q = crate::tabs::mail::contacts::fold(chat.people_query.trim());
    let matches = |chat: &ChatState, s: &Space| {
        q.is_empty()
            || crate::tabs::mail::contacts::fold(&chat.title(s)).contains(&q)
            || chat.others(&s.name).first().and_then(|u| chat.email_of(u)).is_some_and(|e| e.to_lowercase().contains(&q))
    };
    let recents: Vec<Space> = chat.recent_people().into_iter().filter(|s| matches(chat, s)).cloned().collect();
    let mut all: Vec<(String, Space)> = people.into_iter().filter(|s| matches(chat, s)).map(|s| (crate::tabs::mail::contacts::fold(&chat.title(&s)), s)).collect();
    all.sort_by(|a, b| a.0.cmp(&b.0));

    // Recherche : conversations existantes d'abord ; sinon contacts de l'annuaire avec qui en démarrer une.
    let mut starts: Vec<(String, String)> = Vec::new();
    if !q.is_empty() {
        let known: std::collections::HashSet<String> = chat
            .spaces
            .iter()
            .filter(|s| is_person(s))
            .filter_map(|s| chat.others(&s.name).first().and_then(|u| chat.email_of(u)))
            .map(|e| e.to_lowercase())
            .collect();
        for c in book.search(&chat.people_query, 8) {
            if !known.contains(&c.email.to_lowercase()) {
                starts.push((c.display_name().to_string(), c.email.clone()));
            }
        }
        let typed = chat.people_query.trim().to_lowercase();
        if typed.contains('@') && typed.contains('.') && !known.contains(&typed) && !starts.iter().any(|(_, e)| e.eq_ignore_ascii_case(&typed)) {
            starts.push((typed.clone(), typed));
        }
    }
    if enter {
        if let Some((_, s)) = all.first() {
            actions.push(Action::Select(s.name.clone()));
        } else if let Some((_, e)) = starts.first() {
            actions.push(Action::OpenDm(e.clone()));
        }
    }
    if all.is_empty() && starts.is_empty() {
        let hint = if !q.is_empty() { "Aucune conversation ni contact : tape une adresse complète" } else if chat.loading { "Chargement…" } else { "Ouvre une conversation avec « + »" };
        widgets::empty_state(ui, "Aucune personne", hint);
        return;
    }

    egui::ScrollArea::vertical().id_salt("chat_people").auto_shrink([false, false]).show(ui, |ui| {
        let mut hl = Liquid::begin(ui, "chat_people_nav");
        if !recents.is_empty() && q.is_empty() {
            let open = section_header(ui, chat, "recent", "Récents", &recents);
            liquid::collapse(ui, "chat_recent", open, |ui| {
                for s in &recents {
                    recent_row(ui, chat, s, actions);
                }
            });
            section_rule(ui);
        }
        if !starts.is_empty() {
            ui.label(RichText::new("NOUVELLE CONVERSATION").small().color(theme::TEXT_MUTED()));
            ui.add_space(4.0);
            for (name, email) in &starts {
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::click());
                let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Démarrer une conversation privée");
                let p = ui.painter();
                if resp.hovered() {
                    p.rect_filled(rect, CornerRadius::same(8), HOVER);
                }
                avatar(p, Pos2::new(rect.left() + 20.0, rect.center().y), 14.0, name, false);
                let g = one_line(p, name, theme::editor_font(13.0, false), theme::TEXT(), rect.width() - 80.0);
                p.galley(Pos2::new(rect.left() + 42.0, rect.top() + 4.0), g, theme::TEXT());
                let g = one_line(p, email, FontId::proportional(11.0), theme::TEXT_FAINT(), rect.width() - 80.0);
                p.galley(Pos2::new(rect.left() + 42.0, rect.top() + 22.0), g, theme::TEXT_FAINT());
                p.text(Pos2::new(rect.right() - 10.0, rect.center().y), Align2::RIGHT_CENTER, "Écrire ›", FontId::proportional(11.0), theme::LINK());
                if resp.clicked() {
                    actions.push(Action::OpenDm(email.clone()));
                }
            }
            if !all.is_empty() {
                section_rule(ui);
            }
        }
        if all.is_empty() {
            return;
        }
        let open = section_header(ui, chat, "people_all", if q.is_empty() { "Toutes les personnes" } else { "Conversations existantes" }, &all.iter().map(|(_, s)| s.clone()).collect::<Vec<_>>());
        liquid::collapse(ui, "chat_people_all", open, |ui| {
            let mut letter = None;
            for (key, s) in &all {
                let l = key.chars().next().map(|c| c.to_ascii_uppercase()).filter(|c| c.is_alphabetic()).unwrap_or('#');
                if letter != Some(l) {
                    letter = Some(l);
                    ui.add_space(4.0);
                    ui.label(RichText::new(l.to_string()).small().strong().color(theme::TEXT_FAINT()));
                }
                person_entry(ui, chat, s, prefs.chat_favorites.contains(&s.name), &mut hl, actions);
            }
        });
        hl.end(ui, SELECTED(), 8.0);
    });
}

/// En-tête de catégorie repliable ; renvoie `true` si elle est dépliée.
fn section_header(ui: &mut egui::Ui, chat: &ChatState, key: &str, title: &str, list: &[Space]) -> bool {
    let open_id = egui::Id::unique(("wt_chat_section", key));
    let open = ui.ctx().data(|d| d.get_temp::<bool>(open_id)).unwrap_or(true);
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let unread = list.iter().filter(|s| chat.unread(s) && !chat.is_muted(&s.name)).count();
    let label = if unread > 0 && !open { format!("{} · {unread}", title.to_uppercase()) } else { title.to_uppercase() };
    let k = liquid::animate(ui.ctx(), ("chat_section", key), open);
    chevron(ui.painter(), Pos2::new(r.left() + 5.0, r.center().y), k, theme::TEXT_MUTED());
    let g = one_line(ui.painter(), &label, FontId::proportional(11.0), theme::TEXT_MUTED(), r.width() - 20.0);
    ui.painter().galley(Pos2::new(r.left() + 16.0, r.center().y - g.size().y / 2.0), g, if resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() });
    if resp.clicked() {
        ui.ctx().data_mut(|d| d.insert_temp(open_id, !open));
    }
    open
}

/// Ligne compacte de « Récents » : photo et nom.
fn recent_row(ui: &mut egui::Ui, chat: &ChatState, s: &Space, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let active = chat.open.as_deref() == Some(s.name.as_str());
    let muted = chat.is_muted(&s.name);
    let unread = chat.unread(s) && !muted;
    let p = ui.painter();
    if active {
        p.rect_filled(rect, CornerRadius::same(6), crate::theme::overlay(10));
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), HOVER);
    }
    let title = chat.title(s);
    space_avatar(p, chat, Pos2::new(rect.left() + 16.0, rect.center().y), 11.0, s, &title);
    let color = if muted { theme::TEXT_FAINT() } else if unread || active { theme::TEXT() } else { crate::theme::gray(0xC8) };
    let g = one_line(p, &title, theme::editor_font(13.0, unread), color, rect.width() - 56.0);
    p.galley(Pos2::new(rect.left() + 34.0, rect.center().y - g.size().y / 2.0), g, color);
    status_mark(p, Pos2::new(rect.right() - 12.0, rect.center().y), unread, muted, false);
    if resp.clicked() {
        actions.push(Action::Select(s.name.clone()));
    }
}

/// Fiche de l'annuaire : photo, nom, adresse.
fn person_entry(ui: &mut egui::Ui, chat: &ChatState, s: &Space, fav: bool, hl: &mut Liquid, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 44.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let active = chat.open.as_deref() == Some(s.name.as_str());
    let muted = chat.is_muted(&s.name);
    let unread = chat.unread(s) && !muted;
    let p = ui.painter();
    if active {
        hl.target(rect, &s.name);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), HOVER);
    }
    let title = chat.title(s);
    space_avatar(p, chat, Pos2::new(rect.left() + 20.0, rect.center().y), 15.0, s, &title);
    let left = rect.left() + 42.0;
    let color = if muted { theme::TEXT_FAINT() } else if unread || active { theme::TEXT() } else { crate::theme::gray(0xD0) };
    let ng = one_line(p, &title, theme::editor_font(13.5, unread), color, rect.right() - left - 24.0);
    p.galley(Pos2::new(left, rect.top() + 6.0), ng, color);
    let email = chat.others(&s.name).first().and_then(|u| chat.email_of(u)).unwrap_or_default();
    let eg = one_line(p, &email, FontId::proportional(11.0), theme::TEXT_FAINT(), rect.right() - left - 24.0);
    p.galley(Pos2::new(left, rect.top() + 25.0), eg, theme::TEXT_FAINT());
    status_mark(p, Pos2::new(rect.right() - 12.0, rect.center().y), unread, muted, fav);
    if resp.clicked() {
        actions.push(Action::Select(s.name.clone()));
    }
    resp.context_menu(|ui| {
        ui.set_min_width(210.0);
        if ui.button(if muted { "Réactiver les notifications" } else { "Rendre muet" }).clicked() {
            actions.push(Action::Mute(s.name.clone()));
            ui.close();
        }
        if ui.button(if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }).clicked() {
            actions.push(Action::Favorite(s.name.clone()));
            ui.close();
        }
        if !email.is_empty() && ui.button("Écrire un mail").clicked() {
            actions.push(Action::MailTo(email.clone()));
            ui.close();
        }
        if ui.button("Ouvrir dans Google Chat").clicked() {
            let _ = webbrowser::open(&s.web_link());
            ui.close();
        }
    });
}

/// Pastille de non lus, cloche barrée (muet) ou étoile (favori).
fn status_mark(p: &egui::Painter, c: Pos2, unread: bool, muted: bool, fav: bool) {
    if muted {
        bell(p, c, 5.0, true, theme::TEXT_FAINT());
    } else if unread {
        p.circle_filled(c, 4.0, theme::TODAY());
    } else if fav {
        draw_star(p, c, 4.5, true, theme::TEXT_FAINT());
    }
}

/// Cloche (barrée si muet).
fn bell(p: &egui::Painter, c: Pos2, r: f32, crossed: bool, color: Color32) {
    let s = Stroke::new(1.3, color);
    let top = c + vec2(0.0, -r);
    let pts = vec![
        c + vec2(-r * 0.95, r * 0.55),
        c + vec2(-r * 0.7, r * 0.3),
        c + vec2(-r * 0.7, -r * 0.2),
        top + vec2(-r * 0.35, r * 0.15),
        top + vec2(r * 0.35, r * 0.15),
        c + vec2(r * 0.7, -r * 0.2),
        c + vec2(r * 0.7, r * 0.3),
        c + vec2(r * 0.95, r * 0.55),
    ];
    p.add(egui::Shape::closed_line(pts, s));
    p.circle_filled(c + vec2(0.0, r * 0.85), r * 0.18, color);
    if crossed {
        p.line_segment([c + vec2(-r, -r), c + vec2(r, r)], Stroke::new(1.5, color));
    }
}

fn chevron(p: &egui::Painter, c: Pos2, k: f32, color: Color32) {
    let a = k * std::f32::consts::FRAC_PI_2;
    let rot = |x: f32, y: f32| c + vec2(x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos());
    p.add(egui::Shape::line(vec![rot(-2.0, -4.0), rot(2.5, 0.0), rot(-2.0, 4.0)], Stroke::new(1.5, color)));
}

/// Groupe en cours de glisser (identifiant de l'espace).
struct DragSpace(String);

fn space_row(ui: &mut egui::Ui, chat: &ChatState, s: &Space, fav: bool, drag: bool, hl: &mut Liquid, actions: &mut Vec<Action>) {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 48.0), if drag { Sense::click_and_drag() } else { Sense::click() });
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if drag {
        resp.dnd_set_drag_payload(DragSpace(s.name.clone()));
        if resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
        }
    }
    let active = chat.open.as_deref() == Some(s.name.as_str());
    let muted = chat.is_muted(&s.name);
    let unread = chat.unread(s) && !muted;
    let p = ui.painter();
    if active {
        hl.target(rect, &s.name);
    } else if resp.hovered() {
        p.rect_filled(rect, CornerRadius::same(8), HOVER);
    }
    let title = chat.title(s);
    space_avatar(p, chat, Pos2::new(rect.left() + 22.0, rect.center().y), 15.0, s, &title);
    let left = rect.left() + 44.0;
    let time = if s.last_active > 0 { short_date(s.last_active) } else { String::new() };
    let tg = one_line(p, &time, FontId::proportional(10.5), if unread { theme::TODAY() } else { theme::TEXT_FAINT() }, 90.0);
    let tw = tg.size().x;
    p.galley(Pos2::new(rect.right() - 8.0 - tw, rect.top() + 8.0), tg, theme::TEXT_FAINT());
    let name_color = if muted { theme::TEXT_FAINT() } else if unread || active { theme::TEXT() } else { crate::theme::gray(0xC8) };
    let ng = one_line(p, &title, theme::editor_font(13.5, unread), name_color, rect.right() - left - tw - 16.0);
    p.galley(Pos2::new(left, rect.top() + 7.0), ng, name_color);
    // Aperçu : dernier message connu, sinon type de conversation.
    let preview = chat
        .msgs
        .get(&s.name)
        .and_then(|l| l.last())
        .map(|m| {
            let who = if Some(m.sender.as_str()) == chat.me.as_deref() { "Vous : ".to_string() } else { format!("{} : ", first_name(&chat.name_of(&m.sender))) };
            let body = if m.text.is_empty() && !m.attachments.is_empty() { "📎 Pièce jointe".to_string() } else { format::preview(&m.text, &|u| chat.name_of(u)) };
            format!("{who}{body}")
        })
        .unwrap_or_else(|| kind_label(s));
    let pg = one_line(p, &preview, FontId::proportional(11.5), if unread { crate::theme::gray(0xC8) } else { theme::TEXT_FAINT() }, rect.right() - left - 26.0);
    p.galley(Pos2::new(left, rect.top() + 27.0), pg, theme::TEXT_FAINT());
    status_mark(p, Pos2::new(rect.right() - 12.0, rect.top() + 33.0), unread, muted, fav);
    if resp.clicked() {
        actions.push(Action::Select(s.name.clone()));
    }
    let has_icon = chat.icons.contains_key(&s.name);
    resp.context_menu(|ui| {
        ui.set_min_width(210.0);
        if ui.button(if muted { "Réactiver les notifications" } else { "Rendre muet" }).clicked() {
            actions.push(Action::Mute(s.name.clone()));
            ui.close();
        }
        if ui.button(if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }).clicked() {
            actions.push(Action::Favorite(s.name.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button("Choisir une image…").clicked() {
            actions.push(Action::PickIcon(s.name.clone()));
            ui.close();
        }
        if has_icon && ui.button("Retirer l'image").clicked() {
            actions.push(Action::ClearIcon(s.name.clone()));
            ui.close();
        }
        ui.separator();
        if ui.button("Ouvrir dans Google Chat").clicked() {
            let _ = webbrowser::open(&s.web_link());
            ui.close();
        }
    });
}

fn kind_label(s: &Space) -> String {
    match s.kind {
        SpaceKind::Dm => "Message privé".into(),
        SpaceKind::Group => "Discussion de groupe".into(),
        SpaceKind::Space if s.members > 0 => format!("Espace · {} membres", s.members),
        SpaceKind::Space => "Espace".into(),
    }
}

fn first_name(n: &str) -> &str {
    n.split_whitespace().next().unwrap_or(n)
}

/// Image d'une conversation : image choisie / photo de l'interlocuteur ; pour une discussion
/// de groupe sans image, les photos de deux membres superposées ; sinon les initiales.
fn space_avatar(p: &egui::Painter, chat: &ChatState, c: Pos2, r: f32, s: &Space, title: &str) {
    let square = s.kind == SpaceKind::Space;
    if let Some(key) = chat.space_image_key(s) {
        if image_avatar(p, chat, &key, c, r, square) {
            return;
        }
    }
    if s.kind == SpaceKind::Group {
        let others = chat.others(&s.name);
        if others.len() >= 2 {
            let small = r * 0.68;
            user_avatar(p, chat, c + vec2(-r * 0.32, -r * 0.32), small, &others[0]);
            p.circle_filled(c + vec2(r * 0.32, r * 0.32), small + 1.5, theme::SURFACE());
            user_avatar(p, chat, c + vec2(r * 0.32, r * 0.32), small, &others[1]);
            return;
        }
    }
    avatar(p, c, r, title, square);
}

/// Photo d'un utilisateur, sinon ses initiales.
fn user_avatar(p: &egui::Painter, chat: &ChatState, c: Pos2, r: f32, user: &str) {
    if let Some(key) = chat.photo_key(user) {
        if image_avatar(p, chat, &key, c, r, false) {
            return;
        }
    }
    avatar(p, c, r, &chat.name_of(user), false);
}

/// Dessine une image recadrée au centre, ronde (ou carrée arrondie). `false` si pas encore chargée.
fn image_avatar(p: &egui::Painter, chat: &ChatState, key: &str, c: Pos2, r: f32, square: bool) -> bool {
    let Some(ImgState::Ready { texture: Some(t), .. }) = chat.images.peek(key) else { return false };
    let [w, h] = t.size();
    let (w, h) = (w as f32, h as f32);
    let uv = if w > h {
        let d = (1.0 - h / w) / 2.0;
        Rect::from_min_max(egui::pos2(d, 0.0), egui::pos2(1.0 - d, 1.0))
    } else {
        let d = (1.0 - w / h) / 2.0;
        Rect::from_min_max(egui::pos2(0.0, d), egui::pos2(1.0, 1.0 - d))
    };
    let rect = Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r));
    let radius = if square { CornerRadius::same((r * 0.55) as u8) } else { CornerRadius::same(r.min(255.0) as u8) };
    p.add(egui::epaint::RectShape::filled(rect, radius, Color32::WHITE).with_texture(t.id(), uv));
    true
}

/// Pastille ronde avec initiales, couleur dérivée du nom (carrée arrondie pour un espace).
fn avatar(p: &egui::Painter, c: Pos2, r: f32, name: &str, square: bool) {
    const PALETTE: [[u8; 3]; 8] = [
        [0x5B, 0x8D, 0xEF], [0xD0, 0x6A, 0x4E], [0x4C, 0xAF, 0x7A], [0xA3, 0x6A, 0xD6],
        [0xE0, 0xA4, 0x3A], [0x3F, 0xA7, 0xB5], [0xC9, 0x5B, 0x8F], [0x7D, 0x8A, 0x99],
    ];
    let h = name.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    let [cr, cg, cb] = PALETTE[(h % PALETTE.len() as u32) as usize];
    let col = Color32::from_rgb(cr, cg, cb);
    if square {
        p.rect_filled(Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)), CornerRadius::same((r * 0.55) as u8), col);
    } else {
        p.circle_filled(c, r, col);
    }
    let initials: String = name
        .split(|ch: char| ch.is_whitespace() || ch == ',')
        .filter(|w| !w.is_empty())
        .take(2)
        .filter_map(|w| w.chars().next())
        .flat_map(|ch| ch.to_uppercase())
        .collect();
    p.text(c, Align2::CENTER_CENTER, if initials.is_empty() { "?".into() } else { initials }, theme::editor_font(r * 0.8, true), Color32::WHITE);
}

// --- Discussion ---------------------------------------------------------------------------

fn center(ui: &mut egui::Ui, chat: &mut ChatState, acc: &Account, prefs: &Prefs, actions: &mut Vec<Action>) {
    let Some(space) = chat.open.clone().and_then(|o| chat.space(&o).cloned()) else {
        ui.horizontal(|ui| block_toggles(ui, prefs, actions));
        ui.add_space(ui.available_height() * 0.38);
        widgets::empty_state(ui, "Aucune conversation ouverte", "Choisis une conversation à gauche");
        return;
    };
    let title = chat.title(&space);
    let fav = prefs.chat_favorites.contains(&space.name);
    let muted = prefs.chat_muted.contains(&space.name);

    ui.horizontal(|ui| block_toggles(ui, prefs, actions));
    ui.add_space(6.0);
    // En-tête de la conversation.
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
        space_avatar(ui.painter(), chat, r.center(), 16.0, &space, &title);
        ui.vertical(|ui| {
            ui.label(RichText::new(&title).font(theme::title_font(16.0)).color(theme::TEXT()));
            let mut sub = kind_label(&space);
            if space.kind == SpaceKind::Dm {
                if let Some(e) = chat.others(&space.name).first().and_then(|u| chat.email_of(u)) {
                    sub = e;
                }
            }
            ui.label(RichText::new(sub).small().color(theme::TEXT_MUTED()));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add(egui::Button::new(RichText::new("Ouvrir dans Google Chat ↗").small()).corner_radius(CornerRadius::same(255))).clicked() {
                let _ = webbrowser::open(&space.web_link());
            }
            if space.kind == SpaceKind::Dm {
                if let Some(e) = chat.others(&space.name).first().and_then(|u| chat.email_of(u)) {
                    if ui.button("Écrire un mail").on_hover_text(format!("Nouveau mail à {e}")).clicked() {
                        actions.push(Action::MailTo(e));
                    }
                }
            }
            let (r, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
            let resp = resp.on_hover_text(if muted { "Réactiver les notifications" } else { "Rendre muet" }).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), theme::SURFACE_HI());
            }
            bell(ui.painter(), r.center(), 6.5, muted, if muted || resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() });
            if resp.clicked() {
                actions.push(Action::Mute(space.name.clone()));
            }
            let (r, resp) = ui.allocate_exact_size(vec2(28.0, 26.0), Sense::click());
            let resp = resp.on_hover_text(if fav { "Retirer des favoris" } else { "Ajouter aux favoris" }).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(r, CornerRadius::same(6), theme::SURFACE_HI());
            }
            draw_star(ui.painter(), r.center(), 8.0, fav, if fav || resp.hovered() { theme::TEXT() } else { theme::TEXT_MUTED() });
            if resp.clicked() {
                actions.push(Action::Favorite(space.name.clone()));
            }
            if chat.loading_msgs.contains(&space.name) {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
    });
    ui.add_space(6.0);
    ui.separator();

    // Compositeur en bas (hauteur de l'image précédente), messages au-dessus.
    let comp_id = egui::Id::unique("wt_chat_composer_h");
    let comp_h: f32 = ui.ctx().data(|d| d.get_temp(comp_id)).unwrap_or(110.0);
    let list_h = (ui.available_height() - comp_h - 8.0).max(120.0);
    let msgs: Vec<ChatMessage> = chat.msgs.get(&space.name).cloned().unwrap_or_default();
    let scroll_bottom = std::mem::take(&mut chat.scroll_bottom);
    egui::ScrollArea::vertical()
        .id_salt(("chat_msgs", &space.name))
        .max_height(list_h)
        .auto_shrink([false, false])
        .stick_to_bottom(true)
        .show(ui, |ui| {
            ui.set_min_height(list_h);
            if chat.has_more.get(&space.name).copied().unwrap_or(false) {
                ui.vertical_centered(|ui| {
                    if chat.loading_msgs.contains(&space.name) {
                        ui.add(egui::Spinner::new().size(14.0));
                    } else if ui.small_button("Afficher les messages plus anciens").clicked() {
                        actions.push(Action::Older(space.name.clone()));
                    }
                });
                ui.add_space(8.0);
            }
            if msgs.is_empty() {
                if chat.loading_msgs.contains(&space.name) {
                    for r in [0.6, 0.8, 0.5] {
                        widgets::ghost_row(ui, r);
                        ui.add_space(10.0);
                    }
                } else {
                    ui.add_space(30.0);
                    widgets::empty_state(ui, "Aucun message", "Écris le premier message ci-dessous");
                }
            }
            let top: Vec<&ChatMessage> = msgs.iter().filter(|m| !(space.threaded && m.thread_reply)).collect();
            let mut prev: Option<&ChatMessage> = None;
            for m in &top {
                let new_day = prev.is_none_or(|p| day_of(p.created) != day_of(m.created));
                if new_day {
                    day_separator(ui, m.created);
                }
                let compact = !new_day && prev.is_some_and(|p| p.sender == m.sender && m.created - p.created < 5 * 60_000 && !p.thread_reply);
                let replies: Vec<&ChatMessage> = if space.threaded { msgs.iter().filter(|r| r.thread_reply && r.thread == m.thread && !m.thread.is_empty()).collect() } else { Vec::new() };
                message(ui, chat, m, compact, space.threaded, &replies, actions);
                prev = Some(m);
            }
            if scroll_bottom {
                ui.scroll_to_cursor(Some(Align::Max));
            }
        });

    // Compositeur.
    let start = ui.cursor().top();
    ui.add_space(6.0);
    composer(ui, chat, &space, &title, false, actions);
    let h = ui.cursor().top() - start;
    ui.ctx().data_mut(|d| d.insert_temp(comp_id, h));
    let _ = acc;
}

fn day_of(ms: i64) -> chrono::NaiveDate {
    Local.timestamp_millis_opt(ms).single().map(|d| d.date_naive()).unwrap_or_default()
}

fn day_separator(ui: &mut egui::Ui, ms: i64) {
    let d = day_of(ms);
    let today = Local::now().date_naive();
    let label = if d == today {
        "Aujourd'hui".to_string()
    } else if Some(d) == today.pred_opt() {
        "Hier".to_string()
    } else {
        let mut s = format!("{} {} {}", tabs::weekday_fr(d.weekday()), d.day(), tabs::month_fr(d.month()));
        if d.year() != today.year() {
            s.push_str(&format!(" {}", d.year()));
        }
        s
    };
    ui.add_space(10.0);
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width() - 12.0, 18.0), Sense::hover());
    let p = ui.painter();
    let g = p.layout_no_wrap(label, FontId::proportional(11.0), theme::TEXT_MUTED());
    let gw = g.size().x + 16.0;
    p.hline(r.left()..=r.center().x - gw / 2.0, r.center().y, Stroke::new(1.0, theme::BORDER()));
    p.hline(r.center().x + gw / 2.0..=r.right(), r.center().y, Stroke::new(1.0, theme::BORDER()));
    p.galley(r.center() - g.size() / 2.0, g, theme::TEXT_MUTED());
    ui.add_space(6.0);
}

fn hhmm(ms: i64) -> String {
    Local.timestamp_millis_opt(ms).single().map(|d| d.format("%H:%M").to_string()).unwrap_or_default()
}

/// Un message : avatar + nom (sauf messages groupés), texte mis en forme, pièces jointes,
/// réactions, résumé du fil ; barre d'actions au survol.
fn message(ui: &mut egui::Ui, chat: &ChatState, m: &ChatMessage, compact: bool, threaded: bool, replies: &[&ChatMessage], actions: &mut Vec<Action>) {
    let name = chat.name_of(&m.sender);
    let mine = chat.me.as_deref() == Some(m.sender.as_str());
    let editing = chat.editing.as_ref().filter(|(n, _)| *n == m.name).map(|(_, t)| t.clone());
    let confirm: Option<String> = ui.ctx().data(|d| d.get_temp::<Option<String>>(egui::Id::unique("wt_chat_confirm"))).flatten();
    let sending = m.name.starts_with("local/");
    let failed = m.name.starts_with("failed/");
    if sending {
        ui.multiply_opacity(0.55);
    }
    // Messages qui se suivent du même auteur, sans réaction : lignes rapprochées.
    let tight = compact && m.reactions.is_empty();
    let spacing_before = ui.spacing().item_spacing.y;
    // Espacement vertical géré explicitement (les messages d'une même personne se suivent de près).
    ui.spacing_mut().item_spacing.y = 0.0;
    let top = ui.cursor().top();
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    ui.add_space(if compact { 0.0 } else { 10.0 });
    let left = ui.cursor().left();
    if !compact {
        let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        ui.add_space(4.0);
        let c = Pos2::new(r.left() + AVATAR / 2.0 + 4.0, r.top() + AVATAR / 2.0);
        if m.bot {
            avatar(ui.painter(), c, AVATAR / 2.0, &name, true);
        } else {
            user_avatar(ui.painter(), chat, c, AVATAR / 2.0, &m.sender);
        }
        let p = ui.painter();
        let ng = p.layout_no_wrap(name.clone(), theme::editor_font(13.5, true), theme::TEXT());
        let nw = ng.size().x;
        p.galley(Pos2::new(r.left() + AVATAR + 14.0, r.top() + 1.0), ng, theme::TEXT());
        let mut meta = hhmm(m.created);
        if m.bot {
            meta.push_str("  · Application");
        }
        p.text(Pos2::new(r.left() + AVATAR + 22.0 + nw, r.top() + 3.0), Align2::LEFT_TOP, meta, FontId::proportional(11.0), theme::TEXT_FAINT());
    }
    let body_left = left + AVATAR + 14.0;
    let width = (ui.available_width() - AVATAR - 26.0).max(80.0);
    let body_rect = Rect::from_min_size(Pos2::new(body_left, ui.cursor().top()), vec2(width, 100_000.0));
    let _body = ui.scope_builder(egui::UiBuilder::new().max_rect(body_rect).layout(Layout::top_down(Align::Min)), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing.y = if tight { 2.0 } else { 4.0 };
        if let Some(q) = &m.quote {
            ui.add(egui::Label::new(RichText::new(format!("❝ {}", q.chars().take(160).collect::<String>())).small().italics().color(theme::TEXT_MUTED())).wrap());
        }
        if let Some(mut text) = editing {
            let r = ui.add(egui::TextEdit::multiline(&mut text).desired_rows(2).desired_width(f32::INFINITY));
            if r.changed() {
                actions.push(Action::StartEdit(ChatMessage { text: text.clone(), ..m.clone() }));
            }
            ui.horizontal(|ui| {
                if ui.button("Enregistrer").clicked() {
                    actions.push(Action::SaveEdit);
                }
                if ui.button("Annuler").clicked() {
                    actions.push(Action::CancelEdit);
                }
                ui.label(RichText::new("Entrée pour enregistrer · Échap pour annuler").small().color(theme::TEXT_FAINT()));
            });
            if r.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) && !i.modifiers.shift) {
                actions.push(Action::SaveEdit);
            }
        } else if let Some(big) = emoji_only(&m.text) {
            // Message composé uniquement d'émojis : affichés en grand, comme sur iOS / Google Chat.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for e in big {
                    let (r, _) = ui.allocate_exact_size(vec2(40.0, 40.0), Sense::hover());
                    crate::emoji_img::paint(ui.painter(), r, &e, theme::TEXT());
                }
            });
        } else if !m.text.is_empty() {
            for p in format::to_paras(&m.text, &|u| chat.name_of(u)) {
                if let (Some(url), _) = rich_para(ui, &p) {
                    let _ = webbrowser::open(&url);
                }
            }
            if m.edited {
                ui.label(RichText::new("(modifié)").small().color(theme::TEXT_FAINT()));
            }
        }
        attachments(ui, chat, m, actions);
        if !m.reactions.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                for (e, n) in &m.reactions {
                    // Pastille : émoji couleur + nombre.
                    let count = ui.painter().layout_no_wrap(n.to_string(), FontId::proportional(12.0), theme::TEXT());
                    let (r, resp) = ui.allocate_exact_size(vec2(36.0 + count.size().x, 24.0), Sense::click());
                    let resp = resp.on_hover_text("Ajouter / retirer ma réaction").on_hover_cursor(egui::CursorIcon::PointingHand);
                    let p = ui.painter();
                    p.rect(r, CornerRadius::same(12), if resp.hovered() { theme::SURFACE_HI() } else { theme::SURFACE() }, Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
                    crate::emoji_img::paint(p, Rect::from_center_size(Pos2::new(r.left() + 16.0, r.center().y), vec2(17.0, 17.0)), e, theme::TEXT());
                    p.galley(Pos2::new(r.left() + 28.0, r.center().y - count.size().y / 2.0), count, theme::TEXT());
                    if resp.clicked() {
                        actions.push(Action::React(m.clone(), e.clone()));
                    }
                }
            });
        }
        if threaded && !replies.is_empty() {
            let last = replies.last().map(|r| hhmm(r.created)).unwrap_or_default();
            let txt = format!("{} réponse{} · dernière à {last}", replies.len(), if replies.len() > 1 { "s" } else { "" });
            if ui.add(egui::Button::new(RichText::new(txt).small().color(theme::LINK())).frame(false)).clicked() {
                actions.push(Action::OpenThread(m.thread.clone()));
            }
        }
        if sending {
            ui.label(RichText::new("Envoi…").small().italics().color(theme::TEXT_MUTED()));
        }
        if failed {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Échec de l'envoi").small().color(theme::TODAY()));
                if ui.small_button("Réessayer").clicked() {
                    actions.push(Action::RetrySend(m.name.clone()));
                }
                if ui.small_button("Supprimer").clicked() {
                    actions.push(Action::DiscardFailed(m.name.clone()));
                }
            });
        }
        if confirm.as_deref() == Some(m.name.as_str()) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Supprimer ce message ?").small().color(theme::TODAY()));
                if ui.small_button("Supprimer").clicked() {
                    actions.push(Action::Delete(m.clone()));
                }
                if ui.small_button("Annuler").clicked() {
                    actions.push(Action::AskDelete(None));
                }
            });
        }
    });
    let min_bottom = top + if tight { 16.0 } else if compact { 20.0 } else { 40.0 };
    if ui.cursor().top() < min_bottom {
        ui.add_space(min_bottom - ui.cursor().top());
    }
    ui.add_space(if tight { 1.0 } else { 2.0 });
    ui.spacing_mut().item_spacing.y = spacing_before;
    let bottom = ui.cursor().top() - 2.0;
    let rect = Rect::from_min_max(Pos2::new(left, top), Pos2::new(ui.max_rect().right() - 12.0, bottom + 2.0));

    // Survol : fond léger + barre d'actions (réactions rapides, fil, modifier, supprimer).
    if ui.rect_contains_pointer(rect) && chat.editing.is_none() {
        ui.painter().set(bg_idx, egui::Shape::rect_filled(rect, CornerRadius::same(6), crate::theme::overlay(5)));
        if compact {
            ui.painter().text(Pos2::new(left + AVATAR / 2.0 + 4.0, top + 10.0), Align2::CENTER_CENTER, hhmm(m.created), FontId::proportional(9.5), theme::TEXT_FAINT());
        }
        let mut items: Vec<(&str, &str)> = QUICK.iter().map(|e| (*e, "Réagir")).collect();
        items.push(("＋", "Autres réactions"));
        if threaded && !m.thread_reply {
            items.push(("↩", "Répondre dans le fil"));
        }
        if mine {
            items.push(("✎", "Modifier"));
            items.push(("🗑", "Supprimer"));
        }
        items.push(("⧉", "Copier le texte"));
        let bw = 26.0;
        let bar = Rect::from_min_size(Pos2::new(rect.right() - items.len() as f32 * bw - 8.0, rect.top() - 10.0), vec2(items.len() as f32 * bw + 8.0, 28.0));
        ui.painter().rect(bar, CornerRadius::same(8), theme::SURFACE(), Stroke::new(1.0, theme::BORDER()), egui::StrokeKind::Inside);
        for (i, (icon, tip)) in items.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(bar.left() + 4.0 + i as f32 * bw, bar.top() + 2.0), vec2(bw, 24.0));
            let resp = ui.interact(r, ui.scope_id().with(("chat_bar", &m.name, i)), Sense::click()).on_hover_text(*tip).on_hover_cursor(egui::CursorIcon::PointingHand);
            if resp.hovered() {
                ui.painter().rect_filled(r.shrink(1.0), CornerRadius::same(6), theme::SURFACE_HI());
            }
            if *tip == "Réagir" {
                crate::emoji_img::paint(ui.painter(), Rect::from_center_size(r.center(), vec2(17.0, 17.0)), icon, theme::TEXT());
            } else {
                ui.painter().text(r.center(), Align2::CENTER_CENTER, *icon, FontId::proportional(14.0), theme::TEXT());
            }
            if resp.clicked() {
                match *tip {
                    "Réagir" => actions.push(Action::React(m.clone(), icon.to_string())),
                    "Autres réactions" => actions.push(Action::ReactPicker(m.clone(), r.left_bottom())),
                    "Répondre dans le fil" => actions.push(Action::OpenThread(m.thread.clone())),
                    "Modifier" => actions.push(Action::StartEdit(m.clone())),
                    "Supprimer" => actions.push(Action::AskDelete(Some(m.name.clone()))),
                    _ => ui.ctx().copy_text(m.text.clone()),
                }
            }
        }
    }
    if sending {
        ui.multiply_opacity(1.0 / 0.55);
    }
}

/// Émojis d'un message qui n'en contient que quelques-uns (sinon `None`).
fn emoji_only(text: &str) -> Option<Vec<String>> {
    let t = text.trim();
    if t.is_empty() {
        return None;
    }
    let found = crate::emoji_img::find(t);
    let covered: usize = found.iter().map(|(_, n, _)| n).sum();
    let other = t.chars().count() - covered;
    let spaces = t.chars().filter(|c| c.is_whitespace()).count();
    (!found.is_empty() && found.len() <= 5 && other == spaces).then(|| found.into_iter().map(|(_, _, e)| e).collect())
}

fn attachments(ui: &mut egui::Ui, chat: &ChatState, m: &ChatMessage, actions: &mut Vec<Action>) {
    for a in &m.attachments {
        if a.is_image() {
            let key = a.key();
            let ctx = ui.ctx().clone();
            // `get` crée la texture au premier affichage.
            let state = texture_of(chat, &ctx, &key);
            match state {
                Some((tex, size)) => {
                    let max = vec2(320.0_f32.min(ui.available_width()), 240.0);
                    let scale = (max.x / size.x).min(max.y / size.y).min(1.0);
                    let r = ui.add(egui::Image::new((tex, size * scale)).corner_radius(CornerRadius::same(8)).sense(Sense::click()));
                    if r.on_hover_text(format!("{} — agrandir", a.name)).on_hover_cursor(egui::CursorIcon::ZoomIn).clicked() {
                        actions.push(Action::ViewImage(a.clone()));
                    }
                }
                None => {
                    actions.push(Action::Image(a.clone()));
                    file_chip(ui, a, actions);
                }
            }
        } else {
            file_chip(ui, a, actions);
        }
    }
}

/// Texture d'une image déjà chargée (les textures sont créées par `prepare_images`).
fn texture_of(chat: &ChatState, _ctx: &egui::Context, key: &str) -> Option<(egui::TextureId, egui::Vec2)> {
    chat.images.peek(key).and_then(|s| match s {
        ImgState::Ready { texture: Some(t), .. } => Some((t.id(), t.size_vec2())),
        _ => None,
    })
}

fn file_chip(ui: &mut egui::Ui, a: &ChatAttachment, actions: &mut Vec<Action>) {
    let icon = if a.drive_id.is_some() { "Drive" } else { "Fichier" };
    let text = RichText::new(format!("📎 {}", a.name)).color(theme::LINK()).underline();
    let r = ui.add(egui::Button::new(text).frame(false)).on_hover_text(format!("{icon} — ouvrir avec l'application par défaut"));
    if r.clicked() {
        actions.push(Action::OpenAttachment(a.clone()));
    }
}

// --- Compositeur ------------------------------------------------------------------------

fn composer(ui: &mut egui::Ui, chat: &mut ChatState, space: &Space, title: &str, in_thread: bool, actions: &mut Vec<Action>) {
    let key = draft_key(&space.name, if in_thread { chat.thread.as_deref() } else { None });
    let id = egui::Id::unique(("wt_chat_input", &key));
    if !in_thread && !chat.files.is_empty() {
        ui.horizontal_wrapped(|ui| {
            for (i, f) in chat.files.iter().enumerate() {
                egui::Frame::new().fill(theme::SURFACE_HI()).corner_radius(CornerRadius::same(255)).inner_margin(egui::Margin::symmetric(10, 3)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("📎 {}", f.name)).small());
                        if ui.small_button("×").on_hover_text("Retirer").clicked() {
                            actions.push(Action::RemoveFile(i));
                        }
                    });
                });
            }
        });
        ui.add_space(4.0);
    }
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let mut text = chat.drafts.get(&key).cloned().unwrap_or_default();
    let before = text.clone();

    // Suggestions d'émojis pendant la frappe d'un code « :nom » (Tab ou clic pour insérer).
    let pending = emoji::pending_code(&text);
    let sugg = pending.as_ref().map(|(_, p)| emoji::suggestions(ui.ctx(), p, 8)).unwrap_or_default();
    if focused && !sugg.is_empty() {
        let mut chosen = None;
        if ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Tab)) {
            chosen = Some(sugg[0].as_str().to_string());
        }
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for e in &sugg {
                let label = format!("{} :{}:", e.as_str(), e.shortcode().unwrap_or(e.name()));
                if ui.add(egui::Button::new(RichText::new(label).small()).corner_radius(CornerRadius::same(255))).clicked() {
                    chosen = Some(e.as_str().to_string());
                }
            }
            ui.label(RichText::new("Tab pour insérer").small().color(theme::TEXT_FAINT()));
        });
        if let (Some(e), Some((at, _))) = (chosen, &pending) {
            text.truncate(*at);
            text.push_str(&e);
            text.push(' ');
            remember_emoji(chat, &e);
            set_cursor_end(ui.ctx(), id, &text);
        }
    }

    // Raccourcis de mise en forme (avant le champ, qui ne doit pas les recevoir).
    if focused {
        let shortcut = ui.input_mut(|i| {
            if i.consume_key(Modifiers::COMMAND, Key::B) {
                Some(("*", "*"))
            } else if i.consume_key(Modifiers::COMMAND, Key::I) {
                Some(("_", "_"))
            } else if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::X) {
                Some(("~", "~"))
            } else if i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::C) {
                Some(("`", "`"))
            } else if i.consume_key(Modifiers::COMMAND, Key::K) {
                Some(("<https://|", ">"))
            } else {
                None
            }
        });
        if let Some((a, b)) = shortcut {
            wrap_with(ui.ctx(), id, &mut text, a, b);
        }
    }

    let mut send = false;
    egui::Frame::new()
        .fill(theme::BG())
        .stroke(Stroke::new(1.0, theme::BORDER()))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            // Barre d'outils : mise en forme Google Chat (*gras*, _italique_, ~barré~, `code`, lien).
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for (label, a, b, tip) in [
                    ("B", "*", "*", "Gras (Ctrl+B)"),
                    ("I", "_", "_", "Italique (Ctrl+I)"),
                    ("S", "~", "~", "Barré (Ctrl+Maj+X)"),
                    ("</>", "`", "`", "Code (Ctrl+Maj+C)"),
                    ("🔗", "<https://|", ">", "Lien (Ctrl+K) : <adresse|texte>"),
                ] {
                    let rt = match label {
                        "B" => RichText::new(label).strong(),
                        "I" => RichText::new(label).italics(),
                        "S" => RichText::new(label).strikethrough(),
                        _ => RichText::new(label).monospace(),
                    };
                    if ui.add(egui::Button::new(rt).frame(false).min_size(vec2(24.0, 22.0))).on_hover_text(tip).clicked() {
                        wrap_with(ui.ctx(), id, &mut text, a, b);
                    }
                }
                let r = ui.add(egui::Button::new(RichText::new("☺").size(16.0)).frame(false)).on_hover_text("Émojis (ou tape « :nom: »)");
                if r.clicked() {
                    chat.composer_picker = if chat.composer_picker.is_some() { None } else { Some((key.clone(), r.rect.left_top())) };
                }
                if !in_thread && ui.add(egui::Button::new("📎").frame(false)).on_hover_text("Joindre des fichiers (ou glisse-les sur la fenêtre)").clicked() {
                    actions.push(Action::PickFiles);
                }
            });
            // Entrée = envoyer, Maj+Entrée = nouvelle ligne.
            if focused && ui.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) {
                send = true;
            }
            ui.horizontal(|ui| {
                let send_w = 84.0;
                let hint = if in_thread { "Répondre dans le fil".to_string() } else { format!("Message à {title}") };
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .id(id)
                        .hint_text(hint)
                        .desired_rows(1)
                        .frame(egui::Frame::NONE)
                        .desired_width(ui.available_width() - send_w),
                );
                let can = (!text.trim().is_empty() || (!in_thread && !chat.files.is_empty())) && !chat.sending;
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if chat.sending {
                        ui.add(egui::Spinner::new().size(14.0));
                    } else if ui.add_enabled(can, egui::Button::new("Envoyer")).on_hover_text("Entrée").clicked() {
                        send = true;
                    }
                });
            });
        });
    // Sélecteur d'émojis du champ (reste ouvert pour changer de catégorie).
    if let Some((k, pos)) = chat.composer_picker.clone().filter(|(k, _)| *k == key) {
        let recent = chat.recent_emojis.clone();
        let (mut q, mut g) = (std::mem::take(&mut chat.emoji_query), chat.emoji_group);
        let mut picked = None;
        let area = egui::Area::new(egui::Id::unique(("wt_chat_emoji_picker", &k)))
            .order(egui::Order::Foreground)
            .pivot(Align2::LEFT_BOTTOM)
            .fixed_pos(pos - vec2(0.0, 6.0))
            .constrain(true)
            .show(ui.ctx(), |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    picked = emoji::picker(ui, &mut q, &mut g, &recent);
                });
            });
        chat.emoji_query = q;
        chat.emoji_group = g;
        let outside = ui.input(|i| i.pointer.any_pressed() && i.pointer.interact_pos().is_some_and(|p| !area.response.rect.contains(p) && !Rect::from_min_size(pos, vec2(30.0, 30.0)).contains(p)));
        if let Some(e) = picked {
            insert_at_cursor(ui.ctx(), id, &mut text, &e);
            remember_emoji(chat, &e);
            chat.composer_picker = None;
        } else if outside || ui.input(|i| i.key_pressed(Key::Escape)) {
            chat.composer_picker = None;
        }
    }
    // « :nom: » complet → émoji, comme dans Google Chat.
    if text != before {
        if let Some(t) = emoji::replace_codes(&text) {
            text = t;
            set_cursor_end(ui.ctx(), id, &text);
        }
    }
    chat.drafts.insert(key, text);
    if send {
        actions.push(Action::Send(in_thread));
    }
    ui.label(
        RichText::new("Entrée : envoyer · Maj+Entrée : à la ligne · Ctrl+B / Ctrl+I / Ctrl+Maj+X / Ctrl+K · **Markdown** accepté · :nom: pour un émoji")
            .small()
            .color(theme::TEXT_FAINT()),
    );
}

fn remember_emoji(chat: &mut ChatState, e: &str) {
    emoji::remember(e);
    chat.recent_emojis = emoji::recent();
}

fn set_cursor_end(ctx: &egui::Context, id: egui::Id, text: &str) {
    let mut st = TextEditState::load(ctx, id).unwrap_or_default();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(text.chars().count()))));
    st.store(ctx, id);
    ctx.memory_mut(|m| m.request_focus(id));
}

/// Entoure la sélection avec deux marqueurs (identiques pour *gras*, différents pour un lien).
fn wrap_with(ctx: &egui::Context, id: egui::Id, text: &mut String, open: &str, close: &str) {
    let mut st = TextEditState::load(ctx, id).unwrap_or_default();
    let n = text.chars().count();
    let (a, b) = st
        .cursor
        .char_range()
        .map(|r| {
            let (x, y) = (r.primary.index.0.min(n), r.secondary.index.0.min(n));
            (x.min(y), x.max(y))
        })
        .unwrap_or((n, n));
    let byte = |i: usize| text.char_indices().nth(i).map(|(b, _)| b).unwrap_or(text.len());
    let (ba, bb) = (byte(a), byte(b));
    let mid = text[ba..bb].to_string();
    text.replace_range(ba..bb, &format!("{open}{mid}{close}"));
    let m = open.chars().count();
    st.cursor.set_char_range(Some(CCursorRange::two(CCursor::new(a + m), CCursor::new(b + m))));
    st.store(ctx, id);
    ctx.memory_mut(|mem| mem.request_focus(id));
}

fn insert_at_cursor(ctx: &egui::Context, id: egui::Id, text: &mut String, s: &str) {
    let mut st = TextEditState::load(ctx, id).unwrap_or_default();
    let n = text.chars().count();
    let at = st.cursor.char_range().map(|r| r.primary.index.0.min(n)).unwrap_or(n);
    let b = text.char_indices().nth(at).map(|(b, _)| b).unwrap_or(text.len());
    text.insert_str(b, s);
    let c = at + s.chars().count();
    st.cursor.set_char_range(Some(CCursorRange::one(CCursor::new(c))));
    st.store(ctx, id);
    ctx.memory_mut(|mem| mem.request_focus(id));
}

// --- Fil de discussion --------------------------------------------------------------------

fn thread_panel(ui: &mut egui::Ui, chat: &mut ChatState, actions: &mut Vec<Action>) {
    let (Some(space), Some(thread)) = (chat.open.clone().and_then(|o| chat.space(&o).cloned()), chat.thread.clone()) else { return };
    ui.horizontal(|ui| {
        ui.label(RichText::new("FIL DE DISCUSSION").small().strong().color(theme::TEXT_MUTED()));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("✕").on_hover_text("Fermer (Échap)").clicked() {
                actions.push(Action::CloseThread);
            }
        });
    });
    ui.separator();
    let msgs: Vec<ChatMessage> = chat.msgs.get(&space.name).map(|l| l.iter().filter(|m| m.thread == thread).cloned().collect()).unwrap_or_default();
    let comp_id = egui::Id::unique("wt_chat_thread_composer_h");
    let comp_h: f32 = ui.ctx().data(|d| d.get_temp(comp_id)).unwrap_or(110.0);
    let list_h = (ui.available_height() - comp_h - 8.0).max(100.0);
    egui::ScrollArea::vertical().id_salt(("chat_thread", &thread)).max_height(list_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
        ui.set_min_height(list_h);
        let mut prev: Option<&ChatMessage> = None;
        for m in &msgs {
            let compact = prev.is_some_and(|p| p.sender == m.sender && m.created - p.created < 5 * 60_000);
            message(ui, chat, m, compact, false, &[], actions);
            if prev.is_none() && msgs.len() > 1 {
                ui.add_space(4.0);
                ui.label(RichText::new(format!("{} réponse{}", msgs.len() - 1, if msgs.len() > 2 { "s" } else { "" })).small().color(theme::TEXT_MUTED()));
                ui.separator();
            }
            prev = Some(m);
        }
    });
    let start = ui.cursor().top();
    ui.add_space(6.0);
    let title = chat.title(&space);
    composer(ui, chat, &space, &title, true, actions);
    let h = ui.cursor().top() - start;
    ui.ctx().data_mut(|d| d.insert_temp(comp_id, h));
}

// --- Divers -------------------------------------------------------------------------------

fn shortcuts(ui: &mut egui::Ui, chat: &mut ChatState, actions: &mut Vec<Action>) {
    if ui.input(|i| i.key_pressed(Key::Escape)) {
        if chat.editing.is_some() {
            actions.push(Action::CancelEdit);
        } else if chat.thread.is_some() {
            actions.push(Action::CloseThread);
        }
    }
    // Alt+↑ / Alt+↓ : conversation précédente / suivante.
    let (up, down) = ui.input(|i| (i.modifiers.alt && i.key_pressed(Key::ArrowUp), i.modifiers.alt && i.key_pressed(Key::ArrowDown)));
    if up || down {
        let pos = chat.open.as_ref().and_then(|o| chat.spaces.iter().position(|s| s.name == *o));
        let next = match (pos, down) {
            (None, _) => 0,
            (Some(p), true) => (p + 1).min(chat.spaces.len().saturating_sub(1)),
            (Some(p), false) => p.saturating_sub(1),
        };
        if let Some(s) = chat.spaces.get(next) {
            actions.push(Action::Select(s.name.clone()));
        }
    }
}

fn dropped_files(ui: &mut egui::Ui, chat: &mut ChatState) {
    if chat.open.is_none() {
        return;
    }
    let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
    for f in dropped {
        let p = f.path().to_path_buf();
        if !p.as_os_str().is_empty() {
            add_file(chat, &p);
        }
    }
}

fn add_file(chat: &mut ChatState, path: &std::path::Path) {
    match std::fs::read(path) {
        Ok(data) if data.len() <= 200 * 1024 * 1024 => {
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "fichier".into());
            let mime = compose::mime_for(&name).to_string();
            chat.files.push(PendingFile { name, mime, data });
        }
        Ok(_) => chat.info("Fichier trop volumineux (200 Mo maximum)"),
        Err(e) => chat.info(&format!("Lecture impossible : {e}")),
    }
}

fn toast(ui: &mut egui::Ui, chat: &mut ChatState) {
    let Some((text, error, at)) = chat.notice.clone() else { return };
    if at.elapsed() > Duration::from_secs(if error { 8 } else { 3 }) {
        chat.notice = None;
        return;
    }
    egui::Area::new(egui::Id::unique("wt_chat_toast"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-24.0, -24.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::new()
                .fill(if error { Color32::from_rgb(0x5A, 0x22, 0x22) } else { theme::SURFACE_HI() })
                .corner_radius(CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 8))
                .show(ui, |ui| {
                    ui.set_max_width(360.0);
                    ui.label(RichText::new(text).color(theme::TEXT()));
                });
        });
    ui.ctx().request_repaint_after(Duration::from_millis(300));
}

/// Crée les textures des images reçues (appelé avant l'affichage, avec accès mutable).
pub fn prepare_images(chat: &mut ChatState, ctx: &egui::Context) {
    let keys: Vec<String> = chat
        .msgs
        .get(chat.open.as_deref().unwrap_or_default())
        .map(|l| l.iter().flat_map(|m| m.attachments.iter()).filter(|a| a.is_image()).map(|a| a.key()).collect())
        .unwrap_or_default();
    let photos: Vec<String> = chat.people.keys().map(|u| format!("photo:{u}")).chain(chat.icons.keys().map(|s| format!("icon:{s}"))).collect();
    for k in keys.into_iter().chain(photos) {
        let _ = chat.images.get(ctx, &k);
    }
}

/// Visionneuse : l'image en grand par-dessus l'app (molette ou +/- pour zoomer, glisser pour
/// déplacer, Échap / clic à côté pour fermer). Renvoie la pièce jointe à ouvrir dans une application.
fn image_viewer(ctx: &egui::Context, chat: &mut ChatState) -> Option<ChatAttachment> {
    let (a, mut zoom) = chat.viewer.clone()?;
    let Some((tex, size)) = texture_of(chat, ctx, &a.key()) else {
        chat.viewer = None;
        return None;
    };
    let screen = ctx.content_rect();
    let mut close = ctx.input(|i| i.key_pressed(Key::Escape));
    let mut open = None;
    let pan_id = egui::Id::unique("wt_chat_viewer_pan");
    let mut pan: egui::Vec2 = ctx.data(|d| d.get_temp(pan_id)).unwrap_or_default();
    egui::Area::new(egui::Id::unique("wt_chat_viewer")).order(egui::Order::Foreground).fixed_pos(screen.min).show(ctx, |ui| {
        let (bg, bg_resp) = ui.allocate_exact_size(screen.size(), Sense::click());
        ui.painter().rect_filled(bg, CornerRadius::ZERO, Color32::from_black_alpha(215));
        // Taille d'origine réduite pour tenir dans la fenêtre, puis zoom.
        let avail = screen.size() - vec2(80.0, 140.0);
        let fit = (avail.x / size.x).min(avail.y / size.y).min(1.0);
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            zoom = (zoom * (1.0 + scroll * 0.002)).clamp(0.2, 8.0);
        }
        if ui.input(|i| i.key_pressed(Key::Plus) || i.key_pressed(Key::Equals)) {
            zoom = (zoom * 1.25).min(8.0);
        }
        if ui.input(|i| i.key_pressed(Key::Minus)) {
            zoom = (zoom / 1.25).max(0.2);
        }
        if ui.input(|i| i.key_pressed(Key::Num0)) {
            zoom = 1.0;
            pan = egui::Vec2::ZERO;
        }
        let shown = size * fit * zoom;
        let img = Rect::from_center_size(screen.center() - vec2(0.0, 20.0) + pan, shown);
        let img_resp = ui.interact(img, egui::Id::unique("wt_chat_viewer_img"), Sense::click_and_drag());
        if img_resp.dragged() {
            pan += img_resp.drag_delta();
        }
        if img_resp.double_clicked() {
            zoom = if zoom > 1.01 { 1.0 } else { 2.0 };
            pan = egui::Vec2::ZERO;
        }
        ui.painter().image(tex, img, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
        // Barre du bas : nom, zoom, actions.
        let bar = Rect::from_center_size(Pos2::new(screen.center().x, screen.bottom() - 44.0), vec2(520.0_f32.min(screen.width() - 40.0), 44.0));
        ui.scope_builder(egui::UiBuilder::new().max_rect(bar), |ui| {
            egui::Frame::new().fill(theme::SURFACE()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(255)).inner_margin(egui::Margin::symmetric(14, 6)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(&a.name).color(theme::TEXT())).truncate());
                    ui.label(RichText::new(format!("{:.0} %", fit * zoom * 100.0)).small().color(theme::TEXT_MUTED()));
                    if ui.small_button("−").on_hover_text("Dézoomer (−)").clicked() {
                        zoom = (zoom / 1.25).max(0.2);
                    }
                    if ui.small_button("+").on_hover_text("Zoomer (+ ou molette)").clicked() {
                        zoom = (zoom * 1.25).min(8.0);
                    }
                    if ui.small_button("Ouvrir").on_hover_text("Ouvrir avec l'application par défaut").clicked() {
                        open = Some(a.clone());
                    }
                    if ui.small_button("✕").on_hover_text("Fermer (Échap)").clicked() {
                        close = true;
                    }
                });
            });
        });
        if bg_resp.clicked() && !img_resp.hovered() {
            close = true;
        }
    });
    ctx.data_mut(|d| d.insert_temp(pan_id, pan));
    if close {
        chat.viewer = None;
        ctx.data_mut(|d| d.remove::<egui::Vec2>(pan_id));
    } else {
        chat.viewer = Some((a, zoom));
    }
    open
}
