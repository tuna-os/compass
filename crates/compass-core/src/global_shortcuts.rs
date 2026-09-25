//! The global shortcuts the configuration declares, and how they are kept
//! bound: `GlobalShortcutService` (`src/server/src/services/global-shortcuts`)
//! without the backend.
//!
//! The configuration is the only source of truth, as in the C++: the
//! launcher's hotkey is `launcher.hotkey` (the C++ keeps it in
//! `globalShortcuts.toggle`), and each command's is
//! `providers.<p>.entrypoints.<e>.shortcut`, in the spelling the shortcut
//! recorder writes ([`crate::key_combo`]). [`desired`] reads what should be
//! bound; a [`Reconciler`] turns that into the unbinds and binds a backend
//! must make, keeping what is already bound as it is (`reconcile`); and
//! [`validate`] is the recorder's check (`shortcut_conflict::validate`): a
//! modifier, then the launcher's own keys (`KeybindManager::findBoundInfo`),
//! then every global shortcut (`GlobalShortcutService::findConflict`).
//!
//! What a backend is told is a keysym and a modifier mask for the Wayland
//! hotkey protocols ([`keysym`], [`modifier_mask`], after the C++'s
//! `xkbKeysymForQtKey`), or the portal's trigger text ([`portal_trigger`]).

use std::collections::BTreeMap;

use crate::config::Config;
use crate::key_combo::{Key, KeyCombo, MODIFIER_REQUIRED, Modifier, Modifiers};

/// The launcher hotkey's id. The C++ says `@toggle-launcher`; the portal
/// keeps the user's chosen trigger against the id, and Compass has bound its
/// launcher as `toggle` since the first release, so that is kept.
pub const LAUNCHER_ID: &str = "toggle";

/// What the desktop shows for the launcher hotkey.
pub const LAUNCHER_DESCRIPTION: &str = "Open the Compass launcher";

/// How a conflict with the launcher hotkey is named (`findConflict`).
pub const LAUNCHER_HOTKEY_NAME: &str = "the launcher hotkey";

/// How a conflict with an item whose title is unknown is named.
pub const ANOTHER_COMMAND: &str = "another command";

/// The launcher's own keys, which a global shortcut must not take:
/// `KeybindManager`'s, as far as Compass has them (fixed, see
/// [`crate::settings_catalog::KEYBINDINGS`]). `(name, stored shortcut)`.
pub const LAUNCHER_KEYBINDS: &[(&str, &str)] = &[
    ("Toggle action panel", "control+B"),
    ("Open settings", "control+,"),
    ("Quick launch", "control+1"),
    ("Quick launch", "control+2"),
    ("Quick launch", "control+3"),
    ("Quick launch", "control+4"),
    ("Quick launch", "control+5"),
    ("Quick launch", "control+6"),
    ("Quick launch", "control+7"),
    ("Quick launch", "control+8"),
    ("Quick launch", "control+9"),
];

/// What pressing a global shortcut does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Show or hide the launcher (`ToggleLauncherWindow`).
    ToggleLauncher,
    /// Launch the root item with this id, as root search would
    /// (`RunCommand`, `activateEntrypoint`).
    RunCommand(String),
}

/// One shortcut the configuration asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    /// [`LAUNCHER_ID`], or the root item's `provider:entrypoint` id.
    pub id: String,
    /// The shortcut as the configuration spells it.
    pub trigger: String,
    /// The same, read.
    pub combo: KeyCombo,
    /// What the desktop shows for it: the item's title.
    pub description: String,
    /// What pressing it does.
    pub action: Action,
}

/// Every shortcut `config` declares, by id: the launcher hotkey unless it is
/// empty, and each entrypoint's `shortcut` unless it is empty or the
/// entrypoint is turned off. A shortcut that does not read as a combination
/// is left out, as `reconcile` skips one that is not `isValid`. `title`
/// names an item for the desktop's list; an unknown one is named by its id.
#[must_use]
pub fn desired(
    config: &Config,
    title: impl Fn(&str) -> Option<String>,
) -> BTreeMap<String, Binding> {
    let mut bindings = BTreeMap::new();
    let hotkey = config.launcher().hotkey();
    if let Some(combo) = KeyCombo::parse(hotkey) {
        bindings.insert(
            LAUNCHER_ID.to_owned(),
            Binding {
                id: LAUNCHER_ID.to_owned(),
                trigger: hotkey.to_owned(),
                combo,
                description: LAUNCHER_DESCRIPTION.to_owned(),
                action: Action::ToggleLauncher,
            },
        );
    }
    for (provider, settings) in &config.root_config().providers {
        for (entrypoint, item) in &settings.entrypoints {
            let Some(trigger) = item.shortcut.as_deref().filter(|s| !s.is_empty()) else {
                continue;
            };
            if item.enabled == Some(false) {
                continue;
            }
            let Some(combo) = KeyCombo::parse(trigger) else {
                continue;
            };
            let id = format!("{provider}:{entrypoint}");
            bindings.insert(
                id.clone(),
                Binding {
                    description: title(&id).unwrap_or_else(|| id.clone()),
                    id: id.clone(),
                    trigger: trigger.to_owned(),
                    combo,
                    action: Action::RunCommand(id),
                },
            );
        }
    }
    bindings
}

