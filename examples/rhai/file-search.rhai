// file-search.rhai — mock file list (no filesystem access)
// Real filesystem search would need a `fs` capability; this stub keeps the
// example runnable under the sandbox (capabilities: []). Replace the array
// with host-provided data when the fs capability lands.

const FILES = [
    #{ title: "README.md", subtitle: "/home/user/compass/README.md" },
    #{ title: "PLAN.md", subtitle: "/home/user/compass/docs/rust-engine/PLAN.md" },
    #{ title: "hello-world.rhai", subtitle: "examples/rhai/hello-world.rhai" },
];

fn metadata() {
    #{ title: "File Search (mock)", icon: "folder", mode: "list", capabilities: [] }
}

fn search(query) {
    if query == "" { return FILES; }
    let q = query.to_lower();
    let out = [];
    for f in FILES {
        if f.title.to_lower().contains(q) || f.subtitle.to_lower().contains(q) {
            out += f;
        }
    }
    out
}
