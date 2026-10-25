//! Mise en forme Markdown en ligne : **gras**, *italique*, ~~barré~~, `code`, ==surligné==,
//! <u>souligné</u>, [lien](url). En édition les marqueurs restent visibles (estompés),
//! en lecture ils sont masqués.

use std::ops::Range;

use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontFamily, FontId, Stroke};

use crate::theme;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub strike: bool,
    pub code: bool,
    pub underline: bool,
    pub highlight: bool,
}

#[derive(Debug, PartialEq)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
    pub marker: bool,
    pub link: Option<usize>,
}

pub struct Inline {
    pub spans: Vec<Span>,
    pub links: Vec<String>,
}

/// Mise en forme de base d'un bloc.
#[derive(Clone, Copy)]
pub struct Base {
    pub size: f32,
    pub bold: bool,
    pub italic: bool,
    pub color: Color32,
    pub mono: bool,
    pub strike: bool,
}

fn closes_later(text: &str, from: usize, m: &str) -> bool {
    text.get(from..).is_some_and(|r| !r.starts_with(char::is_whitespace) && r.find(m).is_some_and(|k| k > 0))
}

pub fn parse(text: &str) -> Inline {
    let mut spans: Vec<Span> = Vec::new();
    let mut links = Vec::new();
    let mut st = Style::default();
    let mut i = 0;
    let push = |spans: &mut Vec<Span>, r: Range<usize>, style: Style, marker: bool, link: Option<usize>| {
        if r.is_empty() {
            return;
        }
        if let Some(last) = spans.last_mut()
            && last.range.end == r.start
            && last.style == style
            && last.marker == marker
            && last.link == link
        {
            last.range.end = r.end;
            return;
        }
        spans.push(Span { range: r, style, marker, link });
    };
    while i < text.len() {
        let rest = &text[i..];
        // Code : tout est littéral jusqu'à l'accent grave fermant.
        if st.code {
            if rest.starts_with('`') {
                push(&mut spans, i..i + 1, st, true, None);
                st.code = false;
                i += 1;
            } else {
                let n = rest.find('`').unwrap_or(rest.len());
                push(&mut spans, i..i + n, st, false, None);
                i += n;
            }
            continue;
        }
        if rest.starts_with('`') && rest[1..].contains('`') {
            push(&mut spans, i..i + 1, st, true, None);
            st.code = true;
            i += 1;
            continue;
        }
        // Marqueurs doubles / balises.
        let mut handled = false;
        for (m, close) in [("**", None), ("~~", None), ("==", None), ("<u>", Some("</u>")), ("</u>", None)] {
            if !rest.starts_with(m) {
                continue;
            }
            let flag = match m {
                "**" => &mut st.bold,
                "~~" => &mut st.strike,
                "==" => &mut st.highlight,
                _ => &mut st.underline,
            };
            let opening = m != "</u>";
            if *flag && (m == "</u>" || close.is_none()) {
                push(&mut spans, i..i + m.len(), st, true, None);
                // Le marqueur fermant garde le style ; le texte suivant le perd.
                let flag = match m {
                    "**" => &mut st.bold,
                    "~~" => &mut st.strike,
                    "==" => &mut st.highlight,
                    _ => &mut st.underline,
                };
                *flag = false;
                i += m.len();
                handled = true;
                break;
            }
            if opening && !*flag && closes_later(text, i + m.len(), close.unwrap_or(m)) {
                *flag = true;
                push(&mut spans, i..i + m.len(), st, true, None);
                i += m.len();
                handled = true;
                break;
            }
        }
        if handled {
            continue;
        }
        // Italique : * ou _ (pas au milieu d'un mot pour _).
        let c = rest.chars().next().unwrap_or(' ');
        if c == '*' || c == '_' {
            let prev_alnum = text[..i].chars().next_back().is_some_and(char::is_alphanumeric);
            let m = &rest[..1];
            if st.italic {
                push(&mut spans, i..i + 1, st, true, None);
                st.italic = false;
                i += 1;
                continue;
            }
            if !(c == '_' && prev_alnum) && closes_later(text, i + 1, m) {
                st.italic = true;
                push(&mut spans, i..i + 1, st, true, None);
                i += 1;
                continue;
            }
        }
        // Lien [texte](url).
        if c == '['
            && let Some(close) = rest.find("](")
        {
            let label = &rest[1..close];
            let after = &rest[close + 2..];
            if !label.is_empty()
                && !label.contains(']')
                && let Some(end) = after.find(')')
            {
                let url = &after[..end];
                if !url.is_empty() && !url.contains(char::is_whitespace) {
                    let idx = links.len();
                    links.push(url.to_string());
                    push(&mut spans, i..i + 1, st, true, Some(idx));
                    let ls = i + 1;
                    push(&mut spans, ls..ls + label.len(), st, false, Some(idx));
                    let ms = ls + label.len();
                    push(&mut spans, ms..ms + 2 + url.len() + 1, st, true, Some(idx));
                    i = ms + 2 + url.len() + 1;
                    continue;
                }
            }
        }
        push(&mut spans, i..i + c.len_utf8(), st, false, None);
        i += c.len_utf8();
    }
    Inline { spans, links }
}

