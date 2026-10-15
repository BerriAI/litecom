# Approve requests

Choose **Phone**, **Slack**, or **Email** during setup. Add another destination later in **Settings → Approval channels**. They all belong to the same Litecom account. Google sign-in is separate from approval delivery; Slack and email do not require a phone number.

- **Phone:** reply `APPROVE 8F31A2C9`, using the code in that request.
- **Slack:** tap **Approve** on the message.
- **Email:** reply with `Approve` on the first line.

Use `DECLINE` with the text-message code, the Slack **Decline** button, or `Decline` in your email reply to refuse. Email replies must come from the address you connected. Reply to the original email so its request reference is preserved.

One approval authorizes the requested work and sending its result. The agent starts after approval and delivers its final reply and files automatically. You do not return to Litecom to approve sharing. If the agent requests an additional tool permission, you can approve that operation once through the same channels.

The first decision wins. Repeated clicks or replies cannot launch a second run. A changed or cancelled request cannot be approved through an old message. Slack updates the original message; SMS and email send status updates because delivered messages cannot be edited. A link remains available for details and downloads.

For requests that exceed the provider's message limit, open the review link to read and approve the full request. Manually prepared replies with existing attachments also use that review page.

Remove a channel in **Settings** to stop notifications and invalidate its outstanding approval controls. A message already being sent may still arrive. **Revoke access** on a completed request stops future downloads; it cannot recall copies already received.

## Setup effort

After Google sign-in, Email needs one confirmation of your Google address; Slack opens its consent screen; Phone asks for a number and code. You can join the workspace before installing an agent. [All setup counts and prerequisites](setup.md#how-many-steps) are listed together.

## Missing a channel or notification

Only channels enabled by your host appear during setup. Ask your host to [configure another channel](hosting-channels.md) if your preferred option is missing.

Check **Settings → Approval channels** if a message does not arrive. Confirm that the destination is connected, then check spam filtering, Slack notifications, or your phone's message filtering. Your host can check provider delivery errors.
