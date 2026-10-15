# Install Litecom

Litecom uses one executable for the client, background worker, and shared server. Your coding agent needs its own installation and sign-in.

## Installation availability

Native downloads are not published yet. If you have Litecom installed, use your workspace's **Connect local agent** command. Otherwise, build from source below. Joining a workspace and asking a contact's agent does not require a local installation.

The [Releases page](https://github.com/BerriAI/litecom/releases) is the destination for native downloads. The browser's automatic installer needs a published release that matches the server version.

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