/// What a backend has to do to go from what is bound to what is desired.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Plan {
    /// Ids to release, first.
    pub unbind: Vec<String>,
    /// Shortcuts to bind, new or with a new trigger.
    pub bind: Vec<Binding>,
}

impl Plan {
    /// Whether nothing changes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.unbind.is_empty() && self.bind.is_empty()
    }
}

/// What is bound, and what each bound id does (`m_appliedTriggers`,
/// `m_actions`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Reconciler {
    applied: BTreeMap<String, String>,
    actions: BTreeMap<String, Action>,
}

impl Reconciler {
    /// `reconcile`'s diff: a bound id no longer desired, or desired with
    /// another trigger, is released and forgotten; a desired one not bound
    /// with its trigger is to be bound. Report each bind's result with
    /// [`Self::bound`].
    pub fn plan(&mut self, desired: &BTreeMap<String, Binding>) -> Plan {
        let unbind: Vec<String> = self
            .applied
            .iter()
            .filter(|(id, trigger)| desired.get(*id).is_none_or(|b| &b.trigger != *trigger))
            .map(|(id, _)| id.clone())
            .collect();
        for id in &unbind {
            self.applied.remove(id);
            self.actions.remove(id);
        }
        let bind = desired
            .values()
            .filter(|binding| self.applied.get(&binding.id) != Some(&binding.trigger))
            .cloned()
            .collect();
        Plan { unbind, bind }
    }

    /// A bind's result. The trigger counts as applied either way, so a
    /// shortcut the desktop refused is not asked for again until it changes;
    /// only one that was bound does anything when pressed.
    pub fn bound(&mut self, binding: &Binding, result: &Result<(), String>) {
        self.applied
            .insert(binding.id.clone(), binding.trigger.clone());
        match result {
            Ok(()) => {
                self.actions
                    .insert(binding.id.clone(), binding.action.clone());
            }
            Err(_) => {
                self.actions.remove(&binding.id);
            }
        }
    }

    /// Everything was released (`unbindAll`: capturing, or a backend reset).
    pub fn clear(&mut self) {
        self.applied.clear();
        self.actions.clear();
    }

    /// What pressing `id` does, when it is bound (`onActivated`).
    #[must_use]
    pub fn action(&self, id: &str) -> Option<&Action> {
        self.actions.get(id)
    }

    /// The ids bound and live, in order.
    #[must_use]
    pub fn live(&self) -> Vec<&str> {
        self.actions.keys().map(String::as_str).collect()
    }
}

/// `KeybindManager::findBoundInfo`: the name of the launcher key `combo` is.
#[must_use]
pub fn launcher_keybind(combo: &KeyCombo) -> Option<&'static str> {
    LAUNCHER_KEYBINDS
        .iter()
        .find(|(_, stored)| KeyCombo::parse(stored).as_ref() == Some(combo))
        .map(|(name, _)| *name)
}

/// `GlobalShortcutService::findConflict`: the name of the global shortcut
/// already on `combo`, other than `exclude_id`'s. The launcher hotkey
/// (`launcher_hotkey`, as stored) is "the launcher hotkey"; an item is its
/// title from `bound` (`(id, title, stored shortcut)`), or "another command"
/// when it has none.
#[must_use]
pub fn find_conflict<'a>(
    combo: &KeyCombo,
    exclude_id: &str,
    launcher_hotkey: Option<&str>,
    bound: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Option<String> {
    if exclude_id != LAUNCHER_ID
        && launcher_hotkey
            .filter(|hotkey| !hotkey.is_empty())
            .and_then(KeyCombo::parse)
            .as_ref()
            == Some(combo)
    {
        return Some(LAUNCHER_HOTKEY_NAME.to_owned());
    }
    bound
        .into_iter()
        .find(|(id, _, shortcut)| {
            *id != exclude_id
                && !shortcut.is_empty()
                && KeyCombo::parse(shortcut).as_ref() == Some(combo)
        })
        .map(|(_, title, _)| {
            if title.is_empty() {
                ANOTHER_COMMAND.to_owned()
            } else {
                title.to_owned()
            }
        })
}

