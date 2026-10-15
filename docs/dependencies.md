# Open-source components

Litecom owns account and contact mapping, request routing, human approval, and controlled delivery. Existing agents supply reasoning and use their own tools, context, and accounts.

| Project | Use in Litecom |
| --- | --- |
| [MCP Rust SDK (`rmcp`)](https://github.com/modelcontextprotocol/rust-sdk) | Expose Litecom tools and check the installed MCP connection. |
| [ACP Rust SDK](https://github.com/agentclientprotocol/rust-sdk) | Run existing agents and forward their permission requests. |
| [a2a-rust](https://github.com/tomtom215/a2a-rust) | Community `a2a-protocol-types` library for A2A messages and tasks; not an official A2A SDK. |
| [Codex ACP](https://github.com/agentclientprotocol/codex-acp) | Optional external adapter for Codex permission requests; version 2.2.2 exercised with a live agent. |
| [pi-acp](https://github.com/svkozak/pi-acp) | External Pi adapter, used without a Litecom fork. |
| [Tokio](https://github.com/tokio-rs/tokio) and [Axum](https://github.com/tokio-rs/axum) | Background runtime and HTTP server. |
| [SQLite through rusqlite](https://github.com/rusqlite/rusqlite) | Transactional state and database snapshots. |
| [async-imap](https://github.com/async-email/async-imap) and [mail-parser](https://github.com/stalwartlabs/mail-parser) | Receive approval replies through standard IMAP and parse MIME email. |
| [rustls](https://github.com/rustls/rustls) | TLS for the receiving mailbox and HTTP/SMTP transports. |
| [Geist](https://github.com/vercel/geist-font) | Bundled UI typefaces, under the SIL Open Font License. |
| [Lettre](https://github.com/lettre/lettre) | SMTP email delivery. |
| [RustCrypto](https://github.com/RustCrypto/AEADs) | AES-GCM encryption; related RustCrypto crates provide hashes and HMAC. |
| [Dialoguer](https://github.com/console-rs/dialoguer) | Terminal setup prompts. |
| [jsonc-parser](https://github.com/dprint/jsonc-parser) | Preserve comments and formatting when adding Cursor's MCP connection. |
| [Caddy](https://github.com/caddyserver/caddy) | HTTPS in the optional self-hosted Compose deployment. |
| [cargo-dist](https://github.com/axodotdev/cargo-dist) | Build download archives and installer configuration. Published, signed installers remain release work. |
| [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) and [cargo-about](https://github.com/EmbarkStudios/cargo-about) | Check dependency advisories, licenses, and sources; generate release notices. Development tools only. |

[Cargo.toml](../Cargo.toml) lists Rust dependencies; [Cargo.lock](../Cargo.lock) pins resolved versions. External agent adapters are installed separately; see [agent requirements and validation](agents.md).

[Third-party notices](../THIRD-PARTY-NOTICES.html) contain license text and links to the exact dependency sources. [Maintainer checks](../CONTRIBUTING.md#dependency-checks-and-notices) explain how to regenerate them.

AgentPhone, Slack, and Devin are external service integrations, not open-source dependencies. Litecom does not include a separate reasoning agent or service-specific GitHub action layer.
