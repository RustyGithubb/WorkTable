//! Synchronisation de la copie locale avec le serveur, en un minimum de requêtes.
//!
//! - Chaque champ est demandé pour **toutes** les clés d'un coup (opérations `…TableauDe…`).
//! - Séances : `DetailDesSeancesPlaceesTableauDeCours` donne matière, durée, enseignants, salles,
//!   promotions, TD, regroupements, type et début de toutes les séances en un appel.
//! - Première synchronisation : ~20 requêtes. Ensuite : seuls les cours modifiés depuis la
//!   dernière fois (`CoursAvecDerniereModificationEntre2Dates`) et les cours supprimés
//!   (`ClesCoursInvalidesTableauDeCours`) sont relus, soit ~4 requêtes.
//! - Référentiels (matières, enseignants, salles…) relus une fois par jour.

use super::cache::{Cours, Salle, Seance, Snapshot};
use super::soap::{Client, arg_list, elements};
use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use std::collections::{BTreeMap, BTreeSet};

/// Référentiels relus au-delà de ce délai (secondes).
const REFS_MAX_AGE: i64 = 24 * 3600;
/// Nombre de cours demandés par requête (réponses de taille raisonnable).
const CHUNK: usize = 500;

fn now() -> i64 {
    Local::now().timestamp()
}

fn today() -> NaiveDate {
    Local::now().date_naive()
}

fn date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn keys(body: &str, item: &str) -> Vec<u32> {
    elements(body, item).iter().filter_map(|k| k.trim().parse().ok()).collect()
}

/// Liste de clés (sans paramètre) : `ToutesLesSalles`, `TousLesEnseignants`…
fn all(c: &Client, iface: &str, op: &str, item: &str) -> Result<Vec<u32>, String> {
    Ok(keys(&c.call(iface, op, &[])?, item))
}

/// Opération `…TableauDe…` : un résultat par clé, dans l'ordre (`out` : élément des résultats).
fn per_key(c: &Client, iface: &str, op: &str, item: &str, ks: &[u32], out: &str) -> Result<Vec<String>, String> {
    if ks.is_empty() {
        return Ok(Vec::new());
    }
    let r = elements(&c.call(iface, op, &[("ATableau", arg_list(item, ks))])?, out);
    if r.len() != ks.len() {
        return Err(tf!("{op} : réponse incomplète ({} sur {})", "{op}: incomplete response ({} of {})", r.len(), ks.len()));
    }
    Ok(r)
}

/// Clés filtrées par une opération `Restreindre…` / `Cles…Invalides…`.
fn restrict(c: &Client, iface: &str, op: &str, item: &str, ks: &[u32]) -> Result<Vec<u32>, String> {
    if ks.is_empty() {
        return Ok(Vec::new());
    }
    Ok(keys(&c.call(iface, op, &[("ATableau", arg_list(item, ks))])?, item))
}

fn names(c: &Client, iface: &str, all_op: &str, names_op: &str, item: &str) -> Result<BTreeMap<u32, String>, String> {
    let ks = all(c, iface, all_op, item)?;
    let ns = per_key(c, iface, names_op, item, &ks, "string")?;
    Ok(ks.into_iter().zip(ns).collect())
}

