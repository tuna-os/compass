//! The messages on the input server's stdin and stdout.
//!
//! The C++ generates both ends from `figura/snippet.fig` with figura's glaze
//! backend: JSON-RPC 2.0 shaped messages, one per [length-prefixed
//! frame](super::frame). Compass keeps that wire exactly, so either engine can
//! drive either helper — the C++ server spawning the Rust
//! `compass-input-server`, or the Rust engine spawning the C++ one — while
//! both ship side by side (PLAN §5).
//!
//! What figura puts on the wire, read off `src/lib/figura/src/codegen/glaze.hpp`:
//!
//! * a call is `{"jsonrpc":"2.0","method":"Snippet/<name>","id":N,"params":{…}}`,
//!   and `params` is an object keyed by the `.fig` **parameter names**
//!   (`info`, `req`, `delayUs`), not the bare argument;
//! * a reply is `{"id":N,"jsonrpc":"2.0","result":…}`, `null` for `void`;
//! * a failure is `{"jsonrpc":"2.0","method":"","id":N,"error":"…"}`;
//! * an event is `{"jsonrpc":"2.0","method":"Snippet/<event>","params":{"payload":{…}}}`;
//! * enums are their names (`"Keydown"`, `"Word"`), and an absent optional is
//!   left out, as glaze writes one.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The service name figura prefixes every method with.
pub const SERVICE: &str = "Snippet";

/// When a registered trigger fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExpansionMode {
    /// As soon as the trigger is typed.
    Keydown,
    /// When a word separator follows it.
    Word,
}

impl From<ExpansionMode> for crate::snippet::ExpansionMode {
    fn from(mode: ExpansionMode) -> Self {
        match mode {
            ExpansionMode::Keydown => Self::Keydown,
            ExpansionMode::Word => Self::Word,
        }
    }
}

/// An XKB layout, as `setKeymap` takes it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutInfo {
    /// The layout, e.g. `us` or `fr`.
    pub layout: String,
    /// The rules file; the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<String>,
    /// The keyboard model; the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// The layout variant; the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// XKB options; the default when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<String>,
}

/// `InjectExpandRequest`: erase the trigger, paste, and walk the cursor back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectExpand {
    /// Backspaces to send first.
    pub chars_to_delete: u32,
    /// How long to wait between the backspaces and the paste, in µs.
    pub pre_paste_delay_us: u32,
    /// Paste with Ctrl+Shift+V (a terminal) rather than Ctrl+V.
    pub terminal: bool,
    /// Left-arrow presses after the paste, to land on `{cursor}`.
    pub cursor_left_moves: u32,
}

/// `InjectUndoRequest`: erase an expansion and type its trigger back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InjectUndo {
    /// Backspaces to send.
    pub backspace_count: u32,
    /// What to type afterwards.
    pub trigger_text: String,
}

/// One call the engine makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    /// `setKeymap(info)`.
    SetKeymap(LayoutInfo),
    /// `createSnippet(req)`.
    CreateSnippet {
        /// The trigger.
        trigger: String,
        /// When it fires.
        mode: ExpansionMode,
    },
    /// `removeSnippet(req)`.
    RemoveSnippet {
        /// The trigger.
        trigger: String,
    },
    /// `resetContext()`.
    ResetContext,
    /// `injectExpand(req)`.
    InjectExpand(InjectExpand),
    /// `injectUndo(req)`.
    InjectUndo(InjectUndo),
    /// `injectPaste(req)`.
    InjectPaste {
        /// Ctrl+Shift+V rather than Ctrl+V.
        terminal: bool,
    },
    /// `setKeyDelay(delayUs)`.
    SetKeyDelay(i32),
    /// `getCapabilities()`.
    GetCapabilities,
}

