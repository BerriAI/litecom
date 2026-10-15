# Connect your existing agent

Litecom routes requests to the agents you already use. Each agent supplies its own model, login, context, and tools. Litecom does not maintain a reasoning agent.

Run `litecom setup` to choose a detected local agent. For Pi, [install pi-acp and choose Pi](acp.md#pi). For a cloud agent, [connect Devin in the browser](cloud-agents.md).

| Agent | Connection and requirements |
| --- | --- |
| Codex | Native CLI or [Codex ACP](acp.md#codex) for additional tool approvals in Litecom. Native setup adds MCP tools. |
| Claude Code | Native CLI or `claude-agent-acp`. Native setup adds MCP tools. |
| OpenCode | `opencode acp`. Setup adds MCP tools to ordinary and receiving sessions. |
| Cursor | Signed-in Cursor CLI: `cursor-agent acp` or `agent acp`. Setup saves and enables MCP tools; receiving sessions reuse the existing login. Interactive question/plan extensions are not implemented. |
| Pi | Node.js 22+, signed-in Pi, and `pi-acp` 0.0.34. See [Pi limitations](acp.md#pi). |
| OpenClaw | `openclaw acp` with a configured gateway. You may need to stop the gateway before adding the MCP connection. |
| Hermes | `hermes acp` with its ACP dependencies and configured gateway. Session MCP needs gateway support. |
| Devin | Cloud API with an existing account and API access. The relay runs and monitors requests. |

Codex and Codex ACP support verified file-request workflows. Treat the other integrations as experimental: send a small self-request to check your login, tools, and file access before using them for important work. Your agent's provider charges apply.

Start a new session in your agent after setup to load Litecom's tools. You can then ask it to contact someone or check received results. Setup checks the Litecom command and reads back the saved connection; it preserves unrelated MCP servers. `--no-mcp` skips adding tools.

All local adapters currently collect attachments from the selected local folder. A remote OpenClaw/Hermes gateway needs those files available locally; remote attachment fetching is not implemented.

To use an installed agent not detected by the picker, use its [ACP command](acp.md#other-acp-agents). If no local agent is available, setup preserves your Litecom account and explains the missing requirement. You can still send requests with `litecom ask` or connect Devin.
