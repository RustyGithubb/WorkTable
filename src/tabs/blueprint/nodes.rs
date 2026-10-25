//! Catalogue des nodes du Blueprint : events, actions, flux d'exécution et nodes pures.
//!
//! Chaque node a des entrées et des sorties (« pins »), typées comme dans Unreal :
//! - les pins d'exécution (flèche blanche) font avancer le flux ;
//! - les pins de données (rond coloré) transportent une valeur (texte, booléen, mail…).
//!
//! Une entrée peut avoir un champ (texte, case, liste) : il sert de valeur quand aucun fil n'y
//! est branché, et disparaît quand on en branche un. Les réglages (`wire: false`) n'ont pas de
//! pin : ils s'affichent en haut de la node, sur toute sa largeur.

use eframe::egui::Color32;

/// Type d'une pin (couleur et compatibilité des fils).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinType {
    Exec,
    Bool,
    Int,
    Text,
    List,
    Mail,
    Message,
    Contact,
    Event,
    /// Mail ou message (épingler, mettre en favoris).
    Item,
    /// Rappel minuté (tableau de bord › Rappels).
    Reminder,
    /// N'importe quel élément (mail, message, contact, rendez-vous, rappel, note, tâche) :
    /// sa classe et ses informations se lisent avec « Break data ».
    Object,
}

impl PinType {
    /// Couleur du fil et de la pin (palette d'Unreal : booléen rouge, entier vert d'eau, texte
    /// rose, structures bleues…).
    pub fn color(self) -> Color32 {
        match self {
            PinType::Exec => Color32::from_rgb(0xF0, 0xF0, 0xF0),
            PinType::Bool => Color32::from_rgb(0xC0, 0x24, 0x24),
            PinType::Int => Color32::from_rgb(0x2E, 0xD6, 0xA6),
            PinType::Text => Color32::from_rgb(0xE8, 0x4C, 0xB8),
            PinType::List => Color32::from_rgb(0xD4, 0xA0, 0x3A),
            PinType::Mail => Color32::from_rgb(0x3A, 0x8E, 0xE8),
            PinType::Message => Color32::from_rgb(0x8A, 0x62, 0xE0),
            PinType::Contact => Color32::from_rgb(0x2B, 0xB5, 0xE8),
            PinType::Event => Color32::from_rgb(0xE8, 0x7A, 0x30),
            PinType::Item => Color32::from_rgb(0x5A, 0x78, 0xE0),
            PinType::Reminder => Color32::from_rgb(0xE6, 0xC2, 0x3A),
            PinType::Object => Color32::from_rgb(0x9C, 0xD8, 0xF4),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PinType::Exec => t!("Exécution", "Execution"),
            PinType::Bool => t!("Booléen", "Boolean"),
            PinType::Int => t!("Nombre entier", "Integer"),
            PinType::Text => t!("Texte", "Text"),
            PinType::List => t!("Liste de textes", "List of texts"),
            PinType::Mail => "Mail",
            PinType::Message => "Message",
            PinType::Contact => t!("Fiche contact", "Contact card"),
            PinType::Event => t!("Rendez-vous", "Event"),
            PinType::Item => t!("Mail ou message", "Mail or message"),
            PinType::Reminder => t!("Rappel", "Reminder"),
            PinType::Object => t!("Objet (mail, message, note…)", "Object (mail, message, note…)"),
        }
    }

    /// Une sortie de type `from` peut-elle être branchée sur une entrée de type `self` ?
    /// Tout se convertit en texte ; un élément accepte un mail ou un message.
    pub fn accepts(self, from: PinType) -> bool {
        match (self, from) {
            (a, b) if a == b => true,
            (PinType::Exec, _) | (_, PinType::Exec) => false,
            (PinType::Text, _) => true,
            // Un texte qui contient un nombre (variable, champ) se branche sur un nombre.
            (PinType::Int, PinType::Text) => true,
            (PinType::Item, PinType::Mail | PinType::Message) => true,
            // Tout élément est un objet.
            (PinType::Object, PinType::Mail | PinType::Message | PinType::Contact | PinType::Event | PinType::Item | PinType::Reminder) => true,
            _ => false,
        }
    }
}

