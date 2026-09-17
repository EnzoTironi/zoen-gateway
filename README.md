# Executor ∪ Treg (Rust)

One process, one token, two superpowers:

1. **[Executor](https://github.com/UsefulSoftwareCo/executor/)** — configure integrations once (OpenAPI, GraphQL, MCP, Google Discovery), connections, org-outer policy, QuickJS code-mode, MCP `execute` / `skills` / `resume`.
2. **[Treg](https://github.com/superdesigndev/treg)** — search tools by **job**, priced catalog calls, credential ladder (your key always wins and is never metered), team tools, prepaid micro-USD.

This repository is the union in Rust. The Python/TS trees are reference only (not vendored). Live `treg.to` is not required: the bundled seed catalog and in-memory ledger are the self-host default.

The floor is still `execute(path, args)`. Treg adds `/call` by endpoint id. Secrets never appear in tool I/O.

## What you can do

- Register OpenAPI / Swagger / Google Discovery specs, GraphQL endpoints, and MCP servers
- Bind connections (credentials live behind `SecretRef`; agents never see values)
- Gate tools with org-outer policy (`approve` / `require_approval` / `block`)
- Search the priced catalog by job (`encontrar e-mail`) and call with the credential ladder
- Use one daemon bearer as the local “one token” (`Authorization` or `X-Treg-Token`)
- Call from the CLI, in-process SDK, loopback HTTP daemon, MCP, or the **pt-BR console**
- Resume approvals / OAuth pauses in the console (`/resume/{id}`), not URL-print-only
- Run bounded code-mode (`return await tools["path"](args)` natively, or in-process QuickJS)

## Production bar

Bounded in-flight executes (fail-fast overload), deadlines, cancellation, WAL SQLite, `spawn_blocking` on disk, idempotent retries, structured errors, `tracing` + `Metrics`. Specs cap at 64 MiB; code-mode caps at 64 KiB source. Money is integer **micro-USD**; balances change only through grant / reserve / settle / release.

`EXECUTOR_KERNEL` selects the code-mode runtime: unset/`auto` (native subset, then QuickJS), `native`, or `js`.

## Build

Requires Rust **1.98** (edition 2024) and Node.js 22+ for the console / emulate tests.

```bash
cargo build --workspace
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings -D clippy::pedantic
```

Integration tests call [emulate](https://github.com/vercel-labs/emulate). `cargo test` starts `npx emulate@0.11.2` when `GITHUB_EMULATOR_URL` is unset.

## Run

Two processes in development: daemon on `127.0.0.1:4788`, console on `127.0.0.1:43123`.

```bash
# daemon (CLI verbs auto-start this)
executor daemon run --foreground --port 4788

# console
cd console
npm install
npm run dev
```

Then open http://127.0.0.1:43123 or run `executor web`. Copy is Portuguese (Brazil).

```bash
executor catalog search encontrar e-mail
executor catalog get hunter.people.email.find
executor call hunter.people.email.find '{"domain":"stripe.com","full_name":"Patrick Collison"}'
executor connections add hunter work --value token=sk_own
executor connections list
executor balance
executor tools list
executor call --help github --match list --limit 20
executor resume --execution-id <id> --action accept
executor mcp
```

`executor.jsonc` at the working directory or `$EXECUTOR_DATA_DIR/executor.jsonc` is applied on daemon boot.

Protected routes require `Authorization: Bearer`, `x-executor-token`, `x-treg-token`, or `?_token=` from `{data_dir}/server-control/auth.json` (mode `0600`). `GET /health`, `GET /api/health`, well-known, CIMD, OAuth callback, and `GET /api/console/bootstrap` stay public on loopback.

| Path | Purpose |
|---|---|
| `GET /health` | liveness + loaded plugins |
| `GET /api/health` | CLI probe; body `ok` |
| `GET /api/catalog?q=` | job search (`providers`, `catalog_size`) |
| `GET /api/catalog/{id}` | endpoint + price + schema |
| `POST /api/call` | faithful priced/own-key call; `strict_query` → 400 |
| `GET /api/integrations/browse` | plugins + Google Discovery + Treg providers |
| `GET/POST /api/policies` · `DELETE /api/policies/{id}` | org-outer policy |
| `GET/POST /api/connections` | list / create (values write-only) |
| `GET/PATCH/DELETE /api/connections/{owner}/{integration}/{name}` | metadata / patch / remove |
| `POST /api/connections/{…}/refresh` · `/validate` | health + re-resolve tools |
| `POST /api/oauth/clients` · `POST /api/oauth/start` | public client + PKCE |
| `GET /api/balance` · `POST /api/balance/grant` | micro-USD ledger |
| `GET/POST /api/team-tools` | own relay tools |
| `GET /api/console/bootstrap` | loopback UI session |
| `POST /mcp` | Streamable HTTP MCP (`execute` / `skills` / `resume` / `catalog_*` / `connections_list`) |
| `POST /executions` | code-mode |
| `GET /executions/:id` | inspect pause |
| `POST /executions/:id/resume` | approve / decline / cancel |

Default data dir: `EXECUTOR_DATA_DIR` or `~/.executor`. Console origin: `EXECUTOR_CONSOLE_ORIGIN` (default `http://127.0.0.1:43123`). Point Next at a non-default daemon with `EXECUTOR_DAEMON_ORIGIN`.

## Catalog seed

Shipped in `crates/executor-catalog/data/seed.json` (not 3k YAML endpoints):

| Id | Access |
|---|---|
| `demo.echo` | anonymous, free |
| `demo.strict` | anonymous, `strict_query` |
| `hunter.people.email.find` / `.verify` | priced mock |
| `treg.people.email.find` | routed capability → hunter child |
| `github.user.get` | emulate when `GITHUB_EMULATOR_URL` is set |
| `moz.backlinks.lookup` | priced mock |
| `internal.private.crm` | unpublished price → refuse unless own key |

YAML ingest (`Catalog::load_yaml_dir` / `CatalogService::from_env_and_data_dir`) reads Treg-shaped documents from `EXECUTOR_CATALOG_DIR` (colon-separated) and `{data_dir}/catalog` on daemon boot. Seed ids win on merge. `*.extended.yaml` and meta files (`adapters`, `aliases`, `capabilities`, `contracts`) are skipped unless `EXECUTOR_CATALOG_INCLUDE_EXTENDED=1`.

## Crate layout

```
executor-core → executor-storage / executor-secrets → executor-codemode
  → executor-catalog → executor-engine → executor-plugin-{openapi,graphql,mcp}
  → executor-host / executor-sdk → executor-cli
console/                 Next.js App Router + shadcn (pt-BR)
executor-test-support    npx emulate for integration tests
```

## Not in this cut (still on the union list)

Overflow to relay, Stripe top-up, Enrich Arena, vendor CLI jail, skills upload, orgs/invites, daemon-embedded SPA. Deno / workerd kernels stay out on purpose.
