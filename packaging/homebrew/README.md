# Homebrew formula (staging for ublue-os/homebrew-experimental-tap)

`compass.rb` is the formula as submitted to the tap. Audit constraints
learned the hard way — keep them true when the install changes:

- Depends order is load-bearing and not what it looks like. The cop sorts
  every dep alphabetically first (`:linux` sorts as plain `linux`), then
  stable-partitions `:build` deps ahead — so the order is build deps,
  then everything else alphabetical with `:linux` interleaved:
  `pkg-config`, `rust`, `libxkbcommon`, `:linux`, `node`, `openssl@3`.
  Anything else fails `FormulaAudit/DependencyOrder`.
- The install must run `cargo install ... *std_cargo_args`;
  `FormulaAudit/Text` refuses `cargo build`. One install per crate, because
  `-p` cannot combine with the `--path` inside `std_cargo_args`, with
  `--bin compass-sandbox-exec` selecting the one shipped binary out of that
  crate (the probe stays out, as in the install script). A shared
  `CARGO_TARGET_DIR` keeps dependencies building once. The multi-binary
  layout (helpers in `libexec/compass`, data under `share/compass`) is
  spelled out here, mirroring `scripts/packaging/install-rust-engine.sh`,
  which stays the canonical list of what an install contains.
- `node` is a normal (not `:build`) dependency: the installed engine runs
  the extension runtime bundle with the distribution's Node, the way the
  Arch and Nix packages do.
- The `sha256` belongs to the GitHub-generated tag tarball and exists only
  after the tag is pushed; `scripts/bump_version.sh` moves the url and
  resets the hash to the placeholder, so filling it is part of the release.
