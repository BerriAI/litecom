const app = document.querySelector("#app"),
  nav = document.querySelector("#nav");
let profile,
  capabilities,
  account,
  localPairing,
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
const icon = (name) => {
  const paths = {
    workspace:
      '<rect x="3" y="3" width="7" height="7" rx="1.5"/><rect x="14" y="3" width="7" height="7" rx="1.5"/><rect x="3" y="14" width="7" height="7" rx="1.5"/><rect x="14" y="14" width="7" height="7" rx="1.5"/>',
    inbox:
      '<path d="M4 4h16l2 12v4H2v-4L4 4Z"/><path d="M2 15h6l2 3h4l2-3h6"/>',
    contacts:
      '<circle cx="9" cy="8" r="3"/><path d="M3 21v-3a6 6 0 0 1 12 0v3M16 5a3 3 0 0 1 0 6m3 10v-3a6 6 0 0 0-2-4"/>',
    agents:
      '<rect x="4" y="6" width="16" height="12" rx="2"/><path d="M8 22h8m-4-4v4M8 10l2 2-2 2m5 0h3"/>',
    settings:
      '<path d="M4 6h16M4 12h16M4 18h16"/><circle cx="8" cy="6" r="2"/><circle cx="16" cy="12" r="2"/><circle cx="10" cy="18" r="2"/>',
    phone:
      '<rect x="6" y="2" width="12" height="20" rx="3"/><path d="M10 18h4"/>',
    slack:
      '<path d="M21 11a9 9 0 0 1-9 9H3l2-5a9 9 0 1 1 16-4Z"/><path d="M8 10h8m-8 4h5"/>',
    email:
      '<rect x="2" y="4" width="20" height="16" rx="2"/><path d="m3 6 9 7 9-7"/>',
    arrow: '<path d="m9 6 6 6-6 6"/>',
    plus: '<path d="M12 5v14M5 12h14"/>',
    more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  };
  return `<svg class="icon" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${paths[name] || paths.agents}</svg>`;
};
const avatar = (name) =>
  `<span class="avatar" aria-hidden="true">${esc(String(name).trim().slice(0, 1).toUpperCase())}</span>`;
const pageHeading = (title, action = "", description = "") =>
  `<div class="page-heading"><div><h1>${title}</h1>${description ? `<p class="muted">${description}</p>` : ""}</div>${action}</div>`;
const rowMenu = (label, actions) =>
  `<details class="row-menu" name="row-actions"><summary aria-label="${esc(label)}" title="${esc(label)}">${icon("more")}</summary><div class="menu-panel">${actions}</div></details>`;
function shell(connected) {
  document.body.classList.toggle("workspace-shell", connected);
  nav.innerHTML = connected
    ? [
        ["workspace", "/workspace", "Workspace"],
        ["inbox", "/", "Inbox"],
        ["agents", "/agents", "Agents"],
        ["settings", "/settings", "Settings"],
      ]
        .map(
          ([name, href, label]) =>
            `<a href="${href}">${icon(name)}<span>${label}</span></a>`,
        )
        .join("")
    : "";
}
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
  return `<section class="card permission"><p class="eyebrow">Permission needed</p><h2>${esc(permission.title)}</h2><p>Allow this operation once for this request.</p>${operationFields(fields)}<div class="actions"><button class="primary" data-action="approve-permission" data-id="${esc(permission.id)}">Approve once</button><button data-action="decline-permission" data-id="${esc(permission.id)}">Decline operation</button></div></section>`;
}
function draftCard(draft, status) {
  const action = draft.action;
  const body = action
    ? `<p>${action.share_result ? "Approve to run this request and send back the result." : "Approve to run this request. This older proposal requires a separate result review."}</p><details><summary>Agent and access</summary><h2>${esc(action.title)}</h2><p>${esc(action.description)}</p><p class="muted">Acts as ${esc(action.acting_account)}.</p>${action.workspace ? `<p class="muted subtitle">Folder: <code>${esc(action.workspace)}</code></p>` : ""}<details><summary>Technical details</summary><pre>${esc(JSON.stringify(action, null, 2))}</pre></details></details>`
    : `<h2>${status === "completed" ? "Result" : "Reply"}</h2><div class="prose">${esc(draft.text)}</div>${files(draft.attachments)}`;
  const controls =
    status === "awaiting_approval"
      ? `<div class="actions"><button class="primary" data-action="approve">Approve</button><button data-action="decline">Decline</button></div>`
      : "";
  return action && ["approved", "executing"].includes(status)
    ? `<details class="card"><summary>Request approved</summary>${body}</details>`
    : `<section class="card approval-panel">${body}${controls}</section>`;
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
      if (e.status === 403) {
        login();
        error(e);
        return;
      }
      if (e.status !== 401) throw e;
    }
    if (!profile) {
      const pending = sessionStorage.getItem("phone-challenge");
      if (pending) {
        try {
          challenge = JSON.parse(pending);
          if (challenge.expires_at > Date.now()) return phoneCode();
        } catch {}
        sessionStorage.removeItem("phone-challenge");
      }
      return login();
    }
    account = await api("/api/account");
    if (location.pathname === "/slack/connected") return await slackConnected();
    const channels = await api("/api/channels");
    const selected = sessionStorage.getItem("approval-channel");
    if (selected && !channels.some((c) => c.kind === selected && c.verified)) {
      if (!availableChannels().includes(selected)) {
        sessionStorage.removeItem("approval-channel");
        return approvalSetup(
          "That channel is not available on this server. Choose another.",
        );
      }
      if (selected === "phone") {
        if (!account.has_phone) return phoneSetup();
        await api("/api/channels/phone", {});
      } else if (selected === "slack") return await slackSetup();
      else {
        emailSetup();
        const address = sessionStorage.getItem("approval-email-pending");
        if (address && !sessionStorage.getItem("email-challenge")) {
          sessionStorage.removeItem("approval-email-pending");
          await startEmail(address);
        }
        return;
      }
    } else if (
      !selected &&
      !channels.some((c) => c.verified && availableChannels().includes(c.kind))
    ) {
      return approvalSetup();
    }
    sessionStorage.removeItem("approval-channel");
    shell(true);
    await render();
  } catch (e) {
    error(e);
  }
}
function availableChannels() {
  return ["phone", "slack", "email"].filter(
    (kind) =>
      capabilities[kind] &&
      (capabilities.development || capabilities.direct_approvals?.[kind]),
  );
}
function approvalSetup(message = "") {
  shell(false);
  const labels = { phone: "Phone", slack: "Slack", email: "Email" };
  const details = {
    phone: "Approve by text",
    slack: "Approve in a direct message",
    email: account?.google_email || "Approve by replying",
  };
  const kinds = availableChannels();
  app.innerHTML = `<section class="narrow setup"><h1>Where should requests go?</h1><p class="muted">Choose where to approve. Add more later.</p>${message ? `<p class="error" role="alert">${esc(message)}</p>` : ""}<div class="channel-picker">${kinds.map((kind) => `<button class="channel-choice" data-action="choose-channel" data-id="${kind}" aria-label="Connect ${labels[kind]}">${icon(kind)}<span><strong>${labels[kind]}</strong><small>${esc(details[kind])}</small></span>${icon("arrow")}</button>`).join("")}</div>${!kinds.length ? '<p class="banner">No approval channels are available yet. Ask your host to enable one, then refresh.</p>' : ""}<button class="quiet" data-action="logout">Sign out</button></section>`;
}

