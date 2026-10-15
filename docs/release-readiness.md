# Release readiness

The complete product requirements remain in [the design](design/README.md). A green unit test suite alone is not a release.

Litecom connects to existing agents. It does not ship its own reasoning agent or model tool loop. The required agent set is OpenCode, Codex, pi, Claude Code, Cursor, Devin, OpenClaw, and Hermes; individual evidence must support each claim.

| Requirement | Evidence needed | State |
| --- | --- | --- |
| Durable contacts and requests | Acceptance, authorization, restart, queue and cancellation tests | Core tests and two-account browser flow pass; deployment recovery pending |
| Request approval and delivery | Cross-user access, execution gate, immutable result files, stale approvals and race tests | Privacy, snapshot, stale/duplicate decisions and browser transfer pass; independent SQLite-connection races verify one request, one claim, one decision, and no release of unreviewed edits |
| General actions | Real harness code change, test command and PR flow with permission boundaries | Live Codex file creation and exact transfer pass. Broader code/test/PR and external-tool workflows pending |
| GitHub Write access example | Ordinary request through the selected agent's existing GitHub tools; exact recipient/repository approval and truthful grant/invitation result | Pending live harness demonstration; no GitHub-specific Litecom connector or action handler is in scope |
| Terminal setup | Phone/Slack/Email choice, saved progress, main harness, sources, measured controls | Phone path, native detection, folder, saved progress, reconnect, repeat setup and macOS startup pass; Email linking tested locally; Slack/provider validation and clean consumer install pending |
| Phone signup and approval | Real OTP, delivery, opt-out, new-device authentication, mobile review | Real inbound SMS and signed callback pass; outgoing sign-in and approval await carrier registration |
| Slack linking and approval | Phone identity, real app install, signed button approval, original message update | Confidential OAuth, session-bound linking, signed callbacks, token refresh and message-update fixture tests pass; real workspace flow pending |
| Email linking and approval | Phone identity, notification email verification, real mail delivery and reply approval | Local terminal/browser email setup, reply authorization, MIME parsing and IMAP restart/checkpoint tests pass; real SMTP/IMAP service validation pending |
| Multiple approval channels | Account linking, concurrent decisions and synchronization | Signed SMS/Slack callbacks, email reply capabilities, stale/duplicate/foreign decisions, and shared state tests pass; actual provider delivery pending |
| Easy contacts | Share link, mutual acceptance, name lookup and ambiguity handling | Core and browser flow pass; live multi-device onboarding pending |
| One phone identity across agents | Verified phone account, independently revocable agent connections, shared contacts/requests from every harness | Pending |
| Named local and cloud agents | “Get Moe” resolves his default; “get Moe’s Devin” selects Devin; real cloud execution while laptop is offline | Devin transport, relay worker, connection UI and persisted jobs implemented; restart, privacy, failed-launch and cancellation fixtures pass. Live account and generic A2A dispatch pending |
| MCP | Official client integration and supported harness matrix | Official SDK launch/tools probe and real Codex/Claude/OpenCode/Pi setup pass. Remaining live harness matrix pending |
| A2A | Released SDK interoperability, task/artifact/auth lifecycle | Official A2A JavaScript SDK 1.3.0 discovery, request/retry, blocking completion, private artifacts, exact download and cancellation checks pass; full conformance remains pending |
| ACP and native context | Codex/Claude sessions, memory, skills, hooks and mediated tool execution | Native CLI adapters preserve configuration; single request approval, automatic result delivery and subprocess fixtures pass. ACP v1 sessions and exact one-time permission forwarding pass SDK and browser/subprocess fixtures. Live native Codex and Codex ACP project instructions, file creation, and ACP one-time permission approval pass; broader context/tool validation pending |
| Email retrieval example | Real retrieval through the selected agent's existing email tools and reviewable exports | Pending live harness demonstration; notification email remains a separate Litecom feature |
| Background operation | Startup, sleep/offline, reconnect, restart and safe updates | macOS startup, worker heartbeat and repeat setup pass. Linux/Windows definitions implemented; live platform, sleep and upgrade checks pending |
| Downloads | Authenticated browser/MCP receipt, all file types, expiry and interrupted transfer | Streamed browser downloads, explicit receipt, verified CLI/MCP download, preapproval privacy and corruption/revocation checks pass; 2 GiB native upload/browser/CLI transfer and progressing 64-second upload/download checks pass; cross-network validation remains pending |
| Operator deployment | Docker Compose, HTTPS, persistence, secrets, backup/recovery and providers | Compose HTTPS/backup/restore/restart smoke passes. Render HTTPS deployment, persistent key across redeploy, and online backup verification pass; off-machine backup and outgoing provider delivery pending |
| Consumer onboarding | Operated endpoint with no end-user server credentials or provider keys | Pending |
| Distribution | Clean-machine installers for claimed platforms, signatures, uninstall, release artifacts | Native archive packaging configured; Apple Silicon archive runs standalone CLI, relay, SQLite, and bundled UI. Other targets, published installers, signatures, update/uninstall and clean-machine validation pending |
| Open-source readiness | License, dependency review, CI, contribution/security docs, clean source tree | Rust advisory/license/source checks pass; generated notices included and verified in native archive and container. Policy and CI checks configured; full source/release audit pending |
| README | Runnable instructions, real first success, concise diagrams, desktop/mobile rendering | One-command local preview reaches approved download in browser QA; desktop/mobile rendering passes. Hosted install path still pending |

