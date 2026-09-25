set positional-arguments

# Show available development commands.
default:
    @just --list

# Benchmark the pinned upstream release in the same container locally and in CI.
# Currently measures warm IPC and process RSS/PSS; does not claim search parity.
bench-head-to-head output="target/head-to-head":
    python3 scripts/bench/run.py --output "$1"

# Fast checks for the benchmark orchestration, without downloading/building engines.
bench-check:
    python3 -m unittest discover -s scripts/bench -p 'test_*.py'
    python3 -m py_compile scripts/bench/run.py scripts/bench/session.py

# Drive already-running engines with a hand-written advanced workload.
bench-attached config report:
    cargo run --release --locked -p compass-testkit --bin head-to-head -- "$1" "$2"

# Run the VM tier on this branch's pushed head (needs `gh`). `jobs` is all,
# control, compass, launcher or spike-a; `checks` picks the compass job's
# packaging/vmtest/checks.sh subcommands, space-separated (empty: the default set).
vm-tier jobs="all" checks="":
    #!/usr/bin/env bash
    set -euo pipefail
    branch="$(git branch --show-current)"
    args=(-f jobs="$1")
    if [ -n "$2" ]; then args+=(-f checks="$2"); fi
    gh workflow run vm-tier.yaml --ref "$branch" "${args[@]}"
    echo "Dispatched the VM tier on $branch; follow it with: gh run watch \$(gh run list --workflow vm-tier.yaml --branch $branch --limit 1 --json databaseId --jq '.[0].databaseId')"
