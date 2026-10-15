# Back up and restore

A Litecom backup is a folder containing the SQLite snapshot, its encryption key, and a checksum manifest. It includes accounts, requests, replies, attachments, and stored provider connections. **Anyone with the whole backup can decrypt that data.** Store it privately and keep a protected copy outside the relay machine. Checksums detect accidental changes; they do not authenticate a backup supplied by someone else.

The backup command can run while the relay is online. It uses SQLite's snapshot API and includes committed data still in the write-ahead log. Copying only a live `litecom.db` file is not equivalent.

## Native executable

Replace `./litecom-data` with the relay's actual data folder. Use a new backup folder each time:

```sh
litecom backup --data ./litecom-data --output ./backup-1
litecom verify-backup ./backup-1
```

Both commands report **Backup verified** on success. An existing output folder is never overwritten. A failed snapshot without `manifest.json` is incomplete and cannot be restored.

## Docker Compose

From the same checkout and Compose project that runs the relay:

```sh
docker compose exec relay litecom backup --data /data/relay --output /data/backup-1
docker compose exec relay litecom verify-backup /data/backup-1
mkdir -p backups
chmod 700 backups
docker compose cp relay:/data/backup-1 ./backups/
```

Copy `backups/backup-1` to your protected backup destination. A backup that exists only inside `relay-data` is lost if that volume is lost. If you previously changed `LITECOM_DATA_DIR`, use that data path instead of `/data/relay`.

## Restore into a new folder

Stop the original relay and prevent it from restarting before switching to a recovered copy. Do not run both copies as independent servers. A backup can predate revocations or completed actions, so restoration deliberately resets access:

- Accounts, request history, drafts, and attachment bytes remain.
- People must sign in again, pair agents again, and reconnect contacts and approval channels. Old contact links are replaced.
- Previous download access is revoked. Pending work stops and is never automatically replayed.
- Cloud connections are removed. Existing provider sessions may still be running; inspect them directly before sending work again. Restore does not undo external actions or remotely stop those sessions.

For a native relay:

```sh
litecom restore ./backup-1 --data ./recovered-data
litecom relay --public-url https://YOUR-DOMAIN --data ./recovered-data
```

Use your actual configured domain and retain the relay's messaging environment. The destination must not already exist. If restoration fails, leave the original data intact and retry into another new folder. A `.restoring` marker prevents a partial restore from starting.

For Compose, first stop the deployment:

```sh
docker compose stop
```

Import only the three backup files. Run from the checkout where `backups/backup-1` was copied above. This runs the restore as the relay's normal container user and does not need host ownership changes:

```sh
tar -C backups/backup-1 -cf - manifest.json litecom.db storage.key | \
  docker compose run --rm --no-deps -T --entrypoint sh relay -c \
  'mkdir /data/restore-input-1 && tar --no-same-owner -xf - -C /data/restore-input-1 && litecom restore /data/restore-input-1 --data /data/recovered-1'
```

Add `LITECOM_DATA_DIR=/data/recovered-1` to `.env`, then start:

```sh
docker compose up -d
docker compose ps
```

Sign in, reconnect your agent and approval channel, and send a new self-request. Confirm that the old shared result is inaccessible and that the new approved result arrives. This checks recovery beyond a successful file copy. Keep the original data and backup until recovery is verified.
