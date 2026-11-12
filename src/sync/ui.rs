//! Réglages › Compte NAS : connexion au serveur de synchronisation, état, postes.

use std::sync::Mutex;

use eframe::egui::{self, RichText};

use super::{Account, Phase};
use crate::{theme, widgets};

/// Saisie en cours (gardée d'une image à l'autre).
#[derive(Default)]
struct Form {
    server: String,
    login: String,
    password: String,
    first: bool,
    invite: String,
    confirm_logout: bool,
}

static FORM: Mutex<Option<Form>> = Mutex::new(None);

pub fn card(ui: &mut egui::Ui) {
    super::set_ctx(ui.ctx());
    let mut guard = FORM.lock().unwrap_or_else(|e| e.into_inner());
    let form = guard.get_or_insert_with(Form::default);
    let st = super::status();
    let account = Account::load().filter(|_| super::configured());
    widgets::card(ui, t!("Compte NAS", "NAS account"), 0.0, |ui| {
        ui.label(
            RichText::new(t!(
                "Synchronise notes, tâches, Blueprints, réglages et caches avec ton serveur, chiffrés sur ce poste avant l'envoi.",
                "Syncs notes, tasks, Blueprints, settings and caches with your server, encrypted on this computer before sending."
            ))
            .small()
            .color(theme::TEXT_FAINT()),
        );
        ui.add_space(10.0);

        if let Some(code) = &st.recovery {
            recovery(ui, code);
            ui.add_space(10.0);
        }
        match &account {
            None => sign_in(ui, form, &st),
            Some(a) => connected(ui, form, a, &st),
        }
    });
    ui.add_space(12.0);
}

fn sign_in(ui: &mut egui::Ui, form: &mut Form, st: &super::Status) {
    let busy = st.connecting;
    ui.add_enabled_ui(!busy, |ui| {
        widgets::field(ui, t!("Serveur", "Server"), 110.0, egui::TextEdit::singleline(&mut form.server).hint_text("sync.exemple.fr"));
        widgets::field(ui, t!("Identifiant", "Login"), 110.0, egui::TextEdit::singleline(&mut form.login));
        widgets::field(ui, t!("Mot de passe", "Password"), 110.0, egui::TextEdit::singleline(&mut form.password).password(true));
        ui.checkbox(&mut form.first, t!("Premier login (code d'invitation donné par la console du NAS)", "First sign-in (invitation code from the NAS console)"));
        if form.first {
            widgets::field(ui, t!("Code", "Code"), 110.0, egui::TextEdit::singleline(&mut form.invite).hint_text("ABCD-EFGH-JKLM").font(egui::TextStyle::Monospace));
            ui.label(
                RichText::new(t!(
                    "Le mot de passe choisi ici ne quitte jamais ce poste. Oublié, il rend les données du serveur illisibles (sauf avec le code de récupération affiché ensuite).",
                    "The password chosen here never leaves this computer. If forgotten, server data becomes unreadable (except with the recovery code shown next)."
                ))
                .small()
                .color(theme::TEXT_FAINT()),
            );
        }
    });
    ui.add_space(8.0);
    ui.horizontal(|ui| {
        let ready =
            !busy && !form.server.trim().is_empty() && !form.login.trim().is_empty() && form.password.chars().count() >= 8 && (!form.first || !form.invite.trim().is_empty());
        let label = if form.first { t!("Activer le compte", "Activate account") } else { t!("Se connecter", "Sign in") };
        if ui.add_enabled(ready, egui::Button::new(label)).clicked() {
            let invite = form.first.then(|| form.invite.clone());
            super::connect(form.server.clone(), form.login.clone(), std::mem::take(&mut form.password), invite);
        }
        if busy {
            ui.add(egui::Spinner::new().size(14.0));
            ui.label(t!("Connexion et première synchronisation…", "Signing in and first sync…"));
        } else if !form.password.is_empty() && form.password.chars().count() < 8 {
            ui.label(RichText::new(t!("8 caractères minimum", "8 characters minimum")).small().color(theme::TEXT_FAINT()));
        }
    });
    if let Some(e) = &st.connect_error {
        ui.add_space(6.0);
        ui.add(egui::Label::new(RichText::new(e).small().color(theme::TEXT())).wrap());
    }
}

