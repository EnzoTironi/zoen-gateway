# Executor (Rust)

A Rust port of [UsefulSoftwareCo/executor](https://github.com/UsefulSoftwareCo/executor): the integration layer for agents. One catalog of tools, credentials, and policy. Not an agent. Not a chat UI. No web console.

The floor is a single verb — `execute(path, args)` — plus the catalog that makes that verb safe under load.

## What you can do

- Register OpenAPI / Swagger / Google Discovery specs, GraphQL endpoints, and MCP servers
- Bind connections (credentials live behind `SecretRef`; agents never see values)
- Gate tools with org-outer policy (`approve` / `require_approval` / `block`)
- Call tools from the CLI, an in-process SDK, a loopback HTTP daemon, or MCP stdio
- Run bounded code-mode scripts (`return await tools["path"](args)` natively, or real JS in in-process QuickJS)
- Device-login (`login` / `logout` / `whoami`) and named server profiles
- Encrypt the default secret store at rest (`EXS1` ChaCha20-Poly1305 box)
- Enterprise-managed MCP authorization (ID-JAG) when the Resource AS advertises the profile
- Optional Sentry envelopes (`SENTRY_DSN`); Cloudflare fetch-proxy Worker (`wrangler.toml`)

## Production bar

The runtime is built as if it will serve more than one million people: bounded in-flight executes (fail-fast overload, no unbounded queue), deadlines, cancellation, WAL SQLite with a connection pool, `spawn_blocking` on the disk path, idempotent retries, structured errors, and `tracing` + `Metrics` instead of `println`. Specs cap at 64 MiB; code-mode caps at 64 KiB source and 32 tool calls. QuickJS is capped at 64 MiB memory and 1 MiB stack; compute timeout pauses while tools run.

`EXECUTOR_KERNEL` selects the code-mode runtime: unset/`auto` (native subset, then QuickJS), `native`, or `js` (always QuickJS).

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
executor call --code 'return await tools.search({"query":"pets"});' --yes
EXECUTOR_KERNEL=js executor call --code 'return 1 + 2;' --yes
executor serve --port 4788
executor mcp          # attach-bridge to :4788/mcp when healthy, else in-process
executor login --no-poll
executor server add cloud --origin https://example.test --default
```

Daemon HTTP (loopback by default; `EXECUTOR_BIND=0.0.0.0` for containers):

| Path | Purpose |
|---|---|
| `GET /health` | liveness + loaded plugins |
| `GET /metrics` | atomic counters |
| `POST /mcp` | MCP JSON-RPC |
| `POST /api/execute` | `{ "path", "args", "auto_approve", "idempotency_key" }` |
| `POST /api/execute-code` | `{ "source", "auto_approve" }` |
| `GET /api/tools` | catalog page |
| `GET /api/auth/cli-login` | RFC 8628 discovery |
| `POST /api/oauth/register` | RFC 7591 DCR proxy |
| `GET /api/oauth/callback` | authorization-code landing (prints JSON; no chrome) |
| `GET /.well-known/oauth-protected-resource` | RFC 9728 |
| `GET /.well-known/oauth-authorization-server` | RFC 8414 (does **not** advertise ID-JAG) |

Default data dir: `EXECUTOR_DATA_DIR` or `~/.executor`. Catalog and secrets file mode `0600`. Secret key: `EXECUTOR_SECRET_KEY` or `secret.key`.

## Docker

```bash
docker build -t executor .
docker run --rm -p 4788:4788 -v executor-data:/var/lib/executor executor
```

The image binds `0.0.0.0:4788` as `nobody`. Put a reverse proxy in front for anything non-local.

## Crate layout

```
executor-core → executor-storage / executor-secrets → executor-codemode
  → executor-engine → executor-plugin-{openapi,graphql,mcp}
  → executor-host / executor-sdk → executor-cli
executor-test-support   starts/attaches to `npx emulate` for integration tests
```

## Not ported

Web console, desktop shell, marketing site, `executor web`, OAuth browser chrome, QuickJS/Deno/workerd. Cloudflare is a fetch proxy (`workers/proxy.js`), not the original UI+D1+WASM worker. `open` / `docs` print URLs only.
