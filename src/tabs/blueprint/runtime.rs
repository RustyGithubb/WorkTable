//! Exécution réelle des graphs actifs : à l'arrivée d'un vrai mail, d'un vrai message Google
//! Chat, ou au début (ou quelques minutes avant) d'un rendez-vous, chaque graph actif qui a
//! l'event correspondant s'exécute. Ses actions deviennent des effets (`Applied`), que l'app
//! applique (notification, tâche, corbeille…).
//!
//! Rien ne tourne tant qu'un graph n'est pas marqué « Actif ». Les mails et messages déjà là au
//! lancement de l'app ne déclenchent rien : seulement ceux qui arrivent ensuite.

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant};

use chrono::Local;

use super::sim::{self, ChatIn, Effect, EventIn, Incoming, Line, MailIn, ReminderIn};
use super::store::Graph;
use crate::google::calendar::CalEvent;
use crate::google::chat::{self, ChatEvent, ChatMessage};
use crate::google::gmail::{Label, Summary};
use crate::google::{Account, Event};
use crate::prefs::Prefs;
use crate::tabs::calendar::CalendarState;
use crate::tabs::messages::ChatState;

/// Exécutions gardées dans l'historique.
const HISTORY: usize = 40;
/// Relève des rendez-vous qui commencent.
const CALENDAR_EVERY: Duration = Duration::from_secs(15);
/// Un rendez-vous dont l'heure de déclenchement est passée depuis moins que ça déclenche encore
/// (l'app peut avoir été occupée, ou la relève tomber juste après).
const CALENDAR_GRACE: i64 = 90_000;

/// Ce qui a déclenché une exécution (pour les actions qui visent l'élément reçu, et le clic sur
/// une notification).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Origin {
    Mail(Box<Summary>),
    Chat(Box<ChatMessage>),
    Event(Box<CalEvent>),
    Reminder(Box<crate::tasks::Timer>),
}

/// Effet à appliquer, avec le graph et ce qui l'a déclenché.
pub struct Applied {
    pub effect: Effect,
    pub origin: Origin,
}

/// Exécution réelle, pour l'historique du journal (gardé d'un lancement à l'autre).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct RunRecord {
    /// Millisecondes (epoch).
    pub at: i64,
    pub graph: String,
    pub lines: Vec<Line>,
}

impl RunRecord {
    /// « 25/10 14:30:05 » (la date seulement si ce n'est pas aujourd'hui).
    pub fn when(&self) -> String {
        use chrono::TimeZone;
        let Some(t) = Local.timestamp_millis_opt(self.at).single() else { return String::new() };
        if t.date_naive() == Local::now().date_naive() { t.format("%H:%M:%S").to_string() } else { t.format("%d/%m %H:%M:%S").to_string() }
    }
}

/// Suite d'une exécution en attente (« Attendre »), reprise à l'heure `due`.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Waiting {
    pub graph: String,
    pub event: u64,
    pub delay: u64,
    pub due: i64,
    pub inc: Incoming,
    pub origin: Origin,
}

/// Ce qui est gardé d'un lancement à l'autre.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct Saved {
    /// Dernier moment où l'app tournait (mails à rattraper au lancement suivant).
    last_seen: i64,
    runs: VecDeque<RunRecord>,
    waiting: Vec<Waiting>,
}

fn saved_path() -> std::path::PathBuf {
    crate::paths::data().join("blueprint_runs.json")
}

/// Au lancement, les mails non lus arrivés app fermée sont rattrapés, jusqu'à cette ancienneté.
const CATCH_UP: i64 = 24 * 3_600_000;
/// Enregistrement de l'état (dernier passage, journal, attentes).
const SAVE_EVERY: Duration = Duration::from_secs(30);

pub struct Engine {
    started_ms: i64,
    known_mail: HashSet<String>,
    /// Dernière activité connue de chaque conversation (None : premier passage).
    chat_seen: Option<HashMap<String, i64>>,
    /// Conversations dont on attend les nouveaux messages : depuis quand.
    chat_pending: HashMap<String, i64>,
    /// Rendez-vous déjà traités : graph / node / évènement / début.
    events_done: HashSet<String>,
    last_calendar: Option<Instant>,
    pending: Vec<Applied>,
    pub runs: VecDeque<RunRecord>,
    /// Dernier passage du lancement précédent (None : premier lancement).
    last_seen: Option<i64>,
    pub waiting: Vec<Waiting>,
    /// Graphs dont les variables ont changé (à enregistrer).
    pub touched: Vec<String>,
    last_save: Option<Instant>,
}

