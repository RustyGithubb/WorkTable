//! Réglages de l'application. Seul le compte Google est fonctionnel pour l'instant.

use eframe::egui::{self, RichText};

use crate::google::{Account, Status};
use crate::liquid::Liquid;
use crate::prefs::{Animation, Curve, Prefs};
use crate::{theme, widgets};

pub fn show(ui: &mut egui::Ui, acc: &mut Account, prefs: &mut Prefs) {
    widgets::page_header(ui, t!("Application", "Application"), t!("Réglages", "Settings"));
    egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let width = ui.available_width().min(760.0);
        ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
            general(ui, prefs);
            data_card(ui);
            themes(ui, prefs);
            fonts(ui, prefs);
            animations(ui, prefs);
            group_colors(ui, prefs);
            widgets::card(ui, t!("Compte Google", "Google account"), 0.0, |ui| {
                ui.label(
                    RichText::new(t!("Gmail (Calendar, Docs et Sheets utiliseront ce même compte)", "Gmail (Calendar, Docs and Sheets use this same account)"))
                        .small()
                        .color(theme::TEXT_FAINT()),
                );
                ui.add_space(10.0);
                google_account_form(ui, acc);
            });
            ui.add_space(12.0);
            notifications(ui, prefs);
            section(ui, t!("À propos", "About"), &[("Version", "WorkTable", Value::Text(env!("CARGO_PKG_VERSION")))]);
        });
    });
}

/// Langue de l'interface et onglet au démarrage.
fn general(ui: &mut egui::Ui, prefs: &mut Prefs) {
    use crate::i18n::Language;
    widgets::card(ui, t!("Général", "General"), 0.0, |ui| {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(t!("Langue", "Language"));
                ui.label(RichText::new(t!("Langue de l'interface et des notifications", "Language of the interface and notifications")).small().color(theme::TEXT_FAINT()));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                egui::ComboBox::from_id_salt("wt_language").width(180.0).selected_text(prefs.language.label()).show_ui(ui, |ui| {
                    for l in Language::ALL {
                        ui.selectable_value(&mut prefs.language, l, l.label());
                    }
                });
            });
        });
        ui.add_space(4.0);
        ui.separator();
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(t!("Onglet au démarrage", "Startup tab"));
                ui.label(RichText::new(t!("Onglet ouvert au lancement", "Tab opened at launch")).small().color(theme::TEXT_FAINT()));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(crate::app::Tab::Dashboard.label()).color(theme::TEXT_MUTED()));
            });
        });
    });
    ui.add_space(12.0);
}

enum Value {
    Text(&'static str),
    #[allow(dead_code)]
    Button(&'static str),
}

/// Formulaire de connexion / état du compte Google (utilisé ici et dans l'onglet Mail).
pub fn google_account_form(ui: &mut egui::Ui, acc: &mut Account) {
    match acc.status.clone() {
        Status::Connected(email) => {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(t!("Connecté", "Connected"));
                    ui.label(RichText::new(email).small().color(theme::TEXT_MUTED()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t!("Déconnecter", "Sign out")).clicked() {
                        acc.disconnect();
                        // Les conversations gardées sur cet ordinateur appartiennent à ce compte.
                        crate::tabs::messages::store::clear();
                    }
                });
            });
        }
        Status::Connecting => {
            ui.horizontal(|ui| {
                ui.add(egui::Spinner::new().size(14.0));
                ui.label(t!("Connexion en cours…", "Signing in…"));
            });
            ui.label(
                RichText::new(t!(
                    "Termine l'autorisation dans la fenêtre du navigateur qui vient de s'ouvrir.",
                    "Finish the authorization in the browser window that just opened."
                ))
                .small()
                .color(theme::TEXT_MUTED()),
            );
        }
        Status::Disconnected | Status::Failed(_) => {
            widgets::field(ui, "Client ID", 96.0, egui::TextEdit::singleline(&mut acc.config.client_id).hint_text("xxxx.apps.googleusercontent.com"));
            widgets::field(ui, "Client secret", 96.0, egui::TextEdit::singleline(&mut acc.config.client_secret).password(true));
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                let ready = acc.config.is_complete();
                if ui.add_enabled(ready, egui::Button::new(t!("Connecter Gmail", "Connect Gmail"))).clicked() {
                    acc.connect();
                }
                ui.label(
                    RichText::new(t!("Identifiants OAuth : voir README › Configurer Google", "OAuth credentials: see README › Configure Google"))
                        .small()
                        .color(theme::TEXT_FAINT()),
                );
            });
            if let Status::Failed(e) = &acc.status {
                ui.add_space(6.0);
                ui.add(egui::Label::new(RichText::new(e).small().color(theme::TEXT())).wrap());
            }
        }
    }
}