/// Champ d'une entrée, utilisé quand aucun fil n'y est branché.
#[derive(Clone, Copy, Debug)]
pub enum Widget {
    None,
    /// Texte, avec une indication grisée.
    Text(&'static str, &'static str),
    Bool(bool),
    Int(i64, i64, i64),
    /// Liste de choix : (clé enregistrée, français, anglais) ; le premier est la valeur par défaut.
    Choice(&'static [(&'static str, &'static str, &'static str)]),
    /// Logo d'une notification : icônes des onglets.
    Icon,
    /// Mots-clés, affichés en pilules (enregistrés séparés par des virgules).
    Keywords(&'static str, &'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct PinDef {
    pub key: &'static str,
    pub fr: &'static str,
    pub en: &'static str,
    pub ty: PinType,
    pub widget: Widget,
    /// `false` : réglage sans pin (en haut de la node).
    pub wire: bool,
    /// Affiché seulement si le champ `.0` de la node vaut l'une des valeurs `.1`.
    pub show: Option<(&'static str, &'static [&'static str])>,
}

impl PinDef {
    /// N'affiche ce champ que si le champ `key` vaut l'une des `values`.
    const fn when(self, key: &'static str, values: &'static [&'static str]) -> Self {
        PinDef { show: Some((key, values)), ..self }
    }

    pub fn label(&self) -> &'static str {
        t!(self.fr, self.en)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Event,
    Action,
    Flow,
    Pure,
}

impl Cat {
    pub const ALL: [Cat; 4] = [Cat::Event, Cat::Action, Cat::Flow, Cat::Pure];

    /// Couleur de l'en-tête des nodes (comme dans Unreal : events rouges, fonctions bleues,
    /// flux gris, nodes pures vertes).
    pub fn color(self) -> Color32 {
        match self {
            Cat::Event => Color32::from_rgb(0x9C, 0x22, 0x22),
            Cat::Action => Color32::from_rgb(0x2A, 0x5C, 0xA8),
            Cat::Flow => Color32::from_rgb(0x5A, 0x5A, 0x5A),
            Cat::Pure => Color32::from_rgb(0x3E, 0x7A, 0x3E),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Cat::Event => t!("Événements", "Events"),
            Cat::Action => "Actions",
            Cat::Flow => t!("Flux d'exécution", "Flow control"),
            Cat::Pure => t!("Logique et texte", "Logic and text"),
        }
    }
}

pub struct NodeDef {
    pub key: &'static str,
    pub cat: Cat,
    pub fr: &'static str,
    pub en: &'static str,
    pub desc_fr: &'static str,
    pub desc_en: &'static str,
    pub inputs: &'static [PinDef],
    pub outputs: &'static [PinDef],
    pub width: f32,
}

impl NodeDef {
    pub fn title(&self) -> &'static str {
        t!(self.fr, self.en)
    }

    pub fn desc(&self) -> &'static str {
        t!(self.desc_fr, self.desc_en)
    }

    pub fn input(&self, key: &str) -> Option<&'static PinDef> {
        self.inputs.iter().find(|p| p.key == key)
    }

    pub fn output(&self, key: &str) -> Option<&'static PinDef> {
        self.outputs.iter().find(|p| p.key == key)
    }

    /// Réglages (en haut) et pins d'entrée (dans les lignes).
    pub fn params(&self) -> impl Iterator<Item = &'static PinDef> {
        self.inputs.iter().filter(|p| !p.wire)
    }

    pub fn wired_inputs(&self) -> impl Iterator<Item = &'static PinDef> {
        self.inputs.iter().filter(|p| p.wire)
    }
}

// --- Définitions -------------------------------------------------------------------------

const fn pin(key: &'static str, fr: &'static str, en: &'static str, ty: PinType, widget: Widget) -> PinDef {
    PinDef { key, fr, en, ty, widget, wire: true, show: None }
}

const fn param(key: &'static str, fr: &'static str, en: &'static str, ty: PinType, widget: Widget) -> PinDef {
    PinDef { key, fr, en, ty, widget, wire: false, show: None }
}

/// Largeur des nodes, par famille.
const W_EVENT: f32 = 300.0;
const W_ACTION: f32 = 270.0;
const W_FLOW: f32 = 240.0;
const W_PURE: f32 = 240.0;
const W_GATE: f32 = 150.0;

const EXEC_IN: PinDef = pin("exec", "", "", PinType::Exec, Widget::None);
const THEN: PinDef = pin("then", "", "", PinType::Exec, Widget::None);
const VAR_NAME: PinDef = param("name", "Variable", "Variable", PinType::Text, Widget::Text("compteur", "counter"));
const DND: PinDef = param("dnd", "Outrepasser Ne pas déranger", "Override Do not disturb", PinType::Bool, Widget::Bool(false));
const KEYWORDS: PinDef = param("keywords", "Mots-clés", "Keywords", PinType::Text, Widget::Keywords("Ajouter un mot…", "Add a word…"));

pub const ICONS: [(&str, &str, &str); 8] = [
    ("mail", "Mail", "Mail"),
    ("messages", "Message", "Message"),
    ("calendar", "Calendrier", "Calendar"),
    ("notes", "Note", "Note"),
    ("dashboard", "Tâche", "Task"),
    ("reminder", "Rappel", "Reminder"),
    ("files", "Fichier", "File"),
    ("blueprint", "Blueprint", "Blueprint"),
];

