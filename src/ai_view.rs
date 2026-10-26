//! Fenêtre de chat rapide de l'assistant (ouverte depuis le logo de la barre) et notifications.

use eframe::egui::{self, Align2, Color32, CornerRadius, Key, Margin, Modifiers, Pos2, Rect, RichText, Sense, Stroke, vec2};

use crate::ai::{AiState, NotifKind, QUICK, Role};
use crate::tabs::notes::inline::{self, Base};
use crate::theme;

pub enum AiAction {
    OpenMail(String),
    OpenSettings,
}

const W: f32 = 420.0;

/// Fenêtre de chat. Renvoie une question à envoyer (le contexte est ajouté par l'appelant).
pub fn panel(ctx: &egui::Context, window: Rect, ai: &mut AiState, action: &mut Option<AiAction>) -> Option<String> {
    if !ai.open {
        return None;
    }
    let mut send = None;
    let h = (window.height() - 64.0).min(680.0);
    let pos = Pos2::new(window.left() + 16.0 + 56.0 + 12.0, window.center().y - h / 2.0);
    let has_key = ai.has_key();
    egui::Area::new(egui::Id::unique("wt_ai_panel")).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::new()
            .fill(theme::SURFACE())
            .stroke(Stroke::new(1.0, theme::BORDER()))
            .corner_radius(CornerRadius::same(16))
            .inner_margin(Margin::same(14))
            .shadow(egui::Shadow { offset: [0, 10], blur: 32, spread: 0, color: Color32::from_black_alpha(110) })
            .show(ui, |ui| {
                ui.set_width(W);
                ui.set_height(h - 28.0);
                // En-tête.
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    ui.painter().circle_filled(r.center(), 14.0, theme::TODAY());
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, "✦", egui::FontId::proportional(15.0), Color32::WHITE);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("Assistant").font(theme::title_font(16.0)).color(theme::TEXT()));
                        ui.label(RichText::new("Claude · voit ton agenda, tes mails, tâches et notes").small().color(theme::TEXT_FAINT()));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Button::new("✕").frame_when_inactive(false)).on_hover_text("Fermer (Échap)").clicked() {
                            ai.open = false;
                        }
                        if !ai.messages.is_empty() && ui.add(egui::Button::new(RichText::new("Effacer").small()).frame_when_inactive(false)).on_hover_text("Nouvelle conversation").clicked() {
                            ai.clear();
                        }
                    });
                });
                ui.add_space(6.0);
                let sep = ui.available_rect_before_wrap();
                ui.painter().hline(sep.x_range(), sep.top(), Stroke::new(1.0, theme::BORDER()));
                ui.add_space(8.0);

                if !has_key {
                    no_key(ui, ai, action);
                    return;
                }

                // Conversation.
                let input_h = 96.0;
                let list_h = (ui.available_height() - input_h).max(80.0);
                egui::ScrollArea::vertical().max_height(list_h).min_scrolled_height(list_h).auto_shrink([false, false]).stick_to_bottom(true).show(ui, |ui| {
                    ui.set_width(W - 6.0);
                    if ai.messages.is_empty() {
                        ui.add_space(30.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new("Bonjour 👋").font(theme::title_font(20.0)).color(theme::TEXT()));
                            ui.add_space(4.0);
                            ui.label(RichText::new("Pose une question sur ta journée, tes mails, tes tâches…").color(theme::TEXT_MUTED()));
                        });
                        ui.add_space(18.0);
                    }
                    for (role, text) in ai.messages.clone() {
                        match role {
                            Role::User => user_bubble(ui, &text),
                            Role::Assistant => assistant_text(ui, &text),
                        }
                        ui.add_space(10.0);
                    }
                    if ai.thinking {
                        ui.horizontal(|ui| {
                            ui.add(egui::Spinner::new().size(13.0));
                            ui.label(RichText::new("Claude réfléchit…").small().color(theme::TEXT_MUTED()));
                        });
                    }
                    if ai.scroll_bottom {
                        ui.scroll_to_cursor(Some(egui::Align::Max));
                        ai.scroll_bottom = false;
                    }
                });

                // Questions rapides.
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for (label, q) in QUICK {
                        let b = egui::Button::new(RichText::new(label).small()).corner_radius(CornerRadius::same(255));
                        if ui.add_enabled(!ai.thinking, b).clicked() {
                            send = Some(q.to_string());
                        }
                    }
                });
                ui.add_space(6.0);
                // Saisie.
                egui::Frame::new().fill(theme::BG()).stroke(Stroke::new(1.0, theme::BORDER())).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let id = egui::Id::unique("wt_ai_input");
                        let enter = ui.memory(|m| m.has_focus(id)) && ui.input_mut(|i| !i.modifiers.shift && i.consume_key(Modifiers::NONE, Key::Enter));
                        let out = egui::TextEdit::multiline(&mut ai.input)
                            .id(id)
                            .frame(egui::Frame::NONE)
                            .desired_rows(1)
                            .hint_text("Demande à Claude…")
                            .desired_width(ui.available_width() - 40.0)
                            .show(ui);
                        crate::spell::check(ui, &out, &mut ai.input);
                        let r = out.response.response;
                        if ai.focus_input {
                            r.request_focus();
                            ai.focus_input = false;
                        }
                        let go = ui.add_enabled(!ai.thinking && !ai.input.trim().is_empty(), egui::Button::new("➤").corner_radius(CornerRadius::same(255))).on_hover_text("Envoyer (Entrée)");
                        if (enter || go.clicked()) && !ai.thinking && !ai.input.trim().is_empty() {
                            send = Some(std::mem::take(&mut ai.input));
                            ai.focus_input = true;
                        }
                    });
                });
            });
    });
    if ctx.input(|i| i.key_pressed(Key::Escape)) {
        ai.open = false;
    }
    send
}

