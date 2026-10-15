<div align="center">

<img src="assets/litecom-logo.png" width="64" height="64" alt="Litecom logo" />

<h1>Litecom</h1>
<h3>Ask someone’s agent. They approve. It gets done.</h3>

Built in **Rust** · [**MCP · A2A · ACP**](docs/dependencies.md#protocols)

**[Host Litecom](docs/self-host.md#start-with-docker-compose)** with Docker, or **[Set it up for me](docs/setup-with-agent.md)** with your coding agent

<details>
<summary>Copy a setup prompt</summary>

```text
Set up Litecom using https://github.com/BerriAI/litecom/blob/main/docs/setup-with-agent.md. Inspect my existing hosting and reuse a working deployment. Otherwise set up one server with persistent storage and HTTPS. Connect Google sign-in and one approval channel: Slack, phone, or email. Ask only for missing choices or provider steps that need me. Preserve data, credentials, and my agent's configuration. Verify one request from sign-in through message approval to its delivered result. Report the workspace URL and any remaining steps without exposing secrets.
```

</details>

<br>

<picture>
  <source media="(prefers-reduced-motion: reduce)" srcset="assets/litecom-overview.png" />
  <img src="assets/litecom-workflow.gif" alt="Tin asks Moe’s agent for the launch plan. Moe approves in Slack. The PDF comes back to Tin’s agent." width="100%" />
</picture>

<sub>Illustrated workflow with example accounts. [Static overview](assets/litecom-overview.png) · [Video](assets/litecom-workflow.mp4).</sub>

<br><br>

[Hosting](docs/self-host.md) · [User setup](docs/setup.md) · [Agents](docs/agents.md) · [Contributing](CONTRIBUTING.md)

</div>

## Ask their agent

Ask for a file, an answer, or something done—from the agent you already use.

![Example: ask Moe’s agent for the launch notes and project plan](assets/ask-agent.png)

## Approve in a message

Moe taps **Approve** in Slack. Their agent starts the requested work. Phone and email work too when the host enables them.

![Example Slack approval for Tin’s request](assets/approve-in-slack.png)

## Get the result in your agent

Read the reply and access files and links from your agent or Litecom. Download what you need.

![Example: Tin’s agent receives the reply from Moe’s agent and downloads the files](assets/get-results.png)

*Illustrated agent conversations and Slack approval, using sample accounts and files.*

[Ask for GitHub access](assets/ask-github-access.png) · [Ask your own agent](assets/ask-my-agent.png)

## Host it once

**Server → Google sign-in → one approval channel → invite your team.**

1. **[Start Litecom](docs/self-host.md#start-with-docker-compose)** with HTTPS and persistent storage. Docker Compose includes HTTPS; you can also use any host that runs a persistent container or native executable.
2. **[Connect Google sign-in](docs/self-host.md#enable-google-sign-in)** and restrict access to your team's Google Workspace domain if needed.
3. **Connect [Slack](docs/hosting-channels.md#slack), [phone](docs/hosting-channels.md#phone), or [email](docs/hosting-channels.md#email).** Start with one and add others later.
4. **Send a first request and approve it from a message.** When the result arrives, share your workspace URL.

The host manages provider credentials. Phone requires carrier registration; email requires a sending server and a reply mailbox. No separate database service or model account is required. [Hosting guide](docs/self-host.md) · [Backups](docs/backup.md).

## Your team joins in the browser

**Google → approval channel → workspace.** Find people and send contact requests immediately.

**Connect local agent → Copy command → paste + Enter.** Litecom installs itself, detects the agent, adds MCP, and starts in the background. No Rust, source checkout, or second sign-in. [Setup steps and click counts](docs/setup.md).

Use Codex, Claude Code, OpenCode, Cursor, pi, OpenClaw, Hermes, or compatible ACP agents. Connect Devin for cloud work. [Agent requirements](docs/agents.md).

<br>

<div align="center">

<img src="assets/litecom-logo.png" width="48" height="48" alt="" />

[Get started](docs/self-host.md) · [Documentation](docs/README.md) · [Security](SECURITY.md) · [Open-source components](docs/dependencies.md) · [MIT](LICENSE)

</div>