/// Catégories de la boîte de réception Gmail : (libellé Gmail, français, anglais).
pub const CATEGORIES: [(&str, &str, &str); 5] = [
    ("CATEGORY_PERSONAL", "Principale", "Primary"),
    ("CATEGORY_PROMOTIONS", "Promotions", "Promotions"),
    ("CATEGORY_SOCIAL", "Réseaux sociaux", "Social"),
    ("CATEGORY_UPDATES", "Notifications", "Updates"),
    ("CATEGORY_FORUMS", "Forums", "Forums"),
];

/// Catégorie Gmail d'après sa clé ou son nom (« Promotions », « promotions »…).
pub fn category(s: &str) -> Option<&'static (&'static str, &'static str, &'static str)> {
    let s = s.trim();
    CATEGORIES.iter().find(|c| c.0.eq_ignore_ascii_case(s) || c.1.eq_ignore_ascii_case(s) || c.2.eq_ignore_ascii_case(s))
}

pub static ALL: &[NodeDef] = &[
    // Events
    NodeDef {
        key: "ev_mail",
        cat: Cat::Event,
        fr: "Mail reçu",
        en: "Mail received",
        desc_fr: "Se déclenche à l'arrivée d'un mail, de tout le monde ou d'une personne, d'un groupe ou d'une adresse, et éventuellement s'il contient des mots-clés.",
        desc_en: "Fires when a mail arrives, from anyone or from a person, a group or an address, optionally if it contains keywords.",
        inputs: &[
            param(
                "source",
                "Provenance",
                "From",
                PinType::Text,
                Widget::Choice(&[
                    ("all", "Tous les mails", "All mails"),
                    ("person", "D'une personne", "From a person"),
                    ("group", "Reçu via un groupe", "Through a group"),
                    ("address", "D'une adresse", "From an address"),
                ]),
            ),
            param("value", "Valeur", "Value", PinType::Text, Widget::Text("nom, groupe ou adresse", "name, group or address")).when("source", &["person", "group", "address"]),
            KEYWORDS,
            DND,
        ],
        outputs: &[
            THEN,
            pin("mail", "Mail", "Mail", PinType::Mail, Widget::None),
            pin("sender", "Expéditeur", "Sender", PinType::Contact, Widget::None),
            pin("address", "Adresse", "Address", PinType::Text, Widget::None),
            pin("subject", "Sujet", "Subject", PinType::Text, Widget::None),
            pin("body", "Contenu", "Content", PinType::Text, Widget::None),
            pin("recipients", "Destinataires", "Recipients", PinType::List, Widget::None),
        ],
        width: W_EVENT,
    },
    NodeDef {
        key: "ev_chat",
        cat: Cat::Event,
        fr: "Message reçu",
        en: "Message received",
        desc_fr: "Se déclenche à l'arrivée d'un message Google Chat, de tout le monde, d'une personne ou d'une conversation.",
        desc_en: "Fires when a Google Chat message arrives, from anyone, a person or a conversation.",
        inputs: &[
            param(
                "source",
                "Provenance",
                "From",
                PinType::Text,
                Widget::Choice(&[
                    ("all", "Tous les messages", "All messages"),
                    ("person", "D'une personne", "From a person"),
                    ("space", "D'une conversation", "From a conversation"),
                ]),
            ),
            param("value", "Valeur", "Value", PinType::Text, Widget::Text("nom ou conversation", "name or conversation")).when("source", &["person", "space"]),
            KEYWORDS,
            DND,
        ],
        outputs: &[
            THEN,
            pin("message", "Message", "Message", PinType::Message, Widget::None),
            pin("author", "Auteur", "Author", PinType::Contact, Widget::None),
            pin("text", "Contenu", "Content", PinType::Text, Widget::None),
            pin("space", "Conversation", "Conversation", PinType::Text, Widget::None),
        ],
        width: W_EVENT,
    },
    NodeDef {
        key: "ev_event",
        cat: Cat::Event,
        fr: "Rendez-vous qui commence",
        en: "Event starting",
        desc_fr: "Se déclenche au début d'un rendez-vous de l'agenda, ou quelques minutes avant.",
        desc_en: "Fires when a calendar event starts, or a few minutes before.",
        inputs: &[
            param("when", "Moment", "When", PinType::Text, Widget::Choice(&[("start", "Au début", "At the start"), ("before", "Avant le début", "Before the start")])),
            param("minutes", "Minutes avant", "Minutes before", PinType::Int, Widget::Int(10, 1, 1440)).when("when", &["before"]),
            KEYWORDS,
            DND,
        ],
        outputs: &[
            THEN,
            pin("event", "Rendez-vous", "Event", PinType::Event, Widget::None),
            pin("title", "Titre", "Title", PinType::Text, Widget::None),
            pin("start", "Début", "Start", PinType::Text, Widget::None),
            pin("location", "Lieu", "Location", PinType::Text, Widget::None),
        ],
        width: W_EVENT,
    },
    NodeDef {
        key: "ev_reminder",
        cat: Cat::Event,
        fr: "Rappel qui sonne",
        en: "Reminder going off",
        desc_fr: "Se déclenche quand un rappel minuté sonne (tableau de bord › Rappels), éventuellement s'il contient des mots-clés.",
        desc_en: "Fires when a timed reminder goes off (dashboard › Reminders), optionally if it contains keywords.",
        inputs: &[KEYWORDS, DND],
        outputs: &[
            THEN,
            pin("reminder", "Rappel", "Reminder", PinType::Reminder, Widget::None),
            pin("title", "Titre", "Title", PinType::Text, Widget::None),
            pin("due", "Heure", "Time", PinType::Text, Widget::None),
            pin("late", "En retard", "Late", PinType::Bool, Widget::None),
        ],
        width: W_EVENT,
    },
    // Actions
    NodeDef {
        key: "act_notify",
        cat: Cat::Action,
        fr: "Envoyer une notification",
        en: "Send a notification",
        desc_fr: "Affiche le bandeau de notification de WorkTable, avec le logo, le titre et le contenu choisis.",
        desc_en: "Shows the WorkTable notification banner, with the chosen logo, title and content.",
        inputs: &[
            EXEC_IN,
            pin("icon", "Logo", "Logo", PinType::Text, Widget::Icon),
            pin("title", "Titre", "Title", PinType::Text, Widget::Text("Titre de la notification", "Notification title")),
            pin("body", "Contenu", "Content", PinType::Text, Widget::Text("Texte affiché", "Displayed text")),
        ],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_task",
        cat: Cat::Action,
        fr: "Créer une tâche",
        en: "Create a task",
        desc_fr: "Ajoute une tâche au tableau de bord.",
        desc_en: "Adds a task to the dashboard.",
        inputs: &[
            EXEC_IN,
            pin("title", "Titre", "Title", PinType::Text, Widget::Text("Nouvelle tâche", "New task")),
            pin("column", "Colonne", "Column", PinType::Text, Widget::Choice(&[("todo", "À faire", "To do"), ("doing", "En cours", "In progress")])),
        ],
        outputs: &[THEN, pin("obj", "Tâche", "Task", PinType::Object, Widget::None)],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_reminder",
        cat: Cat::Action,
        fr: "Créer un rappel",
        en: "Create a reminder",
        desc_fr: "Ajoute une tâche « À faire » avec un rappel minuté, dans quelques minutes ou à une heure précise (comme le panneau Rappels du tableau de bord).",
        desc_en: "Adds a “To do” task with a timed reminder, in a few minutes or at a given time (like the dashboard's Reminders panel).",
        inputs: &[
            EXEC_IN,
            param("when", "Quand", "When", PinType::Text, Widget::Choice(&[("in", "Dans X minutes", "In X minutes"), ("at", "À une heure", "At a time")])),
            pin("title", "Titre", "Title", PinType::Text, Widget::Text("Rappel", "Reminder")),
            pin("in_minutes", "Dans (min)", "In (min)", PinType::Int, Widget::Int(30, 1, 100_000)).when("when", &["in"]),
            pin("at", "Heure", "Time", PinType::Text, Widget::Text("09:00", "09:00")).when("when", &["at"]),
        ],
        outputs: &[THEN, pin("obj", "Rappel", "Reminder", PinType::Object, Widget::None)],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_snooze",
        cat: Cat::Action,
        fr: "Reporter un rappel",
        en: "Snooze a reminder",
        desc_fr: "Fait sonner le rappel de nouveau dans quelques minutes.",
        desc_en: "Makes the reminder go off again in a few minutes.",
        inputs: &[
            EXEC_IN,
            pin("reminder", "Rappel", "Reminder", PinType::Reminder, Widget::None),
            pin("minutes", "Dans (min)", "In (min)", PinType::Int, Widget::Int(10, 1, 100_000)),
        ],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_unremind",
        cat: Cat::Action,
        fr: "Supprimer un rappel",
        en: "Delete a reminder",
        desc_fr: "Retire le rappel (la tâche reste).",
        desc_en: "Removes the reminder (the task stays).",
        inputs: &[EXEC_IN, pin("reminder", "Rappel", "Reminder", PinType::Reminder, Widget::None)],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_event",
        cat: Cat::Action,
        fr: "Créer un rendez-vous",
        en: "Create an event",
        desc_fr: "Ajoute un rendez-vous à l'agenda Google.",
        desc_en: "Adds an event to the Google calendar.",
        inputs: &[
            EXEC_IN,
            pin("title", "Titre", "Title", PinType::Text, Widget::Text("Nouveau rendez-vous", "New event")),
            pin("in_minutes", "Dans (min)", "In (min)", PinType::Int, Widget::Int(60, 0, 100_000)),
            pin("duration", "Durée (min)", "Duration (min)", PinType::Int, Widget::Int(30, 5, 1440)),
        ],
        outputs: &[THEN, pin("obj", "Rendez-vous", "Event", PinType::Object, Widget::None)],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_pin",
        cat: Cat::Action,
        fr: "Épingler un élément",
        en: "Pin an item",
        desc_fr: "Épingle le mail ou le message au tableau de bord.",
        desc_en: "Pins the mail or message to the dashboard.",
        inputs: &[EXEC_IN, pin("item", "Élément", "Item", PinType::Item, Widget::None)],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_star",
        cat: Cat::Action,
        fr: "Mettre un élément en favoris",
        en: "Add an item to favourites",
        desc_fr: "Met le mail ou le message en favoris.",
        desc_en: "Adds the mail or message to favourites.",
        inputs: &[EXEC_IN, pin("item", "Élément", "Item", PinType::Item, Widget::None)],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_trash",
        cat: Cat::Action,
        fr: "Mettre un mail à la corbeille",
        en: "Move a mail to trash",
        desc_fr: "Place le mail dans la corbeille de Gmail.",
        desc_en: "Moves the mail to the Gmail trash.",
        inputs: &[EXEC_IN, pin("mail", "Mail", "Mail", PinType::Mail, Widget::None)],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_group",
        cat: Cat::Action,
        fr: "Ranger un mail dans un groupe",
        en: "File a mail in a group",
        desc_fr: "Range le mail dans un groupe (libellé Gmail).",
        desc_en: "Files the mail in a group (Gmail label).",
        inputs: &[EXEC_IN, pin("mail", "Mail", "Mail", PinType::Mail, Widget::None), pin("group", "Groupe", "Group", PinType::Text, Widget::Text("Factures", "Invoices"))],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_category",
        cat: Cat::Action,
        fr: "Changer la catégorie d'un mail",
        en: "Change a mail's category",
        desc_fr: "Déplace le mail dans une autre catégorie de la boîte de réception Gmail (Principale, Promotions…).",
        desc_en: "Moves the mail to another Gmail inbox category (Primary, Promotions…).",
        inputs: &[EXEC_IN, pin("mail", "Mail", "Mail", PinType::Mail, Widget::None), pin("category", "Catégorie", "Category", PinType::Text, Widget::Choice(&CATEGORIES))],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_note",
        cat: Cat::Action,
        fr: "Créer une note",
        en: "Create a note",
        desc_fr: "Crée une note dans l'onglet Prise de notes.",
        desc_en: "Creates a note in the Notes tab.",
        inputs: &[
            EXEC_IN,
            pin("title", "Titre", "Title", PinType::Text, Widget::Text("Nouvelle note", "New note")),
            pin("body", "Contenu", "Content", PinType::Text, Widget::Text("Texte de la note", "Note text")),
        ],
        outputs: &[THEN, pin("obj", "Note", "Note", PinType::Object, Widget::None)],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_setvar",
        cat: Cat::Action,
        fr: "Définir une variable",
        en: "Set a variable",
        desc_fr: "Garde une valeur sous un nom, d'une exécution à l'autre (même après avoir fermé l'app). Se relit avec « Lire une variable ».",
        desc_en: "Keeps a value under a name, from one run to the next (even after closing the app). Read it back with “Get a variable”.",
        inputs: &[EXEC_IN, VAR_NAME, pin("value", "Valeur", "Value", PinType::Text, Widget::Text("", ""))],
        outputs: &[THEN],
        width: W_ACTION,
    },
    NodeDef {
        key: "act_addvar",
        cat: Cat::Action,
        fr: "Ajouter à une variable",
        en: "Add to a variable",
        desc_fr: "Compteur : ajoute un nombre à la variable (0 si elle n'existe pas encore) et donne le nouveau total.",
        desc_en: "Counter: adds a number to the variable (0 if it doesn't exist yet) and gives the new total.",
        inputs: &[EXEC_IN, VAR_NAME, pin("amount", "Ajouter", "Add", PinType::Int, Widget::Int(1, -100_000, 100_000))],
        outputs: &[THEN, pin("total", "Total", "Total", PinType::Int, Widget::None)],
        width: W_ACTION,
    },
    // Flux d'exécution
    NodeDef {
        key: "fl_branch",
        cat: Cat::Flow,
        fr: "Condition (Branch)",
        en: "Branch",
        desc_fr: "Continue par « Vrai » ou par « Faux » selon la condition.",
        desc_en: "Continues through “True” or “False” depending on the condition.",
        inputs: &[EXEC_IN, pin("cond", "Condition", "Condition", PinType::Bool, Widget::Bool(false))],
        outputs: &[pin("true", "Vrai", "True", PinType::Exec, Widget::None), pin("false", "Faux", "False", PinType::Exec, Widget::None)],
        width: W_FLOW,
    },
    NodeDef {
        key: "fl_if",
        cat: Cat::Flow,
        fr: "Si (comparer)",
        en: "If (compare)",
        desc_fr: "Compare deux textes (contient, est égal à, commence par…) et continue par « Vrai » ou « Faux ».",
        desc_en: "Compares two texts (contains, equals, starts with…) and continues through “True” or “False”.",
        inputs: &[
            EXEC_IN,
            param(
                "op",
                "Comparaison",
                "Comparison",
                PinType::Text,
                Widget::Choice(&[
                    ("contains", "A contient B", "A contains B"),
                    ("equals", "A est égal à B", "A equals B"),
                    ("starts", "A commence par B", "A starts with B"),
                    ("ends", "A finit par B", "A ends with B"),
                    ("differs", "A est différent de B", "A differs from B"),
                ]),
            ),
            pin("a", "A", "A", PinType::Text, Widget::Text("", "")),
            pin("b", "B", "B", PinType::Text, Widget::Text("", "")),
        ],
        outputs: &[pin("true", "Vrai", "True", PinType::Exec, Widget::None), pin("false", "Faux", "False", PinType::Exec, Widget::None)],
        width: W_FLOW,
    },
    NodeDef {
        key: "fl_foreach",
        cat: Cat::Flow,
        fr: "Pour chaque (For Each)",
        en: "For Each with Break",
        desc_fr: "Exécute « Corps de boucle » pour chaque élément de la liste ; « Arrêter » interrompt la boucle, puis « Terminé » continue.",
        desc_en: "Runs “Loop body” for each item of the list; “Break” stops the loop, then “Completed” continues.",
        inputs: &[EXEC_IN, pin("break", "Arrêter", "Break", PinType::Exec, Widget::None), pin("list", "Liste", "List", PinType::List, Widget::None)],
        outputs: &[
            pin("body", "Corps de boucle", "Loop body", PinType::Exec, Widget::None),
            pin("item", "Élément", "Item", PinType::Text, Widget::None),
            pin("index", "Index", "Index", PinType::Int, Widget::None),
            pin("done", "Terminé", "Completed", PinType::Exec, Widget::None),
        ],
        width: W_FLOW,
    },
    NodeDef {
        key: "fl_sequence",
        cat: Cat::Flow,
        fr: "Séquence (Sequence)",
        en: "Sequence",
        desc_fr: "Exécute « Ensuite 0 », puis « Ensuite 1 ».",
        desc_en: "Runs “Then 0”, then “Then 1”.",
        inputs: &[EXEC_IN],
        outputs: &[pin("then0", "Ensuite 0", "Then 0", PinType::Exec, Widget::None), pin("then1", "Ensuite 1", "Then 1", PinType::Exec, Widget::None)],
        width: W_FLOW,
    },
    NodeDef {
        key: "fl_once",
        cat: Cat::Flow,
        fr: "Une fois par période (Do Once)",
        en: "Do Once per period",
        desc_fr: "Laisse passer le flux la première fois de la période (jour, heure, semaine, ou une seule fois) ; ensuite, il continue par « Déjà fait ».",
        desc_en: "Lets the flow through the first time in the period (day, hour, week, or only once); after that it continues through “Already done”.",
        inputs: &[
            EXEC_IN,
            param(
                "period",
                "Période",
                "Period",
                PinType::Text,
                Widget::Choice(&[("day", "Par jour", "Per day"), ("hour", "Par heure", "Per hour"), ("week", "Par semaine", "Per week"), ("ever", "Une seule fois", "Only once")]),
            ),
        ],
        outputs: &[pin("then", "Première fois", "First time", PinType::Exec, Widget::None), pin("done", "Déjà fait", "Already done", PinType::Exec, Widget::None)],
        width: W_FLOW,
    },
    NodeDef {
        key: "fl_delay",
        cat: Cat::Flow,
        fr: "Attendre (Delay)",
        en: "Delay",
        desc_fr: "Attend quelques minutes, puis continue. L'attente se poursuit même si l'app est fermée et rouverte entre-temps.",
        desc_en: "Waits a few minutes, then continues. The wait carries on even if the app is closed and reopened meanwhile.",
        inputs: &[EXEC_IN, pin("minutes", "Minutes", "Minutes", PinType::Int, Widget::Int(10, 1, 100_000))],
        outputs: &[THEN],
        width: W_FLOW,
    },
    // Nodes pures (sans exécution : calculées quand on lit leur sortie)
    NodeDef {
        key: "pu_contains",
        cat: Cat::Pure,
        fr: "Contient (Contains)",
        en: "Contains",
        desc_fr: "Vrai si le texte contient la recherche.",
        desc_en: "True if the text contains the search.",
        inputs: &[
            param("case", "Ignorer la casse", "Ignore case", PinType::Bool, Widget::Bool(true)),
            pin("text", "Texte", "Text", PinType::Text, Widget::Text("", "")),
            pin("search", "Recherche", "Search", PinType::Text, Widget::Text("urgent", "urgent")),
        ],
        outputs: &[pin("result", "", "", PinType::Bool, Widget::None)],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_break",
        cat: Cat::Pure,
        fr: "Décomposer (Break data)",
        en: "Break data",
        desc_fr: "Décompose un objet (mail, message, contact, rendez-vous, rappel, note, tâche) : sa classe, son logo, son titre, sa catégorie, son groupe…",
        desc_en: "Breaks an object (mail, message, contact, event, reminder, note, task) into its class, logo, title, category, group…",
        inputs: &[pin("obj", "Objet", "Object", PinType::Object, Widget::None)],
        outputs: &[
            pin("class", "Classe", "Class", PinType::Text, Widget::None),
            pin("logo", "Logo", "Logo", PinType::Text, Widget::None),
            pin("title", "Titre", "Title", PinType::Text, Widget::None),
            pin("text", "Contenu", "Content", PinType::Text, Widget::None),
            pin("author", "Auteur", "Author", PinType::Text, Widget::None),
            pin("category", "Catégorie", "Category", PinType::Text, Widget::None),
            pin("group", "Groupe", "Group", PinType::Text, Widget::None),
            pin("date", "Date", "Date", PinType::Text, Widget::None),
            pin("id", "Identifiant", "Identifier", PinType::Text, Widget::None),
        ],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_getvar",
        cat: Cat::Pure,
        fr: "Lire une variable (Get)",
        en: "Get a variable",
        desc_fr: "Valeur gardée par « Définir une variable » ou « Ajouter à une variable » (vide si elle n'existe pas).",
        desc_en: "Value kept by “Set a variable” or “Add to a variable” (empty if it doesn't exist).",
        inputs: &[VAR_NAME],
        outputs: &[pin("value", "", "", PinType::Text, Widget::None)],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_math",
        cat: Cat::Pure,
        fr: "Calcul (Math)",
        en: "Math",
        desc_fr: "Calcule A + B, A − B, A × B, A ÷ B (division entière) ou le reste de A ÷ B.",
        desc_en: "Computes A + B, A − B, A × B, A ÷ B (integer division) or the remainder of A ÷ B.",
        inputs: &[
            param(
                "op",
                "Opération",
                "Operation",
                PinType::Text,
                Widget::Choice(&[
                    ("add", "A + B", "A + B"),
                    ("sub", "A − B", "A − B"),
                    ("mul", "A × B", "A × B"),
                    ("div", "A ÷ B", "A ÷ B"),
                    ("mod", "Reste de A ÷ B", "Remainder of A ÷ B"),
                ]),
            ),
            pin("a", "A", "A", PinType::Int, Widget::Int(0, -1_000_000, 1_000_000)),
            pin("b", "B", "B", PinType::Int, Widget::Int(1, -1_000_000, 1_000_000)),
        ],
        outputs: &[pin("result", "", "", PinType::Int, Widget::None)],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_compare",
        cat: Cat::Pure,
        fr: "Comparer des nombres",
        en: "Compare numbers",
        desc_fr: "Vrai si A est plus petit, égal, plus grand… que B.",
        desc_en: "True if A is less than, equal to, greater than… B.",
        inputs: &[
            param(
                "op",
                "Comparaison",
                "Comparison",
                PinType::Text,
                Widget::Choice(&[
                    ("lt", "A < B", "A < B"),
                    ("le", "A ≤ B", "A ≤ B"),
                    ("eq", "A = B", "A = B"),
                    ("ge", "A ≥ B", "A ≥ B"),
                    ("gt", "A > B", "A > B"),
                    ("ne", "A ≠ B", "A ≠ B"),
                ]),
            ),
            pin("a", "A", "A", PinType::Int, Widget::Int(0, -1_000_000, 1_000_000)),
            pin("b", "B", "B", PinType::Int, Widget::Int(0, -1_000_000, 1_000_000)),
        ],
        outputs: &[pin("result", "", "", PinType::Bool, Widget::None)],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_now",
        cat: Cat::Pure,
        fr: "Maintenant (Now)",
        en: "Now",
        desc_fr: "Date et heure au moment de l'exécution : pour agir selon l'heure ou le jour (heures de bureau, week-end…).",
        desc_en: "Date and time when it runs: to act depending on the time or the day (office hours, weekend…).",
        inputs: &[],
        outputs: &[
            pin("time", "Heure", "Time", PinType::Text, Widget::None),
            pin("date", "Date", "Date", PinType::Text, Widget::None),
            pin("weekday", "Jour", "Day", PinType::Text, Widget::None),
            pin("hour", "Heures", "Hours", PinType::Int, Widget::None),
            pin("minute", "Minutes", "Minutes", PinType::Int, Widget::None),
            pin("weekend", "Week-end", "Weekend", PinType::Bool, Widget::None),
        ],
        width: W_PURE,
    },
    NodeDef {
        key: "pu_not",
        cat: Cat::Pure,
        fr: "Non (NOT)",
        en: "NOT",
        desc_fr: "Inverse un booléen.",
        desc_en: "Inverts a boolean.",
        inputs: &[pin("a", "", "", PinType::Bool, Widget::Bool(false))],
        outputs: &[pin("result", "", "", PinType::Bool, Widget::None)],
        width: W_GATE,
    },
    NodeDef {
        key: "pu_and",
        cat: Cat::Pure,
        fr: "Et (AND)",
        en: "AND",
        desc_fr: "Vrai si A et B sont vrais.",
        desc_en: "True if both A and B are true.",
        inputs: &[pin("a", "A", "A", PinType::Bool, Widget::Bool(false)), pin("b", "B", "B", PinType::Bool, Widget::Bool(false))],
        outputs: &[pin("result", "", "", PinType::Bool, Widget::None)],
        width: W_GATE,
    },
    NodeDef {
        key: "pu_or",
        cat: Cat::Pure,
        fr: "Ou (OR)",
        en: "OR",
        desc_fr: "Vrai si A ou B est vrai.",
        desc_en: "True if A or B is true.",
        inputs: &[pin("a", "A", "A", PinType::Bool, Widget::Bool(false)), pin("b", "B", "B", PinType::Bool, Widget::Bool(false))],
        outputs: &[pin("result", "", "", PinType::Bool, Widget::None)],
        width: W_GATE,
    },
    NodeDef {
        key: "pu_join",
        cat: Cat::Pure,
        fr: "Joindre (Append)",
        en: "Append",
        desc_fr: "Assemble A et B, séparés par le séparateur.",
        desc_en: "Joins A and B with the separator.",
        inputs: &[
            pin("a", "A", "A", PinType::Text, Widget::Text("", "")),
            pin("sep", "Séparateur", "Separator", PinType::Text, Widget::Text(" ", " ")),
            pin("b", "B", "B", PinType::Text, Widget::Text("", "")),
        ],
        outputs: &[pin("result", "", "", PinType::Text, Widget::None)],
        width: W_PURE,
    },
];

