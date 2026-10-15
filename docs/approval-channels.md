# Approve where you are

Choose **Phone**, **Slack**, or **Email** during setup. Add another destination later in **Settings → Approval channels**. They all belong to the same phone account.

| Channel | Approve a request | Required actions |
| --- | --- | --- |
| Phone | Reply `APPROVE 8F31A2C9`, using the code in that request | One text reply |
| Slack | Tap **Approve** on the message | One button |
| Email | Reply with `Approve` on the first line | One email reply |

Use `DECLINE` with the text-message code, the Slack **Decline** button, or `Decline` in your email reply to refuse. Email replies must come from the address you connected. Reply to the original email so its request reference is preserved.

One approval authorizes the requested work and sending its result. The agent starts after approval and delivers its final reply and files automatically. You do not return to Litecom to approve sharing. If the agent requests an additional tool permission, its exact operation can be approved once through the same channels.

The first decision wins. Repeated clicks or replies cannot launch a second run. A changed or cancelled request cannot be approved through an old message. Slack updates the original message; SMS and email send status updates because delivered messages cannot be edited. A link remains available for details and downloads.

Direct replies require the relay's incoming-message configuration below. If a request is too long to display completely within a provider's message limit, its notice links to the complete review instead of offering approval of truncated text. Manually prepared replies with existing attachments also use that review page. Your agent's ordinary request → approval → result flow supports direct message approval.

Remove a channel in **Settings** to stop notifications and invalidate its outstanding approval controls. A message already being sent may still arrive. **Revoke access** on a completed request stops future downloads; it cannot recall copies already received.

## Setup effort

After installation and native-agent sign-in, the measured Phone setup uses **six activations and three fields**: select Phone, enter name/phone/code, connect the device, confirm the agent, and confirm the folder. Email adds two buttons and two fields. Slack adds its workspace consent flow, whose controls depend on workspace policy.

These counts exclude obtaining the binary and server address, reading codes, and operating-system prompts. See [the measured setup path](setup.md#how-much-work-is-setup).

## Configure the relay

End users do not configure providers or run a receiving mailbox. The operator does this once. Both Litecom components are open source; Twilio and Slack are services. Email uses standard SMTP and IMAP, including self-hosted servers.

Keep provider credentials in the relay environment. `--dev` prints local notices and disables incoming provider callbacks and mailbox polling; it sends no messages.

### Phone

Set:

```text
TWILIO_ACCOUNT_SID
TWILIO_AUTH_TOKEN
TWILIO_MESSAGING_SERVICE_SID
```

In the Twilio Messaging Service's **Integration** settings, select **Send a webhook** for incoming messages. Set the request URL to `https://YOUR_RELAY_HOST/webhooks/twilio`, with method **POST**, replacing the host with the relay's public HTTPS address. Use that exact address without a trailing slash or query parameters.

Attach a two-way SMS sender and complete the required registration for the countries you serve. Alphanumeric senders that cannot receive replies cannot support text approval. Configure Twilio's opt-out handling. A signed `STOP` callback disables Litecom's phone channel too; reconnect it in Settings after restoring consent with the provider.

Litecom verifies [Twilio's signature](https://www.twilio.com/docs/usage/security), account SID, sender number, and the request code. The code selects a request; it is not sufficient without the verified sender. Provider acceptance is recorded separately from actual delivery. Delivery-status callbacks are not implemented.

### Email

For outgoing notices:

```text
SMTP_HOST
SMTP_FROM
SMTP_PORT=465
SMTP_USERNAME
SMTP_PASSWORD
```

`SMTP_FROM` is a sender your server authorizes, for example `Litecom <requests@example.com>`. Supply username and password together if authentication is required. Port 465 uses implicit TLS; other ports, usually 587, require STARTTLS.

For replies, provide a **dedicated receiving mailbox**:

```text
IMAP_HOST
IMAP_PORT=993
IMAP_USERNAME
IMAP_PASSWORD
```

The mailbox must receive mail for `SMTP_FROM`. If it has a different address, set `SMTP_REPLY_TO` to that address. Use the mail server's application password when required. Restart the relay after changing its environment.

Litecom reads INBOX over TLS using `async-imap` and parses replies with `mail-parser`. It preserves read/unread flags, never deletes mail, and checkpoints message UIDs so restarts do not reprocess the inbox. Only an explicit first-line `Approve` or `Decline`, a matching connected sender, and the private reference from the original notification can decide a request. Automatic replies, forwarded quotes, unknown references, and links opened by mail scanners do not approve anything.

Treat that mailbox and the connected email accounts as approval credentials. Do not auto-forward request emails to untrusted recipients or give untrusted agents access to them. SMTP acceptance does not prove inbox delivery; verify receiving and replying with your actual mail service.

### Slack

Create a Slack app using [server-side OAuth](https://docs.slack.dev/authentication/installing-with-oauth/).

1. Add a bot user with scopes `chat:write` and `im:write`.
2. In **OAuth & Permissions**, add `https://YOUR_RELAY_HOST/oauth/slack/callback` as a redirect URL.
3. In **Interactivity & Shortcuts**, enable interactivity and set the request URL to `https://YOUR_RELAY_HOST/webhooks/slack`.
4. Set `SLACK_CLIENT_ID`, `SLACK_CLIENT_SECRET`, and `SLACK_SIGNING_SECRET` from **Basic Information → App Credentials** on the relay, then restart it.
5. In Litecom, choose **Settings → Connect Slack** and allow the workspace app.

Do not enable Slack's public-client PKCE option for this server-side app. Token rotation is supported. Workspace administrators may need to allow installation.

Litecom opens a DM with the installing user. It requests no message-history access. Incoming buttons require [Slack's signing secret and timestamp verification](https://docs.slack.dev/authentication/verifying-requests-from-slack/) and the linked workspace, user, and DM channel. The action is bound to the exact proposal. Provider tokens remain encrypted on the relay.

## Verify delivery

Submit a request from a contact or your own agent. Confirm that the full request and approval control arrive, approve in the message, and check that execution starts once. Repeat the approval to check the already-decided response. Confirm that the final result appears and the other connected channels update.

Signed webhook fixtures, Slack message-update fixtures, email parsing, IMAP checkpoint/reconnect, and cross-channel decision tests pass locally. Actual carrier, workspace, and mail-provider delivery still need live validation; local tests do not establish those outcomes.
