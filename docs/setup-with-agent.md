# Set up Litecom with your coding agent

Paste this into your agent. You need access to your hosting environment, a domain, Google Auth Platform, and one messaging provider. Your agent can prepare the server and configuration; provider sign-in, consent, and phone carrier registration may need you.

```text
Set up Litecom using https://github.com/BerriAI/litecom/blob/main/docs/self-host.md and https://github.com/BerriAI/litecom/blob/main/docs/hosting-channels.md. Inspect my existing deployment first and reuse it if it works. Otherwise run one Litecom server with persistent storage and HTTPS, using Docker Compose or my existing host. Connect Google sign-in and ask where I want approvals: Slack, phone, or email. Enable only channels that can both deliver requests and receive decisions. Reuse authorized provider configuration and preserve existing data, credentials, and agent settings. Ask only for missing choices or provider steps that need me. Never print secrets. Verify sign-in, channel connection, one approved request, and its delivered result before declaring it ready. Give me the workspace URL, a backup plan, and any remaining steps. End users should join through the browser and connect their existing coding agent with the generated command; they should not need Rust or a source checkout.
```

## What to expect

1. **Host:** your agent sets up one server, persistent storage, and HTTPS.
2. **Sign-in:** you provide or create a Google web OAuth client with the exact callback URL.
3. **Approvals:** you connect one provider. Slack is a good starting point if your team already uses it. Phone needs an outbound-enabled number and carrier registration; email needs SMTP and an IMAP reply mailbox.
4. **First request:** sign in, connect your approval channel and agent, then approve a small request. The result should arrive in Litecom.

Your host needs no model account. Requests use each person's connected agent and its existing billing. Invite your team after the first request works; their route is [Google → approval channel → workspace](setup.md).

[Manual hosting guide](self-host.md) · [Approval provider setup](hosting-channels.md)
