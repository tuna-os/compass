# Upstream Vicinae sources for the differential probes

Verbatim copies of two libraries from upstream Vicinae **v0.29.0**
(`vicinaehq/vicinae@c3415a3ed56676d2960d90975ab319ae8a7aba6e`, the release
`../baseline.json` pins), at the same paths under `src/lib/` there:

- `fuzzy/include/fuzzy/*.hpp`: the header-only scorer. `../fuzzy/cpp_rank.cpp`
  times it for `compare.sh`, and `../probes/fuzzy-probe.cpp` exposes it to
  `compass-testkit --bin scorer-parity`.
- `crypto/`: AES-256-GCM and HKDF over OpenSSL (the Linux backend only).
  `../probes/crypto-probe.cpp` exposes it to `compass-testkit --bin crypto-parity`.

They were the fork's in-tree `src/lib/fuzzy` and `src/lib/crypto` until the C++
engine was removed (ADR-0021), and were byte-identical to upstream's at the pinned
commit when they moved. Do not edit them: to compare against a newer release,
replace them with that release's files and update the commit above.

Nothing in the Rust build compiles them.
