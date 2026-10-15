# Run your own Litecom server

Set up **one shared server and one sending phone number** for everyone. This page is for the person running that service. People joining it follow [Connect your agent](setup.md); they do not need a hosting account, AgentPhone account, API key, or their own sending number.

## What you set up once

1. **A running server:** one Litecom relay, an HTTPS address, and a persistent data directory. Use the downloadable executable or the Docker image built from this repository on your preferred host.
2. **A sending number:** one AgentPhone agent and two-way number, with its signed webhook connected to the relay. The [phone setup](approval-channels.md#phone) walks through the four environment values.
3. **A working first request:** sign in, connect your coding agent, and ask it for a short reply. Receive the approval text, reply to approve, and check the delivered result.
4. **A backup and a handoff:** back up the data directory and encryption key, then give people your server's setup command. Add Slack and email only when you want them.

There is no separate database service, model account, or coding agent required on the relay. It stores accounts, requests, approvals, and transfers. Local work runs in each user's installed agent; connected cloud agents run at their provider.

## What each person does

Send them a [Litecom download](distribution.md) and this command, replacing `YOUR-DOMAIN` with your server:

```sh
./litecom setup --relay https://YOUR-DOMAIN
```

They choose **Phone**, **Slack**, or **Email** for approvals, verify their phone, and pick an installed coding agent and working folder. Litecom connects MCP and starts its background worker. This happens once per computer; normal requests need only the message approval. People add each other by sharing a contact link.

The server address is the only hosting detail in that command. A distribution built with your live `LITECOM_DEFAULT_RELAY` can omit `--relay`. Hosting and messaging credentials never go to end users.

## Choose a host

Any host that can run a persistent process or container works. Keep **one relay instance** with a persistent writable directory for SQLite, files, and its encryption key. Storage must survive restarts and deployments. Put HTTPS in front of port 4141, pass the public HTTPS address as `--public-url`, and use `/health` for the host's health check. Avoid sleeping or ephemeral instances: they interrupt callbacks and background work or lose accounts and files.

For a host that already provides HTTPS, use the [same native download](distribution.md#self-hosting-uses-the-same-download) with your provider environment values:

```sh
./litecom relay --listen 0.0.0.0:4141 \
  --public-url https://YOUR-DOMAIN --data /data/relay
```

Mount your persistent storage at `/data` and configure the host to restart the process. With the repository's Dockerfile, the executable is already the entrypoint; supply `relay --listen 0.0.0.0:4141 --public-url https://YOUR-DOMAIN --data /data/relay` as its command. The process runs as UID 65532, which must be able to write to the mounted directory. Keep backups outside that directory too.

The Compose route below adds HTTPS for a server you manage yourself. Public container images and a hosted default are not published. [Deployment validation](release-readiness.md) records the checks completed.

## Start with Docker Compose

You need this checkout, Docker with Compose, and a server with a domain pointing to it. Allow inbound ports **80 and 443**. You also need an AgentPhone account with a sender enabled for the countries you support. [Provider setup](approval-channels.md) covers the required account values and optional Slack/email delivery.

From the repository folder:

```sh
cp .env.example .env
chmod 600 .env
```

Edit `.env`. Set your domain in `LITECOM_HOST`, your certificate contact address in `ACME_EMAIL`, and the four AgentPhone values from [phone setup](approval-channels.md#phone). Leave the SMTP and Slack values empty until you want those approval channels.

```sh
docker compose up -d --build
docker compose ps
```

The relay should become **healthy** and the HTTPS service should be running. The initial build takes longer than later starts. Caddy obtains and renews HTTPS certificates for your domain; the relay's internal port is not published. [Caddy's domain requirements](https://caddyserver.com/docs/automatic-https)

Open `https://YOUR-DOMAIN` using the domain you configured. Sign in with your phone, then connect a client:

```sh
litecom setup --relay https://YOUR-DOMAIN
```

Ask your own agent for a short reply. Approve once in a connected channel, then receive the result. Seeing that reply verifies the useful path beyond container health. Real message delivery and your chosen agent's login must work for this check to pass.

The deployment has two services: **Litecom**, which stores its SQLite database and encryption key in the `relay-data` volume, and **Caddy**, which stores certificates in its own volumes. Local agents continue to run on their owners' computers. The server does not gain access to those computers' files by being deployed.

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

There is no released-version upgrade path yet. Before changing a running source deployment, make and verify a backup. Do not assume an older executable can open a database from a newer release.
