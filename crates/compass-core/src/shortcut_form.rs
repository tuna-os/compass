//! The quicklink form: what it prefills, what it demands, and what it saves.
//!
//! A port of `ShortcutFormViewHost`
//! (`src/server/src/builtins/shortcut/shortcut-form-view-host.cpp`), without
//! the QML. [`crate::shortcut`] parses the link this form collects and
//! [`crate::shortcut_store`] stores the result.

/// Why the form is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// A new quicklink.
    #[default]
    Create,
    /// Editing an existing one in place.
    Edit,
    /// Starting from an existing one, saved as a new one.
    Duplicate,
}

/// The app id meaning "whatever opens this link".
pub const DEFAULT_APP: &str = "default";
/// The icon id meaning "whatever the link resolves to".
pub const DEFAULT_ICON: &str = "default";
/// The label of the default icon entry.
pub const DEFAULT_ICON_LABEL: &str = "Default";

/// The error on a field left empty.
pub const REQUIRED: &str = "Required";
/// The toast when the form does not validate.
pub const VALIDATION_FAILED: &str = "Validation failed";
/// The toast on a successful edit.
pub const UPDATED: &str = "Shortcut updated";
/// The toast on a failed edit.
pub const UPDATE_FAILED: &str = "Failed to update shortcut";
/// The toast on a successful create.
pub const CREATED: &str = "Shortcut created";
/// The toast on a failed create.
pub const CREATE_FAILED: &str = "Failed to create shortcut";
/// The submit action's title.
pub const SUBMIT_TITLE: &str = "Submit";

/// One entry in the link field's completion menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LinkCompletion {
    /// What the menu shows.
    pub title: &'static str,
    /// The placeholder id it inserts.
    pub value: &'static str,
    /// The text inserted, when it is more than `{value}`.
    pub template: Option<&'static str>,
    /// Where to put the cursor within that text.
    pub cursor_offset: Option<usize>,
}

/// The three completions the link field offers.
///
/// The first two insert a bare placeholder; the third inserts
/// `{argument name=""}` and puts the cursor **between the quotes**, which is
/// character 16 — the point of the offset is that the user types the argument's
/// name next.
pub const LINK_COMPLETIONS: &[LinkCompletion] = &[
    LinkCompletion {
        title: "Selected Text",
        value: "selection",
        template: None,
        cursor_offset: None,
    },
    LinkCompletion {
        title: "Clipboard Text",
        value: "clipboard",
        template: None,
        cursor_offset: None,
    },
    LinkCompletion {
        title: "Argument",
        value: "argument",
        template: Some("{argument name=\"\"}"),
        cursor_offset: Some(16),
    },
];

/// An existing quicklink the form starts from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Existing {
    /// Its id.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its link.
    pub url: String,
    /// The app id it opens with.
    pub app: String,
    /// Its icon id.
    pub icon: String,
}

/// The form's initial state.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Initial {
    /// The name field.
    pub name: String,
    /// The link field.
    pub link: String,
    /// The selected app id.
    pub app: String,
    /// The selected icon id.
    pub icon: String,
    /// The navigation title, empty when creating.
    pub navigation_title: String,
}

/// What the form starts with.
///
/// `app_exists` answers `appDb->findById(...)`, and `icon_exists` answers
/// whether the icon id is one of the offered icons; a quicklink whose app was
/// uninstalled or whose icon is unknown falls back to the defaults rather than
/// carrying a dangling reference into the form.
#[must_use]
pub fn initial(
    mode: Mode,
    existing: Option<&Existing>,
    app_exists: bool,
    icon_exists: bool,
) -> Initial {
    let Some(existing) = existing else {
        return Initial {
            app: DEFAULT_APP.to_owned(),
            icon: DEFAULT_ICON.to_owned(),
            ..Initial::default()
        };
    };

    Initial {
        // `tr("Copy of %1")` -- only when duplicating.
        name: match mode {
            Mode::Duplicate => format!("Copy of {}", existing.name),
            _ => existing.name.clone(),
        },
        link: existing.url.clone(),
        // `!isDefaultApp() && findById(appId)` -- both, so an uninstalled app
        // reverts to "default" rather than showing a stale name.
        app: if existing.app != DEFAULT_APP && app_exists {
            existing.app.clone()
        } else {
            DEFAULT_APP.to_owned()
        },
        icon: if icon_exists {
            existing.icon.clone()
        } else {
            DEFAULT_ICON.to_owned()
        },
        navigation_title: match mode {
            Mode::Create => String::new(),
            Mode::Edit => format!("Edit \"{}\"", existing.name),
            Mode::Duplicate => format!("Duplicate \"{}\"", existing.name),
        },
    }
}

