# Install Litecom

Litecom uses one executable for the client, background worker, and shared server. Your coding agent needs its own installation and sign-in.

## Connect from your workspace

Open your host's Litecom link, sign in, and choose **Connect local agent → Copy command**. Paste the command into a terminal in your agent's working folder and press Enter.

The command downloads the native executable for your computer, connects your account, configures MCP, and starts the background worker. You do not need Rust or a source checkout. Wait for **Connected** in the browser and **Listening for requests** in the terminal, then close the terminal. [Full setup guide](setup.md).

Downloads support Apple Silicon and Intel Macs, ARM64 and x86-64 Linux, and x86-64 Windows. Linux needs glibc 2.35 or later and a systemd user session for background startup. Your coding agent must already be installed and signed in.

Joining a workspace and asking a contact's agent does not require a local installation.

## Install the executable yourself

Use this route to run a server or install without a workspace connection command. On macOS or Linux:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/BerriAI/litecom/releases/download/v0.1.0-alpha.7/litecom-installer.sh \
  -o /tmp/litecom-installer.sh
sh /tmp/litecom-installer.sh
```

On Windows, run in PowerShell:

```powershell
& ([scriptblock]::Create((Invoke-RestMethod https://github.com/BerriAI/litecom/releases/download/v0.1.0-alpha.7/litecom-installer.ps1)))
```

Follow the installer's PATH instructions, then run `litecom --version`. You can also download an archive and its checksum from [Releases](https://github.com/BerriAI/litecom/releases/tag/v0.1.0-alpha.7).

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
