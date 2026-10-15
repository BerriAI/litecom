# Security model

Litecom has not had an independent security review.

CI checks Rust dependencies against the RustSec advisory database and the repository's license/source policy. Release archives and the container include generated third-party notices. These automated checks do not establish that the application, external agents, or container operating system have no vulnerabilities.

## Authority

Google sign-in or a verified phone challenge establishes an account. Browser sessions are separate from device and agent credentials. Browser changes require an authenticated session, matching origin, and a session-bound CSRF token. Direct approvals require a provider-verified sender and an opaque capability bound to the exact task and proposal. SMS uses AgentPhone timestamped HMAC-SHA256 signatures and the verified phone; Slack uses signed callbacks and the linked workspace, user, and DM; email uses the connected sender and a secret reference in the original notification. Connected inboxes and messaging accounts therefore carry approval authority. Agent bearer credentials cannot approve responses, pair other devices, block people, or change account settings.

One relay is one workspace. Members can request any unblocked agent in that workspace; the owner still approves execution and sharing. Requests target a fixed account and agent ID; names are selectors. Each connection can be revoked independently.

## Content

The relay stores encrypted phone numbers, prompts, response bodies, attachment metadata, and attachment bytes using AES-256-GCM with record-bound authenticated data. New attachments use independently authenticated chunks bound to the file ID and chunk position, with a SHA-256 digest for the complete snapshot. Uploads stage encrypted bytes in temporary files and commit the snapshot atomically; existing single-blob attachments remain readable. Litecom authentication tokens are stored as hashes. Provider credentials that must be reused, including Slack and Devin tokens, are encrypted. Phone uniqueness uses a keyed fingerprint. The storage key is a separate file; both the key and database are needed for recovery.

The relay operator can decrypt payloads. This design does not provide end-to-end encryption. TLS protects non-local connections. Clients refuse redirects for authenticated API calls.

Approval binds an immutable snapshot and action to the task, recipient, owner, agent, version, and digest. No draft is available before approval. A request approval authorizes execution and automatic delivery of its final reply and selected files. It does not claim those future bytes were individually reviewed. Manually prepared replies still bind approval to their exact snapshot. Download URLs require authentication. Responses are served as attachments, not executable browser content.

Revocation prevents future access. It cannot recall copies already received. New requests and downloads have no automatic expiry. Litecom does not automatically delete stored payloads or operator backups. Include their storage and access in your retention policy.

## Backups and recovery

The backup command snapshots SQLite and copies its matching storage key. The complete backup can decrypt stored content and provider credentials; protect it like the live data. SHA-256 checksums, SQLite integrity checks, and an authenticated key check detect an incomplete or mismatched snapshot, but do not authenticate a backup obtained from an untrusted source.

Restore writes only into a new folder. It revokes existing sessions and download access, preserves saved blocks, removes channel/cloud connections, rotates old invite links, and prevents pending actions from resuming. This prevents a stale snapshot from restoring previously revoked authority or replaying an action. People must reconnect after recovery. A partial restore cannot start while its marker remains. Restore does not stop external sessions or reverse effects that occurred after the snapshot. See [the recovery procedure](docs/backup.md).

## Execution

A work lease belongs to one agent connection. Preparation can retry after a lost lease. An execution whose outcome is unknown is recorded as uncertain and is not automatically repeated. Cancelling a running operation is a request to stop, not proof that its effects were reversed.

Raw execution output remains private to the owner. Public receipts expose a generic outcome, not arbitrary tool output or private URLs. Native Codex/Claude runs require one human approval before launch and automatically deliver the final result under that approval. The worker rechecks its live execution lease immediately before starting the agent. Preparation constructs the proposal without launching the model. Litecom uses Codex workspace-write shell mode with additional permission prompts denied, or Claude’s saved permissions with prompts denied. Native hooks and MCP tools retain their own access; Litecom does not sandbox those tools. The displayed launch scope includes that access. Attachment paths must resolve inside the selected working folder. Process groups are stopped on cancellation, but detached external jobs and completed side effects cannot be undone.

ACP connections forward new tool-permission requests to the owner. Decisions bind the exact operation to its session, task, credential and live lease. Litecom selects only a one-time permission; missing scope, persistent-only choices, changed operations and foreign sessions are rejected. The agent's saved permissions still apply: tools and hooks that do not ask ACP for permission are outside this mediation. The initial run approval covers that access. See [agent compatibility](docs/agents.md) for connection limitations.

Devin uses the same single request approval for launch and result delivery. The relay persists a job before starting a session, resumes known sessions after restart, and does not repeat an uncertain launch. Provider session links and unshared output remain private to the owner. Disconnecting requests termination; pending cancellation is not reported as confirmed until the provider accepts it. A running job retains an encrypted credential snapshot for monitoring/termination; that copy is removed when the job ends. Signed attachment redirects never receive the Devin bearer token and use checked, pinned public DNS addresses.

## Optional Slack agent

The optional hosted Slack agent receives signed `app_mention` events through the same app used for approval DMs. It maps the sender's workspace and user IDs to a verified Litecom Slack connection. Its tools can read the source channel, list linked people, and create a request. They cannot approve, execute commands, read local files, or change accounts. The server fixes the sender and reply destination independently of model output, restricts recipients to linked members of that Slack workspace, and rechecks blocks and workspace access.

The host's model provider receives the selected Slack context and directory information. Events and model progress are encrypted in the relay database. The owner sees the proposed work, original message, gathered context, and destination before approving. Approval also authorizes posting the final reply to everyone in that Slack channel; downloads remain restricted to the requester and owner. Final results bypass the hosted model. Already-posted Slack content cannot be recalled by revoking a Litecom download.

The event ID binds retries to one request. Model output cannot supply approval authority, a different sender, or another reply channel. Restoring a backup discards pending Slack jobs so stale events cannot submit work again. See [optional setup](docs/slack-agent.md).

## Development delivery

`--dev` is allowed only with loopback binding and a loopback public URL. It prints test codes and notifications to the local terminal. It must never be used as an internet-facing authentication service.

## Reporting a vulnerability

Do not post credentials, private files, phone numbers, or live exploit details in public issues. Contact a repository maintainer to arrange a private report before sharing exploit details.

## Google sign-in

Google sign-in uses `openidconnect` for authorization-code exchange with PKCE and RS256 ID-token verification. One-use state is encrypted, expires after ten minutes, and is bound to an HttpOnly browser cookie. Signature, issuer, audience, authorized party, expiry, nonce, verified email, and configured Workspace domains are checked. The Google subject is the identity key; matching email addresses never merge accounts. Workspace domain policy is rechecked for existing human and agent credentials.

The transitive `rsa` dependency has [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071.html), an unpatched private-key timing disclosure. Production Litecom performs public-key signature verification only; it does not load an RSA private key, sign RSA tokens, or decrypt RSA ciphertext. The generated public test key is used only in `#[cfg(test)]` code. `deny.toml` records this specific non-reachable advisory exception. Reassess it if RSA private-key operations are ever introduced.

The workspace directory exposes member names and agent names to signed-in members. It does not expose phone numbers, email addresses, private configuration, or invitations. Configure `GOOGLE_ALLOWED_DOMAINS` when the directory should be limited to a Google Workspace organization. Local connection commands contain a short-lived, one-use agent-pairing credential; keep them private and never include them in logs or screenshots shared with others.

Local run logs include agent and tool output and remain on the receiving computer. They are not attached to the shared reply. See [monitoring and cleanup](docs/local-agent.md#monitor-local-runs).
