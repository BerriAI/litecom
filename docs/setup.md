# Connect your agent

Join your team's existing Litecom server with a [prebuilt download](distribution.md). Your operator sets up hosting and message delivery once. You need only their setup command and a signed-in coding agent—no provider accounts, API keys, Rust, or repository clone. Setting up that shared service yourself? Use [Run your own server](self-host.md).

## Connect your computer

Obtain a Litecom server address from its operator. Install and sign in to your [coding agent](agents.md) first. For a temporary local preview, use the single command in the [README](../README.md); for a persistent development server, see [Run a local server](#run-a-local-server) below.

From the extracted download folder:

```sh
./litecom setup --relay https://YOUR-SERVER
```

Replace `https://YOUR-SERVER` with your operator's address, or paste their complete command unchanged. Choose where you want to approve requests: **Phone**, **Slack**, or **Email**. Enter your name and phone number in the browser, then the verification code sent to your phone. Check the displayed pairing code and choose **Connect agent**.

Back in the terminal, select the detected agent and confirm its working folder. Litecom saves the connection, adds its tools to the selected agent, and registers background startup. The ready message requires a real worker heartbeat. Start a new session in your coding agent to load the tools. See [agent compatibility](agents.md) for requirements and validation status.

Installed ACP adapters also appear in the picker. See [Connect an ACP agent](acp.md) for other compatible executables and per-operation approvals.

Phone, Email, and Slack are available when their providers are configured on the relay. Selecting Email verifies a notification address after phone signup; selecting Slack opens the workspace app connection. Add more later in Settings. See [approval channels](approval-channels.md) for setup and validation limits.

## Try it before connecting a server

```sh
./litecom try
```

This local preview handles the server and sample request. Measured after the command starts: two terminal selections, three browser setup buttons, and three fields. Then approve once and download the resulting file. A downloaded binary and a signed-in native agent are prerequisites. The automated measurement uses a fixture agent and local delivery.

## How much work is setup?

Measured with the terminal/browser test on macOS, starting with `litecom setup --relay` and an existing native agent:

| Step | Activations | Typing |
| --- | --- | --- |
| Select Phone in the terminal | 1 Enter | None |
| Sign in in the browser | 2 buttons | Name, phone, code |
| Connect the agent | 1 button | None; existing default selected |
| Confirm detected agent | 1 Enter | None |
| Confirm the current working folder | 1 Enter | None |
| **Total** | **6 activations** | **3 fields** |

This excludes obtaining the binary, installing/signing in to the native agent, copying the setup command, the relay address, reading the code, and any operating-system prompts. The test used local delivery and a fake agent process. It did verify actual macOS background startup. It is not evidence of a complete one-click consumer install.

The Email route adds two buttons and two fields, for **eight activations and five fields** in the local test. Slack provider steps remain unmeasured.

A returning, valid connection skips pairing and the saved agent/folder questions. An expired or revoked connection resumes pairing. A server outage preserves settings and explains how to retry.

## First useful result

```sh
./litecom ask me "Find the launch notes and attach them."
```

Approve in the channel you chose: reply `APPROVE` and the code by text, tap **Approve** in Slack, or reply `Approve` to the email. The request link also provides a browser review. [Channel setup and behavior](approval-channels.md).

That one decision authorizes the requested work and sending the final reply and files. There is no second sharing decision. Existing native hooks and tools keep their permissions; a new ACP permission can be approved once in the same channel.

When the result arrives, click a file to download it. Your browser streams it to its download folder. **Mark received** acknowledges receipt; CLI and MCP downloads verify the complete hash and report receipt automatically.

A request to another person follows the same owner approval path. The requester needs no account invitation for their own agents.

Once background setup is complete, the computer runs unattended while it is awake. ACP tool permission requests use the same connected channels, with a browser review available. The direct Codex/Claude adapters deny new permission prompts instead of opening a local approval dialog; choose a compatible ACP adapter when you need those decisions forwarded remotely.

## Add another person or agent

Run `./litecom contacts --invite` and share the link. The recipient opens it and selects **Send contact request**; you select **Accept contact**. Name lookup works after that mutual acceptance.

To give another harness a separate, revocable connection:

```sh
./litecom --connection claude setup --relay http://127.0.0.1:4141
```

Use the same verified phone account. Select the agent this connection represents in the browser. Create another named destination under **Agents** first if it needs its own work queue.

Use `setup --requester-only` for a connection that only sends requests. It does not start a receiving worker. `mcp-config` prints the configuration for any compatible MCP client without embedding a credential.

## Background operation

The worker runs as your signed-in OS user. Closing the terminal or coding app does not stop it. Sleeping, signing out, or shutting down the computer makes its local agent unavailable. Requests remain queued. Devin sessions are monitored by the relay and can continue while your computer is off. See [cloud agents](cloud-agents.md) for setup and validation status.

```sh
./litecom doctor
./litecom service status
./litecom service start
./litecom service stop
./litecom service remove
```

`service remove` removes automatic startup and preserves your account and settings. It does not revoke remote credentials. Use **Settings → Connected apps → Disconnect** to revoke a connection.

Repeating setup reuses an identical installed executable. When replacing a different build on Windows, stop Litecom's workers and close agent sessions using its MCP connection first, then rerun setup.

macOS uses a per-user LaunchAgent; Linux uses a systemd user service; Windows uses a sign-in task. Only macOS startup has been exercised locally. Native agent configuration is reused; environment-only provider secrets are not copied into startup definitions.

To keep everything in the terminal:

```sh
./litecom setup --relay http://127.0.0.1:4141 --no-background
./litecom worker
```

## If something needs attention

| What you see | Next action |
| --- | --- |
| Server cannot be reached | Retry `setup` when the server is available; saved settings remain intact. |
| Connection expired or disconnected | Run `setup`; it opens pairing again. `setup --reconnect` also starts this explicitly. |
| No coding agents found | Install and sign in to a supported coding agent, then rerun `setup`. Your phone account setup is saved. |
| Folder unavailable | Reconnect the volume, or rerun `setup --workspace /absolute/folder`. |
| MCP server name already exists | Setup preserves the other server and selects an unused name. A matching, enabled Litecom connection is reused. |
| Matching Litecom server is disabled | Enable that connection in your coding agent, then retry setup. |
| OpenClaw cannot save the connection | Stop its gateway through its service manager, rerun setup, then restart the gateway. If your configuration is managed, its owner must add the connection. |
| Background startup failed | Run `worker` in a terminal while checking the reported service error. |
| Agent or folder changed after approval | Create a new request and approve the new settings. |
| Agent stopped or execution is uncertain | Check the external effects and private diagnostic path shown to the owner before retrying. Litecom will not rerun the action. |
| A tool was denied | Adjust the native agent's permissions yourself if appropriate, then submit a new request. |
| File is outside the selected folder | Move the intended file into that folder or select the right working folder, then submit a new request. |

Use `--connection NAME` with these commands when managing a named connection. Use `--home PATH` to store its private configuration somewhere else.

Successful run diagnostics are removed after staging the result. Failed runs retain private local diagnostics for inspection. Automatic retention of those diagnostics and relay payload deletion are still release work.


## Run a local server

From the extracted download folder:

```sh
./litecom relay --public-url http://127.0.0.1:4141 --data .litecom/dev --dev
```

Leave that terminal open. In another terminal, follow **Connect your computer** above using `http://127.0.0.1:4141`. Verification codes appear in the relay terminal; no messages are delivered. The server keeps its data in `.litecom/dev` across restarts.

For the complete temporary preview in one terminal, use `./litecom try`. It starts a private local server, pairs an agent, and opens a real sample request. It uses a sample folder, does not install a background service or change your MCP configuration, and removes its temporary data on normal exit. Native tools keep their existing permissions. The preview is not a hosted installation.