/// Matières, enseignants, salles, promotions, TD/options, regroupements (16 requêtes).
fn refs(c: &Client, snap: &mut Snapshot) -> Result<(), String> {
    snap.matieres = names(c, "Matieres", "ToutesLesMatieres", "LibellesTableauDeMatieres", "THpSvcWCleMatiere")?;

    let ens = all(c, "Enseignants", "TousLesEnseignants", "THpSvcWCleEnseignant")?;
    let noms = per_key(c, "Enseignants", "NomsTableauDEnseignants", "THpSvcWCleEnseignant", &ens, "string")?;
    let prenoms = per_key(c, "Enseignants", "PrenomsTableauDEnseignants", "THpSvcWCleEnseignant", &ens, "string")?;
    snap.enseignants = ens.into_iter().zip(noms.into_iter().zip(prenoms)).map(|(k, (n, p))| (k, format!("{n} {p}").trim().to_string())).collect();

    const S: &str = "THpSvcWCleSalle";
    let salles = all(c, "Salles", "ToutesLesSalles", S)?;
    let noms = per_key(c, "Salles", "NomsTableauDeSalles", S, &salles, "string")?;
    let caps = per_key(c, "Salles", "CapacitesTableauDeSalles", S, &salles, "unsignedInt")?;
    let groups: BTreeSet<u32> = restrict(c, "Salles", "RestreindreTableauDeSallesAuxGroupesDeSalles", S, &salles)?.into_iter().collect();
    let members = restrict(c, "Salles", "RestreindreTableauDeSallesAuxSallesDansUnGroupeDeSalles", S, &salles)?;
    let parent: BTreeMap<u32, u32> = if members.is_empty() {
        BTreeMap::new()
    } else {
        let g = keys(&c.call("Salles", "GroupesDeSallesTableauDeSalles", &[("ATableau", arg_list(S, &members))])?, S);
        members.into_iter().zip(g).collect()
    };
    snap.salles = salles
        .into_iter()
        .zip(noms.into_iter().zip(caps))
        .map(|(k, (name, cap))| (k, Salle { name, capacity: cap.trim().parse().unwrap_or(0), is_group: groups.contains(&k), group: parent.get(&k).copied() }))
        .collect();

    snap.promotions = names(c, "Promotions", "ToutesLesPromotions", "NomsTableauDePromotions", "THpSvcWClePromotion")?;
    snap.tdoptions = names(c, "TDOptions", "TousLesTDOptions", "NomsTableauDeTDOptions", "THpSvcWCleTDOption")?;
    snap.regroupements = names(c, "Regroupements", "TousLesRegroupements", "NomsTableauDeRegroupements", "THpSvcWCleRegroupement")?;
    snap.refs_at = now();
    Ok(())
}

/// Début de séance : `2026-11-02T09:00:00` (fractions de seconde ou fuseau éventuels ignorés).
fn parse_start(s: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(s.trim().get(..19)?, "%Y-%m-%dT%H:%M:%S").ok()
}

/// Séance décrite par un élément `THpSvcWTypeSeance` (renvoie aussi la clé de son cours).
fn parse_seance(x: &str) -> Option<(u32, Seance)> {
    let one = |tag: &str| elements(x, tag).into_iter().next().unwrap_or_default();
    let cours = one("Cours").trim().parse().ok()?;
    // Durée en jours (2 h = 2/24).
    let days: f64 = one("Duree").trim().parse().ok()?;
    Some((
        cours,
        Seance {
            start: parse_start(&one("JourEtHeureDebut"))?,
            minutes: (days * 1440.0).round().max(0.0) as u32,
            matiere: one("Matiere").trim().parse().unwrap_or(0),
            type_cours: one("TypeCours"),
            enseignants: keys(x, "THpSvcWCleEnseignant"),
            salles: keys(x, "THpSvcWCleSalle"),
            promotions: keys(x, "THpSvcWClePromotion"),
            tdoptions: keys(x, "THpSvcWCleTDOption"),
            regroupements: keys(x, "THpSvcWCleRegroupement"),
        },
    ))
}

/// Séances et mémos des cours `ks` (3 requêtes par tranche de [`CHUNK`] cours).
fn fetch_courses(c: &Client, ks: &[u32]) -> Result<BTreeMap<u32, Cours>, String> {
    const C: &str = "THpSvcWCleCours";
    let mut out: BTreeMap<u32, Cours> = BTreeMap::new();
    for chunk in ks.chunks(CHUNK) {
        // Le détail échoue si un cours n'est pas placé : on ne garde que les cours placés.
        let placed = restrict(c, "Cours", "RestreindreTableauDeCoursAuxCoursPlaces", C, chunk)?;
        if placed.is_empty() {
            continue;
        }
        let body = c.call("Cours", "DetailDesSeancesPlaceesTableauDeCours", &[("ATableau", arg_list(C, &placed))])?;
        for x in elements(&body, "THpSvcWTypeSeance") {
            if let Some((k, s)) = parse_seance(&x) {
                let list = &mut out.entry(k).or_default().seances;
                if !list.contains(&s) {
                    list.push(s);
                }
            }
        }
        for (k, memo) in placed.iter().zip(per_key(c, "Cours", "MemosTableauDeCours", C, &placed, "string")?) {
            if !memo.trim().is_empty() {
                out.entry(*k).or_default().memo = memo;
            }
        }
    }
    for cours in out.values_mut() {
        cours.seances.sort_by_key(|s| s.start);
    }
    Ok(out)
}

