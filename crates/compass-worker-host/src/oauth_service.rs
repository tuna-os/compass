//! The `OAuth` extension API.
//!
//! [`OAuthService`] ports the three methods of `ExtOAuthService`
//! (`src/server/src/extension/api/oauth-service.hpp`) that are the token
//! store. `OAuth/authorize` is [`AuthorizeService`]: it opens a browser and
//! waits for a person, so it defers, behind an [`Authorizer`] the engine
//! supplies.
//!
//! # The wire shape, read off the client
//!
//! `src/typescript/api/src/api/oauth.ts` does
//! `const { set } = await getClient().OAuth.getTokens(...)` and then
//! `if (!set) return undefined`, so an extension with no stored tokens needs
//! `set` to be falsy — this answers `{}`, leaving the key out. It also does
//! `new Date(set.updatedAt * 1000)`, which is what fixes `updatedAt` as
//! **seconds**, and reads `accessToken`, `refreshToken`, `idToken`, `scope` and
//! `expiresIn` by those camelCase names, which are `figura/tsapi.fig`'s.
//!
//! The columns are named differently — `access_token` and friends — and
//! [`compass_oauth_store`] speaks the column names. The translation is here, in
//! one place, and `the_field_names_are_the_idls` pins it against the IDL.

use compass_oauth_store::{TokenSet, TokenStore};

use crate::tsapi::{self, Call};

/// The methods [`OAuthService`] serves; `OAuth/authorize` is
/// [`AuthorizeService`]'s, in [`DEFERRED_METHODS`].
pub const METHODS: &[&str] = &["OAuth/getTokens", "OAuth/setTokens", "OAuth/removeTokens"];

/// Serves the token store for one extension.
pub struct OAuthService<'a> {
    store: TokenStore<'a>,
    extension_id: String,
    now: Box<dyn Fn() -> i64 + 'a>,
}

impl std::fmt::Debug for OAuthService<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OAuthService")
            .field("extension_id", &self.extension_id)
            .finish_non_exhaustive()
    }
}

impl<'a> OAuthService<'a> {
    /// Serves `store` for `extension_id`, reading the wall clock for
    /// `updated_at`.
    #[must_use]
    pub fn new(store: TokenStore<'a>, extension_id: impl Into<String>) -> Self {
        Self::with_clock(store, extension_id, || {
            i64::try_from(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            )
            .unwrap_or(i64::MAX)
        })
    }

    /// As [`new`](Self::new), with the clock supplied.
    #[must_use]
    pub fn with_clock(
        store: TokenStore<'a>,
        extension_id: impl Into<String>,
        now: impl Fn() -> i64 + 'a,
    ) -> Self {
        Self {
            store,
            extension_id: extension_id.into(),
            now: Box::new(now),
        }
    }

    /// Answers `call`, or `None` if it is not one of ours.
    #[must_use]
    pub fn handle(&self, call: &Call) -> Option<String> {
        let id = call.id?;
        if !METHODS.contains(&call.method.as_str()) {
            return None;
        }

        Some(match self.answer(call) {
            Ok(result) => tsapi::reply(id, result),
            Err(error) => tsapi::reply_error(id, &error.to_string()),
        })
    }

    fn answer(&self, call: &Call) -> Result<serde_json::Value, compass_oauth_store::Error> {
        // `getTokens(id?)` and `removeTokens(id?)` send `{ id: undefined }`
        // when the extension has no provider id, and `JSON.stringify` drops
        // the key; either way it reads as absent.
        let provider = call.params.get("id").and_then(serde_json::Value::as_str);

        match call.method.as_str() {
            "OAuth/getTokens" => {
                let set = self.store.get(&self.extension_id, provider)?;
                Ok(match set {
                    // `{}`, not `{"set": null}`: the client tests `if (!set)`,
                    // so both work, and this is the smaller promise.
                    None => serde_json::json!({}),
                    Some(set) => serde_json::json!({ "set": to_wire(&set) }),
                })
            }
            "OAuth/setTokens" => {
                let payload = call
                    .params
                    .get("payload")
                    .unwrap_or(&serde_json::Value::Null);
                let set = from_wire(&self.extension_id, payload);
                self.store.set(&set, (self.now)())?;
                Ok(serde_json::Value::Null)
            }
            "OAuth/removeTokens" => {
                self.store.remove(&self.extension_id, provider)?;
                Ok(serde_json::Value::Null)
            }
            other => Ok(serde_json::Value::String(format!(
                "{other} is not an OAuth method"
            ))),
        }
    }
}

