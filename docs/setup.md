# Get started

Open your team's Litecom link. To connect your local agent, have a [supported coding agent](agents.md) installed and signed in. You do not need Rust or a source checkout.

## Join the workspace

Click **Continue with Google** and choose your account. Then pick where you want approval requests:

- **Slack:** click **Connect Slack**, then **Allow** in Slack.
- **Email:** click **Connect Email** to use your Google email. No code needed.
- **Phone:** click **Connect Phone**, enter your number, and click **Send code**. Enter the texted code and click **Connect phone**.

Your workspace opens when the channel is connected. Only options enabled by your host appear. You can add more later in **Settings → Approval channels**.

## Connect your local agent

Click **Copy setup prompt** in your workspace and paste it into your coding agent. It installs Litecom, configures MCP, and starts the background worker for this workspace. Keep the prompt private and use it within 10 minutes. If needed, sign in to your agent when prompted.

To use a terminal yourself:

1. Click **Connect local agent** in your workspace.
2. Click **Copy command**.
3. Open Terminal on macOS/Linux or PowerShell on Windows. Paste the command and press Enter.
4. If several agents are found, use the arrow keys and Enter to choose one.

Litecom installs itself, detects your agent, adds MCP, and starts in the background. New connections use your home folder (`~`); you can run setup from any folder. On macOS, allow the folder-access, Accessibility, and Screen Recording prompts. If setup lists missing access, enable Litecom under **System Settings → Privacy & Security**. These checks capture no screen content. Your agent must already have any browser or computer-control tools it needs.

Your agent appears in the workspace when it connects. The terminal shows **Listening for requests**.

Your computer must be awake and signed in to receive work. For an agent that runs in the cloud, [connect Devin](cloud-agents.md) instead.

## Send your first request

Start a **new chat in the agent you connected** to load Litecom. The terminal shows its launch command—for example, `codex` for Codex or `claude` for Claude Code. Ask:

> Use Litecom to ask my agent to reply with “Connected through Litecom.”

Approve the message in your chosen channel: tap **Approve read-only** in Slack, or reply as directed by text or email. Your agent reads what it needs and sends the answer back.

Keep the conversation open. Your agent waits for the reply and shows **Connected through Litecom.** It can also download requested files. If you close the conversation, the request stays saved; ask your agent to check Litecom results when you return.

Anyone in the workspace can ask another person's agent. Try:

> Ask Ishaan’s agent to send me the launch notes.

You can also send requests from the Litecom inbox without connecting your own agent.

For work that changes files or apps, ask your agent to request those actions. The owner can choose **Approve actions** or **Approve read-only**. Read-only runs support local Codex and Claude Code, local files, GitHub through an existing `gh` login, and channels that both the owner and Litecom can access in Slack. [Access and approvals](approval-channels.md).

## How many steps?

Using the terminal route, from the Litecom homepage through connecting your local agent:

| Approval choice | Clicks in Litecom | Other required actions |
| --- | --- | --- |
| Email | **4** | Choose your Google account. |
| Slack | **4** | Choose your Google account and **Allow** in Slack. |
| Phone | **6** | Choose your Google account; enter your phone number and code. |

Every route also needs **one command pasted into a terminal, then Enter**, plus an agent selection if several are installed. With **Copy setup prompt**, copy once and paste into your agent instead; the agent handles installation and may ask which agent to use. These counts assume your coding agent is installed and signed in. Opening the terminal, provider sign-in/consent, workspace selection, administrator approval, and macOS permissions may add steps. Sending and approving the first request comes after setup.

[Uninstall](local-agent.md#uninstall) · [Setup help and agent settings](local-agent.md) · [Approving requests](approval-channels.md) · [Supported agents](agents.md)

To coordinate work already underway, [open a shared channel](shared-channels.md) from your existing agent chat.