/// `shortcut_conflict::validate`: whether `combo` may be recorded for
/// `exclude_id` ([`LAUNCHER_ID`] for the launcher hotkey, else a root item's
/// id). A modifier is needed unless it is a function key or modifiers alone;
/// then it must not be one of the launcher's keys, the launcher hotkey or
/// another item's shortcut.
///
/// # Errors
///
/// [`MODIFIER_REQUIRED`], or `Already bound to "<name>"`.
pub fn validate<'a>(
    combo: &KeyCombo,
    exclude_id: &str,
    launcher_hotkey: Option<&str>,
    bound: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
) -> Result<(), String> {
    if combo.modifiers.is_empty() && !combo.key.is_function_key() && !combo.is_modifier_only() {
        return Err(MODIFIER_REQUIRED.to_owned());
    }
    let taken = launcher_keybind(combo)
        .map(str::to_owned)
        .or_else(|| find_conflict(combo, exclude_id, launcher_hotkey, bound));
    match taken {
        Some(name) => Err(format!("Already bound to \"{name}\"")),
        None => Ok(()),
    }
}

/// `xkbKeysymForQtKey`: the unshifted XKB keysym of `key`.
#[must_use]
pub fn keysym(key: &Key) -> Option<u32> {
    use xkeysym::Keysym;
    let sym = match key {
        Key::Modifier(Modifier::Super) => Keysym::Super_L,
        Key::Modifier(Modifier::Control) => Keysym::Control_L,
        Key::Modifier(Modifier::Alt) => Keysym::Alt_L,
        Key::Modifier(Modifier::Shift) => Keysym::Shift_L,
        Key::Named(name) => match name.as_str() {
            "return" => Keysym::Return,
            "enter" => Keysym::KP_Enter,
            "escape" => Keysym::Escape,
            "tab" => Keysym::Tab,
            "backspace" => Keysym::BackSpace,
            "delete" => Keysym::Delete,
            "home" => Keysym::Home,
            "end" => Keysym::End,
            "pageup" => Keysym::Page_Up,
            "pagedown" => Keysym::Page_Down,
            "arrowleft" => Keysym::Left,
            "arrowright" => Keysym::Right,
            "arrowup" => Keysym::Up,
            "arrowdown" => Keysym::Down,
            "space" => Keysym::space,
            other => {
                if let Some(number) = other.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
                    && (1..=35).contains(&number)
                {
                    return Some(Keysym::F1.raw() + number - 1);
                }
                let mut chars = other.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) if c.is_ascii_graphic() => {
                        Keysym::from_char(c.to_ascii_lowercase())
                    }
                    _ => return None,
                }
            }
        },
    };
    Some(sym.raw())
}

