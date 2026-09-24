//! `vicinae dmenu`: a list from stdin, shown in the running launcher.
//!
//! As in the C++ (`IpcService::dmenu`), the list is shown by the resident
//! launcher rather than a window of its own, and the command waits for the
//! choice. The engine keeps the list under a token, pushes
//! `WindowCommand::Dmenu(token)` to the window, which fetches the list and
//! answers the choice; the waiting request then returns it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use compass_ipc::DmenuSpec;
use tokio::sync::oneshot;

/// The C++ threshold under which a narrow list drops quick look and the
/// footer: `DMENU_SMALL_WIDTH_THRESHOLD`.
pub const SMALL_WIDTH_THRESHOLD: u32 = 500;

/// The request `vicinae dmenu` sends for `args` and the text read from
/// stdin.
#[must_use]
pub fn spec(args: crate::cli::DmenuArgs, content: String) -> DmenuSpec {
    let narrow = args
        .width
        .is_some_and(|width| width < SMALL_WIDTH_THRESHOLD);
    DmenuSpec {
        content,
        navigation_title: args.navigation_title,
        section_title: args.section_title,
        output_index: args.format.eq_ignore_ascii_case("index"),
        placeholder: args.placeholder,
        query: args.query,
        width: args.width,
        height: args.height,
        no_section: args.no_section,
        no_quick_look: args.no_quick_look || narrow,
        no_metadata: args.no_metadata,
        no_footer: args.no_footer || narrow,
    }
}

/// The lists waiting on the launcher, by token.
#[derive(Debug, Default)]
pub struct Pending {
    next: AtomicU64,
    lists: Mutex<HashMap<u64, (DmenuSpec, oneshot::Sender<String>)>>,
}

impl Pending {
    /// Keeps `spec` under a new token, and returns it with the choice to
    /// wait for. A list still waiting is dismissed first: the launcher shows
    /// one at a time, as the C++ replaces its view.
    pub fn open(&self, spec: DmenuSpec) -> (u64, oneshot::Receiver<String>) {
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        if let Ok(mut lists) = self.lists.lock() {
            for (_, (_, waiting)) in lists.drain() {
                let _ = waiting.send(String::new());
            }
            lists.insert(token, (spec, sender));
        }
        (token, receiver)
    }

    /// The list behind `token`, if it is still waiting.
    #[must_use]
    pub fn spec(&self, token: u64) -> Option<DmenuSpec> {
        self.lists
            .lock()
            .ok()?
            .get(&token)
            .map(|(spec, _)| spec.clone())
    }

    /// Answers the list behind `token`: what to print, or nothing when it
    /// was dismissed. `false` when no list is waiting under that token.
    pub fn choose(&self, token: u64, output: Option<String>) -> bool {
        let Some((_, sender)) = self
            .lists
            .lock()
            .ok()
            .and_then(|mut lists| lists.remove(&token))
        else {
            return false;
        };
        let _ = sender.send(output.unwrap_or_default());
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> crate::cli::DmenuArgs {
        crate::cli::DmenuArgs {
            navigation_title: None,
            section_title: None,
            format: "INDEX".into(),
            placeholder: None,
            query: None,
            width: Some(400),
            height: None,
            no_section: false,
            no_quick_look: false,
            no_metadata: false,
            no_footer: false,
        }
    }

    #[test]
    fn a_narrow_list_drops_quick_look_and_the_footer() {
        let spec = spec(args(), "a\nb\n".into());
        assert!(spec.output_index);
        assert!(spec.no_quick_look && spec.no_footer);
        assert_eq!(spec.content, "a\nb\n");
    }

    #[tokio::test]
    async fn a_choice_reaches_the_waiting_list_and_a_new_list_dismisses_the_old() {
        let pending = Pending::default();
        let (first, first_choice) = pending.open(DmenuSpec::default());
        let (second, second_choice) = pending.open(DmenuSpec {
            content: "x".into(),
            ..DmenuSpec::default()
        });
        assert_eq!(first_choice.await.unwrap(), "", "replaced, so dismissed");
        assert!(pending.spec(first).is_none());
        assert_eq!(
            pending.spec(second).map(|s| s.content).as_deref(),
            Some("x")
        );
        assert!(pending.choose(second, Some("x".into())));
        assert_eq!(second_choice.await.unwrap(), "x");
        assert!(!pending.choose(second, None), "answered once");
    }
}
