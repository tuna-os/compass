# This Makefile is only for use on UNIX systems.

SHELL							:= /bin/sh

# Build the engine, then start it with its window open.
dev: build-rust
	cargo run -p compass -- start
.PHONY: dev

genicon:
	node scripts/generate-icons.js
.PHONY: genicon

# Regenerate the glyph table (crates/compass-core/glyph/README.md). Needs node
# >= 22.18 and network access; commit the result.
emoji:
	cd crates/compass-core/glyph/scripts && node fetch.ts && node gen.ts
.PHONY: emoji

# Regenerate the committed TypeScript protocol bindings from figura/*.fig.
# compass-figura's test fails while they are stale.
figen:
	cargo run -q -p compass-figura -- regenerate .
.PHONY: figen

tsfmt:
	cd src/typescript && biome format --write .
.PHONY: tsfmt

format: tsfmt fmt-rust
.PHONY: format

check-format: check-format-rust
.PHONY: check-format

bump-patch:
	./scripts/bump_version.sh patch
.PHONY: bump-patch

bump-minor:
	./scripts/bump_version.sh minor
.PHONY: bump-minor

bump-major:
	./scripts/bump_version.sh major
.PHONY: bump-major

nix-hash:
	$(SHELL) scripts/update-nix-npm-hashes.sh
.PHONY: nix-hashes

nix-hash-check:
	$(SHELL) scripts/update-nix-npm-hashes.sh --check
.PHONY: nix-hash-check

clean:
	cargo clean
	$(RM) -rf ./src/typescript/api/node_modules
	$(RM) -rf ./src/typescript/api/dist
	$(RM) -rf ./src/typescript/extension-manager/dist/
	$(RM) -rf ./src/typescript/extension-manager/node_modules
	$(RM) -rf ./scripts/.tmp
.PHONY: clean

# ---------------------------------------------------------------------------
# Rust engine (crates/). See docs/rust-engine/PLAN.md.
# ---------------------------------------------------------------------------

build-rust:
	cargo build --workspace
.PHONY: build-rust

# The extension runtime the Rust host drives in its real_runtime test. Not part
# of check-rust: it needs npm, and the test skips without it rather than
# failing.
extension-runtime:
	./scripts/build-extension-runtime.sh
.PHONY: extension-runtime

test-rust:
	cargo test --workspace --all-targets
	cargo test --workspace --doc
.PHONY: test-rust

lint-rust:
	cargo clippy --workspace --all-targets -- -D warnings
.PHONY: lint-rust

fmt-rust:
	cargo fmt --all
.PHONY: fmt-rust

check-format-rust:
	cargo fmt --all -- --check
.PHONY: check-format-rust

# Everything CI runs for the Rust workspace, in the same order.
.PHONY: design design-shots design-states
design-states: ## Print the launcher's design tokens and states to tools/design/states.json
	cargo run -q -p compass-ui --example design_states > tools/design/states.json

design: design-states ## Serve the design surrogate at http://127.0.0.1:8173
	@echo "http://127.0.0.1:8173 — Ctrl-C to stop"
	@cd tools/design && python3 -m http.server 8173

design-shots: design-states ## Screenshot every design state, both appearances
	node tools/design/shoot.mjs

check-rust: check-format-rust lint-rust test-rust
.PHONY: check-rust

# THE TEST LADDER — cheapest first, each rung proving what the one below can't.
#
#   t0  logic     unit + integration, no display              seconds
#   t1  paint     the launcher RENDERED, pixels checked        ~3 s / backend
#   t2  session   real Mutter + portal in a container          ~92 s
#   t3  target    real GNOME in a VM, screenshots diffed       ~30 min
#
# Climb only as far as the question needs. A failure at t3 should be pulled
# down to the cheapest rung that reproduces it — see RENDER-HARNESSES.md for
# which rung owns which question. `make test-fast` is the per-edit loop.
.PHONY: test-fast test-t0 test-t1 test-t2 test-t3

test-fast: test-t0 test-t1 ## t0 + t1: logic and paint, the per-edit loop

test-t0: ## Ladder t0: unit and integration tests
	@if command -v cargo-nextest >/dev/null 2>&1; then \
		cargo nextest run --workspace --all-targets; \
	else \
		cargo test --workspace --all-targets; \
	fi

# Each backend is FORCED and CHECKED. Iced falls back from wgpu to tiny-skia
# silently; without PAINT_EXPECT_BACKEND a "wgpu" run could pass on tiny-skia
# and prove nothing about wgpu. wgpu needs an adapter — on a machine with no
# GPU, `apt install mesa-vulkan-drivers` provides lavapipe.
test-t1: ## Ladder t1: the paint tier, on wgpu (if an adapter exists) and tiny-skia
	@if ls /usr/share/vulkan/icd.d/*.json >/dev/null 2>&1; then \
		echo "== t1 paint: wgpu =="; \
		ICED_TEST_BACKEND=wgpu PAINT_EXPECT_BACKEND=wgpu \
			cargo test -p compass-ui --test paint; \
	else \
		echo "== t1 paint: wgpu SKIPPED — no Vulkan driver (apt install mesa-vulkan-drivers) =="; \
	fi
	@echo "== t1 paint: tiny-skia =="
	@ICED_TEST_BACKEND=tiny-skia PAINT_EXPECT_BACKEND=tiny-skia \
		cargo test -p compass-ui --test paint

test-t2: ## Ladder t2: the launcher in a real Mutter session, in a container
	@command -v podman >/dev/null 2>&1 || { \
		echo "t2 needs podman. It boots real Mutter and xdg-desktop-portal-gnome"; \
		echo "in a Fedora container — the tier that proves a window appears."; \
		exit 1; }
	scripts/tier2/headless-gnome-spike.sh scripts/tier2/prove-smoke.sh

test-t3: ## Ladder t3: real GNOME in a VM — runs in CI, not locally
	@echo "t3 boots Bluefin in QEMU through corral and diffs real screenshots."
	@echo "It is too heavy to be a local target. Run it on your branch with:"
	@echo ""
	@echo "    gh workflow run vm-tier.yaml --ref \$$(git branch --show-current)"
	@echo ""
	@echo "It also runs nightly (03:00 UTC) and on every v* release tag."
	@echo "Before reaching for it: can t1 or t2 reproduce the failure? They are"
	@echo "seconds and a minute and a half; this is half an hour."


FLATPAK_MANIFEST := packaging/flatpak/org.tunaos.compass.yaml

# Regenerate the offline dependency manifest Flathub builds require. Needs
# flatpak-cargo-generator.py from flatpak/flatpak-builder-tools on PATH.
flatpak-sources:
	flatpak-cargo-generator.py Cargo.lock -o packaging/flatpak/cargo-sources.json
.PHONY: flatpak-sources

flatpak-rust:
	flatpak-builder --user --install --force-clean build-flatpak $(FLATPAK_MANIFEST)
.PHONY: flatpak-rust

flatpak-run: flatpak-rust
	flatpak run org.tunaos.compass -- doctor --check-only
.PHONY: flatpak-run
