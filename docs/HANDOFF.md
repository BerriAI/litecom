# Litecom launch handoff

October 15, 2026 · [App](https://litecom.onrender.com) · [Source](https://github.com/BerriAI/litecom) · [Render service](https://dashboard.render.com/web/srv-db8hglo473hc73evp9eg)

The redesigned app is deployed. New-user activation still needs Google credentials, a working approval channel, and published installers. The source repository is **Internal** and there is **no published native release** yet.

## What Tin needs to do

Send Tin this section; he can create the client himself.

1. Open [Google Auth Platform](https://console.cloud.google.com/auth/clients) in a project he can manage. Check whether `berri-litecom` exists first; its creation was not confirmed, and the current browser account cannot access it.
2. Configure the app name **Litecom**, a monitored support email, and its audience. Use **Internal** for a Berri-only pilot. Use **External** if people outside the organization must sign in, and complete the applicable publishing requirements.
3. Create a **Web application** OAuth client with this exact authorized redirect URI:

   ```text
   https://litecom.onrender.com/api/auth/google/callback
   ```

   Litecom requests `openid email profile` only. No Gmail, Drive, service account, or JavaScript origin is needed. [Google's configuration reference](https://developers.google.com/identity/openid-connect/openid-connect#settingup).
4. In the [Render service's Environment settings](https://dashboard.render.com/web/srv-db8hglo473hc73evp9eg/env), add `GOOGLE_CLIENT_ID` and `GOOGLE_CLIENT_SECRET`. Set `GOOGLE_ALLOWED_DOMAINS=berri.ai` for the Berri-only pilot; leaving it empty allows any verified Google account to join and see the workspace directory. Keep the secret in Render or an approved secret manager, not a chat or the repository.
5. Save and deploy. Open Litecom in a fresh browser session and choose **Continue with Google**. Success means reaching the approval-channel picker under the correct name.

## Turn on an approval channel

Only **one** working channel is needed to start. Add the others later.

- **Slack:** open [Create the Litecom Slack app](https://litecom.onrender.com/setup/slack), select your workspace, and create the prefilled app. Copy its Client ID, Client Secret, and Signing Secret into Render as `SLACK_CLIENT_ID`, `SLACK_CLIENT_SECRET`, `SLACK_SIGNING_SECRET`; redeploy. Users choose **Connect Slack → Allow**.
- **Phone:** finish [AgentPhone carrier registration](https://agentphone.ai/register) with the legal business name, EIN, address, consent details, and policy URLs. **+1 504 332 5746** and its webhook are already connected; do not recreate them. Users enter their number and verification code.
- **Email:** provide a dedicated mailbox with SMTP sending and IMAP receiving. Save the [mailbox settings](approval-channels.md#email) in Render and redeploy. Users confirm their verified Google email.

Slack's callbacks are prefilled; a workspace administrator may need to allow installation. Phone can receive now but cannot send until registration clears. The EIN was not found in the supplied document. Email and Slack do not depend on carrier registration.

## Release work still owned by the maintainer

- Publish matching native archives and installers after platform checks; verify the exact command copied from the hosted app on a clean machine. Existing-binary and mirrored-install tests pass, but they do not make public download URLs exist. Complete source/publication review before making the repository public.
- Run a live two-person flow: Google sign-in → connect a channel → accept contact → connect an agent → ask for a file → approve from the message → verify the received bytes. Repeat the approval and confirm that work runs only once. Run this on each channel before advertising it as operational.
- Finish live validation for the [agent/platform matrix](agents.md), especially cloud Devin and Linux/Windows startup. Local fixtures do not establish all harness behavior.
- Keep an encrypted backup outside the Render disk and exercise recovery. The verified pre-deploy backup at `/data/backups/before-ui-20261015` is on the same disk as the app. Follow [backup and restore](backup.md).
- Resolve the remaining [release requirements](release-readiness.md), including account recovery/linking, retention/deletion, and generic outbound A2A agent routing. These are engineering gaps, not tasks for end users.

The deployed build is `45da8fe` (`0.1.0-alpha.4`), on the approved 1 CPU / 2 GB / 100 GB service. Auto-deploy is off. Desktop/mobile UI, accessibility, contact/transfer, copied-command MCP setup, and real macOS background-service tests passed locally. Provider fixtures were used for Google, Slack, email, and deterministic agent responses; live outbound delivery remains unverified.