fn full(c: &Client, account: &str) -> Result<Snapshot, String> {
    let mut snap = Snapshot { account: account.to_string(), ..Default::default() };
    refs(c, &mut snap)?;
    let first = c.call_value("Admin", "DatePremierJourBase", &[])?;
    let start = NaiveDate::parse_from_str(first.trim().get(..10).unwrap_or(""), "%Y-%m-%d").unwrap_or_else(|_| today() - Duration::days(120));
    let end = start + Duration::days(371);
    let ks = keys(&c.call("Cours", "TousLesCoursEntre2Dates", &[("ADate1", date(start)), ("ADate2", date(end))])?, "THpSvcWCleCours");
    snap.cours = fetch_courses(c, &ks)?;
    snap.window = Some((start, end));
    Ok(snap)
}

fn incremental(c: &Client, mut snap: Snapshot, since: NaiveDate) -> Result<Snapshot, String> {
    const C: &str = "THpSvcWCleCours";
    if now() - snap.refs_at > REFS_MAX_AGE {
        refs(c, &mut snap)?;
    }
    let cached: Vec<u32> = snap.cours.keys().copied().collect();
    for k in restrict(c, "Cours", "ClesCoursInvalidesTableauDeCours", C, &cached)? {
        snap.cours.remove(&k);
    }
    let modified = keys(&c.call("Cours", "CoursAvecDerniereModificationEntre2Dates", &[("ADate1", date(since)), ("ADate2", date(today() + Duration::days(1)))])?, C);
    if !modified.is_empty() {
        for k in &modified {
            snap.cours.remove(k);
        }
        snap.cours.extend(fetch_courses(c, &modified)?);
    }
    Ok(snap)
}

/// Met la copie à jour (complète si elle est vide, d'un autre compte ou d'une autre année).
/// Renvoie la nouvelle copie et le nombre de requêtes envoyées.
pub fn run(c: &Client, account: &str, snap: Snapshot) -> Result<(Snapshot, u32), String> {
    let before = c.calls();
    let reusable = snap.account == account && snap.window.is_some_and(|(_, end)| today() < end);
    let mut snap = match snap.synced_day {
        Some(since) if reusable => incremental(c, snap, since)?,
        _ => full(c, account)?,
    };
    snap.synced_at = now();
    snap.synced_day = Some(today());
    Ok((snap, c.calls() - before))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seance_is_parsed() {
        let x = r#"<Matiere>7</Matiere><Duree>0.125</Duree><TableauEnseignant><hp:THpSvcWCleEnseignant>3</hp:THpSvcWCleEnseignant><hp:THpSvcWCleEnseignant>4</hp:THpSvcWCleEnseignant></TableauEnseignant><TableauRegroupement/><TableauPromotion><hp:THpSvcWClePromotion>9</hp:THpSvcWClePromotion></TableauPromotion><TableauTDOption/><TableauSalle><hp:THpSvcWCleSalle>12</hp:THpSvcWCleSalle></TableauSalle><PonderationAvantApport>0</PonderationAvantApport><PonderationApresApport>0</PonderationApresApport><Cours>55</Cours><TypeCours>TD</TypeCours><JourEtHeureDebut>2026-11-02T09:00:00.000</JourEtHeureDebut>"#;
        let (k, s) = parse_seance(x).unwrap();
        assert_eq!(k, 55);
        assert_eq!(s.minutes, 180);
        assert_eq!(s.matiere, 7);
        assert_eq!(s.enseignants, vec![3, 4]);
        assert_eq!(s.salles, vec![12]);
        assert_eq!(s.promotions, vec![9]);
        assert!(s.regroupements.is_empty());
        assert_eq!(s.type_cours, "TD");
        assert_eq!(s.start.format("%d/%m %H:%M").to_string(), "02/11 09:00");
    }
}
