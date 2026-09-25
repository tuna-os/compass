//! The developer extension: Create Extension.
//!
//! The form's rules are `compass_core::create_extension` and the files it
//! writes are `compass_core::boilerplate`; this runs them against the disk
//! and answers with the sentences the form shows.

use compass_core::create_extension::{self, Form, Submission};
use compass_ipc::{ErrorKind, ProtocolError, Response};

/// The version tag the generated `package.json` depends on the API at,
/// as the C++ passes `VICINAE_GIT_TAG`.
#[must_use]
pub fn git_tag() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

/// Validates `form` and, when it passes, writes the boilerplate: the
/// answer is where it went, or which fields failed (`Form has errors`), or
/// `Failed to create extension` with the generator's reason.
#[must_use]
pub fn create_extension(form: &Form) -> Response {
    let home = compass_core::xdg_dirs::home_dir()
        .map(|home| home.to_string_lossy().into_owned())
        .unwrap_or_default();
    let target = create_extension::expand_path(&form.location, &home);
    let is_directory = std::path::Path::new(&target).is_dir();
    match create_extension::submit(form, &home, is_directory) {
        Submission::Rejected { errors, toast } => {
            let fields = [
                ("author", errors.author),
                ("title", errors.title),
                ("description", errors.description),
                ("location", errors.location),
                ("command title", errors.command_title),
                ("command description", errors.command_description),
            ]
            .into_iter()
            .filter_map(|(field, error)| error.map(|error| format!("{field}: {error}")))
            .collect::<Vec<_>>()
            .join("; ");
            Response::Error(ProtocolError::new(
                ErrorKind::BadRequest,
                format!("{toast} ({fields})"),
            ))
        }
        Submission::Generate { target } => {
            let config = compass_core::boilerplate::Config {
                author: form.author.clone(),
                title: form.title.clone(),
                description: form.description.clone(),
                commands: vec![compass_core::boilerplate::CommandConfig {
                    title: form.command_title.clone(),
                    description: form.command_description.clone(),
                    template_id: form.template_id.clone(),
                }],
            };
            match compass_core::boilerplate::generate(
                std::path::Path::new(&target),
                &config,
                &git_tag(),
            ) {
                Ok(path) => Response::ExtensionCreated {
                    path: path.to_string_lossy().into_owned(),
                },
                Err(error) => {
                    tracing::error!(%error, "failed to create an extension");
                    Response::Error(ProtocolError::new(
                        ErrorKind::Internal,
                        format!("{}: {error}", create_extension::GENERATION_FAILED),
                    ))
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(location: &str) -> Form {
        Form {
            author: "zoe".into(),
            title: "Hello World".into(),
            description: "Says hello to the whole world".into(),
            location: location.into(),
            command_title: "Say Hello".into(),
            command_description: "Says hello".into(),
            template_id: ":boilerplate/tmpl-list".into(),
        }
    }

    #[test]
    fn a_valid_form_writes_the_boilerplate_and_an_invalid_one_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let Response::ExtensionCreated { path } =
            create_extension(&form(&dir.path().to_string_lossy()))
        else {
            panic!("not created");
        };
        let root = std::path::Path::new(&path);
        assert!(root.starts_with(dir.path()));
        let manifest = std::fs::read_to_string(root.join("package.json")).unwrap();
        assert!(
            manifest.contains("\"title\": \"Hello World\""),
            "{manifest}"
        );
        assert!(manifest.contains("Say Hello"));

        let Response::Error(err) = create_extension(&Form {
            author: "z".into(),
            ..form("/definitely/not/here")
        }) else {
            panic!("not refused");
        };
        assert_eq!(err.kind, ErrorKind::BadRequest);
        assert!(
            err.message.starts_with("Form has errors")
                && err.message.contains("author: Min. 3 chars")
                && err.message.contains("location: Must exist"),
            "{}",
            err.message
        );
    }
}
