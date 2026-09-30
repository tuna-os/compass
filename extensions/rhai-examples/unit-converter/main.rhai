// Unit Converter
//
//   10 km          -> every other length
//   10 km to mi    -> just miles (first), then the rest
//   72f in c       -> temperatures work too
//
// Shows: parsing the query, a constant table, sections, accessories, and
// `copy` actions (which is why the manifest asks for `clipboard.write`).

// symbol: [kind, factor to the base unit, name]. Temperatures are special-cased
// below because they are offsets, not factors.
const UNITS = #{
    mm: ["length", 0.001, "millimetres"],
    cm: ["length", 0.01, "centimetres"],
    m: ["length", 1.0, "metres"],
    km: ["length", 1000.0, "kilometres"],
    "in": ["length", 0.0254, "inches"],
    ft: ["length", 0.3048, "feet"],
    yd: ["length", 0.9144, "yards"],
    mi: ["length", 1609.344, "miles"],
    mg: ["mass", 0.001, "milligrams"],
    g: ["mass", 1.0, "grams"],
    kg: ["mass", 1000.0, "kilograms"],
    oz: ["mass", 28.349523125, "ounces"],
    lb: ["mass", 453.59237, "pounds"],
    ml: ["volume", 0.001, "millilitres"],
    l: ["volume", 1.0, "litres"],
    floz: ["volume", 0.0295735295625, "US fluid ounces"],
    cup: ["volume", 0.2365882365, "US cups"],
    gal: ["volume", 3.785411784, "US gallons"],
    c: ["temperature", 0.0, "degrees Celsius"],
    f: ["temperature", 0.0, "degrees Fahrenheit"],
    k: ["temperature", 0.0, "kelvin"],
};

fn to_base(value, unit) {
    let spec = global::UNITS[unit];
    if spec[0] != "temperature" { return value * spec[1]; }
    switch unit {
        "c" => value,
        "f" => (value - 32.0) * 5.0 / 9.0,
        _ => value - 273.15,
    }
}

fn from_base(value, unit) {
    let spec = global::UNITS[unit];
    if spec[0] != "temperature" { return value / spec[1]; }
    switch unit {
        "c" => value,
        "f" => value * 9.0 / 5.0 + 32.0,
        _ => value + 273.15,
    }
}

// 4 significant decimals, without a trailing ".0".
fn pretty(x) {
    let rounded = (x * 10000.0).round() / 10000.0;
    let text = rounded.to_string();
    if text.ends_with(".0") { text.sub_string(0, text.len() - 2) } else { text }
}

// "10km to mi" -> #{ amount: 10.0, from: "km", to: "mi" }, or () if it is not one.
fn parse(query) {
    let words = [];
    for word in query.to_lower().split(" ") {
        if word != "" && word != "to" && word != "in" && word != "=" { words.push(word); }
    }
    if words.is_empty() { return (); }

    // Split a leading number off the first word, so "10km" works like "10 km".
    let first = words[0];
    let digits = "";
    for ch in first.chars() {
        if (ch >= '0' && ch <= '9') || ch == '.' || ch == '-' { digits += ch; } else { break; }
    }
    if digits == "" { return (); }
    let rest = first.sub_string(digits.len());
    words.remove(0);
    if rest != "" { words.insert(0, rest); }

    let amount = 0.0;
    try { amount = parse_float(digits); } catch { return (); }
    if words.is_empty() || !(words[0] in global::UNITS) { return (); }
    let to = if words.len() > 1 && words[1] in global::UNITS { words[1] } else { () };
    #{ amount: amount, from: words[0], to: to }
}

fn row(amount, from, unit) {
    let value = from_base(to_base(amount, from), unit);
    let spec = global::UNITS[unit];
    let answer = `${pretty(value)} ${unit}`;
    #{
        id: unit,
        title: answer,
        subtitle: `${pretty(amount)} ${from} = ${pretty(value)} ${spec[2]}`,
        icon: "calculator",
        accessory: #{ tag: spec[0] },
        actions: [
            #{ title: "Copy Result", copy: pretty(value) },
            #{ title: "Copy With Unit", copy: answer, shortcut: "ctrl+shift+c" },
        ],
    }
}

fn search(query) {
    let q = parse(query);
    if q == () {
        return #{
            items: [],
            placeholder: "10 km to mi",
            empty: #{
                title: "Type an amount and a unit",
                description: "For example 10 km, 72 f to c or 3 lb in kg",
                icon: "calculator",
            },
        };
    }

    let kind = global::UNITS[q.from][0];
    let others = [];
    for unit in global::UNITS.keys() {
        if unit != q.from && unit != q.to && global::UNITS[unit][0] == kind {
            others.push(row(q.amount, q.from, unit));
        }
    }
    let sections = [];
    if q.to != () {
        if global::UNITS[q.to][0] != kind {
            return #{ items: [], empty: `Cannot convert ${kind} to ${global::UNITS[q.to][0]}` };
        }
        sections.push(#{ title: "Result", items: [row(q.amount, q.from, q.to)] });
    }
    sections.push(#{ title: `Other ${kind} units`, items: others });
    #{ sections: sections, placeholder: "10 km to mi" }
}
