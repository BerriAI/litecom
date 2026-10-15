# Approve where you are

Choose **Phone**, **Slack**, or **Email** during setup. Add another destination later in **Settings → Approval channels**. They all belong to the same Litecom account. Google sign-in is separate from approval delivery; Slack and email do not require a phone number.

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

After Google sign-in, Email needs one confirmation of your Google address; Slack opens its consent screen; Phone asks for a number and code. You can join the workspace before installing an agent. [All setup counts and prerequisites](setup.md#how-many-steps) are listed together.

## Configure the relay

End users do not configure providers or run a receiving mailbox. The operator does this once. Both Litecom components are open source; AgentPhone and Slack are services. Email uses standard SMTP and IMAP, including self-hosted servers.

Keep provider credentials in the relay environment. `--dev` prints local notices and disables incoming provider callbacks and mailbox polling; it sends no messages.

### Phone

Use [AgentPhone](https://agentphone.ai) for sign-in codes, approval messages, and status updates. End users keep the same **Phone** choice; they never need an AgentPhone account.

**Enable outbound messaging before inviting users.** An active number can receive texts while still being unable to send sign-in codes. Complete [AgentPhone's carrier registration](https://agentphone.ai/register); its [registration reference](https://docs.agentphone.ai/api-reference/registration/register-a-2-p-v-1-register-post) currently lists a $25 standard registration fee and a typical 7–10 business-day approval period. This is one registration for the operator, not for each Litecom user. The operator supplies its business details, message examples, opt-in flow, and applicable policy URLs. Start this step while setting up the server.

1. Get an API key from AgentPhone. Create a dedicated **Litecom** agent with `voiceMode: "webhook"`, and attach a two-way phone number. The [AgentPhone quickstart](https://docs.agentphone.ai/welcome) includes the create, provision, and attach commands. Save the agent ID and number ID shown in their details. Litecom uses messaging only; it does not use AgentPhone's hosted AI or voice calls.
2. Register the callback below once, replacing the API key, agent ID, and relay host below with your values:

   ```sh
   export AGENTPHONE_API_KEY='your_api_key'
   curl --fail-with-body https://api.agentphone.ai/v1/agents/YOUR_AGENT_ID/webhook \
     -H "Authorization: Bearer $AGENTPHONE_API_KEY" \
     -H 'Content-Type: application/json' \
     -d '{"url":"https://YOUR_RELAY_HOST/webhooks/agentphone","contextLimit":0}'
   ```

   Save the returned `secret`. Creating or updating this webhook rotates that secret; do not repeat this command as a health check.
3. Set these relay environment values, then restart the relay:

   ```dotenv
   AGENTPHONE_API_KEY=your_api_key
   AGENTPHONE_AGENT_ID=agt_your_agent
   AGENTPHONE_NUMBER_ID=num_your_number
   AGENTPHONE_WEBHOOK_SECRET=whsec_your_secret
   ```

   For Docker Compose, put them in `.env` and run `docker compose up -d`. All four belong to the same AgentPhone project and sender. The explicit number keeps outgoing messages on the same line that receives approvals.
4. Sign in to Litecom, request a short reply from your own agent, and reply to its approval text. Confirm that the result arrives and that a repeated approval does not run it again. Use a real incoming text to check the callback: AgentPhone's synthetic test can supply an invalid `numberId`, which Litecom rejects. A successful callback alone does not prove outbound delivery or an approval.

AgentPhone chooses iMessage where both the sender and recipient support it; this does not make an SMS-only number an iMessage sender. Its number-details API reports `outboundSms: "registration_required"` when SMS registration is still needed. Verify the sender's actual capabilities and complete registration for the countries and channels you serve. See [AgentPhone messaging](https://docs.agentphone.ai/documentation/guides/messages). Litecom sends the full request without a local character cutoff.

The first sign-in text identifies Litecom, confirms that the user requested it, and includes STOP instructions, as required by [AgentPhone's first-message guidance](https://docs.agentphone.ai/documentation/reference/messaging-rate-limits#first-message-requirements).

Litecom verifies the [webhook signature and timestamp](https://docs.agentphone.ai/documentation/guides/webhooks), configured agent and number, connected sender, and exact request code. Calls, reactions, outbound echoes, and group messages cannot approve. Signed `STOP` replies disable the phone channel and invalidate its outstanding approval codes; reconnect in Settings after restoring consent with the provider. Callback receipts survive relay restarts.

Provider acceptance is recorded separately from actual delivery. Delivery-status callbacks are not implemented. AgentPhone credentials are required for live validation; the automated tests use a local HTTP fixture and send no real messages.

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

1. Open `https://YOUR_RELAY_HOST/setup/slack`, replacing the host with your running Litecom server. This opens Slack’s app creation flow with its name, bot scopes, OAuth callback, and interactive-message URL prefilled. Select your workspace and create the app.
2. Copy **Client ID**, **Client Secret**, and **Signing Secret** from **Basic Information → App Credentials** into `SLACK_CLIENT_ID`, `SLACK_CLIENT_SECRET`, and `SLACK_SIGNING_SECRET` on the relay. Restart it.
3. In Litecom, choose **Connect Slack** and allow the app. To serve other Slack workspaces, enable distribution in the Slack app’s settings; their administrators may restrict installation.
4. Send a test request and approve it in the resulting DM. Verify that the result arrives and the original approval message updates.

The generated [Slack app manifest](https://docs.slack.dev/app-manifests/configuring-apps-with-app-manifests/) requests only `chat:write` and `im:write`. For manual setup, the OAuth callback is `/oauth/slack/callback` and the Interactivity request URL is `/webhooks/slack`, both under your public HTTPS address.

Do not enable Slack's public-client PKCE option for this server-side app. Token rotation is supported. Workspace administrators may need to allow installation.

Litecom opens a DM with the installing user. It requests no message-history access. Incoming buttons require [Slack's signing secret and timestamp verification](https://docs.slack.dev/authentication/verifying-requests-from-slack/) and the linked workspace, user, and DM channel. The action is bound to the exact proposal. Provider tokens remain encrypted on the relay.

## Verify delivery

Submit a request from a contact or your own agent. Confirm that the full request and approval control arrive, approve in the message, and check that execution starts once. Repeat the approval to check the already-decided response. Confirm that the final result appears and the other connected channels update.

Signed webhook fixtures, Slack message-update fixtures, email parsing, IMAP checkpoint/reconnect, and cross-channel decision tests pass locally. Real Google-to-Slack linking, DM delivery, and button approval also pass in BerriAI: one approval triggered one deterministic test execution and automatic file delivery with verified bytes. Outgoing phone, mail-provider delivery, and a live two-person flow remain unverified. See [release validation](release-readiness.md).