impl Default for Engine {
    fn default() -> Self {
        Engine {
            started_ms: Local::now().timestamp_millis(),
            known_mail: HashSet::new(),
            chat_seen: None,
            chat_pending: HashMap::new(),
            events_done: HashSet::new(),
            last_calendar: None,
            pending: Vec::new(),
            runs: VecDeque::new(),
            last_seen: None,
            waiting: Vec::new(),
            touched: Vec::new(),
            last_save: None,
        }
    }
}

/// Un graph actif a-t-il un event de ce type ?
fn wanted(graphs: &[Graph], kind: &str) -> bool {
    graphs.iter().any(|g| g.active && g.nodes.iter().any(|n| n.kind == kind))
}

/// Adresse dans « Nom <adresse> » (ou le texte tel quel).
fn email_of(from: &str) -> String {
    match from.split_once('<') {
        Some((_, rest)) => rest.trim_end_matches('>').trim().to_string(),
        None => from.trim().to_string(),
    }
}

/// Date et heure (« 25/10/2026 14:30 »).
fn when(ms: i64) -> String {
    use chrono::TimeZone;
    Local.timestamp_millis_opt(ms).single().map(|t| t.format("%d/%m/%Y %H:%M").to_string()).unwrap_or_default()
}

/// Catégorie de la boîte de réception d'après les libellés Gmail.
fn category(labels: &[String]) -> String {
    let has = |l: &str| labels.iter().any(|x| x == l);
    if has("CATEGORY_SOCIAL") {
        t!("Réseaux sociaux", "Social")
    } else if has("CATEGORY_PROMOTIONS") {
        t!("Promotions", "Promotions")
    } else if has("CATEGORY_UPDATES") {
        t!("Notifications", "Updates")
    } else if has("CATEGORY_FORUMS") {
        t!("Forums", "Forums")
    } else if has("SPAM") {
        t!("Spam", "Spam")
    } else if has("INBOX") {
        t!("Principale", "Primary")
    } else {
        t!("Tous les messages", "All mail")
    }
    .to_string()
}

/// Mail reçu, avec ses groupes (libellés créés par l'utilisateur, `names`) ; à défaut, la liste
/// de diffusion.
pub fn mail_in(m: &Summary, names: &[Label]) -> MailIn {
    let groups: Vec<&str> = m.labels.iter().filter_map(|l| names.iter().find(|n| n.user && &n.id == l)).map(|n| n.name.as_str()).collect();
    let group = if groups.is_empty() { m.list_id.trim().trim_matches(['<', '>']).to_string() } else { groups.join(", ") };
    MailIn {
        category: category(&m.labels),
        group,
        date: when(m.date),
        id: m.id.clone(),
        from_name: m.sender_name().to_string(),
        from_email: email_of(&m.from),
        via: format!("{} {}", m.recipients, m.list_id),
        subject: m.subject.clone(),
        body: m.snippet.clone(),
        to: m.recipients.split(',').map(|r| email_of(r.trim())).filter(|r| !r.is_empty()).collect(),
    }
}

impl Engine {
    /// Reprend le journal, les attentes et le dernier passage du lancement précédent.
    pub fn load(&mut self) {
        let Some(saved) = std::fs::read_to_string(saved_path()).ok().and_then(|t| serde_json::from_str::<Saved>(&t).ok()) else { return };
        self.last_seen = (saved.last_seen > 0).then_some(saved.last_seen);
        self.runs = saved.runs;
        self.waiting = saved.waiting;
    }

    /// Enregistre l'état (toutes les 30 s, ou tout de suite si `now`).
    pub fn save(&mut self, now: bool) {
        if !now && self.last_save.is_some_and(|t| t.elapsed() < SAVE_EVERY) {
            return;
        }
        self.last_save = Some(Instant::now());
        let saved = Saved { last_seen: Local::now().timestamp_millis(), runs: self.runs.clone(), waiting: self.waiting.clone() };
        crate::paths::save_json(&saved_path(), &saved);
    }

    /// Effets en attente d'être appliqués par l'app.
    pub fn take(&mut self) -> Vec<Applied> {
        std::mem::take(&mut self.pending)
    }

    /// Exécute l'event `node` de chaque graph actif concerné ; garde l'exécution si l'event s'est
    /// déclenché (filtre passé).
    fn fire(&mut self, graphs: &mut [Graph], only: Option<(&str, u64)>, inc: Incoming, origin: Origin, dnd: bool) {
        for g in graphs.iter_mut().filter(|g| g.active) {
            let events: Vec<u64> = g.nodes.iter().filter(|n| n.kind == inc.kind()).map(|n| n.id).collect();
            for node in events {
                if only.is_some_and(|(gid, nid)| gid != g.id || nid != node) {
                    continue;
                }
                let trace = sim::run_real(g, node, inc.clone(), dnd);
                if trace.fired {
                    self.record(g, node, trace, &inc, &origin);
                }
            }
        }
    }

