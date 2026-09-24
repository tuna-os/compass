// emoji-search.rhai — filterable list (stateful, sandboxed, no runtime)
// Capabilities: [] — static data, no host I/O

const EMOJIS = [
    #{ title: "Grinning", subtitle: "😀 grinning", keywords: "grin smile" },
    #{ title: "Rocket", subtitle: "🚀 rocket", keywords: "launch ship" },
    #{ title: "Compass", subtitle: "🧭 compass", keywords: "navigation map" },
    #{ title: "Crab", subtitle: "🦀 crab", keywords: "rust" },
];

fn metadata() {
    #{ title: "Emoji Search", icon: "face-smile", mode: "list", capabilities: [] }
}

fn search(query) {
    if query == "" {
        EMOJIS
    } else {
        let q = query.to_lower();
        let out = [];
        for item in EMOJIS {
            if item.title.to_lower().contains(q) || item.keywords.contains(q) || item.subtitle.contains(q) {
                out += item;
            }
        }
        out
    }
}
