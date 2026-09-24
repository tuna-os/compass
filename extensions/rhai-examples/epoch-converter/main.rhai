// Epoch Converter
//
//   (empty)                    -> the current time, every way
//   1700000000                 -> that instant as a date (seconds)
//   1700000000000              -> the same, read as milliseconds
//   2024-03-01T12:00:00Z       -> back to a timestamp
//   2024-03-01                 -> midnight UTC that day
//
// Shows: the `time::` helpers every script has, a detail pane per row, and
// error handling with try/catch. All times are UTC.

// "3 days ago" / "in 2 hours", from a difference in seconds.
fn relative(delta) {
    let past = delta < 0;
    let n = delta.abs();
    let text = if n < 60 { `${n} seconds` }
        else if n < 3600 { `${n / 60} minutes` }
        else if n < 86400 { `${n / 3600} hours` }
        else if n < 86400 * 365 { `${n / 86400} days` }
        else { `${n / (86400 * 365)} years` };
    if n < 1 { "now" } else if past { `${text} ago` } else { `in ${text}` }
}

fn row(title, subtitle, value, icon) {
    #{
        id: subtitle,
        title: title,
        subtitle: subtitle,
        icon: icon,
        actions: [#{ title: "Copy", copy: value }],
    }
}

// Everything we know about one instant.
fn describe(seconds, heading) {
    let iso = time::format(seconds);
    let p = time::parts(seconds);
    let when = relative(seconds - time::now());
    let detail = #{
        markdown: `## ${iso}` + "\n\n" + `${p.weekday}, day ${p.ordinal} of ${p.year}`,
        metadata: [
            #{ title: "Unix seconds", text: seconds },
            #{ title: "Unix milliseconds", text: seconds * 1000 },
            #{ title: "Relative", text: when },
        ],
    };
    let human = time::format(seconds, "[weekday], [day] [month repr:long] [year] [hour]:[minute]:[second]");
    let items = [
        row(iso, "ISO 8601 (UTC)", iso, "calendar"),
        row(human, "Human readable (UTC)", human, "calendar"),
        row(`${seconds}`, "Unix seconds", `${seconds}`, "clock"),
        row(`${seconds * 1000}`, "Unix milliseconds", `${seconds * 1000}`, "clock"),
        row(when, "Relative to now", when, "clock"),
    ];
    // A `for` variable is a copy, so write through the index.
    for i in 0..items.len() { items[i].detail = detail; }
    #{ sections: [#{ title: heading, items: items }], placeholder: "1700000000 or 2024-03-01T12:00:00Z" }
}

fn search(query) {
    let q = query;
    q.trim(); // string methods like trim() change the string in place
    if q == "" || q == "now" {
        return describe(time::now(), "Now");
    }

    // A number: seconds, or milliseconds if it is too big to be seconds.
    let number = ();
    try { number = parse_int(q); } catch { }
    if number != () {
        if number.abs() > 100_000_000_000 {
            return describe(number / 1000, "From milliseconds");
        }
        return describe(number, "From seconds");
    }

    // A date. A bare day means midnight UTC.
    let text = if q.len() == 10 { `${q}T00:00:00Z` } else { q };
    try {
        return describe(time::parse(text), "From date");
    } catch (error) {
        return #{
            items: [],
            empty: #{
                title: "Not a timestamp or a date",
                description: "Try 1700000000, 1700000000000, 2024-03-01 or 2024-03-01T12:00:00Z",
                icon: "clock",
            },
        };
    }
}
