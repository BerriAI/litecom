# Approve requests

Choose **Phone**, **Slack**, or **Email** during setup. Add another destination later in **Settings → Approval channels**. They all belong to the same Litecom account. Google sign-in is separate from approval delivery; Slack and email do not require a phone number.

- **Phone:** reply `APPROVE 8F31A2C9`, using the code in that request.
- **Slack:** tap **Approve read-only** or, for an action request, **Approve actions**.
- **Email:** reply with `Approve` on the first line.

For Slack mentions, Litecom tags you and puts approval buttons in the source thread as well as your DM. Only your linked Slack account can use them.

Use `DECLINE` with the text-message code, the Slack **Decline** button, or `Decline` in your email reply to refuse. Email replies must come from the address you connected. Reply to the original email so its request reference is preserved.

One approval authorizes the requested work and sending its result. The agent starts after approval and delivers its final reply and requested files automatically. You do not return to Litecom to approve sharing.

## Choose access

**Approve read-only** lets a local Codex or Claude Code read files, GitHub and Slack, then return an answer. It cannot edit your files, send messages, change access, or call action tools. Delivery of the approved answer and requested existing files is included. You can choose read-only even when the requester asked for an action; the agent explains which work needs an action request.

**Approve actions** lets the agent use its tools to carry out the requested work. This option appears when the sender's agent marks the request as needing actions, or the sender chooses **Request actions** in the inbox. Tool permissions are granted for that run. Operating-system permissions and account sign-in still apply.

For email, reply `Read-only` on the first line to downgrade an action request. For text messages, reply `READONLY 8F31A2C9` with that request's code. A plain `Approve` follows the request's access: ask-only requests stay read-only.

Read-only requires an updated Litecom worker and a supported native agent. Older workers leave the request waiting; they never run it with action access. Other ACP agents and Devin do not yet support this mode.

GitHub reads use the owner's existing GitHub CLI login. If it is missing, run `gh auth login` on that computer. Slack reads use the owner's linked workspace and check channel membership each time. Add Litecom to the channel before requesting its messages. If Slack reports missing permissions after a host upgrade, reconnect it from **Settings → Approval channels**. Read-only does not require the optional hosted Slack agent.

The first decision wins. Repeated clicks or replies cannot launch a second run. A changed or cancelled request cannot be approved through an old message. Slack updates the original message; SMS and email send status updates because delivered messages cannot be edited. A link remains available for details and downloads.

For requests that exceed the provider's message limit, open the review link to read and approve the full request. Manually prepared replies with existing attachments also use that review page.

Remove a channel in **Settings** to stop notifications and invalidate its outstanding approval controls. A message already being sent may still arrive. **Revoke access** on a completed request stops future downloads; it cannot recall copies already received.

## Setup effort

After Google sign-in, Email needs one confirmation of your Google address; Slack opens its consent screen; Phone asks for a number and code. You can join the workspace before installing an agent. [All setup counts and prerequisites](setup.md#how-many-steps) are listed together.

## Missing a channel or notification

Only channels enabled by your host appear during setup. Ask your host to [configure another channel](hosting-channels.md) if your preferred option is missing.

Check **Settings → Approval channels** if a message does not arrive. Confirm that the destination is connected, then check spam filtering, Slack notifications, or your phone's message filtering. Your host can check provider delivery errors.
