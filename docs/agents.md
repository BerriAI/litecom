# Connect your existing agent

Litecom routes requests to the agents you already use. Each agent supplies its own model, login, context, and tools. Litecom does not maintain a reasoning agent.

Run `litecom setup` to choose a detected local agent. For Pi, [install pi-acp and choose Pi](acp.md#pi). For a cloud agent, [connect Devin in the browser](cloud-agents.md).

| Agent | Connection and requirements |
| --- | --- |
| Codex | Native CLI or [Codex ACP](acp.md#codex) for additional tool approvals in Litecom. Native setup adds MCP tools. |
| Claude Code | Native CLI or `claude-agent-acp`. Native setup adds MCP tools. |
| OpenCode | `opencode acp`. Setup adds MCP tools to ordinary and receiving sessions; checked with 1.18.29. |
| Cursor | Signed-in Cursor CLI: `cursor-agent acp` or `agent acp`. Setup saves and enables MCP tools. Interactive question/plan extensions are not implemented. |
| Pi | Node.js 22+, signed-in Pi, and `pi-acp` 0.0.34. Automatic MCP setup checked with Pi 1.2.0. See [Pi limitations](acp.md#pi). |
| OpenClaw | `openclaw acp` with a configured gateway. Setup uses its native MCP command; a running gateway may need stopping first. |
| Hermes | `hermes acp` with its ACP dependencies and configured gateway. Setup saves native MCP tools; session MCP needs gateway support. |
| Devin | Cloud API with an existing account and API access. The relay runs and monitors requests. |

MCP setup has been exercised with real Codex, Claude Code, OpenCode, and Pi CLIs. Before the single-approval change, a live Codex 0.160.0 run read project instructions, created a file, and delivered its exact bytes after two separate approvals. Codex ACP 2.2.2 also passed with a one-time tool permission approved through Litecom. Other live model execution remains unverified. Pi's adapter browser flow uses simulated Pi RPC; Cursor, OpenClaw, Hermes, and Devin currently have fixture evidence. Broader context, skills, hooks, and tool compatibility still need validation. See [release evidence](release-readiness.md).

Start a new session in your agent after setup to load Litecom's tools. You can then ask it to contact someone or check received results. Setup checks the Litecom command and reads back the saved connection; it preserves unrelated MCP servers. `--no-mcp` skips adding tools.

All local adapters currently collect attachments from the selected local folder. A remote OpenClaw/Hermes gateway needs those files available locally; remote attachment fetching is not implemented.

To use an installed agent not detected by the picker, use its [ACP command](acp.md#other-acp-agents). If no local agent is available, setup preserves your phone account and explains the missing requirement. You can still send requests with `litecom ask` or connect Devin.