External prerequisites will be recorded when they become relevant. They must not be silently replaced by mock results, local-only demonstrations, or narrower acceptance criteria.

## Evidence from the Rust build

- `cargo test --locked`: 67 tests passed on the development Mac, including real loopback HTTP, SDK MCP negotiation, native MCP configuration, A2A task serialization/routing, independent SQLite-connection races, interrupted downloads, and backup/recovery. These are local tests, not provider or cloud-agent validation.
- `cargo clippy --all-targets -- -D warnings`: passed.
- `scripts/ui-smoke.py`: passed using two isolated Chrome contexts at desktop and phone widths. It verifies phone-account test authentication, mutual contact acceptance, private draft visibility, approval, a byte-verified download, receipt, and revocation. Delivery was explicitly local development mode.
- README source commands replace the removed TypeScript scaffold. Rust is the product implementation; browser assets are bundled static files.
- Filename edge cases retain the original response metadata while saving portable names within filesystem limits. CLI downloads publish atomically and never replace an existing file. Truncated/corrupt HTTP responses create neither a local file nor a receipt.
- Browser fault tests reproduce and cover lost download acknowledgments and background updates arriving during editing. A browser download uses the browser’s native streaming path and requires an explicit receipt; CLI/MCP verifies the complete hash before publishing a local file. In both cases, unsaved edits survive background polling. Stale approvals and late conflicting decisions cannot release a changed or declined response. Fast contact changes cannot replace the selected person’s agents with an older lookup response.
- Slack connection completion can be retried from the original browser after a lost response, without reusing the OAuth exchange, restoring a disabled channel, or replacing a newer destination. This uses local fixtures; real Slack linking remains pending.
- Download headers preserve spaces, literal plus signs, stars, and Unicode through the standard encoded filename field. Native execution, ACP, and agent configuration commands set `PWD` to the selected folder instead of inheriting the launch terminal's folder. The ACP subprocess assertion and a fresh real Codex run pass after this correction.

Still absent: verified published installers, outbound phone delivery, live Slack/email verification, live cloud-provider and remaining ACP adapter validation, Gmail retrieval, GitHub grants, cross-network transfer tests, and full release validation. The hosted endpoint and inbound phone callback are verified below. Codex has live file-operation evidence; successful Claude execution and broader native context/tool compatibility remain unverified. The full requirement set above remains in scope.

## Content and transfer limits

New requests have no built-in file-size, total-transfer, attachment-count, request-text, response-text, command-argument or pending-permission quota. They do not automatically expire. The storage upgrade removes future deadlines from older requests without reviving previously expired access. Storage and filesystem capacity still apply.

HTTP multipart uploads use Axum’s upstream parser; native and cloud transfers stream through encrypted staging into SQLite chunks. Backups still contain one database and its key. Authenticated JSON remains supported for older API clients, though that compatibility format buffers its body. Public sign-in/pairing bodies retain a small 16 KiB abuse-protection bound; authentication rate controls, session expiry, short-lived codes, and renewable worker leases remain.

