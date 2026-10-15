// Independent interoperability check using the released official A2A JS SDK.
// Usage: node scripts/a2a-smoke.mjs /path/to/node_modules/@a2a-js/sdk [litecom-binary]
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { spawn } from "node:child_process";
import { mkdtemp, open, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createServer } from "node:net";
import { createHash, randomUUID } from "node:crypto";
import { setTimeout as sleep } from "node:timers/promises";

assert(process.argv[2], "Pass an isolated installation of @a2a-js/sdk");
const requireSdk = createRequire(resolve(process.argv[2], "package.json"));
const { ClientFactory, DefaultAgentCardResolver, JsonRpcTransportFactory } =
  requireSdk("@a2a-js/sdk/client");
const {
  SendMessageRequest,
  GetTaskRequest,
  CancelTaskRequest,
  Role,
  TaskState,
} = requireSdk("@a2a-js/sdk");
const version = JSON.parse(
  await readFile(resolve(process.argv[2], "package.json"), "utf8"),
).version;
const root = await mkdtemp(join(tmpdir(), "litecom-a2a-"));
const logPath = join(root, "relay.log");
const log = await open(logPath, "w", 0o600);
const socket = createServer();
await new Promise((resolve, reject) => {
  socket.once("error", reject);
  socket.listen(0, "127.0.0.1", resolve);
});
const port = socket.address().port;
await new Promise((resolve) => socket.close(resolve));
const origin = `http://127.0.0.1:${port}`;
const relay = spawn(
  resolve(process.argv[3] || "target/debug/litecom"),
  [
    "relay",
    "--listen",
    `127.0.0.1:${port}`,
    "--public-url",
    origin,
    "--data",
    join(root, "data"),
    "--dev",
  ],
  { stdio: ["ignore", log.fd, log.fd] },
);
const exited = new Promise((resolve) => relay.once("exit", resolve));

class Human {
  cookies = new Map();
  async call(path, data) {
    const response = await fetch(origin + path, {
      method: data === undefined ? "GET" : "POST",
      headers: {
        Origin: origin,
        "Content-Type": "application/json",
        Cookie: [...this.cookies].map(([k, v]) => `${k}=${v}`).join("; "),
        "X-CSRF-Token": this.cookies.get("litecom_csrf") || "",
      },
      body: data === undefined ? undefined : JSON.stringify(data),
    });
    for (const header of response.headers.getSetCookie()) {
      const cookie = header.split(";", 1)[0];
      const split = cookie.indexOf("=");
      this.cookies.set(cookie.slice(0, split), cookie.slice(split + 1));
    }
    assert(
      response.ok,
      `${path}: ${response.status} ${await response.clone().text()}`,
    );
    return response.status === 204 ? undefined : response.json();
  }
  async login(name, phone) {
    const challenge = await this.call("/auth/start", { name, phone });
    const codes = [
      ...(await readFile(logPath, "utf8")).matchAll(
        /Your Litecom sign-in code is (\d{6})/g,
      ),
    ];
    this.profile = (
      await this.call("/auth/verify", {
        id: challenge.id,
        code: codes.at(-1)[1],
      })
    ).profile;
  }
}

