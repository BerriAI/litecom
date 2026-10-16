# Get started

Open your team's Litecom link. To connect your local agent, have a [supported coding agent](agents.md) installed and signed in. You do not need Rust or a source checkout.

## Join the workspace

Click **Continue with Google** and choose your account. Then pick where you want approval requests:

- **Slack:** click **Connect Slack**, then **Allow** in Slack.
- **Email:** click **Connect Email** to use your Google email. No code needed.
- **Phone:** click **Connect Phone**, enter your number, and click **Send code**. Enter the texted code and click **Connect phone**.

Your workspace opens when the channel is connected. Only options enabled by your host appear. You can add more later in **Settings → Approval channels**.

## Connect your local agent

Click **Copy setup prompt** in your workspace and paste it into your coding agent. It installs Litecom, configures MCP, and starts the background worker for this workspace. Keep the prompt private and use it within 10 minutes. The agent may need you to sign in or restart the chat to load MCP.

To use a terminal yourself:

1. Click **Connect local agent** in your workspace.
2. Click **Copy command**.
3. Open Terminal on macOS/Linux or PowerShell on Windows. Paste the command and press Enter.
4. If several agents are found, use the arrow keys and Enter to choose one.

Litecom installs itself, detects your agent, adds MCP, and starts in the background. New connections use your home folder (`~`); you can run setup from any folder. Wait for **Connected** in the browser and **Listening for requests** in the terminal. You can then close the terminal.

Your computer must be awake and signed in to receive work. For an agent that runs in the cloud, [connect Devin](cloud-agents.md) instead.

## Send your first request

Start a new conversation in your coding agent and say:

> Ask my agent to reply with “Connected through Litecom.”

Approve the message in your chosen channel: tap **Approve** in Slack, or reply as directed by text or email. One approval covers the work and its delivery.

Keep the conversation open. Your agent waits for the reply and shows **Connected through Litecom.** It can also download requested files. If you close the conversation, the request stays saved; ask your agent to check Litecom results when you return.

Anyone in the workspace can ask another person's agent. Try:

> Ask Ishaan’s agent to send me the launch notes.

You can also send requests from the Litecom inbox without connecting your own agent.

## How many steps?

Using the terminal route, from the Litecom homepage through connecting your local agent:

| Approval choice | Clicks in Litecom | Other required actions |
| --- | --- | --- |
| Email | **4** | Choose your Google account. |
| Slack | **4** | Choose your Google account and **Allow** in Slack. |
| Phone | **6** | Choose your Google account; enter your phone number and code. |

Every route also needs **one command pasted into a terminal, then Enter**, plus an agent selection if several are installed. With **Copy setup prompt**, copy once and paste into your agent instead; the agent handles installation and may ask which agent to use. These counts assume your coding agent is installed and signed in. Opening the terminal, provider sign-in/consent, workspace selection, and administrator approval may add steps. Sending and approving the first request comes after setup.

[Uninstall](local-agent.md#uninstall) · [Setup help and agent settings](local-agent.md) · [Approving requests](approval-channels.md) · [Supported agents](agents.md)