/// The hotkey protocols' modifier mask (`fromQtMods`): shift 1, ctrl 2,
/// alt 4, super 8.
#[must_use]
pub fn modifier_mask(modifiers: Modifiers) -> u32 {
    [
        (modifiers.shift, 1),
        (modifiers.control, 2),
        (modifiers.alt, 4),
        (modifiers.super_key, 8),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .map(|(_, bit)| bit)
    .sum()
}

/// The portal's `preferred_trigger` for `combo`, in the "shortcuts"
/// specification's spelling: `CTRL+ALT+SHIFT+LOGO+`, then the XKB keysym
/// name (`LOGO+space`, `CTRL+SHIFT+a`, `ALT+F4`).
#[must_use]
pub fn portal_trigger(combo: &KeyCombo) -> Option<String> {
    let sym = xkeysym::Keysym::new(keysym(&combo.key)?);
    let name = sym.name()?.strip_prefix("XK_")?;
    let mut trigger = String::new();
    for (held, modifier) in [
        (combo.modifiers.control, "CTRL"),
        (combo.modifiers.alt, "ALT"),
        (combo.modifiers.shift, "SHIFT"),
        (combo.modifiers.super_key, "LOGO"),
    ] {
        if held {
            trigger.push_str(modifier);
            trigger.push('+');
        }
    }
    trigger.push_str(name);
    Some(trigger)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(json: &str) -> Config {
        serde_json::from_str(json).expect("a configuration")
    }

    fn combo(text: &str) -> KeyCombo {
        KeyCombo::parse(text).expect("a combination")
    }

    #[test]
    fn the_launcher_and_every_enabled_items_shortcut_are_desired() {
        let config = config(
            r#"{
              "launcher": {"hotkey": "alt+SPACE"},
              "providers": {
                "clipboard": {"entrypoints": {
                  "history": {"shortcut": "super+shift+V"},
                  "off": {"shortcut": "super+O", "enabled": false},
                  "empty": {"shortcut": ""},
                  "garbage": {"shortcut": "control+nope"}
                }},
                "applications": {"entrypoints": {"firefox": {"shortcut": "super+F"}}}
              }
            }"#,
        );
        let desired = desired(&config, |id| {
            (id == "clipboard:history").then(|| "Clipboard History".to_owned())
        });
        assert_eq!(
            desired.keys().map(String::as_str).collect::<Vec<_>>(),
            ["applications:firefox", "clipboard:history", LAUNCHER_ID]
        );
        let launcher = &desired[LAUNCHER_ID];
        assert_eq!(launcher.action, Action::ToggleLauncher);
        assert_eq!(launcher.combo, combo("alt+space"));
        let history = &desired["clipboard:history"];
        assert_eq!(history.description, "Clipboard History");
        assert_eq!(
            history.action,
            Action::RunCommand("clipboard:history".to_owned())
        );
        assert_eq!(
            desired["applications:firefox"].description,
            "applications:firefox"
        );
    }

    #[test]
    fn the_launcher_hotkey_defaults_to_super_space_and_an_empty_one_binds_nothing() {
        let default = desired(&Config::default(), |_| None);
        assert_eq!(default[LAUNCHER_ID].combo, combo("super+space"));
        let empty = desired(&config(r#"{"launcher": {"hotkey": ""}}"#), |_| None);
        assert!(empty.is_empty());
    }

    fn binding(id: &str, trigger: &str) -> Binding {
        Binding {
            id: id.to_owned(),
            trigger: trigger.to_owned(),
            combo: combo(trigger),
            description: id.to_owned(),
            action: if id == LAUNCHER_ID {
                Action::ToggleLauncher
            } else {
                Action::RunCommand(id.to_owned())
            },
        }
    }

    fn set(bindings: &[Binding]) -> BTreeMap<String, Binding> {
        bindings.iter().map(|b| (b.id.clone(), b.clone())).collect()
    }

    #[test]
    fn reconciling_binds_what_is_new_rebinds_what_changed_and_keeps_the_rest() {
        let mut reconciler = Reconciler::default();
        let first = set(&[
            binding(LAUNCHER_ID, "super+SPACE"),
            binding("a:b", "super+B"),
        ]);
        let plan = reconciler.plan(&first);
        assert!(plan.unbind.is_empty());
        assert_eq!(plan.bind.len(), 2);
        for b in &plan.bind {
            reconciler.bound(b, &Ok(()));
        }
        assert!(
            reconciler.plan(&first).is_empty(),
            "nothing changed, nothing to do"
        );

        let second = set(&[binding(LAUNCHER_ID, "alt+SPACE"), binding("c:d", "super+D")]);
        let plan = reconciler.plan(&second);
        assert_eq!(plan.unbind, ["a:b".to_owned(), LAUNCHER_ID.to_owned()]);
        assert_eq!(
            plan.bind.iter().map(|b| b.id.as_str()).collect::<Vec<_>>(),
            ["c:d", LAUNCHER_ID]
        );
        assert_eq!(
            reconciler.action("a:b"),
            None,
            "a released shortcut does nothing"
        );
    }

    #[test]
    fn a_refused_bind_does_nothing_when_pressed_and_is_not_asked_for_again() {
        let mut reconciler = Reconciler::default();
        let desired = set(&[binding("a:b", "super+B")]);
        let plan = reconciler.plan(&desired);
        reconciler.bound(&plan.bind[0], &Err("taken".to_owned()));
        assert_eq!(reconciler.action("a:b"), None);
        assert!(reconciler.plan(&desired).is_empty());
        reconciler.clear();
        assert_eq!(
            reconciler.plan(&desired).bind.len(),
            1,
            "a reset binds again"
        );
    }

    #[test]
    fn a_bound_shortcut_runs_its_action() {
        let mut reconciler = Reconciler::default();
        let desired = set(&[
            binding(LAUNCHER_ID, "super+SPACE"),
            binding("a:b", "super+B"),
        ]);
        for b in reconciler.plan(&desired).bind {
            reconciler.bound(&b, &Ok(()));
        }
        assert_eq!(
            reconciler.action(LAUNCHER_ID),
            Some(&Action::ToggleLauncher)
        );
        assert_eq!(
            reconciler.action("a:b"),
            Some(&Action::RunCommand("a:b".to_owned()))
        );
        assert_eq!(reconciler.action("someone-else"), None);
        assert_eq!(reconciler.live(), ["a:b", LAUNCHER_ID]);
    }

    #[test]
    fn the_recorder_refuses_the_launchers_own_keys() {
        assert_eq!(
            validate(&combo("control+B"), "a:b", None, []),
            Err("Already bound to \"Toggle action panel\"".to_owned())
        );
        assert_eq!(
            validate(&combo("control+3"), "a:b", None, []),
            Err("Already bound to \"Quick launch\"".to_owned())
        );
        assert_eq!(
            validate(&combo("control+,"), "a:b", None, []),
            Err("Already bound to \"Open settings\"".to_owned())
        );
    }

    #[test]
    fn the_recorder_refuses_the_launcher_hotkey_except_for_itself() {
        let hotkey = Some("super+SPACE");
        assert_eq!(
            validate(&combo("super+space"), "a:b", hotkey, []),
            Err("Already bound to \"the launcher hotkey\"".to_owned())
        );
        assert_eq!(
            validate(&combo("super+space"), LAUNCHER_ID, hotkey, []),
            Ok(())
        );
        assert_eq!(validate(&combo("super+space"), "a:b", Some(""), []), Ok(()));
    }

    #[test]
    fn the_recorder_refuses_another_items_shortcut_by_its_title() {
        let bound = [("a:b", "Files", "super+F"), ("c:d", "", "super+G")];
        assert_eq!(
            validate(&combo("super+f"), "x:y", None, bound),
            Err("Already bound to \"Files\"".to_owned())
        );
        assert_eq!(validate(&combo("super+f"), "a:b", None, bound), Ok(()));
        assert_eq!(
            validate(&combo("super+g"), "x:y", None, bound),
            Err("Already bound to \"another command\"".to_owned())
        );
        assert_eq!(
            validate(&combo("super+f"), LAUNCHER_ID, None, bound),
            Err("Already bound to \"Files\"".to_owned()),
            "the launcher hotkey may not take an item's shortcut either"
        );
        assert_eq!(
            validate(&combo("h"), "x:y", None, bound),
            Err(MODIFIER_REQUIRED.to_owned())
        );
    }

    #[test]
    fn keys_become_the_keysyms_the_cpp_asks_for() {
        assert_eq!(keysym(&combo("super+space").key), Some(0x20));
        assert_eq!(keysym(&combo("super+A").key), Some(u32::from(b'a')));
        assert_eq!(keysym(&combo("super+5").key), Some(u32::from(b'5')));
        assert_eq!(keysym(&combo("super+.").key), Some(0x2e));
        assert_eq!(keysym(&combo("F1").key), Some(0xffbe));
        assert_eq!(keysym(&combo("F12").key), Some(0xffc9));
        assert_eq!(keysym(&combo("super+return").key), Some(0xff0d));
        assert_eq!(keysym(&combo("super+enter").key), Some(0xff8d));
        assert_eq!(keysym(&combo("super+arrowup").key), Some(0xff52));
        assert_eq!(keysym(&combo("control+super").key), Some(0xffeb));
    }

    #[test]
    fn modifiers_become_the_protocols_mask() {
        assert_eq!(modifier_mask(combo("super+space").modifiers), 8);
        assert_eq!(
            modifier_mask(combo("super+control+alt+shift+A").modifiers),
            15
        );
        assert_eq!(modifier_mask(combo("control+shift+A").modifiers), 3);
    }

    #[test]
    fn the_portal_is_asked_in_the_specifications_spelling() {
        assert_eq!(
            portal_trigger(&combo("super+SPACE")).as_deref(),
            Some("LOGO+space")
        );
        assert_eq!(
            portal_trigger(&combo("super+control+alt+shift+A")).as_deref(),
            Some("CTRL+ALT+SHIFT+LOGO+a")
        );
        assert_eq!(portal_trigger(&combo("alt+F4")).as_deref(), Some("ALT+F4"));
        assert_eq!(
            portal_trigger(&combo("control+return")).as_deref(),
            Some("CTRL+Return")
        );
        assert_eq!(
            portal_trigger(&combo("control+,")).as_deref(),
            Some("CTRL+comma")
        );
    }
}
