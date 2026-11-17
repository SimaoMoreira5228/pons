#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

export PYTHONDONTWRITEBYTECODE=1
export PYTHONPYCACHEPREFIX="$work/pycache"

python3 -m pytest -p no:cacheprovider "$root/tests" -q

mkdir "$work/bin"
ln -s "$root/pons-emit-python" "$work/bin/pons-emit-python"
export PATH="$work/bin:$PATH"

pons-emit-python describe > "$work/manifest.json"
python3 - "$work/manifest.json" <<'PY'
import json
import sys

manifest = json.loads(open(sys.argv[1], encoding="utf-8").read())
assert manifest["name"] == "python", manifest
assert manifest["protocol"] == {"min": "3.0", "max": "3.0"}, manifest
assert manifest["ir_version"] == {"min": "0.3", "max": "0.3"}, manifest
assert manifest["capabilities"]["enum_reprs"] == [
    "string",
    "external",
    "internal",
    "adjacent",
    "untagged",
], manifest
assert manifest["capabilities"]["pagination"] == [
    "pagination.cursor",
    "pagination.offset",
], manifest
assert manifest["capabilities"]["opaque"] is True, manifest
assert manifest["capabilities"]["validation"]["backends"] == ["builtin"], manifest
assert (
    manifest["capabilities"]["validation"]["capabilities"]["validation.enum.untagged"]
    == "exact"
), manifest
assert (
    manifest["capabilities"]["validation"]["capabilities"]["validation.opaque"]
    == "exact"
), manifest
PY

cargo build -q --manifest-path "$root/../../Cargo.toml" -p pons-cli
pons="$root/../../target/debug/pons"

"$pons" emit --target python "$root/../../spec/fixtures/exact.json" "$work/exact" --lossy 2> "$work/exact.err"
test -f "$work/exact/pons_client/__init__.py"
grep -q "test.example.source/v1" "$work/exact.err"

"$pons" emit --target python "$root/tests/fixtures/client.json" "$work/client"
test -f "$work/client/pons_client/client.py"

"$pons" emit --target python "$root/../../spec/fixtures/paginated.json" "$work/paginated"
test -f "$work/paginated/pons_client/client.py"
grep -q "def list_all(" "$work/paginated/pons_client/client.py"

"$pons" emit --target python "$root/tests/fixtures/untagged_opaque.json" "$work/untagged_opaque"
test -f "$work/untagged_opaque/pons_client/types.py"
grep -q "def decode_Shape(" "$work/untagged_opaque/pons_client/types.py"
grep -q "from typing import Any" "$work/untagged_opaque/pons_client/types.py"

cat > "$work/pons.toml" <<EOF
[project]
name = "validated"
ir = "$root/../../spec/fixtures/exact.json"

[emit.python]
out_dir = "$work/validated"
validation = { backend = "builtin" }
EOF
(
    cd "$work"
    "$pons" emit --target python --lossy --coverage coverage.json
)
test -f "$work/validated/pons_client/validators.py"
python3 - "$work/validated/coverage.json" <<'PY'
import json
import sys

report = json.loads(open(sys.argv[1], encoding="utf-8").read())
coverage = report["validation_coverage"]
assert coverage["lossy"] == [], coverage
assert coverage["unsupported"] == [], coverage
assert "types.User" in coverage["exact"], coverage
assert "operations.events.watch" in coverage["exact"], coverage
PY

if "$pons" emit --target python "$root/../../spec/fixtures/login.json" "$work/login" --lossy > "$work/login.out" 2> "$work/login.err"; then
    echo "login.json must be refused" >&2
    exit 1
fi
test ! -d "$work/login" || test -z "$(ls -A "$work/login")"
grep -q "streams" "$work/login.err" "$work/login.out"

echo "python emitter ok"