/// The form's errors; `None` where the field is fine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Errors {
    /// The link field's error.
    pub link: Option<&'static str>,
    /// The app field's error.
    pub app: Option<&'static str>,
    /// The icon field's error.
    pub icon: Option<&'static str>,
}

impl Errors {
    /// Whether anything failed.
    #[must_use]
    pub fn any(&self) -> bool {
        self.link.is_some() || self.app.is_some() || self.icon.is_some()
    }
}

/// Validates the three required fields.
///
/// The **name is not one of them**: a quicklink with no name is allowed, and
/// the list shows an empty title for it. That is the C++'s behaviour and it is
/// reproduced rather than tidied, because a form that suddenly demanded a name
/// would reject quicklinks that already exist.
#[must_use]
pub fn validate(link: &str, app: &str, icon: &str) -> Errors {
    Errors {
        link: link.is_empty().then_some(REQUIRED),
        app: app.is_empty().then_some(REQUIRED),
        icon: icon.is_empty().then_some(REQUIRED),
    }
}

/// What submitting the form does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// Show the errors and one toast.
    Rejected {
        /// Which fields failed.
        errors: Errors,
        /// The toast's text.
        toast: &'static str,
    },
    /// Update the existing quicklink.
    Update {
        /// Which one.
        id: String,
        /// The name, which may be empty.
        name: String,
        /// The icon id, with `default` already resolved.
        icon: String,
        /// The link.
        link: String,
        /// The app id.
        app: String,
        /// The toast on success.
        success: &'static str,
        /// The toast on failure.
        failure: &'static str,
    },
    /// Create a new one. **Duplicating comes here too**: the C++ branches on
    /// `Mode::Edit` alone, so a duplicate is a create with a prefilled form.
    Create {
        /// The name, which may be empty.
        name: String,
        /// The icon id, with `default` already resolved.
        icon: String,
        /// The link.
        link: String,
        /// The app id.
        app: String,
        /// The toast on success.
        success: &'static str,
        /// The toast on failure.
        failure: &'static str,
    },
}

/// Decides what a submission does.
///
/// `resolved_default_icon` is what the `default` icon currently stands for —
/// the opener's icon, a favicon, or the built-in link glyph. The id stored is
/// that resolved value, not the word "default", so a quicklink keeps the icon
/// it was shown with even if the default later changes.
#[must_use]
pub fn submit(
    mode: Mode,
    existing: Option<&Existing>,
    name: &str,
    link: &str,
    app: &str,
    icon: &str,
    resolved_default_icon: &str,
) -> Submission {
    let errors = validate(link, app, icon);
    if errors.any() {
        return Submission::Rejected {
            errors,
            toast: VALIDATION_FAILED,
        };
    }

    let icon = if icon == DEFAULT_ICON {
        resolved_default_icon.to_owned()
    } else {
        icon.to_owned()
    };

    if mode == Mode::Edit
        && let Some(existing) = existing
    {
        return Submission::Update {
            id: existing.id.clone(),
            name: name.to_owned(),
            icon,
            link: link.to_owned(),
            app: app.to_owned(),
            success: UPDATED,
            failure: UPDATE_FAILED,
        };
    }

    Submission::Create {
        name: name.to_owned(),
        icon,
        link: link.to_owned(),
        app: app.to_owned(),
        success: CREATED,
        failure: CREATE_FAILED,
    }
}

/// What the `default` icon resolves to as the link changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultIcon {
    /// The built-in link glyph: nothing better is known yet.
    BuiltinLink,
    /// The icon of the application that opens this link.
    Opener {
        /// Its icon reference.
        icon: String,
    },
    /// The site's favicon, labelled with the host.
    Favicon {
        /// The host the icon belongs to.
        host: String,
    },
}

/// Resolves the default icon for a link.
///
/// Two steps, in this order: the default opener's icon if the link has one, and
/// then — only for a scheme *starting with* `http`, which takes in `https` and
/// also `httpx` — the site's favicon, which wins because it arrives later.
#[must_use]
pub fn default_icon(opener_icon: Option<&str>, scheme: &str, host: &str) -> DefaultIcon {
    if scheme.starts_with("http") && !host.is_empty() {
        return DefaultIcon::Favicon {
            host: host.to_owned(),
        };
    }

    match opener_icon {
        Some(icon) => DefaultIcon::Opener {
            icon: icon.to_owned(),
        },
        None => DefaultIcon::BuiltinLink,
    }
}
