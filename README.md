# Litecom

**Ask someone’s agent. They approve. It gets done.**

> “Ask Moe for the launch notes.”\
> “Ask my agent to find the final PDF.”\
> “Ask Ishaan to give me GitHub access.”

Host a workspace where people can send requests to each other's coding agents. Owners approve through **Slack, phone, or email**. Their agents do the work and return the reply and files.

```mermaid
flowchart LR
  A[Send a request] --> B[Owner approves]
  B --> C[Their agent does the work]
  C --> D[Receive the result]
```

## Host Litecom

**Start the server → connect Google sign-in → connect one approval channel → invite your team.**

You need Git, Docker with Compose, and a domain pointing to your server. Open ports **80 and 443**. Compose sets up persistent storage and HTTPS. Requests run through your users’ existing coding agents.

### 1. Start the server

```sh
git clone https://github.com/BerriAI/litecom.git
cd litecom
cp .env.example .env
chmod 600 .env
```

Set these values in `.env`:

```dotenv
LITECOM_HOST=litecom.example.com
ACME_EMAIL=you@example.com
```

Use your domain and an email address for certificate notices, then start:

```sh
docker compose up -d --build
```

### 2. Connect Google sign-in

Create a **Web application** OAuth client in [Google Auth Platform](https://console.cloud.google.com/auth/clients). Set its redirect URI to `https://YOUR-DOMAIN/api/auth/google/callback`, then add its credentials to `.env`:

```dotenv
GOOGLE_CLIENT_ID=your_client_id
GOOGLE_CLIENT_SECRET=your_client_secret
GOOGLE_ALLOWED_DOMAINS=yourcompany.com
```

Use your team's Google Workspace domain to restrict who can join. Leave `GOOGLE_ALLOWED_DOMAINS` empty to allow any verified Google account. [Google setup details](docs/self-host.md#enable-google-sign-in).

### 3. Connect one approval channel

Pick one to start; add others later. Save its credentials in `.env`.

- **[Slack](docs/hosting-channels.md#slack):** open `https://YOUR-DOMAIN/setup/slack`, create the prefilled app, and copy its three credentials.
- **[Email](docs/hosting-channels.md#email):** connect a sending server and a dedicated reply mailbox.
- **[Phone](docs/hosting-channels.md#phone):** get an AgentPhone number, complete carrier registration, and connect its webhook.

### 4. Check it, then invite your team

Apply the credentials and check the server:

```sh
docker compose up -d
docker compose ps
```

The `relay` service should be **healthy** and `https` should be running. Open your domain, choose **Continue with Google**, and connect your approval channel.

For a first check, [connect your agent](docs/setup.md#connect-your-local-agent), ask it for a short reply, and approve from the message. Once the result arrives, share your workspace URL with your team.

[Other hosting options](docs/self-host.md) · [Troubleshooting](docs/self-host.md#check-a-problem) · [Backups](docs/backup.md)

## Your team’s setup

Users open your link, sign in with Google, and connect one of the approval channels you enabled. They can then find people and send contact requests.

To connect an agent, they choose **Connect local agent**, copy one command, and run it in their working folder. It connects their account, configures MCP, and starts the background worker. [User setup and agent requirements](docs/setup.md).

The command installs the native Litecom executable when needed. Users do not need Rust or a source checkout. Joining the workspace and asking contacts does not require a local installation.

## Permissions

One approval authorizes the request and delivery of its result. The agent starts after approval and uses its existing tools and accounts. An additional tool permission can require another decision through the same channel. Offline local agents receive queued requests when they reconnect.

Downloads require sign-in. Owners can revoke future access; files already downloaded cannot be recalled. The server stores encrypted content, and its operator can decrypt it. [Security and permissions](SECURITY.md).

[Documentation](docs/README.md) · [Open-source components](docs/dependencies.md) · [Contributing](CONTRIBUTING.md)

Apache-2.0 licensed.
