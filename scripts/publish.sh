#!/usr/bin/env bash
# Verify and publish runarchive to crates.io.
#
# Usage:
#   scripts/publish.sh            # verify only: builds, tests, package dry-run
#   scripts/publish.sh --publish  # verify, then actually publish (needs `cargo login` done once)
set -euo pipefail
cd "$(dirname "$0")/.."

version=$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["packages"][0]["version"])')
echo "==> runarchive v${version}"

if [[ -n "$(git status --porcelain)" ]]; then
    echo "warning: working tree is not clean; the package is built from the files on disk" >&2
fi

# The feature sets are mutually exclusive: build and test each one on its own.
for features in "" sync async-tokio async-futures; do
    label=${features:-none}
    echo "==> build (features: ${label})"
    cargo build ${features:+--features "$features"}
    if [[ -n "$features" ]]; then
        echo "==> test (features: ${label})"
        # dhat uses a single global profiler, so the allocation suite must not run in parallel.
        cargo test --features "$features" --test allocation -- --test-threads=1
        cargo test --features "$features" --test parser --test extraction --test http
    fi
done

echo "==> package contents"
cargo package --list --allow-dirty

echo "==> publish dry run"
cargo publish --dry-run --allow-dirty

if [[ "${1:-}" == "--publish" ]]; then
    echo "==> publishing v${version} to crates.io"
    cargo publish
    echo "==> done; consider tagging: git tag v${version} && git push origin v${version}"
else
    echo "==> dry run complete; run 'scripts/publish.sh --publish' to publish v${version}"
fi
