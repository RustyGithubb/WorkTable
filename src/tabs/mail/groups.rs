//! Mails reçus via une adresse intermédiaire (liste de diffusion, alias de groupe) :
//! détection, couleur personnalisable, palette.

use eframe::egui::{self, Color32, CornerRadius, RichText, Sense, Stroke, vec2};

use super::contacts;
use crate::google::Summary;
use crate::prefs::GroupColor;
use crate::theme;

/// Palette proposée (lisible sur fond sombre).
pub const PALETTE: [[u8; 3]; 10] = [
    [0x4C, 0x9A, 0xFF], // bleu
    [0x3D, 0xD6, 0xC6], // turquoise
    [0x5C, 0xD6, 0x6B], // vert
    [0xC8, 0xE0, 0x4A], // anis
    [0xFF, 0xC8, 0x3D], // jaune
    [0xFF, 0x8A, 0x3D], // orange
    [0xFF, 0x5C, 0x6C], // rouge
    [0xFF, 0x6F, 0xC8], // rose
    [0xB2, 0x7C, 0xFF], // violet
    [0xC8, 0xC8, 0xC8], // gris clair
];

pub fn color32(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

fn emails(list: &str) -> Vec<String> {
    contacts::split_list(list).iter().filter_map(|m| contacts::parse_mailbox(m).map(|(_, e)| e)).collect()
}

/// `List-Id: <theotter.artfx.fr>` correspond à l'adresse theotter@artfx.fr (format Google Groups).
fn list_id_matches(list_id: &str, address: &str) -> bool {
    let id = list_id.to_lowercase();
    !id.is_empty() && id.contains(&address.to_lowercase().replacen('@', ".", 1))
}

/// Groupe coloré auquel le mail a été adressé, s'il y en a un.
pub fn group_of<'a>(m: &Summary, groups: &'a [GroupColor]) -> Option<&'a GroupColor> {
    if groups.is_empty() {
        return None;
    }
    let rcpt = emails(&m.recipients);
    groups.iter().find(|g| rcpt.iter().any(|e| e.eq_ignore_ascii_case(&g.address)) || list_id_matches(&m.list_id, &g.address))
}

/// Adresses intermédiaires d'un mail reçu : les destinataires quand je n'y figure pas
/// moi-même (reçu via un groupe / une liste), ou l'adresse déduite du List-Id.
pub fn intermediates(m: &Summary, own: &str) -> Vec<String> {
    if m.labels.iter().any(|l| l == "SENT") || own.is_empty() {
        return Vec::new();
    }
    let own = own.to_lowercase();
    let rcpt = emails(&m.recipients);
    let mut out: Vec<String> = Vec::new();
    if !rcpt.contains(&own) {
        out.extend(rcpt.into_iter().filter(|e| *e != own));
    }
    // Google Groups : <nom.domaine.tld>
    let id = m.list_id.trim().trim_start_matches(|c| c != '<').trim_matches(['<', '>', ' ']).to_lowercase();
    if let Some((name, domain)) = id.split_once('.')
        && domain.contains('.')
    {
        let addr = format!("{name}@{domain}");
        if !out.contains(&addr) && addr != own {
            out.push(addr);
        }
    }
    out
}

/// Couleur proposée pour un nouveau groupe (première couleur pas encore utilisée).
pub fn next_color(groups: &[GroupColor]) -> [u8; 3] {
    PALETTE.iter().copied().find(|c| !groups.iter().any(|g| g.color == *c)).unwrap_or(PALETTE[0])
}

pub enum GroupEdit {
    Color(String, [u8; 3]),
    Remove(String),
}

/// Pastille de couleur cliquable ouvrant la palette (couleurs proposées + couleur libre).
pub fn swatch(ui: &mut egui::Ui, g: &GroupColor, size: f32) -> Option<GroupEdit> {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(t!("Changer la couleur", "Change colour"));
    ui.painter().circle_filled(rect.center(), size / 2.0, color32(g.color));
    if resp.hovered() {
        ui.painter().circle_stroke(rect.center(), size / 2.0 + 2.0, Stroke::new(1.0, theme::TEXT()));
    }
    let mut edit = None;
    egui::Popup::menu(&resp).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
        edit = palette(ui, g);
    });
    edit
}

