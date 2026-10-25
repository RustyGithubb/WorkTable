//! Exécution d'un graph : chaque event vérifie son filtre sur un mail, un message ou un
//! rendez-vous, puis le flux avance de node en node.
//!
//! - `run` : simulation, à blanc, sur les données d'exemple (`store::Sample`). Les actions ne font
//!   rien : elles décrivent dans le journal ce qu'elles auraient fait, avec les valeurs reçues.
//! - `run_real` : exécution pour de vrai, quand un vrai mail, message ou rendez-vous arrive. Les
//!   actions deviennent des effets (`Effect`), appliqués ensuite par l'app.
//!
//! Le résultat (`Trace`) liste les étapes dans l'ordre (nodes exécutées, fils parcourus) pour
//! l'animation, les lignes du journal, chacune rattachée à l'étape où elle apparaît, et les effets.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use super::nodes::{self, Cat, PinType, Widget};
use super::store::{Graph, Sample};

/// Au-delà, la simulation s'arrête (boucle sans fin).
const MAX_STEPS: usize = 500;

/// Valeur qui circule sur un fil de données.
#[derive(Clone, Debug, PartialEq)]
pub enum Val {
    None,
    Bool(bool),
    Int(i64),
    Text(String),
    List(Vec<String>),
    /// Objet (mail, message, contact, rendez-vous, rappel, note, tâche) : son type, le texte
    /// qui le résume, son identifiant et ses données (pour « Break data »).
    Rec(PinType, String, String, Arc<Obj>),
}

/// Données d'un objet, telles que les sort « Break data ».
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Obj {
    /// Classe : « Mail », « Message », « Note »…
    pub class: String,
    /// Clé du logo (`nodes::ICONS`) : se branche sur le logo d'une notification.
    pub logo: String,
    pub title: String,
    pub text: String,
    pub author: String,
    pub category: String,
    pub group: String,
    pub date: String,
    pub id: String,
}

impl Obj {
    /// Clé de sortie de « Break data » → valeur.
    pub fn field(&self, key: &str) -> &str {
        match key {
            "class" => &self.class,
            "logo" => &self.logo,
            "title" => &self.title,
            "text" => &self.text,
            "author" => &self.author,
            "category" => &self.category,
            "group" => &self.group,
            "date" => &self.date,
            "id" => &self.id,
            _ => "",
        }
    }
}

/// Classe et logo d'un type d'objet.
fn class_of(ty: PinType) -> (&'static str, &'static str) {
    match ty {
        PinType::Mail => (t!("Mail", "Mail"), "mail"),
        PinType::Message => (t!("Message", "Message"), "messages"),
        PinType::Contact => (t!("Contact", "Contact"), "messages"),
        PinType::Event => (t!("Rendez-vous", "Event"), "calendar"),
        PinType::Reminder => (t!("Rappel", "Reminder"), "reminder"),
        _ => (t!("Objet", "Object"), "blueprint"),
    }
}

/// Objet `ty` (classe et logo déduits du type).
fn rec(ty: PinType, summary: &str, id: &str, o: Obj) -> Val {
    let (class, logo) = class_of(ty);
    let o = Obj { class: class.into(), logo: logo.into(), id: id.into(), ..o };
    Val::Rec(ty, summary.into(), id.into(), Arc::new(o))
}

/// Objet créé par une action (note, tâche, rappel, rendez-vous).
fn made(class: &str, logo: &str, o: Obj) -> Val {
    let summary = o.title.clone();
    Val::Rec(PinType::Object, summary, String::new(), Arc::new(Obj { class: class.into(), logo: logo.into(), ..o }))
}

fn now_text() -> String {
    chrono::Local::now().format("%d/%m/%Y %H:%M").to_string()
}

/// Ce qui déclenche un event : un mail, un message ou un rendez-vous (vrai ou d'exemple).
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct MailIn {
    pub id: String,
    pub from_name: String,
    pub from_email: String,
    /// Destinataires et liste de diffusion (pour « Reçu via un groupe »).
    pub via: String,
    pub subject: String,
    pub body: String,
    pub to: Vec<String>,
    /// Catégorie de la boîte (Principale, Promotions…).
    pub category: String,
    /// Groupes (libellés) du mail.
    pub group: String,
    pub date: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ChatIn {
    /// « spaces/X/messages/Y »
    pub id: String,
    pub author: String,
    pub author_email: String,
    pub space: String,
    pub text: String,
    pub date: String,
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct EventIn {
    pub id: String,
    pub title: String,
    pub start: String,
    pub location: String,
    /// Agenda du rendez-vous.
    pub calendar: String,
    pub date: String,
}

/// Rappel minuté qui sonne.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ReminderIn {
    pub id: String,
    pub title: String,
    pub due: String,
    /// L'app était fermée à l'heure prévue.
    pub late: bool,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub enum Incoming {
    Mail(MailIn),
    Chat(ChatIn),
    Event(EventIn),
    Reminder(ReminderIn),
}

impl Incoming {
    /// Type d'event que déclenche cette arrivée.
    pub fn kind(&self) -> &'static str {
        match self {
            Incoming::Mail(_) => "ev_mail",
            Incoming::Chat(_) => "ev_chat",
            Incoming::Event(_) => "ev_event",
            Incoming::Reminder(_) => "ev_reminder",
        }
    }
}

fn sample_incoming(s: &Sample) -> Vec<Incoming> {
    vec![
        Incoming::Mail(MailIn {
            id: String::new(),
            from_name: s.mail_from_name.clone(),
            from_email: s.mail_from_email.clone(),
            via: s.mail_via.clone(),
            subject: s.mail_subject.clone(),
            body: s.mail_body.clone(),
            to: keywords(&s.mail_to),
            category: t!("Principale", "Primary").into(),
            group: s.mail_via.clone(),
            date: now_text(),
        }),
        Incoming::Chat(ChatIn {
            id: String::new(),
            author: s.chat_author.clone(),
            author_email: s.chat_author_email.clone(),
            space: s.chat_space.clone(),
            text: s.chat_text.clone(),
            date: now_text(),
        }),
        Incoming::Event(EventIn {
            id: String::new(),
            title: s.event_title.clone(),
            start: s.event_start.clone(),
            location: s.event_location.clone(),
            calendar: t!("Agenda principal", "Main calendar").into(),
            date: s.event_start.clone(),
        }),
        Incoming::Reminder(ReminderIn { id: String::new(), title: s.reminder_title.clone(), due: s.reminder_due.clone(), late: false }),
    ]
}

/// Mail ou message visé par une action (épingler, mettre en favoris).
#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    Mail(String),
    Message(String),
}

