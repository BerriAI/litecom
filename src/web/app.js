const app = document.querySelector("#app"),
  nav = document.querySelector("#nav");
let profile,
  capabilities,
  currentTask,
  busy = false,
  challenge;
const esc = (v) =>
  String(v ?? "").replace(
    /[&<>"']/g,
    (c) =>
      ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[
        c
      ],
  );
const csrf = () =>
  document.cookie
    .split("; ")
    .find((c) => c.startsWith("litecom_csrf="))
    ?.split("=")[1] || "";
async function api(path, body, method) {
  const res = await fetch(path, {
    method: method || (body === undefined ? "GET" : "POST"),
    headers:
      body !== undefined
        ? { "Content-Type": "application/json", "X-CSRF-Token": csrf() }
        : { "X-CSRF-Token": csrf() },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  if (res.status === 204) return null;
  const data = await res.json();
  if (!res.ok) {
    const err = new Error(
      data.error?.message || "Could not finish this request. Please retry.",
    );
    err.status = res.status;
    throw err;
  }
  return data;
}
function notice(text) {
  const el = document.querySelector("#notice");
  el.textContent = text;
  el.style.display = "block";
  clearTimeout(notice.timer);
  notice.timer = setTimeout(() => (el.style.display = "none"), 6000);
}
function error(err, id = "page-error") {
  const old = document.getElementById(id);
  if (old?.textContent === err.message) return;
  if (old) old.remove();
  const el = document.createElement("p");
  el.id = id;
  el.className = "error";
  el.setAttribute("role", "alert");
  el.textContent = err.message;
  app.prepend(el);
}
const badge = (s, t) =>
  `<span class="badge" data-status="${esc(s)}">${esc(s === "completed" && t?.revoked_at ? "Access revoked" : s === "completed" && t?.expires_at != null && t.expires_at <= Date.now() / 1000 ? "Expired" : { queued: "Waiting for agent", preparing: "Preparing", awaiting_approval: "Ready for review", approved: "Approved · waiting for agent", executing: "Running", cancel_requested: "Cancellation requested", completed: "Available", declined: "Declined", cancelled: "Cancelled", failed: "Needs attention", expired: "Expired" }[s] || s)}</span>`;
const files = (items) =>
  items.length
    ? `<ul class="files">${items.map((f) => `<li><a href="/api/files/${encodeURIComponent(f.id)}" data-download="${esc(f.id)}" data-name="${esc(f.name)}" data-sha256="${esc(f.sha256)}" download>${esc(f.name)}</a><small>${(f.size / 1024).toFixed(1)} KB</small></li>`).join("")}</ul>`
    : "";
function operationFields(value, depth = 0) {
  if (typeof value === "string") return `<pre>${esc(value)}</pre>`;
  if (value === null || typeof value !== "object")
    return `<pre>${esc(JSON.stringify(value))}</pre>`;
  if (Object.keys(value).length === 0)
    return `<pre>${Array.isArray(value) ? "[]" : "{}"}</pre>`;
  if (depth >= 3) return `<pre>${esc(JSON.stringify(value, null, 2))}</pre>`;
  if (Array.isArray(value))
    return value.map((item) => operationFields(item, depth + 1)).join("");
  const labels = {
    rawInput: "Inputs",
    path: "File path",
    file_path: "File path",
    old_string: "Current text",
    new_string: "Replacement text",
    content: "Content",
    command: "Command",
    locations: "Affected files",
    kind: "Tool type",
    name: "Tool",
    title: "Operation",
  };
  return `<dl class="operation-fields">${Object.entries(value)
    .map(
      ([key, item]) =>
        `<dt>${esc(labels[key] || key)}</dt><dd>${operationFields(item, depth + 1)}</dd>`,
    )
    .join("")}</dl>`;
}
function permissionCard(permission) {
  const operation = permission.details?.operation ?? permission.details;
  const fields =
    operation && typeof operation === "object" && !Array.isArray(operation)
      ? { ...operation }
      : operation;
  if (fields?.title === permission.title) delete fields.title;
  return `<section class="card permission"><p class="preview-label">Your approval is needed</p><h2>${esc(permission.title)}</h2><p>This operation needs additional permission. Allow it once for this request.</p>${operationFields(fields)}<div class="actions"><button class="primary" data-action="approve-permission" data-id="${esc(permission.id)}">Approve once</button><button data-action="decline-permission" data-id="${esc(permission.id)}">Decline operation</button></div></section>`;
}
function draftCard(draft, status) {
  const action = draft.action;
  const body = action
    ? `<p>${action.share_result ? "Approve to let your agent do this and send back the result." : "Approve to run this request. This older proposal requires a separate result review."}</p><details><summary>Agent and access</summary><h2>${esc(action.title)}</h2><p>${esc(action.description)}</p><p class="muted">Acts as ${esc(action.acting_account)}.</p>${action.workspace ? `<p class="muted subtitle">Folder: <code>${esc(action.workspace)}</code></p>` : ""}<pre>${esc(JSON.stringify(action, null, 2))}</pre></details>`
    : `<h2>${status === "completed" ? "Result" : "Reply"}</h2><div class="prose">${esc(draft.text)}</div>${files(draft.attachments)}`;
  const controls =
    status === "awaiting_approval"
      ? `<div class="actions"><button class="primary" data-action="approve">Approve</button><button data-action="decline">Decline</button></div>`
      : "";
  return action && ["approved", "executing"].includes(status)
    ? `<details class="card"><summary>Request approved</summary>${body}</details>`
    : `<section class="card">${body}${controls}</section>`;
}

async function boot() {
  try {
    capabilities = await api("/api/capabilities");
    const selectedChannel = new URLSearchParams(location.search).get("approve");
    if (
      location.pathname.startsWith("/connect/") &&
      ["phone", "email", "slack"].includes(selectedChannel) &&
      sessionStorage.getItem("channel-choice-url") !== location.href
    ) {
      sessionStorage.setItem("approval-channel", selectedChannel);
      sessionStorage.setItem("channel-choice-url", location.href);
    }
    try {
      profile = await api("/api/me");
    } catch (e) {
      if (e.status !== 401) throw e;
    }
    if (!profile) return login();
    if (sessionStorage.getItem("approval-channel") === "phone") {
      await api("/api/channels/phone", {});
      sessionStorage.removeItem("approval-channel");
    }
    if (location.pathname === "/slack/connected") return slackConnected();
    if (
      sessionStorage.getItem("approval-channel") === "email" &&
      !(await api("/api/channels")).some(
        (c) => c.kind === "email" && c.verified,
      )
    )
      return emailSetup();
    if (
      sessionStorage.getItem("approval-channel") === "slack" &&
      !(await api("/api/channels")).some(
        (c) => c.kind === "slack" && c.verified,
      )
    )
      return slackSetup();
    nav.innerHTML =
      '<a href="/">Inbox</a><a href="/contacts">Contacts</a><a href="/agents">Agents</a><a href="/settings">Settings</a>';
    await render();
  } catch (e) {
    error(e);
  }
}
function login() {
  nav.innerHTML = "";
  const requested = new URLSearchParams(location.search).get("approve");
  const chosen = ["phone", "slack", "email"].includes(requested)
    ? requested
    : null;
  if (chosen) sessionStorage.setItem("approval-channel", chosen);
  app.innerHTML = `<section class="narrow"><h1>Your agents, connected.</h1><p class="muted">Ask for something. Approve the request once. Pick it up from any of your agents.</p>${capabilities.development ? '<p class="banner">Local development. Codes appear in the relay terminal; no phone messages are sent.</p>' : ""}<form id="login">${chosen ? `<input type="hidden" name="channel" value="${chosen}"><p class="banner">Approve requests on ${esc(chosen)}. You can add channels later.</p>` : `<fieldset><legend>Where do you want to approve requests?</legend><div class="choices">${["Phone", "Slack", "Email"].map((c, i) => `<label><input type="radio" name="channel" value="${c.toLowerCase()}" ${!i ? "checked" : ""}>${c}</label>`).join("")}</div></fieldset>`}<label>Your name<input name="name" autocomplete="name" required></label><label>Phone number<input name="phone" type="tel" autocomplete="tel" placeholder="+1 415 555 0123" required></label><p class="muted subtitle">Your phone connects all your agents to one account. Choose Phone to receive request notifications by text. Message rates may apply; reply STOP to unsubscribe.</p><button class="primary" ${!capabilities.phone ? "disabled" : ""}>Send sign-in code</button>${!capabilities.phone ? '<p class="error">Phone delivery is not enabled on this server. Ask its owner to connect a messaging provider.</p>' : ""}</form></section>`;
}

function slackSetup() {
  app.innerHTML =
    '<section class="narrow"><h1>Approve requests in Slack</h1><p class="muted">Connect the Litecom app to your workspace. It sends you a direct message with a review link and updates that message when the request changes.</p><button class="primary" data-action="add-slack">Connect Slack</button></section>';
}
async function slackConnected() {
  const params = new URLSearchParams(location.search);
  if (params.has("error")) {
    app.innerHTML =
      '<section class="narrow"><h1>Slack was not connected.</h1><p class="muted">You can retry, or ask your workspace administrator to allow the Litecom app.</p><button class="primary" data-action="add-slack">Try again</button></section>';
    return;
  }
  await api("/api/channels/slack/finish", { state: params.get("state") });
  sessionStorage.removeItem("approval-channel");
  const next = new URL(
    sessionStorage.getItem("slack-return") || "/settings",
    location.origin,
  );
  sessionStorage.removeItem("slack-return");
  history.replaceState(
    null,
    "",
    next.origin === location.origin && !next.pathname.startsWith("/slack/")
      ? next.pathname + next.search
      : "/settings",
  );
  notice("Slack connected.");
  await boot();
}

function emailSetup() {
  const pending = sessionStorage.getItem("email-challenge");
  app.innerHTML = `<section class="narrow"><h1>${pending ? "Check your email" : "Approve requests by email"}</h1><p class="muted">Your phone account stays the same. Email receives a link to review each request.</p>${capabilities.development ? '<p class="banner">Local development: email codes print in the relay terminal.</p>' : ""}${pending ? `<form id="email-verify"><label>Email code<input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required autofocus></label><button class="primary">Connect email</button></form><button data-action="restart-email">Use another email</button>` : `<form id="email-start"><label>Email address<input name="email" type="email" autocomplete="email" required></label><button class="primary">Send email code</button></form>`}</section>`;
}

async function render() {
  const parts = location.pathname.split("/").filter(Boolean);
  const title =
    {
      request: "Request",
      connect: "Connect agent",
      invite: "Add contact",
      contacts: "Contacts",
      agents: "Agents",
      settings: "Account",
    }[parts[0]] || "Inbox";
  document.title = `${title} · Litecom`;
  document.querySelectorAll("#nav a").forEach((link) => {
    if (new URL(link.href).pathname === location.pathname)
      link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  });
  if (parts[0] === "request" && parts[1]) return taskPage(parts[1]);
  if (parts[0] === "connect" && parts[1]) return connectPage(parts[1]);
  if (parts[0] === "invite" && parts[1]) return invitePage(parts[1]);
  if (parts[0] === "contacts") return contactsPage();
  if (parts[0] === "agents") return agentsPage();
  if (parts[0] === "settings") return settingsPage();
  return inbox();
}
async function inbox() {
  const cursor = new URLSearchParams(location.search);
  const query =
    cursor.has("before_time") && cursor.has("before_id")
      ? "?" +
        new URLSearchParams({
          before_time: cursor.get("before_time"),
          before_id: cursor.get("before_id"),
        })
      : "";
  const [tasks, contacts, agents] = await Promise.all([
    api("/api/tasks" + query),
    api("/api/contacts"),
    api("/api/agents"),
  ]);
  const names = new Map(contacts.map((c) => [c.id, c.name]));
  names.set(profile.id, "You");
  app.innerHTML = `<div class="intro"><h1>Your inbox</h1><p class="muted">Requests and responses, shared across your agents.</p></div><details class="card"><summary>Ask an agent</summary><form id="ask"><label>Who<select name="owner"><option value="${esc(profile.id)}">My agents</option>${contacts
    .filter((c) => c.status === "accepted")
    .map((c) => `<option value="${esc(c.id)}">${esc(c.name)}</option>`)
    .join(
      "",
    )}</select></label><label>Agent<select name="agent"><option value="">Default agent</option>${agents.map((a) => `<option value="${esc(a.id)}">${esc(a.name)}</option>`).join("")}</select></label><label>What do you need?<textarea name="request" placeholder="Find the launch notes and send me the final PDF." required></textarea></label><button class="primary">Send request</button></form></details>${tasks.length ? tasks.map((t) => `<article class="card"><div class="row"><a class="title-link" href="/request/${encodeURIComponent(t.id)}">${esc(t.request.length > 180 ? t.request.slice(0, 180) + "…" : t.request)}</a>${badge(t.status, t)}</div><p class="muted subtitle">${esc(names.get(t.requester_id) || "Contact")} → ${esc(names.get(t.owner_id) || "Contact")}${t.received_at ? " · Received" : ""}${t.revoked_at ? " · Access revoked" : ""}</p></article>`).join("") : query ? '<div class="card empty"><h2>No older requests.</h2></div>' : '<div class="card empty"><h2>Start with a request.</h2><p>Ask one of your own agents above, or <a href="/contacts">connect with someone</a>.</p></div>'}`;
  const last = tasks.at(-1);
  if (query || tasks.length === 100) {
    const paging = document.createElement("nav");
    paging.className = "actions";
    paging.setAttribute("aria-label", "Inbox pages");
    if (query) paging.innerHTML = '<a href="/">Latest requests</a>';
    if (tasks.length === 100) {
      const link = document.createElement("a");
      link.textContent = "Older requests";
      link.href =
        "/?" +
        new URLSearchParams({
          before_time: last.created_at,
          before_id: last.id,
        });
      paging.append(link);
    }
    app.append(paging);
  }
}
function editingTask() {
  return (
    ["INPUT", "TEXTAREA", "SELECT"].includes(document.activeElement?.tagName) ||
    app.querySelector("#draft")?.dataset.dirty === "true"
  );
}
async function taskPage(id, update, previous) {
  const [t, contacts, agents] = await Promise.all([
    update || api(`/api/tasks/${encodeURIComponent(id)}`),
    api("/api/contacts"),
    api("/api/agents"),
  ]);
  // A poll may finish after typing or another action began. Do not replace that UI.
  if (previous && (currentTask !== previous || busy || editingTask())) return;
  const oldVersion = currentTask?.draft?.version;
  const focused = app.contains(document.activeElement)
    ? document.activeElement
    : null;
  const focusAction = focused?.dataset.action;
  const opened = [...app.querySelectorAll("details[open] > summary")].map(
    (el) => el.textContent,
  );
  currentTask = t;
  const owner = t.owner_id === profile.id,
    requester = t.requester_id === profile.id,
    names = new Map(contacts.map((c) => [c.id, c.name]));
  names.set(profile.id, "You");
  const agent = agents.find((a) => a.id === t.agent_id);
  const editable =
    owner && ["queued", "preparing", "awaiting_approval"].includes(t.status);
  const d = owner ? t.draft : null;
  app.innerHTML = `<p><a href="/">← Inbox</a></p><div class="row"><h1>${owner && !requester ? esc(names.get(t.requester_id) || "Your contact") + " asks" : "Your request"}</h1>${badge(t.status, t)}</div><p class="muted">${esc(names.get(t.requester_id) || "Contact")} → ${esc(names.get(t.owner_id) || "Contact")}${agent ? " · " + esc(agent.name) : ""}</p><div class="card"><h2>Requested</h2><div class="prose">${esc(t.request)}</div></div>${t.downloads?.length ? `<p class="muted subtitle">${t.downloads.length} file download${t.downloads.length === 1 ? "" : "s"} verified by the recipient.</p>` : ""}${t.revoked_at ? '<p class="banner">Access has been revoked. Files already downloaded cannot be recalled.</p>' : ""}${t.status === "queued" ? '<p class="banner">This request is saved. The selected agent will pick it up when it connects.</p>' : ""}${(t.permissions || []).map(permissionCard).join("")}${d ? draftCard(d, t.status) : ""}${t.response && !owner ? `<section class="card"><h2>Response</h2><div class="prose">${esc(t.response.text)}</div>${files(t.response.attachments)}</section>` : ""}${t.execution ? `<section class="card"><h2>Cloud session</h2><p class="prose">${esc(t.execution.message)}</p>${t.execution.url ? `<a href="${esc(t.execution.url)}" target="_blank" rel="noopener noreferrer">Open in Devin ↗</a>` : ""}</section>` : ""}${t.receipt ? `<section class="card"><h2>Action result</h2><p class="prose">${esc(t.receipt.summary)}</p></section>` : ""}${editable ? `<details class="card"><summary>${d ? "Edit response" : "Prepare a response yourself"}</summary><form id="draft"><label>Reply<textarea name="text">${esc(d?.text || "")}</textarea></label><label>Attach files<input name="files" type="file" multiple></label><p class="muted subtitle">Saving a change requires a fresh approval.</p><button>Save for review</button></form></details>` : ""}${requester && t.status === "completed" && !t.revoked_at ? `<div class="actions">${t.received_at ? '<p class="muted">Received.</p>' : '<button data-action="received">Mark received</button>'}</div>` : ""}${!["declined", "cancelled", "failed", "expired"].includes(t.status) ? `<div class="actions"><button class="danger" data-action="cancel">${owner && t.status === "completed" ? "Revoke access" : "Cancel request"}</button></div>` : ""}`;
  if (oldVersion === t.draft?.version) {
    app.querySelectorAll("details > summary").forEach((el) => {
      if (opened.includes(el.textContent)) el.parentElement.open = true;
    });
  }
  if (focused && !focused.isConnected) {
    const replacement =
      oldVersion === t.draft?.version && focusAction
        ? app.querySelector(`[data-action="${CSS.escape(focusAction)}"]`)
        : null;
    (replacement || app).focus({ preventScroll: true });
  }
}
async function contactsPage() {
  const contacts = await api("/api/contacts");
  app.innerHTML = `<h1>People you can ask</h1><p class="muted">Share a link. They accept. Then ask by name from any agent.</p><section class="card"><label>Your contact link<input id="invite-link" readonly value="${esc(location.origin + "/invite/" + profile.invite)}"></label><button data-action="copy-invite">Copy link</button></section><form id="contact" class="card"><label>Add a contact<input name="link" type="url" placeholder="Paste their Litecom contact link" required></label><button>Connect</button></form>${contacts.map((c) => `<article class="card"><div class="row"><h2>${esc(c.name)}</h2><span class="badge">${esc(c.status)}</span></div>${c.status === "pending" && c.incoming ? `<button class="primary" data-action="accept-contact" data-id="${esc(c.id)}">Accept contact</button>` : ""}${c.status === "accepted" ? `<p class="muted">${c.agents.map((a) => esc(a.name) + (a.is_default ? " (default)" : "")).join(" · ")}</p><button class="danger" data-action="block-contact" data-id="${esc(c.id)}">Block</button>` : ""}</article>`).join("")}`;
}
async function agentsPage() {
  const agents = await api("/api/agents");
  app.innerHTML = `<h1>Your agents</h1><p class="muted">One account. A separate connection for every agent.</p>${agents.map((a) => `<section class="card"><div class="row"><h2>${esc(a.name)}</h2><span class="badge">${a.is_default ? "Default" : esc(a.kind)}</span></div><p class="muted">${a.last_seen_at ? "Last connected " + new Date(a.last_seen_at * 1000).toLocaleString() : a.kind === "devin" ? "Connect Devin below to enable cloud requests" : "Waiting for a runner to connect"}</p><div class="actions">${!a.is_default ? `<button data-action="default-agent" data-id="${esc(a.id)}">Make default</button>` : ""}<button class="danger" data-action="revoke-agent" data-id="${esc(a.id)}">Disconnect</button></div></section>`).join("")}<details class="card"><summary>Add another local agent</summary><form id="agent"><label>Name<input name="name" placeholder="My laptop" required></label><input name="kind" type="hidden" value="local"><p class="muted subtitle">Adding a name creates a destination. It becomes available after you connect its runner.</p><button>Create agent</button></form></details><details class="card" id="devin-connect"><summary>Connect Devin</summary><p class="muted">Use Devin even when your computer is off. Get a personal access token or service-user key from <a href="https://app.devin.ai/settings/api-keys" target="_blank" rel="noopener noreferrer">Devin Settings → Devin API</a>. Your existing Devin plan is required.</p><form id="devin"><label>Devin token<input name="token" type="password" autocomplete="off" placeholder="cog_…" required></label><details><summary>Name and organization</summary><label>Agent name<input name="name" value="Devin" required></label><label>Organization ID (if not detected)<input name="org_id" placeholder="org-…"></label><p class="muted subtitle">To reconnect, use the same agent name. The organization ID is at the top of Devin Settings → Devin API.</p></details><p class="muted subtitle">The server stores this token encrypted. Connecting checks access without starting a session. Each run needs your approval and uses your normal Devin billing.</p><button class="primary">Connect Devin</button></form></details>`;
}
async function connectPage(code) {
  const [device, agents] = await Promise.all([
    api(`/api/device/${encodeURIComponent(code)}`),
    api("/api/agents"),
  ]);
  app.innerHTML = `<section class="narrow"><h1>Connect this agent?</h1><p><strong>${esc(device.label)}</strong> wants to join your Litecom account.</p><div class="card"><p class="muted">Check that this code matches the one in your terminal.</p><p class="code">${esc(code)}</p><form id="device" data-code="${esc(code)}"><label>Connect as<select name="agent">${agents.map((a) => `<option value="${esc(a.id)}">${esc(a.name)}${a.is_default ? " (default)" : ""}</option>`).join("")}</select></label><p class="muted subtitle">It can ask your contacts, access your inbox, and prepare work for this agent. Only you can approve.</p><button class="primary">Connect agent</button></form></div></section>`;
}
async function invitePage(code) {
  const p = await api(`/api/invite/${encodeURIComponent(code)}`);
  app.innerHTML = `<section class="narrow"><h1>Connect with ${esc(p.name)}</h1><p>Once they accept, your agents can ask each other for things. Each request still needs approval.</p><button class="primary" data-action="use-invite" data-id="${esc(code)}">Send contact request</button></section>`;
}
async function settingsPage() {
  const [connections, channels] = await Promise.all([
    api("/api/connections"),
    api("/api/channels"),
  ]);
  const labels = { phone: "Phone", email: "Email", slack: "Slack" };
  const deliveryLabels = {
    pending: "Waiting to send",
    sending: "Sending",
    accepted_by_provider: "Last notice accepted by provider",
    failed: "Delivery failed. Check the channel or reconnect.",
  };
  const rows = ["phone", "email", "slack"]
    .map((kind) => {
      const channel = channels.find((c) => c.kind === kind && c.verified);
      const instructions = capabilities.direct_approvals?.[kind]
        ? {
            phone: "Reply APPROVE and the request code by text.",
            email: "Reply Approve to the request email.",
            slack: "Tap Approve in the Slack message.",
          }[kind]
        : "Open the link in your notification to review.";
      const addAction = {
        phone: "enable-phone",
        email: "add-email",
        slack: "add-slack",
      }[kind];
      return `<div class="card"><div class="row"><h3>${labels[kind]}</h3><span class="muted subtitle">${channel ? "Connected" : "Not connected"}</span></div><p class="muted subtitle">${instructions}</p>${channel?.delivery_status && deliveryLabels[channel.delivery_status] ? `<p class="${channel.delivery_status === "failed" ? "error" : "muted subtitle"}">${deliveryLabels[channel.delivery_status]}</p>` : ""}<div class="actions">${capabilities[kind] ? `<button data-action="${addAction}">${channel ? (kind === "phone" ? "Keep phone enabled" : "Reconnect " + labels[kind]) : "Connect " + labels[kind]}</button>` : '<p class="muted subtitle">Not enabled on this server.</p>'}${channel ? `<button data-action="disable-channel" data-id="${esc(channel.id)}">Turn off ${labels[kind]}</button>` : ""}</div></div>`;
    })
    .join("");
  app.innerHTML = `<h1>Account</h1><p class="muted">Signed in as ${esc(profile.name)}.</p><h2>Approval channels</h2><p class="muted">Approve from a connected channel. One decision updates them all. Add or remove channels without changing your phone account. Reply STOP to stop texts.</p>${rows}${capabilities.development ? '<p class="banner">Local development: notifications print in the relay terminal.</p>' : ""}<h2>Connected apps</h2>${connections.map((c) => `<div class="card row"><span>${esc(c.label)}</span><button data-action="revoke-connection" data-id="${esc(c.id)}">Disconnect</button></div>`).join("")}<button data-action="logout">Sign out</button>`;
}
document.addEventListener("submit", async (e) => {
  e.preventDefault();
  if (busy) return;
  busy = true;
  const form = e.target,
    data = new FormData(form),
    buttons = [...form.querySelectorAll("button")];
  buttons.forEach((b) => (b.disabled = true));
  try {
    if (form.id === "login") {
      const channel = data.get("channel");
      if (channel !== "phone" && !capabilities[channel]) {
        throw new Error(
          `${channel === "slack" ? "Slack" : "Email"} is not enabled on this server. Choose an available approval channel.`,
        );
      }
      sessionStorage.setItem("approval-channel", channel);
      challenge = await api("/auth/start", {
        name: data.get("name"),
        phone: data.get("phone").replace(/[ ()-]/g, ""),
      });
      app.innerHTML = `<section class="narrow"><h1>Check your phone</h1><p class="muted">Enter the six-digit code. It expires in 10 minutes.</p><form id="verify"><label>Sign-in code<input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required autofocus></label><button class="primary">Continue</button></form><p><button data-action="restart-login">Use another number</button></p></section>`;
    } else if (form.id === "verify") {
      sessionStorage.removeItem("email-challenge");
      await api("/auth/verify", { id: challenge.id, code: data.get("code") });
      if (sessionStorage.getItem("approval-channel") === "phone")
        await api("/api/channels/phone", {});
      await boot();
    } else if (form.id === "email-start") {
      const c = await api("/api/channels/email/start", {
        email: data.get("email"),
      });
      sessionStorage.setItem("email-challenge", c.id);
      emailSetup();
    } else if (form.id === "email-verify") {
      await api("/api/channels/email/verify", {
        id: sessionStorage.getItem("email-challenge"),
        code: data.get("code"),
      });
      sessionStorage.removeItem("email-challenge");
      sessionStorage.removeItem("approval-channel");
      notice("Email connected.");
      await boot();
    } else if (form.id === "ask") {
      const t = await api("/api/tasks", {
        owner_id: data.get("owner"),
        agent_id: data.get("agent") || null,
        request: data.get("request"),
        idempotency_key:
          form.dataset.requestId ||
          (form.dataset.requestId = crypto.randomUUID()),
      });
      location.href = "/request/" + t.id;
    } else if (form.id === "draft") {
      const attachment_ids = [
        ...(currentTask.draft?.attachments || []).map((a) => a.id),
      ];
      for (const f of form.elements.files.files) {
        const upload = new FormData();
        upload.append(
          "metadata",
          JSON.stringify({
            name: f.name,
            media_type: f.type || "application/octet-stream",
          }),
        );
        upload.append("file", f);
        const response = await fetch(`/api/tasks/${currentTask.id}/files`, {
          method: "POST",
          headers: { "X-CSRF-Token": csrf() },
          body: upload,
        });
        const a = await response.json();
        if (!response.ok)
          throw new Error(
            a.error?.message || "The file could not be uploaded. Please retry.",
          );
        attachment_ids.push(a.id);
      }
      await api(`/api/tasks/${currentTask.id}/draft`, {
        draft: {
          text: data.get("text"),
          attachment_ids,
          action: currentTask.draft?.action || null,
        },
      });
      await render();
    } else if (form.id === "contact") {
      const url = new URL(data.get("link"));
      if (
        url.origin !== location.origin ||
        !url.pathname.startsWith("/invite/")
      )
        throw new Error("Use a contact link from this Litecom server.");
      await api("/api/contacts", { invite: url.pathname.split("/")[2] });
      await render();
    } else if (form.id === "agent") {
      await api("/api/agents", {
        name: data.get("name"),
        kind: data.get("kind"),
      });
      await render();
    } else if (form.id === "devin") {
      const result = await api("/api/agents/devin", {
        name: data.get("name"),
        token: data.get("token"),
        org_id: data.get("org_id") || null,
      });
      form.reset();
      await render();
      notice(
        `${result.name} connected. Ask it from your inbox, or make it your default agent.`,
      );
    } else if (form.id === "device") {
      await api(`/api/device/${encodeURIComponent(form.dataset.code)}`, {
        agent_id: data.get("agent"),
      });
      app.innerHTML =
        '<section class="narrow"><h1>Agent connected.</h1><p>Return to your terminal to finish setup.</p><a href="/">Open inbox</a></section>';
    }
  } catch (err) {
    error(err);
  } finally {
    busy = false;
    buttons.forEach((b) => (b.disabled = false));
  }
});
document.addEventListener("input", (e) => {
  if (e.target.form?.id === "draft") e.target.form.dataset.dirty = "true";
});
document.addEventListener("change", async (e) => {
  if (e.target.form?.id === "draft") e.target.form.dataset.dirty = "true";
  if (e.target.form?.id === "ask") delete e.target.form.dataset.requestId;
  if (e.target.name === "owner") {
    const form = e.target.form,
      owner = e.target.value,
      select = form.elements.agent;
    select.innerHTML = '<option value="">Default agent</option>';
    select.disabled = true;
    try {
      const agents = await api(
        "/api/agents?owner_id=" + encodeURIComponent(owner),
      );
      if (!form.isConnected || form.elements.owner.value !== owner) return;
      select.innerHTML =
        '<option value="">Default agent</option>' +
        agents
          .map((a) => `<option value="${esc(a.id)}">${esc(a.name)}</option>`)
          .join("");
    } catch (err) {
      if (form.isConnected && form.elements.owner.value === owner) error(err);
    } finally {
      if (form.isConnected && form.elements.owner.value === owner)
        select.disabled = false;
    }
  }
});
document.addEventListener("click", async (e) => {
  const fileLink = e.target.closest("[data-download]");
  if (fileLink) {
    e.preventDefault();
    if (busy) return;
    busy = true;
    try {
      const response = await fetch(fileLink.href, { method: "HEAD" });
      if (!response.ok) throw new Error("This file is no longer available.");
      if (response.headers.get("x-content-sha256") !== fileLink.dataset.sha256)
        throw new Error(
          "The file did not match the approved snapshot. Please refresh the request.",
        );
      // The browser streams directly to its download destination. JavaScript
      // cannot reliably observe its completion, so receipt remains explicit.
      const a = document.createElement("a");
      a.href = fileLink.href;
      a.download = fileLink.dataset.name;
      a.click();
      notice(
        "Download started. Mark the response received when your download finishes.",
      );
    } catch (err) {
      error(err);
    } finally {
      busy = false;
    }
    return;
  }
  const button = e.target.closest("[data-action]");
  if (!button || busy) return;
  busy = true;
  button.disabled = true;
  try {
    const { action, id } = button.dataset;
    if (action === "restart-login") return login();
    if (action === "add-slack") {
      if (location.pathname !== "/slack/connected")
        sessionStorage.setItem(
          "slack-return",
          location.pathname + location.search,
        );
      const link = await api("/api/channels/slack/start", {});
      location.assign(link.url);
      return;
    }
    if (action === "add-email") {
      sessionStorage.removeItem("email-challenge");
      return emailSetup();
    }
    if (action === "restart-email") {
      sessionStorage.removeItem("email-challenge");
      return emailSetup();
    }
    if (action === "copy-invite") {
      await navigator.clipboard.writeText(
        location.origin + "/invite/" + profile.invite,
      );
      return notice("Contact link copied.");
    }
    if (action === "approve-permission" || action === "decline-permission") {
      const permission = currentTask.permissions.find((p) => p.id === id);
      if (!permission)
        throw new Error(
          "This permission is no longer pending. Refresh the request.",
        );
      await api(`/api/tasks/${currentTask.id}/permissions/${id}/decision`, {
        digest: permission.digest,
        decision: action === "approve-permission" ? "approve" : "decline",
      });
    } else if (action === "approve" || action === "decline") {
      const d = currentTask.draft;
      await api(`/api/tasks/${currentTask.id}/decision`, {
        version: d.version,
        digest: d.digest,
        decision: action,
      });
    } else if (action === "cancel" || action === "received")
      await api(`/api/tasks/${currentTask.id}/${action}`, {});
    else if (action === "accept-contact" || action === "block-contact")
      await api(
        `/api/contacts/${id}/${action === "accept-contact" ? "accept" : "block"}`,
        {},
      );
    else if (action === "default-agent")
      await api(`/api/agents/${id}/default`, {});
    else if (action === "revoke-agent")
      await api(`/api/agents/${id}`, undefined, "DELETE");
    else if (action === "revoke-connection")
      await api(`/api/connections/${id}`, undefined, "DELETE");
    else if (action === "use-invite") {
      await api("/api/contacts", { invite: id });
      location.href = "/contacts";
      return;
    } else if (action === "enable-phone") await api("/api/channels/phone", {});
    else if (action === "disable-channel")
      await api(`/api/channels/${id}`, undefined, "DELETE");
    else if (action === "logout") {
      await api("/auth/logout", {});
      profile = null;
      sessionStorage.removeItem("email-challenge");
      return login();
    }
    await render();
  } catch (err) {
    error(err);
  } finally {
    busy = false;
    button.disabled = false;
  }
});
setInterval(async () => {
  if (
    !profile ||
    !currentTask ||
    busy ||
    document.hidden ||
    !location.pathname.startsWith("/request/") ||
    editingTask()
  )
    return;
  try {
    const previous = currentTask;
    const t = await api(`/api/tasks/${previous.id}`);
    document.getElementById("sync-error")?.remove();
    if (currentTask !== previous || busy || editingTask()) return;
    if (
      t.updated_at !== currentTask.updated_at ||
      t.status !== currentTask.status ||
      t.draft?.version !== currentTask.draft?.version ||
      JSON.stringify(t.permissions) !==
        JSON.stringify(currentTask.permissions) ||
      t.received_at !== currentTask.received_at ||
      t.revoked_at !== currentTask.revoked_at ||
      t.downloads?.length !== currentTask.downloads?.length
    )
      await taskPage(t.id, t, previous);
  } catch (err) {
    error(
      {
        message: `Live updates paused. ${err.message} Retrying automatically…`,
      },
      "sync-error",
    );
  }
}, 4000);
boot();
