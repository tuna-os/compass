// Web Search
//
// Type a query, pick where to search it. Copy this directory and edit ENGINES
// to add your own.
//
// Shows: the smallest useful script — a constant table, `url::encode`, and
// `open` actions (which is why the manifest asks for `application.open`).

const ENGINES = [
    #{ id: "ddg", name: "DuckDuckGo", url: "https://duckduckgo.com/?q=", icon: "globe" },
    #{ id: "wikipedia", name: "Wikipedia", url: "https://en.wikipedia.org/w/index.php?search=", icon: "book" },
    #{ id: "github", name: "GitHub", url: "https://github.com/search?q=", icon: "code" },
    #{ id: "crates", name: "crates.io", url: "https://crates.io/search?q=", icon: "package" },
];

fn search(query) {
    let q = query;
    q.trim();
    let items = [];
    for engine in global::ENGINES {
        let link = engine.url + url::encode(q);
        items.push(#{
            id: engine.id,
            title: if q == "" { engine.name } else { `Search ${engine.name} for “${q}”` },
            subtitle: if q == "" { "Type a query first" } else { link },
            icon: engine.icon,
            actions: if q == "" { [] } else {
                [
                    #{ title: `Open in Browser`, open: link },
                    #{ title: "Copy Link", copy: link, shortcut: "ctrl+shift+c" },
                ]
            },
        });
    }
    #{ items: items, placeholder: "Search the web…" }
}