fn connected(ui: &mut egui::Ui, form: &mut Form, a: &Account, st: &super::Status) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(tf!("{} sur {}", "{} on {}", a.login, a.server));
            let (text, alert) = match &st.phase {
                Phase::Off | Phase::Idle => match st.last_sync {
                    Some(ms) => (tf!("Synchronisé — {}", "Synced — {}", when(ms)), false),
                    None => (t!("En attente de la première synchronisation", "Waiting for the first sync").to_string(), false),
                },
                Phase::Syncing => (t!("Synchronisation…", "Syncing…").to_string(), false),
                Phase::Offline(e) => (tf!("Serveur injoignable, nouvel essai chaque minute ({e})", "Server unreachable, retrying every minute ({e})"), true),
                Phase::SignedOut => {
                    (t!("Session révoquée ou expirée : déconnecte-toi puis reconnecte-toi", "Session revoked or expired: sign out then sign in again").to_string(), true)
                }
                Phase::Error(e) => (e.clone(), true),
            };
            ui.add(egui::Label::new(RichText::new(text).small().color(if alert { theme::TODAY() } else { theme::TEXT_FAINT() })).wrap());
            if st.quota > 0 {
                let size = |b: u64| if b >= 1 << 30 { format!("{:.1} Go", b as f64 / (1u64 << 30) as f64) } else { crate::tabs::mail::attachments::human_size(b.max(1)) };
                ui.label(RichText::new(tf!("{} fichiers · {} sur {}", "{} files · {} of {}", st.files, size(st.used), size(st.quota))).small().color(theme::TEXT_FAINT()));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let busy = st.phase == Phase::Syncing;
            if ui.add_enabled(!busy, egui::Button::new(t!("Synchroniser", "Sync"))).clicked() {
                super::sync_now();
            }
            if busy {
                ui.add(egui::Spinner::new().size(14.0));
            }
        });
    });

    // Changements reçus : appliqués au redémarrage.
    if !st.pending.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                let from: Vec<&str> = {
                    let mut d: Vec<&str> = st.pending.iter().map(|(_, dev)| dev.as_str()).collect();
                    d.sort();
                    d.dedup();
                    d
                };
                ui.label(RichText::new(tf!("{} fichier(s) modifié(s) sur {}", "{} file(s) changed on {}", st.pending.len(), from.join(", "))).color(theme::TODAY()));
                ui.label(
                    RichText::new(t!(
                        "Appliqués au prochain démarrage de WorkTable (fusionnés avec tes modifications d'ici).",
                        "Applied at the next WorkTable start (merged with your changes here)."
                    ))
                    .small()
                    .color(theme::TEXT_FAINT()),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(t!("Redémarrer et appliquer", "Restart and apply")).clicked() {
                    crate::update::restart(ui.ctx());
                }
            });
        });
    }
    if !st.conflicts.is_empty() {
        ui.add_space(6.0);
        ui.label(
            RichText::new(tf!(
                "{} fichier(s) modifié(s) sur deux postes : la version d'ici est gardée en copie « conflit ».",
                "{} file(s) changed on two computers: this computer's version is kept as a “conflict” copy.",
                st.conflicts.len()
            ))
            .small()
            .color(theme::TODAY()),
        );
    }
    if !st.skipped.is_empty() {
        ui.add_space(6.0);
        ui.label(RichText::new(tf!("Non synchronisés (plus de 7 Mo) : {}", "Not synced (over 7 MB): {}", st.skipped.join(", "))).small().color(theme::TEXT_FAINT()));
    }

    // Postes du compte.
    if !st.devices.is_empty() {
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(6.0);
        ui.label(RichText::new(t!("Postes connectés", "Signed-in computers")).strong());
        for d in &st.devices {
            ui.horizontal(|ui| {
                let me = d.id == st.device_id;
                let name = if me { tf!("{} (ce poste)", "{} (this computer)", d.name) } else { d.name.clone() };
                ui.label(name);
                ui.label(RichText::new(when(d.last_seen as i64 * 1000)).small().color(theme::TEXT_FAINT()));
                if !me {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button(t!("Révoquer", "Revoke")).clicked() {
                            super::revoke(d.id);
                        }
                    });
                }
            });
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if form.confirm_logout {
            ui.label(RichText::new(t!("Ce poste ne synchronisera plus (ses fichiers restent). Sûr ?", "This computer will stop syncing (its files stay). Sure?")).small());
            if ui.button(t!("Se déconnecter", "Sign out")).clicked() {
                super::disconnect();
                form.confirm_logout = false;
            }
            if ui.button(t!("Annuler", "Cancel")).clicked() {
                form.confirm_logout = false;
            }
        } else if ui.button(t!("Se déconnecter", "Sign out")).clicked() {
            form.confirm_logout = true;
        }
    });
}

/// Code de récupération : affiché une seule fois, juste après l'activation du compte.
fn recovery(ui: &mut egui::Ui, code: &str) {
    egui::Frame::new().fill(theme::blend(theme::SURFACE(), theme::TODAY(), 0.12)).corner_radius(egui::CornerRadius::same(8)).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(t!("Code de récupération", "Recovery code")).strong());
        ui.label(
            RichText::new(t!(
                "Seul moyen de relire tes données si tu oublies ton mot de passe. Note-le en lieu sûr : il ne sera plus affiché.",
                "The only way to read your data again if you forget your password. Keep it safe: it won't be shown again."
            ))
            .small()
            .color(theme::TEXT_MUTED()),
        );
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(code).monospace().size(16.0));
            if ui.button(t!("Copier", "Copy")).clicked() {
                ui.ctx().copy_text(code.to_string());
            }
            if ui.button(t!("C'est noté", "Got it")).clicked() {
                super::with_status(|s| s.recovery = None);
            }
        });
    });
}

/// « aujourd'hui 14:05 », ou la date.
fn when(ms: i64) -> String {
    use chrono::TimeZone;
    let Some(d) = chrono::Local.timestamp_millis_opt(ms).single() else { return String::new() };
    if d.date_naive() == chrono::Local::now().date_naive() { tf!("aujourd'hui {}", "today {}", d.format("%H:%M")) } else { d.format("%d/%m %H:%M").to_string() }
}
