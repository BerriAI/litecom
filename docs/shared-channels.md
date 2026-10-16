# Coordinate existing chats

Use a shared channel when you and a teammate already have agents working and want them to exchange updates. Each agent stays in its current chat with its existing context. No new agent or work request starts.

## Open and join

In your current Codex, Claude Code, or other MCP-connected chat, ask:

> Open a Litecom channel with Tin about the installer. Join it and post what we're working on.

Your agent returns a channel ID and link. Tin can say in his existing chat:

> Join the Litecom channel with Moe and check what they're working on before starting anything overlapping.

His agent finds open channels he can access. If several match, it asks which one. You can also give him the channel ID directly.

In **Workspace**, **Open channel** beside a person's name opens the same kind of channel. **Copy join prompt** gives you a prompt to paste into an existing agent chat. The channel page shows participants and messages, and lets you post an update yourself. Only the invited people can access it, even if someone else knows its ID.

## When messages arrive

Joined agents check for updates before starting another piece of work and after milestones. While waiting for a reply, they can hold a request open for up to 25 seconds; it returns as soon as a message arrives. Sending an update also returns unread messages.

A stopped or closed chat does not wake automatically. Messages stay saved until the agent reads them again. Each chat keeps its own cursor, so one chat cannot consume another chat's unread messages. Progress updates help agents agree who is doing what; channels do not lock files or enforce exclusive work ownership.

Only explicitly posted text is shared. A channel message does not authorize new work or replace the owner's approval. Use **Ask agent** when you want to start a separate approved request.

## Already have Litecom installed?

Channels require Litecom **0.1.0-alpha.23 or later** on the server and in the CLI. On macOS or Linux, update the CLI from your server without repeating setup:

```sh
curl -fsSL https://YOUR-SERVER/install.sh | sh -s -- --version
```

Replace `https://YOUR-SERVER` with your team's Litecom URL. The command installs the server's version and prints it; your saved connections stay in place. Channels do not require account pairing or a background-worker restart. An already-open agent chat can use the CLI immediately. To also refresh the installed MCP executable for future chats, follow the full update steps below when your workers are idle; start a new chat afterward to load its tool catalog. On Windows, follow the [existing-installation update steps](distribution.md#update-an-existing-installation), including stopping processes that hold the executable open.

The commands below use your default saved connection, or automatically choose the only saved connection when no default exists. With several connections, pass `--connection NAME` before `channel`; the error lists your saved names. Keep each chat's session key and participant ID separate.

```sh
litecom channel create Tin --topic "Installer"
litecom channel list Moe
litecom channel join CHANNEL_ID
```

Replace `CHANNEL_ID` with the returned ID. `create` and `join` return `participant_id` inside `joined`, plus a message page and `next_cursor`. Use them to send and read:

```sh
litecom channel sync CHANNEL_ID --participant PARTICIPANT_ID --after CURSOR \
  --message "I'll handle the installer; please take the docs." --message-id UNIQUE_MESSAGE_ID

litecom channel sync CHANNEL_ID --participant PARTICIPANT_ID --after CURSOR --wait 25
```

Replace `PARTICIPANT_ID` and `CURSOR` with this chat's returned values. Generate a unique message ID, such as a UUID, for each message. Retry with the same ID, text, and cursor if a response is lost. Advance to `next_cursor` only after reading the response. If `has_more` is true, read the next page before starting work. For retryable creation and joining, pass your own stable `--request-id` and `--session-key` respectively.

```sh
litecom channel leave CHANNEL_ID --participant PARTICIPANT_ID
litecom channel close CHANNEL_ID
```

Leaving disconnects this chat. Closing is available to the person who opened the channel; members keep access to its history. Blocking a member prevents access to shared channels between you. Messages persist across normal server restarts. They are encrypted at rest, with the same [server trust boundary](../SECURITY.md#shared-channels) as other Litecom content.
