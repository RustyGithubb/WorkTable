//! Mise en forme des messages Google Chat : *gras*, _italique_, ~barré~, `code`,
//! ```bloc de code```, listes « * » / « - », citations « > », liens <url|texte>
//! et mentions <users/…> (affichées « @Nom »).

use crate::google::html::{Para, Run, linkify};

/// Couleur des mentions (@Nom).
const MENTION: [u8; 3] = [0x8A, 0xB4, 0xF8];

/// Markdown courant → syntaxe Google Chat : **gras** → *gras*, __italique__ → _italique_,
/// ~~barré~~ → ~barré~, [texte](https://…) → <https://…|texte>. Les blocs de code sont laissés tels quels.
pub fn markdown_to_chat(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, part) in text.split("```").enumerate() {
        if i > 0 {
            out.push_str("```");
        }
        if i % 2 == 1 {
            out.push_str(part);
            continue;
        }
        let mut t = part.replace("**", "*").replace("~~", "~").replace("__", "_");
        // Liens [texte](url)
        let mut res = String::new();
        while let Some(a) = t.find('[') {
            let rest = &t[a..];
            let parsed = rest.find("](").and_then(|m| rest[m + 2..].find(')').map(|e| (m, m + 2 + e)));
            match parsed {
                Some((m, e)) if !rest[1..m].contains('\n') && rest[m + 2..e].starts_with("http") => {
                    res.push_str(&t[..a]);
                    res.push_str(&format!("<{}|{}>", &rest[m + 2..e], &rest[1..m]));
                    t = rest[e + 1..].to_string();
                }
                _ => {
                    res.push_str(&t[..a + 1]);
                    t = t[a + 1..].to_string();
                }
            }
        }
        res.push_str(&t);
        out.push_str(&res);
    }
    out
}

pub fn to_paras(text: &str, name_of: &dyn Fn(&str) -> String) -> Vec<Para> {
    let mut out = Vec::new();
    let mut in_code = false;
    let text = markdown_to_chat(text);
    for line in text.replace("\r\n", "\n").split('\n') {
        let trimmed = line.trim();
        // Bloc de code ``` … ``` (sur une ou plusieurs lignes).
        if trimmed.starts_with("```") {
            let inner = trimmed.trim_start_matches("```");
            if !in_code && inner.ends_with("```") && inner.len() >= 3 {
                out.push(code_para(inner.trim_end_matches("```")));
                continue;
            }
            in_code = !in_code;
            if !inner.is_empty() && in_code {
                out.push(code_para(inner));
            }
            continue;
        }
        if in_code {
            out.push(code_para(line));
            continue;
        }
        let mut para = Para::default();
        let mut rest = line;
        while let Some(r) = rest.strip_prefix("> ").or_else(|| rest.strip_prefix('>')) {
            para.quote += 1;
            rest = r;
        }
        let t = rest.trim_start();
        let mut prefix = None;
        for b in ["* ", "- ", "• "] {
            if let Some(r) = t.strip_prefix(b) {
                para.indent = 1;
                prefix = Some(r);
                break;
            }
        }
        let body = match prefix {
            Some(r) => {
                para.runs.push(Run { text: "•  ".into(), ..Default::default() });
                r
            }
            None => rest,
        };
        if body.is_empty() && para.runs.is_empty() {
            out.push(para);
            continue;
        }
        for r in inline(body, &Run::default(), name_of) {
            linkify(r, &mut para.runs);
        }
        // Liens en bleu souligné (même rendu que les liens de téléchargement des mails).
        for r in para.runs.iter_mut().filter(|r| r.link.is_some()) {
            r.download = true;
        }
        out.push(para);
    }
    // Pas de paragraphe vide en fin de message.
    while out.last().is_some_and(|p: &Para| p.runs.is_empty()) {
        out.pop();
    }
    out
}

fn code_para(text: &str) -> Para {
    Para { runs: vec![Run { text: text.to_string(), code: true, ..Default::default() }], ..Default::default() }
}