    /// Garde le résultat d'une exécution : effets, variables, attentes, journal.
    fn record(&mut self, g: &mut Graph, event: u64, trace: sim::Trace, inc: &Incoming, origin: &Origin) {
        for effect in trace.effects {
            self.pending.push(Applied { effect, origin: origin.clone() });
        }
        if let Some(mem) = trace.memory {
            g.memory = mem;
            if !self.touched.contains(&g.id) {
                self.touched.push(g.id.clone());
            }
        }
        let now = Local::now().timestamp_millis();
        for (delay, minutes) in trace.delays {
            self.waiting.push(Waiting { graph: g.id.clone(), event, delay, due: now + minutes * 60_000, inc: inc.clone(), origin: origin.clone() });
        }
        self.runs.push_front(RunRecord { at: now, graph: g.name.clone(), lines: trace.lines });
        self.runs.truncate(HISTORY);
    }

    /// Reprend les exécutions dont l'attente (« Attendre ») est finie.
    fn resume_due(&mut self, graphs: &mut [Graph]) {
        let now = Local::now().timestamp_millis();
        let (due, wait): (Vec<Waiting>, Vec<Waiting>) = std::mem::take(&mut self.waiting).into_iter().partition(|w| w.due <= now);
        self.waiting = wait;
        for w in due {
            // Graph supprimé, désactivé ou modifié entre-temps (node absente) : abandon.
            let Some(g) = graphs.iter_mut().find(|g| g.id == w.graph && g.active) else { continue };
            if g.node(w.event).is_none() || g.node(w.delay).is_none() {
                continue;
            }
            let trace = sim::resume(g, w.event, w.delay, w.inc.clone());
            self.record(g, w.event, trace, &w.inc, &w.origin);
        }
    }

    /// Mails reçus (chargement complet : `initial` ; relève : nouveaux mails). Renvoie les
    /// mails à exécuter, dont l'app va chercher le contenu complet (puis `on_full_mail`).
    /// Au chargement complet, seuls les mails non lus arrivés depuis la dernière fermeture de
    /// l'app (moins de 24 h) sont rattrapés.
    pub fn on_mails(&mut self, graphs: &[Graph], list: &[Summary], initial: bool) -> Vec<Summary> {
        let want = wanted(graphs, "ev_mail");
        let now = Local::now().timestamp_millis();
        let mut out = Vec::new();
        for m in list {
            let fresh = self.known_mail.insert(m.id.clone());
            let inbox = m.labels.iter().any(|l| l == "INBOX");
            if !want || !fresh || !inbox || !m.unread() {
                continue;
            }
            let ok = if initial { self.last_seen.is_some_and(|t| m.date > t && m.date > now - CATCH_UP) } else { m.date >= self.started_ms - 120_000 };
            if ok {
                out.push(m.clone());
            }
        }
        out
    }

    /// Mail complet (contenu entier, pas seulement l'extrait) : exécute les graphs.
    pub fn on_full_mail(&mut self, graphs: &mut [Graph], full: &crate::google::gmail::FullMessage, labels: &[Label], dnd: bool) {
        let m = &full.summary;
        let mut inc = mail_in(m, labels);
        if !full.body.trim().is_empty() {
            inc.body = full.body.trim().to_string();
        }
        if m.date < self.started_ms {
            inc.date = tf!("{} (reçu app fermée)", "{} (received while closed)", inc.date);
        }
        self.fire(graphs, None, Incoming::Mail(inc), Origin::Mail(Box::new(m.clone())), dnd);
    }

    /// Liste des conversations mise à jour : celles qui ont bougé sont relues.
    pub fn on_chat_spaces(&mut self, graphs: &[Graph], chat: &ChatState, acc: &Account) {
        let first = self.chat_seen.is_none();
        let want = wanted(graphs, "ev_chat");
        let seen = self.chat_seen.get_or_insert_with(HashMap::new);
        for sp in &chat.spaces {
            let prev = seen.insert(sp.name.clone(), sp.last_active);
            let Some(prev) = prev else { continue };
            if first || !want || sp.last_active <= prev || sp.last_active < self.started_ms {
                continue;
            }
            self.chat_pending.insert(sp.name.clone(), prev);
            let name = sp.name.clone();
            acc.run(move |s| {
                let list = chat::messages(s, &name, Some(prev), None, 10).map(|r| r.0).unwrap_or_default();
                Ok(Event::Chat(ChatEvent::Messages { space: name, list, replace_from: None, older: false, more: false }))
            });
        }
    }

