# Run your own Litecom server

This is the operator path. People connecting to your server only need its URL; they do not configure a database, messaging provider, or GitHub integration.

The Compose deployment builds the source and adds HTTPS. Public container images and a hosted default are not published. [Deployment validation](release-readiness.md) records the checks completed.

## Start with Docker Compose

You need this checkout, Docker with Compose, and a server with a domain pointing to it. Allow inbound ports **80 and 443**. You also need a Twilio Messaging Service with a sender enabled for the countries you support. [Provider setup](approval-channels.md) covers the required account values and optional Slack/email delivery.

From the repository folder:

```sh
cp .env.example .env
chmod 600 .env
```

Edit `.env`. Set your domain in `LITECOM_HOST`, your certificate contact address in `ACME_EMAIL`, and the three Twilio values. Leave the SMTP and Slack values empty until you want those approval channels.

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
- **Phone code does not arrive:** check the Twilio sender/service configuration and provider delivery status. A healthy relay does not prove SMS delivery.
- **Agent is offline:** check its computer and run `litecom doctor` there. Requests for an offline local agent remain queued.

`docker compose stop` stops services and preserves data. `docker compose down` removes containers and networks but preserves named volumes. Adding `--volumes` deletes those volumes; it is not part of routine operation.

## Back up and recover

Follow [backup and restore](backup.md). The backup includes the database, stored account connections, and its matching encryption key. Keep a protected copy outside this server and separately preserve `.env`. Environment-based delivery credentials and TLS certificate files are not included in the database backup.

There is no released-version upgrade path yet. Before changing a running source deployment, make and verify a backup. Do not assume an older executable can open a database from a newer release.
