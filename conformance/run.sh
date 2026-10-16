#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
temporary="$(mktemp -d)"
trap 'rm -rf "$temporary"' EXIT

cargo run --quiet --manifest-path "$root/Cargo.toml" -p reference-api > "$temporary/axum.json"
(
    cd "$root/jvm"
    ./gradlew --quiet :conformance-reference:ponsExport > "$temporary/spring.json"
)
cargo run --quiet --manifest-path "$root/Cargo.toml" -p pons-conformance -- \
    "$temporary/axum.json" "$temporary/spring.json"