function phoneSetup() {
  const pending = sessionStorage.getItem("phone-link-challenge");
  app.innerHTML = `<section class="narrow"><h1>${pending ? "Check your phone" : "Approve requests by text"}</h1><p class="muted">Receive approval requests by SMS. Message rates may apply; reply STOP to unsubscribe.</p>${pending ? '<form id="phone-link-verify"><label>Phone code<input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required autofocus></label><button class="primary">Connect phone</button></form><button class="quiet" data-action="restart-phone-link">Use another number</button>' : '<form id="phone-link-start"><label>Phone number<input name="phone" type="tel" autocomplete="tel" placeholder="+1 415 555 0123" required></label><button class="primary">Send code</button></form>'}<button class="quiet" data-action="choose-another-channel">Choose another channel</button></section>`;
}
function login() {
  shell(false);
  if (capabilities.google) {
    const next =
      location.pathname === "/"
        ? "/workspace"
        : location.pathname + location.search;
    app.innerHTML = `<section class="narrow"><h1>Your agents, connected.</h1><p class="muted">Ask someone’s agent. They approve. It gets done.</p><a class="button primary full-width" href="/api/auth/google?next=${encodeURIComponent(next)}">Continue with Google</a><p class="muted subtitle">Your name and agents will be visible in this workspace.</p>${new URLSearchParams(location.search).has("signin") ? '<p class="error" role="alert">Sign-in was not completed. Try again.</p>' : ""}${capabilities.phone_signin ? '<button class="quiet" data-action="phone-login">Sign in with phone instead</button>' : ""}</section>`;
    return;
  }
  phoneLogin();
}
function phoneLogin() {
  const requested = new URLSearchParams(location.search).get("approve");
  const chosen = ["phone", "slack", "email"].includes(requested)
    ? requested
    : null;
  if (chosen) sessionStorage.setItem("approval-channel", chosen);
  app.innerHTML = `<section class="narrow"><h1>Your agents, connected.</h1><p class="muted">Ask someone’s agent. They approve. It gets done.</p>${capabilities.development ? '<p class="banner">Local development. Codes appear in the relay terminal; no phone messages are sent.</p>' : ""}<form id="login">${
    chosen
      ? `<input type="hidden" name="channel" value="${chosen}"><p class="banner">Approve requests on ${esc(chosen)}. You can add channels later.</p>`
      : `<fieldset><legend>Where do you want to approve requests?</legend><div class="choices">${availableChannels()
          .map((c) => ({ phone: "Phone", slack: "Slack", email: "Email" })[c])
          .map(
            (c, i) =>
              `<label><input type="radio" name="channel" value="${c.toLowerCase()}" ${!i ? "checked" : ""}>${c}</label>`,
          )
          .join("")}</div></fieldset>`
  }<label>Your name<input name="name" autocomplete="name" required></label><label id="approval-email-field" hidden>Email for approvals<input name="approval_email" type="email" autocomplete="email" disabled></label><label>Phone number<input name="phone" type="tel" autocomplete="tel" placeholder="+1 415 555 0123" required></label><p class="muted subtitle">We’ll text a sign-in code. Message rates may apply; reply STOP to unsubscribe.</p><button class="primary" ${!capabilities.phone ? "disabled" : ""}>Send sign-in code</button>${!capabilities.phone ? '<p class="error">Phone delivery is not enabled on this server. Ask its owner to connect a messaging provider.</p>' : ""}</form></section>`;
  syncApprovalFields();
}
function syncApprovalFields() {
  const form = document.getElementById("login");
  if (!form) return;
  const email = new FormData(form).get("channel") === "email";
  const field = document.getElementById("approval-email-field");
  field.hidden = !email;
  const input = field.querySelector("input");
  input.disabled = !email;
  input.required = email;
}
function phoneCode() {
  app.innerHTML = `<section class="narrow"><h1>Check your phone</h1><p class="muted">Enter the six-digit code. It expires in 10 minutes.</p><form id="verify"><label>Sign-in code<input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required autofocus></label><button class="primary">Continue</button></form><p><button class="quiet" data-action="restart-login">Use another number</button></p></section>`;
}
async function startEmail(address) {
  const c = await api("/api/channels/email/start", { email: address });
  sessionStorage.setItem("email-challenge", c.id);
  emailSetup();
}

