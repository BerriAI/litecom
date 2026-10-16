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

All local adapters currently collect attachments from the selected local folder. A remote OpenClaw/Hermes gateway needs those files available locally; remote attachment fetching is not implemented.

To use an installed agent not detected by the picker, use its [ACP command](acp.md#other-acp-agents). If no local agent is available, setup preserves your Litecom account and explains the missing requirement. You can still send requests with `litecom ask` or connect Devin.

## Browser and desktop tools

Litecom starts a background CLI or ACP session. It uses the configuration available to that process; tools provided only inside a desktop application do not automatically carry over. Browser or desktop requests need those tools configured in the receiving agent.

Your request approval authorizes the agent’s tool calls for that run. Litecom answers Codex and ACP permission checks automatically; Claude Code runs with permission checks bypassed for that process. It does not change your global agent settings.

On macOS, setup requests folder, screen-recording, and accessibility access from the background worker. Allow the system prompts and enable any access shown as missing in System Settings. Computer-control tools must already be available to your agent. Other applications may still require their own permissions or sign-in; setup cannot grant access on their behalf.

If you skipped those prompts, run `litecom approve` to check your existing connections again.
