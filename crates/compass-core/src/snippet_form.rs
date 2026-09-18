//! The snippet form: what it demands of a snippet before saving it.
//!
//! A port of `SnippetFormViewHost::submit`
//! (`src/server/src/builtins/snippet/snippet-form-view-host.cpp`) and
//! `snippet::Expansion::validateKeyword`
//! (`src/server/src/services/snippet/snippet-db.hpp`).
//!
//! [`crate::snippet`] expands snippets once they exist; this is the gate they
//! pass through first.

/// The shortest acceptable name.
pub const MIN_NAME: usize = 2;
/// The longest acceptable expansion keyword.
pub const MAX_KEYWORD: usize = 32;

/// The error on a name shorter than [`MIN_NAME`].
pub const NAME_TOO_SHORT: &str = "2 chars min.";
/// The error on empty content.
pub const CONTENT_EMPTY: &str = "Content should not be empty";
/// The error on content with more than one `{cursor}`.
pub const TOO_MANY_CURSORS: &str = "Only one {cursor} placeholder is allowed";
/// The error on an empty keyword, which `validateKeyword` rejects even though
/// the form never calls it with one.
pub const KEYWORD_EMPTY: &str = "Keyword cannot be empty";
/// The error on an over-long keyword.
pub const KEYWORD_TOO_LONG: &str = "Keyword exceeds maximum length of 32";
/// The error on a keyword with a space or a non-ASCII character.
pub const KEYWORD_NOT_PRINTABLE: &str =
    "Keyword must only contain printable ASCII characters (no spaces)";

/// The toast when the form does not validate.
pub const VALIDATION_FAILED: &str = "Validation failed";
/// The toast on a successful edit.
pub const UPDATED: &str = "Snippet updated";
/// The toast on a successful create. Note it is not symmetrical with
/// [`UPDATED`]: the C++ says "successfully created" here and plain "updated"
/// there.
pub const CREATED: &str = "Snippet successfully created";

/// `Expansion::validateKeyword`, which answers with the message or nothing.
///
/// The character rule is `uc <= 127 && isprint(uc) && !isspace(uc)`, so the
/// keyword must be printable ASCII with no spaces — an accented letter is
/// rejected, and so is a tab.
#[must_use]
pub fn validate_keyword(keyword: &str) -> Option<&'static str> {
    if keyword.is_empty() {
        return Some(KEYWORD_EMPTY);
    }
    // `keyword.size()` counts bytes, and so does this: a keyword of non-ASCII
    // characters is rejected by the next rule anyway, so the two agree.
    if keyword.len() > MAX_KEYWORD {
        return Some(KEYWORD_TOO_LONG);
    }
    if !keyword
        .bytes()
        .all(|byte| byte <= 127 && byte.is_ascii_graphic())
    {
        return Some(KEYWORD_NOT_PRINTABLE);
    }
    None
}

/// The form's errors.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Errors {
    /// The name field's error.
    pub name: Option<&'static str>,
    /// The content field's error.
    pub content: Option<&'static str>,
    /// The keyword field's error.
    pub keyword: Option<&'static str>,
}

impl Errors {
    /// Whether anything failed.
    #[must_use]
    pub fn any(&self) -> bool {
        self.name.is_some() || self.content.is_some() || self.keyword.is_some()
    }
}

/// Validates the form.
///
/// `cursor_placeholders` is how many `{cursor}` placeholders the content
/// holds — [`crate::shortcut::parse_link`] finds them — and is only consulted
/// when the content is non-empty, because the C++ checks it in the `else` of
/// the emptiness test.
///
/// The keyword is validated **only when it is not empty**: an expansion is
/// optional, and leaving the field blank means "no keyboard expansion" rather
/// than an error.
#[must_use]
pub fn validate(name: &str, content: &str, cursor_placeholders: usize, keyword: &str) -> Errors {
    Errors {
        name: (name.chars().count() < MIN_NAME).then_some(NAME_TOO_SHORT),
        content: if content.is_empty() {
            Some(CONTENT_EMPTY)
        } else {
            (cursor_placeholders > 1).then_some(TOO_MANY_CURSORS)
        },
        keyword: (!keyword.is_empty())
            .then(|| validate_keyword(keyword))
            .flatten(),
    }
}

/// What submitting does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submission {
    /// Show the errors and one toast.
    Rejected {
        /// Which fields failed.
        errors: Errors,
        /// The toast's text.
        toast: &'static str,
    },
    /// Save it, as a new snippet or over an old one.
    Save {
        /// The id to overwrite, or `None` to create.
        id: Option<String>,
        /// The snippet's name.
        name: String,
        /// Its text.
        content: String,
        /// Its expansion, when a keyword was given.
        expansion: Option<Expansion>,
        /// The toast on success.
        success: &'static str,
    },
}

/// The keyboard expansion attached to a snippet.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    /// The keyword that triggers it.
    pub keyword: String,
    /// Whether it only fires on a word boundary.
    pub word: bool,
    /// The window classes it is limited to; empty means everywhere.
    pub apps: Vec<String>,
}

/// Decides what a submission does.
///
/// A failed save reports the *store's* error text rather than one of this
/// module's: `toast->failure(result.error().c_str())`. That is why [`Submission::Save`]
/// carries only the success message.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn submit(
    editing: Option<&str>,
    name: &str,
    content: &str,
    cursor_placeholders: usize,
    keyword: &str,
    expand_as_word: bool,
    apps: &[String],
) -> Submission {
    let errors = validate(name, content, cursor_placeholders, keyword);
    if errors.any() {
        return Submission::Rejected {
            errors,
            toast: VALIDATION_FAILED,
        };
    }

    Submission::Save {
        id: editing.map(str::to_owned),
        name: name.to_owned(),
        content: content.to_owned(),
        expansion: (!keyword.is_empty()).then(|| Expansion {
            keyword: keyword.to_owned(),
            word: expand_as_word,
            apps: apps.to_vec(),
        }),
        success: if editing.is_some() { UPDATED } else { CREATED },
    }
}
