# Pons

Pons reads the HTTP API a server already has, records it as one versioned IR document, and compiles that document into clients.

A frontend reads routes, extractor types, and serializer configuration. An emitter turns the IR into source. The IR keeps enum representations, absent versus null, wide integers, typed errors, and credential provisioning, which an OpenAPI round trip tends to lose.

## Use it

```sh
cargo install --path cli/pons-cli

pons init
pons export
pons check api/ir.json
pons emit --target ts
```

`pons init` writes a `pons.toml`. `pons export` builds and runs the configured frontend; no server starts. `pons emit` writes the client.

```toml
[project]
name = "acme"
ir = "api/ir.json"

[frontend]
kind = "axum" # "axum", "actix", or "spring"
command = "cargo pons export"

[emit.ts]
out_dir = "web/src/api"
```

Without a config file:

```sh
pons emit --target ts api/ir.json client
pons emit --target openapi api/ir.json openapi
pons emit --target go api/ir.json internal/api
```

TypeScript and OpenAPI are built in. Go and Python are external executables named `pons-emit-go` and `pons-emit-python`; put them on `PATH` or in `[plugins] search`.

## Frontends and targets

Frontends: Axum, actix-web, Spring Boot, Tako, a FastAPI producer, and an OpenAPI 3.0/3.1 importer.

Targets: TypeScript, OpenAPI 3.1, Go, and Python.

## Test

```sh
cargo test --workspace --all-features
./conformance/run.sh
./emitters/pons-emit-go/test.sh
./emitters/pons-emit-python/test.sh
python3 -m pytest frontends/pons-fastapi/tests
```

Conformance exports the same API from Axum, Spring Boot, and Tako, then compares the documents.

## Status

IR 0.3, pre-1.0. Nothing is published yet. Minor versions may break, so pin a commit and regenerate server and clients together.

## License

MIT or Apache-2.0, at your option. See `LICENSE-MIT` and `LICENSE-APACHE`.