/// Découpe une ligne en segments mis en forme (récursif : *_gras italique_*).
fn inline(text: &str, base: &Run, name_of: &dyn Fn(&str) -> String) -> Vec<Run> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<Run> = Vec::new();
    let mut buf = String::new();
    let push_text = |buf: &mut String, out: &mut Vec<Run>| {
        if !buf.is_empty() {
            out.push(Run { text: std::mem::take(buf), ..base.clone() });
        }
    };
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        // <url|texte>, <url>, <users/123>
        if c == '<'
            && let Some(end) = chars[i + 1..].iter().position(|&x| x == '>')
        {
            let inner: String = chars[i + 1..i + 1 + end].iter().collect();
            if let Some(id) = inner.strip_prefix("users/").map(|id| format!("users/{id}")) {
                push_text(&mut buf, &mut out);
                let name = if id == "users/all" { t!("tout le monde", "everyone").to_string() } else { name_of(&id) };
                out.push(Run { text: format!("@{name}"), bold: true, color: Some(MENTION), ..base.clone() });
                i += end + 2;
                continue;
            }
            if inner.starts_with("http://") || inner.starts_with("https://") || inner.starts_with("mailto:") {
                push_text(&mut buf, &mut out);
                let (url, label) = inner.split_once('|').unwrap_or((&inner, &inner));
                out.push(Run { text: label.to_string(), link: Some(url.to_string()), ..base.clone() });
                i += end + 2;
                continue;
            }
        }
        // Marqueurs de style : ouverture en début de mot, fermeture en fin de mot.
        if matches!(c, '*' | '_' | '~' | '`') {
            let opens = i + 1 < chars.len() && !chars[i + 1].is_whitespace() && (i == 0 || !chars[i - 1].is_alphanumeric());
            if opens {
                let close = (i + 2..chars.len()).find(|&j| chars[j] == c && !chars[j - 1].is_whitespace() && (j + 1 == chars.len() || !chars[j + 1].is_alphanumeric()));
                if let Some(j) = close {
                    push_text(&mut buf, &mut out);
                    let inner: String = chars[i + 1..j].iter().collect();
                    let mut style = base.clone();
                    match c {
                        '*' => style.bold = true,
                        '_' => style.italic = true,
                        '~' => style.strike = true,
                        _ => style.code = true,
                    }
                    if c == '`' {
                        out.push(Run { text: inner, ..style });
                    } else {
                        out.extend(inline(&inner, &style, name_of));
                    }
                    i = j + 1;
                    continue;
                }
            }
        }
        buf.push(c);
        i += 1;
    }
    push_text(&mut buf, &mut out);
    out
}

/// Texte brut d'un message (aperçu dans la liste des conversations).
pub fn preview(text: &str, name_of: &dyn Fn(&str) -> String) -> String {
    to_paras(text, name_of).iter().map(|p| p.runs.iter().map(|r| r.text.as_str()).collect::<String>()).collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(t: &str) -> Vec<Run> {
        to_paras(t, &|id| format!("N{id}")).into_iter().flat_map(|p| p.runs).collect()
    }

    #[test]
    fn chat_markup() {
        let r = runs("Salut *Léa* et _toi_ ~non~ `x`");
        assert!(r.iter().any(|x| x.text == "Léa" && x.bold));
        assert!(r.iter().any(|x| x.text == "toi" && x.italic));
        assert!(r.iter().any(|x| x.text == "non" && x.strike));
        assert!(r.iter().any(|x| x.text == "x" && x.code));
    }

    #[test]
    fn mentions_and_links() {
        let r = runs("<users/42> regarde <https://a.fr|ça> et https://b.fr");
        assert_eq!(r[0].text, "@Nusers/42");
        assert!(r.iter().any(|x| x.text == "ça" && x.link.as_deref() == Some("https://a.fr")));
        assert!(r.iter().any(|x| x.link.as_deref() == Some("https://b.fr")));
    }

    #[test]
    fn markdown_is_converted() {
        assert_eq!(markdown_to_chat("**gras** et ~~non~~ [site](https://a.fr)"), "*gras* et ~non~ <https://a.fr|site>");
        assert_eq!(markdown_to_chat("```**brut**```"), "```**brut**```");
    }

    #[test]
    fn not_markup_inside_words() {
        let r = runs("snake_case_name 2*3*4");
        assert!(r.iter().all(|x| !x.italic && !x.bold));
    }
}