try {
  for (let attempt = 0; ; attempt++) {
    try {
      assert(
        (await (await fetch(origin + "/api/capabilities")).json()).development,
      );
      break;
    } catch (error) {
      if (attempt >= 100) throw error;
      await sleep(100);
    }
  }
  const a = new Human(),
    b = new Human();
  await a.login("Requester", "+14155550391");
  await b.login("Owner", "+14155550392");
  await a.call("/api/contacts", { invite: b.profile.invite });
  await b.call(`/api/contacts/${a.profile.id}/accept`, {});
  const requesterAgent = (await a.call("/api/agents"))[0].id;
  const ownerAgent = (await b.call("/api/agents"))[0].id;
  const device = await a.call("/device/start", {
    label: "Official A2A SDK fixture",
  });
  await a.call(`/api/device/${device.user_code}`, { agent_id: requesterAgent });
  const { token } = await a.call("/device/poll", {
    device_code: device.device_code,
  });
  const authenticatedFetch = (url, options = {}) => {
    assert.equal(
      new URL(url).origin,
      origin,
      "The test never forwards credentials off the local relay",
    );
    const headers = new Headers(options.headers);
    headers.set("Authorization", `Bearer ${token}`);
    return fetch(url, { ...options, headers });
  };
  const factory = new ClientFactory({
    transports: [
      new JsonRpcTransportFactory({ fetchImpl: authenticatedFetch }),
    ],
    cardResolver: new DefaultAgentCardResolver({
      fetchImpl: authenticatedFetch,
    }),
  });
  const client = await factory.createFromUrl(
    origin,
    `/a2a/${b.profile.id}/${ownerAgent}/.well-known/agent-card.json`,
  );
  const message = (text) =>
    SendMessageRequest.fromJSON({
      message: {
        messageId: randomUUID(),
        role: Role.ROLE_USER,
        parts: [{ text }],
      },
      configuration: { returnImmediately: true },
    });
  const params = message("Send the exact binary fixture and a response.");
  const task = await client.sendMessage(params);
  assert.equal(task.status.state, TaskState.TASK_STATE_SUBMITTED);
  assert.equal((await client.sendMessage(params)).id, task.id);
  const payload = Buffer.from([0, 1, 2, 127, 128, 255]);
  const file = await b.call(`/api/tasks/${task.id}/files`, {
    name: "fixture.bin",
    media_type: "application/octet-stream",
    data: payload.toString("base64"),
  });
  const review = await b.call(`/api/tasks/${task.id}/draft`, {
    draft: {
      text: "Private fixture response",
      attachment_ids: [file.id],
      action: null,
    },
  });
  const pending = await client.getTask(
    GetTaskRequest.fromJSON({ id: task.id }),
  );
  assert.equal(pending.status.state, TaskState.TASK_STATE_WORKING);
  assert.equal(pending.artifacts.length, 0);
  assert.equal(
    (await authenticatedFetch(`${origin}/api/files/${file.id}`)).status,
    403,
  );
  // Omitted configuration and explicit false must both wait, including retries
  // of a message that initially requested an immediate response.
  const blocking = [];
  for (const configuration of [undefined, { returnImmediately: false }]) {
    const retry = { ...params, configuration };
    let returned = false;
    const result = client.sendMessage(retry).then((value) => {
      returned = true;
      return value;
    });
    await sleep(200);
    assert.equal(
      returned,
      false,
      "Blocking SendMessage returned before completion",
    );
    blocking.push(result);
  }
  await b.call(`/api/tasks/${task.id}/decision`, {
    version: review.draft.version,
    digest: review.draft.digest,
    decision: "approve",
  });
  const done = await client.getTask(GetTaskRequest.fromJSON({ id: task.id }));
  assert.equal(done.status.state, TaskState.TASK_STATE_COMPLETED);
  for (const result of blocking) {
    const completed = await result;
    assert.equal(completed.status.state, TaskState.TASK_STATE_COMPLETED);
    assert.equal(completed.artifacts.length, done.artifacts.length);
  }
  assert(
    done.artifacts.some((artifact) =>
      artifact.parts.some(
        (part) =>
          part.content?.$case === "text" &&
          part.content.value === "Private fixture response",
      ),
    ),
  );
  const artifact = done.artifacts.find(
    (artifact) => artifact.artifactId === file.id,
  );
  assert(artifact);
  const part = artifact.parts[0];
  assert.equal(part.filename, "fixture.bin");
  assert.equal(part.content.$case, "url");
  assert.equal((await fetch(part.content.value)).status, 401);
  const downloaded = Buffer.from(
    await (await authenticatedFetch(part.content.value)).arrayBuffer(),
  );
  assert.deepEqual(downloaded, payload);
  assert.equal(
    createHash("sha256").update(downloaded).digest("hex"),
    part.metadata.sha256,
  );
  await assert.rejects(
    client.cancelTask(CancelTaskRequest.fromJSON({ id: task.id })),
  );
  const cancelled = await client.sendMessage(
    message("Cancel this queued request."),
  );
  const outcome = await client.cancelTask(
    CancelTaskRequest.fromJSON({ id: cancelled.id }),
  );
  assert.equal(outcome.status.state, TaskState.TASK_STATE_CANCELED);
  const repeatedCancel = await client.cancelTask(
    CancelTaskRequest.fromJSON({ id: cancelled.id }),
  );
  assert.equal(repeatedCancel.status.state, TaskState.TASK_STATE_CANCELED);
  await assert.rejects(
    client.getTask(GetTaskRequest.fromJSON({ id: randomUUID() })),
  );
  console.log(
    `PASS: official @a2a-js/sdk ${version}; authenticated discovery, SendMessage retry, private GetTask, exact artifacts/download, cancellation, and missing-task errors.`,
  );
} finally {
  relay.kill("SIGTERM");
  await exited;
  await log.close();
  await rm(root, { recursive: true, force: true });
}
