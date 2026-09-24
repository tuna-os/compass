# Six desktop-entry parser bugs, for reporting upstream

This is the draft of the courtesy report [#14](https://github.com/tuna-os/compass/issues/14) asks
for: ADR-0007 says we should tell [vicinaehq/vicinae](https://github.com/vicinaehq/vicinae) about
the defects we found while porting `src/lib/xdgpp`, since we are forking away from them.

**Status: not filed.** Filing on another project's tracker is a decision for a maintainer of this
one, not for the port. The text below is ready to paste; line references are against this fork's
`src/lib/xdgpp`, which is unmodified from upstream at the point it was taken.

**Bug 1 is worth reporting regardless of how anyone feels about the fork** — it is an unbounded
loop with unbounded allocation, reachable from any malformed `.desktop` file the parser is pointed
at, and a launcher reads every `.desktop` file on the system by design.

---

## 1. A truncated locale suffix is an infinite loop that allocates without bound

`xdgpp/desktop-entry/reader.cpp:74`

```cpp
std::string DesktopEntryReader::parseRawLocale() {
  std::string locale;
  consume('[');
  while (!isPeek(']')) {
    locale += consume();
  }
  consume(']');
  return locale;
}
```

with, at lines 61–72:

```cpp
char DesktopEntryReader::consume() { const char c = peek(); ++m_cursor; return c; }
char DesktopEntryReader::peek() const {
  if (m_cursor < m_data.size()) return m_data[m_cursor];
  return 0;
}
bool DesktopEntryReader::isPeek(char c) const { return peek() && peek() == c; }
```

At end of input `peek()` returns `0`, so `isPeek(']')` is `false` and the loop does not terminate.
`consume()` returns `0` and advances the cursor anyway, so each iteration appends a NUL to
`locale`. The string grows until allocation fails.

**Reproduction.** Any entry whose last line is a truncated locale suffix:

```
[Desktop Entry]
Type=Application
Name=Example
Name[fr
```

**Suggested fix.** Terminate on `]`, on a line terminator, or at end of input. A locale suffix
cannot span a line, so stopping at the newline also stops the parser from swallowing the next key.

## 2. A key with no separator can swallow the following line

A key with no `=` consumes the following character, and on mismatch skips to the *next* newline.
When the character it consumed *was* the newline, the skip runs to the line after, and that line is
silently dropped.

**Suggested fix.** Skip only when the mismatched character is not the line terminator.

## 3. A field code glued to the rest of a word reorders the argument

Field codes are pushed directly into `args` while the in-progress word is still held in `part`, so
the pending word lands *after* the expansion:

```
Exec=prog --name=%c     →     ["prog", "MyFile", "--name="]
```

`--file=%f` and `--name=%c` forms are common in the wild. Upstream's own tests all use standalone
field codes, which is why this passes them.

**Suggested fix.** Substitute single-valued codes (`%f %u %c %k`) into the current word; flush the
word before multi-valued ones (`%F %U %i`). Every existing test result is unchanged.

## 4. The last element of a string list is not unescaped

`xdgpp/desktop-entry/value.cpp:34`, `asStringList` unescapes each element as it reaches a separator
but pushes the trailing residue raw:

```
Keywords=a;b\sc     →     ["a", "b\\sc"]        (expected ["a", "b c"])
```

**Suggested fix.** Unescape the final element on the same path as the others.

## 5. An unknown `Type` silently becomes `Application`

The `if`/`else` chain that maps `Type` has no `else`, so `Type=ServiceType` — or any typo — is
treated as an application and appears in the launcher.

**Suggested fix.** Represent an unrecognised type distinctly and exclude it from application
listings. A *missing* `Type` defaulting to `Application` is the existing behaviour and is fine.

## 6. Three optional getters can never be `nullopt`

`genericName()`, `version()` and `unlocalizedName()` return `std::optional<std::string>` built from
a plain `std::string`, so an absent key reads as `Some("")` rather than `nullopt`. Callers that
check `has_value()` cannot distinguish "absent" from "present and empty".

**Suggested fix.** Return `std::nullopt` when the key is absent.

---

## What we did with them

Each is listed in [`PARITY.md`](PARITY.md) under *`compass-xdg` — six C++ bugs deliberately **not**
reproduced*, with the test that pins our behaviour. Four behaviours we matched **on purpose** and
are not bugs: field codes are not expanded inside quotes; unknown and deprecated field codes expand
to nothing; a redeclared group replaces rather than merges; and localized score ties resolve to the
last declaration.