/// Action à appliquer pour de vrai (exécution réelle).
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// `dnd` : passe outre « Ne pas déranger » (réglage de l'event).
    Notify {
        icon: String,
        title: String,
        body: String,
        dnd: bool,
    },
    Task {
        title: String,
        doing: bool,
    },
    Event {
        title: String,
        in_minutes: i64,
        duration: i64,
    },
    Pin(Item),
    Star(Item),
    Trash(String),
    Group {
        mail: String,
        group: String,
    },
    /// Catégorie Gmail (`CATEGORY_…`).
    Category {
        mail: String,
        label: String,
    },
    Note {
        title: String,
        body: String,
    },
    /// Rappel dans `in_minutes`, ou à l'heure `at` (« HH:MM », aujourd'hui ou demain).
    Reminder {
        title: String,
        in_minutes: Option<i64>,
        at: Option<String>,
    },
    Snooze {
        reminder: u64,
        minutes: i64,
    },
    Unremind(u64),
}

impl Val {
    pub fn text(&self) -> String {
        match self {
            Val::None => String::new(),
            Val::Bool(b) => if *b { t!("vrai", "true") } else { t!("faux", "false") }.to_string(),
            Val::Int(i) => i.to_string(),
            Val::Text(s) | Val::Rec(_, s, ..) => s.clone(),
            Val::List(l) => l.join(", "),
        }
    }

    pub fn truthy(&self) -> bool {
        match self {
            Val::None => false,
            Val::Bool(b) => *b,
            Val::Int(i) => *i != 0,
            Val::Text(s) => !s.trim().is_empty(),
            Val::List(l) => !l.is_empty(),
            Val::Rec(..) => true,
        }
    }

    fn list(&self) -> Vec<String> {
        match self {
            Val::List(l) => l.clone(),
            Val::None => Vec::new(),
            Val::Text(s) => s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect(),
            v => vec![v.text()],
        }
    }
}

/// Étape de la simulation : une node exécutée (ou calculée) et le fil qui y mène.
#[derive(Clone, Debug, PartialEq)]
pub struct Step {
    pub node: u64,
    /// Indice du fil dans `Graph::links`.
    pub link: Option<usize>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Line {
    /// Étape à partir de laquelle la ligne apparaît.
    pub step: usize,
    pub warn: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default)]
pub struct Trace {
    pub steps: Vec<Step>,
    pub lines: Vec<Line>,
    /// Actions demandées (appliquées seulement pour une exécution réelle).
    pub effects: Vec<Effect>,
    /// L'event a-t-il passé son filtre ?
    pub fired: bool,
    /// Variables du graph après l'exécution, si elles ont changé (à enregistrer).
    pub memory: Option<BTreeMap<String, String>>,
    /// « Attendre » atteints (exécution réelle) : node et minutes ; la suite est reprise plus tard.
    pub delays: Vec<(u64, i64)>,
}

struct Loop {
    item: String,
    index: i64,
    broke: bool,
}

struct Run<'a> {
    g: &'a Graph,
    /// Arrivées disponibles (une par type d'event).
    incoming: Vec<Incoming>,
    /// Exécution réelle (formulation du journal).
    real: bool,
    /// « Ne pas déranger » actif : seuls les events qui passent outre se déclenchent.
    dnd: bool,
    /// L'event en cours passe outre « Ne pas déranger ».
    dnd_override: bool,
    /// Event en cours et ses sorties.
    event: Option<(u64, HashMap<&'static str, Val>)>,
    loops: HashMap<u64, Loop>,
    /// Variables du graph (copie de travail) et si elles ont changé.
    mem: BTreeMap<String, String>,
    mem_changed: bool,
    /// Reprise après un « Attendre » : la node d'où repartir.
    resume: Option<u64>,
    /// Objet créé par chaque action exécutée (note, tâche, rappel, rendez-vous).
    made: HashMap<u64, Val>,
    trace: Trace,
    stopped: bool,
    executed: usize,
}

/// Exécute `f` sur un fil à grande pile : le flux est suivi récursivement (jusqu'à
/// `MAX_STEPS` étapes), trop profond pour la pile du fil principal sous Windows (1 Mo).
fn big_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    // Tests : la langue est propre à chaque fil.
    #[cfg(test)]
    let lang = if crate::i18n::en() { crate::i18n::Language::English } else { crate::i18n::Language::French };
    let body = move || {
        #[cfg(test)]
        crate::i18n::set(lang);
        f()
    };
    std::thread::scope(|s| std::thread::Builder::new().stack_size(64 << 20).spawn_scoped(s, body).expect("fil d'exécution du graph").join().expect("exécution du graph"))
}

/// Simule tout le graph : chaque event, l'un après l'autre.
pub fn run(g: &Graph) -> Trace {
    big_stack(|| run_inner(g))
}

fn run_inner(g: &Graph) -> Trace {
    let mut r = Run::new(g, sample_incoming(&g.sample), false, false);
    let events: Vec<u64> = g.nodes.iter().filter(|n| n.def().is_some_and(|d| d.cat == Cat::Event)).map(|n| n.id).collect();
    if events.is_empty() {
        r.warn(t!("Aucun event : ajoute un event (clic droit › Événements) pour démarrer un flux.", "No event: add one (right-click › Events) to start a flow."));
    }
    for id in events {
        r.trigger(id);
    }
    let n = r.executed;
    r.say(tf!("Fin de la simulation — {n} node(s) exécutée(s).", "Simulation finished — {n} node(s) executed."));
    r.trace
}

/// Exécute pour de vrai l'event `node` du graph, déclenché par `inc`. `dnd` : « Ne pas
/// déranger » est actif.
pub fn run_real(g: &Graph, node: u64, inc: Incoming, dnd: bool) -> Trace {
    big_stack(|| {
        let mut r = Run::new(g, vec![inc], true, dnd);
        r.trigger(node);
        r.finish()
    })
}

/// Reprend une exécution réelle après le « Attendre » `delay` (même event, même arrivée : le
/// filtre et « Ne pas déranger » ne sont pas revérifiés).
pub fn resume(g: &Graph, node: u64, delay: u64, inc: Incoming) -> Trace {
    big_stack(|| {
        let mut r = Run::new(g, vec![inc], true, false);
        r.resume = Some(delay);
        r.trigger(node);
        r.finish()
    })
}

/// Mots d'un texte, en minuscules et sans accents (« Réunion » → « reunion »).
fn words(s: &str) -> Vec<String> {
    let fold = |c: char| match c {
        'à' | 'â' | 'ä' | 'á' | 'ã' => 'a',
        'é' | 'è' | 'ê' | 'ë' => 'e',
        'î' | 'ï' | 'í' => 'i',
        'ô' | 'ö' | 'ó' | 'õ' => 'o',
        'ù' | 'û' | 'ü' | 'ú' => 'u',
        'ç' => 'c',
        'ÿ' => 'y',
        c => c,
    };
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(|w| w.chars().map(fold).collect()).collect()
}