Downloads have connection/read-inactivity timeouts, with no fixed total transfer duration. Native uploads use a connection timeout and remain cancellable through their worker heartbeat, without a response-header deadline during transfer. ACP initialization remains cancellable through the worker instead of having a 30-second completion deadline. Notification retries continue with backoff; disconnecting the channel stops them. Reopening an older database recovers deliveries abandoned by the former retry cutoff without restoring disconnected destinations. An inbox page contains 100 entries, with an older-page cursor in the browser, CLI and MCP.

Local tests cover requests beyond 100 KB, replies beyond 3 MB, 105 attachments and pending permissions, availability after 40 days, encrypted-chunk tampering, cancelled staging, full-storage rollback and retry, revocation during streaming, legacy attachment migration, and 205-row pagination. These numbers are test cases, not new product limits. A separately invoked test throttles a native multipart upload over 64 seconds while keeping its work lease alive; it passes without sharing the resulting file before approval.

## Setup and worker evidence

- `scripts/onboarding-smoke.py --with-service`: actual terminal pairing, mobile sign-in, channel carry-through, agent detection, folder default, per-user macOS service installation, queue heartbeat, repeat setup, self request, two human approval stages, hash-verified download, and revoked-connection recovery. The temporary service is removed afterwards. Native execution uses an explicit deterministic fixture; no model calls or real SMS.
- Setup measurement for that path: three terminal Enter presses, three browser buttons, three typed fields. Installation, native account login, server availability/address, and code reading are prerequisites outside this count.
- Native process integration tests exercise both Codex and Claude output formats over the real HTTP API. They reject changed agent/folder settings after approval, enforce attachment paths, and keep execution output private until a new immutable sharing decision.
- Real installed Codex and Claude CLI commands were exercised against isolated temporary configuration directories for MCP add/get; user configuration was not changed. These setup checks do not call a model.
- `scripts/mcp-setup-smoke.py` passed with actual OpenCode 1.18.29 and Pi 1.2.0 CLIs in isolated configuration directories. Both paths complete terminal/browser pairing, preserve an unrelated server name, save the new connection, repeat setup without configuration changes, and report the Litecom MCP server connected. No model or worker was started. Other agents' native setup still needs live validation.
- Cursor configuration tests preserve comments, unrelated servers, symlinked dotfiles, and a private backup. Native CLI fixtures reject an apparently successful save when the expected connection is absent. Setup probes the exact saved Litecom executable through the official MCP SDK before adding it to the agent.
- Codex and Claude use the shared collision and retry handling. Codex's structured CLI output and Claude's saved JSON preserve exact argument boundaries; malformed configuration is not treated as an empty configuration. Claude additionally checks the effective user scope and connected status through its native CLI.
- The full setup smoke passed with Codex 0.160.0 and Claude Code 2.1.296: actual terminal/browser pairing, preserved server-name collision, unchanged configuration on retry, disabled connection rejection without creating a duplicate, and malformed configuration preservation. Claude reports a connected MCP server; Codex returns the exact saved command and arguments. These checks use temporary profiles and make no model calls.
- Repeating binary installation preserves an identical executable; the Windows test additionally holds a read-only shared handle to exercise replacement restrictions. Windows service status reads Task Scheduler's numeric running state instead of treating every installed task as running. `scripts/windows-service-smoke.ps1` is configured in CI to check real stopped/running/removed states; its result must be verified separately from the local macOS tests.

## Live native execution

`scripts/live-agent-smoke.py --agent codex` passed with Codex 0.160.0 on macOS using its existing account and normal configuration. The test uses two local browser accounts and an isolated working folder. It verifies:

- No result file exists before **Approve run**.
- The real agent reads the project instructions and creates a file with the expected calculation.
- The reply and file remain private until a separate sharing approval.
- The recipient downloads exactly the expected bytes and acknowledges receipt.

The test passed again after correcting the native launch setting to `approval_policy="never"`. No desktop permission prompt was required. This validates one local file workflow, not all native skills, hooks, tools, or external actions. Phone authentication and notifications used local development delivery.

The same test with Claude Code 2.1.296 reached its configured provider, which returned a spending-limit error. No successful Claude model execution is claimed, and the test did not change providers or bypass the limit. Structured provider errors now appear in the owner's private failure receipt, including errors returned with a nonzero process exit. Requesters retain the generic failure view.

The live OpenCode 1.18.29 attempt failed while initializing its configured tools. A direct OpenCode run independently reported a missing custom tool file in the existing user configuration. Litecom did not alter that configuration or switch accounts to pass the test. Successful live OpenCode execution remains unverified.

