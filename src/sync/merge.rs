//! Fusion de deux versions d'un fichier JSON modifié sur deux postes (`tasks.json`,
//! `contacts.json`, `prefs.json`…), à partir de leur ancêtre commun (dernière version synchronisée).
//!
//! - Objets : fusion clé par clé ; une clé changée d'un seul côté prend ce changement.
//! - Listes d'objets ayant tous un identifiant (`id`, `email` ou `name`) : fusion élément par
//!   élément ; ajouts des deux côtés gardés, suppression d'un côté appliquée si l'autre n'a pas
//!   modifié l'élément.
//! - Valeur changée des deux côtés : celle de ce poste (`local`) l'emporte.

use serde_json::{Map, Value};

/// Champs servant d'identifiant aux éléments d'une liste, dans l'ordre de préférence.
const KEYS: [&str; 3] = ["id", "email", "name"];

/// Fusion à trois voies. `base` : `None` si le fichier n'existait pas à la dernière synchro.
pub fn merge(base: Option<&Value>, local: &Value, remote: &Value) -> Value {
    if local == remote {
        return local.clone();
    }
    match base {
        Some(b) if b == local => return remote.clone(),
        Some(b) if b == remote => return local.clone(),
        _ => {}
    }
    match (local, remote) {
        (Value::Object(l), Value::Object(r)) => {
            let empty = Map::new();
            let b = base.and_then(Value::as_object).unwrap_or(&empty);
            Value::Object(merge_objects(b, l, r))
        }
        (Value::Array(l), Value::Array(r)) => {
            let b: &[Value] = base.and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]);
            match key_of(b, l, r) {
                Some(k) => Value::Array(merge_lists(k, b, l, r)),
                None => local.clone(),
            }
        }
        _ => local.clone(),
    }
}

fn merge_objects(b: &Map<String, Value>, l: &Map<String, Value>, r: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    // Ordre : celui de ce poste, puis les clés apparues ailleurs.
    let keys = l.keys().chain(r.keys().filter(|k| !l.contains_key(*k))).cloned().collect::<Vec<_>>();
    for k in keys {
        match (b.get(&k), l.get(&k), r.get(&k)) {
            (_, Some(lv), Some(rv)) => {
                out.insert(k.clone(), merge(b.get(&k), lv, rv));
            }
            // Supprimée ici : gardée seulement si l'autre poste l'a modifiée.
            (Some(bv), None, Some(rv)) => {
                if bv != rv {
                    out.insert(k.clone(), rv.clone());
                }
            }
            (None, None, Some(rv)) => {
                out.insert(k.clone(), rv.clone());
            }
            (Some(bv), Some(lv), None) => {
                if bv != lv {
                    out.insert(k.clone(), lv.clone());
                }
            }
            (None, Some(lv), None) => {
                out.insert(k.clone(), lv.clone());
            }
            (_, None, None) => {}
        }
    }
    out
}

/// Champ identifiant commun à tous les éléments des trois listes (valeurs uniques), s'il existe.
fn key_of(b: &[Value], l: &[Value], r: &[Value]) -> Option<&'static str> {
    let all: Vec<&Value> = b.iter().chain(l).chain(r).collect();
    if all.is_empty() {
        return None;
    }
    KEYS.into_iter().find(|k| {
        all.iter().all(|v| v.get(k).is_some_and(|id| !id.is_null() && !id.is_object() && !id.is_array()))
            && [b, l, r].iter().all(|list| {
                let mut seen = std::collections::HashSet::new();
                list.iter().all(|v| seen.insert(v[*k].to_string()))
            })
    })
}

