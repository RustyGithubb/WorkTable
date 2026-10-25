//! Fichiers « graph » du Blueprint : un fichier JSON par graph dans `data\blueprints\`.
//!
//! ```json
//! { "id": "…", "name": "Mails urgents", "favorite": false, "created": 0, "updated": 0,
//!   "view": { "pan": [0, 0], "zoom": 1 }, "nodes": [], "links": [] }
//! ```
//! Chaque node a un identifiant, un type (voir `nodes::ALL`), une position et les valeurs de
//! ses champs ; chaque lien relie une sortie d'une node à une entrée d'une autre.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Position de la caméra dans le graph (point au centre de la vue) et zoom.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub pan: [f32; 2],
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { pan: [0.0, 0.0], zoom: 1.0 }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Graph {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub favorite: bool,
    /// Le graph s'exécute pour de vrai à l'arrivée des mails, messages et rendez-vous.
    #[serde(default)]
    pub active: bool,
    #[serde(default)]
    pub created: i64,
    #[serde(default)]
    pub updated: i64,
    #[serde(default)]
    pub view: View,
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub links: Vec<Link>,
    /// Données d'exemple de la simulation.
    #[serde(default)]
    pub sample: Sample,
    /// Variables gardées d'une exécution réelle à l'autre (« Définir une variable »…).
    #[serde(default)]
    pub memory: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub id: u64,
    /// Type de node (`nodes::NodeDef::key`).
    pub kind: String,
    /// Coin haut gauche, en coordonnées du graph.
    pub pos: [f32; 2],
    /// Valeur des champs (clé de la pin → valeur), quand elle diffère de celle par défaut.
    #[serde(default)]
    pub values: BTreeMap<String, serde_json::Value>,
}

/// Fil d'une sortie (`from`, `out`) vers une entrée (`to`, `input`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: u64,
    pub out: String,
    pub to: u64,
    pub input: String,
}

/// Faux mail, message et rendez-vous utilisés par « Simuler » (modifiables).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sample {
    pub mail_from_name: String,
    pub mail_from_email: String,
    /// Adresse de groupe par laquelle le mail est arrivé (vide : directement).
    pub mail_via: String,
    pub mail_subject: String,
    pub mail_body: String,
    /// Destinataires, séparés par des virgules.
    pub mail_to: String,
    pub chat_author: String,
    pub chat_author_email: String,
    pub chat_space: String,
    pub chat_text: String,
    pub event_title: String,
    pub event_start: String,
    pub event_location: String,
    pub reminder_title: String,
    pub reminder_due: String,
}

impl Default for Sample {
    fn default() -> Self {
        Self {
            mail_from_name: "Camille Martin".into(),
            mail_from_email: "camille.martin@exemple.fr".into(),
            mail_via: String::new(),
            mail_subject: "Urgent : réunion de projet demain".into(),
            mail_body: "Bonjour, peux-tu confirmer ta présence à la réunion de demain ? Merci !".into(),
            mail_to: "moi@exemple.fr, equipe@exemple.fr".into(),
            chat_author: "Léo Bernard".into(),
            chat_author_email: "leo.bernard@exemple.fr".into(),
            chat_space: "Projet WorkTable".into(),
            chat_text: "Tu as vu le dernier build ?".into(),
            event_title: "Point hebdo".into(),
            event_start: "lundi 10:00".into(),
            event_location: "Salle 2".into(),
            reminder_title: "Appeler le fournisseur".into(),
            reminder_due: "15:30".into(),
        }
    }
}

impl Node {
    pub fn def(&self) -> Option<&'static super::nodes::NodeDef> {
        super::nodes::get(&self.kind)
    }

    /// Valeur d'un champ (celle par défaut si rien n'a été saisi).
    pub fn value(&self, key: &str) -> serde_json::Value {
        if let Some(v) = self.values.get(key) {
            return v.clone();
        }
        let def = self.def().and_then(|d| d.input(key));
        def.map(|p| super::nodes::default_value(p.widget)).unwrap_or(serde_json::Value::Null)
    }

    pub fn text(&self, key: &str) -> String {
        match self.value(key) {
            serde_json::Value::String(s) => s,
            serde_json::Value::Null => String::new(),
            v => v.to_string(),
        }
    }

    pub fn flag(&self, key: &str) -> bool {
        self.value(key).as_bool().unwrap_or(false)
    }

    pub fn int(&self, key: &str) -> i64 {
        self.value(key).as_i64().unwrap_or(0)
    }
}

