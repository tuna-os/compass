// Generate
//
//   (empty)        -> one of each
//   uuid 5         -> five UUIDs
//   lorem 3        -> three paragraphs of lorem ipsum
//   password 32    -> a 32-character password
//
// Shows: `random::` helpers, sub-commands parsed from the query, `paste`
// alongside `copy`, and a detail pane for long text.

const WORDS = [
    "lorem", "ipsum", "dolor", "sit", "amet", "consectetur", "adipiscing", "elit", "sed", "do",
    "eiusmod", "tempor", "incididunt", "ut", "labore", "et", "dolore", "magna", "aliqua", "enim",
    "ad", "minim", "veniam", "quis", "nostrud", "exercitation", "ullamco", "laboris", "nisi",
    "aliquip", "ex", "ea", "commodo", "consequat", "duis", "aute", "irure", "in", "reprehenderit",
    "voluptate", "velit", "esse", "cillum", "fugiat", "nulla", "pariatur", "excepteur", "sint",
    "occaecat", "cupidatat", "non", "proident", "sunt", "culpa", "qui", "officia", "deserunt",
    "mollit", "anim", "id", "est", "laborum",
];

const ALPHABET = "abcdefghijkmnopqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789-_.!?";

fn sentence() {
    let words = [];
    for i in 0..random::int(6, 14) {
        words.push(global::WORDS[random::int(0, global::WORDS.len())]);
    }
    let text = words.reduce(|acc, w| if acc == "" { w } else { `${acc} ${w}` }, "");
    text.to_upper().sub_string(0, 1) + text.sub_string(1) + "."
}

fn paragraph() {
    let sentences = [];
    for i in 0..random::int(3, 6) { sentences.push(sentence()); }
    sentences.reduce(|acc, s| if acc == "" { s } else { `${acc} ${s}` }, "")
}

fn password(length) {
    let out = "";
    for i in 0..length {
        out += global::ALPHABET[random::int(0, global::ALPHABET.len())];
    }
    out
}

fn actions(value) {
    [
        #{ title: "Copy", copy: value },
        #{ title: "Paste", paste: value, shortcut: "ctrl+return" },
    ]
}

fn uuids(n) {
    let items = [];
    for i in 0..n {
        let id = random::uuid();
        items.push(#{ title: id, subtitle: "UUID v4", icon: "fingerprint", actions: actions(id) });
    }
    items
}

fn lorem(n) {
    let paragraphs = [];
    for i in 0..n { paragraphs.push(paragraph()); }
    // Back-tick strings are raw: "\n" only means a newline in a quoted string.
    let text = paragraphs.reduce(|acc, p| if acc == "" { p } else { acc + "\n\n" + p }, "");
    [#{
        title: paragraphs[0].sub_string(0, 60) + "…",
        subtitle: `${n} paragraph(s)`,
        icon: "text",
        detail: text,
        actions: actions(text),
    }]
}

fn passwords(length) {
    let pw = password(length);
    [#{ title: pw, subtitle: `${length} characters`, icon: "key", actions: actions(pw) }]
}

// "uuid 5" -> ["uuid", 5]; the count is clamped so nobody asks for a million.
fn command(query) {
    let words = query.to_lower().split(" ").filter(|w| w != "");
    let name = if words.is_empty() { "" } else { words[0] };
    let count = ();
    if words.len() > 1 {
        try { count = parse_int(words[1]); } catch { }
    }
    [name, count]
}

fn search(query) {
    let parsed = command(query);
    let name = parsed[0];
    let count = parsed[1];
    let clamp = |n, lo, hi, fallback| if n == () { fallback } else if n < lo { lo } else if n > hi { hi } else { n };

    if name != "" && "uuid".starts_with(name) {
        return uuids(clamp.call(count, 1, 20, 3));
    }
    if name != "" && "lorem".starts_with(name) {
        return lorem(clamp.call(count, 1, 10, 1));
    }
    if name != "" && "password".starts_with(name) {
        return passwords(clamp.call(count, 8, 128, 20));
    }
    #{
        sections: [
            #{ title: "uuid [count]", items: uuids(1) },
            #{ title: "lorem [paragraphs]", items: lorem(1) },
            #{ title: "password [length]", items: passwords(20) },
        ],
        placeholder: "uuid 5, lorem 3, password 32",
    }
}
