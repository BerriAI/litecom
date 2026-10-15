# Configure approval channels

Configure at least one channel so users can approve requests. Keep provider credentials in the relay environment; users connect their own Slack account, email address, or phone number in Litecom.

[Slack](#slack) · [Email](#email) · [Phone](#phone) · [Check delivery](#check-delivery)

For Docker Compose, put these values in `.env` and run `docker compose up -d` after saving changes. For a native server, update its environment and restart the process. `--dev` sends no provider messages and disables incoming callbacks and mailbox polling.

## Slack

Create a Slack app using [server-side OAuth](https://docs.slack.dev/authentication/installing-with-oauth/).

1. Open `https://YOUR_RELAY_HOST/setup/slack`, replacing the host with your running Litecom server. This opens Slack’s app creation flow with its name, bot scopes, OAuth callback, and interactive-message URL prefilled. Select your workspace and create the app.
2. Copy **Client ID**, **Client Secret**, and **Signing Secret** from **Basic Information → App Credentials** into `SLACK_CLIENT_ID`, `SLACK_CLIENT_SECRET`, and `SLACK_SIGNING_SECRET` on the relay. Restart it.
3. In Litecom, choose **Connect Slack** and allow the app. To serve other Slack workspaces, enable distribution in the Slack app’s settings; their administrators may restrict installation.
4. Send a test request and approve it in the resulting DM. Verify that the result arrives and the original approval message updates.

The generated [Slack app manifest](https://docs.slack.dev/app-manifests/configuring-apps-with-app-manifests/) requests only `chat:write` and `im:write`. For manual setup, the OAuth callback is `/oauth/slack/callback` and the Interactivity request URL is `/webhooks/slack`, both under your public HTTPS address.

Do not enable Slack's public-client PKCE option for this server-side app. Token rotation is supported. Workspace administrators may need to allow installation.

Litecom opens a DM with the installing user. It requests no message-history access. Incoming buttons require [Slack's signing secret and timestamp verification](https://docs.slack.dev/authentication/verifying-requests-from-slack/) and the linked workspace, user, and DM channel. The action is bound to the exact proposal. Provider tokens remain encrypted on the relay.

To also start requests with `@Litecom`, [enable the optional Slack agent](slack-agent.md). It uses this same app and adds message-history permissions. Approval DMs work without it.

## Email

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

## Phone

Use [AgentPhone](https://agentphone.ai) for sign-in codes, approval messages, and status updates. Users select **Connect Phone** in Litecom and verify their number; they do not need an AgentPhone account.

**Enable outbound messaging before inviting users.** A provisioned number can receive texts before it can send sign-in codes. Complete [AgentPhone's carrier registration](https://agentphone.ai/register) with your business details, message examples, opt-in flow, and policy URLs. Check AgentPhone for the current fees and approval times. You register the sending number once; users do not register their own numbers.

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

Litecom records provider acceptance, not confirmed delivery. Delivery-status callbacks are not supported. Check AgentPhone if an accepted message does not arrive.

## Check delivery

Sign in to Litecom and connect the channel. Ask your own agent for a short reply, approve from the message, and confirm that the result arrives. Repeat the approval to check that the request does not run again. With multiple channels connected, check that each shows the decision.

A healthy server does not confirm message delivery. Check the provider's delivery status and the relay logs if a notification does not arrive. After this check, share the server link and [user setup guide](setup.md).
