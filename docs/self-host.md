# Run your own Litecom server

Configure **hosting, Google sign-in, and at least one approval channel**. Users open your link, sign in, and connect their own destination. Keep provider credentials on the server.

[Google sign-in](#enable-google-sign-in) · [Approval channels](#enable-approval-channels) · [Hosting](#choose-a-host) · [Docker Compose](#start-with-docker-compose) · [Troubleshooting](#check-a-problem)

## What you set up once

| Part | Host setup | What users do |
| --- | --- | --- |
| Server | Run Litecom at an HTTPS address with persistent storage | Open your link |
| Google | Create a web OAuth client; save two credentials and the callback below | Continue with Google |
| Slack, optional | Create the prefilled app; save its three credentials | Connect Slack → Allow |
| Email, optional | Provide SMTP delivery and a dedicated IMAP reply mailbox | Confirm their Google email address |
| Phone, optional | Get an AgentPhone number, finish carrier registration, and connect its webhook | Enter their number and verification code |

Choose any combination of channels. Litecom only offers configured channels with incoming approval support. Connecting a provider is not proof of delivery: test a complete request and approval before inviting users.

There is no separate database service, model account, or coding agent required on the relay. Local work runs through each person’s existing agent. Cloud work runs at their connected provider.

## Enable Google sign-in

Litecom connects to Google through OAuth and OpenID Connect.

1. In [Google Cloud’s Clients page](https://console.cloud.google.com/auth/clients), create an OAuth client of type **Web application**. Complete your app’s branding and audience settings if Google asks.
2. Add this authorized redirect URI, replacing the domain with your public server address:

   ```text
   https://YOUR-DOMAIN/api/auth/google/callback
   ```

3. Set these values on the relay and restart it:

   ```dotenv
   GOOGLE_CLIENT_ID=your_web_client_id
   GOOGLE_CLIENT_SECRET=your_web_client_secret
   GOOGLE_ALLOWED_DOMAINS=yourcompany.com
   ```

`GOOGLE_ALLOWED_DOMAINS` is optional and accepts comma-separated Google Workspace domains. Leave it empty only if any verified Google account may join this server and see its people/agent directory. Domain restrictions are checked against Google’s verified Workspace claim and enforced on existing account connections too. Restricting domains disables phone-only signup.

Keep the client secret on the server. Use Google’s audience/publishing controls to make sign-in available to the intended users. The callback must exactly match the configured URL. [Google’s setup reference](https://developers.google.com/identity/openid-connect/openid-connect#settingup).

## Enable approval channels

Configure at least one of [Slack, Email, or Phone](hosting-channels.md). For Slack, opening `https://YOUR-DOMAIN/setup/slack` prefills app creation with the required scopes and callbacks. Email requires both outgoing delivery and incoming replies. Phone requires an outbound-enabled number; start carrier registration early.

Save a backup of the data and encryption key, then test sign-in, channel connection, one request, message approval, and the final result. Send users your server’s homepage link. Their [setup guide](setup.md) covers every route and separates provider prompts from Litecom’s click counts.

## What users do

**Google → one approval channel → workspace.** They can ask anyone in the workspace immediately. Connecting their own agent is optional: **Agents → Connect local agent → Copy command → paste and run**. The command connects that browser account, configures MCP, and starts the background worker.

The generated command downloads the matching native executable when needed. Users do not need Rust or a source checkout. See [installation options](distribution.md) for supported systems and manual installation.

## Choose a host

Any host that can run a persistent process or container works. Keep **one relay instance** with a persistent writable directory for SQLite, files, and its encryption key. Storage must survive restarts and deployments. Put HTTPS in front of port 4141, pass the public HTTPS address as `--public-url`, and use `/health` for the host's health check. Avoid sleeping or ephemeral instances: they interrupt callbacks and background work or lose accounts and files.

For a host that provides HTTPS, use the [native executable](distribution.md) with your provider environment values:

```sh
./litecom relay --listen 0.0.0.0:4141 \
  --public-url https://YOUR-DOMAIN --data /data/relay
```

Mount your persistent storage at `/data` and configure the host to restart the process. With the repository's Dockerfile, the executable is already the entrypoint; supply `relay --listen 0.0.0.0:4141 --public-url https://YOUR-DOMAIN --data /data/relay` as its arguments. If your host overrides the entrypoint as well as the arguments, include `/usr/local/bin/litecom` at the beginning. The process runs as UID 65532, which must be able to write to the mounted directory. Keep backups outside that directory too.

The Compose route below builds the container from source and adds HTTPS for a server you manage yourself.

## Start with Docker Compose

You need the [source checkout](https://github.com/BerriAI/litecom), Docker with Compose, and a server with a domain pointing to it. Allow inbound ports **80 and 443**. You also need Google OAuth credentials and at least one configured approval provider. [Provider setup](hosting-channels.md) covers the required values.

Clone the repository and prepare its configuration:

```sh
git clone https://github.com/BerriAI/litecom.git
cd litecom
cp .env.example .env
chmod 600 .env
```

Edit `.env`. Set your domain in `LITECOM_HOST`, your certificate contact address in `ACME_EMAIL`, the Google values above, and your chosen approval channel values. Leave unused channels empty; a phone number is not required for a Google + Slack or Email deployment.

```sh
docker compose up -d --build
docker compose ps
```

The relay should become **healthy** and the HTTPS service should be running. The initial build takes longer than later starts. Caddy obtains and renews HTTPS certificates for your domain; the relay's internal port is not published. [Caddy's domain requirements](https://caddyserver.com/docs/automatic-https)

Open `https://YOUR-DOMAIN` using the domain you configured. Sign in with Google, choose your approval channel, then use **Connect local agent** to obtain the complete client command.

Ask your own agent for a short reply. Approve once in a connected channel, then receive the result. Seeing that reply verifies the useful path beyond container health. Real message delivery and your chosen agent's login must work for this check to pass.

The deployment has two services: **Litecom**, which stores its SQLite database and encryption key in the `relay-data` volume, and **Caddy**, which stores certificates in its own volumes. Local agents continue to run on their owners' computers. The server does not gain access to those computers' files by being deployed.

## Resource limits

The server applies these defaults without extra setup. Set a positive integer in the relay environment to change a limit, then restart it. Request counts and stored-file accounting survive restarts; retries of the same request do not consume another slot.

| Environment variable | Default | Limit |
| --- | --- | --- |
| `LITECOM_REQUESTS_PER_MINUTE` | `60` | New requests per person per minute |
| `LITECOM_PENDING_REQUESTS` | `200` | Unfinished requests per person |
| `LITECOM_SLACK_PER_MINUTE` | `30` | Slack mentions per sender per minute |
| `LITECOM_PENDING_SLACK_JOBS` | `1000` | Unfinished Slack intake jobs |
| `LITECOM_MAX_FILE_BYTES` | `1073741824` | 1 GiB per file |
| `LITECOM_MAX_TASK_FILE_BYTES` | `5368709120` | 5 GiB per request |
| `LITECOM_MAX_STORED_FILE_BYTES` | `53687091200` | 50 GiB of stored attachments |

Uploads allow two concurrent transfers per account and eight per server. Leave disk headroom for those temporary encrypted uploads, SQLite, logs, and backups. The attachment limit does not cover those files. Rate-limited clients receive `Retry-After: 60`; storage-limit errors keep existing files intact.

## Check a problem

```sh
docker compose logs --tail=100 relay https
```

- **Relay fails to start:** check the reported environment value and volume permissions. Missing storage keys must be restored, not replaced.
- **HTTPS fails:** check domain resolution, inbound ports, and Caddy's logs. Do not disable certificate verification on clients.
- **Phone code does not arrive:** check the AgentPhone sender configuration and provider delivery status. A healthy relay does not prove SMS delivery.
- **Agent is offline:** check its computer and run `litecom doctor` there. Requests for an offline local agent remain queued.

`docker compose stop` stops services and preserves data. `docker compose down` removes containers and networks but preserves named volumes. Adding `--volumes` deletes those volumes; it is not part of routine operation.

## Back up and recover

Follow [backup and restore](backup.md). The backup includes the database, stored account connections, and its matching encryption key. Keep a protected copy outside this server and separately preserve `.env`. Environment-based delivery credentials and TLS certificate files are not included in the database backup.

Before updating a running deployment, make and verify a backup. Do not assume an older executable can open a database written by a newer version.
