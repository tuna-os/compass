# Single-character Unicode boundary audit

The same [untimed capture](../2026-09-20-query-audit/capture.py), pinned upstream
instrumentation and 738-entry corpus after correcting the single-character
Unicode boundary scan. [report.json](./report.json) records revisions, binary
hashes, full ordered IDs and scores. This is not a timing or memory comparison.

The defect is visible in nucleo 0.3.1's `substring_match_1_non_ascii`: nonmatching
characters skip the previous-character update. The same code is present in
[upstream's exact.rs](https://github.com/helix-editor/nucleo/blob/3d46b62546aa8fa6784d37b13942cc7b6b062134/matcher/src/exact.rs).
The adapter uses nucleo's public postfix scorer at each matching position, so
the actual previous character determines the bonus. No scoring constants are
copied, the matcher is not replaced, and the scan remains linear.

The Bear Factory editor regression failed at raw score 26 versus 36 before the
fix. Their positions and scores now agree with pinned Vicinae. The lower-level
1685-query scorer corpus removes six differences (1417 to 1411), with unchanged
100% top-result agreement, including all 920 contested queries. The six corrected
query/title pairs have explicit regression tests.

The end-to-end audit still has a failure:

| Query | Upstream | Rust | Ordered IDs equal |
|---|---:|---:|---|
| empty | 448 | 448 | yes |
| chrom | 3 | 3 | yes |
| terminal | 8 | 8 | yes |
| calculator | 2 | 2 | yes |
| code | 19 | 19 | yes |
| é | 105 | 105 | no |
| zzzznonexistent | 0 | 0 | yes |

All queries retain equal membership. For `é`, the first 91 positions now agree;
positions 92–105 still differ. Upstream ties these items at 50, while Rust splits
them at 44 and 41. The underlying matcher score difference is not fixed by this
Unicode boundary correction. No responses, corpus entries or queries were
discarded. **Search timing remains gated on the unresolved ordered-result
parity failure.** Other providers and visible-launcher target performance remain
separate requirements.
