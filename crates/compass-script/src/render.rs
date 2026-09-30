//! What a script returns, turned into `compass-extension-api`'s [`View`].
//!
//! Scripts return plain Rhai values — arrays and object maps — in a shape
//! designed to be written by hand (see `docs/rust-engine/RHAI-SCRIPTS.md`).
//! This module is the only place that shape is known. It produces the same
//! [`View`] the TypeScript tier produces, so the launcher has one thing to
//! draw, and it mints the [`HandlerId`] tokens actions carry, remembering what
//! each one means in a [`Handler`] table the instance keeps until the next
//! render.
//!
//! Anything malformed is an [`ScriptError::InvalidView`] naming the path to
//! the bad value; nothing is silently dropped, because a row that quietly
//! vanishes is harder to debug than an error that says `items[3].title`.

use compass_extension_api::{
    Accessory, Action, ActionPanel, ActionStyle, Capability, Color, Detail, EmptyState, HandlerId,
    Image, KeyModifier, ListItem, ListSection, ListView, MetadataItem, Shortcut, View,
};
use rhai::{AST, Dynamic, FnPtr, Map};

use crate::engine::Grants;
use crate::error::ScriptError;

/// What an action token stands for.
#[derive(Debug, Clone)]
pub(crate) enum Handler {
    /// Call a script function by name.
    Named { name: String, args: Option<Dynamic> },
    /// Call a function pointer or closure.
    Pointer {
        pointer: FnPtr,
        args: Option<Dynamic>,
    },
    /// Copy text (`clipboard.write`).
    Copy(String),
    /// Paste text (`clipboard.paste`).
    Paste(String),
    /// Open a URL or path (`application.open`).
    Open(String),
}

/// State threaded through one render.
pub(crate) struct Render<'a> {
    pub(crate) render: u64,
    pub(crate) ast: &'a AST,
    pub(crate) grants: &'a Grants,
    pub(crate) handlers: Vec<(HandlerId, Handler)>,
}

type Result<T> = std::result::Result<T, ScriptError>;

