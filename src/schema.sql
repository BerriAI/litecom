CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS profiles (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, phone_key TEXT UNIQUE,
  phone BLOB, invite_hash TEXT NOT NULL UNIQUE, invite BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS agents (
  id TEXT PRIMARY KEY, owner_id TEXT NOT NULL REFERENCES profiles(id),
  name TEXT NOT NULL COLLATE NOCASE, kind TEXT NOT NULL, is_default INTEGER NOT NULL DEFAULT 0,
  revoked INTEGER NOT NULL DEFAULT 0, last_seen_at INTEGER, config BLOB,
  UNIQUE(owner_id,name)
);
CREATE UNIQUE INDEX IF NOT EXISTS default_agent ON agents(owner_id) WHERE is_default=1 AND revoked=0;
CREATE TABLE IF NOT EXISTS credentials (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES profiles(id), agent_id TEXT REFERENCES agents(id),
  token_hash TEXT NOT NULL UNIQUE, role TEXT NOT NULL CHECK(role IN ('human','agent')),
  label TEXT NOT NULL, csrf_hash TEXT, expires_at INTEGER NOT NULL, revoked INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS contacts (
  a TEXT NOT NULL REFERENCES profiles(id), b TEXT NOT NULL REFERENCES profiles(id),
  initiator TEXT NOT NULL REFERENCES profiles(id), status TEXT NOT NULL CHECK(status IN ('pending','accepted','blocked')),
  created_at INTEGER NOT NULL, PRIMARY KEY(a,b), CHECK(a<b)
);
CREATE TABLE IF NOT EXISTS tasks (
  id TEXT PRIMARY KEY, requester_id TEXT NOT NULL REFERENCES profiles(id), owner_id TEXT NOT NULL REFERENCES profiles(id),
  agent_id TEXT NOT NULL REFERENCES agents(id), idempotency_key TEXT NOT NULL, status TEXT NOT NULL,
  created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, expires_at INTEGER NOT NULL, data BLOB NOT NULL,
  UNIQUE(requester_id,idempotency_key)
);
CREATE INDEX IF NOT EXISTS worker_queue ON tasks(agent_id,status,created_at,id);
CREATE INDEX IF NOT EXISTS task_requester ON tasks(requester_id,created_at,id);
CREATE INDEX IF NOT EXISTS task_owner ON tasks(owner_id,created_at,id);
CREATE TABLE IF NOT EXISTS artifacts (
  id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES tasks(id), size INTEGER NOT NULL,
  metadata BLOB NOT NULL, data BLOB NOT NULL
);
CREATE TABLE IF NOT EXISTS artifact_chunks (
  artifact_id TEXT NOT NULL REFERENCES artifacts(id) ON DELETE CASCADE,
  sequence INTEGER NOT NULL, data BLOB NOT NULL,
  PRIMARY KEY(artifact_id,sequence)
);
CREATE TABLE IF NOT EXISTS approvals (
  task_id TEXT NOT NULL REFERENCES tasks(id), version INTEGER NOT NULL, digest TEXT NOT NULL,
  user_id TEXT NOT NULL REFERENCES profiles(id), decision TEXT NOT NULL, created_at INTEGER NOT NULL,
  PRIMARY KEY(task_id,version)
);
CREATE TABLE IF NOT EXISTS events (
  sequence INTEGER PRIMARY KEY AUTOINCREMENT, task_id TEXT NOT NULL REFERENCES tasks(id), kind TEXT NOT NULL, created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS otp (
  id TEXT PRIMARY KEY, phone_key TEXT NOT NULL, code_hash TEXT NOT NULL, data BLOB NOT NULL,
  expires_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, used INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS otp_phone ON otp(phone_key,expires_at);
CREATE TABLE IF NOT EXISTS devices (
  device_hash TEXT PRIMARY KEY, user_code TEXT NOT NULL UNIQUE, label TEXT NOT NULL,
  expires_at INTEGER NOT NULL, user_id TEXT REFERENCES profiles(id), agent_id TEXT REFERENCES agents(id), consumed INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS local_pairings (
  device_hash TEXT PRIMARY KEY REFERENCES devices(device_hash),
  credential_id TEXT REFERENCES credentials(id), ready_at INTEGER, last_seen_at INTEGER
);
CREATE TABLE IF NOT EXISTS channels (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES profiles(id), kind TEXT NOT NULL,
  destination BLOB NOT NULL, verified INTEGER NOT NULL DEFAULT 0, UNIQUE(user_id,kind)
);
CREATE TABLE IF NOT EXISTS deliveries (
  event_id INTEGER NOT NULL REFERENCES events(sequence), channel_id TEXT NOT NULL REFERENCES channels(id),
  state TEXT NOT NULL DEFAULT 'pending', attempts INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0,
  remote_id TEXT, error TEXT, PRIMARY KEY(event_id,channel_id)
);
CREATE TABLE IF NOT EXISTS channel_challenges (
  id TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES profiles(id), credential_id TEXT NOT NULL REFERENCES credentials(id),
  kind TEXT NOT NULL DEFAULT 'email',
  destination_key TEXT NOT NULL, code_hash TEXT NOT NULL, data BLOB NOT NULL,
  expires_at INTEGER NOT NULL, attempts INTEGER NOT NULL DEFAULT 0, used INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS channel_challenge_rate ON channel_challenges(user_id,expires_at);
CREATE TABLE IF NOT EXISTS oauth_links (
  state_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES profiles(id),
  credential_id TEXT NOT NULL REFERENCES credentials(id), data BLOB NOT NULL,
  expires_at INTEGER NOT NULL, phase TEXT NOT NULL DEFAULT 'pending'
);
CREATE TABLE IF NOT EXISTS login_states (
  state_hash TEXT PRIMARY KEY, browser_hash TEXT NOT NULL,
  data BLOB NOT NULL, expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS login_identities (
  provider TEXT NOT NULL, subject_key TEXT NOT NULL, user_id TEXT NOT NULL REFERENCES profiles(id),
  data BLOB NOT NULL, PRIMARY KEY(provider,subject_key), UNIQUE(provider,user_id)
);
CREATE TABLE IF NOT EXISTS cloud_jobs (
  task_id TEXT PRIMARY KEY REFERENCES tasks(id), phase TEXT NOT NULL,
  data BLOB NOT NULL, claim_hash TEXT, claim_until INTEGER NOT NULL DEFAULT 0,
  next_attempt INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS tool_permissions (
  task_id TEXT NOT NULL REFERENCES tasks(id), id TEXT NOT NULL,
  decision TEXT, data BLOB NOT NULL, PRIMARY KEY(task_id,id)
);

CREATE TABLE IF NOT EXISTS channel_actions (
  token_hash TEXT PRIMARY KEY, code TEXT NOT NULL UNIQUE,
  channel_id TEXT NOT NULL REFERENCES channels(id), task_id TEXT NOT NULL REFERENCES tasks(id),
  target_key TEXT NOT NULL, data BLOB NOT NULL,
  UNIQUE(channel_id,task_id,target_key)
);

-- Hash the signed body, not the unsigned delivery-ID header, to reject callback replays.
CREATE TABLE IF NOT EXISTS phone_events (
  body_hash TEXT PRIMARY KEY, created_at INTEGER NOT NULL
);

-- Slack's event ID is stable across retries. Payloads and model conversations
-- use the same encrypted storage as requests; tokens stay in channels.
CREATE TABLE IF NOT EXISTS slack_jobs (
  id TEXT PRIMARY KEY, data BLOB NOT NULL,
  done INTEGER NOT NULL DEFAULT 0, next_attempt INTEGER NOT NULL DEFAULT 0,
  claim TEXT, claim_until INTEGER NOT NULL DEFAULT 0, attempts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS slack_intake_queue ON slack_jobs(done,next_attempt,claim_until);

-- Persistent intake counters contain hashed actor IDs, never message content.
CREATE TABLE IF NOT EXISTS intake_limits (
  actor TEXT PRIMARY KEY, count INTEGER NOT NULL, expires_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS intake_limit_expiry ON intake_limits(expires_at);
CREATE INDEX IF NOT EXISTS artifact_task ON artifacts(task_id);
