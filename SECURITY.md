# Security model

Litecom has not had an independent security review. [Release evidence](docs/release-readiness.md) records the checks completed and outstanding.

CI checks Rust dependencies against the RustSec advisory database and the repository's license/source policy. Release archives and the container include generated third-party notices. These automated checks do not establish that the application, external agents, or container operating system have no vulnerabilities.

## Authority

A verified phone challenge establishes an account. Browser sessions are separate from device and agent credentials. Browser changes require an authenticated session, matching origin, and a session-bound CSRF token. Direct approvals require a provider-verified sender and an opaque capability bound to the exact task and proposal. SMS uses Twilio signatures and the verified phone; Slack uses signed callbacks and the linked workspace, user, and DM; email uses the connected sender and a secret reference in the original notification. Connected inboxes and messaging accounts therefore carry approval authority. Agent bearer credentials cannot approve responses, pair other devices, add contacts, or change account settings.

Contact acceptance is mutual. Requests target a fixed account and agent ID; names are selectors. Each connection can be revoked independently.

## Content

The relay stores encrypted phone numbers, prompts, response bodies, attachment metadata, and attachment bytes using AES-256-GCM with record-bound authenticated data. New attachments use independently authenticated chunks bound to the file ID and chunk position, with a SHA-256 digest for the complete snapshot. Uploads stage encrypted bytes in temporary files and commit the snapshot atomically; existing single-blob attachments remain readable. Litecom authentication tokens are stored as hashes. Provider credentials that must be reused, including Slack and Devin tokens, are encrypted. Phone uniqueness uses a keyed fingerprint. The storage key is a separate file; both the key and database are needed for recovery.

The relay operator can decrypt payloads. This design does not provide end-to-end encryption. TLS protects non-local connections. Clients refuse redirects for authenticated API calls.

Approval binds an immutable snapshot and action to the task, recipient, owner, agent, version, and digest. No draft is available before approval. A request approval authorizes execution and automatic delivery of its final reply and selected files. It does not claim those future bytes were individually reviewed. Manually prepared replies still bind approval to their exact snapshot. Download URLs require authentication. Responses are served as attachments, not executable browser content.

Revocation prevents future access. It cannot recall copies already received. New requests and downloads have no automatic expiry. The storage upgrade removes future deadlines from older requests without restoring already expired access. Database retention and operator backup deletion remain release work.

## Backups and recovery

The backup command snapshots SQLite and copies its matching storage key. The complete backup can decrypt stored content and provider credentials; protect it like the live data. SHA-256 checksums, SQLite integrity checks, and an authenticated key check detect an incomplete or mismatched snapshot, but do not authenticate a backup obtained from an untrusted source.

Restore writes only into a new folder. It revokes existing sessions and download access, removes contacts and channel/cloud connections, rotates contact links, and prevents pending actions from resuming. This prevents a stale snapshot from restoring previously revoked authority or replaying an action. People must reconnect after recovery. A partial restore cannot start while its marker remains. Restore does not stop external sessions or reverse effects that occurred after the snapshot. See [the recovery procedure](docs/backup.md).

## Execution

A work lease belongs to one agent connection. Preparation can retry after a lost lease. An execution whose outcome is unknown is recorded as uncertain and is not automatically repeated. Cancelling a running operation is a request to stop, not proof that its effects were reversed.

Raw execution output remains private to the owner. Public receipts expose a generic outcome, not arbitrary tool output or private URLs. Native Codex/Claude runs require one human approval before launch and automatically deliver the final result under that approval. The worker rechecks its live execution lease immediately before starting the agent. Preparation constructs the proposal without launching the model. Litecom uses Codex workspace-write shell mode with additional permission prompts denied, or Claude’s saved permissions with prompts denied. Native hooks and MCP tools retain their own access; Litecom does not sandbox those tools. The displayed launch scope includes that access. Attachment paths must resolve inside the selected working folder. Process groups are stopped on cancellation, but detached external jobs and completed side effects cannot be undone.

ACP connections forward new tool-permission requests to the owner. Decisions bind the exact operation to its session, task, credential and live lease. Litecom selects only a one-time permission; missing scope, persistent-only choices, changed operations and foreign sessions are rejected. The agent's saved permissions still apply: tools and hooks that do not ask ACP for permission are outside this mediation. The initial run approval covers that access. Live adapter behavior and broader scope enforcement remain release work.

Devin uses the same single request approval for launch and result delivery. The relay persists a job before starting a session, resumes known sessions after restart, and does not repeat an uncertain launch. Provider session links and unshared output remain private to the owner. Disconnecting requests termination; pending cancellation is not reported as confirmed until the provider accepts it. A running job retains an encrypted credential snapshot for monitoring/termination; that copy is removed when the job ends. Signed attachment redirects never receive the Devin bearer token and use checked, pinned public DNS addresses.

## Development delivery

`--dev` is allowed only with loopback binding and a loopback public URL. It prints test codes and notifications to the local terminal. It must never be used as an internet-facing authentication service.

## Reporting a vulnerability

Do not post credentials, private files, phone numbers, or live exploit details in public issues. Private vulnerability reporting and a maintainer contact must be configured when the GitHub repository is published.
