# Litecom launch handoff

October 15, 2026 · [App](https://litecom.onrender.com) · [Source](https://github.com/BerriAI/litecom) · [Render service](https://dashboard.render.com/web/srv-db8hglo473hc73evp9eg)

Google sign-in and Slack approvals are live. A real Slack approval triggered one test execution and returned the exact file automatically. The source repository is **Internal** and there is **no published native release** yet; fresh installation still needs that release.

## Google sign-in is connected

The OAuth client is created, its credentials are saved in Render, and a real Google sign-in reached Litecom’s approval-channel picker on October 15. No further client creation is needed.

The callback is `https://litecom.onrender.com/api/auth/google/callback`; Litecom requests only `openid email profile`. Access from outside Berri has not been verified. Before opening the workspace more broadly, review **Audience** in [Google Auth Platform](https://console.cloud.google.com/auth/audience) and the [workspace access settings](self-host.md#enable-google-sign-in). `GOOGLE_ALLOWED_DOMAINS` is currently unset.

## Slack is connected

The **Litecom** app is installed in **BerriAI**, its credentials are saved in Render, and Moe's Google account is linked. New users choose **Connect Slack → Allow** after Google sign-in. No phone number is required. The app requests only `chat:write` and `im:write`.

On October 15, Moe tapped **Approve** on the real Slack DM. The hosted relay accepted the signed callback, the temporary worker ran once, and Litecom returned `slack-test.txt` automatically. Its bytes and SHA-256 matched. This used a deterministic test program, not a model call. The worker is stopped and its connection credential is revoked; the Slack account remains linked.

## Optional approval channels

Slack is enough to start. These can be added later.

- **Phone:** finish [AgentPhone carrier registration](https://agentphone.ai/register) with the legal business name, EIN, address, consent details, and policy URLs. **+1 504 332 5746** and its webhook are already connected; do not recreate them. Users enter their number and verification code.
- **Email:** provide a dedicated mailbox with SMTP sending and IMAP receiving. Save the [mailbox settings](approval-channels.md#email) in Render and redeploy. Users confirm their verified Google email.

Phone can receive now but cannot send until registration clears. The EIN was not found in the supplied document. Email and Slack do not depend on carrier registration. Other Slack workspaces require app distribution settings and may require administrator approval.

## Release work still owned by the maintainer

- Publish matching native archives and installers after platform checks; verify the exact command copied from the hosted app on a clean machine. Existing-binary and mirrored-install tests pass, but they do not make public download URLs exist. Complete source/publication review before making the repository public.
- Run a live two-person flow: Google sign-in → connect a channel → accept contact → connect an agent → ask for a file → approve from the message → verify the received bytes. Repeat the approval and confirm that work runs only once. Run this on each channel before advertising it as operational.
- Finish live validation for the [agent/platform matrix](agents.md), especially cloud Devin and Linux/Windows startup. Local fixtures do not establish all harness behavior.
- Keep an encrypted backup outside the Render disk and exercise recovery. The verified pre-deploy backup at `/data/backups/before-ui-20261015` is on the same disk as the app. Follow [backup and restore](backup.md).
- Resolve the remaining [release requirements](release-readiness.md), including account recovery/linking, retention/deletion, and generic outbound A2A agent routing. These are engineering gaps, not tasks for end users.

The deployed build is `45da8fe` (`0.1.0-alpha.4`), on the approved 1 CPU / 2 GB / 100 GB service. Auto-deploy is off. Desktop/mobile UI, accessibility, contact/transfer, copied-command MCP setup, and real macOS background-service tests passed locally. Google sign-in and Slack linking, delivery, and approval pass against live providers. Email and outgoing phone delivery remain unverified.