fn no_key(ui: &mut egui::Ui, ai: &mut AiState, action: &mut Option<AiAction>) {
    ui.add_space(10.0);
    ui.label(RichText::new("Connecter Claude").font(theme::title_font(16.0)).color(theme::TEXT()));
    ui.add_space(4.0);
    ui.label(
        RichText::new(
            "L'assistant utilise l'API d'Anthropic avec ta propre clé. Crée-la sur console.anthropic.com \
             (facturée à l'usage, indépendamment de l'abonnement Claude), puis colle-la ici.",
        )
        .color(theme::TEXT_MUTED()),
    );
    ui.add_space(10.0);
    ui.add(egui::TextEdit::singleline(&mut ai.key_input).password(true).hint_text("sk-ant-…").desired_width(f32::INFINITY));
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(!ai.key_input.trim().is_empty(), egui::Button::new("Enregistrer la clé")).clicked() {
            ai.save_key();
        }
        if ui.button("Obtenir une clé ↗").clicked() {
            let _ = webbrowser::open("https://console.anthropic.com/settings/keys");
        }
        if ui.button("Réglages").clicked() {
            *action = Some(AiAction::OpenSettings);
        }
    });
    if let Some((ok, msg)) = &ai.key_status {
        ui.add_space(6.0);
        ui.label(RichText::new(msg).small().color(if *ok { theme::TEXT_MUTED() } else { theme::TODAY() }));
    }
}

fn user_bubble(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
        egui::Frame::new().fill(theme::SURFACE_HI()).corner_radius(CornerRadius::same(12)).inner_margin(Margin::symmetric(12, 8)).show(ui, |ui| {
            ui.set_max_width(W * 0.78);
            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                ui.add(egui::Label::new(RichText::new(text).color(theme::TEXT())).wrap());
            });
        });
    });
}