impl Call {
    /// The method name after `Snippet/`.
    #[must_use]
    pub const fn method(&self) -> &'static str {
        match self {
            Self::SetKeymap(_) => "setKeymap",
            Self::CreateSnippet { .. } => "createSnippet",
            Self::RemoveSnippet { .. } => "removeSnippet",
            Self::ResetContext => "resetContext",
            Self::InjectExpand(_) => "injectExpand",
            Self::InjectUndo(_) => "injectUndo",
            Self::InjectPaste { .. } => "injectPaste",
            Self::SetKeyDelay(_) => "setKeyDelay",
            Self::GetCapabilities => "getCapabilities",
        }
    }

    /// The `params` object, keyed by the `.fig` parameter names.
    #[must_use]
    pub fn params(&self) -> Value {
        match self {
            Self::SetKeymap(info) => json!({ "info": info }),
            Self::CreateSnippet { trigger, mode } => {
                json!({ "req": { "trigger": trigger, "mode": mode } })
            }
            Self::RemoveSnippet { trigger } => json!({ "req": { "trigger": trigger } }),
            Self::ResetContext | Self::GetCapabilities => json!({}),
            Self::InjectExpand(req) => json!({ "req": req }),
            Self::InjectUndo(req) => json!({ "req": req }),
            Self::InjectPaste { terminal } => json!({ "req": { "terminal": terminal } }),
            Self::SetKeyDelay(delay) => json!({ "delayUs": delay }),
        }
    }

    /// The request as the engine writes it.
    #[must_use]
    pub fn encode(&self, id: i32) -> Vec<u8> {
        let message = json!({
            "jsonrpc": "2.0",
            "method": format!("{SERVICE}/{}", self.method()),
            "id": id,
            "params": self.params(),
        });
        message.to_string().into_bytes()
    }

    /// Reads a request, as the server's `route` does: the id, and the call if
    /// the method is one it knows.
    ///
    /// # Errors
    ///
    /// When the bytes are not a JSON-RPC request, the method is unknown, or
    /// its params do not have the shape the method takes. The id comes back
    /// with the error when it could be read, so the failure can be answered.
    pub fn decode(bytes: &[u8]) -> Result<(i32, Self), (Option<i32>, String)> {
        #[derive(Deserialize)]
        struct Request {
            method: String,
            #[serde(default)]
            id: i32,
            #[serde(default)]
            params: Value,
        }
        #[derive(Deserialize)]
        struct Req<T> {
            req: T,
        }
        #[derive(Deserialize)]
        struct Trigger {
            trigger: String,
        }
        #[derive(Deserialize)]
        struct Create {
            trigger: String,
            mode: ExpansionMode,
        }
        #[derive(Deserialize)]
        struct Paste {
            terminal: bool,
        }
        #[derive(Deserialize)]
        struct Info {
            info: LayoutInfo,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Delay {
            delay_us: i32,
        }

        let request: Request = serde_json::from_slice(bytes).map_err(|e| (None, e.to_string()))?;
        let id = request.id;
        let fail = |e: serde_json::Error| (Some(id), e.to_string());
        let params = request.params;
        let name = request
            .method
            .strip_prefix(SERVICE)
            .and_then(|rest| rest.strip_prefix('/'))
            .unwrap_or_default();

        let call = match name {
            "setKeymap" => {
                Self::SetKeymap(serde_json::from_value::<Info>(params).map_err(fail)?.info)
            }
            "createSnippet" => {
                let req = serde_json::from_value::<Req<Create>>(params)
                    .map_err(fail)?
                    .req;
                Self::CreateSnippet {
                    trigger: req.trigger,
                    mode: req.mode,
                }
            }
            "removeSnippet" => Self::RemoveSnippet {
                trigger: serde_json::from_value::<Req<Trigger>>(params)
                    .map_err(fail)?
                    .req
                    .trigger,
            },
            "resetContext" => Self::ResetContext,
            "injectExpand" => Self::InjectExpand(
                serde_json::from_value::<Req<InjectExpand>>(params)
                    .map_err(fail)?
                    .req,
            ),
            "injectUndo" => Self::InjectUndo(
                serde_json::from_value::<Req<InjectUndo>>(params)
                    .map_err(fail)?
                    .req,
            ),
            "injectPaste" => Self::InjectPaste {
                terminal: serde_json::from_value::<Req<Paste>>(params)
                    .map_err(fail)?
                    .req
                    .terminal,
            },
            "setKeyDelay" => Self::SetKeyDelay(
                serde_json::from_value::<Delay>(params)
                    .map_err(fail)?
                    .delay_us,
            ),
            "getCapabilities" => Self::GetCapabilities,
            _ => {
                return Err((Some(id), format!("unknown method {:?}", request.method)));
            }
        };
        Ok((id, call))
    }
}

