# Ask an agent from Slack

The optional hosted Slack agent turns `@Litecom` mentions into requests. It uses the **same Slack app** that sends approval DMs.

> `@Litecom can you ask @Moe whether the Lens fixes shipped and which PRs they're in?`

Litecom reads the conversation, asks Moe for approval, and posts his agent's response in the thread. Moe's agent does the work. The hosted Slack agent handles the handoff.

## Enable it once

Start with a running Litecom server and [connected Slack app](hosting-channels.md#slack). Leave these settings empty to keep approval DMs only.

1. **Choose a model that supports tool calling.** Add these values to the relay environment:

   ```dotenv
   SLACK_AGENT_BASE_URL=https://YOUR_MODEL_PROVIDER/v1
   SLACK_AGENT_MODEL=YOUR_MODEL_NAME
   SLACK_AGENT_API_KEY=YOUR_MODEL_API_KEY
   ```

   Use the provider's OpenAI-compatible base URL, not a `/chat/completions` URL. For a LiteLLM gateway, use its base URL and configured model name. Supply an API key if the provider requires authentication. HTTP is allowed only on localhost. These are host settings; users never enter model keys.

   Restart the relay after saving. With Docker Compose:

   ```sh
   docker compose up -d --build
   ```

2. **Copy the generated Slack configuration.** Open `https://YOUR_RELAY_HOST/setup/slack/manifest?agent=1` and copy the JSON. At [Slack app settings](https://api.slack.com/apps), select your existing **Litecom** app, open **App Manifest**, choose **JSON**, paste it, and save. Litecom fills in the permissions and callback URLs.

3. **Reconnect Slack in Litecom Settings** and allow the additional permissions. Invite the existing Litecom app into the channels where people will use it. No second app is needed.

For a new Slack app, open `https://YOUR_RELAY_HOST/setup/slack?agent=1` to start with the configuration prefilled. Save its credentials using [Slack setup](hosting-channels.md#slack), then reconnect from Litecom. Existing installations should update their app instead of creating another.

The optional configuration adds `app_mentions:read` and `files:write` to the approval and read permissions in the base app. Slack sends mentions to `/webhooks/slack/events` and approval buttons to `/webhooks/slack`. If your app has other custom settings, keep them when reviewing the manifest changes.

## Use it

Each person signs in to Litecom and connects Slack once. Only someone receiving work needs a connected coding agent.

In a channel containing Litecom, mention the app and the person:

> `@Litecom ask @Moe's agent for the launch notes.`

You can also ask your own agent or name a particular connected agent, such as Moe's Devin. When a name or request is ambiguous, Litecom asks a short question. Include `@Litecom` in your reply so it receives the follow-up.

“Can you get this from Moe?” works when the conversation supplies enough context. The hosted agent can read that thread and preceding messages in the same channel. It asks for clarification when the reference is unclear. It cannot read other channels or fetch arbitrary links.

Litecom tags the recipient in the source thread and shows **Ask**, **From**, and **Where**, with **Approve read-only**, **Decline**, and **View request** buttons. Requests that need changes also offer **Approve actions**. The same controls arrive by DM. Only the recipient's linked Slack account can decide; other channel members cannot approve for them. The full request and conversation context are available under **View request**. The card keeps its summary as its status changes. Custom or changed agent proposals use the private review page.

Read-only approval allows retrieval; action approval also allows the agent to make changes. Both share the reply and requested files in the source thread. Results use Slack formatting; files arrive as Slack attachments. Channel members can access those copies under their workspace’s policies. If Slack cannot accept a file, **Download files** opens the authenticated Litecom download. Older requests approved before native file sharing retain their original download permissions. Results do not go back through the hosted model.

## Check it

Mention Litecom with a request for your own agent to reply “Connected.” Before approving, check that the request is waiting and no work has run. Approve in the source thread or Slack DM. One status message shows sent and approved; the final answer arrives as a new reply in the same thread. Then try a request referring to a previous message.

If it does not respond, check that the app is in the channel, Event Subscriptions shows **Verified**, and you have reconnected Slack after adding scopes. A request to connect your account means the sender's Slack identity has not been linked. A model error means the host should check the configured URL, model, credentials, and provider quota.

## What the host runs

The Rust relay uses [AgentChat](https://github.com/BerriAI/agentchat) for Slack API calls and message payloads. It runs the tool loop; there is no separate service or coding harness to install. Its tools list linked people, read messages in the source channel, and submit an ordinary Litecom request. It has no command execution, approval, file access, or account-management tools. Sender identity and the reply destination come from signed Slack events, not model output.

Slack context and the linked workspace directory are sent to the host's chosen model provider. Configure spending controls there. Litecom adds no model output-token cap.

Events and progress are encrypted in the existing database. Retries and restarts reuse the same request. A lost Slack acknowledgement can still produce a duplicate notification, but cannot approve or execute the work again. The recipient's existing offline and execution-recovery behavior applies.

To disable intake, clear the three `SLACK_AGENT_*` values and restart the relay. Existing approval DMs keep working. Requests already submitted remain in Litecom; queued Slack intake and thread updates resume if the host enables the feature again.
