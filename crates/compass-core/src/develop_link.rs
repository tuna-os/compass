//! The deeplinks `vici develop` sends while an extension is being developed.
//!
//! The SDK's CLI (`src/typescript/api/src/commands/develop`) builds the
//! extension into the extension directory and then sends
//! `compass://api/extensions/develop/start?id=<name>`, `…/refresh?id=` after
//! every rebuild and `…/stop?id=` when it exits. Upstream Vicinae answers them
//! with a development session. Compass answers them by rescanning the
//! extension directories, which is what makes a new build appear and an
//! edited manifest take effect; a command opened after a rebuild runs the new
//! bundle, since every run starts a fresh worker. An answer of anything but
//! success stops the CLI before it starts watching, so a link this does not
//! recognise is reported rather than ignored.

/// What the CLI asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DevelopAction {
    /// The first build is in place.
    Start,
    /// A rebuild is in place.
    Refresh,
    /// The CLI is exiting.
    Stop,
}

/// A parsed `api/extensions/develop` deeplink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DevelopLink {
    /// What the CLI asks for.
    pub action: DevelopAction,
    /// The extension's `name`, which is also its directory's name.
    pub id: String,
}

/// `url` as a develop deeplink: `None` when it is not one, `Some(Err)` with a
/// usage message when it is one that cannot be acted on.
#[must_use]
pub fn parse_develop_link(url: &str) -> Option<Result<DevelopLink, &'static str>> {
    let parsed = url::Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "compass" | "vicinae") || parsed.host_str() != Some("api") {
        return None;
    }
    let action = parsed.path().strip_prefix("/extensions/develop/")?;
    let action = match action.trim_end_matches('/') {
        "start" => DevelopAction::Start,
        "refresh" => DevelopAction::Refresh,
        "stop" => DevelopAction::Stop,
        _ => return Some(Err("the develop action is one of start, refresh and stop")),
    };
    let id = parsed
        .query_pairs()
        .find(|(key, _)| key == "id")
        .map(|(_, value)| value.into_owned())
        .filter(|id| !id.is_empty());
    let Some(id) = id else {
        return Some(Err(
            "a develop deeplink needs the extension's id: ?id=<name>",
        ));
    };
    Some(Ok(DevelopLink { action, id }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_links_the_cli_sends_are_recognised() {
        for (url, action) in [
            (
                "compass://api/extensions/develop/start?id=hello",
                DevelopAction::Start,
            ),
            (
                "compass://api/extensions/develop/refresh?id=hello",
                DevelopAction::Refresh,
            ),
            (
                "vicinae://api/extensions/develop/stop?id=hello",
                DevelopAction::Stop,
            ),
        ] {
            assert_eq!(
                parse_develop_link(url),
                Some(Ok(DevelopLink {
                    action,
                    id: "hello".to_owned()
                })),
                "{url}"
            );
        }
    }

    #[test]
    fn a_develop_link_without_an_id_or_with_an_unknown_action_says_why() {
        assert!(matches!(
            parse_develop_link("compass://api/extensions/develop/start"),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_develop_link("compass://api/extensions/develop/start?id="),
            Some(Err(_))
        ));
        assert!(matches!(
            parse_develop_link("compass://api/extensions/develop/rebuild?id=hello"),
            Some(Err(_))
        ));
    }

    #[test]
    fn other_links_are_not_develop_links() {
        for url in [
            "compass://extensions/raycast/github/search",
            "compass://launch/clipboard",
            "raycast://api/extensions/develop/start?id=hello",
            "https://api/extensions/develop/start?id=hello",
            "not a url",
        ] {
            assert_eq!(parse_develop_link(url), None, "{url}");
        }
    }
}
