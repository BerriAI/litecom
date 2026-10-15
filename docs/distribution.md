# Downloads and self-hosting

Litecom is packaged as one native executable. The client, background worker, MCP server, relay, database engine, and browser pages are bundled together. Litecom itself needs no Rust, Node, or separate database installation. Your coding agent and any adapter still need their own installation, runtime, and sign-in; for example, Pi's ACP adapter requires Node.js.

**Release status:** local archives can be built. Public downloads, signed installers, a hosted default, automatic updates, and clean-machine validation are not released yet.

## Intended setup

Download and open the installer. It starts the terminal setup:

1. Choose **Phone**, **Slack**, or **Email** for approvals.
2. Verify your phone account in the browser.
3. Confirm your main agent and working folder.

Litecom connects the agent and starts in the background. Ordinary users will not enter a server address or configure message delivery. Exact installation click counts depend on the final signed installers and must be measured before release. The current guided setup counts are in [Set up Litecom](setup.md#how-much-work-is-setup).

Running `litecom` without arguments now starts guided setup. Existing valid connections reuse their saved choices. `litecom setup --relay https://YOUR-SERVER` connects to a self-hosted server instead; replace `YOUR-SERVER` with the server address supplied by its operator.

## Self-hosting uses the same download

The same executable runs the relay. An operator can extract the archive and start a persistent local test without a compiler:

```sh
./litecom relay --public-url http://127.0.0.1:4141 --data ./litecom-data --dev
```

On Windows, use `./litecom.exe`. Open <http://127.0.0.1:4141>; the sign-in page confirms that the relay is running. Development verification codes print in the terminal and no messages are sent. Keep `litecom-data`: it contains the database and its encryption key.

A public deployment also needs HTTPS, configured [message delivery](approval-channels.md), persistent storage, and tested backup/recovery. The [Compose deployment](self-host.md) packages the relay and HTTPS service; Docker is optional when you use the executable directly. Both paths support the same [backup commands](backup.md).

## Build download archives

This section is for maintainers. Install [cargo-dist 0.33.0](https://github.com/axodotdev/cargo-dist/releases/tag/v0.33.0), then run from the checkout:

```sh
dist build --artifacts=local
```

The archive and SHA-256 checksum appear in `target/distrib/`. Use `--target` with a platform from `dist-workspace.toml` to select a build explicitly. Cross-compiling requires that platform's toolchain; native CI runners avoid that extra setup.

For an Apple Silicon build, verify the extracted executable with:

```sh
python3 scripts/check-package.py target/distrib/litecom-aarch64-apple-darwin.tar.gz
```

The check requires Python 3.12+. It runs the packaged CLI and relay with an empty executable search path, checks the bundled pages and database initialization, and removes its test data. It does not establish operating-system compatibility, signed installation, native agent access, or real provider delivery.

The manual **Build downloads** GitHub workflow builds archives for macOS ARM/Intel, Linux ARM/Intel, and Windows Intel, tests them on their native runners, and retains them as workflow artifacts. It does not publish a release. These targets remain unverified until that workflow and clean-machine checks pass.

The repository URL is configured as [BerriAI/litecom](https://github.com/BerriAI/litecom). When preparing the first release, run `dist init` to generate the release workflow and download installers. Set `LITECOM_DEFAULT_RELAY` when building a hosted-service release only after that HTTPS endpoint is operating. Self-hosted server selection remains available in every build.