pub fn dir() -> PathBuf {
    crate::paths::data().join("blueprints")
}

fn now() -> i64 {
    chrono::Local::now().timestamp_millis()
}

/// Identifiant unique : date en millisecondes + compteur (deux créations dans la même ms).
fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!("{:x}{:02x}", now(), N.fetch_add(1, Ordering::Relaxed) % 256)
}

impl Graph {
    pub fn new(name: &str) -> Self {
        let t = now();
        Self {
            id: new_id(),
            name: name.to_string(),
            favorite: false,
            active: false,
            created: t,
            updated: t,
            view: View::default(),
            nodes: Vec::new(),
            links: Vec::new(),
            sample: Sample::default(),
            memory: BTreeMap::new(),
        }
    }

    pub fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn node_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    fn next_id(&self) -> u64 {
        self.nodes.iter().map(|n| n.id).max().unwrap_or(0) + 1
    }

    /// Ajoute une node du type `kind` à la position `pos` ; renvoie son identifiant.
    pub fn add_node(&mut self, kind: &str, pos: [f32; 2]) -> u64 {
        let id = self.next_id();
        self.nodes.push(Node { id, kind: kind.to_string(), pos, values: BTreeMap::new() });
        id
    }

    /// Copies des nodes `ids` (décalées), avec les fils qui les relient entre elles.
    /// Renvoie les identifiants des copies.
    pub fn duplicate_nodes(&mut self, ids: &[u64]) -> Vec<u64> {
        let mut map = BTreeMap::new();
        let mut next = self.next_id();
        let copies: Vec<Node> = self
            .nodes
            .iter()
            .filter(|n| ids.contains(&n.id))
            .map(|n| {
                map.insert(n.id, next);
                let c = Node { id: next, kind: n.kind.clone(), pos: [n.pos[0] + 32.0, n.pos[1] + 32.0], values: n.values.clone() };
                next += 1;
                c
            })
            .collect();
        let links: Vec<Link> =
            self.links.iter().filter_map(|l| Some(Link { from: *map.get(&l.from)?, out: l.out.clone(), to: *map.get(&l.to)?, input: l.input.clone() })).collect();
        self.nodes.extend(copies);
        self.links.extend(links);
        map.into_values().collect()
    }

    pub fn remove_nodes(&mut self, ids: &[u64]) {
        self.nodes.retain(|n| !ids.contains(&n.id));
        self.links.retain(|l| !ids.contains(&l.from) && !ids.contains(&l.to));
    }

