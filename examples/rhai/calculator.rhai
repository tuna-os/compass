// calculator.rhai — arithmetic via Rhai's pure stdlib (no I/O, sandboxed)
// Capabilities: [] — needs no host functions

fn metadata() {
    #{ title: "Calculator", icon: "accessories-calculator", mode: "list", capabilities: [] }
}

fn search(query) {
    if query == "" {
        #{ title: "Calculator", subtitle: "Type an expression, e.g. 2 * (3 + 4)" }
    } else {
        // Rhai can eval arithmetic; errors surface as Compile in the host
        let result = eval(query);
        #{ title: query + " = " + result.to_string(), subtitle: "Calculator • Rhai" }
    }
}
