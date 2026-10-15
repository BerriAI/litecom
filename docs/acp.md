# Connect an ACP agent

Litecom supports **Agent Client Protocol (ACP) v1** through its official Rust SDK. ACP connects Litecom to a local agent process. The agent keeps its own tools and configuration; Litecom handles human decisions and delivery.

## Set up

Install and sign in to your preferred agent first. Litecom detects OpenCode, Cursor CLI, OpenClaw, Hermes, and installed Codex/Claude/Pi ACP adapters. [Agent requirements and compatibility →](agents.md)

### Codex

To receive Codex's additional permission requests in Litecom, install its maintained ACP adapter and select it:

```sh
npm install -g @agentclientprotocol/codex-acp@2.2.2
litecom setup --agent codex-acp
```

The adapter includes a compatible Codex dependency and uses your existing login and configuration. Its selected mode determines which operations need permission; Litecom forwards the requests it emits. Direct `--agent codex` uses the local CLI and denies additional permissions instead of forwarding them.

### Pi

Pi uses the external [pi-acp adapter](https://github.com/svkozak/pi-acp). With Node.js 22+ and a signed-in Pi installation on your PATH:

```sh
npm install -g pi-acp@0.0.34
litecom setup --agent pi
```

Choose your approval channel, connect your Litecom account, and confirm a working folder. Setup adds Litecom's tools through `pi mcp add`. Litecom starts the adapter; the adapter starts your existing Pi in RPC mode. Pi keeps its model configuration, skills, extensions, and tools. No separate Litecom model or API key is needed.

Pi's ordinary tools keep their native access. Extension confirmation prompts are forwarded for one-time approval. Multiple-choice extension prompts are declined because Litecom's approval controls cannot select between their meanings. Free-text extension prompts are unsupported by this adapter. `pi-acp` 0.0.34 does not apply ACP session MCP settings, so Litecom uses Pi's native MCP configuration.

### Other ACP agents

For another installed ACP agent:

```sh
litecom setup --acp /absolute/path/to/your-agent
```

Replace the path with that agent's executable. Add `--acp-arg=VALUE` for each required argument. Choose your working folder when prompted. Your Litecom connection and agent selection are saved; you do not need to keep this terminal open after background startup succeeds.

Litecom supplies its MCP tools to session-capable adapters automatically. Named Pi, Cursor, OpenCode, OpenClaw, and Hermes connections also save tools in the agent's native settings during setup. For a custom ACP executable, use `litecom mcp-config` if you also want tools in its ordinary sessions. `--no-mcp` skips adding tools and disables session injection; it does not remove an existing native connection. Direct Codex and Claude Code connections remain available alongside their ACP adapters.

The maintained adapter projects are [Codex ACP](https://github.com/agentclientprotocol/codex-acp) and [Claude Agent ACP](https://github.com/agentclientprotocol/claude-agent-acp). Sign in to the underlying provider before connecting an adapter. Support for native memory, hooks, and tools depends on that adapter.

## Handle a request

1. **Approve** authorizes the request and its result delivery, then starts the selected agent in your working folder.
2. If the agent asks for tool permission, review the proposed operation and select **Approve once** or **Decline operation**. Each additional permission is a separate decision.
3. The final reply and files are sent automatically. No second sharing approval is needed.

Litecom never chooses an ACP “always allow” option. A request without a one-time choice, with insufficient operation details, or for another session is declined. An operation changed while awaiting approval stops the run. Canceling a request stops the local agent process; effects already completed are not undone.

The selected agent's existing permissions and hooks still apply. Some tools can run without asking ACP for permission. ACP itself is not a sandbox, and Litecom does not claim to intercept those tools. The initial run approval describes that access. Only the agent's final reply and selected files are staged for sharing; its internal reasoning is excluded.

## If the agent needs attention

If the adapter reports that authentication is required, sign in using that agent's supported login flow and send a new request. Litecom does not collect its provider password. A disconnected or interrupted run is not automatically repeated.

An agent must return the requested reply and attachment list. Malformed output leaves a private diagnostic file and is not published. Attachments must be regular files inside the selected working folder, without escaping through symlinks.

See [agent compatibility](agents.md) before choosing an adapter.