/// Réponse de Claude : Markdown simple (titres, listes, gras, italique, code).
fn assistant_text(ui: &mut egui::Ui, text: &str) {
    let width = ui.available_width() - 8.0;
    let base = Base { size: 13.5, bold: false, italic: false, color: theme::TEXT(), mono: false, strike: false };
    ui.spacing_mut().item_spacing.y = 3.0;
    let mut code = false;
    for raw in text.lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            code = !code;
            continue;
        }
        if code {
            ui.label(RichText::new(line).monospace().color(theme::TEXT_MUTED()));
            continue;
        }
        let t = line.trim_start();
        if t.is_empty() {
            ui.add_space(4.0);
            continue;
        }
        let (prefix, body, b) = if let Some(r) = t.strip_prefix("### ").or_else(|| t.strip_prefix("## ")).or_else(|| t.strip_prefix("# ")) {
            ("", r, Base { size: 14.5, bold: true, ..base })
        } else if let Some(r) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")).or_else(|| t.strip_prefix("• ")) {
            ("•", r, base)
        } else if let Some((n, r)) = t.split_once(". ").filter(|(n, _)| !n.is_empty() && n.len() <= 2 && n.chars().all(|c| c.is_ascii_digit())) {
            let _ = n;
            ("#", r, base)
        } else {
            ("", t, base)
        };
        let indent = if prefix.is_empty() { 0.0 } else { 16.0 };
        let (job, _) = inline::job(body, b, false, width - indent);
        let galley = ui.painter().layout_job(job);
        let (r, _) = ui.allocate_exact_size(vec2(width, galley.size().y), Sense::hover());
        match prefix {
            "•" => {
                ui.painter().circle_filled(Pos2::new(r.left() + 6.0, r.top() + b.size * 0.75), 2.4, theme::TEXT_MUTED());
            }
            "#" => {
                let n = t.split_once(". ").map(|(n, _)| n).unwrap_or("");
                ui.painter().text(Pos2::new(r.left() + 12.0, r.top()), Align2::RIGHT_TOP, format!("{n}."), egui::FontId::proportional(13.0), theme::TEXT_MUTED());
            }
            _ => {}
        }
        ui.painter().galley(Pos2::new(r.left() + indent, r.top()), galley, theme::TEXT());
    }
    ui.spacing_mut().item_spacing.y = 8.0;
}

/// Notifications (nouveaux mails résumés, événements proches), en haut à droite.
pub fn notifications(ctx: &egui::Context, window: Rect, ai: &mut AiState) -> Option<AiAction> {
    if ai.notifications.is_empty() {
        return None;
    }
    let mut out = None;
    let mut close = None;
    let w = 340.0;
    egui::Area::new(egui::Id::unique("wt_ai_notifs")).order(egui::Order::Foreground).fixed_pos(Pos2::new(window.right() - w - 24.0, window.top() + 20.0)).show(ctx, |ui| {
        ui.set_width(w);
        for (i, n) in ai.notifications.iter().enumerate().rev().take(4) {
            // Apparition en fondu.
            let k = (n.at.elapsed().as_secs_f32() / 0.25).min(1.0);
            ui.multiply_opacity(k);
            let resp = egui::Frame::new()
                .fill(theme::SURFACE_HI())
                .stroke(Stroke::new(1.0, theme::BORDER()))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(Margin::symmetric(12, 10))
                .shadow(egui::Shadow { offset: [0, 6], blur: 20, spread: 0, color: Color32::from_black_alpha(90) })
                .show(ui, |ui| {
                    ui.set_width(w - 24.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&n.title).strong().color(theme::TEXT()));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(egui::Button::new(RichText::new("✕").small()).frame(false)).clicked() {
                                close = Some(i);
                            }
                            if matches!(n.kind, NotifKind::Mail(_)) {
                                ui.label(RichText::new("résumé par Claude").small().color(theme::TEXT_FAINT()));
                            }
                        });
                    });
                    ui.add(egui::Label::new(RichText::new(&n.text).color(theme::TEXT_MUTED())).wrap());
                })
                .response;
            ui.multiply_opacity(if k > 0.0 { 1.0 / k } else { 1.0 });
            let click = ui.interact(resp.rect, egui::Id::unique(("wt_ai_notif", i)), Sense::click());
            if click.clicked() && close.is_none() {
                if let NotifKind::Mail(id) = &n.kind {
                    out = Some(AiAction::OpenMail(id.clone()));
                }
                close = Some(i);
            }
            ui.add_space(8.0);
        }
    });
    if let Some(i) = close {
        ai.notifications.remove(i);
    }
    out
}