/// Le mot-clé figure-t-il en entier dans le texte (« urgent » trouve « URGENT : devis » mais pas
/// « urgentiste » ; un mot-clé de plusieurs mots doit apparaître tel quel) ?
pub fn has_word(text: &str, keyword: &str) -> bool {
    let (t, k) = (words(text), words(keyword));
    !k.is_empty() && t.windows(k.len()).any(|w| w == k.as_slice())
}

fn contains_ci(hay: &str, needle: &str) -> bool {
    hay.to_lowercase().contains(&needle.to_lowercase())
}

fn keywords(s: &str) -> Vec<String> {
    s.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect()
}

impl<'a> Run<'a> {
    fn finish(mut self) -> Trace {
        if self.mem_changed {
            self.trace.memory = Some(self.mem);
        }
        self.trace
    }

    fn new(g: &'a Graph, incoming: Vec<Incoming>, real: bool, dnd: bool) -> Self {
        Run {
            g,
            incoming,
            real,
            dnd,
            dnd_override: false,
            event: None,
            loops: HashMap::new(),
            mem: g.memory.clone(),
            mem_changed: false,
            resume: None,
            made: HashMap::new(),
            trace: Trace::default(),
            stopped: false,
            executed: 0,
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        let step = self.trace.steps.len();
        self.trace.lines.push(Line { step, warn: false, text: text.into() });
    }

    fn warn(&mut self, text: impl Into<String>) {
        let step = self.trace.steps.len();
        self.trace.lines.push(Line { step, warn: true, text: text.into() });
    }

    fn step(&mut self, node: u64, link: Option<usize>) -> bool {
        if self.trace.steps.len() >= MAX_STEPS {
            if !self.stopped {
                self.stopped = true;
                self.warn(tf!("Simulation arrêtée : plus de {MAX_STEPS} étapes (boucle sans fin ?).", "Simulation stopped: more than {MAX_STEPS} steps (endless loop?)."));
            }
            return false;
        }
        self.trace.steps.push(Step { node, link });
        true
    }

    fn title(&self, id: u64) -> String {
        self.g.node(id).and_then(|n| n.def()).map(|d| d.title().to_string()).unwrap_or_default()
    }

    /// Déclenche un event avec l'arrivée de son type (mail, message, rendez-vous), si elle
    /// passe le filtre.
    fn trigger(&mut self, id: u64) {
        let Some(n) = self.g.node(id) else { return };
        let Some(inc) = self.incoming.iter().find(|i| i.kind() == n.kind).cloned() else { return };
        let title = self.title(id);
        let real = self.real;
        let (outputs, filter_ok, what): (HashMap<&'static str, Val>, Result<(), String>, String) = match &inc {
            Incoming::Mail(m) => {
                let from = if m.from_email.is_empty() { m.from_name.clone() } else { format!("{} <{}>", m.from_name, m.from_email) };
                let mut o = HashMap::new();
                let mail = Obj {
                    title: m.subject.clone(),
                    text: m.body.clone(),
                    author: from.clone(),
                    category: m.category.clone(),
                    group: m.group.clone(),
                    date: m.date.clone(),
                    ..Obj::default()
                };
                o.insert("mail", rec(PinType::Mail, &m.subject, &m.id, mail));
                let who = Obj { title: m.from_name.clone(), text: m.from_email.clone(), author: from.clone(), group: m.group.clone(), date: m.date.clone(), ..Obj::default() };
                o.insert("sender", rec(PinType::Contact, &from, &m.from_email, who));
                o.insert("address", Val::Text(m.from_email.clone()));
                o.insert("subject", Val::Text(m.subject.clone()));
                o.insert("body", Val::Text(m.body.clone()));
                o.insert("recipients", Val::List(m.to.clone()));
                let v = n.text("value");
                let source = match n.text("source").as_str() {
                    "person" => Some((contains_ci(&m.from_name, &v) || contains_ci(&m.from_email, &v), t!("la personne", "the person"))),
                    "group" => Some((!m.via.is_empty() && contains_ci(&m.via, v.trim()), t!("le groupe", "the group"))),
                    "address" => Some((m.from_email.eq_ignore_ascii_case(v.trim()), t!("l'adresse", "the address"))),
                    _ => None,
                };
                let ok = self.filter(source, &v, &n.text("keywords"), &[&m.subject, &m.body]);
                let what = if real {
                    tf!("le mail de {from} « {} »", "the mail from {from} “{}”", m.subject)
                } else {
                    tf!("le mail d'exemple de {from} « {} »", "the sample mail from {from} “{}”", m.subject)
                };
                (o, ok, what)
            }
            Incoming::Chat(c) => {
                let author = if c.author_email.is_empty() { c.author.clone() } else { format!("{} <{}>", c.author, c.author_email) };
                let mut o = HashMap::new();
                let msg = Obj {
                    title: c.space.clone(),
                    text: c.text.clone(),
                    author: author.clone(),
                    category: t!("Google Chat", "Google Chat").into(),
                    group: c.space.clone(),
                    date: c.date.clone(),
                    ..Obj::default()
                };
                o.insert("message", rec(PinType::Message, &c.text, &c.id, msg));
                let who = Obj { title: c.author.clone(), text: c.author_email.clone(), author: author.clone(), group: c.space.clone(), date: c.date.clone(), ..Obj::default() };
                o.insert("author", rec(PinType::Contact, &author, &c.author_email, who));
                o.insert("text", Val::Text(c.text.clone()));
                o.insert("space", Val::Text(c.space.clone()));
                let v = n.text("value");
                let source = match n.text("source").as_str() {
                    "person" => Some((contains_ci(&c.author, &v) || contains_ci(&c.author_email, &v), t!("la personne", "the person"))),
                    "space" => Some((contains_ci(&c.space, &v), t!("la conversation", "the conversation"))),
                    _ => None,
                };
                let ok = self.filter(source, &v, &n.text("keywords"), &[&c.text]);
                let what = if real {
                    tf!("le message de {} dans « {} »", "the message from {} in “{}”", c.author, c.space)
                } else {
                    tf!("le message d'exemple de {} dans « {} »", "the sample message from {} in “{}”", c.author, c.space)
                };
                (o, ok, what)
            }
            Incoming::Reminder(r) => {
                let mut o = HashMap::new();
                let rem = Obj { title: r.title.clone(), category: t!("Rappels", "Reminders").into(), date: r.due.clone(), ..Obj::default() };
                o.insert("reminder", rec(PinType::Reminder, &r.title, &r.id, rem));
                o.insert("title", Val::Text(r.title.clone()));
                o.insert("due", Val::Text(r.due.clone()));
                o.insert("late", Val::Bool(r.late));
                let ok = self.filter(None, "", &n.text("keywords"), &[&r.title]);
                let late = if r.late { t!(", en retard", ", late") } else { "" };
                let what = if real {
                    tf!("le rappel « {} » ({}{late})", "the reminder “{}” ({}{late})", r.title, r.due)
                } else {
                    tf!("le rappel d'exemple « {} » ({}{late})", "the sample reminder “{}” ({}{late})", r.title, r.due)
                };
                (o, ok, what)
            }
            Incoming::Event(e) => {
                let mut o = HashMap::new();
                let ev = Obj { title: e.title.clone(), text: e.location.clone(), category: e.calendar.clone(), group: e.calendar.clone(), date: e.date.clone(), ..Obj::default() };
                o.insert("event", rec(PinType::Event, &e.title, &e.id, ev));
                o.insert("title", Val::Text(e.title.clone()));
                o.insert("start", Val::Text(e.start.clone()));
                o.insert("location", Val::Text(e.location.clone()));
                let ok = self.filter(None, "", &n.text("keywords"), &[&e.title]);
                let when = if n.text("when") == "before" {
                    tf!("{} min avant le début", "{} min before the start", n.int("minutes"))
                } else {
                    t!("au début", "at the start").to_string()
                };
                let what = if real {
                    tf!("le rendez-vous « {} » ({when})", "the event “{}” ({when})", e.title)
                } else {
                    tf!("le rendez-vous d'exemple « {} » ({when})", "the sample event “{}” ({when})", e.title)
                };
                (o, ok, what)
            }
        };
        let dnd = n.flag("dnd");
        if !self.step(id, None) {
            return;
        }
        if let Some(delay) = self.resume {
            self.trace.fired = true;
            let minutes = self.g.node(delay).map(|n| n.int("minutes")).unwrap_or(0);
            self.say(tf!("Reprise après l'attente de {minutes} min, pour {what}", "Resumed after waiting {minutes} min, for {what}"));
            self.event = Some((id, outputs));
            self.follow(delay, "then");
            self.event = None;
            return;
        }
        if let Err(why) = filter_ok {
            self.warn(tf!("« {title} » ne se déclenche pas pour {what} : {why}", "“{title}” does not fire for {what}: {why}"));
            return;
        }
        if self.dnd && !dnd {
            self.warn(tf!(
                "« {title} » ne se déclenche pas : « Ne pas déranger » est actif (coche « Outrepasser Ne pas déranger » pour qu'il passe quand même).",
                "“{title}” does not fire: “Do not disturb” is on (tick “Override Do not disturb” to let it through)."
            ));
            return;
        }
        self.trace.fired = true;
        self.executed += 1;
        self.dnd_override = dnd;
        let extra = if dnd { t!(" — passe outre Ne pas déranger", " — overrides Do not disturb") } else { "" };
        self.say(tf!("Event « {title} » déclenché par {what}{extra}", "Event “{title}” fired by {what}{extra}"));
        self.event = Some((id, outputs));
        if self.g.links_from(id, "then").next().is_none() {
            self.warn(tf!("Rien n'est branché après « {title} ».", "Nothing is connected after “{title}”."));
        }
        self.follow(id, "then");
        self.event = None;
    }

    /// Provenance (correspond ?, nom du critère) et mots-clés (au moins un, dans un des textes).
    fn filter(&self, source: Option<(bool, &str)>, value: &str, kw: &str, texts: &[&str]) -> Result<(), String> {
        if let Some((ok, what)) = source {
            if value.trim().is_empty() {
                return Err(tf!("aucune valeur indiquée pour {what}", "no value given for {what}"));
            }
            if !ok {
                return Err(tf!("{what} ne correspond pas à « {} »", "{what} does not match “{}”", value.trim()));
            }
        }
        let kws = keywords(kw);
        if !kws.is_empty() && !kws.iter().any(|k| texts.iter().any(|t| has_word(t, k))) {
            return Err(tf!("aucun des mots-clés ({}) n'y figure", "none of the keywords ({}) appear", kws.join(", ")));
        }
        Ok(())
    }

    /// Suit les fils d'exécution qui partent de (`from`, `out`).
    fn follow(&mut self, from: u64, out: &str) {
        let next: Vec<usize> = self.g.links.iter().enumerate().filter(|(_, l)| l.from == from && l.out == out).map(|(i, _)| i).collect();
        for i in next {
            if self.stopped {
                return;
            }
            let l = &self.g.links[i];
            let (to, input) = (l.to, l.input.clone());
            self.exec(to, &input, i);
        }
    }

    fn exec(&mut self, id: u64, entry: &str, via: usize) {
        let Some(n) = self.g.node(id) else { return };
        let kind = n.kind.clone();
        if !self.step(id, Some(via)) {
            return;
        }
        let title = self.title(id);
        match kind.as_str() {
            "fl_branch" => {
                self.executed += 1;
                let c = self.input(id, "cond").truthy();
                let out = if c { "true" } else { "false" };
                self.say(tf!(
                    "Branch : condition {}, suite par « {} »",
                    "Branch: condition {}, continues through “{}”",
                    Val::Bool(c).text(),
                    if c { t!("Vrai", "True") } else { t!("Faux", "False") }
                ));
                self.follow(id, out);
            }
            "fl_if" => {
                self.executed += 1;
                let (a, b) = (self.input(id, "a").text(), self.input(id, "b").text());
                let op = self.g.node(id).map(|n| n.text("op")).unwrap_or_default();
                let (al, bl) = (a.to_lowercase(), b.to_lowercase());
                let res = match op.as_str() {
                    "equals" => al.trim() == bl.trim(),
                    "starts" => al.starts_with(&bl),
                    "ends" => al.ends_with(&bl),
                    "differs" => al.trim() != bl.trim(),
                    _ => al.contains(&bl),
                };
                self.say(tf!(
                    "Si : « {} » {} « {} », suite par « {} »",
                    "If: “{}” {} “{}”, continues through “{}”",
                    a,
                    op_label(&op),
                    b,
                    if res { t!("Vrai", "True") } else { t!("Faux", "False") }
                ));
                self.follow(id, if res { "true" } else { "false" });
            }
            // Corps à part : `exec` est récursive, son cadre de pile doit rester petit.
            "fl_once" => self.once(id),
            "fl_delay" => self.delay(id),
            "fl_sequence" => {
                self.executed += 1;
                self.follow(id, "then0");
                self.follow(id, "then1");
            }
            "fl_foreach" if entry == "break" => {
                if let Some(l) = self.loops.get_mut(&id) {
                    l.broke = true;
                }
                self.say(t!("Boucle arrêtée (Arrêter).", "Loop stopped (Break)."));
            }
            "fl_foreach" => {
                self.executed += 1;
                let list = self.input(id, "list").list();
                self.say(tf!("Pour chaque : {} élément(s)", "For each: {} item(s)", list.len()));
                for (i, item) in list.into_iter().enumerate() {
                    self.loops.insert(id, Loop { item, index: i as i64, broke: false });
                    self.follow(id, "body");
                    if self.stopped || self.loops.get(&id).is_some_and(|l| l.broke) {
                        break;
                    }
                }
                self.loops.remove(&id);
                self.follow(id, "done");
            }
            k if k.starts_with("act_") => {
                self.executed += 1;
                let line = self.action(id, k);
                self.say(tf!("Action « {title} » : {line}", "Action “{title}”: {line}"));
                self.follow(id, "then");
            }
            _ => {}
        }
    }

    #[inline(never)]
    fn once(&mut self, id: u64) {
        self.executed += 1;
        let period = self.g.node(id).map(|n| n.text("period")).unwrap_or_default();
        let now = chrono::Local::now();
        let stamp = match period.as_str() {
            "hour" => now.format("%Y-%m-%d %H").to_string(),
            "week" => now.format("%G-W%V").to_string(),
            "ever" => "1".to_string(),
            _ => now.format("%Y-%m-%d").to_string(),
        };
        let key = format!("__once/{id}");
        if self.mem.get(&key) == Some(&stamp) {
            self.say(t!("Une fois par période : déjà fait, suite par « Déjà fait »", "Do once: already done, continues through “Already done”"));
            self.follow(id, "done");
        } else {
            self.mem.insert(key, stamp);
            self.mem_changed = true;
            self.say(t!("Une fois par période : première fois, suite par « Première fois »", "Do once: first time, continues through “First time”"));
            self.follow(id, "then");
        }
    }

    #[inline(never)]
    fn delay(&mut self, id: u64) {
        self.executed += 1;
        let m = self.int_input(id, "minutes").max(1);
        if self.real {
            self.say(tf!("Attendre : la suite reprendra dans {m} min", "Delay: the rest resumes in {m} min"));
            self.trace.delays.push((id, m));
        } else {
            self.say(tf!("Attendre : {m} min (simulation : la suite est jouée tout de suite)", "Delay: {m} min (simulation: the rest plays right away)"));
            self.follow(id, "then");
        }
    }

    /// Ce que fait l'action : la description pour le journal, et l'effet à appliquer (une
    /// exécution réelle l'applique ; une simulation s'en tient à la description).
    fn action(&mut self, id: u64, kind: &str) -> String {
        let txt = |r: &mut Self, k: &str| r.input(id, k).text();
        let (line, effect) = match kind {
            "act_notify" => {
                let key = txt(self, "icon");
                let icon = nodes::ICONS.iter().find(|i| i.0 == key).map(|i| t!(i.1, i.2)).unwrap_or("Mail");
                let (title, body) = (txt(self, "title"), txt(self, "body"));
                let line = tf!("logo {icon}, titre « {title} », contenu « {body} »", "logo {icon}, title “{title}”, content “{body}”");
                (line, Some(Effect::Notify { icon: key, title, body, dnd: self.dnd_override }))
            }
            "act_task" => {
                let doing = txt(self, "column") == "doing";
                let col = if doing { t!("En cours", "In progress") } else { t!("À faire", "To do") };
                let title = txt(self, "title");
                self.made.insert(id, made(t!("Tâche", "Task"), "dashboard", Obj { title: title.clone(), category: col.into(), date: now_text(), ..Obj::default() }));
                (tf!("« {title} » dans {col}", "“{title}” in {col}"), (!title.trim().is_empty()).then_some(Effect::Task { title, doing }))
            }
            "act_event" => {
                let title = txt(self, "title");
                let (i, d) = (self.int_input(id, "in_minutes"), self.int_input(id, "duration"));
                let at = (chrono::Local::now() + chrono::Duration::minutes(i)).format("%d/%m/%Y %H:%M").to_string();
                self.made.insert(
                    id,
                    made(
                        t!("Rendez-vous", "Event"),
                        "calendar",
                        Obj { title: title.clone(), text: tf!("{d} min", "{d} min"), category: t!("Agenda principal", "Main calendar").into(), date: at, ..Obj::default() },
                    ),
                );
                (tf!("« {title} » dans {i} min, pendant {d} min", "“{title}” in {i} min, for {d} min"), Some(Effect::Event { title, in_minutes: i, duration: d.max(5) }))
            }
            "act_pin" | "act_star" => {
                let (line, item) = self.item(id, "item");
                let fx = item.map(|i| if kind == "act_pin" { Effect::Pin(i) } else { Effect::Star(i) });
                (line, fx)
            }
            "act_trash" => {
                let (line, item) = self.item(id, "mail");
                (line, item.and_then(|i| if let Item::Mail(m) = i { Some(Effect::Trash(m)) } else { None }))
            }
            "act_group" => {
                let (m, item) = self.item(id, "mail");
                let group = txt(self, "group");
                let fx = match item {
                    Some(Item::Mail(mail)) if !group.trim().is_empty() => Some(Effect::Group { mail, group: group.trim().to_string() }),
                    _ => None,
                };
                (tf!("{m} → groupe « {group} »", "{m} → group “{group}”"), fx)
            }
            "act_category" => {
                let (m, item) = self.item(id, "mail");
                let raw = txt(self, "category");
                match nodes::category(&raw) {
                    Some(c) => {
                        let name = t!(c.1, c.2);
                        let fx = match item {
                            Some(Item::Mail(mail)) => Some(Effect::Category { mail, label: c.0.to_string() }),
                            _ => None,
                        };
                        (tf!("{m} → catégorie « {name} »", "{m} → category “{name}”"), fx)
                    }
                    None => {
                        self.warn(tf!(
                            "  Catégorie inconnue « {raw} » : Principale, Promotions, Réseaux sociaux, Notifications ou Forums.",
                            "  Unknown category “{raw}”: Primary, Promotions, Social, Updates or Forums."
                        ));
                        (tf!("{m} → catégorie inconnue", "{m} → unknown category"), None)
                    }
                }
            }
            "act_reminder" => {
                let title = txt(self, "title");
                let at_time = self.g.node(id).is_some_and(|n| n.text("when") == "at");
                let due = if at_time {
                    txt(self, "at")
                } else {
                    let m = self.int_input(id, "in_minutes").max(1);
                    (chrono::Local::now() + chrono::Duration::minutes(m)).format("%H:%M").to_string()
                };
                self.made
                    .insert(id, made(t!("Rappel", "Reminder"), "reminder", Obj { title: title.clone(), category: t!("Rappels", "Reminders").into(), date: due, ..Obj::default() }));
                if at_time {
                    let at = txt(self, "at");
                    (tf!("« {title} » à {at}", "“{title}” at {at}"), (!title.trim().is_empty()).then_some(Effect::Reminder { title, in_minutes: None, at: Some(at) }))
                } else {
                    let m = self.int_input(id, "in_minutes").max(1);
                    (tf!("« {title} » dans {m} min", "“{title}” in {m} min"), (!title.trim().is_empty()).then_some(Effect::Reminder { title, in_minutes: Some(m), at: None }))
                }
            }
            "act_snooze" | "act_unremind" => {
                let (line, rid) = self.reminder(id);
                if kind == "act_snooze" {
                    let m = self.int_input(id, "minutes").max(1);
                    (tf!("{line}, de nouveau dans {m} min", "{line}, again in {m} min"), rid.map(|r| Effect::Snooze { reminder: r, minutes: m }))
                } else {
                    (line, rid.map(Effect::Unremind))
                }
            }
            "act_setvar" | "act_addvar" => {
                let name = self.g.node(id).map(|n| n.text("name").trim().to_string()).unwrap_or_default();
                if name.is_empty() {
                    self.warn(t!("  Aucun nom de variable : rien n'a été gardé.", "  No variable name: nothing was kept."));
                    return t!("(sans nom)", "(no name)").to_string();
                }
                let value = if kind == "act_setvar" {
                    txt(self, "value")
                } else {
                    let total = self.mem.get(&name).and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0) + self.int_input(id, "amount");
                    self.made.insert(id, Val::Int(total));
                    total.to_string()
                };
                self.mem.insert(name.clone(), value.clone());
                self.mem_changed = true;
                return tf!("« {name} » = {value}", "“{name}” = {value}");
            }
            "act_note" => {
                let (title, body) = (txt(self, "title"), txt(self, "body"));
                self.made.insert(
                    id,
                    made(t!("Note", "Note"), "notes", Obj { title: title.clone(), text: body.clone(), category: t!("Notes", "Notes").into(), date: now_text(), ..Obj::default() }),
                );
                (tf!("« {title} » — {body}", "“{title}” — {body}"), Some(Effect::Note { title, body }))
            }
            _ => (String::new(), None),
        };
        if let Some(fx) = effect {
            self.trace.effects.push(fx);
        }
        line
    }

