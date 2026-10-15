# Contributing

Litecom is an unreleased Rust project. Keep the CLI, background service, and relay in one binary. Prefer a complete, understandable user flow over more configuration or infrastructure.

## Reuse before building

Prefer established open-source libraries and adapters for capabilities outside Litecom's core: account/contact mapping, request routing, human approval, and controlled delivery. Reuse authentication and cryptographic primitives, protocol SDKs, provider transports, and packaging tools. Use existing agents for reasoning and their connected tools for service actions. Do not add our own model loop or service-specific GitHub/email execution layer.

Before adopting a dependency, check its license, maintenance, documented compatibility, permission behavior, and installation burden. Prefer released versions and verify the actual integration. For example, Pi uses the external `pi-acp` adapter through our shared ACP client; Litecom does not maintain a second Pi protocol bridge. A dependency's limitations belong in the compatibility docs.

[Open-source components and their roles →](docs/dependencies.md)

## Develop

```sh
cargo build --locked
cargo test --locked
cargo fmt --all --check
cargo clippy --all-targets --locked -- -D warnings
```

Run the relay using [Run a local server](docs/setup.md#run-a-local-server). `--dev` binds development delivery to loopback and prints test codes. Production delivery never prints codes.

The browser smoke test requires Python 3, Playwright, and installed Google Chrome. Run it only against a fresh development relay; it creates test accounts and transfers a fixture file.

```sh
python3 -m venv .litecom/qa
.litecom/qa/bin/pip install playwright
mkdir -p .litecom/qa-data
./target/debug/litecom relay \
  --public-url http://127.0.0.1:4141 \
  --data .litecom/qa-data \
  --dev > .litecom/qa.log 2>&1 &
relay_pid=$!
.litecom/qa/bin/python scripts/ui-smoke.py --dev-log .litecom/qa.log
.litecom/qa/bin/python scripts/onboarding-smoke.py --dev-log .litecom/qa.log
kill "$relay_pid"
```

On macOS, add `--with-service` to the onboarding test to exercise a temporary per-user LaunchAgent; the test removes it during cleanup. Native execution uses a deterministic fixture and does not call a model.

The transfer test uses two isolated browser accounts, checks privacy before approval, downloads and hashes a real file, acknowledges receipt, revokes access, and inspects desktop/mobile layout. Test delivery does not establish that an SMS provider works.

To exercise the README preview without managing a relay:

```sh
cargo build --locked
.litecom/qa/bin/python scripts/preview-smoke.py
```

This uses the actual temporary-server flow and a fixture agent, and verifies that the preview cleans up its data and leaves native MCP configuration alone.

Run `.litecom/qa/bin/python scripts/acp-smoke.py` for terminal setup with a fixture ACP process, browser tool approval and rejection, file transfer, and cancellation. It starts and cleans up its own local development relay and does not call a model.

For browser recovery and a timed core-protocol stress test:

```sh
.litecom/qa/bin/python scripts/reliability-smoke.py
python3 scripts/soak.py --seconds 3600
```

Both start isolated development relays and clean them up. The browser test covers stale decisions, interrupted/corrupt downloads, lost receipts, and edits during background updates. The seeded soak test varies binary payloads, self/contact requests, approvals, cancellation, permission decisions, and relay restarts. Runs longer than 125 seconds also check real lease expiry. Agent operations and delivery are fixtures; neither test calls a model or sends messages. The soak report is saved locally in `.preview/soak.json`.

To check the actual released Pi adapter against a simulated Pi backend, install it in an isolated test folder and run:

```sh
npm install --prefix .litecom/pi-adapter --ignore-scripts --no-audit --no-fund pi-acp@0.0.34
.litecom/qa/bin/python scripts/acp-smoke.py --agent pi \
  --pi-acp .litecom/pi-adapter/node_modules/.bin/pi-acp
```

This tests upstream ACP/RPC translation and Litecom's review flow without invoking a model. It does not validate a real Pi provider login, extensions, or native tools.

To test setup against an installed coding-agent CLI, use Python 3.11+:

```sh
.litecom/qa/bin/python scripts/mcp-setup-smoke.py --agent codex
.litecom/qa/bin/python scripts/mcp-setup-smoke.py --agent claude
.litecom/qa/bin/python scripts/mcp-setup-smoke.py --agent opencode
.litecom/qa/bin/python scripts/mcp-setup-smoke.py --agent pi
```

Pi requires both `pi` and `pi-acp` on PATH; use `--path /absolute/bin/directory` to add a temporary installation. These tests use isolated configuration directories, preserve a conflicting MCP server, and repeat setup. Claude, OpenCode, and Pi report the connection status; Codex checks its saved configuration. Codex/Claude also test disabled connections and malformed configuration. They do not start a worker or call a model.

## Live agent checks

With an installed, signed-in Codex, Claude Code, or OpenCode and the Python environment above:

```sh
cargo build --locked
.litecom/qa/bin/python scripts/live-agent-smoke.py --agent codex
# Or, using your Claude Code account:
.litecom/qa/bin/python scripts/live-agent-smoke.py --agent claude
# Or OpenCode through its ACP interface:
.litecom/qa/bin/python scripts/live-agent-smoke.py --agent opencode
```

These tests call a real model through the agent's existing configuration and incur its normal usage. They create two local test accounts, request a calculation in a temporary folder, approve execution, review the result, and verify the downloaded bytes. They check project instructions and privacy between the two approvals. No messages are sent, no MCP settings or startup services are installed, and temporary data is removed afterward.

Private screenshots and summaries are saved under `.preview/live-agent/`. A failed test also retains native diagnostics there; inspect these locally and do not publish credentials or account details. A provider spending limit or missing permission is a failure to investigate, not a reason to bypass it or silently switch accounts.

For a live Codex ACP permission check, install the adapter in an isolated directory and run:

```sh
npm install --prefix .litecom/codex-adapter --ignore-scripts --no-audit --no-fund @agentclientprotocol/codex-acp@2.2.2
.litecom/qa/bin/python scripts/live-agent-smoke.py --agent codex-acp \
  --path .litecom/codex-adapter/node_modules/.bin --review-permissions
```

The test starts the adapter in read-only mode. When a permission is requested, it prints the path to a private proposal file. Inspect the exact operation, then write `approve` or `decline` to the printed `.decision` path. The test clicks the corresponding Litecom control. It never selects a persistent grant; a decline will normally prevent the expected file result and fail the success check. The default test deadline is five minutes and can be changed with `--timeout`.

On Windows, `./scripts/windows-service-smoke.ps1` checks the built release executable against a temporary Task Scheduler task. It distinguishes installed/stopped from running, then exercises stop and removal. The task has no automatic trigger and is removed during cleanup; this check does not connect an account or model.

## Container checks

The [self-hosting guide](docs/self-host.md) covers the Compose deployment. To test the container without sending messages or calling a model:

```sh
docker build -t litecom:qa .
python3 scripts/container-smoke.py
```

The test uses isolated Compose resources, fixture credentials, a local HTTPS certificate, and the documented backup/restore import. It removes its containers and volumes afterwards. `--context NAME` selects a separate Docker context.

## Dependency checks and notices

Install the pinned audit and notice tools once:

```sh
cargo install cargo-deny --version 0.20.2 --locked
cargo install cargo-about --version 0.9.2 --features cli --locked
```

After changing dependencies, run:

```sh
cargo deny --locked check advisories licenses sources
cargo about generate --locked --fail -o THIRD-PARTY-NOTICES.html about.hbs
```

Commit the lockfile and regenerated notices together. CI checks the dependency policy and rejects stale notices. The generated file includes source links and license text for the configured release platforms; downloads and the container image include it. `option-ext`, an unmodified dependency of `directories`, has an explicit MPL-2.0 allowance. New licenses require review in `deny.toml` and `about.toml`.

The minimum supported compiler is Rust 1.88. CI checks the library, binary, and test code with that version on Linux, and runs tests on current stable Rust across macOS, Linux, and Windows. Advisory checks cover the Rust dependency graph; they do not replace review of Litecom's authorization rules, toolchain runtime, bundled C libraries, operating-system packages, or external agents.

## Code map

| Module | Responsibility |
| --- | --- |
| `store` | SQLite transactions, contacts, requests, snapshots, decisions, leases |
| `backup`, `store/recovery` | Online snapshots, verification, and restoration without replaying authority |
| `auth` | Phone challenges, browser sessions, device pairing, revocation |
| `api` | HTTP authorization, CSRF, request schemas, downloads |
| `notifications`, `providers` | Durable delivery queue and provider transport |
| `client`, `mcp`, `main` | Terminal client and official Rust MCP SDK integration |
| `onboarding`, `agent_setup`, `runner`, `service` | Guided setup, native MCP configuration, agent execution, per-user startup |
| `acp`, `store/permissions` | ACP sessions and exact per-operation human decisions |
| `cloud`, `devin` | Durable relay jobs, Devin transport, private result collection |
| `preview` | One-command temporary local trial |
| `web` | Small browser pages for sign-in, review, contacts, and downloads |

State transitions belong in `store`, not protocol adapters. Notifications never decide a request's state. Model prompts never replace access control. Provider and harness failures must leave a truthful, recoverable status.

## Before a public release

Use [Build download archives](docs/distribution.md#build-download-archives) to package the same executable for clients and self-hosted relays. The manual **Build downloads** workflow prepares and checks native archives without publishing a release.

Work against the full [release checklist](docs/release-readiness.md). Keep actual evidence separate from intended behavior. Do not claim universal harness support, working phone delivery, cloud execution, or signed installation based on local tests.
