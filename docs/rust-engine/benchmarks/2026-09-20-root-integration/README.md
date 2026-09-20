# Application root integration audit

The same [untimed capture](../2026-09-20-query-audit/capture.py), same upstream
instrumentation and same 738-entry corpus, after wiring the application root
scorer into the port. [report.json](./report.json) records exact revisions, binary
hashes, ordered IDs and reported scores. No timing or memory claim is made.

| Query | Upstream | Rust | Ordered IDs equal |
|---|---:|---:|---|
| empty | 448 | 447 | no |
| chrom | 3 | 3 | yes |
| terminal | 8 | 8 | yes |
| calculator | 2 | 2 | yes |
| code | 19 | 19 | yes |
| é | 105 | 105 | no |
| zzzznonexistent | 0 | 0 | yes |

Every sampled nonempty query now has the same membership, without removing any
corpus files or discarding mismatching responses. The integration removes action
rows from root search, retains unresolved TryExec apps, uses upstream root field
weights and preserves alphabetical input order for score ties. Desktop actions
are available through each application's panel instead.

Following the XDG adapter was essential: upstream's `XdgApplication::keywords`
combines categories and keywords, both at root keyword weight 0.6. Looking only at
`AppRootItem::keywords` obscures that fact. The adapter is now exercised explicitly.

Two failures remain and **search performance is still not gated**:

- `host--Singular-manual.desktop` is a Type=Link URL, excluded by the application
  index. It needs a real URI launch path, not a benchmark-only synthetic entry.
- Unicode query `é` produces the same 105 IDs but different rankings. For example,
  upstream scores `Bear Factory — Animation Editor` at 100 while Rust reports 72.
  This is a scorer difference, not merely a different tie order. The raw ordered
  IDs and scores retain the failure for the next matcher comparison.

The capture uses Qt offscreen and a Rust daemon, not a visible launcher workload.
UI routing is separately covered by a task-level query/panel/action test; real
GNOME/Flatpak behavior still requires the target-session CI gate. Other providers,
root configuration, grouped presentation and UI persisted history remain open.
