# Connect Devin

Devin runs in the cloud. Litecom’s relay monitors its work, so your laptop can be off. You need a Devin account with API access and an existing phone account on your Litecom server.

## Connect once

1. Open **Agents → Connect Devin** in Litecom.
2. Create or copy a token from [Devin Settings → API Keys](https://app.devin.ai/settings/api-keys), then paste it into **Devin token**.
3. Select **Connect Devin**.

Litecom detects the account and organization and checks access without starting a session. If the organization cannot be detected, open **Name and organization** and enter the organization ID from Devin Settings → Devin API. The default name is **Devin**; you can change it there too.

The relay stores your token encrypted. Reconnect with the same agent name to replace a token. Select **Make default** if “ask me” or “ask Moe” should choose Devin automatically. Otherwise select Devin by name.

## Ask and receive

From **Inbox → Ask an agent**, choose yourself or a contact, select **Devin**, and describe the request. From an already connected terminal:

```sh
litecom ask Moe "Find the release notes and attach the final PDF." --agent Devin
```

The owner reviews the request and connected Devin identity, then selects **Approve run**. A session starts with that organization’s existing context and default security settings. It can perform actions within its access; normal Devin charges apply.

If Devin needs a reply or permission, the owner follows **Open in Devin** on the request. When it finishes, Litecom prepares its reply and selected attachments for a separate sharing approval. Only then can the requester read and download them. Private session links stay with the owner.

## If interrupted

- A relay restart resumes a recorded session. It does not launch a second one.
- If a session launch cannot be confirmed, Litecom marks the result uncertain. Check Devin before creating a new request.
- Cancellation or disconnect asks Devin to stop. Until Devin confirms, Litecom keeps the cancellation pending and shows the owner a session link when available. Changes already made may remain.
- A changed connection requires a new request if its launch approval no longer matches.

The transport and relay worker are implemented. Live Devin account validation is still pending; see [release readiness](release-readiness.md). Provider setup steps have not been measured as a one-click flow.