    /// Fils qui partent de la sortie (`from`, `out`).
    pub fn links_from<'a>(&'a self, from: u64, out: &'a str) -> impl Iterator<Item = &'a Link> + 'a {
        self.links.iter().filter(move |l| l.from == from && l.out == out)
    }

    /// Une pin est-elle reliée ?
    pub fn is_linked(&self, node: u64, pin: &str, output: bool) -> bool {
        self.links.iter().any(|l| if output { l.from == node && l.out == pin } else { l.to == node && l.input == pin })
    }

    /// Relie une sortie à une entrée, si les types le permettent. Comme dans Unreal : une entrée
    /// de données et une sortie d'exécution n'ont qu'un fil (le nouveau remplace l'ancien).
    pub fn connect(&mut self, from: u64, out: &str, to: u64, input: &str) -> bool {
        use super::nodes::PinType;
        if from == to {
            return false;
        }
        let (Some(a), Some(b)) = (self.node(from).and_then(|n| n.def()), self.node(to).and_then(|n| n.def())) else { return false };
        let (Some(po), Some(pi)) = (a.output(out), b.input(input)) else { return false };
        if !pi.wire || !pi.ty.accepts(po.ty) {
            return false;
        }
        if po.ty == PinType::Exec {
            self.links.retain(|l| !(l.from == from && l.out == out));
        } else {
            self.links.retain(|l| !(l.to == to && l.input == input));
        }
        self.links.push(Link { from, out: out.to_string(), to, input: input.to_string() });
        true
    }

    /// Retire tous les fils d'une pin.
    pub fn break_links(&mut self, node: u64, pin: &str, output: bool) {
        self.links.retain(|l| if output { !(l.from == node && l.out == pin) } else { !(l.to == node && l.input == pin) });
    }

    /// Copie sous un nouveau nom (nouvel identifiant, pas favori).
    pub fn duplicate(&self, name: &str) -> Self {
        let t = now();
        Self { id: new_id(), name: name.to_string(), favorite: false, active: false, created: t, updated: t, ..self.clone() }
    }

    fn path(&self) -> PathBuf {
        dir().join(format!("{}.json", self.id))
    }

    /// `touch` : change la date de modification (pas pour la caméra ni les favoris).
    pub fn save(&mut self, touch: bool) {
        if touch {
            self.updated = now();
        }
        crate::paths::save_json(&self.path(), self);
    }

    pub fn delete(&self) {
        let _ = std::fs::remove_file(self.path());
    }

    pub fn load_all() -> Vec<Graph> {
        let Ok(rd) = std::fs::read_dir(dir()) else { return Vec::new() };
        rd.flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| serde_json::from_str::<Graph>(&std::fs::read_to_string(e.path()).ok()?).ok())
            .collect()
    }
}

/// État des nodes et des fils, pour annuler / rétablir (ordre des nodes ignoré : passer une
/// node au premier plan n'est pas une modification).
#[derive(Clone, PartialEq)]
pub struct Snap {
    nodes: Vec<Node>,
    links: Vec<Link>,
}

impl Snap {
    pub fn of(g: &Graph) -> Self {
        let mut nodes = g.nodes.clone();
        nodes.sort_by_key(|n| n.id);
        let mut links = g.links.clone();
        links.sort_by(|a, b| (a.from, &a.out, a.to, &a.input).cmp(&(b.from, &b.out, b.to, &b.input)));
        Snap { nodes, links }
    }

    pub fn apply(&self, g: &mut Graph) {
        g.nodes = self.nodes.clone();
        g.links = self.links.clone();
    }
}

/// Nodes copiées (Ctrl + C), avec les fils qui les relient entre elles. Positions relatives au
/// coin haut gauche de la sélection.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Clip {
    /// Marque du presse-papiers WorkTable.
    pub worktable_blueprint: u32,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
}

impl Clip {
    pub fn of(g: &Graph, ids: &[u64]) -> Option<Self> {
        let nodes: Vec<Node> = g.nodes.iter().filter(|n| ids.contains(&n.id)).cloned().collect();
        let x = nodes.iter().map(|n| n.pos[0]).fold(f32::INFINITY, f32::min);
        let y = nodes.iter().map(|n| n.pos[1]).fold(f32::INFINITY, f32::min);
        if nodes.is_empty() {
            return None;
        }
        let nodes = nodes.into_iter().map(|n| Node { pos: [n.pos[0] - x, n.pos[1] - y], ..n }).collect();
        let links = g.links.iter().filter(|l| ids.contains(&l.from) && ids.contains(&l.to)).cloned().collect();
        Some(Clip { worktable_blueprint: 1, nodes, links })
    }

    /// Presse-papiers du système : seulement s'il vient du Blueprint.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_str::<Clip>(text.trim()).ok().filter(|c| c.worktable_blueprint == 1 && !c.nodes.is_empty())
    }
}