/// A stored set as `TokenSet` on the wire.
fn to_wire(set: &TokenSet) -> serde_json::Value {
    let mut out = serde_json::Map::new();
    out.insert(
        "accessToken".to_owned(),
        serde_json::Value::String(set.access_token.clone()),
    );
    out.insert("updatedAt".to_owned(), set.updated_at.into());
    for (name, value) in [
        ("refreshToken", set.refresh_token.as_deref()),
        ("idToken", set.id_token.as_deref()),
        ("scope", set.scope.as_deref()),
    ] {
        if let Some(value) = value {
            out.insert(name.to_owned(), serde_json::Value::String(value.to_owned()));
        }
    }
    if let Some(expires_in) = set.expires_in {
        out.insert("expiresIn".to_owned(), expires_in.into());
    }
    serde_json::Value::Object(out)
}

/// A `SetTokensRequest` as a stored set.
///
/// A missing `accessToken` becomes the empty string rather than a refusal,
/// which is what the C++ does: `payload.accessToken` is a `std::string` that
/// glaze leaves default-constructed when the key is absent.
fn from_wire(extension_id: &str, payload: &serde_json::Value) -> TokenSet {
    let text = |name: &str| {
        payload
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    };

    TokenSet {
        extension_id: extension_id.to_owned(),
        provider_id: text("providerId"),
        access_token: text("accessToken").unwrap_or_default(),
        refresh_token: text("refreshToken"),
        id_token: text("idToken"),
        scope: text("scope"),
        expires_in: payload.get("expiresIn").and_then(serde_json::Value::as_i64),
        // Ignored by the store, which stamps its own.
        updated_at: 0,
    }
}

impl tsapi::Service for OAuthService<'_> {
    fn handle(&self, call: &Call) -> Option<String> {
        Self::handle(self, call)
    }
}

/// The method that answers when a person has finished with a browser.
pub const DEFERRED_METHODS: &[&str] = &["OAuth/authorize"];

/// An `OAuth/authorize` call, read off `AuthorizeRequest` in the IDL.
///
/// The extension builds the whole authorization URL itself — PKCE verifier,
/// challenge, redirect URI and all (`PKCEClient.authorizationRequest` in
/// `src/typescript/api/src/api/oauth.ts`) — and exchanges the code for tokens
/// itself too. The host's part is the middle: open the URL, wait for the
/// provider to redirect back to `raycast://oauth?code=…&state=…`, and answer
/// with the code. The `state` is what ties the redirect to this call, as
/// `OAuthService::authorize(state)` keys it in the C++.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeRequest {
    /// The provider's authorization URL, to open in the browser.
    pub url: String,
    /// The URL's `state` parameter, or `None` when it has none — which the
    /// host cannot wait on, since nothing would tie a redirect to it.
    pub state: Option<String>,
    /// The provider's name, e.g. "GitHub".
    pub provider: String,
    /// What connecting does, in the provider's words.
    pub description: String,
}