async function startSlack() {
  if (location.pathname !== "/slack/connected") {
    const returnTo = location.pathname + location.search;
    sessionStorage.setItem("slack-return", returnTo);
    sessionStorage.setItem("slack-started-url", returnTo);
  }
  const link = await api("/api/channels/slack/start", {});
  location.assign(link.url);
}
async function slackSetup() {
  app.innerHTML =
    '<section class="narrow"><h1>Approve requests in Slack</h1><p class="muted">Allow Litecom to send you approval requests in Slack.</p><button class="primary" data-action="add-slack">Continue to Slack</button><button class="quiet" data-action="choose-another-channel">Choose another channel</button></section>';
  // Choosing Slack already expresses intent. Go straight to its consent page,
  // but do not send the user back there automatically after Back or a failure.
  if (
    sessionStorage.getItem("slack-started-url") !==
    location.pathname + location.search
  ) {
    const button = app.querySelector("[data-action=add-slack]");
    button.disabled = true;
    button.textContent = "Opening Slack…";
    try {
      await startSlack();
    } catch (e) {
      button.disabled = false;
      button.textContent = "Try again";
      throw e;
    }
  }
}
async function slackConnected() {
  const params = new URLSearchParams(location.search);
  if (params.has("error")) {
    app.innerHTML =
      '<section class="narrow"><h1>Slack was not connected.</h1><p class="muted">You can retry, or ask your workspace administrator to allow the Litecom app.</p><button class="primary" data-action="add-slack">Try again</button><button class="quiet" data-action="choose-another-channel">Choose another channel</button></section>';
    return;
  }
  await api("/api/channels/slack/finish", { state: params.get("state") });
  sessionStorage.removeItem("approval-channel");
  sessionStorage.removeItem("slack-started-url");
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
  app.innerHTML = `<section class="narrow"><h1>${pending ? "Check your email" : "Approve requests by email"}</h1><p class="muted">Receive requests here. Reply Approve to run them.</p>${capabilities.development ? '<p class="banner">Local development: email codes print in the relay terminal.</p>' : ""}${pending ? `<form id="email-verify"><label>Email code<input name="code" inputmode="numeric" autocomplete="one-time-code" pattern="[0-9]{6}" maxlength="6" required autofocus></label><button class="primary">Connect email</button></form><button class="quiet" data-action="restart-email">Use another email</button>` : `<form id="email-start"><label>Email address<input name="email" type="email" autocomplete="email" required></label><button class="primary">Send email code</button></form>`}<button class="quiet" data-action="choose-another-channel">Choose another channel</button></section>`;
}