/// What the server writes back for a call.
#[must_use]
pub fn reply(id: i32, result: &Value) -> Vec<u8> {
    json!({ "id": id, "jsonrpc": "2.0", "result": result })
        .to_string()
        .into_bytes()
}

/// What the server writes back for a call that failed.
#[must_use]
pub fn reply_error(id: i32, error: &str) -> Vec<u8> {
    json!({ "jsonrpc": "2.0", "method": "", "id": id, "error": error })
        .to_string()
        .into_bytes()
}

/// The result `createSnippet` answers with: `CreateSnippetResponse{}`, whose
/// `ok` the C++ never sets.
#[must_use]
pub fn create_snippet_result() -> Value {
    json!({ "ok": false })
}

/// The result `removeSnippet` answers with: `RemoveSnippetResponse{}`, whose
/// `removed` the C++ never sets either.
#[must_use]
pub fn remove_snippet_result() -> Value {
    json!({ "removed": false })
}

/// The result `getCapabilities` answers with.
#[must_use]
pub fn capabilities_result(injection: bool) -> Value {
    json!({ "injection": injection })
}

/// An event the server raises.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// `triggerSnippet`: this trigger was typed.
    Trigger(String),
    /// `undoSnippet`: Backspace straight after this trigger expanded.
    Undo(String),
}

impl Event {
    /// The event as the server writes it.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let (name, trigger) = match self {
            Self::Trigger(trigger) => ("triggerSnippet", trigger),
            Self::Undo(trigger) => ("undoSnippet", trigger),
        };
        json!({
            "jsonrpc": "2.0",
            "method": format!("{SERVICE}/{name}"),
            "params": { "payload": { "trigger": trigger } },
        })
        .to_string()
        .into_bytes()
    }
}

/// One message from the server, as the engine reads it.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerMessage {
    /// The answer to call `id`.
    Reply {
        /// The call's id.
        id: i32,
        /// Its result, or the error the server gave.
        result: Result<Value, String>,
    },
    /// An event.
    Event(Event),
    /// A notification this build does not know, kept for the log.
    Unknown(String),
}