pub fn palette(ui: &mut egui::Ui, g: &GroupColor) -> Option<GroupEdit> {
    let mut edit = None;
    ui.set_min_width(220.0);
    ui.label(RichText::new(&g.address).small().color(theme::TEXT_MUTED()));
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
        for c in PALETTE {
            let (r, resp) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
            let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            ui.painter().rect_filled(r, CornerRadius::same(6), color32(c));
            if c == g.color {
                ui.painter().rect_stroke(r.expand(2.0), CornerRadius::same(8), Stroke::new(1.5, theme::TEXT()), egui::StrokeKind::Outside);
            }
            if resp.clicked() {
                edit = Some(GroupEdit::Color(g.address.clone(), c));
            }
        }
    });
    ui.add_space(8.0);
    ui.label(RichText::new(t!("Couleur libre", "Custom colour")).small().color(theme::TEXT_MUTED()));
    let mut c = color32(g.color);
    if egui::widgets::color_picker::color_picker_color32(ui, &mut c, egui::widgets::color_picker::Alpha::Opaque) {
        edit = Some(GroupEdit::Color(g.address.clone(), [c.r(), c.g(), c.b()]));
    }
    ui.add_space(6.0);
    if ui.button(t!("Retirer la couleur de ce groupe", "Remove this group's colour")).clicked() {
        edit = Some(GroupEdit::Remove(g.address.clone()));
    }
    edit
}

pub fn apply(groups: &mut Vec<GroupColor>, e: GroupEdit) {
    match e {
        GroupEdit::Color(addr, c) => {
            if let Some(g) = groups.iter_mut().find(|g| g.address.eq_ignore_ascii_case(&addr)) {
                g.color = c;
            } else {
                groups.push(GroupColor { address: addr.to_lowercase(), color: c });
            }
        }
        GroupEdit::Remove(addr) => groups.retain(|g| !g.address.eq_ignore_ascii_case(&addr)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(to: &str, list_id: &str) -> Summary {
        Summary { recipients: to.into(), list_id: list_id.into(), labels: vec!["INBOX".into()], ..Default::default() }
    }

    #[test]
    fn detects_group_mail() {
        let own = "dorian@artfx.fr";
        let groups = vec![GroupColor { address: "theotter@artfx.fr".into(), color: PALETTE[0] }];
        // Adressé au groupe (je n'y figure pas).
        let m = mail("Promo <TheOtter@artfx.fr>", "");
        assert_eq!(intermediates(&m, own), vec!["theotter@artfx.fr"]);
        assert!(group_of(&m, &groups).is_some());
        // Google Groups : repéré par List-Id même si le To diffère.
        let m = mail("undisclosed-recipients:;", "Promo <theotter.artfx.fr>");
        assert!(group_of(&m, &groups).is_some());
        assert_eq!(intermediates(&m, own), vec!["theotter@artfx.fr"]);
        // Mail qui m'est adressé directement : pas d'intermédiaire.
        let m = mail("Dorian <dorian@artfx.fr>, lea@studio.fr", "");
        assert!(intermediates(&m, own).is_empty());
        assert!(group_of(&m, &groups).is_none());
    }

    #[test]
    fn edits() {
        let mut g = Vec::new();
        apply(&mut g, GroupEdit::Color("TheOtter@artfx.fr".into(), PALETTE[2]));
        assert_eq!(g[0].address, "theotter@artfx.fr");
        apply(&mut g, GroupEdit::Color("theotter@artfx.fr".into(), PALETTE[3]));
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].color, PALETTE[3]);
        assert_eq!(next_color(&g), PALETTE[0]);
        apply(&mut g, GroupEdit::Remove("theotter@artfx.fr".into()));
        assert!(g.is_empty());
    }
}
