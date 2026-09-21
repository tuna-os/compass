// hello-world.rhai — minimal Rhai extension (copy-paste starter)
// Docs: see examples/rhai/README.md

fn metadata() {
    #{ title: "Hello World", icon: "star", mode: "list", capabilities: [] }
}

fn search(query) {
    if query == "" {
        #{ title: "Hello, Compass!", subtitle: "Type to filter — this is a Rhai script, no Node required" }
    } else {
        #{ title: "You typed: " + query, subtitle: "Hello World • Rhai tier" }
    }
}