fn invalid(path: &str, message: impl Into<String>) -> ScriptError {
    ScriptError::InvalidView {
        path: if path.is_empty() {
            "the returned value".to_owned()
        } else {
            path.to_owned()
        },
        message: message.into(),
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

/// Text from a string, a number or a character. Anything else is an error:
/// printing a map as a row title is never what the author meant.
fn text(value: &Dynamic, path: &str) -> Result<String> {
    if value.is_string() || value.is_int() || value.is_float() || value.is_char() || value.is_bool()
    {
        Ok(value.to_string())
    } else {
        Err(invalid(
            path,
            format!("expected text, found {}", value.type_name()),
        ))
    }
}

fn field_text(map: &Map, key: &str, path: &str) -> Result<Option<String>> {
    match map.get(key) {
        None => Ok(None),
        Some(value) if value.is_unit() => Ok(None),
        Some(value) => text(value, &join(path, key)).map(|t| Some(t).filter(|t| !t.is_empty())),
    }
}

fn field_bool(map: &Map, key: &str, path: &str) -> Result<Option<bool>> {
    match map.get(key) {
        None => Ok(None),
        Some(value) if value.is_unit() => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .map_err(|found| invalid(&join(path, key), format!("expected a bool, found {found}"))),
    }
}

fn field_array(map: &Map, key: &str, path: &str) -> Result<Option<rhai::Array>> {
    match map.get(key) {
        None => Ok(None),
        Some(value) if value.is_unit() => Ok(None),
        Some(value) => value.clone().into_array().map(Some).map_err(|found| {
            invalid(
                &join(path, key),
                format!("expected an array, found {found}"),
            )
        }),
    }
}

fn as_map(value: &Dynamic, path: &str) -> Result<Map> {
    value.clone().try_cast::<Map>().ok_or_else(|| {
        invalid(
            path,
            format!("expected an object map, found {}", value.type_name()),
        )
    })
}

fn check_keys(map: &Map, allowed: &[&str], path: &str) -> Result<()> {
    match map.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(invalid(
            &join(path, key),
            format!("unknown field; expected one of: {}", allowed.join(", ")),
        )),
        None => Ok(()),
    }
}

impl Render<'_> {
    /// The root: an array of items, or a map describing a list or a detail.
    pub(crate) fn view(&mut self, value: &Dynamic) -> Result<View> {
        if value.is_array() {
            let items = self.items(value.clone().into_array().unwrap_or_default(), "items")?;
            let show_detail = items.iter().any(|item| item.detail.is_some());
            return Ok(View::List(ListView {
                show_detail,
                sections: vec![ListSection::untitled(items)],
                ..ListView::default()
            }));
        }
        let map = as_map(value, "")?;
        if map.contains_key("markdown") || map.contains_key("metadata") {
            check_keys(
                &map,
                &["markdown", "metadata", "title", "actions", "loading"],
                "",
            )?;
            return Ok(View::Detail(self.detail_map(&map, "")?));
        }
        self.list(&map)
    }

    fn list(&mut self, map: &Map) -> Result<View> {
        check_keys(
            map,
            &[
                "items",
                "sections",
                "placeholder",
                "title",
                "empty",
                "filtering",
                "show_detail",
                "loading",
                "actions",
            ],
            "",
        )?;
        let mut sections = Vec::new();
        if let Some(items) = field_array(map, "items", "")? {
            sections.push(ListSection::untitled(self.items(items, "items")?));
        }
        if let Some(raw) = field_array(map, "sections", "")? {
            for (index, section) in raw.iter().enumerate() {
                let path = format!("sections[{index}]");
                let section = as_map(section, &path)?;
                check_keys(&section, &["title", "subtitle", "id", "items"], &path)?;
                let items = field_array(&section, "items", &path)?.unwrap_or_default();
                sections.push(ListSection {
                    key: field_text(&section, "id", &path)?,
                    title: field_text(&section, "title", &path)?,
                    subtitle: field_text(&section, "subtitle", &path)?,
                    items: self.items(items, &join(&path, "items"))?,
                    ..ListSection::default()
                });
            }
        }

        let any_detail = sections
            .iter()
            .flat_map(|s| &s.items)
            .any(|item| item.detail.is_some());
        let mut view = ListView {
            navigation_title: field_text(map, "title", "")?,
            is_loading: field_bool(map, "loading", "")?.unwrap_or(false),
            show_detail: field_bool(map, "show_detail", "")?.unwrap_or(any_detail),
            sections,
            actions: self.panel(map, "")?,
            ..ListView::default()
        };
        view.search.placeholder = field_text(map, "placeholder", "")?;
        view.search.host_filtering = field_bool(map, "filtering", "")?.unwrap_or(false);
        if let Some(empty) = map.get("empty").filter(|e| !e.is_unit()) {
            view.empty_state = Some(self.empty(empty)?);
        }
        Ok(View::List(view))
    }

    fn empty(&mut self, value: &Dynamic) -> Result<EmptyState> {
        if value.is_string() {
            return Ok(EmptyState {
                title: value.to_string(),
                ..EmptyState::default()
            });
        }
        let map = as_map(value, "empty")?;
        check_keys(&map, &["title", "description", "icon", "actions"], "empty")?;
        Ok(EmptyState {
            title: field_text(&map, "title", "empty")?.unwrap_or_default(),
            description: field_text(&map, "description", "empty")?,
            icon: field_text(&map, "icon", "empty")?.map(Image::builtin),
            actions: self.panel(&map, "empty")?,
            ..EmptyState::default()
        })
    }

    fn items(&mut self, raw: rhai::Array, path: &str) -> Result<Vec<ListItem>> {
        raw.iter()
            .enumerate()
            .map(|(index, item)| self.item(item, &format!("{path}[{index}]")))
            .collect()
    }

    fn item(&mut self, value: &Dynamic, path: &str) -> Result<ListItem> {
        let map = as_map(value, path)?;
        check_keys(
            &map,
            &[
                "id",
                "title",
                "subtitle",
                "icon",
                "keywords",
                "accessory",
                "accessories",
                "detail",
                "actions",
            ],
            path,
        )?;
        let title = field_text(&map, "title", path)?
            .ok_or_else(|| invalid(&join(path, "title"), "every item needs a title"))?;

        let keywords = field_array(&map, "keywords", path)?
            .unwrap_or_default()
            .iter()
            .enumerate()
            .map(|(i, k)| text(k, &format!("{}[{i}]", join(path, "keywords"))))
            .collect::<Result<Vec<_>>>()?;

        let mut accessories = Vec::new();
        if let Some(single) = map.get("accessory").filter(|a| !a.is_unit()) {
            accessories.push(accessory(single, &join(path, "accessory"))?);
        }
        for (i, raw) in field_array(&map, "accessories", path)?
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            accessories.push(accessory(
                raw,
                &format!("{}[{i}]", join(path, "accessories")),
            )?);
        }

        let detail = match map.get("detail").filter(|d| !d.is_unit()) {
            None => None,
            Some(d) if d.is_string() => Some(Detail {
                markdown: Some(d.to_string()),
                ..Detail::default()
            }),
            Some(d) => {
                let detail_path = join(path, "detail");
                let detail_map = as_map(d, &detail_path)?;
                check_keys(&detail_map, &["markdown", "metadata"], &detail_path)?;
                Some(self.detail_map(&detail_map, &detail_path)?)
            }
        };

        Ok(ListItem {
            key: field_text(&map, "id", path)?,
            title,
            subtitle: field_text(&map, "subtitle", path)?,
            icon: field_text(&map, "icon", path)?.map(Image::builtin),
            accessories,
            keywords,
            detail,
            actions: self.panel(&map, path)?,
            ..ListItem::default()
        })
    }

    fn detail_map(&mut self, map: &Map, path: &str) -> Result<Detail> {
        let mut metadata = Vec::new();
        for (i, row) in field_array(map, "metadata", path)?
            .unwrap_or_default()
            .iter()
            .enumerate()
        {
            let row_path = format!("{}[{i}]", join(path, "metadata"));
            if row.is_unit() {
                metadata.push(MetadataItem::Separator);
                continue;
            }
            let row = as_map(row, &row_path)?;
            check_keys(&row, &["title", "text"], &row_path)?;
            metadata.push(MetadataItem::Label {
                title: field_text(&row, "title", &row_path)?.unwrap_or_default(),
                text: field_text(&row, "text", &row_path)?.unwrap_or_default(),
                icon: None,
                color: None,
            });
        }
        Ok(Detail {
            markdown: field_text(map, "markdown", path)?,
            metadata,
            is_loading: field_bool(map, "loading", path)?.unwrap_or(false),
            navigation_title: field_text(map, "title", path)?,
            actions: self.panel(map, path)?,
            ..Detail::default()
        })
    }

    fn panel(&mut self, map: &Map, path: &str) -> Result<Option<ActionPanel>> {
        let Some(raw) = field_array(map, "actions", path)? else {
            return Ok(None);
        };
        if raw.is_empty() {
            return Ok(None);
        }
        let actions_path = join(path, "actions");
        let actions = raw
            .iter()
            .enumerate()
            .map(|(i, a)| self.action(a, &format!("{actions_path}[{i}]")))
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(ActionPanel::of(actions)))
    }

    fn action(&mut self, value: &Dynamic, path: &str) -> Result<Action> {
        let map = as_map(value, path)?;
        check_keys(
            &map,
            &[
                "title", "id", "icon", "shortcut", "style", "run", "args", "copy", "paste", "open",
            ],
            path,
        )?;
        let title = field_text(&map, "title", path)?
            .ok_or_else(|| invalid(&join(path, "title"), "every action needs a title"))?;

        let kinds: Vec<&str> = ["run", "copy", "paste", "open"]
            .into_iter()
            .filter(|k| map.get(*k).is_some_and(|v| !v.is_unit()))
            .collect();
        let [kind] = kinds.as_slice() else {
            return Err(invalid(
                path,
                "an action needs exactly one of `run`, `copy`, `paste` or `open`",
            ));
        };
        let args = map.get("args").filter(|a| !a.is_unit()).cloned();
        if args.is_some() && *kind != "run" {
            return Err(invalid(&join(path, "args"), "`args` only applies to `run`"));
        }

        let handler = match *kind {
            "run" => self.run_handler(&map["run"], args, &join(path, "run"))?,
            "copy" => {
                self.require(&Capability::CLIPBOARD_WRITE)?;
                Handler::Copy(text(&map["copy"], &join(path, "copy"))?)
            }
            "paste" => {
                self.require(&Capability::CLIPBOARD_PASTE)?;
                Handler::Paste(text(&map["paste"], &join(path, "paste"))?)
            }
            _ => {
                self.require(&Capability::APPLICATION_OPEN)?;
                Handler::Open(text(&map["open"], &join(path, "open"))?)
            }
        };

        let token = HandlerId::new(format!("r{}.a{}", self.render, self.handlers.len()));
        self.handlers.push((token.clone(), handler));

        let style = match field_text(&map, "style", path)?.as_deref() {
            None | Some("regular") => ActionStyle::Regular,
            Some("destructive") => ActionStyle::Destructive,
            Some(other) => {
                return Err(invalid(
                    &join(path, "style"),
                    format!("unknown style `{other}`; expected `regular` or `destructive`"),
                ));
            }
        };
        let shortcut = field_text(&map, "shortcut", path)?
            .map(|s| shortcut(&s, &join(path, "shortcut")))
            .transpose()?;

        Ok(Action {
            key: field_text(&map, "id", path)?,
            icon: field_text(&map, "icon", path)?.map(Image::builtin),
            shortcut,
            style,
            ..Action::new(title, token.0)
        })
    }

    fn run_handler(&self, run: &Dynamic, args: Option<Dynamic>, path: &str) -> Result<Handler> {
        if let Some(pointer) = run.clone().try_cast::<FnPtr>() {
            return Ok(Handler::Pointer { pointer, args });
        }
        let name = text(run, path)?;
        let arity = usize::from(args.is_some());
        let defined = self
            .ast
            .iter_functions()
            .any(|f| f.name == name && f.params.len() == arity);
        if !defined {
            let signature = if arity == 1 { "(args)" } else { "()" };
            return Err(invalid(
                path,
                format!("the script defines no `fn {name}{signature}`"),
            ));
        }
        Ok(Handler::Named { name, args })
    }

    fn require(&self, cap: &Capability) -> Result<()> {
        self.grants
            .get(cap)
            .map(drop)
            .map_err(ScriptError::CapabilityDenied)
    }
}