    fn int_input(&mut self, id: u64, key: &str) -> i64 {
        match self.input(id, key) {
            Val::Int(i) => i,
            v => v.text().trim().parse().unwrap_or(0),
        }
    }

    /// Rappel reçu par l'entrée « Rappel » d'une action (sinon un avertissement).
    fn reminder(&mut self, id: u64) -> (String, Option<u64>) {
        match self.input(id, "reminder") {
            Val::Rec(PinType::Reminder, s, rid, _) => (tf!("rappel « {s} »", "reminder “{s}”"), rid.parse().ok()),
            Val::None => {
                self.warn(t!("  Aucun rappel branché : l'action n'aurait rien fait.", "  No reminder connected: the action would have done nothing."));
                (t!("(aucun rappel)", "(no reminder)").to_string(), None)
            }
            v => (v.text(), None),
        }
    }

    /// Mail ou message reçu par une entrée d'action (sinon un avertissement).
    fn item(&mut self, id: u64, key: &str) -> (String, Option<Item>) {
        match self.input(id, key) {
            Val::Rec(ty, s, item_id, _) => {
                let item = match ty {
                    PinType::Mail if !item_id.is_empty() => Some(Item::Mail(item_id)),
                    PinType::Message if !item_id.is_empty() => Some(Item::Message(item_id)),
                    // Données d'exemple : pas d'identifiant, rien à appliquer.
                    _ => None,
                };
                (format!("{} « {s} »", ty.label()), item)
            }
            Val::None => {
                self.warn(t!("  Aucun élément branché : l'action n'aurait rien fait.", "  No item connected: the action would have done nothing."));
                (t!("(rien)", "(nothing)").to_string(), None)
            }
            v => (v.text(), None),
        }
    }