impl ServerMessage {
    /// Reads one message, as `RpcTransport::dispatchMessage` does: an `id`
    /// makes it a reply, otherwise a `method` makes it an event.
    ///
    /// # Errors
    ///
    /// When the bytes are not a JSON object.
    pub fn decode(bytes: &[u8]) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct Incoming {
            id: Option<i32>,
            method: Option<String>,
            error: Option<String>,
            #[serde(default)]
            result: Value,
            #[serde(default)]
            params: Value,
        }
        let message: Incoming = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if let Some(id) = message.id {
            let result = match message.error {
                Some(error) => Err(error),
                None => Ok(message.result),
            };
            return Ok(Self::Reply { id, result });
        }
        let Some(method) = message.method else {
            return Ok(Self::Unknown(String::new()));
        };
        let trigger = || {
            message
                .params
                .get("payload")
                .and_then(|payload| payload.get("trigger"))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let event = match method
            .strip_prefix(SERVICE)
            .and_then(|m| m.strip_prefix('/'))
        {
            Some("triggerSnippet") => trigger().map(Event::Trigger),
            Some("undoSnippet") => trigger().map(Event::Undo),
            _ => None,
        };
        Ok(event.map_or(Self::Unknown(method), Self::Event))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_call_carries_its_fig_parameter_names() {
        let create = Call::CreateSnippet {
            trigger: ";sig".into(),
            mode: ExpansionMode::Word,
        };
        let value: Value = serde_json::from_slice(&create.encode(7)).unwrap();
        assert_eq!(
            value,
            json!({"jsonrpc":"2.0","method":"Snippet/createSnippet","id":7,
                   "params":{"req":{"trigger":";sig","mode":"Word"}}})
        );

        let delay: Value = serde_json::from_slice(&Call::SetKeyDelay(2000).encode(1)).unwrap();
        assert_eq!(delay["params"], json!({"delayUs": 2000}));

        let expand = Call::InjectExpand(InjectExpand {
            chars_to_delete: 5,
            pre_paste_delay_us: 0,
            terminal: true,
            cursor_left_moves: 2,
        });
        let value: Value = serde_json::from_slice(&expand.encode(2)).unwrap();
        assert_eq!(
            value["params"],
            json!({"req":{"charsToDelete":5,"prePasteDelayUs":0,"terminal":true,"cursorLeftMoves":2}})
        );

        let keymap = Call::SetKeymap(LayoutInfo {
            layout: "fr".into(),
            ..LayoutInfo::default()
        });
        let value: Value = serde_json::from_slice(&keymap.encode(3)).unwrap();
        assert_eq!(
            value["params"],
            json!({"info":{"layout":"fr"}}),
            "absent optionals are left out, as glaze writes them"
        );
    }

    #[test]
    fn every_call_survives_the_round_trip() {
        let calls = [
            Call::SetKeymap(LayoutInfo {
                layout: "de".into(),
                variant: Some("nodeadkeys".into()),
                ..LayoutInfo::default()
            }),
            Call::CreateSnippet {
                trigger: "x".into(),
                mode: ExpansionMode::Keydown,
            },
            Call::RemoveSnippet {
                trigger: "x".into(),
            },
            Call::ResetContext,
            Call::InjectExpand(InjectExpand::default()),
            Call::InjectUndo(InjectUndo {
                backspace_count: 3,
                trigger_text: ";a".into(),
            }),
            Call::InjectPaste { terminal: false },
            Call::SetKeyDelay(-1),
            Call::GetCapabilities,
        ];
        for (id, call) in calls.into_iter().enumerate() {
            let id = i32::try_from(id).unwrap();
            assert_eq!(Call::decode(&call.encode(id)), Ok((id, call)));
        }
    }

    #[test]
    fn a_glaze_request_decodes() {
        // Byte for byte what the generated C++ client writes: glaze emits
        // JsonRpcRequest's members in declaration order.
        let raw = br#"{"jsonrpc":"2.0","method":"Snippet/injectUndo","id":12,"params":{"req":{"backspaceCount":4,"triggerText":";sig"}}}"#;
        assert_eq!(
            Call::decode(raw),
            Ok((
                12,
                Call::InjectUndo(InjectUndo {
                    backspace_count: 4,
                    trigger_text: ";sig".into()
                })
            ))
        );
        let raw = br#"{"jsonrpc":"2.0","method":"Snippet/resetContext","id":1,"params":{}}"#;
        assert_eq!(Call::decode(raw), Ok((1, Call::ResetContext)));
    }

    #[test]
    fn an_unknown_method_is_refused_with_its_id() {
        let raw = br#"{"jsonrpc":"2.0","method":"Snippet/selfDestruct","id":9,"params":{}}"#;
        let (id, _) = Call::decode(raw).unwrap_err();
        assert_eq!(id, Some(9));
        assert_eq!(Call::decode(b"not json").unwrap_err().0, None);
    }

    #[test]
    fn replies_and_events_have_the_glaze_shape() {
        let parse = |bytes: Vec<u8>| serde_json::from_slice::<Value>(&bytes).unwrap();
        assert_eq!(
            parse(reply(4, &Value::Null)),
            json!({"id":4,"jsonrpc":"2.0","result":null})
        );
        assert_eq!(
            parse(Event::Trigger(";sig".into()).encode()),
            json!({"jsonrpc":"2.0","method":"Snippet/triggerSnippet","params":{"payload":{"trigger":";sig"}}})
        );
    }

    #[test]
    fn the_engine_reads_what_the_server_writes() {
        assert_eq!(
            ServerMessage::decode(&reply(4, &capabilities_result(true))),
            Ok(ServerMessage::Reply {
                id: 4,
                result: Ok(json!({"injection": true}))
            })
        );
        assert_eq!(
            ServerMessage::decode(&reply_error(5, "nope")),
            Ok(ServerMessage::Reply {
                id: 5,
                result: Err("nope".into())
            })
        );
        for event in [Event::Trigger("a".into()), Event::Undo("b".into())] {
            assert_eq!(
                ServerMessage::decode(&event.encode()),
                Ok(ServerMessage::Event(event))
            );
        }
        assert_eq!(
            ServerMessage::decode(br#"{"jsonrpc":"2.0","method":"Snippet/other","params":{}}"#),
            Ok(ServerMessage::Unknown("Snippet/other".into()))
        );
    }
}