fn section(ui: &mut egui::Ui, title: &str, rows: &[(&str, &str, Value)]) {
    widgets::card(ui, title, 0.0, |ui| {
        for (i, (label, hint, value)) in rows.iter().enumerate() {
            if i > 0 {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(*label);
                    ui.label(RichText::new(*hint).small().color(theme::TEXT_FAINT()));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match value {
                    Value::Text(t) => {
                        ui.label(RichText::new(*t).color(theme::TEXT_MUTED()));
                    }
                    Value::Button(t) => {
                        ui.add_enabled(false, egui::Button::new(*t));
                    }
                });
            });
        }
    });
    ui.add_space(12.0);
}

/// Où sont enregistrées les données de l'utilisateur (selon le mode de lancement).
fn data_card(ui: &mut egui::Ui) {
    use crate::paths::{self, Mode};
    widgets::card(ui, t!("Données", "Data"), 0.0, |ui| {
        let mode = match paths::mode() {
            Mode::Installed => t!("Application installée : données dans votre profil Windows.", "Installed app: data stored in your Windows profile."),
            Mode::Portable => {
                t!("Version portable : données dans le dossier « data » à côté du programme.", "Portable version: data stored in the “data” folder next to the program.")
            }
            Mode::Development => t!("Développement : données dans le dossier « data » du projet.", "Development: data stored in the project's “data” folder."),
            Mode::Forced => t!("Dossier choisi par la variable WORKTABLE_DATA.", "Folder set by the WORKTABLE_DATA variable."),
        };
        ui.label(RichText::new(mode).small().color(theme::TEXT_FAINT()));
        if let Some(n) = ui.ctx().data(|d| d.get_temp::<usize>(egui::Id::unique("wt_import_result"))) {
            ui.label(
                RichText::new(tf!("{n} fichier(s) importé(s) — redémarrez l'app pour les prendre en compte.", "{n} file(s) imported — restart the app to use them."))
                    .small()
                    .color(theme::TODAY()),
            );
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(paths::root().display().to_string()).monospace().color(theme::TEXT_MUTED())).truncate());
            if ui.button(t!("Ouvrir le dossier", "Open folder")).clicked() {
                let _ = std::fs::create_dir_all(paths::root());
                let _ = open::that_detached(paths::root());
            }
            if ui
                .button(t!("Importer…", "Import…"))
                .on_hover_text(t!(
                    "Reprendre les données d'un autre dossier « data » (version portable, autre poste…). Redémarrez ensuite l'app.",
                    "Bring in data from another “data” folder (portable version, other computer…). Then restart the app."
                ))
                .clicked()
                && let Some(dir) = rfd::FileDialog::new().set_title(t!("Dossier « data » à importer", "“data” folder to import")).pick_folder()
            {
                let n = paths::import_from(&dir);
                let id = egui::Id::unique("wt_import_result");
                ui.ctx().data_mut(|d| d.insert_temp(id, n));
            }
        });
        // Copie locale chiffrée des conversations Google Chat récentes.
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        use crate::tabs::messages::store;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(t!("Conversations hors ligne", "Offline conversations"));
                let size = store::size().map(|b| crate::tabs::mail::attachments::human_size(b.max(1)));
                let text = match size {
                    Some(s) => tf!(
                        "Les {} conversations Google Chat les plus récentes, chiffrées sur cet ordinateur ({s}).",
                        "The {} most recent Google Chat conversations, encrypted on this computer ({s}).",
                        store::KEEP_SPACES
                    ),
                    None => tf!(
                        "Les {} conversations Google Chat les plus récentes seront gardées ici, chiffrées.",
                        "The {} most recent Google Chat conversations will be kept here, encrypted.",
                        store::KEEP_SPACES
                    ),
                };
                ui.label(RichText::new(text).small().color(theme::TEXT_FAINT()));
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(store::size().is_some(), egui::Button::new(t!("Vider", "Clear")))
                    .on_hover_text(t!("Supprime la copie locale (elle se reconstitue au prochain lancement)", "Deletes the local copy (rebuilt at the next launch)"))
                    .clicked()
                {
                    store::clear();
                }
            });
        });
    });
    ui.add_space(12.0);
}

