# Approval channels

Your verified phone number identifies your account. Phone, Slack, and Email are places to receive review links; adding one does not create another account or grant access to a mailbox.

Choose a destination during `litecom setup`, or add one later in **Settings**. All links open the same authenticated request. One decision applies everywhere. Slack updates its original notification; SMS and email send status notices because delivered messages cannot be rewritten.

## For people using Litecom

| Destination | Setup after phone verification | Current evidence |
| --- | --- | --- |
| Phone | Already connected when selected during sign-in | Local delivery and mobile review tested; live SMS still needs validation |
| Email | Enter email → Send email code → enter code → Connect email | Full terminal/browser setup and refresh/resume tested with local delivery |
| Slack | Connect Slack → allow the workspace app → return to Litecom | OAuth state, token exchange/refresh, and message-update fixtures pass; real workspace install still needs validation |

Phone setup has six measured activations and three typed fields after the setup command. Email adds two buttons and two fields: **eight activations, five fields**. These counts exclude installation, native-agent authentication, obtaining the server address, reading codes, and OS prompts. Slack's provider controls vary with workspace policy and are not yet measured.

Remove a destination under **Settings** to stop future notifications. A message already being sent may still arrive. Removing a destination does not revoke a response already shared. Use **Revoke access** on that request to stop future downloads.

## For the relay operator

Ordinary users should not supply these credentials. Configure them once on the hosted relay. Both client and relay code are open source; SMS and Slack rely on their respective service providers. Email uses standard SMTP and can use a self-hosted mail server.

Provider secrets belong in the relay's environment, never in the client, README, or source tree. A configured provider makes its channel available in setup. `--dev` prints codes and notifications locally instead of sending messages.

### Phone

Set all three values from your Twilio account and Messaging Service:

```text
TWILIO_ACCOUNT_SID
TWILIO_AUTH_TOKEN
TWILIO_MESSAGING_SERVICE_SID
```

The Messaging Service needs a sender approved for its destination countries. Complete its sender registration and opt-out configuration. Litecom records provider acceptance separately from actual delivery. Provider callback handling and real STOP/delivery validation remain release work.

### Email

```text
SMTP_HOST
SMTP_FROM
SMTP_USERNAME
SMTP_PASSWORD
SMTP_PORT=465
```

`SMTP_HOST` and `SMTP_FROM` are required. Supply username and password together when the server requires authentication. Port 465 uses implicit TLS; other ports, normally 587, require STARTTLS. Plaintext SMTP is not used. `SMTP_FROM` can be an address or a sender such as `Litecom <requests@example.com>`; replace it with a sender your mail server authorizes.

Connect an email address in **Settings**, then submit a request and check that its review notice arrives. An SMTP acceptance response is not proof that the message reached the inbox. Live delivery and spam-folder behavior still require validation.

### Slack

Create a Slack app for the relay using [the standard server-side OAuth flow](https://docs.slack.dev/authentication/installing-with-oauth/).

1. Add a bot user and the bot scopes `chat:write` and `im:write`.
2. Add `https://YOUR_RELAY_HOST/oauth/slack/callback` as its OAuth redirect URL, replacing the host with the relay's HTTPS address.
3. Set `SLACK_CLIENT_ID` and `SLACK_CLIENT_SECRET` from the app's **Basic Information → App Credentials**.
4. Restart the relay. In Litecom, choose **Settings → Connect Slack** and allow the app in the intended workspace.

This is a confidential server-side app. Do not enable Slack's public-client PKCE setting for this configuration. Token rotation is supported when enabled on the Slack app. Workspace administrators may need to allow installation.

Litecom opens a DM with the installing Slack user. It does not request message-history access. Tokens are encrypted on the relay and never returned to agents or browser JavaScript. A one-use OAuth state and the original authenticated browser session must both match before the Slack destination is linked.

Submit a request, confirm that the DM arrives, approve it, and confirm that the same message updates. That real workspace check is still a release requirement; mock protocol tests do not establish it.