## Channel evidence

- Phone and Email terminal/browser setup pass with local delivery. Email verification survives refresh and uses the same phone account. Email adds two controls and two fields to the measured Phone route.
- OTP tests cover role restrictions, session binding, exhausted attempts, replay and notification-channel isolation.
- Slack uses confidential server-side OAuth, with one-use state and final linking bound to the original browser session. Protocol fixtures cover token exchange, rotation, DM creation, posting and updating the original message. Actual Slack app consent and delivery are unverified.
- Notification deliveries commit with task transitions. Outage tests advance the clock by an hour and prove that both channels still receive the review event. Superseded pending changes are coalesced. Live provider callbacks, opt-out and failure visibility still need work.

## Local preview and Devin evidence

- `cargo run --locked -- try` starts a temporary local server, guides pairing, selects the native runner, and opens a sample request. It uses a sample working folder and removes temporary data on normal exit. It does not install a service or change native MCP configuration.
- `scripts/preview-smoke.py` passes from the actual binary through browser pairing, two separate approvals, exact file download, and clean exit. Observed work: one command, two terminal selections, three browser setup buttons and three fields, then two approval buttons and one download. Compiling/installing and signing in to an existing native agent are prerequisites. The runner is a fixture and messages stay local.
- Devin protocol fixtures verify read-only connection checks, one session creation after launch approval, unchanged approvals after a repeated identical connection, invalidation after a changed credential, persisted session polling after SQLite reopen, uncertain launch handling without retries, cancellation after disconnect, private session links, selected attachment collection, idempotent snapshot uploads, and a separate sharing decision.
- Devin signed downloads reject unsupported origins and non-public addresses, pin checked DNS answers, and omit bearer credentials on redirects. Live attachment-host and provider-account validation remain pending.
- The current README is rendered and inspected at 390px and 1100px widths. It has one setup command; persistent server and connection details live in the setup guide.

## Packaging evidence

- `cargo-dist 0.33.0` produces a native Apple Silicon archive and SHA-256 checksum. `scripts/check-package.py` verifies the checksum and runs the extracted CLI and relay in an empty working directory with an empty executable search path. SQLite and the browser pages are bundled; no compiler, Node, or separate database is needed at runtime.
- The **Build downloads** workflow is configured for five native targets. It has not run on GitHub; those platform builds are not claimed as verified. It only uploads workflow artifacts and does not publish a release.
- Running `litecom` without a subcommand now starts guided setup. A hosted default, signed installers, updater, and full uninstall remain release work. Packaging does not change the existing hosted or self-hosted account boundaries.
- The extracted Apple Silicon archive also passes the full browser preview: phone-account pairing, two approvals, exact file download, and cleanup. This uses the deterministic native-agent and delivery fixtures. A separate terminal check verifies the no-argument Phone/Slack/Email picker.
- Per-operation permission tests cover private exact-operation approval, owner/session boundaries, changed digests, cancellation, and lease expiry. The ACP v1 client uses this broker for new native tool-permission requests.

## Dependency and minimum-version evidence