impl AuthorizeRequest {
    /// Reads the call's `payload`; `None` when it has no URL.
    #[must_use]
    pub fn from_call(call: &Call) -> Option<Self> {
        let payload = call.params.get("payload")?;
        let url = payload.get("url")?.as_str()?.to_owned();
        let client = payload.get("client");
        let text = |key: &str| {
            client
                .and_then(|client| client.get(key))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        Some(Self {
            state: query_value(&url, "state"),
            provider: text("name"),
            description: text("description"),
            url,
        })
    }
}

/// The value of `key` in `url`'s query, decoded.
#[must_use]
pub fn query_value(url: &str, key: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

/// What a provider's redirect back to the launcher said.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Redirect {
    /// The person approved: the code to exchange, for the request `state`.
    Code {
        /// Which request.
        state: String,
        /// The authorization code.
        code: String,
    },
    /// The provider refused, or the person declined (RFC 6749 §4.1.2.1).
    Refused {
        /// Which request.
        state: String,
        /// `error_description`, else `error`.
        reason: String,
    },
}

impl Redirect {
    /// Reads an `oauth` deeplink: `raycast://oauth?…`,
    /// `com.raycast:/oauth?…`, `compass://oauth?…` or `vicinae://oauth?…`.
    ///
    /// # Errors
    ///
    /// A sentence: not an OAuth redirect, or one without a `state`, or one
    /// with neither a code nor an error.
    pub fn parse(raw: &str) -> Result<Self, String> {
        let url = url::Url::parse(raw).map_err(|err| format!("{raw} is not a URL: {err}"))?;
        let is_oauth = url.host_str() == Some("oauth") || url.path().trim_matches('/') == "oauth";
        if !matches!(
            url.scheme(),
            "raycast" | "com.raycast" | "compass" | "vicinae"
        ) || !is_oauth
        {
            return Err(format!("{raw} is not an OAuth redirect"));
        }
        let value = |key: &str| {
            url.query_pairs()
                .find(|(name, _)| name == key)
                .map(|(_, value)| value.into_owned())
                .filter(|value| !value.is_empty())
        };
        let state = value("state").ok_or("the OAuth redirect has no state")?;
        if let Some(code) = value("code") {
            return Ok(Self::Code { state, code });
        }
        match value("error_description").or_else(|| value("error")) {
            Some(reason) => Ok(Self::Refused { state, reason }),
            None => Err("the OAuth redirect has neither a code nor an error".to_owned()),
        }
    }

    /// The request it answers.
    #[must_use]
    pub fn state(&self) -> &str {
        match self {
            Self::Code { state, .. } | Self::Refused { state, .. } => state,
        }
    }
}

/// Whatever shows the person where to go and waits for the redirect.
pub trait Authorizer {
    /// Opens `request` and holds `deferral` until the redirect answers it,
    /// or the request is abandoned; either way the deferral must be settled.
    fn authorize(&self, request: AuthorizeRequest, deferral: &tsapi::Deferral);
}

/// Serves `OAuth/authorize`: the call defers, and the [`Authorizer`] owns
/// the answer.
#[derive(Debug)]
pub struct AuthorizeService<A> {
    authorizer: A,
}

impl<A: Authorizer> AuthorizeService<A> {
    /// Serves authorize calls through `authorizer`.
    pub const fn new(authorizer: A) -> Self {
        Self { authorizer }
    }

    /// The authorizer.
    pub const fn authorizer(&self) -> &A {
        &self.authorizer
    }
}

impl<A: Authorizer> tsapi::Service for AuthorizeService<A> {
    fn handle(&self, call: &Call) -> Option<String> {
        if !DEFERRED_METHODS.contains(&call.method.as_str()) {
            return None;
        }
        let id = call.id?;
        // Refused now rather than deferred: an authorize with no URL, or one
        // with no `state` to match a redirect to, would wait for ever.
        match AuthorizeRequest::from_call(call) {
            None => Some(tsapi::reply_error(id, "OAuth/authorize needs a URL")),
            Some(request) if request.state.is_none() => Some(tsapi::reply_error(
                id,
                "The authorization URL has no state parameter, so Compass cannot match the \
                 provider's redirect to it",
            )),
            Some(_) => None,
        }
    }

    fn defer(&self, call: &Call) -> Option<tsapi::Deferral> {
        if !DEFERRED_METHODS.contains(&call.method.as_str()) {
            return None;
        }
        let request = AuthorizeRequest::from_call(call)?;
        let deferral = tsapi::Deferral::for_call(call)?;
        self.authorizer.authorize(request, &deferral);
        Some(deferral)
    }
}

#[cfg(test)]
mod authorize_tests {
    use super::*;
    use tsapi::Service as _;