/// Notifications : types actifs, avance des rappels, son.
fn notifications(ui: &mut egui::Ui, prefs: &mut Prefs) {
    widgets::card(ui, "Notifications", 0.0, |ui| {
        ui.label(
            RichText::new(t!(
                "Bandeau en haut au centre de l'écran, par-dessus les autres fenêtres (un clic y mène).",
                "Banner at the top centre of the screen, above other windows (click it to go there)."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        ui.add_space(8.0);
        let toggle = |ui: &mut egui::Ui, off: &mut bool, label: &str| {
            let mut on = !*off;
            if ui.checkbox(&mut on, label).changed() {
                *off = !on;
            }
        };
        toggle(ui, &mut prefs.notif_no_mail, t!("✉️  Nouveau mail reçu", "✉️  New mail received"));
        toggle(ui, &mut prefs.notif_no_chat, t!("💬  Nouveau message Google Chat (conversations non muettes)", "💬  New Google Chat message (unmuted conversations)"));
        toggle(ui, &mut prefs.notif_no_share, t!("📄  Document partagé avec moi sur Google Drive", "📄  Document shared with me on Google Drive"));
        ui.horizontal(|ui| {
            toggle(ui, &mut prefs.notif_no_event, t!("📅  Rendez-vous qui commence dans", "📅  Event starting in"));
            let mut m = if prefs.notif_event_minutes == 0 { 10 } else { prefs.notif_event_minutes };
            if ui.add_enabled(!prefs.notif_no_event, egui::DragValue::new(&mut m).range(1..=120).suffix(" min")).changed() {
                prefs.notif_event_minutes = m;
            }
        });
        ui.add_space(4.0);
        toggle(ui, &mut prefs.notif_no_sound, t!("Petit son à l'arrivée d'un mail ou d'un message", "Short sound when a mail or message arrives"));
    });
    ui.add_space(12.0);
}

/// Choix du thème (9 thèmes), chacun montré par un aperçu de ses couleurs.
fn themes(ui: &mut egui::Ui, prefs: &mut Prefs) {
    use crate::theme::{ThemeId, rgb};
    widgets::card(ui, t!("Apparence", "Appearance"), 0.0, |ui| {
        ui.label(
            RichText::new(t!("Thème de couleurs, appliqué immédiatement à toute l'application.", "Colour theme, applied immediately to the whole app."))
                .small()
                .color(theme::TEXT_FAINT()),
        );
        ui.add_space(6.0);
        let mut clock = !prefs.clock_hidden;
        if ui.checkbox(&mut clock, t!("Heure en haut à droite (un clic dessus la masque)", "Clock at the top right (click it to hide it)")).changed() {
            prefs.clock_hidden = !clock;
        }
        ui.add_space(10.0);
        let gap = 10.0;
        let cols = 3;
        let w = ((ui.available_width() - gap * (cols - 1) as f32) / cols as f32).floor();
        for line in ThemeId::ALL.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for &t in line {
                    let p = t.palette();
                    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 74.0), egui::Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
                    let sel = prefs.theme == t;
                    let painter = ui.painter();
                    // Aperçu : fond, carte, texte, accent.
                    painter.rect_filled(r, egui::CornerRadius::same(10), rgb(p.bg));
                    let card = egui::Rect::from_min_size(r.min + egui::vec2(10.0, 10.0), egui::vec2(r.width() - 20.0, 30.0));
                    painter.rect(card, egui::CornerRadius::same(6), rgb(p.surface), egui::Stroke::new(1.0, rgb(p.border)), egui::StrokeKind::Inside);
                    painter.rect_filled(egui::Rect::from_min_size(card.min + egui::vec2(8.0, 8.0), egui::vec2(card.width() * 0.45, 5.0)), egui::CornerRadius::same(3), rgb(p.text));
                    painter.rect_filled(
                        egui::Rect::from_min_size(card.min + egui::vec2(8.0, 18.0), egui::vec2(card.width() * 0.3, 4.0)),
                        egui::CornerRadius::same(2),
                        rgb(p.muted),
                    );
                    painter.circle_filled(egui::pos2(card.right() - 12.0, card.center().y), 5.0, rgb(p.accent));
                    painter.circle_filled(egui::pos2(card.right() - 26.0, card.center().y), 5.0, rgb(p.link));
                    painter.text(egui::pos2(r.left() + 10.0, r.bottom() - 15.0), egui::Align2::LEFT_CENTER, t.label(), egui::FontId::proportional(12.5), rgb(p.text));
                    let stroke = if sel {
                        egui::Stroke::new(2.0, rgb(p.accent))
                    } else if resp.hovered() {
                        egui::Stroke::new(1.0, rgb(p.muted))
                    } else {
                        egui::Stroke::new(1.0, rgb(p.border))
                    };
                    painter.rect_stroke(r, egui::CornerRadius::same(10), stroke, egui::StrokeKind::Inside);
                    if resp.clicked() {
                        prefs.theme = t;
                    }
                }
            });
            ui.add_space(gap);
        }
    });
    ui.add_space(12.0);
}

