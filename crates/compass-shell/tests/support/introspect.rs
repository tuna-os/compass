//! Structural comparison of two D-Bus introspection documents.
//!
//! WHY THIS EXISTS
//!
//! `crates/compass-shell/dbus/*.xml` is described in `contract.rs` as "the
//! canonical definition ... so that the extension and the engine can be
//! reviewed against one another", and the proxies are described as mirroring
//! it "exactly". Nothing enforced either claim.
//!
//! What existed was a substring test that asserted the XML `contains`
//! `<method name="ActivateWindow">`, against a list of member names typed into
//! the same test file. That check cannot fail for the reason it was written:
//!
//!   * it never touches the proxies, so if `ActivateWindow` were renamed in
//!     `proxy.rs` and in the mock, the XML and the test's own hardcoded list
//!     would still agree with each other and the test would still pass;
//!   * it never looks at a signature, so `ActivateWindow(u)` could become
//!     `ActivateWindow(s)` on the wire and the XML would still "contain"
//!     `<method name="ActivateWindow">`.
//!
//! So the XML was decorative. This module makes it load-bearing by parsing it
//! and comparing it, member by member and argument by argument, against the
//! introspection the running object server actually emits.
//!
//! WHAT THE COMPARISON CAN AND CANNOT SEE
//!
//! `zbus` does not emit **names for out arguments** — measured, not assumed:
//! the live document for `ListWindows` is `<arg type="aa{sv}"
//! direction="out"/>` while the checked-in XML names it `windows`. Argument
//! names are therefore compared only where both documents supply one, which
//! keeps in-argument names honest (they come from the Rust parameter names)
//! and lets the checked-in XML document out-argument names for human readers.
//!
//! Types, directions, argument counts, argument order, method and signal sets,
//! and property types and access modes are all compared strictly.

use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One argument of a method or signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Arg {
    /// `name` attribute, or `None` when the document omits it.
    pub name: Option<String>,
    /// D-Bus type signature.
    pub ty: String,
    /// `"in"` or `"out"`. Signal arguments are always `"out"` by convention;
    /// the D-Bus specification defaults a method argument with no `direction`
    /// to `"in"`.
    pub direction: String,
}

/// A property: its type and access mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// D-Bus type signature.
    pub ty: String,
    /// `read`, `write` or `readwrite`.
    pub access: String,
}

/// One `<interface>` element, reduced to what a client can observe.
#[derive(Debug, Clone, Default)]
pub struct Interface {
    /// Methods by name, arguments in declaration order.
    pub methods: BTreeMap<String, Vec<Arg>>,
    /// Signals by name, arguments in declaration order.
    pub signals: BTreeMap<String, Vec<Arg>>,
    /// Properties by name.
    pub properties: BTreeMap<String, Property>,
}

/// Parses `interface` out of an introspection document.
///
/// Returns `None` when the document has no such interface, which is a real
/// outcome the caller must distinguish from "present but different" — an
/// interface that vanished and an interface that changed are different bugs.
///
/// # Panics
///
/// Panics when `xml` is not well-formed. A malformed document in either
/// position is a defect in this repository, not a condition to report.
pub fn parse(xml: &str, interface: &str) -> Option<Interface> {
    // `roxmltree` rejects documents with a DTD by default, as billion-laughs
    // protection, and every D-Bus introspection document opens with the
    // standard `<!DOCTYPE node PUBLIC ...>`. The reference is external with no
    // internal subset, and both documents compared here are produced inside
    // this repository, so there is nothing to expand and nothing untrusted.
    let options = roxmltree::ParsingOptions {
        allow_dtd: true,
        ..roxmltree::ParsingOptions::default()
    };
    let doc = roxmltree::Document::parse_with_options(xml, options).unwrap_or_else(|err| {
        panic!("the introspection document said to declare {interface} is not well-formed: {err}")
    });

    let node = doc
        .descendants()
        .find(|n| n.has_tag_name("interface") && n.attribute("name") == Some(interface))?;

    let mut out = Interface::default();

    for child in node.children().filter(|c| c.is_element()) {
        let name = child.attribute("name").unwrap_or_default().to_owned();
        match child.tag_name().name() {
            "method" => {
                out.methods.insert(name, args(child, "in"));
            }
            "signal" => {
                out.signals.insert(name, args(child, "out"));
            }
            "property" => {
                out.properties.insert(
                    name,
                    Property {
                        ty: child.attribute("type").unwrap_or_default().to_owned(),
                        access: child.attribute("access").unwrap_or_default().to_owned(),
                    },
                );
            }
            // `annotation` and anything else a future document carries is not
            // part of the wire contract a client depends on.
            _ => {}
        }
    }

    Some(out)
}

