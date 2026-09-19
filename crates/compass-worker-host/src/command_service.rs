//! The `Command` half of the extension API.
//!
//! Ports `ExtCommandService`
//! (`src/server/src/extension/api/command-service.hpp`): four methods by which
//! an extension launches a sibling command, relabels itself in the root list,
//! or sends the user to its preferences.
//!
//! The registry, the navigation controller and the settings controller are one
//! [`Commands`] trait here. They are three collaborators in the C++ because
//! they are three Qt objects; what the *service* does with them is small, and
//! that is what this pins.

use crate::tsapi::{self, Call};

/// The methods this serves, as they appear on the wire.
pub const METHODS: &[&str] = &[
    "Command/launchCommand",
    "Command/updateCommandMetadata",
    "Command/openExtensionPreferences",
    "Command/openCommandPreferences",
];

/// The error `launchCommand` fails with, verbatim from the C++.
pub const NO_SUCH_COMMAND: &str = "No such command";

/// `LaunchProps`, as `transformApiLaunchProps` fills it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LaunchProps {
    /// `options.context`, passed through untouched — it is `any` in the IDL and
    /// belongs to the extension that sent it.
    pub launch_context: Option<serde_json::Value>,
    /// The text the user had typed, when the launch carries one.
    pub fallback_text: Option<String>,
    /// The named arguments, in key order.
    pub arguments: Vec<(String, String)>,
}

/// What the command service needs from the rest of the launcher.
pub trait Commands {
    /// The entrypoint id of `command_id` in the named extension, if it exists.
    ///
    /// The C++ walks the root manager's extensions comparing the repository's
    /// name and author and then each command's id; a backend can index that
    /// however it likes, as long as all three have to match.
    fn find_command(
        &self,
        extension_name: &str,
        owner_or_author: &str,
        command_id: &str,
    ) -> Option<String>;

    /// `NavigationController::activateEntrypoint`.
    fn activate(&self, entrypoint_id: &str, props: LaunchProps);

    /// `ExtensionCommand::setSubtitleOverride`; `None` clears it.
    fn set_subtitle_override(&self, subtitle: Option<&str>);

    /// `emit RootItemManager::itemsChanged()`.
    fn items_changed(&self);

    /// `SettingsController::openExtensionPreferences` for the running command.
    fn open_preferences(&self);
}

/// Serves `Command` for one running command.
#[derive(Debug)]
pub struct CommandService<C> {
    commands: C,
}

impl<C: Commands> CommandService<C> {
    /// Serves `commands`.
    pub const fn new(commands: C) -> Self {
        Self { commands }
    }

    /// The backend this serves.
    pub const fn commands(&self) -> &C {
        &self.commands
    }

    /// Answers `call`, or `None` if it is not a `Command` call.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        Some(match call.method.as_str() {
            "Command/launchCommand" => {
                let options = call.params.get("options");
                let field = |name: &str| {
                    options
                        .and_then(|options| options.get(name))
                        .filter(|value| !value.is_null())
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                };
                match self.commands.find_command(
                    field("extensionName"),
                    field("ownerOrAuthorName"),
                    field("name"),
                ) {
                    Some(entrypoint) => {
                        self.commands.activate(&entrypoint, launch_props(options));
                        tsapi::reply(id, serde_json::Value::Null)
                    }
                    None => tsapi::reply_error(id, NO_SUCH_COMMAND),
                }
            }
            "Command/updateCommandMetadata" => {
                // `if (payload.subtitle && !payload.subtitle->empty())` --
                // an empty subtitle clears the override rather than setting an
                // empty one, which is how an extension takes its subtitle back
                // off the root list.
                let subtitle = call
                    .params
                    .get("payload")
                    .and_then(|payload| payload.get("subtitle"))
                    .and_then(serde_json::Value::as_str)
                    .filter(|subtitle| !subtitle.is_empty());
                self.commands.set_subtitle_override(subtitle);
                // Emitted whether or not anything changed: the root list has to
                // redraw either way, and the C++ does not guard it.
                self.commands.items_changed();
                tsapi::reply(id, serde_json::Value::Null)
            }
            // "for now both behave the same" -- the C++ `openExtensionPreferences`
            // is a one-line delegation to `openCommandPreferences`.
            _ => {
                self.commands.open_preferences();
                tsapi::reply(id, serde_json::Value::Null)
            }
        })
    }
}

impl<C: Commands> tsapi::Service for CommandService<C> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The C++ `transformApiLaunchProps`.
///
/// Two rules that are easy to miss: `arguments` is read only when it *is an
/// object* (`args->is_object()`), and within it only **string** values are
/// kept. An extension that passes a number or a nested object loses that
/// argument silently, exactly as it does today.
///
/// The pairs come out in key order. `serde_json::Map` is a `BTreeMap` by
/// default, so this is deterministic; the C++ order is glaze's, over its own
/// object type.
#[must_use]
pub fn launch_props(options: Option<&serde_json::Value>) -> LaunchProps {
    let Some(options) = options else {
        return LaunchProps::default();
    };

    let arguments = options
        .get("arguments")
        .and_then(serde_json::Value::as_object)
        .map(|args| {
            args.iter()
                .filter_map(|(key, value)| {
                    value.as_str().map(|value| (key.clone(), value.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default();

    LaunchProps {
        launch_context: options
            .get("context")
            .filter(|context| !context.is_null())
            .cloned(),
        fallback_text: options
            .get("fallbackText")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        arguments,
    }
}