    /// Nouveaux messages d'une conversation attendue : une exécution par message (pas les miens).
    pub fn on_chat_messages(&mut self, graphs: &mut [Graph], space: &str, list: &[ChatMessage], chat: &ChatState, dnd: bool) {
        let Some(since) = self.chat_pending.remove(space) else { return };
        let title = chat.space(space).map(|s| chat.title(s)).unwrap_or_else(|| "Google Chat".into());
        for m in list.iter().filter(|m| m.created > since && !chat.is_me(&m.sender) && !m.text.trim().is_empty()) {
            let inc = Incoming::Chat(ChatIn {
                id: m.name.clone(),
                author: chat.name_of(&m.sender),
                author_email: String::new(),
                space: title.clone(),
                text: m.text.clone(),
                date: when(m.created),
            });
            self.fire(graphs, None, inc, Origin::Chat(Box::new(m.clone())), dnd);
        }
    }

    /// Rappel minuté qui sonne (`late` : l'app était fermée à l'heure prévue).
    pub fn on_timer(&mut self, graphs: &mut [Graph], t: &crate::tasks::Timer, late: bool, dnd: bool) {
        if !wanted(graphs, "ev_reminder") {
            return;
        }
        let inc = Incoming::Reminder(ReminderIn { id: t.id.to_string(), title: t.title.clone(), due: crate::tabs::calendar::hhmm(t.due), late });
        self.fire(graphs, None, inc, Origin::Reminder(Box::new(t.clone())), dnd);
    }