/// Collects `<arg>` children, defaulting the direction the way D-Bus does.
fn args(node: roxmltree::Node<'_, '_>, default_direction: &str) -> Vec<Arg> {
    node.children()
        .filter(|c| c.is_element() && c.has_tag_name("arg"))
        .map(|c| Arg {
            name: c.attribute("name").map(str::to_owned),
            ty: c.attribute("type").unwrap_or_default().to_owned(),
            direction: c
                .attribute("direction")
                .unwrap_or(default_direction)
                .to_owned(),
        })
        .collect()
}

/// Every way in which `live` differs from `declared`, one human-readable line
/// each. Empty means the two agree.
///
/// `declared` is the checked-in XML and `live` is what the object server
/// emitted, so the messages are phrased from the point of view of someone who
/// changed the code and now has to update the document.
pub fn differences(declared: &Interface, live: &Interface) -> Vec<String> {
    let mut out = Vec::new();

    compare_members("method", &declared.methods, &live.methods, &mut out);
    compare_members("signal", &declared.signals, &live.signals, &mut out);

    for (name, declared_prop) in &declared.properties {
        match live.properties.get(name) {
            None => out.push(format!(
                "property {name}: declared in the XML, absent from the served interface"
            )),
            Some(live_prop) if live_prop != declared_prop => out.push(format!(
                "property {name}: XML says type {:?} access {:?}, served interface says type {:?} access {:?}",
                declared_prop.ty, declared_prop.access, live_prop.ty, live_prop.access
            )),
            Some(_) => {}
        }
    }
    for name in live.properties.keys() {
        if !declared.properties.contains_key(name) {
            out.push(format!(
                "property {name}: served by the interface, missing from the XML"
            ));
        }
    }

    out
}

fn compare_members(
    kind: &str,
    declared: &BTreeMap<String, Vec<Arg>>,
    live: &BTreeMap<String, Vec<Arg>>,
    out: &mut Vec<String>,
) {
    for (name, declared_args) in declared {
        let Some(live_args) = live.get(name) else {
            out.push(format!(
                "{kind} {name}: declared in the XML, absent from the served interface"
            ));
            continue;
        };
        if declared_args.len() != live_args.len() {
            out.push(format!(
                "{kind} {name}: XML declares {} argument(s), the served interface has {}",
                declared_args.len(),
                live_args.len()
            ));
            continue;
        }
        for (index, (want, got)) in declared_args.iter().zip(live_args).enumerate() {
            let mut problem = String::new();
            if want.ty != got.ty {
                let _ = write!(problem, " type {:?} vs {:?};", want.ty, got.ty);
            }
            if want.direction != got.direction {
                let _ = write!(
                    problem,
                    " direction {:?} vs {:?};",
                    want.direction, got.direction
                );
            }
            // Only where both documents name the argument: see the module
            // comment on out-argument names.
            if let (Some(want_name), Some(got_name)) = (&want.name, &got.name)
                && want_name != got_name
            {
                let _ = write!(problem, " name {want_name:?} vs {got_name:?};");
            }
            if !problem.is_empty() {
                out.push(format!(
                    "{kind} {name} argument {index}: XML vs served interface —{problem}"
                ));
            }
        }
    }
    for name in live.keys() {
        if !declared.contains_key(name) {
            out.push(format!(
                "{kind} {name}: served by the interface, missing from the XML"
            ));
        }
    }
}
