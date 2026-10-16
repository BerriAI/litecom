# Manage your local agent

For your first connection, follow [Get started](setup.md). Use this page to change a connection, manage background startup, or fix a problem.

[Terminal setup](#set-up-from-the-terminal) · [Background operation](#background-operation) · [Monitor runs](#monitor-local-runs) · [Uninstall](#uninstall) · [Troubleshooting](#if-something-needs-attention)

Keep the browser-generated command private. If setup fails after pairing, rerun it on the same computer to resume. If it expires before pairing, click **Connect local agent** for a new command.

Commands below use `NAME` for the value after `--connection` in your copied setup command or terminal output. If you set up without that flag, use `default`.

## Set up from the terminal

The browser-generated command is the shortest route. Existing installations can also run:

```sh
litecom --connection NAME setup --relay https://YOUR-SERVER
```

Replace the address with your host’s URL. This alternative selects an approval channel and opens browser pairing. A returning valid connection reuses saved settings. `--requester-only` configures sending without a receiving worker; `--connection NAME` gives another harness its own revocable connection.

New connections start in your home folder (`~`), regardless of where you run setup. Rerunning setup or switching agents keeps the saved folder. To use a different folder:

```sh
litecom --connection NAME setup --workspace /absolute/folder
```

Replace `/absolute/folder` with the folder you want the agent to use. On macOS/Linux, `--workspace ~` selects your home folder; in PowerShell, use `--workspace "$HOME"`. The selected folder appears in setup output and each request’s **Agent and access** details. Your agent’s tool permissions still apply.

## Background operation

The worker runs as your signed-in OS user. Closing the terminal or coding app does not stop it. Sleeping, signing out, or shutting down the computer makes its local agent unavailable. Requests remain queued. Devin sessions are monitored by the relay and can continue while your computer is off. See [cloud agents](cloud-agents.md) for connection requirements.

| To… | Run |
| --- | --- |
| Check your connection | `litecom --connection NAME doctor` |
| Check the background worker | `litecom --connection NAME service status` |
| Start or restart it | `litecom --connection NAME service start` |
| Stop it | `litecom --connection NAME service stop` |
| Remove automatic startup | `litecom --connection NAME service remove` |

`service remove` removes automatic startup and preserves your account and settings. It does not revoke remote credentials. Use **Settings → Connected apps → Disconnect** to revoke a connection.

Repeating setup reuses an identical installed executable. When replacing a different build on Windows, stop Litecom's workers and close agent sessions using its MCP connection first, then rerun setup.

macOS uses a per-user LaunchAgent; Linux uses a systemd user service; Windows uses a sign-in task. Native agent configuration is reused. Secrets exported only in your terminal are not copied to the background service; save credentials through your agent’s supported sign-in flow. Linux and Windows background startup remain experimental; use foreground mode below if startup fails.

The Windows task runs under your signed-in account without elevation. It permits battery power and has no fixed runtime cutoff. Closing the terminal does not stop it; signing out still does.

To keep everything in the terminal:

```sh
litecom --connection NAME setup --relay https://YOUR-SERVER --no-background
litecom --connection NAME worker
```

## Monitor local runs

List the requests running on this computer and their Codex or ACP session IDs:

```sh
litecom runs
```

Follow the latest active run:

```sh
litecom logs --follow
```

To follow a specific request, copy its ID from `litecom runs`:

```sh
litecom logs REQUEST_ID --follow
```

Press Ctrl+C to stop watching; the agent keeps running. `litecom runs --active` shows only running requests. `litecom runs --json` gives paths and session IDs for scripts. `litecom logs REQUEST_ID --raw` shows the original events. No sign-in or connection selection is needed to inspect local runs.

A brief relay outage retries the connection and delivery without rerunning the agent. If the outage outlasts the execution lease (two minutes since the last confirmed heartbeat), the agent stops and the request needs attention. Check its logs and any external effects before sending another request.

## Uninstall

Open **Settings → Uninstall from this computer**, click **Copy command**, then paste it into your terminal and press Enter. Select the connections to remove with Space, then press Enter. The list shows each agent and its working folder.

For the hosted workspace on macOS or Linux:

```sh
curl -fsSL https://litecom.onrender.com/uninstall.sh | sh
```

On Windows, paste into PowerShell:

```powershell
& ([scriptblock]::Create((irm https://litecom.onrender.com/uninstall.ps1)))
```

Wait for **Selected connections uninstalled** or **Litecom uninstalled**. This disconnects your selections and removes their background startup and MCP entries. Removing the last connection also removes the installed binaries. Your coding agents, working files, downloads, workspace account, and hosted data stay. Shared shell PATH settings stay too.

If a connection cannot be removed, the command explains what needs attention. Resolve it and rerun the same command. On Windows, close agent sessions using Litecom first. For a custom configuration folder, run `litecom --home PATH uninstall`.

## If something needs attention

| What you see | Next action |
| --- | --- |
| Server cannot be reached | Retry `litecom --connection NAME setup` when the server is available; saved settings remain intact. |
| Connection expired or disconnected | Run `litecom --connection NAME setup`; it opens pairing again. `litecom --connection NAME setup --reconnect` also starts this explicitly. |
| No coding agents found | Install and sign in to a supported coding agent, then rerun `litecom --connection NAME setup`. Your existing account is preserved. |
| Folder unavailable | Reconnect the volume, or run `litecom --connection NAME setup --workspace /absolute/folder`. |
| MCP server name already exists | Setup preserves the other server and selects an unused name. A matching, enabled Litecom connection is reused. |
| Matching Litecom server is disabled | Enable that connection in your coding agent, then retry setup. |
| OpenClaw cannot save the connection | Stop its gateway through its service manager, rerun setup, then restart the gateway. If your configuration is managed, its owner must add the connection. |
| Background startup failed | Run `litecom --connection NAME worker` in a terminal while checking the reported service error. |
| Agent or folder changed after approval | Create a new request and approve the new settings. |
| Agent stopped or execution is uncertain | Check the external effects and private diagnostic path shown to the owner before retrying. Litecom will not rerun the action. |
| A tool was denied | Adjust the native agent's permissions yourself if appropriate, then submit a new request. |
| File is outside the selected folder | Move the intended file into that folder or select the right working folder, then submit a new request. |

Use `--home PATH` to store private configuration somewhere else. After a fix, run `litecom --connection NAME doctor` and send a small request to your own agent.

Run logs stay in the private local configuration folder, including successful runs. They can contain agent output and tool details. `litecom runs --json` shows each run’s directory; delete a finished run’s directory when you no longer need its logs. There is no automatic cleanup of local run logs or relay payloads.