    /// Valeur d'une entrée : le fil branché, sinon le champ de la node.
    fn input(&mut self, id: u64, key: &str) -> Val {
        let link = self.g.links.iter().position(|l| l.to == id && l.input == key);
        if let Some(i) = link {
            let l = &self.g.links[i];
            let (from, out) = (l.from, l.out.clone());
            let is_pure = self.g.node(from).and_then(|n| n.def()).is_some_and(|d| d.cat == Cat::Pure);
            // Le fil de données s'allume ; une node pure s'allume aussi, quand elle est calculée.
            if is_pure || self.trace.steps.last().is_some_and(|s| s.node != from) {
                self.step(from, Some(i));
            }
            return self.output(from, &out);
        }
        let Some(n) = self.g.node(id) else { return Val::None };
        let Some(p) = n.def().and_then(|d| d.input(key)) else { return Val::None };
        match p.widget {
            Widget::Bool(_) => Val::Bool(n.flag(key)),
            Widget::Int(..) => Val::Int(n.int(key)),
            Widget::Text(..) | Widget::Choice(_) | Widget::Icon | Widget::Keywords(..) => Val::Text(n.text(key)),
            Widget::None => Val::None,
        }
    }

    /// Valeur d'une sortie : celle de l'event en cours, de la boucle, ou le calcul d'une node pure.
    fn output(&mut self, id: u64, key: &str) -> Val {
        if let Some((ev, outs)) = &self.event
            && *ev == id
        {
            return outs.get(key).cloned().unwrap_or(Val::None);
        }
        let Some(n) = self.g.node(id) else { return Val::None };
        let kind = n.kind.clone();
        let ignore_case = n.flag("case");
        match kind.as_str() {
            "fl_foreach" => match self.loops.get(&id) {
                Some(l) if key == "item" => Val::Text(l.item.clone()),
                Some(l) if key == "index" => Val::Int(l.index),
                _ => Val::None,
            },
            "pu_contains" => {
                let (t, q) = (self.input(id, "text").text(), self.input(id, "search").text());
                Val::Bool(if ignore_case { contains_ci(&t, &q) } else { t.contains(&q) })
            }
            "pu_not" => Val::Bool(!self.input(id, "a").truthy()),
            "pu_and" => {
                let a = self.input(id, "a").truthy();
                Val::Bool(a && self.input(id, "b").truthy())
            }
            "pu_or" => {
                let a = self.input(id, "a").truthy();
                Val::Bool(a || self.input(id, "b").truthy())
            }
            "pu_join" => {
                let (a, sep, b) = (self.input(id, "a").text(), self.input(id, "sep").text(), self.input(id, "b").text());
                Val::Text(format!("{a}{sep}{b}"))
            }
            "pu_getvar" => {
                let name = self.g.node(id).map(|n| n.text("name").trim().to_string()).unwrap_or_default();
                Val::Text(self.mem.get(&name).cloned().unwrap_or_default())
            }
            "pu_math" => {
                let (a, b) = (self.int_input(id, "a"), self.int_input(id, "b"));
                let op = self.g.node(id).map(|n| n.text("op")).unwrap_or_default();
                Val::Int(match op.as_str() {
                    "sub" => a.saturating_sub(b),
                    "mul" => a.saturating_mul(b),
                    "div" => a.checked_div(b).unwrap_or(0),
                    "mod" => a.checked_rem(b).unwrap_or(0),
                    _ => a.saturating_add(b),
                })
            }
            "pu_compare" => {
                let (a, b) = (self.int_input(id, "a"), self.int_input(id, "b"));
                let op = self.g.node(id).map(|n| n.text("op")).unwrap_or_default();
                Val::Bool(match op.as_str() {
                    "le" => a <= b,
                    "eq" => a == b,
                    "ge" => a >= b,
                    "gt" => a > b,
                    "ne" => a != b,
                    _ => a < b,
                })
            }
            "pu_now" => {
                use chrono::{Datelike, Timelike};
                let now = chrono::Local::now();
                let wd = now.weekday().num_days_from_monday() as usize;
                const DAYS: [(&str, &str); 7] = [
                    ("lundi", "Monday"),
                    ("mardi", "Tuesday"),
                    ("mercredi", "Wednesday"),
                    ("jeudi", "Thursday"),
                    ("vendredi", "Friday"),
                    ("samedi", "Saturday"),
                    ("dimanche", "Sunday"),
                ];
                match key {
                    "time" => Val::Text(now.format("%H:%M").to_string()),
                    "date" => Val::Text(now.format("%d/%m/%Y").to_string()),
                    "weekday" => Val::Text(t!(DAYS[wd].0, DAYS[wd].1).to_string()),
                    "hour" => Val::Int(now.hour() as i64),
                    "minute" => Val::Int(now.minute() as i64),
                    "weekend" => Val::Bool(wd >= 5),
                    _ => Val::None,
                }
            }
            "pu_break" => match self.input(id, "obj") {
                Val::Rec(_, _, _, o) => Val::Text(o.field(key).to_string()),
                Val::None => Val::None,
                // Simple texte : c'est son titre.
                v if key == "title" || key == "text" => Val::Text(v.text()),
                _ => Val::None,
            },
            k if k.starts_with("act_") => match self.made.get(&id) {
                Some(v) => v.clone(),
                None => {
                    let title = self.title(id);
                    self.warn(tf!("  « {title} » n'a pas encore été exécutée : son objet est vide ici.", "  “{title}” has not run yet: its object is empty here."));
                    Val::None
                }
            },
            k if k.starts_with("ev_") => {
                let title = self.title(id);
                self.warn(tf!("  « {title} » ne s'est pas déclenché : sa valeur est vide ici.", "  “{title}” did not fire: its value is empty here."));
                Val::None
            }
            _ => Val::None,
        }
    }
}

