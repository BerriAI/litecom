# Changelog

## 0.1.0-alpha.16

- Optionally enable a hosted Slack agent on the existing approval app. It reads channel context, resolves the recipient, and submits a request for owner approval.
- Show sent and approved status in the originating Slack thread, then post the final response as a new reply.
- Preserve Slack intake across restarts and reuse one request when Slack retries an event.
- Generate one Slack app configuration for hosts. Model settings are optional and stay on the server.
- Use consistent buttons, compact action menus, and quieter list dividers across Workspace, Agents, Inbox, and Settings. Keep secondary actions hidden until opened, with keyboard and mobile support.

## 0.1.0-alpha.14

- Keep local agent runs alive through brief relay outages and retry result delivery without rerunning the agent.
- Request anyone in your workspace without contact acceptance. Existing blocks still apply.
- Wait for results inside the calling agent with MCP.
- Inspect local sessions with `litecom runs` and `litecom logs --follow`.
- Copy a workspace-specific setup prompt for your coding agent.

## 0.1.0-alpha.12

- Choose from installed agents when connecting your computer. Repeated setup keeps your choice.
- Copy an uninstall command from Settings and select the connections to remove. Other connections, agents, and working files stay.
- Revoke removed connections and clean up their background startup and MCP settings. Remove the shared installation when its last connection is removed.

## 0.1.0-alpha.10

- License Litecom under MIT. Third-party components retain their own licenses.
- Show requests and returned files in agent conversations throughout the README.

## 0.1.0-alpha.9

- Include the Litecom logo when building the Docker image.
- Show the request, message approval, and returned file in the README’s animated walkthrough.

## 0.1.0-alpha.8

- Show your email, computer name, and selected coding agent in your workspace. Disconnected agents no longer appear connected.
- Add the Litecom logo to navigation and the README.
- Wait for Windows workers to exit before restarting or removing their background task.

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
