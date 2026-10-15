# Changelog

## 0.1.0-alpha.7

- Stop the existing Windows background task before replacing its configuration during repeated setup.

## 0.1.0-alpha.6

- Start the Linux background worker once per command so repeated starts do not exhaust systemd's restart allowance.

## 0.1.0-alpha.5

- Sign in with Google, choose an approval channel, and find contacts in your workspace.
- Connect an installed coding agent with a command from the browser. Setup adds MCP tools and starts the background worker.
- Connect and reconnect a named local agent without changing another connection.
- Approve requests from Slack messages with one decision for execution and result delivery.
- Preserve native MCP configuration during setup, including disabled connections and malformed configuration.
- Keep Windows background workers running on battery power without a fixed runtime cutoff.

## 0.1.0-alpha.3

- Sign-in texts acknowledge the requested message and include opt-out instructions.
- MCP reports the installed package version automatically.
- Bundled notices use the correct release version and source link.
- Hosting instructions explain carrier registration and container startup requirements.

## 0.1.0-alpha.2

- AgentPhone handles phone sign-in, approval texts, and status messages.
- Signed incoming replies bind approvals to the connected phone, configured agent, and sending number; callback receipts survive restart.
- Group messages, reactions, voice transcripts, and outbound echoes cannot approve.
- Shared-server setup and individual onboarding have separate instructions.


## 0.1.0-alpha.1

- One phone account for contacts and local or cloud agents.
- One request approval before execution and automatic result delivery.
- Text replies, Slack buttons, and email replies for configured approval channels.
- Native/ACP background workers, MCP tools, A2A tasks, and resumable Devin sessions.
- Encrypted file snapshots, authenticated downloads, cancellation, and revocation.
- Native archives and shell/PowerShell installers built with cargo-dist.


See [setup](docs/setup.md), [installation](docs/distribution.md), and [agent compatibility](docs/agents.md).