fn merge_lists(k: &str, b: &[Value], l: &[Value], r: &[Value]) -> Vec<Value> {
    let find = |list: &[Value], id: &Value| list.iter().find(|v| &v[k] == id).cloned();
    let mut out = Vec::new();
    // Éléments de ce poste, dans son ordre.
    for lv in l {
        let id = &lv[k];
        match (find(b, id), find(r, id)) {
            (_, Some(rv)) => out.push(merge(find(b, id).as_ref(), lv, &rv)),
            // Supprimé sur l'autre poste : gardé seulement s'il a changé ici.
            (Some(bv), None) => {
                if &bv != lv {
                    out.push(lv.clone());
                }
            }
            (None, None) => out.push(lv.clone()),
        }
    }
    // Éléments venus de l'autre poste, insérés après leur voisin d'origine.
    for (i, rv) in r.iter().enumerate() {
        let id = &rv[k];
        if l.iter().any(|v| &v[k] == id) {
            continue;
        }
        match find(b, id) {
            // Supprimé ici et pas modifié là-bas : reste supprimé.
            Some(bv) if &bv == rv => continue,
            _ => {}
        }
        let after = i.checked_sub(1).map(|p| &r[p][k]);
        let pos = after.and_then(|a| out.iter().position(|v| &v[k] == a)).map(|p| p + 1).unwrap_or(if i == 0 { 0 } else { out.len() });
        out.insert(pos, rv.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tasks_added_on_both_sides() {
        let base = json!({ "list": [{ "id": 1, "title": "A" }], "next_id": 1 });
        let local = json!({ "list": [{ "id": 1, "title": "A" }, { "id": 2, "title": "Ici" }], "next_id": 2 });
        let remote = json!({ "list": [{ "id": 1, "title": "A modifiée" }, { "id": 3, "title": "Là-bas" }], "next_id": 3 });
        let m = merge(Some(&base), &local, &remote);
        let titles: Vec<&str> = m["list"].as_array().unwrap().iter().map(|t| t["title"].as_str().unwrap()).collect();
        // Les deux ajouts sont gardés ; celui de l'autre poste suit son voisin d'origine.
        assert_eq!(titles, ["A modifiée", "Là-bas", "Ici"]);
    }

    #[test]
    fn deletion_wins_unless_edited() {
        let base = json!([{ "id": 1, "t": "x" }, { "id": 2, "t": "y" }]);
        let local = json!([{ "id": 2, "t": "y" }]); // 1 supprimée ici
        let remote = json!([{ "id": 1, "t": "x" }, { "id": 2, "t": "y2" }]);
        assert_eq!(merge(Some(&base), &local, &remote), json!([{ "id": 2, "t": "y2" }]));
        // Supprimée ici mais modifiée là-bas : gardée.
        let remote = json!([{ "id": 1, "t": "x2" }, { "id": 2, "t": "y" }]);
        let m = merge(Some(&base), &local, &remote);
        assert_eq!(m.as_array().unwrap().len(), 2);
        assert!(m.as_array().unwrap().iter().any(|v| v["t"] == "x2"));
    }

    #[test]
    fn same_field_changed_twice_keeps_local() {
        let base = json!({ "theme": "a", "lang": "fr" });
        let local = json!({ "theme": "b", "lang": "fr" });
        let remote = json!({ "theme": "c", "lang": "en" });
        assert_eq!(merge(Some(&base), &local, &remote), json!({ "theme": "b", "lang": "en" }));
    }

    #[test]
    fn contacts_by_email() {
        let base = json!({ "contacts": [] });
        let local = json!({ "contacts": [{ "email": "a@x.fr", "count": 1 }] });
        let remote = json!({ "contacts": [{ "email": "b@x.fr", "count": 2 }] });
        let m = merge(Some(&base), &local, &remote);
        assert_eq!(m["contacts"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn lists_without_ids_keep_local() {
        let base = json!([1, 2]);
        assert_eq!(merge(Some(&base), &json!([1, 2, 3]), &json!([0, 1, 2])), json!([1, 2, 3]));
        assert_eq!(merge(Some(&base), &json!([1, 2]), &json!([0, 1, 2])), json!([0, 1, 2]));
    }
}
