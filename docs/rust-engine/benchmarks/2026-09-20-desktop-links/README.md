# Desktop-link integration audit

The same [untimed capture](../2026-09-20-query-audit/capture.py), pinned upstream
v0.29.0 instrumentation and 738-entry real desktop corpus, after adding link
indexing and URI dispatch. [report.json](./report.json) preserves both engines'
ordered IDs and scores, exact revisions and binary hashes.

| Query | Upstream | Rust | Ordered IDs equal |
|---|---:|---:|---|
| empty | 448 | 448 | yes |
| chrom | 3 | 3 | yes |
| terminal | 8 | 8 | yes |
| calculator | 2 | 2 | yes |
| code | 19 | 19 | yes |
| é | 105 | 105 | no |
| zzzznonexistent | 0 | 0 | yes |

The previously missing `host--Singular-manual.desktop` is now present, without
removing any corpus entries or changing the workload. All sampled queries have
equal membership. Unicode ranking still differs: the Bear Factory editor titles
remain examples of score differences, not merely changed tie ordering.

**Search timing remains gated on ordered-result parity.** This offscreen Qt and
Rust-daemon capture is not a timing, memory, renderer or target-launch test.
Separate tests cover core indexing, UI action filtering and daemon queries;
Linux URI argument tests prove neither a browser window nor a document viewer
actually opened. Target-session link activation remains to be checked.