    /// Rendez-vous qui commencent (ou qui commenceront dans les minutes réglées sur l'event).
    pub fn background(&mut self, graphs: &mut [Graph], cal: &CalendarState, prefs: &Prefs) {
        self.resume_due(graphs);
        self.save(false);
        if !wanted(graphs, "ev_event") || self.last_calendar.is_some_and(|t| t.elapsed() < CALENDAR_EVERY) {
            return;
        }
        self.last_calendar = Some(Instant::now());
        let now = Local::now().timestamp_millis();
        let mut todo: Vec<(String, u64, CalEvent)> = Vec::new();
        for g in graphs.iter().filter(|g| g.active) {
            for n in g.nodes.iter().filter(|n| n.kind == "ev_event") {
                let before = if n.text("when") == "before" { n.int("minutes").max(0) * 60_000 } else { 0 };
                // Déclenchement à `début - avance` : on cherche les rendez-vous dont ce moment
                // vient de passer.
                for e in cal.events_between(prefs, now - CALENDAR_GRACE + before, now + before + 1) {
                    let at = e.start - before;
                    if e.all_day || at > now || at <= now - CALENDAR_GRACE || at < self.started_ms - CALENDAR_GRACE {
                        continue;
                    }
                    if self.events_done.insert(format!("{}/{}/{}/{}/{}", g.id, n.id, e.calendar, e.id, e.start)) {
                        todo.push((g.id.clone(), n.id, e.clone()));
                    }
                }
            }
        }
        for (gid, nid, e) in todo {
            let when = crate::tabs::calendar::hhmm(e.start);
            let calendar = cal.calendars.iter().find(|c| c.id == e.calendar).map(|c| c.name.clone()).unwrap_or_default();
            let inc = Incoming::Event(EventIn { id: e.id.clone(), title: e.title.clone(), start: when, location: e.location.clone(), calendar, date: self::when(e.start) });
            self.fire(graphs, Some((&gid, nid)), inc, Origin::Event(Box::new(e)), prefs.do_not_disturb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(id: &str, from: &str, subject: &str) -> Summary {
        Summary {
            id: id.into(),
            from: from.into(),
            subject: subject.into(),
            date: Local::now().timestamp_millis(),
            labels: vec!["INBOX".into(), "UNREAD".into()],
            recipients: "moi@exemple.fr, equipe@exemple.fr".into(),
            ..Default::default()
        }
    }

    /// Relève de mails, puis contenu complet de chaque mail retenu (comme l'app).
    fn mails(e: &mut Engine, graphs: &[Graph], list: &[Summary], initial: bool, dnd: bool) {
        let mut graphs = graphs.to_vec();
        for m in e.on_mails(&graphs, list, initial) {
            let full = crate::google::gmail::FullMessage { body: m.snippet.clone(), summary: m, ..Default::default() };
            e.on_full_mail(&mut graphs, &full, &[], dnd);
        }
    }

    fn graph(active: bool) -> Graph {
        let mut g = Graph::new("Urgents");
        g.active = active;
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let n = g.add_node("act_notify", [300.0, 0.0]);
        let t = g.add_node("act_trash", [600.0, 0.0]);
        g.connect(ev, "then", n, "exec");
        g.connect(ev, "subject", n, "title");
        g.connect(n, "then", t, "exec");
        g.connect(ev, "mail", t, "mail");
        g.node_mut(ev).unwrap().values.insert("keywords".into(), "urgent".into());
        g
    }

    #[test]
    fn new_mail_runs_active_graphs_only() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut e = Engine::default();
        // Chargement initial : rien.
        mails(&mut e, &[graph(true)], &[mail("a", "Camille <c@x.fr>", "Urgent : devis")], true, false);
        assert!(e.take().is_empty());
        // Nouveau mail, graph inactif : rien.
        mails(&mut e, &[graph(false)], &[mail("b", "Camille <c@x.fr>", "Urgent : devis")], false, false);
        assert!(e.take().is_empty());
        // Nouveau mail, graph actif : notification puis corbeille de ce mail.
        mails(&mut e, &[graph(true)], &[mail("c", "Camille <c@x.fr>", "Urgent : devis")], false, false);
        let fx: Vec<Effect> = e.take().into_iter().map(|a| a.effect).collect();
        assert_eq!(fx.len(), 2);
        assert!(matches!(&fx[0], Effect::Notify { title, .. } if title == "Urgent : devis"));
        assert_eq!(fx[1], Effect::Trash("c".into()));
        assert_eq!(e.runs.len(), 1);
        // Le même mail ne déclenche pas deux fois ; un mail sans le mot-clé ne déclenche pas.
        mails(&mut e, &[graph(true)], &[mail("c", "Camille <c@x.fr>", "Urgent : devis"), mail("d", "Léo <l@x.fr>", "Bonjour")], false, false);
        assert!(e.take().is_empty());
    }

    #[test]
    fn mails_received_while_closed_are_caught_up() {
        let mut e = Engine { last_seen: Some(Local::now().timestamp_millis() - 3_600_000), ..Engine::default() };
        let mut old = mail("old", "C <c@x.fr>", "urgent");
        old.date -= 2 * 3_600_000;
        let mut missed = mail("missed", "C <c@x.fr>", "urgent");
        missed.date -= 600_000;
        let got = e.on_mails(&[graph(true)], &[old, missed], true);
        assert_eq!(got.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), vec!["missed"]);
        // Premier lancement : rien à rattraper.
        let mut e = Engine::default();
        assert!(e.on_mails(&[graph(true)], &[mail("x", "C <c@x.fr>", "urgent")], true).is_empty());
    }

    #[test]
    fn do_not_disturb_blocks_unless_overridden() {
        let mut e = Engine::default();
        let mut g = graph(true);
        mails(&mut e, std::slice::from_ref(&g), &[mail("a", "C <c@x.fr>", "urgent")], false, true);
        assert!(e.take().is_empty());
        let ev = g.nodes.iter().find(|n| n.kind == "ev_mail").unwrap().id;
        g.node_mut(ev).unwrap().values.insert("dnd".into(), true.into());
        mails(&mut e, &[g], &[mail("b", "C <c@x.fr>", "urgent")], false, true);
        let fx = e.take();
        assert!(matches!(&fx[0].effect, Effect::Notify { dnd: true, .. }));
    }

    #[test]
    fn reminder_can_be_snoozed() {
        let mut g = Graph::new("Rappels");
        g.active = true;
        let ev = g.add_node("ev_reminder", [0.0, 0.0]);
        let sn = g.add_node("act_snooze", [300.0, 0.0]);
        g.connect(ev, "then", sn, "exec");
        g.connect(ev, "reminder", sn, "reminder");
        let mut e = Engine::default();
        let t = crate::tasks::Timer { id: 7, task: 1, title: "Appeler".into(), due: 0, created: 0, fired: true };
        e.on_timer(&mut [g], &t, false, false);
        let fx: Vec<Effect> = e.take().into_iter().map(|a| a.effect).collect();
        assert_eq!(fx, vec![Effect::Snooze { reminder: 7, minutes: 10 }]);
    }

    #[test]
    fn sender_address_is_extracted() {
        let m = mail_in(&mail("a", "\"Camille Martin\" <camille@x.fr>", "s"), &[]);
        assert_eq!(m.from_name, "Camille Martin");
        assert_eq!(m.from_email, "camille@x.fr");
        assert_eq!(m.to, vec!["moi@exemple.fr", "equipe@exemple.fr"]);
    }
}