fn op_label(op: &str) -> &'static str {
    match op {
        "equals" => t!("est égal à", "equals"),
        "starts" => t!("commence par", "starts with"),
        "ends" => t!("finit par", "ends with"),
        "differs" => t!("est différent de", "differs from"),
        _ => t!("contient", "contains"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(t: &Trace) -> Vec<String> {
        t.lines.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn mail_flow_with_branch() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let c = g.add_node("pu_contains", [0.0, 300.0]);
        let br = g.add_node("fl_branch", [300.0, 0.0]);
        let notif = g.add_node("act_notify", [600.0, 0.0]);
        let task = g.add_node("act_task", [600.0, 200.0]);
        g.connect(ev, "then", br, "exec");
        g.connect(ev, "subject", c, "text");
        g.connect(c, "result", br, "cond");
        g.connect(br, "true", notif, "exec");
        g.connect(br, "false", task, "exec");
        g.connect(ev, "subject", notif, "title");
        let t = run(&g);
        let all = texts(&t).join("\n");
        assert!(all.contains("condition vrai, suite par « Vrai »"), "{all}");
        assert!(all.contains("Urgent : réunion de projet demain"), "{all}");
        assert!(!all.contains("Créer une tâche"), "{all}");
        assert!(t.steps.iter().any(|s| s.node == notif));
        assert!(!t.steps.iter().any(|s| s.node == task));
    }

    #[test]
    fn break_data_of_mail_and_created_note() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let br = g.add_node("pu_break", [0.0, 300.0]);
        let notif = g.add_node("act_notify", [300.0, 0.0]);
        let note = g.add_node("act_note", [600.0, 0.0]);
        let br2 = g.add_node("pu_break", [600.0, 300.0]);
        let notif2 = g.add_node("act_notify", [900.0, 0.0]);
        g.connect(ev, "then", notif, "exec");
        g.connect(ev, "mail", br, "obj");
        g.connect(br, "logo", notif, "icon");
        g.connect(br, "class", notif, "title");
        g.connect(br, "category", notif, "body");
        g.connect(notif, "then", note, "exec");
        g.node_mut(note).unwrap().values.insert("title".into(), "Compte rendu".into());
        g.connect(note, "then", notif2, "exec");
        g.connect(note, "obj", br2, "obj");
        g.connect(br2, "logo", notif2, "icon");
        g.connect(br2, "class", notif2, "title");
        g.connect(br2, "title", notif2, "body");
        let t = run(&g);
        let all = texts(&t).join("\n");
        assert!(all.contains("logo Mail, titre « Mail », contenu « Principale »"), "{all}");
        assert!(all.contains("logo Note, titre « Note », contenu « Compte rendu »"), "{all}");
    }

    #[test]
    fn change_category() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let cat = g.add_node("act_category", [300.0, 0.0]);
        g.connect(ev, "then", cat, "exec");
        g.connect(ev, "mail", cat, "mail");
        g.node_mut(cat).unwrap().values.insert("category".into(), "CATEGORY_PROMOTIONS".into());
        let inc = Incoming::Mail(MailIn { id: "m1".into(), subject: "Soldes".into(), ..MailIn::default() });
        let t = run_real(&g, ev, inc, false);
        assert_eq!(t.effects, vec![Effect::Category { mail: "m1".into(), label: "CATEGORY_PROMOTIONS".into() }]);
        assert!(texts(&t).join("\n").contains("catégorie « Promotions »"));
    }

    #[test]
    fn keywords_match_whole_words() {
        assert!(has_word("URGENT : devis", "urgent"));
        assert!(!has_word("Un urgentiste a appelé", "urgent"));
        assert!(has_word("Réunion demain", "reunion"));
        assert!(has_word("Le rendez-vous est confirmé", "rendez-vous"));
        assert!(!has_word("Le rendez-vous est confirmé", "rendez-vous annulé"));
        assert!(!has_word("abc", "a"));
        assert!(!has_word("texte", ""));
    }

    #[test]
    fn variables_once_and_delay() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let add = g.add_node("act_addvar", [300.0, 0.0]);
        let once = g.add_node("fl_once", [600.0, 0.0]);
        let delay = g.add_node("fl_delay", [900.0, 0.0]);
        let note = g.add_node("act_note", [1200.0, 0.0]);
        let get = g.add_node("pu_getvar", [900.0, 300.0]);
        g.connect(ev, "then", add, "exec");
        g.connect(add, "then", once, "exec");
        g.connect(once, "then", delay, "exec");
        g.connect(delay, "then", note, "exec");
        g.connect(get, "value", note, "body");
        g.node_mut(add).unwrap().values.insert("name".into(), "mails".into());
        g.node_mut(get).unwrap().values.insert("name".into(), "mails".into());
        let inc = || Incoming::Mail(MailIn { id: "m".into(), subject: "s".into(), ..MailIn::default() });
        // 1re exécution : compteur à 1, première fois, attente (la note n'est pas encore créée).
        let t = run_real(&g, ev, inc(), false);
        let mem = t.memory.clone().expect("variables changées");
        assert_eq!(mem.get("mails").map(String::as_str), Some("1"));
        assert_eq!(t.delays, vec![(delay, 10)]);
        assert!(t.effects.is_empty());
        g.memory = mem;
        // Reprise après l'attente : la note reçoit la valeur de la variable.
        let t = resume(&g, ev, delay, inc());
        assert_eq!(t.effects, vec![Effect::Note { title: String::new(), body: "1".into() }]);
        // 2e exécution le même jour : compteur à 2, « Déjà fait ».
        let t = run_real(&g, ev, inc(), false);
        assert_eq!(t.memory.as_ref().and_then(|m| m.get("mails")).map(String::as_str), Some("2"));
        assert!(t.delays.is_empty());
        assert!(texts(&t).join("\n").contains("déjà fait"));
    }

    #[test]
    fn math_compare_now() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let math = g.add_node("pu_math", [0.0, 300.0]);
        let cmp = g.add_node("pu_compare", [300.0, 300.0]);
        let br = g.add_node("fl_branch", [300.0, 0.0]);
        let notif = g.add_node("act_notify", [600.0, 0.0]);
        let now = g.add_node("pu_now", [600.0, 300.0]);
        let set = |g: &mut Graph, n: u64, k: &str, v: serde_json::Value| {
            g.node_mut(n).unwrap().values.insert(k.into(), v);
        };
        set(&mut g, math, "op", "mul".into());
        set(&mut g, math, "a", 6.into());
        set(&mut g, math, "b", 7.into());
        set(&mut g, cmp, "op", "eq".into());
        set(&mut g, cmp, "b", 42.into());
        g.connect(math, "result", cmp, "a");
        g.connect(cmp, "result", br, "cond");
        g.connect(ev, "then", br, "exec");
        g.connect(br, "true", notif, "exec");
        g.connect(now, "time", notif, "title");
        let t = run(&g);
        let all = texts(&t).join("\n");
        assert!(all.contains("condition vrai"), "{all}");
        assert!(t.steps.iter().any(|s| s.node == notif));
    }

    #[test]
    fn filter_stops_the_flow() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let n = g.add_node("act_notify", [300.0, 0.0]);
        g.connect(ev, "then", n, "exec");
        let node = g.node_mut(ev).unwrap();
        node.values.insert("source".into(), "person".into());
        node.values.insert("value".into(), "quelqu'un d'autre".into());
        let t = run(&g);
        assert!(texts(&t).iter().any(|l| l.contains("ne se déclenche pas")));
        assert!(!t.steps.iter().any(|s| s.node == n));
    }

    #[test]
    fn foreach_with_break() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let fe = g.add_node("fl_foreach", [300.0, 0.0]);
        let notif = g.add_node("act_notify", [600.0, 0.0]);
        let done = g.add_node("act_task", [600.0, 300.0]);
        g.connect(ev, "then", fe, "exec");
        g.connect(ev, "recipients", fe, "list");
        g.connect(fe, "body", notif, "exec");
        g.connect(fe, "item", notif, "title");
        // Arrêt après le premier élément.
        g.connect(notif, "then", fe, "break");
        g.connect(fe, "done", done, "exec");
        let t = run(&g);
        let notifs = t.steps.iter().filter(|s| s.node == notif).count();
        assert_eq!(notifs, 1);
        assert!(t.steps.iter().any(|s| s.node == done));
        assert!(texts(&t).iter().any(|l| l.contains("moi@exemple.fr")));
    }

    #[test]
    fn reminder_flow_creates_a_reminder() {
        crate::i18n::set(crate::i18n::Language::French);
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_mail", [0.0, 0.0]);
        let r = g.add_node("act_reminder", [300.0, 0.0]);
        g.connect(ev, "then", r, "exec");
        g.connect(ev, "subject", r, "title");
        let t = run(&g);
        assert!(t.effects.contains(&Effect::Reminder { title: "Urgent : réunion de projet demain".into(), in_minutes: Some(30), at: None }));
        // Rappel d'exemple qui sonne : l'event se déclenche aussi en simulation.
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_reminder", [0.0, 0.0]);
        let n = g.add_node("act_notify", [300.0, 0.0]);
        g.connect(ev, "then", n, "exec");
        g.connect(ev, "title", n, "title");
        let t = run(&g);
        assert!(t.lines.iter().any(|l| l.text.contains("Appeler le fournisseur")));
    }

    #[test]
    fn endless_loop_is_stopped() {
        let mut g = Graph::new("t");
        let ev = g.add_node("ev_chat", [0.0, 0.0]);
        let a = g.add_node("fl_sequence", [300.0, 0.0]);
        let b = g.add_node("fl_sequence", [600.0, 0.0]);
        g.connect(ev, "then", a, "exec");
        g.connect(a, "then0", b, "exec");
        g.connect(b, "then0", a, "exec");
        let t = run(&g);
        assert!(t.steps.len() <= MAX_STEPS);
    }
}