    fn call(payload: serde_json::Value) -> Call {
        serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "id": 4, "method": "OAuth/authorize",
            "params": {"payload": payload},
        }))
        .expect("a call")
    }

    #[derive(Default)]
    struct Recorded(std::sync::Mutex<Vec<(AuthorizeRequest, u64)>>);

    impl Authorizer for &Recorded {
        fn authorize(&self, request: AuthorizeRequest, deferral: &tsapi::Deferral) {
            self.0.lock().unwrap().push((request, deferral.id));
        }
    }

    #[test]
    fn an_authorize_call_defers_with_the_provider_and_the_state_of_its_url() {
        let recorded = Recorded::default();
        let service = AuthorizeService::new(&recorded);
        let call = call(serde_json::json!({
            "client": {"name": "GitHub", "description": "Connect your account"},
            "url": "https://github.com/login/oauth/authorize?client_id=x&state=eyJmbGF2b3Ii%3D&code_challenge=c",
        }));
        assert_eq!(service.handle(&call), None);
        let deferral = service.defer(&call).expect("deferred");
        assert_eq!(deferral.id, 4);
        let seen = recorded.0.lock().unwrap();
        let (request, id) = &seen[0];
        assert_eq!(*id, 4);
        assert_eq!(request.provider, "GitHub");
        assert_eq!(request.description, "Connect your account");
        assert_eq!(
            request.state.as_deref(),
            Some("eyJmbGF2b3Ii="),
            "decoded, as the redirect will carry it"
        );
    }

    #[test]
    fn a_url_without_a_state_is_refused_rather_than_waited_on() {
        let recorded = Recorded::default();
        let service = AuthorizeService::new(&recorded);
        let call = call(serde_json::json!({
            "client": {"name": "X", "description": ""},
            "url": "https://example.com/authorize?client_id=x",
        }));
        let answer = service.handle(&call).expect("answered now");
        assert!(answer.contains("no state parameter"), "{answer}");
        assert!(recorded.0.lock().unwrap().is_empty());
    }

    #[test]
    fn every_redirect_shape_raycast_uses_parses() {
        for raw in [
            "raycast://oauth?package_name=Extension&code=abc&state=s1",
            "com.raycast:/oauth?package_name=Extension&code=abc&state=s1",
            "vicinae://oauth?code=abc&state=s1",
        ] {
            assert_eq!(
                Redirect::parse(raw),
                Ok(Redirect::Code {
                    state: "s1".into(),
                    code: "abc".into()
                }),
                "{raw}"
            );
        }
        assert_eq!(
            Redirect::parse("raycast://oauth?state=s1&error=access_denied"),
            Ok(Redirect::Refused {
                state: "s1".into(),
                reason: "access_denied".into()
            })
        );
        assert!(Redirect::parse("raycast://extensions/x/y").is_err());
        assert!(Redirect::parse("raycast://oauth?code=abc").is_err());
        assert!(Redirect::parse("https://oauth?code=abc&state=s").is_err());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use compass_sqlcipher_sys::rusqlite::Connection;
    use std::path::Path;

    const FIG: &str = "figura/tsapi.fig";

    fn read_fig() -> String {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("two levels below the repository root")
            .join(FIG);
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
    }

    /// The field names a `struct` declares in the IDL.
    fn fields_of(name: &str) -> Vec<String> {
        let fig = read_fig();
        let body = fig
            .split(&format!("struct {name} {{"))
            .nth(1)
            .unwrap_or_else(|| panic!("{FIG} no longer declares struct {name}"))
            .split('}')
            .next()
            .expect("the block is closed");
        body.lines()
            .map(|l| l.split("//").next().unwrap_or("").trim())
            .filter(|l| !l.is_empty())
            .filter_map(|l| l.split(':').next())
            .map(|f| f.trim().trim_end_matches('?').to_owned())
            .filter(|f| !f.is_empty())
            .collect()
    }

    fn open() -> (tempfile::TempDir, Connection) {
        let dir = tempfile::tempdir().expect("a temporary directory");
        let db = compass_sqlcipher_sys::open(&dir.path().join("vicinae.db"), &[])
            .expect("an unencrypted db");
        compass_db::vicinae::run(&db).expect("the migrations apply");
        (dir, db)
    }

    fn call(method: &str, params: serde_json::Value) -> Call {
        Call {
            jsonrpc: crate::rpc::VERSION.to_owned(),
            id: Some(1),
            method: method.to_owned(),
            params,
        }
    }

    fn result_of(service: &OAuthService<'_>, call: &Call) -> serde_json::Value {
        let payload = service.handle(call).expect("an OAuth call is answered");
        let answer: serde_json::Value = serde_json::from_str(&payload).expect("a JSON reply");
        assert!(
            answer.get("error").is_none(),
            "unexpected error: {}",
            answer["error"]
        );
        answer["result"].clone()
    }

    fn service<'a>(db: &'a Connection) -> OAuthService<'a> {
        OAuthService::with_clock(TokenStore::new(db), "hn", || 1_700_000_000)
    }

    #[test]
    fn the_methods_are_on_the_ledger_and_the_store_does_not_take_authorize() {
        for method in METHODS.iter().chain(DEFERRED_METHODS) {
            assert!(
                tsapi::is_implemented(method),
                "{method} is served here but is not on the tsapi ledger"
            );
        }
        // The store answers now; authorize waits on a person, so it is the
        // authorizer's, and a store that claimed it would answer before the
        // browser had even opened.
        let (_dir, db) = open();
        let call: Call = serde_json::from_value(serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "OAuth/authorize", "params": {}
        }))
        .unwrap();
        assert_eq!(service(&db).handle(&call), None);
    }

    #[test]
    fn a_token_set_survives_a_set_then_a_get() {
        let (_dir, db) = open();
        let service = service(&db);

        assert_eq!(
            result_of(
                &service,
                &call(
                    "OAuth/setTokens",
                    serde_json::json!({ "payload": {
                        "accessToken": "a",
                        "refreshToken": "r",
                        "scope": "read",
                        "expiresIn": 3600,
                    }}),
                )
            ),
            serde_json::Value::Null,
            "a void method answers null"
        );

        let result = result_of(&service, &call("OAuth/getTokens", serde_json::json!({})));
        assert_eq!(
            result,
            serde_json::json!({ "set": {
                "accessToken": "a",
                "refreshToken": "r",
                "scope": "read",
                "expiresIn": 3600,
                "updatedAt": 1_700_000_000i64,
            }}),
            "the reply must use the IDL's camelCase names and the store's own timestamp"
        );
    }

    #[test]
    fn nothing_stored_answers_an_object_with_no_set() {
        // `const { set } = ...; if (!set) return undefined;` -- the key is
        // left out rather than set to null, and either would work; what would
        // not is answering the TokenSet directly, or an error.
        let (_dir, db) = open();
        let result = result_of(
            &service(&db),
            &call("OAuth/getTokens", serde_json::json!({})),
        );
        assert_eq!(result, serde_json::json!({}));
        assert!(result["set"].is_null(), "`set` must be falsy: {result}");
    }

    #[test]
    fn the_field_names_are_the_idls() {
        // Every field of TokenSet is either sent or explained. The wire names
        // are camelCase and the column names are snake_case, and this is the
        // one place they meet.
        let declared = fields_of("TokenSet");
        assert_eq!(
            declared.len(),
            6,
            "parsed {declared:?} from {FIG}, which is not the shape this test expects"
        );

        let wire = to_wire(&TokenSet {
            extension_id: "hn".to_owned(),
            provider_id: Some("github".to_owned()),
            access_token: "a".to_owned(),
            refresh_token: Some("r".to_owned()),
            id_token: Some("i".to_owned()),
            scope: Some("s".to_owned()),
            expires_in: Some(1),
            updated_at: 2,
        });
        let mut keys: Vec<String> = wire
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        let mut expected = declared;
        keys.sort();
        expected.sort();
        assert_eq!(
            keys, expected,
            "the fields this sends are not the fields {FIG} declares"
        );

        // And the request side, which the extension fills in.
        let request = fields_of("SetTokensRequest");
        let parsed = from_wire(
            "hn",
            &serde_json::json!({
                "providerId": "github",
                "accessToken": "a",
                "refreshToken": "r",
                "idToken": "i",
                "scope": "s",
                "expiresIn": 1,
            }),
        );
        for field in &request {
            let read = match field.as_str() {
                "providerId" => parsed.provider_id.is_some(),
                "accessToken" => !parsed.access_token.is_empty(),
                "refreshToken" => parsed.refresh_token.is_some(),
                "idToken" => parsed.id_token.is_some(),
                "scope" => parsed.scope.is_some(),
                "expiresIn" => parsed.expires_in.is_some(),
                other => panic!("{FIG} declares SetTokensRequest.{other}, which is not read here"),
            };
            assert!(read, "SetTokensRequest.{field} was not read");
        }
    }

    #[test]
    fn a_provider_id_selects_the_row() {
        let (_dir, db) = open();
        let service = service(&db);

        service
            .handle(&call(
                "OAuth/setTokens",
                serde_json::json!({ "payload": { "accessToken": "unnamed" }}),
            ))
            .expect("set");
        service
            .handle(&call(
                "OAuth/setTokens",
                serde_json::json!({ "payload": {
                    "providerId": "github", "accessToken": "named",
                }}),
            ))
            .expect("set");

        assert_eq!(
            result_of(&service, &call("OAuth/getTokens", serde_json::json!({})))["set"]["accessToken"],
            "unnamed"
        );
        assert_eq!(
            result_of(
                &service,
                &call("OAuth/getTokens", serde_json::json!({ "id": "github" }))
            )["set"]["accessToken"],
            "named"
        );
    }

    #[test]
    fn removing_takes_effect_and_only_for_the_named_provider() {
        let (_dir, db) = open();
        let service = service(&db);

        for provider in [None, Some("github")] {
            let mut payload = serde_json::json!({ "accessToken": "a" });
            if let Some(provider) = provider {
                payload["providerId"] = serde_json::json!(provider);
            }
            service
                .handle(&call(
                    "OAuth/setTokens",
                    serde_json::json!({ "payload": payload }),
                ))
                .expect("set");
        }

        result_of(
            &service,
            &call("OAuth/removeTokens", serde_json::json!({ "id": "github" })),
        );
        assert_eq!(
            result_of(
                &service,
                &call("OAuth/getTokens", serde_json::json!({ "id": "github" }))
            ),
            serde_json::json!({})
        );
        assert!(
            !result_of(&service, &call("OAuth/getTokens", serde_json::json!({})))["set"].is_null(),
            "removing one provider removed another"
        );
    }

    #[test]
    fn one_extension_cannot_read_anothers_tokens() {
        // `m_extensionId` is fixed when the service is constructed and never
        // comes from the payload, so an extension cannot ask for someone
        // else's row. This is the check that keeps it that way.
        let (_dir, db) = open();
        service(&db)
            .handle(&call(
                "OAuth/setTokens",
                serde_json::json!({ "payload": { "accessToken": "secret" }}),
            ))
            .expect("set");

        let other = OAuthService::with_clock(TokenStore::new(&db), "someone-else", || 0);
        assert_eq!(
            result_of(&other, &call("OAuth/getTokens", serde_json::json!({}))),
            serde_json::json!({})
        );

        // Not even by naming one in the payload: `extension_id` is not a
        // parameter of any of these methods.
        let smuggled = result_of(
            &other,
            &call(
                "OAuth/getTokens",
                serde_json::json!({ "extension_id": "hn", "extensionId": "hn" }),
            ),
        );
        assert_eq!(smuggled, serde_json::json!({}));
    }

    #[test]
    fn authorize_is_declined_rather_than_answered() {
        let (_dir, db) = open();
        assert_eq!(
            service(&db).handle(&call("OAuth/authorize", serde_json::json!({}))),
            None
        );
    }

    #[test]
    fn a_call_for_another_service_and_an_event_are_both_declined() {
        let (_dir, db) = open();
        let service = service(&db);
        assert_eq!(
            service.handle(&call("Storage/get", serde_json::json!({ "key": "k" }))),
            None
        );

        let mut event = call("OAuth/removeTokens", serde_json::json!({}));
        event.id = None;
        assert_eq!(service.handle(&event), None);
    }
}
