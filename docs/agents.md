# Connect your existing agent

Litecom routes requests to the agents you already use. Each agent supplies its own model, login, context, and tools. Hosts can also enable an optional [Slack agent](slack-agent.md) to turn mentions into approval requests.

Run `litecom setup` to choose a detected local agent. For Pi, [install pi-acp and choose Pi](acp.md#pi). For a cloud agent, [connect Devin in the browser](cloud-agents.md).

| Agent | Connection and requirements |
| --- | --- |
| Codex | Native CLI through Codex app-server, or [Codex ACP](acp.md#codex). Native setup adds MCP tools. |
| Claude Code | Native CLI or `claude-agent-acp`. Native setup adds MCP tools. |
| OpenCode | `opencode acp`. Setup adds MCP tools to ordinary and receiving sessions. |
| Cursor | Signed-in Cursor CLI: `cursor-agent acp` or `agent acp`. Setup saves and enables MCP tools; receiving sessions reuse the existing login. Interactive question/plan extensions are not implemented. |
| Pi | Node.js 22+, signed-in Pi, and `pi-acp` 0.0.34. See [Pi limitations](acp.md#pi). |
| OpenClaw | `openclaw acp` with a configured gateway. You may need to stop the gateway before adding the MCP connection. |
| Hermes | `hermes acp` with its ACP dependencies and configured gateway. Session MCP needs gateway support. |
| Devin | Cloud API with an existing account and API access. The relay runs and monitors requests. |

Codex and Codex ACP support verified file-request workflows. Treat the other integrations as experimental: send a small self-request to check your login, tools, and file access before using them for important work. Your agent's provider charges apply.

Start a new chat in the agent you connected to load Litecom's tools. Setup shows its launch command and a first request to try. You can ask another person's agent and receive the result in that conversation. Standalone ACP adapters receive MCP in their Litecom sessions; setup shows how to add it to another chat separately. `--no-mcp` skips adding tools.

After sending a request, your agent checks briefly. If it is still pending, it gives you the request link and returns control; human approval can happen later. Ask your agent to check that request when you want an update. Scheduled check-ins require support in your agent host; Litecom's tools do not wake a stopped chat automatically.

All local adapters currently collect attachments from the selected local folder. A remote OpenClaw/Hermes gateway needs those files available locally; remote attachment fetching is not implemented.

To use an installed agent not detected by the picker, use its [ACP command](acp.md#other-acp-agents). If no local agent is available, setup preserves your Litecom account and explains the missing requirement. You can still send requests with `litecom ask` or connect Devin.

## Read-only requests

Native Codex and Claude Code support **Approve read-only** for local files, GitHub and Slack. Use an updated Litecom worker and native CLI; verified versions are Codex 0.160.0 and Claude Code 2.1.296. Claude requires `--restricted` support (2.1.248 or later). If the native CLI cannot apply the restrictions, the request stops instead of falling back to full access.

Your existing native login stays in place. GitHub reads use your installed, signed-in `gh` CLI. Slack reads use your Litecom Slack connection and channels that both you and the app can access. Other tools are unavailable in this mode. [Read access and approval](approval-channels.md) explains how to request actions when needed.

Computer administrators and native runtimes remain trusted. Claude's managed hooks follow administrator policy; see [the read-only boundary](../SECURITY.md#read-only-approval). ACP and cloud agents currently require action approval.

## Browser and desktop tools

Litecom starts a background CLI or ACP session. It uses the configuration available to that process; tools provided only inside a desktop application do not automatically carry over. Browser or desktop requests need those tools configured in the receiving agent.

**Approve actions** authorizes the agent’s tool calls for that run. Litecom answers Codex and ACP permission checks automatically; Claude Code runs with permission checks bypassed for that process. It does not change your global agent settings.

On macOS, setup requests folder, screen-recording, and accessibility access from the background worker. Allow the system prompts and enable any access shown as missing in System Settings. Computer-control tools must already be available to your agent. Other applications may still require their own permissions or sign-in; setup cannot grant access on their behalf.

If you skipped those prompts, run `litecom approve` to check your existing connections again.
