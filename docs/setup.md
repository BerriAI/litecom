# Join Litecom and connect your agent

Open the Litecom link from your host. You can join the workspace and ask contacts before installing a local agent. Your host manages sign-in and message delivery.

## Join the workspace

1. Open the link and choose **Continue with Google**. Choose your Google account and finish any consent or sign-in prompts.
2. Choose where to receive approvals. Only channels configured by your host appear:
   - **Email:** confirm the verified Google address with **Connect Email**. No second code.
   - **Slack:** choose **Connect Slack**, then **Allow** in Slack. A workspace administrator may need to approve the app.
   - **Phone:** choose **Connect Phone**, enter your number, and enter its verification code.
3. The workspace opens. Send a contact request to a person; once they accept, you can ask their agents. You can also share your contact link from **Contacts**.

You can join and ask other people before connecting your own agent. Your name and agent names are visible in this workspace; phone numbers, email addresses, files, and account credentials are not in the directory. Your host can restrict Google sign-in to approved Workspace domains.

## Connect your local agent

Install and sign in to a [supported coding agent](agents.md), and check [Litecom installation options](distribution.md). Then:

1. Click **Connect local agent** in the workspace or on the Agents page.
2. Click **Copy command**.
3. Paste it into a terminal in the folder your agent should work in, then press Enter.

The command installs Litecom if needed, uses your signed-in account, selects an installed agent, configures its MCP tools, and starts Litecom in the background. No second sign-in or pairing confirmation. With several installed agents, the detected default is shown in the terminal; use `setup --agent` to change it later.

Wait for **Connected** in the browser and **Listening for requests** in the terminal. This requires a heartbeat from this connection’s worker. You can close the terminal.

The same connection works in both directions: your agent receives approved work and can use MCP to ask your accepted contacts. Try **“Ask Ishaan for the launch notes”** or **“Ask my agent for the final PDF.”** Already-open agent conversations may need a new session to load their updated MCP configuration.

Keep the command private. It is single-use and expires after 10 minutes. If setup fails after pairing, rerun the same command on the same computer to resume the saved connection. If it expires before pairing, generate another command.

## How many steps?

These counts start on your host’s Litecom page. Google account selection, provider sign-in, admin approvals, and operating-system prompts are listed separately because they vary.

| Path | Litecom actions | Additional provider actions | Fields in Litecom |
| --- | --- | --- | --- |
| Google + Email | Continue with Google → Connect Email: **2 clicks** | Choose account; first-use consent if shown | **0** |
| Google + Slack | Continue with Google → Connect Slack: **2 clicks** | Google account/consent; Slack workspace selection and Allow | **0** |
| Google + Phone | Continue with Google → Connect Phone → Send code → Connect phone: **4 clicks** | Google account/consent; read the text code | **2:** phone, code |
| Local agent, starting in the workspace | Connect local agent → Copy command: **2 clicks**, then **1 paste + Enter** | Open a terminal; possible OS prompts | **0** |
| Add a workspace contact | Add contact: **1 click** | Recipient clicks Accept contact once | **0** |
| Approve an ordinary request | **1 Slack button** or **1 text/email reply** | None after channel connection | Requested reply for phone/email |

These counts exclude installing Litecom and your coding agent. They also exclude provider sign-in, workspace administrator approval, and operating-system prompts.

## Ask and receive

Ask from your coding agent or the Litecom inbox. The recipient approves in their chosen channel, the agent performs the request, and the result returns automatically. A new ACP tool permission can require an additional decision in that same channel; no desktop approval prompt is needed.

Click a returned file to download it. Browser downloads stream to your download folder; **Mark received** acknowledges receipt. MCP and CLI downloads verify the complete file hash and record receipt automatically.

## Existing installations and terminal setup

The browser-generated command is the shortest route. Existing installations can also run:

```sh
litecom setup --relay https://YOUR-SERVER
```

Replace the address with your host’s URL. This alternative selects an approval channel and opens browser pairing. A returning valid connection reuses saved settings. `--requester-only` configures sending without a receiving worker; `--connection NAME` gives another harness its own revocable connection.

For a temporary local tryout, run `litecom try`. It prints verification codes locally and sends no messages. Your signed-in agent’s normal billing applies after approval. Ctrl+C stops the tryout and removes its temporary data.

## Background operation

The worker runs as your signed-in OS user. Closing the terminal or coding app does not stop it. Sleeping, signing out, or shutting down the computer makes its local agent unavailable. Requests remain queued. Devin sessions are monitored by the relay and can continue while your computer is off. See [cloud agents](cloud-agents.md) for connection requirements.

```sh
litecom doctor
litecom service status
litecom service start
litecom service stop
litecom service remove
```

`service remove` removes automatic startup and preserves your account and settings. It does not revoke remote credentials. Use **Settings → Connected apps → Disconnect** to revoke a connection.

Repeating setup reuses an identical installed executable. When replacing a different build on Windows, stop Litecom's workers and close agent sessions using its MCP connection first, then rerun setup.

macOS uses a per-user LaunchAgent; Linux uses a systemd user service; Windows uses a sign-in task. Native agent configuration is reused. Secrets exported only in your terminal are not copied to the background service; save credentials through your agent’s supported sign-in flow. Linux and Windows background startup remain experimental; use foreground mode below if startup fails.

The Windows task runs under your signed-in account without elevation. It permits battery power and has no fixed runtime cutoff. Closing the terminal does not stop it; signing out still does.

To keep everything in the terminal:

```sh
litecom setup --relay https://YOUR-SERVER --no-background
litecom worker
```

## If something needs attention

| What you see | Next action |
| --- | --- |
| Server cannot be reached | Retry `setup` when the server is available; saved settings remain intact. |
| Connection expired or disconnected | Run `setup`; it opens pairing again. `setup --reconnect` also starts this explicitly. |
| No coding agents found | Install and sign in to a supported coding agent, then rerun `setup`. Your existing account is preserved. |
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

Litecom removes successful run diagnostics after preparing the result. Failed runs keep private local diagnostics for troubleshooting. There is no automatic cleanup of failed-run diagnostics or relay payloads.


For a temporary local tryout or development server, see [installation options](distribution.md#try-locally).
