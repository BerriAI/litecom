# Install Litecom

Litecom uses one executable for the client, background worker, and shared server. Your coding agent needs its own installation and sign-in.

## Connect from your workspace

Open your host's Litecom link, sign in, and choose **Connect local agent → Copy command**. Paste the command into any terminal and press Enter. New connections use your home folder (`~`).

The command downloads the native executable for your computer, connects your account, configures MCP, and starts the background worker. You do not need Rust or a source checkout. Wait for **Connected** in the browser and **Listening for requests** in the terminal, then close the terminal. [Full setup guide](setup.md).

Downloads support Apple Silicon and Intel Macs, ARM64 and x86-64 Linux, and x86-64 Windows. Linux needs glibc 2.35 or later and a systemd user session for background startup. Your coding agent must already be installed and signed in.

Joining a workspace and asking a workspace member's agent does not require a local installation.

## Install the executable yourself

Use this route to run a server or install without a workspace connection command. On macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/BerriAI/litecom/releases/download/v0.1.0-alpha.24/litecom-installer.sh \
  -o /tmp/litecom-installer.sh
sh /tmp/litecom-installer.sh
```

On Windows, run in PowerShell:

```powershell
& ([scriptblock]::Create((Invoke-RestMethod https://github.com/BerriAI/litecom/releases/download/v0.1.0-alpha.24/litecom-installer.ps1)))
```

Follow the installer's PATH instructions, then run `litecom --version`. You can also download an archive and its checksum from [Releases](https://github.com/BerriAI/litecom/releases/tag/v0.1.0-alpha.24).

## Update an existing installation

Run this in your own terminal or existing agent chat:

```sh
litecom update
```

`litecom upgrade` is an alias. The command downloads the newest published release (including alpha releases), verifies its checksum, and updates both the CLI and installed worker/MCP executable. It pauses new requests, waits up to ten minutes for active requests to finish, and restarts only the background workers that were running. Saved connections, agent choices, folders, MCP configuration, and service environment stay in place. No sign-in or pairing is needed.

Use `litecom update --check` to check without installing, or `litecom update --wait 1800` to allow thirty minutes for active work. Run it outside a Litecom-dispatched request: that request itself would keep its worker busy. If an update is interrupted, the server's temporary pause expires within three minutes. A failed download leaves the installation alone; a failed replacement restores the previous executables. A restart failure lists the connections that need attention.

The server must support updates (alpha.24 or later). Self-hosters should upgrade the server first. This command does not restart a self-hosted relay or manually started `litecom worker` processes; restart those through your normal process manager when idle.

Already-open chats can keep using CLI tools. Start a new Codex, Claude Code, or other agent chat when you want its refreshed MCP tool catalog. Updating does not grant additional operating-system permissions; run `litecom approve` if you previously skipped permission setup.

### Upgrading from alpha.23 or earlier

Older builds do not have the update command. On macOS or Linux, run the following once, using your team's Litecom URL:

```sh
curl -fsSL https://YOUR-SERVER/install.sh | sh -s -- update
```

This installs the new command and runs it against your saved connections. On Windows, run the versioned PowerShell installer above, then `litecom update`. Future upgrades use `litecom update` on every platform. Litecom does not install updates automatically.

Local connections have no routine sign-in expiry. If a connection was revoked or its credentials were lost, run `litecom reconnect`; it keeps the saved agent and folder. Browser sessions expire separately and offer **Sign in again** on the current page.

## Build from source

You need Git, Rust 1.88 or later, and access to the repository:

```sh
git clone https://github.com/BerriAI/litecom.git
cd litecom
cargo install --locked --path .
litecom --version
```

Cargo installs Litecom in its binary directory, usually `~/.cargo/bin`. Keep that directory on your PATH. `litecom --version` should print the installed version.

To connect your agent, return to your workspace and follow [Connect your local agent](setup.md#connect-your-local-agent). For an existing installation, you can also run:

```sh
litecom setup --relay https://YOUR-SERVER
```

Replace the address with the Litecom link from your host.

## Try locally

With Litecom and a signed-in coding agent installed:

```sh
litecom try
```

Choose **Phone**, use the verification code printed in the terminal, and select your agent. Approve the sample request and download its result. Your agent's normal usage charges apply. Messages stay local; **Ctrl+C** stops the tryout and removes its temporary data.

The tryout does not install a background service or change your agent's MCP settings. Native tools keep their existing permissions.

## Run a local server

For a server that keeps its data across restarts:

```sh
litecom relay --public-url http://127.0.0.1:4141 --data .litecom/dev --dev
```

Open <http://127.0.0.1:4141>. Leave the terminal open; it displays verification codes and notifications. Keep `.litecom/dev`, which contains the database and its encryption key.

Use [self-hosting](self-host.md) for a shared server with HTTPS and real notifications. The same executable supports both setups.