/// Police et taille de chaque groupe de textes, avec aperçu.
fn fonts(ui: &mut egui::Ui, prefs: &mut Prefs) {
    use crate::theme::{FontChoice, FontGroup, FontPrefs};
    widgets::card(ui, t!("Polices", "Fonts"), 0.0, |ui| {
        ui.label(
            RichText::new(t!(
                "Chaque groupe de textes a sa police et sa taille, appliquées dans tous les onglets.",
                "Each group of text has its own font and size, applied in every tab."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        ui.add_space(8.0);
        let f = &mut prefs.fonts;
        let groups: [(&str, &str, &mut FontGroup, egui::FontId, &str); 4] = [
            (t!("Titres", "Titles"), t!("Titres d'onglets, de sections, de notes", "Tab, section and note titles"), &mut f.titles, theme::title_font(20.0), "Google Drive"),
            (
                t!("Interface", "Interface"),
                t!("Menus, listes, boutons, étiquettes", "Menus, lists, buttons, labels"),
                &mut f.interface,
                egui::FontId::proportional(14.0),
                t!("Boîte de réception · 12 non lus", "Inbox · 12 unread"),
            ),
            (
                t!("Texte", "Text"),
                t!("Mails, notes, messages", "Mails, notes, messages"),
                &mut f.content,
                theme::editor_font(14.5, false),
                t!("Bonjour, voici le compte rendu 🙂", "Hi, here are the meeting notes 🙂"),
            ),
            ("Code", t!("Blocs de code, chasse fixe", "Code blocks, monospace"), &mut f.code, egui::FontId::monospace(13.0), "cargo run --release"),
        ];
        for (i, (label, hint, g, preview, sample)) in groups.into_iter().enumerate() {
            if i > 0 {
                ui.add_space(4.0);
                ui.separator();
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    ui.label(label);
                    ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT()));
                });
                egui::ComboBox::from_id_salt(("wt_font_group", i)).width(200.0).selected_text(g.font.label()).show_ui(ui, |ui| {
                    for c in FontChoice::ALL {
                        ui.selectable_value(&mut g.font, c, c.label());
                    }
                });
                let mut pct = (g.scale * 100.0).round();
                if ui.add(egui::Slider::new(&mut pct, 80.0..=140.0).suffix(" %").step_by(5.0).fixed_decimals(0)).changed() {
                    g.scale = pct / 100.0;
                }
            });
            ui.horizontal(|ui| {
                ui.add_space(208.0);
                ui.label(RichText::new(sample).font(preview).color(theme::TEXT_MUTED()));
            });
        }
        ui.add_space(6.0);
        if ui.button(t!("Réinitialiser", "Reset")).clicked() {
            *f = FontPrefs::default();
        }
    });
    ui.add_space(12.0);
}