pub fn get(key: &str) -> Option<&'static NodeDef> {
    ALL.iter().find(|d| d.key == key)
}

/// Valeur d'un champ quand rien n'a été saisi.
pub fn default_value(w: Widget) -> serde_json::Value {
    match w {
        Widget::None => serde_json::Value::Null,
        Widget::Text(..) => serde_json::Value::String(String::new()),
        Widget::Bool(b) => serde_json::Value::Bool(b),
        Widget::Int(v, ..) => serde_json::Value::from(v),
        Widget::Choice(opts) => serde_json::Value::String(opts[0].0.to_string()),
        Widget::Icon => serde_json::Value::String(ICONS[0].0.to_string()),
        Widget::Keywords(..) => serde_json::Value::String(String::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<&str> = ALL.iter().map(|d| d.key).collect();
        keys.sort();
        let n = keys.len();
        keys.dedup();
        assert_eq!(keys.len(), n);
        for d in ALL {
            let mut pins: Vec<&str> = d.inputs.iter().chain(d.outputs).map(|p| p.key).collect();
            pins.sort();
            let n = pins.len();
            pins.dedup();
            assert_eq!(pins.len(), n, "pins en double dans {}", d.key);
        }
    }

    #[test]
    fn compatibility() {
        assert!(PinType::Text.accepts(PinType::Mail));
        assert!(PinType::Item.accepts(PinType::Message));
        assert!(!PinType::Mail.accepts(PinType::Text));
        assert!(!PinType::Exec.accepts(PinType::Text));
        assert!(!PinType::Text.accepts(PinType::Exec));
    }
}