pub fn font(size: f32, bold: bool, mono: bool) -> FontId {
    if mono { FontId::new(size, FontFamily::Monospace) } else { theme::editor_font(size, bold) }
}

/// Construit le texte mis en forme. Renvoie aussi les plages (en caractères du texte affiché)
/// des liens, pour les rendre cliquables.
pub fn job(text: &str, base: Base, show_markers: bool, wrap: f32) -> (LayoutJob, Vec<(Range<usize>, String)>) {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let mut link_ranges = Vec::new();
    let fmt_plain = |color: Color32| TextFormat {
        font_id: font(base.size, base.bold, base.mono),
        color,
        italics: base.italic,
        strikethrough: if base.strike { Stroke::new(1.0, color) } else { Stroke::NONE },
        line_height: Some(base.size * 1.5),
        ..Default::default()
    };
    if base.mono {
        job.append(text, 0.0, fmt_plain(base.color));
        return (job, link_ranges);
    }
    let parsed = parse(text);
    let mut chars = 0usize;
    for s in &parsed.spans {
        if s.marker && !show_markers {
            continue;
        }
        let piece = &text[s.range.clone()];
        let st = s.style;
        let mut color = if s.marker { theme::TEXT_FAINT() } else { base.color };
        let mention = s.link.is_some_and(|l| parsed.links[l].starts_with("wt:"));
        if mention && s.marker {
            // Adresse d'une mention : invisible et minuscule, même en édition.
            job.append(piece, 0.0, TextFormat { font_id: font(1.0, false, false), color: Color32::TRANSPARENT, line_height: Some(base.size * 1.5), ..Default::default() });
            chars += piece.chars().count();
            continue;
        }
        if s.link.is_some() && !s.marker {
            color = if mention { theme::TEXT() } else { theme::LINK() };
        }
        let size = if st.code { base.size * 0.88 } else { base.size };
        let mut f =
            TextFormat { font_id: font(size, base.bold || st.bold, st.code), color, italics: base.italic || st.italic, line_height: Some(base.size * 1.5), ..Default::default() };
        if st.strike || base.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        if st.underline || (s.link.is_some() && !s.marker && !mention) {
            f.underline = Stroke::new(1.0, color);
        }
        if mention {
            // Mention (mail, événement, tâche…) : pastille discrète.
            f.background = crate::theme::gray(0x26);
            f.font_id = font(size * 0.94, true, false);
        }
        if st.code {
            f.background = theme::SURFACE_HI();
            if !s.marker {
                f.color = Color32::from_rgb(0xE6, 0x9A, 0x84);
            }
        } else if st.highlight {
            f.background = theme::TODAY().gamma_multiply(0.35);
        }
        job.append(piece, 0.0, f);
        let n = piece.chars().count();
        if let Some(l) = s.link.filter(|_| !s.marker) {
            link_ranges.push((chars..chars + n, parsed.links[l].clone()));
        }
        chars += n;
    }
    (job, link_ranges)
}

/// Texte sans marqueurs (aperçus).
pub fn strip(text: &str) -> String {
    let p = parse(text);
    p.spans.iter().filter(|s| !s.marker).map(|s| &text[s.range.clone()]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_markers() {
        assert_eq!(strip("du **gras**, de l'*italique* et `code`"), "du gras, de l'italique et code");
        assert_eq!(strip("~~non~~ ==oui== <u>sous</u>"), "non oui sous");
        assert_eq!(strip("voir [le site](https://a.b) ici"), "voir le site ici");
        // Marqueurs isolés : laissés tels quels.
        assert_eq!(strip("2 * 3 = 6 et snake_case_name"), "2 * 3 = 6 et snake_case_name");
        assert_eq!(strip("**ouvert sans fin"), "**ouvert sans fin");
    }

    #[test]
    fn styles() {
        let p = parse("a **b** c");
        let bold: Vec<_> = p.spans.iter().filter(|s| s.style.bold && !s.marker).map(|s| s.range.clone()).collect();
        assert_eq!(bold, vec![4..5]);
        let p = parse("[x](u)");
        assert_eq!(p.links, ["u"]);
    }
}