/// Réglages de l'animation liquide (sélection d'onglets, de dossiers, de mails…).
fn animations(ui: &mut egui::Ui, prefs: &mut Prefs) {
    widgets::card(ui, "Animations", 0.0, |ui| {
        let a = &mut prefs.animation;
        let row = |ui: &mut egui::Ui, label: &str, hint: &str, add: &mut dyn FnMut(&mut egui::Ui)| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(200.0);
                    ui.label(label);
                    ui.label(RichText::new(hint).small().color(theme::TEXT_FAINT()));
                });
                add(ui);
            });
            ui.add_space(6.0);
        };

        row(ui, t!("Vitesse", "Speed"), t!("Durée d'un déplacement", "Duration of one movement"), &mut |ui| {
            ui.add(egui::Slider::new(&mut a.duration_ms, 100.0..=1500.0).suffix(" ms").step_by(10.0).fixed_decimals(0));
        });
        row(ui, t!("Opacité", "Opacity"), t!("Intensité de la forme liquide", "Strength of the liquid shape"), &mut |ui| {
            let mut pct = a.opacity * 100.0;
            if ui.add(egui::Slider::new(&mut pct, 10.0..=100.0).suffix(" %").step_by(1.0).fixed_decimals(0)).changed() {
                a.opacity = pct / 100.0;
            }
        });
        row(ui, t!("Courbe de mouvement", "Motion curve"), t!("Accélération du déplacement", "Acceleration of the movement"), &mut |ui| {
            let opts: Vec<(Curve, &str)> = Curve::ALL.iter().map(|c| (*c, c.label())).collect();
            widgets::segmented(ui, "curve", &opts, &mut a.curve);
        });

        // Aperçu : la forme passe d'une case à l'autre en continu.
        row(ui, t!("Aperçu", "Preview"), t!("Avec les réglages ci-dessus", "With the settings above"), &mut |ui| preview(ui));

        if ui.button(t!("Réinitialiser", "Reset")).clicked() {
            *a = Animation::default();
        }
    });
    ui.add_space(12.0);
}

fn preview(ui: &mut egui::Ui) {
    let anim = crate::liquid::settings(ui.ctx());
    let period = (anim.duration_ms / 1000.0) as f64 + 0.7;
    let step = (ui.input(|i| i.time) / period) as usize % 4;
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
    egui::Frame::new().fill(theme::BG()).stroke(egui::Stroke::new(1.0, theme::BORDER())).corner_radius(egui::CornerRadius::same(255)).inner_margin(egui::Margin::same(4)).show(
        ui,
        |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                let mut hl = Liquid::begin(ui, "anim_preview");
                for i in 0..4 {
                    let (r, _) = ui.allocate_exact_size(egui::vec2(36.0, 26.0), egui::Sense::hover());
                    if i == step {
                        hl.target(r, i);
                    }
                    let c = crate::sidebar::lerp_color(theme::TEXT_FAINT(), theme::BG(), hl.coverage(r));
                    ui.painter().circle_filled(r.center(), 3.0, c);
                }
                hl.end(ui, theme::TEXT(), 13.0);
            });
        },
    );
}

/// Couleurs des mails reçus via une adresse de groupe (liste de diffusion, alias).
fn group_colors(ui: &mut egui::Ui, prefs: &mut Prefs) {
    use crate::tabs::mail::groups;
    widgets::card(ui, t!("Couleurs des groupes", "Group colours"), 0.0, |ui| {
        ui.label(
            RichText::new(t!(
                "Les mails reçus via ces adresses sont colorés dans la boîte de réception. Ajout depuis l'onglet Mail (section Groupes ou « Reçu via »).",
                "Mails received through these addresses are coloured in the inbox. Add them from the Mail tab (Groups section or “Received via”)."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        ui.add_space(8.0);
        if prefs.groups.is_empty() {
            ui.label(RichText::new(t!("Aucun groupe coloré", "No coloured group")).color(theme::TEXT_MUTED()));
        }
        let mut edits = Vec::new();
        for g in &prefs.groups {
            ui.horizontal(|ui| {
                if let Some(e) = groups::swatch(ui, g, 16.0) {
                    edits.push(e);
                }
                ui.add_space(6.0);
                ui.label(RichText::new(&g.address).color(groups::color32(g.color)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button(t!("Retirer", "Remove")).clicked() {
                        edits.push(groups::GroupEdit::Remove(g.address.clone()));
                    }
                });
            });
            ui.add_space(4.0);
        }
        for e in edits {
            groups::apply(&mut prefs.groups, e);
        }
    });
    ui.add_space(12.0);
}
