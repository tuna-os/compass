// Quick Notes
//
// Type to filter your notes; press Enter on "Add note" to save what you typed.
//
// Shows: persistent state with `storage::`, actions that run script
// functions (`run` plus `args`), a destructive action with a shortcut, and
// returning effects (`rerender`, `toast`) from an action.

fn notes() {
    storage::get("notes") ?? []
}

fn ago(seconds) {
    let d = time::now() - seconds;
    if d < 60 { "just now" }
    else if d < 3600 { `${d / 60}m ago` }
    else if d < 86400 { `${d / 3600}h ago` }
    else { `${d / 86400}d ago` }
}

// Action handlers: called with the `args` the action carried.
fn add(text) {
    let all = notes();
    all.insert(0, #{ id: random::uuid(), text: text, created: time::now() });
    storage::set("notes", all);
    #{ rerender: true, toast: "Note saved", style: "success" }
}

fn remove(id) {
    let all = notes().filter(|n| n.id != id);
    storage::set("notes", all);
    #{ rerender: true, toast: "Note deleted" }
}

fn search(query) {
    let q = query;
    q.trim();
    let needle = q.to_lower();

    let items = [];
    for note in notes() {
        if needle == "" || note.text.to_lower().contains(needle) {
            items.push(#{
                id: note.id,
                title: note.text,
                icon: "notes",
                accessory: ago(note.created),
                actions: [
                    #{ title: "Copy Note", copy: note.text },
                    #{ title: "Delete Note", run: "remove", args: note.id,
                       style: "destructive", shortcut: "ctrl+x" },
                ],
            });
        }
    }

    let sections = [];
    if q != "" {
        sections.push(#{
            items: [#{
                id: "add",
                title: `Add note: ${q}`,
                icon: "plus",
                actions: [#{ title: "Add Note", run: "add", args: q }],
            }],
        });
    }
    sections.push(#{ title: "Notes", items: items });

    #{
        sections: sections,
        placeholder: "Filter notes, or type a new one",
        empty: #{ title: "No notes yet", description: "Type something and press Enter to save it" },
    }
}