fn accessory(value: &Dynamic, path: &str) -> Result<Accessory> {
    if !value.is_map() {
        return Ok(Accessory {
            text: Some(text(value, path)?),
            ..Accessory::default()
        });
    }
    let map = as_map(value, path)?;
    check_keys(&map, &["text", "tag", "color", "icon", "tooltip"], path)?;
    Ok(Accessory {
        text: field_text(&map, "text", path)?,
        tag: field_text(&map, "tag", path)?,
        color: field_text(&map, "color", path)?.map(Color::Named),
        icon: field_text(&map, "icon", path)?.map(Image::builtin),
        tooltip: field_text(&map, "tooltip", path)?,
    })
}

/// `"ctrl+shift+c"` → a [`Shortcut`]. The last segment is the key.
fn shortcut(spec: &str, path: &str) -> Result<Shortcut> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let Some((key, modifiers)) = parts.split_last() else {
        return Err(invalid(path, "empty shortcut"));
    };
    if key.is_empty() {
        return Err(invalid(path, format!("shortcut `{spec}` has no key")));
    }
    let modifiers = modifiers
        .iter()
        .map(|m| match m.to_ascii_lowercase().as_str() {
            // As the TypeScript tier reads them: `cmd` is Control off macOS.
            "ctrl" | "control" | "cmd" => Ok(KeyModifier::Ctrl),
            "alt" | "opt" | "option" => Ok(KeyModifier::Alt),
            "shift" => Ok(KeyModifier::Shift),
            "meta" | "super" | "windows" => Ok(KeyModifier::Meta),
            other => Err(invalid(path, format!("unknown modifier `{other}`"))),
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Shortcut::new(modifiers, key.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_parse_modifiers_and_reject_nonsense() {
        let s = shortcut("Ctrl+Shift+C", "p").unwrap();
        assert_eq!(
            s,
            Shortcut::new([KeyModifier::Ctrl, KeyModifier::Shift], "c")
        );
        assert_eq!(shortcut("return", "p").unwrap(), Shortcut::bare("return"));
        assert!(shortcut("hyper+x", "p").is_err());
        assert!(shortcut("ctrl+", "p").is_err());
    }
}
