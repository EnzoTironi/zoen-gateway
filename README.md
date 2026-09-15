# Executor (Rust)

A Rust port of [UsefulSoftwareCo/executor](https://github.com/UsefulSoftwareCo/executor): the integration layer for agents. One catalog of tools, credentials, and policy. Not an agent. Not a chat UI. No web console.

The floor is a single verb — `execute(path, args)` — plus the catalog that makes that verb safe under load.

## What you can do

- Register OpenAPI / Swagger / Google Discovery specs, GraphQL endpoints, and MCP servers
- Bind connections (credentials live behind `SecretRef`; agents never see values)
- Gate tools with org-outer policy (`approve` / `require_approval` / `block`)
- Call tools from the CLI, an in-process SDK, a loopback HTTP daemon, or MCP stdio

## Production bar

The runtime is built as if it will serve more than one million people: bounded in-flight executes (fail-fast overload, no unbounded queue), deadlines, cancellation, WAL SQLite with a connection pool, `spawn_blocking` on the disk path, idempotent retries, structured errors, and `tracing` + `Metrics` instead of `println`.

## Build

Requires Rust **1.98** (edition 2024).

```bash
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::pedantic
```

Integration tests call [emulate](https://github.com/vercel-labs/emulate) — the local GitHub / Google / Linear API emulator — they do not vendor it. `cargo test` starts `npx emulate@0.11.2` when `GITHUB_EMULATOR_URL` is unset. Requires Node.js 22+.

```bash
# optional: start the emulator yourself, then point the tests at it
npx --yes emulate@0.11.2 --service github,google,linear --seed emulate.config.yaml --port 18400
export GITHUB_EMULATOR_URL=http://127.0.0.1:18400
export GOOGLE_EMULATOR_URL=http://127.0.0.1:18401
export LINEAR_EMULATOR_URL=http://127.0.0.1:18402
cargo test --workspace --all-features
```

## Run

```bash
# in-process CLI against ~/.executor (or EXECUTOR_DATA_DIR)
executor tools list
executor tools integrations
executor call executor.openapi.addSpec '{"slug":"pets","spec":"{...}"}' --yes
executor call tools.pets.org.work.pets.listPets '{}' --yes
executor serve --port 4788
executor mcp          # stdio MCP host
```

Daemon HTTP (loopback only):

| Path | Purpose |
|---|---|
| `GET /health` | liveness + loaded plugins |
| `GET /metrics` | atomic counters |
| `POST /mcp` | MCP JSON-RPC |
| `POST /api/execute` | `{ "path", "args", "auto_approve", "idempotency_key" }` |
| `GET /api/tools` | catalog page |

Default data dir: `EXECUTOR_DATA_DIR` or `~/.executor`. Catalog file mode `0600`.

## Crate layout

```
executor-core → executor-storage / executor-secrets → executor-engine
  → executor-plugin-{openapi,graphql,mcp} → executor-host / executor-sdk → executor-cli
executor-test-support   starts/attaches to `npx emulate` for integration tests
```

## Not ported

Web console, desktop shell, marketing site, `executor web`, OAuth browser chrome, QuickJS multi-tool scripts. See the in-repo quality notes: clippy pedantic is deny in CI.
