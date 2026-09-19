//! The "Create Extension" form: what it demands, what it says when it is not
//! satisfied, and what happens next.
//!
//! A port of `CreateExtensionViewHost::submit`
//! (`src/server/src/builtins/developer/create-extension-view-host.cpp`).
//!
//! # Every field is checked, every time
//!
//! `submit()` clears all six errors, runs all six checks without an early
//! return, and only then reports. A form that stopped at the first problem
//! would make the user submit six times to find six mistakes.

/// The minimum length of the author, title, command title and command
/// description fields.
pub const MIN_SHORT: usize = 3;
/// The minimum length of the extension description — longer, because it is
/// what a reader sees in a list.
pub const MIN_DESCRIPTION: usize = 16;

/// The error on a field that is too short at [`MIN_SHORT`].
pub const TOO_SHORT: &str = "Min. 3 chars";
/// The error on a description that is too short at [`MIN_DESCRIPTION`].
pub const DESCRIPTION_TOO_SHORT: &str = "Min. 16 chars";
/// The error on a location that is not a directory.
pub const LOCATION_MISSING: &str = "Must exist";

/// The toast when the form has any error at all.
pub const FORM_HAS_ERRORS: &str = "Form has errors";
/// The toast when generation itself fails.
pub const GENERATION_FAILED: &str = "Failed to create extension";
/// The submit action's title.
pub const SUBMIT_TITLE: &str = "Create extension";
/// The navigation title of the view that follows a success.
pub const SUCCESS_TITLE: &str = "Extension created!";
/// The emoji that view is shown with.
pub const SUCCESS_EMOJI: &str = "🥳";

/// The form's fields, as typed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Form {
    /// Who is writing it.
    pub author: String,
    /// The extension's title.
    pub title: String,
    /// The extension's description.
    pub description: String,
    /// Where to create it; `~` is expanded before the check.
    pub location: String,
    /// The first command's title.
    pub command_title: String,
    /// The first command's description.
    pub command_description: String,
    /// The chosen boilerplate template's id.
    pub template_id: String,
}

/// The errors, one per field; `None` where the field is fine.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Errors {
    /// The author field's error.
    pub author: Option<&'static str>,
    /// The title field's error.
    pub title: Option<&'static str>,
    /// The description field's error.
    pub description: Option<&'static str>,
    /// The location field's error.
    pub location: Option<&'static str>,
    /// The command title field's error.
    pub command_title: Option<&'static str>,
    /// The command description field's error.
    pub command_description: Option<&'static str>,
}

impl Errors {
    /// Whether anything failed.
    #[must_use]
    pub fn any(&self) -> bool {
        [
            self.author,
            self.title,
            self.description,
            self.location,
            self.command_title,
            self.command_description,
        ]
        .iter()
        .any(Option::is_some)
    }
}

/// `expandPath`: `~` and `~/...` only.
///
/// A bare `~` becomes the home directory and `~/x` becomes `$HOME/x`. Anything
/// else — including `~user` — is left alone, so a path like `~root/thing` is
/// checked literally and will not exist.
#[must_use]
pub fn expand_path(path: &str, home: &str) -> String {
    if path == "~" {
        return home.to_owned();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("{home}/{rest}");
    }
    path.to_owned()
}

/// Validates `form`.
///
/// `location_is_directory` answers `fs::is_directory(expandPath(location))` —
/// the check is that the directory *exists*, not that it is empty or writable,
/// so a generation failure afterwards is still possible and is reported
/// separately.
///
/// The C++ measures `QString::size()`, which counts UTF-16 code units. This
/// counts characters, which differ for anything outside the BMP: an author
/// called "🥳🥳🥳" passes here and passes there too (three characters, six
/// code units), while one called "🥳" fails both (one character, two units)
/// only because both are under three. The difference is reachable in principle
/// and has never mattered in practice; counting characters is the behaviour a
/// user expects from "min. 3 chars".
#[must_use]
pub fn validate(form: &Form, location_is_directory: bool) -> Errors {
    Errors {
        author: (form.author.chars().count() < MIN_SHORT).then_some(TOO_SHORT),
        title: (form.title.chars().count() < MIN_SHORT).then_some(TOO_SHORT),
        description: (form.description.chars().count() < MIN_DESCRIPTION)
            .then_some(DESCRIPTION_TOO_SHORT),
        location: (!location_is_directory).then_some(LOCATION_MISSING),
        command_title: (form.command_title.chars().count() < MIN_SHORT).then_some(TOO_SHORT),
        command_description: (form.command_description.chars().count() < MIN_SHORT)
            .then_some(TOO_SHORT),
    }
}

/// What submitting does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// The form is not valid; show the errors and one toast.
    Rejected {
        /// Which fields failed.
        errors: Errors,
        /// The toast's text.
        toast: &'static str,
    },
    /// Generate the boilerplate. What follows depends on whether it worked.
    Generate {
        /// Where to write it, with `~` expanded.
        target: String,
    },
}

/// Decides what a submission does, given the form and whether its location
/// exists.
#[must_use]
pub fn submit(form: &Form, home: &str, location_is_directory: bool) -> Submission {
    let errors = validate(form, location_is_directory);

    if errors.any() {
        return Submission::Rejected {
            errors,
            toast: FORM_HAS_ERRORS,
        };
    }

    Submission::Generate {
        target: expand_path(&form.location, home),
    }
}

/// What a successful generation does to the navigation stack.
///
/// `popSelf()` then `pushView(successView)`: the form is replaced rather than
/// stacked on, so escaping from the success view goes back to where the form
/// was opened from and not to a filled-in form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuccessNavigation {
    /// The view to leave.
    pub pop_self: bool,
    /// The navigation title.
    pub title: &'static str,
    /// The navigation icon.
    pub emoji: &'static str,
}

/// The navigation change after a successful generation.
#[must_use]
pub const fn success_navigation() -> SuccessNavigation {
    SuccessNavigation {
        pop_self: true,
        title: SUCCESS_TITLE,
        emoji: SUCCESS_EMOJI,
    }
}