impl Graph {
    /// Colle `clip` avec son coin haut gauche en `at` (nouveaux identifiants) ; renvoie les
    /// nouvelles nodes. Les types inconnus sont ignorés.
    pub fn paste(&mut self, clip: &Clip, at: [f32; 2]) -> Vec<u64> {
        let mut map = BTreeMap::new();
        for (next, n) in (self.next_id()..).zip(clip.nodes.iter().filter(|n| n.def().is_some())) {
            map.insert(n.id, next);
            self.nodes.push(Node { id: next, pos: [at[0] + n.pos[0], at[1] + n.pos[1]], ..n.clone() });
        }
        for l in &clip.links {
            if let (Some(&from), Some(&to)) = (map.get(&l.from), map.get(&l.to)) {
                self.links.push(Link { from, out: l.out.clone(), to, input: l.input.clone() });
            }
        }
        map.into_values().collect()
    }
}

/// Nom libre : « Nouveau graph », puis « Nouveau graph 2 », « 3 »…
pub fn unique_name(base: &str, taken: &[Graph]) -> String {
    let used = |n: &str| taken.iter().any(|g| g.name.eq_ignore_ascii_case(n));
    if !used(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !used(n)).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        let g = vec![Graph::new("Nouveau graph"), Graph::new("Nouveau graph 2")];
        assert_eq!(unique_name("Nouveau graph", &g), "Nouveau graph 3");
        assert_eq!(unique_name("Autre", &g), "Autre");
    }

    #[test]
    fn ids_differ() {
        assert_ne!(Graph::new("a").id, Graph::new("a").id);
    }

    #[test]
    fn links_follow_unreal_rules() {
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let a = g.add_node("act_notify", [300.0, 0.0]);
        let b = g.add_node("act_task", [300.0, 200.0]);
        assert!(g.connect(ev, "then", a, "exec"));
        // Une sortie d'exécution n'a qu'un fil : le nouveau remplace l'ancien.
        assert!(g.connect(ev, "then", b, "exec"));
        assert_eq!(g.links_from(ev, "then").count(), 1);
        // Une sortie de données peut alimenter plusieurs entrées ; tout se convertit en texte.
        assert!(g.connect(ev, "subject", a, "title"));
        assert!(g.connect(ev, "mail", b, "title"));
        // Types incompatibles, ou exécution vers donnée : refusé.
        assert!(!g.connect(ev, "subject", a, "exec"));
        let pin = g.add_node("act_pin", [0.0, 0.0]);
        assert!(!g.connect(ev, "subject", pin, "item"));
        assert!(g.connect(ev, "mail", pin, "item"));
        g.remove_nodes(&[ev]);
        assert!(g.links.is_empty());
    }

    #[test]
    fn duplicates_keep_inner_links() {
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let a = g.add_node("act_notify", [300.0, 0.0]);
        g.connect(ev, "then", a, "exec");
        let copies = g.duplicate_nodes(&[ev, a]);
        assert_eq!(copies.len(), 2);
        assert_eq!(g.links.len(), 2);
    }

    #[test]
    fn copy_paste_and_snapshots() {
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [100.0, 100.0]);
        let a = g.add_node("act_notify", [400.0, 100.0]);
        g.connect(ev, "then", a, "exec");
        let before = Snap::of(&g);
        let clip = Clip::parse(&serde_json::to_string(&Clip::of(&g, &[ev, a]).unwrap()).unwrap()).unwrap();
        let mut other = Graph::new("u");
        let ids = other.paste(&clip, [0.0, 0.0]);
        assert_eq!(ids.len(), 2);
        assert_eq!(other.links.len(), 1);
        assert_eq!(other.nodes[0].pos, [0.0, 0.0]);
        assert_eq!(other.nodes[1].pos, [300.0, 0.0]);
        assert!(Clip::parse("du texte").is_none());
        // Annuler : l'état d'avant revient ; passer une node devant n'est pas une modification.
        g.remove_nodes(&[a]);
        assert!(Snap::of(&g) != before);
        before.apply(&mut g);
        assert_eq!(g.nodes.len(), 2);
        g.nodes.reverse();
        assert!(Snap::of(&g) == before);
    }

    #[test]
    fn reads_old_or_partial_files() {
        let g: Graph = serde_json::from_str(r#"{"id":"x","name":"Test","nodes":[],"links":[]}"#).unwrap();
        assert_eq!(g.view, View::default());
        assert!(g.nodes.is_empty());
    }
}