async function render() {
  const parts = location.pathname.split("/").filter(Boolean);
  const title =
    {
      request: "Request",
      connect: "Connect agent",
      invite: "Ask an agent",
      workspace: "Workspace",
      contacts: "Contacts",
      agents: "Agents",
      settings: "Account",
    }[parts[0]] || "Inbox";
  document.title = `${title} · Litecom`;
  document.querySelectorAll("#nav a").forEach((link) => {
    if (
      new URL(link.href).pathname ===
      (location.pathname.startsWith("/request/") ? "/" : location.pathname)
    )
      link.setAttribute("aria-current", "page");
    else link.removeAttribute("aria-current");
  });
  if (parts[0] === "request" && parts[1]) return taskPage(parts[1]);
  if (parts[0] === "connect" && parts[1]) return connectPage(parts[1]);
  if (parts[0] === "invite" && parts[1]) return invitePage(parts[1]);
  if (parts[0] === "workspace") return workspacePage();
  if (parts[0] === "contacts") { location.replace("/workspace"); return; }
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
  const [tasks, contacts, ownAgents] = await Promise.all([
    api("/api/tasks" + query),
    api("/api/contacts"),
    api("/api/agents"),
  ]);
  const requested = new URLSearchParams(location.search).get("ask");
  const owner = contacts.some(
    (c) => c.id === requested && c.status === "accepted",
  )
    ? requested
    : profile.id;
  const agents =
    owner === profile.id
      ? ownAgents
      : await api("/api/agents?owner_id=" + encodeURIComponent(owner));
  const names = new Map(contacts.map((c) => [c.id, c.name]));
  names.set(profile.id, "You");
  app.innerHTML = `${pageHeading("Inbox")}<details class="composer" ${requested ? "open" : ""}><summary>${icon("plus")}Ask an agent</summary><form id="ask"><label>Who<select name="owner"><option value="${esc(profile.id)}">My agents</option>${contacts
    .filter((c) => c.status === "accepted")
    .map(
      (c) =>
        `<option value="${esc(c.id)}" ${c.id === owner ? "selected" : ""}>${esc(c.name)}</option>`,
    )
    .join(
      "",
    )}</select></label><label>Agent<select name="agent" required>${agentOptions(agents)}</select></label><label>What do you need?<textarea name="request" placeholder="Find the launch notes and send me the final PDF." required></textarea></label><button class="primary">Send request</button></form></details><div class="request-list">${tasks.length ? tasks.map((t) => `<article class="request-row"><div class="row"><a class="title-link" href="/request/${encodeURIComponent(t.id)}">${esc(t.request.length > 180 ? t.request.slice(0, 180) + "…" : t.request)}</a>${badge(t.status, t)}</div><p class="muted subtitle">${esc(names.get(t.requester_id) || "Contact")} → ${esc(names.get(t.owner_id) || "Contact")}${t.received_at ? " · Received" : ""}${t.revoked_at ? " · Access revoked" : ""}</p></article>`).join("") : query ? '<div class="empty"><p>No older requests.</p></div>' : '<div class="empty"><h2>No requests yet</h2><p>Ask an agent above, or <a href="/workspace">find someone to ask</a>.</p></div>'}</div>`;
  const requestedAgent = new URLSearchParams(location.search).get("agent");
  if (agents.some((agent) => agent.id === requestedAgent)) {
    document.querySelector('#ask select[name="agent"]').value = requestedAgent;
  }
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
  app.innerHTML = `<p><a href="/">← Inbox</a></p><div class="row"><h1>${owner && !requester ? esc(names.get(t.requester_id) || "Your contact") + " asks" : "Your request"}</h1>${badge(t.status, t)}</div><p class="muted">${esc(names.get(t.requester_id) || "Contact")} → ${esc(names.get(t.owner_id) || "Contact")}${agent ? " · " + esc(agent.name) : ""}</p><div class="request-body prose">${esc(t.request)}</div>${t.downloads?.length ? `<p class="muted subtitle">${t.downloads.length} file download${t.downloads.length === 1 ? "" : "s"} verified by the recipient.</p>` : ""}${t.revoked_at ? '<p class="banner">Access has been revoked. Files already downloaded cannot be recalled.</p>' : ""}${t.status === "queued" ? '<p class="banner">Waiting for the agent to connect.</p>' : ""}${(t.permissions || []).map(permissionCard).join("")}${d ? draftCard(d, t.status) : ""}${t.response && !owner ? `<section class="card"><h2>Response</h2><div class="prose">${esc(t.response.text)}</div>${files(t.response.attachments)}</section>` : ""}${t.execution ? `<section class="card"><h2>Cloud session</h2><p class="prose">${esc(t.execution.message)}</p>${t.execution.url ? `<a href="${esc(t.execution.url)}" target="_blank" rel="noopener noreferrer">Open in Devin ↗</a>` : ""}</section>` : ""}${t.receipt ? `<section class="card"><h2>Action result</h2><p class="prose">${esc(t.receipt.summary)}</p></section>` : ""}${editable ? `<details class="disclosure"><summary>${d ? "Edit response" : "Prepare a response yourself"}</summary><form id="draft"><label>Reply<textarea name="text">${esc(d?.text || "")}</textarea></label><label>Attach files<input name="files" type="file" multiple></label><p class="muted subtitle">Saving a change requires a fresh approval.</p><button>Save for review</button></form></details>` : ""}${requester && t.status === "completed" && !t.revoked_at ? `<div class="actions">${t.received_at ? '<p class="muted">Received.</p>' : '<button data-action="received">Mark received</button>'}</div>` : ""}${!["declined", "cancelled", "failed", "expired"].includes(t.status) ? `<div class="actions"><button class="danger" data-action="cancel">${owner && t.status === "completed" ? "Revoke access" : "Cancel request"}</button></div>` : ""}`;
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
function agentLabel(agent) {
  const name = agent.name === "Default" ? "Local agent" : agent.name;
  return agent.local?.harness ? `${name} · ${agent.local.harness}` : name;
}
function agentOptions(agents) {
  return agents.length
    ? agents
        .map(
          (agent) =>
            `<option value="${esc(agent.id)}" ${agent.is_default ? "selected" : ""}>${esc(agentLabel(agent))}</option>`,
        )
        .join("")
    : '<option value="">No agents available</option>';
}
function agentSummary(agents) {
  return agents.filter((agent) => agent.connected).map((agent) => esc(agentLabel(agent))).join(" / ") || "No agents connected";
}
function agentStatus(agent) {
  if (!agent.connected) return "Not connected";
  if (agent.kind !== "local") return "Connected";
  return agent.last_seen_at >= Date.now() / 1000 - 60
    ? "Online"
    : "Offline · last seen " + new Date(agent.last_seen_at * 1000).toLocaleString();
}
function yourAgents(agents) {
  const connected = agents.filter((agent) => agent.connected);
  return connected.length ? `<div class="list">${connected.map((agent) => `<article class="list-row">${icon("agents")}<div class="row-content"><h3>${esc(agentLabel(agent))}${agent.is_default ? '<span class="inline-meta">Default</span>' : ""}</h3><p class="muted">${esc(agentStatus(agent))}</p></div><a class="button" href="/?ask=me&agent=${encodeURIComponent(agent.id)}" aria-label="Ask ${esc(agentLabel(agent))}">Ask agent</a></article>`).join("")}</div>` : '<p class="muted">No agents connected</p>';
}
async function workspacePage() {
  try {
    localPairing = JSON.parse(sessionStorage.getItem("local-pairing"));
  } catch {
    sessionStorage.removeItem("local-pairing");
  }
  const [people, agents] = await Promise.all([
    api("/api/people"),
    api("/api/agents"),
  ]);
  app.innerHTML = `${pageHeading("Your workspace", `<div class="row-actions"><button class="quiet" data-action="copy-setup-prompt">Copy setup prompt</button><button class="primary" data-action="connect-local">${icon("plus")}Connect local agent</button></div>`, "Find a person. Ask their agent.")}<div id="local-command"></div><section class="section"><h2 class="section-label">You</h2><div class="list-row identity-row">${avatar(profile.name)}<div class="row-content"><h2>${esc(profile.name)}</h2>${account.google_email ? `<p class="muted">${esc(account.google_email)}</p>` : ""}</div></div><div id="your-agents">${yourAgents(agents)}</div></section><section class="section"><div class="section-heading"><h2 class="section-label">People</h2><button class="quiet" data-action="copy-invite">Invite someone</button></div><div class="list">${people.map((person) => `<article class="list-row">${avatar(person.name)}<div class="row-content"><h2>${esc(person.name)}</h2><p class="muted">${agentSummary(person.agents)}</p></div><div class="row-actions"><a class="button" href="/?ask=${encodeURIComponent(person.id)}" aria-label="Ask ${esc(person.name)}’s agent">Ask agent</a>${rowMenu(`Manage ${person.name}`, `<button class="danger" data-action="block-contact" data-id="${esc(person.id)}">Block person</button>`)}</div></article>`).join("") || '<div class="empty"><p>No one else is here yet. Invite someone to your workspace.</p></div>'}</div></section>`;
  showLocalCommand();
  await pollLocalConnection();
}

async function agentsPage() {
  const agents = await api("/api/agents");
  try {
    localPairing = JSON.parse(sessionStorage.getItem("local-pairing"));
  } catch {
    sessionStorage.removeItem("local-pairing");
  }
  app.innerHTML = `${pageHeading("Your agents", `<button class="primary" data-action="connect-local">${icon("plus")}Connect local agent</button>`)}<div id="local-command"></div><div class="list">${agents.map((a) => `<section class="list-row">${icon("agents")}<div class="row-content"><h2 data-agent-name="${esc(a.id)}">${esc(agentLabel(a))}${a.is_default ? '<span class="inline-meta">Default</span>' : ""}</h2><p class="muted" data-agent-status="${esc(a.id)}">${esc(agentStatus(a))}</p></div>${rowMenu(`Manage ${a.name}`, `${a.kind === "local" ? `<button data-action="connect-local" data-id="${esc(a.id)}" aria-label="Connect ${esc(a.name)}">Connect</button>` : ""}${!a.is_default ? `<button data-action="default-agent" data-id="${esc(a.id)}">Make default</button>` : ""}<button class="danger" data-action="revoke-agent" data-id="${esc(a.id)}">Disconnect</button>`)}</section>`).join("")}</div><details class="disclosure"><summary>Add another local agent</summary><form id="agent"><label>Name<input name="name" placeholder="My laptop" required></label><input name="kind" type="hidden" value="local"><p class="muted subtitle">Create a name, then connect the agent from its computer.</p><button>Create agent</button></form></details><details class="disclosure" id="devin-connect"><summary>Connect Devin</summary><p class="muted">Use a token from <a href="https://app.devin.ai/settings/api-keys" target="_blank" rel="noopener noreferrer">Devin Settings ↗</a>.</p><form id="devin"><label>Devin token<input name="token" type="password" autocomplete="off" placeholder="cog_…" required></label><details><summary>Name and organization</summary><label>Agent name<input name="name" value="Devin" required></label><label>Organization ID (if not detected)<input name="org_id" placeholder="org-…"></label><p class="muted subtitle">Use the same name to reconnect.</p></details><p class="muted subtitle">Your token is encrypted on this server. Approved runs use your Devin plan.</p><button class="primary">Connect Devin</button></form></details>`;
  showLocalCommand();
  await pollLocalConnection();
}

const shellQuote = (value) => "'" + value.replaceAll("'", "'\"'\"'") + "'";
const psQuote = (value) => "'" + value.replaceAll("'", "''") + "'";
async function connectLocal(agentId, method = "command") {
  const agents = await api("/api/agents");
  const agent = agentId
    ? agents.find((a) => a.id === agentId && a.kind === "local")
    : agents.find((a) => a.kind === "local" && a.is_default) ||
      agents.find((a) => a.kind === "local") ||
      (await api("/api/agents", { name: "This computer", kind: "local" }));
  if (!agent)
    throw new Error(
      "This agent is no longer available. Refresh and try again.",
    );
  localPairing = await api("/api/connections/local", { agent_id: agent.id });
  localPairing.agent_id = agent.id;
  localPairing.agent_name = ["Default", "This computer"].includes(agent.name) ? "this computer" : agent.name;
  localPairing.started = Date.now();
  localPairing.method = method;
  sessionStorage.setItem("local-pairing", JSON.stringify(localPairing));
  showLocalCommand();
}
function showLocalCommand() {
  const target = document.getElementById("local-command");
  if (!target || !localPairing) return;
  document
    .querySelectorAll("[data-action=connect-local]")
    .forEach((button) => (button.hidden = true));
  const name = "local-" + localPairing.user_code.toLowerCase();
  const windows = /Windows/i.test(navigator.userAgent);
  const command = windows
    ? `& ([scriptblock]::Create((irm ${psQuote(location.origin + "/install.ps1")}))) -Relay ${psQuote(location.origin)} -Code ${psQuote(localPairing.device_code)} -Connection ${psQuote(name)}`
    : `curl -fsSL ${shellQuote(location.origin + "/install.sh")} | sh -s -- --connection ${shellQuote(name)} setup --relay ${shellQuote(location.origin)} --connect-code ${shellQuote(localPairing.device_code)}`;
  if (localPairing.method === "prompt") {
    target.innerHTML = `<section class="connection-panel"><div class="row"><h2>Set up with your agent</h2><button class="primary" data-action="copy-setup-prompt">Copy prompt</button></div><p class="muted">Paste the prompt into your coding agent. It connects to this workspace and sets up MCP.</p><p class="muted subtitle">Private prompt · Expires in 10 minutes</p><p id="connect-status" class="connection-status" role="status">Waiting for your agent to connect.</p><details><summary>View setup prompt</summary><textarea id="setup-prompt" readonly rows="7" spellcheck="false" aria-label="Agent setup prompt">${esc(setupPrompt(command, windows, name))}</textarea></details><details><summary>Use a terminal instead</summary><label class="command-label">${windows ? "PowerShell command" : "Terminal command"}<input id="connect-command" readonly spellcheck="false" value="${esc(command)}"></label><button data-action="copy-connect">Copy command</button></details></section>`;
    return;
  }
  target.innerHTML = `<section class="connection-panel"><div class="row"><h2>Connect ${esc(localPairing.agent_name || "this computer")}</h2><button class="primary" data-action="copy-connect">Copy command</button></div><p class="muted">Paste into any terminal. Your agent starts in your home folder.</p><label class="command-label">${windows ? "PowerShell command" : "Terminal command"}<input id="connect-command" readonly spellcheck="false" value="${esc(command)}"></label><p class="muted subtitle">Private command · Expires in 10 minutes</p><p id="connect-status" class="connection-status" role="status">Waiting for you to run the command.</p><details><summary>Set up with your agent</summary><p class="muted">Copy this prompt into your coding agent.</p><textarea id="setup-prompt" readonly rows="5" spellcheck="false" aria-label="Agent setup prompt">${esc(setupPrompt(command, windows, name))}</textarea><button data-action="copy-setup-prompt">Copy setup prompt</button></details><details><summary>What this sets up</summary><p>Installs Litecom, connects your chosen agent and adds MCP. The agent runs in the background. Requests need your approval.</p></details></section>`;
}
function setupPrompt(command, windows, connection) {
  return `Connect this coding agent to my Litecom workspace at ${location.origin}. Use the private one-use setup command below (expires in 10 minutes). Install the published binary, connect this agent, configure Litecom MCP, and start its background worker. No Rust or source checkout is needed.

${windows ? "PowerShell" : "Terminal"} command:
${command}

Run from any folder. New connections use the user’s home folder by default. Detect which supported coding agent you are using and pass ${windows ? "-Agent" : "--agent"} to select it without interactive prompts. Do not set ${windows ? "-Workspace" : "--workspace"} unless I specifically request a different folder. Preserve the saved folder when reconnecting. Use only the existing signed-in agent account; preserve other connections, MCP servers, skills, and hooks. Do not print or share the private connection code or credentials. If no supported signed-in coding agent is available, tell me what to install or sign in to. If this command has expired, ask me to copy a new prompt from the workspace.

Verify with litecom --connection ${connection} doctor. Confirm the worker is connected and MCP is configured; tell me if this chat needs a restart to load the tools. Do not approve requests or send a test request unless I ask. Finish with the workspace URL and how to ask another person's agent.`;
}
async function pollLocalConnection() {
  if (!localPairing || !document.getElementById("connect-status") || busy)
    return;
  const current = localPairing;
  const target = document.getElementById("connect-status");
  if (Date.now() - current.started > 30 * 60 * 1000) {
    document
      .querySelectorAll("[data-action=connect-local]")
      .forEach((button) => (button.hidden = false));
    target.textContent =
      "This connection check has stopped. Refresh to check again, or generate a new command.";
    return;
  }
  try {
    const result = await api(
      `/api/connections/local/${encodeURIComponent(current.user_code)}`,
    );
    if (localPairing !== current || !target.isConnected) return;
    const messages = {
      waiting: current.method === "prompt" ? "Waiting for your agent to connect." : "Waiting for you to run the command.",
      starting: current.method === "prompt" ? "Your agent is finishing setup." : "Finishing setup. Keep the terminal open.",
      connected: "Connected. Ready for requests.",
      offline:
        "Setup finished, but the worker is offline. Run litecom doctor on this computer.",
      expired: "This command expired. Click Connect local agent for a new one.",
      disconnected:
        "This connection was disconnected. Generate a new command to reconnect.",
    };
    target.textContent = messages[result.status] || "Checking connection…";
    if (["connected", "expired", "disconnected"].includes(result.status)) {
      document
        .querySelectorAll("[data-action=connect-local]")
        .forEach((button) => (button.hidden = false));
      sessionStorage.removeItem("local-pairing");
      localPairing = null;
      const panel = document.getElementById("local-command");
      if (panel)
        panel.innerHTML = `<div class="connection-result"><p id="connect-status" role="status">${esc(messages[result.status])}</p>${result.status === "connected" ? '<a href="/?ask=me">Ask your agent →</a>' : ""}</div>`;
      if (result.status === "connected") {
        const row = [...document.querySelectorAll("[data-agent-status]")].find(
          (el) => el.dataset.agentStatus === current.agent_id,
        );
        const agents = await api("/api/agents");
        const connected = agents.find((agent) => agent.id === current.agent_id);
        if (row && connected) row.textContent = agentStatus(connected);
        const heading = document.querySelector(`[data-agent-name="${CSS.escape(current.agent_id)}"]`);
        if (heading && connected) heading.innerHTML = `${esc(agentLabel(connected))}${connected.is_default ? '<span class="inline-meta">Default</span>' : ""}`;
        const own = document.getElementById("your-agents");
        if (own) own.innerHTML = yourAgents(agents);
      }
    }
  } catch (e) {
    if (target.isConnected)
      target.textContent = "Connection check failed. Retrying…";
  }
}
setInterval(pollLocalConnection, 2000);
async function connectPage(code) {
  const [device, agents] = await Promise.all([
    api(`/api/device/${encodeURIComponent(code)}`),
    api("/api/agents"),
  ]);
  app.innerHTML = `<section class="narrow"><h1>Connect this agent?</h1><p><strong>${esc(device.label)}</strong> wants to join your Litecom account.</p><div class="card"><p class="muted">Check that this code matches the one in your terminal.</p><p class="code">${esc(code)}</p><form id="device" data-code="${esc(code)}"><label>Connect as<select name="agent">${agents.map((a) => `<option value="${esc(a.id)}">${esc(agentLabel(a))}${a.is_default ? " (default)" : ""}</option>`).join("")}</select></label><p class="muted subtitle">It can ask workspace agents, access your inbox, and prepare work for this agent. Only you can approve.</p><button class="primary">Connect agent</button></form></div></section>`;
}
async function invitePage(code) {
  const p = await api(`/api/invite/${encodeURIComponent(code)}`);
  app.innerHTML = `<section class="narrow"><h1>Ask ${esc(p.name)}’s agent</h1><p>They approve the request. Their agent sends the result.</p><a class="button primary" href="/?ask=${encodeURIComponent(p.id)}">Ask their agent</a></section>`;
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
    failed: "Delivery failed. Reconnect to try again.",
  };
  const rows = availableChannels()
    .map((kind) => {
      const channel = channels.find((c) => c.kind === kind && c.verified);
      const addAction = {
        phone: "enable-phone",
        email: "add-email",
        slack: "add-slack",
      }[kind];
      return `<div class="list-row">${icon(kind)}<div class="row-content"><h3>${labels[kind]}</h3><p class="muted">${channel ? "Connected" : "Not connected"}</p>${channel?.delivery_status === "failed" ? `<p class="error">${deliveryLabels.failed}</p>` : ""}</div>${channel ? rowMenu(`Manage ${labels[kind]}`, `${kind !== "phone" ? `<button data-action="${addAction}">Reconnect ${labels[kind]}</button>` : ""}<button class="danger" data-action="disable-channel" data-id="${esc(channel.id)}">Turn off ${labels[kind]}</button>${channel.delivery_status && deliveryLabels[channel.delivery_status] ? `<p class="muted subtitle">${deliveryLabels[channel.delivery_status]}</p>` : ""}`) : `<button data-action="${addAction}">Connect ${labels[kind]}</button>`}</div>`;
    })
    .join("");
  const uninstallCommand = /Windows/i.test(navigator.userAgent)
    ? `& ([scriptblock]::Create((irm ${psQuote(location.origin + "/uninstall.ps1")})))`
    : `curl -fsSL ${shellQuote(location.origin + "/uninstall.sh")} | sh`;
  app.innerHTML = `${pageHeading("Settings")}<section class="section"><h2>Approval channels</h2><p class="muted">One approval updates every channel.</p><div class="list">${rows || '<p class="empty">No channels enabled. Ask your host to connect one.</p>'}</div>${capabilities.development ? '<p class="muted subtitle">Development mode · Notifications appear in the terminal.</p>' : ""}</section><section class="section"><h2>Connected apps</h2><div class="list">${connections.map((c) => `<div class="list-row"><div class="row-content">${esc(c.label)}</div><button class="quiet danger" data-action="revoke-connection" data-id="${esc(c.id)}">Disconnect</button></div>`).join("") || '<p class="muted">No connected apps.</p>'}</div></section><section class="section"><details><summary>Uninstall from this computer</summary><p>Run in your terminal, then select the connections to remove. Your agents, files and workspace account stay.</p><div class="row"><label class="command-label row-content">Uninstall command<input id="uninstall-command" readonly spellcheck="false" value="${esc(uninstallCommand)}"></label><button class="quiet" data-action="copy-uninstall">Copy command</button></div></details></section><section class="section account-row"><div>${avatar(profile.name)}<span>${esc(profile.name)}</span></div><button class="quiet" data-action="logout">Sign out</button></section>`;
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
    if (form.id === "phone-link-start") {
      const c = await api("/api/channels/phone/start", {
        phone: data.get("phone").replace(/[ ()-]/g, ""),
      });
      sessionStorage.setItem("phone-link-challenge", c.id);
      phoneSetup();
    } else if (form.id === "phone-link-verify") {
      await api("/api/channels/phone/verify", {
        id: sessionStorage.getItem("phone-link-challenge"),
        code: data.get("code"),
      });
      sessionStorage.removeItem("phone-link-challenge");
      sessionStorage.removeItem("approval-channel");
      await boot();
    } else if (form.id === "login") {
      const channel = data.get("channel");
      if (channel !== "phone" && !capabilities[channel]) {
        throw new Error(
          `${channel === "slack" ? "Slack" : "Email"} is not enabled on this server. Choose an available approval channel.`,
        );
      }
      sessionStorage.setItem("approval-channel", channel);
      sessionStorage.removeItem("slack-started-url");
      if (channel === "email") {
        sessionStorage.setItem(
          "approval-email-pending",
          data.get("approval_email"),
        );
      } else {
        sessionStorage.removeItem("approval-email-pending");
      }
      challenge = await api("/auth/start", {
        name: data.get("name"),
        phone: data.get("phone").replace(/[ ()-]/g, ""),
      });
      challenge.expires_at = Date.now() + challenge.expires_in * 1000;
      sessionStorage.setItem("phone-challenge", JSON.stringify(challenge));
      phoneCode();
    } else if (form.id === "verify") {
      sessionStorage.removeItem("email-challenge");
      await api("/auth/verify", { id: challenge.id, code: data.get("code") });
      sessionStorage.removeItem("phone-challenge");
      if (sessionStorage.getItem("approval-channel") === "phone")
        await api("/api/channels/phone", {});
      await boot();
    } else if (form.id === "email-start") {
      await startEmail(data.get("email"));
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
      if (form.elements.agent.disabled || !data.get("agent"))
        throw new Error("Choose an available agent before sending.");
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
      const agent = await api("/api/agents", {
        name: data.get("name"),
        kind: data.get("kind"),
      });
      await render();
      await connectLocal(agent.id);
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
        '<section class="narrow"><h1>Computer connected.</h1><p>Litecom is finishing setup automatically. Keep the terminal open until it says it is listening for requests. Any setup issue will appear there.</p><a href="/">Open inbox</a></section>';
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
  if (e.target.name === "channel") syncApprovalFields();
  if (e.target.form?.id === "draft") e.target.form.dataset.dirty = "true";
  if (e.target.form?.id === "ask") delete e.target.form.dataset.requestId;
  if (e.target.name === "owner") {
    const form = e.target.form,
      owner = e.target.value,
      select = form.elements.agent;
    select.innerHTML = '<option value="">Loading agents…</option>';
    select.disabled = true;
    try {
      const agents = await api(
        "/api/agents?owner_id=" + encodeURIComponent(owner),
      );
      if (!form.isConnected || form.elements.owner.value !== owner) return;
      select.innerHTML = agentOptions(agents);
    } catch (err) {
      if (form.isConnected && form.elements.owner.value === owner) {
        select.innerHTML = '<option value="">Choose a person to retry</option>';
        error(err);
      }
    } finally {
      if (form.isConnected && form.elements.owner.value === owner)
        select.disabled = false;
    }
  }
});
document.addEventListener("click", async (e) => {
  document.querySelectorAll(".row-menu[open]").forEach((menu) => {
    if (!menu.contains(e.target)) menu.open = false;
  });
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
    if (action === "connect-local") return await connectLocal(id);
    if (action === "copy-setup-prompt") {
      if (!localPairing || Date.now() - localPairing.started >= 10 * 60 * 1000)
        await connectLocal(id, "prompt");
      else {
        localPairing.method = "prompt";
        sessionStorage.setItem("local-pairing", JSON.stringify(localPairing));
        showLocalCommand();
      }
      const prompt = document.getElementById("setup-prompt");
      try {
        await navigator.clipboard.writeText(prompt.value);
        return notice("Setup prompt copied. Paste it into your coding agent.");
      } catch {
        prompt.closest("details").open = true;
        prompt.focus();
        prompt.select();
        return notice("Select and copy the setup prompt below.");
      }
    }
    if (action === "copy-connect" || action === "copy-uninstall") {
      await navigator.clipboard.writeText(
        document.getElementById(action === "copy-connect" ? "connect-command" : "uninstall-command").value,
      );
      return notice("Command copied. Paste it into your terminal and run it.");
    }
    if (action === "phone-login") return phoneLogin();
    if (action === "choose-another-channel") {
      sessionStorage.removeItem("approval-channel");
      return approvalSetup();
    }
    if (action === "restart-phone-link") {
      sessionStorage.removeItem("phone-link-challenge");
      return phoneSetup();
    }
    if (action === "choose-channel") {
      sessionStorage.setItem("approval-channel", id);
      sessionStorage.removeItem("slack-started-url");
      if (id === "email" && account.google_email) {
        await api("/api/channels/email/google", {});
        sessionStorage.removeItem("approval-channel");
      }
      return await boot();
    }

    if (action === "restart-login") {
      sessionStorage.removeItem("phone-challenge");
      return login();
    }
    if (action === "add-slack") {
      await startSlack();
      return;
    }
    if (action === "add-email") {
      if (account.google_email) {
        await api("/api/channels/email/google", {});
        await render();
        return notice("Email connected.");
      }
      sessionStorage.removeItem("email-challenge");
      return emailSetup();
    }
    if (action === "restart-email") {
      sessionStorage.removeItem("email-challenge");
      return emailSetup();
    }
    if (action === "copy-invite") {
      await navigator.clipboard.writeText(
        location.origin + "/workspace",
      );
      return notice("Workspace link copied.");
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
    else if (action === "block-contact") {
      if (!confirm("Block requests between you and this person? This also revokes shared results.")) return;
      await api(`/api/contacts/${id}/block`, {});
    }
    else if (action === "default-agent")
      await api(`/api/agents/${id}/default`, {});
    else if (action === "revoke-agent")
      await api(`/api/agents/${id}`, undefined, "DELETE");
    else if (action === "revoke-connection")
      await api(`/api/connections/${id}`, undefined, "DELETE");
    else if (action === "enable-phone") {
      if (!account.has_phone) {
        sessionStorage.setItem("approval-channel", "phone");
        return phoneSetup();
      }
      await api("/api/channels/phone", {});
    } else if (action === "disable-channel")
      await api(`/api/channels/${id}`, undefined, "DELETE");
    else if (action === "logout") {
      await api("/auth/logout", {});
      profile = null;
      sessionStorage.clear();
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
document.addEventListener("keydown", (e) => {
  if (e.key !== "Escape") return;
  const menu = document.activeElement?.closest(".row-menu[open]");
  if (menu) {
    menu.open = false;
    menu.querySelector("summary").focus();
    e.preventDefault();
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