- On 2026-10-15, `cargo-deny` 0.20.2 passed advisories, licenses, and sources against RustSec revision `edb58f5974b83eb8302332821cb6d389777aa351`. No advisory is ignored. The policy covers the five configured release targets, including build and development dependency licenses.
- `cargo-about` 0.9.2 generates the third-party notices from the lockfile. The file contains license text and exact crate-source download links; repeated generation produced identical bytes. `option-ext` 0.2.0 has an explicit MPL-2.0 allowance and an unmodified source link.
- The rebuilt Apple Silicon archive passes checksum, standalone CLI, relay, database, bundled UI, and exact notice-file checks. The rebuilt Linux ARM64 container passes the Compose HTTPS, live backup, restore, restart, and exact notice-file checks. These use local fixtures and send no messages.
- The pinned Rust 1.88 container build passed with the then-current 52-test suite on Linux ARM64. CI declares a minimum-version compile job and dependency-policy/notice-freshness checks. Consult [GitHub Actions](https://github.com/BerriAI/litecom/actions) for current hosted results; local evidence alone does not establish those results.
- This checks Cargo dependencies, not every bundled C library, toolchain runtime, operating-system package, or external agent. Application authorization review and the remaining live integration/release checks are still required.

## ACP evidence

- The official ACP Rust SDK negotiates v1, starts a native agent session with the selected folder and Litecom MCP connection, and streams the final reply. No model, mode, or output-token limit is overridden.
- SDK peer tests cover one-time approval, decline, persistent-only rejection, foreign-session rejection, changed operation scope, cancellation, private reasoning, and a separate sharing decision. Windows and Unix absolute paths are accepted by the relay without interpreting them as its own local paths.
- `scripts/acp-smoke.py` passes actual terminal pairing, mobile review, a deterministic ACP subprocess, approval/refusal, process cancellation, private reasoning, and a byte-verified file download. This is protocol and workflow evidence; it does not validate live Codex/Claude ACP sessions or all native memories, hooks, and skills.

- `scripts/live-agent-smoke.py --agent codex-acp --review-permissions` passed with the released `@agentclientprotocol/codex-acp` 2.2.2 package and a real model call. Test-only read-only mode forced a command permission request. The exact local Python calculation was reviewed, the result file was absent before approval, **Approve once** allowed execution, and the resulting bytes were verified after separate sharing approval. This is one live workflow; it does not establish all default-mode, tool, or native-context behavior.

## Existing-agent adapters

Pi uses the unmodified, published `pi-acp` 0.0.34 package through the shared ACP client. No separate Pi JSON/RPC bridge or Litecom reasoning agent remains. `scripts/acp-smoke.py --agent pi --pi-acp PATH` passed terminal setup, browser run approval, extension confirmation approval/refusal, separate sharing, exact file download, thought exclusion, and subprocess cancellation with a simulated Pi RPC backend. No live model was called.

The reviewed adapter preserves native Pi tools and extensions but does not inject ACP session MCP servers. It forwards extension confirmations, not every tool operation. Litecom declines ambiguous multiple-choice permission requests; free-text extension prompts are unsupported upstream. Upstream can map some Pi failures to ACP `end_turn`, so that status is not independent proof that an external action succeeded. Live provider/error recovery and native context checks remain required before declaring full Pi compatibility.

OpenCode 1.18.29 negotiated ACP v1 and created a session in an isolated configuration without a model call. Cursor, OpenClaw, and Hermes have named ACP presets and native MCP setup implementations; live execution and native CLI validation remain pending. OpenClaw's configuration ownership can require stopping its gateway to add a server. Custom profile environments and gateway attachment access also need validation. See the [agent matrix](agents.md).

## Source checkpoint

Gitleaks 8.30.1 scanned the 80 non-ignored source files with its default rules and inline allow-comments disabled. It reported no findings. This checks for recognizable secrets; it is not a complete security audit. Local screenshots, provider diagnostics, credentials, and build products remain excluded from Git.

## Single approval and message replies

The current native/ACP/Devin request flow uses one approval for execution and automatic delivery of the final reply and selected files. Earlier evidence that mentions two approvals predates this change. Stored execution-only approvals retain their original consent boundary.

- The worker validates its live execution lease before launching a native or ACP process. A malicious prompt, forged preparation work item, cancelled lease, requester credential, or agent credential cannot grant execution authority.
- `cargo test --locked`: 80 passed, one deliberately ignored slow-transfer test. Current cases cover single-approval native and Devin results, legacy consent, signed Twilio callbacks, signed/timestamped Slack buttons, exact linked sender/channel, email reply capabilities, and an IMAP protocol fixture with persistent UID checkpoints and non-destructive reads.
- Message approval from a second channel returns the existing decision. It cannot launch another run or approve a changed proposal. Disconnecting/replacing a channel removes its pending capabilities. New ACP operations need their own exact one-time permission.
- The current preview browser test passes with one approval, automatic result delivery, exact file download and normal cleanup. The ACP subprocess/browser path also passes, including additional tool permission, rejection and cancellation. These use deterministic agents and local delivery.
- A 180-second single-approval soak completed 4,445 rounds, 67,627 HTTP checks, 2,778 exact transfers, 1,110 automatic deliveries, 556 relay restarts, and a real lease-expiry check without a failure. No model calls or provider messages were made.
- Repeating setup while a native fixture is running preserved its process and the real macOS background service. Queue polling and active execution now both update the same agent heartbeat.
- Dependency advisory, license and source checks pass with the new mailbox dependencies. Actual carrier/Slack/mail delivery and live agent runs under the new single-approval flow remain unverified.


## AgentPhone and hosting handoff

AgentPhone replaces the initial Twilio transport. The relay uses one explicitly configured agent and sending number for sign-in codes, complete approval texts, and status updates. Incoming callbacks verify HMAC-SHA256 over the raw body and timestamp, the agent/number, and the connected phone. Reactions, calls, outbound echoes, and group messages cannot approve. Persisted body hashes prevent a repeated callback from replaying a STOP after the user reconnects.

- `cargo test --locked`: 84 passed, one explicitly ignored slow-transfer test. New coverage exercises signed callbacks, changed/expired signatures, sender/agent/number mismatch, iMessage approval, one approval through completion, callback replay across restart, disabled-channel silence, complete UTF-8 messages beyond the former character cutoff, HTTP failures, malformed provider responses, and refusing authenticated redirects.
- `cargo clippy --all-targets --locked -- -D warnings` and dependency advisory/license/source checks pass. Third-party notices were regenerated after removing the Twilio-only SHA-1 dependency.
- The local first-use preview and accessibility checks passed again; the browser editing check passed after updating its obsolete heading selector. See [interface review](ui-review.md).
- The [operator guide](self-host.md) separates service/number/webhook/storage setup from each person's agent connection. It supports a native process or Docker on any suitable host; the product has no Render dependency.
- These automated checks sent no real phone messages. The subsequent hosted checks below verified a real incoming message; outbound delivery still requires carrier registration.

## Latest single-approval checks

- A fresh live Codex 0.160.0 run with the AgentPhone-enabled build passed project-instruction handling, the pre-execution approval gate, a verified local file operation, automatic result delivery, and a second account's exact download and receipt. It used local test identities and development notifications; it did not test carrier delivery.
- The current-build 180-second reliability run passed 4,246 rounds and 64,603 HTTP checks, including 2,654 exact transfers, 1,061 single-approval deliveries, 531 relay restarts, cancellation, stale decisions, revocation, and a real lease expiry. It used deterministic fixtures with no model calls or provider messages.
- A frozen `227a39e` single-approval build completed 30 minutes: 1,987 rounds, 30,242 HTTP checks, 1,241 exact transfers, 496 automatic deliveries, 248 restarts, and a real lease expiry. A separate hour-long run exercised legacy two-stage consent compatibility through 813 rounds and 12,930 checks. Both passed with deterministic fixtures and no provider messages or model calls; neither is a live phone or complete agent-matrix test.

## Hosted checks — October 15, 2026

- The relay is live on Render with 1 CPU, 2 GB RAM, and a 100 GB persistent disk. This deployment uses the same Docker image and relay command as other hosts. Auto-deploy is disabled.
- Version `0.1.0-alpha.3` at commit `d25bad4` responds to the health check. Development mode is off. Unauthenticated account access and unsigned phone callbacks return HTTP 401.
- A real incoming SMS reached the configured AgentPhone sender at 18:27 UTC. Its signed callback succeeded on the first attempt with HTTP 200. This verifies inbound transport and callback authentication, not a complete approval flow.
- The process runs as a non-root user. The relay directory has mode `700` and its encryption key has mode `600`. An online backup verified successfully; the same key and backup survived a deployment to a new instance. The backup is currently on the same disk, so off-machine recovery remains unverified.
- Outgoing SMS is still marked `registration_required` by AgentPhone. Real phone sign-in and approval delivery cannot be claimed until registration is approved and those flows pass. The provider's synthetic webhook test supplies a non-string `numberId` and returns HTTP 400; the real incoming callback has a valid number ID and succeeds. Field-shape diagnostics disclose neither the message nor the sender.

## Cursor authentication checks

The real Cursor CLI `2026.10.14-941492e` negotiated ACP v1 and rejected a new session before authentication. Litecom now performs Cursor's [documented existing-login handshake](https://cursor.com/docs/cli/acp) before opening a session. Background runs disable browser opening; a missing or expired login produces a private instruction to sign in, without starting the requested work.

The SDK regression test covers authentication before session creation, failed authentication, and a missing supported authentication method. The full local suite passes 85 tests with one intentionally ignored slow-transfer test, and Clippy reports no warnings. Separately, the real Cursor CLI enabled a temporary project's Litecom MCP server and discovered its ask, receive, contacts, and download tools. This did not change user-level settings or call a model. Full guided Cursor setup and live model execution remain unverified because the CLI has no signed-in account on the test machine.
