use crate::{
    crypto::secret,
    error::{Error, Result, require, validate},
    model::*,
    providers::Providers,
    store::Store,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, FromRequest, Multipart, Path, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Mutex<Store>>,
    pub origin: String,
    pub providers: Providers,
    limits: Arc<Mutex<HashMap<String, (Instant, u32)>>>,
}
impl AppState {
    pub fn new(mut store: Store, origin: String, providers: Providers) -> Self {
        store.workspace_policy = providers.google.clone();
        Self {
            store: Arc::new(Mutex::new(store)),
            origin: origin.trim_end_matches('/').into(),
            providers,
            limits: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn db(&self) -> Result<MutexGuard<'_, Store>> {
        self.store
            .lock()
            .map_err(|_| Error::Internal(anyhow::anyhow!("database lock poisoned")))
    }
    fn limit(&self, key: &str, count: u32) -> Result<()> {
        let mut limits = self
            .limits
            .lock()
            .map_err(|_| Error::Internal(anyhow::anyhow!("rate limit lock poisoned")))?;
        limits.retain(|_, (start, _)| start.elapsed() < Duration::from_secs(60));
        if limits.len() >= 10_000 && !limits.contains_key(key) {
            return Err(Error::RateLimited);
        }
        let entry = limits.entry(key.into()).or_insert((Instant::now(), 0));
        entry.1 += 1;
        if entry.1 > count {
            Err(Error::RateLimited)
        } else {
            Ok(())
        }
    }
    fn origin(&self, h: &HeaderMap) -> Result<()> {
        require(
            h.get(header::ORIGIN).and_then(|h| h.to_str().ok()) == Some(&self.origin),
            "Open this page directly on your Litecom server.",
        )
    }
    pub fn principal(&self, h: &HeaderMap, write: bool) -> Result<Principal> {
        if let Some(token) = h
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        {
            let p = self.db()?.authenticate(token, Role::Agent)?;
            self.check_workspace(&p)?;
            return Ok(p);
        }
        let token = cookie(h, "litecom_session").ok_or(Error::Unauthorized)?;
        let db = self.db()?;
        let p = db.authenticate(&token, Role::Human)?;
        if write {
            self.origin(h)?;
            db.check_csrf(
                &p,
                h.get("x-csrf-token")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or(""),
            )?;
        }
        drop(db);
        self.check_workspace(&p)?;
        Ok(p)
    }
    pub(crate) fn check_workspace(&self, p: &Principal) -> Result<()> {
        self.db()?.check_workspace(&p.user_id)
    }
    fn cookie(&self, name: &str, value: &str, http_only: bool, age: i64) -> String {
        format!(
            "{name}={value}; Path=/; SameSite=Strict; Max-Age={age}{}{}",
            if http_only { "; HttpOnly" } else { "" },
            if self.origin.starts_with("https:") {
                "; Secure"
            } else {
                ""
            }
        )
    }
}
pub fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|s| {
            let (k, v) = s.trim().split_once('=')?;
            (k == name).then(|| v.to_string())
        })
}
fn cookies(mut response: Response, values: Vec<String>) -> Result<Response> {
    for c in values {
        response.headers_mut().append(
            header::SET_COOKIE,
            HeaderValue::from_str(&c).map_err(|e| Error::Internal(e.into()))?,
        );
    }
    Ok(response)
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/a2a/{owner}/{agent}", post(crate::a2a::rpc))
        .route(
            "/a2a/{owner}/{agent}/.well-known/agent-card.json",
            get(crate::a2a::agent_card),
        )
        .route(
            "/health",
            get(|| async { Json(json!({"status":"ok","version":env!("CARGO_PKG_VERSION")})) }),
        )
        .route("/api/capabilities", get(capabilities))
        .route("/setup/slack", get(slack_app_setup))
        .route("/setup/slack/manifest", get(slack_app_manifest))
        .route(
            "/install.sh",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("install/connect.sh")
                        .replace("@VERSION@", env!("CARGO_PKG_VERSION")),
                )
            }),
        )
        .route(
            "/install.ps1",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("install/connect.ps1")
                        .replace("@VERSION@", env!("CARGO_PKG_VERSION")),
                )
            }),
        )
        .route(
            "/uninstall.sh",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("install/uninstall.sh")
                        .replace("@VERSION@", env!("CARGO_PKG_VERSION")),
                )
            }),
        )
        .route(
            "/uninstall.ps1",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                    include_str!("install/uninstall.ps1")
                        .replace("@VERSION@", env!("CARGO_PKG_VERSION")),
                )
            }),
        )
        .route(
            "/webhooks/agentphone",
            post(crate::channel_replies::agentphone),
        )
        .route("/webhooks/slack", post(crate::channel_replies::slack))
        .route("/webhooks/slack/events", post(crate::slack_agent::events))
        .route(
            "/auth/start",
            post(start_login).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/auth/verify",
            post(verify_login).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route("/auth/logout", post(logout))
        .route(
            "/device/start",
            post(start_device).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/device/poll",
            post(poll_device).layer(DefaultBodyLimit::max(16 * 1024)),
        )
        .route(
            "/api/device/{code}",
            get(device_info).post(authorize_device),
        )
        .route("/api/me", get(me))
        .route("/api/channels", get(channels))
        .route("/api/channels/phone", post(enable_phone))
        .route("/api/channels/phone/start", post(start_phone))
        .route("/api/channels/phone/verify", post(verify_phone))
        .route("/api/account", get(account))
        .route("/api/auth/google", get(google_start))
        .route("/api/auth/google/callback", get(google_callback))
        .route("/api/channels/email/google", post(use_google_email))
        .route("/api/channels/email/start", post(start_email))
        .route("/api/channels/email/verify", post(verify_email))
        .route("/api/channels/slack/start", post(start_slack))
        .route("/api/channels/slack/finish", post(finish_slack))
        .route("/oauth/slack/callback", get(slack_callback))
        .route("/api/channels/{id}", delete(disable_channel))
        .route("/api/agents", get(agents).post(add_agent))
        .route("/api/agents/devin", post(connect_devin))
        .route("/api/agents/{id}", delete(revoke_agent))
        .route("/api/agents/{id}/default", post(default_agent))
        .route("/api/connections", get(connections))
        .route("/api/connections/local", post(start_local_connection))
        .route(
            "/api/connections/local/{code}",
            get(local_connection_status),
        )
        .route("/api/connection/ready", post(local_connection_ready))
        .route("/api/connection/identity", post(local_agent_identity))
        .route("/api/connections/{id}", delete(revoke_connection))
        .route("/api/contacts", get(contacts).post(add_contact))
        .route("/api/people", get(people))
        .route("/api/people/{id}/contact", post(contact_person))
        .route("/api/contacts/{id}/accept", post(accept_contact))
        .route("/api/contacts/{id}/block", post(block_contact))
        .route("/api/invite/rotate", post(rotate_invite))
        .route("/api/invite/{code}", get(invite))
        .route("/api/resolve", post(resolve))
        .route("/api/tasks", get(tasks).post(new_task))
        .route("/api/tasks/lookup", post(lookup_request))
        .route("/api/tasks/{id}", get(task))
        .route("/api/tasks/{id}/draft", post(prepare))
        .route("/api/tasks/{id}/decision", post(decide))
        .route("/api/tasks/{id}/cancel", post(cancel))
        .route("/api/tasks/{id}/received", post(received))
        .route("/api/tasks/{id}/heartbeat", post(heartbeat))
        .route("/api/tasks/{id}/finish", post(finish))
        .route("/api/tasks/{id}/result", post(stage_result))
        .route("/api/tasks/{id}/permissions", post(request_permission))
        .route(
            "/api/tasks/{id}/permissions/{permission}/status",
            post(permission_status),
        )
        .route(
            "/api/tasks/{id}/permissions/{permission}/decision",
            post(decide_permission),
        )
        .route(
            "/api/tasks/{id}/files",
            post(upload).layer(DefaultBodyLimit::disable()),
        )
        .route("/api/files/{id}", get(download))
        .route("/api/files/{id}/received", post(download_received))
        .route("/api/work/claim", post(claim))
        .route("/api/tasks/{id}/read/slack", post(crate::slack_reads::read))
        .route("/api/work/prepare", post(claim_preparation))
        .route("/api/events", get(events))
        .route(
            "/fonts/geist.woff2",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/woff2")],
                    include_bytes!("web/fonts/geist.woff2").as_slice(),
                )
            }),
        )
        .route(
            "/fonts/geist-mono.woff2",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "font/woff2")],
                    include_bytes!("web/fonts/geist-mono.woff2").as_slice(),
                )
            }),
        )
        .route(
            "/logo.png",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    include_bytes!("../assets/litecom-logo.png").as_slice(),
                )
            }),
        )
        .route(
            "/app.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("web/app.js"),
                )
            }),
        )
        .route(
            "/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("web/style.css"),
                )
            }),
        )
        .fallback(get(|| async {
            axum::response::Html(include_str!("web/index.html"))
        }))
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            authenticate_api,
        ))
        .layer(middleware::from_fn(security_headers))
        .with_state(state)
}
async fn authenticate_api(
    State(s): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response> {
    let path = request.uri().path();
    if (path.starts_with("/api/")
        && !matches!(
            path,
            "/api/capabilities" | "/api/auth/google" | "/api/auth/google/callback"
        ))
        || path.starts_with("/a2a/")
    {
        s.principal(
            request.headers(),
            !matches!(
                *request.method(),
                axum::http::Method::GET | axum::http::Method::HEAD
            ),
        )?;
    }
    Ok(next.run(request).await)
}
async fn security_headers(request: axum::extract::Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let h = response.headers_mut();
    for (k, v) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "no-referrer"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' blob:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
        ),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        h.insert(k, HeaderValue::from_static(v));
    }
    response
}
async fn slack_app_setup(
    State(s): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> axum::response::Redirect {
    let agent = s.providers.slack_agent.is_some() || query.get("agent").is_some_and(|v| v == "1");
    let manifest = slack_manifest(&s.origin, agent);
    let mut url = url::Url::parse("https://api.slack.com/apps").expect("constant Slack URL");
    url.query_pairs_mut()
        .append_pair("new_app", "1")
        .append_pair("manifest_json", &manifest.to_string());
    axum::response::Redirect::to(url.as_str())
}
async fn slack_app_manifest(
    State(s): State<AppState>,
    Query(query): Query<HashMap<String, String>>,
) -> Json<Value> {
    Json(slack_manifest(
        &s.origin,
        s.providers.slack_agent.is_some() || query.get("agent").is_some_and(|v| v == "1"),
    ))
}
fn slack_manifest(origin: &str, agent: bool) -> Value {
    let mut manifest = json!({
        "display_information":{"name":"Litecom","description":"Ask someone's agent. They approve. It gets done."},
        "features":{"bot_user":{"display_name":"Litecom","always_online":false}},
        "oauth_config":{"redirect_urls":[format!("{origin}/oauth/slack/callback")],"scopes":{"bot":crate::slack::scopes(agent)}},
        "settings":{"interactivity":{"is_enabled":true,"request_url":format!("{origin}/webhooks/slack")},"socket_mode_enabled":false}
    });
    if agent {
        manifest["settings"]["event_subscriptions"] = json!({"request_url":format!("{origin}/webhooks/slack/events"),"bot_events":["app_mention"]});
    }
    manifest
}
async fn capabilities(State(s): State<AppState>) -> Json<Value> {
    Json(
        json!({"google":s.providers.google.is_some(),"phone_signin":s.providers.phone_available() && !s.providers.google.as_ref().is_some_and(|g|g.restricts_workspace()),"phone":s.providers.phone_available(),"email":s.providers.email_available(),"slack":s.providers.slack.is_some(),"slack_agent":s.providers.slack_agent.is_some() && !s.providers.dev,"development":s.providers.dev,"direct_approvals":{"phone":s.providers.phone_provider().is_some(),"slack":s.providers.slack_signing_secret().is_some(),"email":s.providers.email_replies_available()}}),
    )
}
#[derive(serde::Serialize, Deserialize)]
struct GoogleLoginState {
    pending: crate::google::PendingGoogle,
    next: String,
}
#[derive(Deserialize)]
struct GoogleStart {
    next: Option<String>,
}
fn login_return(value: &str) -> String {
    let base = url::Url::parse("https://litecom.invalid").expect("constant URL");
    let Ok(url) = base.join(value) else {
        return "/".into();
    };
    if url.origin() != base.origin() {
        return "/".into();
    }
    let path = url.path();
    if !matches!(
        path,
        "/" | "/workspace" | "/contacts" | "/agents" | "/settings"
    ) && !["/connect/", "/request/", "/invite/"]
        .iter()
        .any(|prefix| path.starts_with(prefix))
    {
        return "/".into();
    }
    let channel = url.query_pairs().find(|(key, value)| {
        key == "approve" && matches!(value.as_ref(), "phone" | "email" | "slack")
    });
    match channel {
        Some((_, value)) => format!("{path}?approve={value}"),
        None => path.into(),
    }
}
async fn google_start(State(s): State<AppState>, Query(v): Query<GoogleStart>) -> Result<Response> {
    s.limit("google-login", 30)?;
    let google = s.providers.google.as_ref().ok_or_else(|| {
        Error::Unavailable("Google sign-in is not configured on this server.".into())
    })?;
    let (state, browser) = (secret(), secret());
    let (url, pending) = google.begin(&s.origin, &state);
    s.db()?.save_login_state(
        &state,
        &browser,
        &GoogleLoginState {
            pending,
            next: login_return(v.next.as_deref().unwrap_or("/")),
        },
    )?;
    cookies(
        axum::response::Redirect::to(url.as_str()).into_response(),
        vec![
            s.cookie("litecom_google", &browser, true, 600)
                .replace("SameSite=Strict", "SameSite=Lax"),
        ],
    )
}
#[derive(Deserialize)]
struct GoogleCallback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn google_callback(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(v): Query<GoogleCallback>,
) -> Result<Response> {
    let google = s.providers.google.as_ref().ok_or_else(|| {
        Error::Unavailable("Google sign-in is not configured on this server.".into())
    })?;
    let login: GoogleLoginState = match s
        .db()?
        .take_login_state(&v.state, &cookie(&h, "litecom_google").unwrap_or_default())
    {
        Ok(login) => login,
        Err(_) => {
            return cookies(
                axum::response::Redirect::to("/?signin=expired").into_response(),
                vec![s.cookie("litecom_google", "", true, 0)],
            );
        }
    };
    if v.error.is_some() || v.code.as_deref().is_none_or(str::is_empty) {
        return cookies(
            axum::response::Redirect::to("/?signin=cancelled").into_response(),
            vec![s.cookie("litecom_google", "", true, 0)],
        );
    }
    let identity = match google
        .exchange(
            s.providers.http(),
            &s.origin,
            v.code.unwrap(),
            &login.pending,
        )
        .await
    {
        Ok(identity) => identity,
        Err(_) => {
            return cookies(
                axum::response::Redirect::to("/?signin=failed").into_response(),
                vec![s.cookie("litecom_google", "", true, 0)],
            );
        }
    };
    let session = {
        let db = s.db()?;
        db.session_for(db.google_profile(&identity)?)?
    };
    cookies(
        axum::response::Redirect::to(&login.next).into_response(),
        vec![
            s.cookie("litecom_google", "", true, 0),
            s.cookie("litecom_session", &session.token, true, 30 * 86400),
            s.cookie("litecom_csrf", &session.csrf, false, 30 * 86400),
        ],
    )
}
async fn account(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>> {
    let p = s.principal(&h, false)?;
    p.human()?;
    let db = s.db()?;
    Ok(Json(
        json!({"google_email":db.google_identity(&p)?.map(|v|v.email),"has_phone":db.has_phone(&p)?}),
    ))
}
async fn use_google_email(State(s): State<AppState>, h: HeaderMap) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    if !s.providers.email_available() {
        return Err(Error::Unavailable(
            "Email is not enabled on this server.".into(),
        ));
    }
    s.db()?.connect_google_email(&p)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct LoginInput {
    name: String,
    phone: String,
}
async fn start_login(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<LoginInput>,
) -> Result<Response> {
    s.origin(&h)?;
    s.limit("login-global", 30)?;
    if s.providers
        .google
        .as_ref()
        .is_some_and(|g| g.restricts_workspace())
    {
        return Err(Error::Forbidden(
            "Sign in with Google to join this workspace.".into(),
        ));
    }
    if !s.providers.phone_available() {
        return Err(Error::Unavailable(
            "Phone delivery is not enabled on this server.".into(),
        ));
    }
    let nonce = secret();
    let (challenge, code) = s.db()?.begin_login(&v.name, &v.phone, &nonce)?;
    s.providers
        .sms(
            &v.phone,
            &format!(
                "Your Litecom sign-in code is {code}. You requested this text. It expires in 10 minutes. Never share this code. Reply STOP to unsubscribe."
            ),
        )
        .await?;
    cookies(
        Json(challenge).into_response(),
        vec![s.cookie("litecom_login", &nonce, true, 600)],
    )
}
#[derive(Deserialize)]
struct VerifyInput {
    id: String,
    code: String,
}
async fn verify_login(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<VerifyInput>,
) -> Result<Response> {
    s.origin(&h)?;
    s.limit("verify-global", 120)?;
    let session = s.db()?.verify_login(
        &v.id,
        &v.code,
        &cookie(&h, "litecom_login").unwrap_or_default(),
    )?;
    cookies(
        Json(json!({"profile":session.profile})).into_response(),
        vec![
            s.cookie("litecom_session", &session.token, true, 30 * 86400),
            s.cookie("litecom_csrf", &session.csrf, false, 30 * 86400),
            s.cookie("litecom_login", "", true, 0),
        ],
    )
}
async fn logout(State(s): State<AppState>, h: HeaderMap) -> Result<Response> {
    let p = s.principal(&h, true)?;
    s.db()?.logout(&p)?;
    cookies(
        StatusCode::NO_CONTENT.into_response(),
        vec![
            s.cookie("litecom_session", "", true, 0),
            s.cookie("litecom_csrf", "", false, 0),
        ],
    )
}
#[derive(Deserialize)]
struct DeviceInput {
    label: String,
}
async fn start_device(
    State(s): State<AppState>,
    Json(v): Json<DeviceInput>,
) -> Result<Json<Value>> {
    s.limit("device-global", 60)?;
    Ok(Json(serde_json::to_value(s.db()?.begin_device(&v.label)?)?))
}
#[derive(Deserialize)]
struct DevicePoll {
    device_code: String,
}
async fn poll_device(State(s): State<AppState>, Json(v): Json<DevicePoll>) -> Result<Json<Value>> {
    s.limit(&format!("poll:{}", crate::crypto::hash(&v.device_code)), 30)?;
    Ok(Json(serde_json::to_value(
        s.db()?.poll_device(&v.device_code)?,
    )?))
}
async fn device_info(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<Value>> {
    s.principal(&h, false)?.human()?;
    Ok(Json(json!({"label":s.db()?.device_label(&code)?})))
}
#[derive(Deserialize)]
struct AgentChoice {
    agent_id: String,
}
async fn start_local_connection(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<AgentChoice>,
) -> Result<Json<crate::auth::DeviceCode>> {
    let p = s.principal(&h, true)?;
    s.limit(&format!("connect:{}", p.user_id), 10)?;
    Ok(Json(s.db()?.begin_local_connection(&p, &v.agent_id)?))
}
async fn local_connection_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<Value>> {
    let p = s.principal(&h, false)?;
    Ok(Json(
        json!({"status":s.db()?.local_connection_status(&p, &code)?}),
    ))
}
async fn local_connection_ready(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>> {
    let p = s.principal(&h, true)?;
    s.db()?.local_connection_ready(&p)?;
    Ok(Json(
        json!({"listening":s.db()?.local_connection_listening(&p)?}),
    ))
}
async fn local_agent_identity(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(identity): Json<LocalAgentIdentity>,
) -> Result<Json<Value>> {
    let p = s.principal(&h, true)?;
    s.db()?.identify_local_agent(&p, &identity)?;
    Ok(Json(json!({"updated":true})))
}
async fn authorize_device(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(code): Path<String>,
    Json(v): Json<AgentChoice>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.authorize_device(&p, &code, &v.agent_id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn me(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Profile>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.profile(&p)?))
}
#[derive(Deserialize)]
struct OwnerQuery {
    owner_id: Option<String>,
}
async fn agents(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<OwnerQuery>,
) -> Result<Json<Vec<Agent>>> {
    let p = s.principal(&h, false)?;
    Ok(Json(
        s.db()?
            .agents(&p, q.owner_id.as_deref().unwrap_or(&p.user_id))?,
    ))
}
#[derive(Deserialize)]
struct AgentInput {
    name: String,
    kind: AgentKind,
    #[serde(default)]
    make_default: bool,
}
async fn add_agent(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<AgentInput>,
) -> Result<Json<Agent>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.register_agent(
        &p,
        &v.name,
        v.kind,
        v.make_default,
    )?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DevinInput {
    name: String,
    token: String,
    org_id: Option<String>,
}
async fn connect_devin(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<DevinInput>,
) -> Result<Json<Agent>> {
    let p = s.principal(&h, true)?;
    p.human()?;
    s.limit(&format!("connect-devin:{}", p.user_id), 10)?;
    let connection = crate::devin::Devin::new()?
        .connect(v.token.trim(), v.org_id.as_deref().map(str::trim))
        .await?;
    // The session may have been revoked while the provider check was in flight.
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.connect_devin(&p, &v.name, connection)?))
}
async fn default_agent(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.set_default(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn revoke_agent(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.revoke_agent(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn connections(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>> {
    let p = s.principal(&h, false)?;
    Ok(Json(serde_json::to_value(s.db()?.connections(&p)?)?))
}
async fn revoke_connection(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.revoke_connection(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn contacts(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Vec<Contact>>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.contacts(&p)?))
}
#[derive(Deserialize)]
struct InviteInput {
    invite: String,
}
async fn add_contact(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<InviteInput>,
) -> Result<Json<Contact>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.request_contact(&p, &v.invite)?))
}
async fn accept_contact(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.accept_contact(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn block_contact(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.block_contact(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn rotate_invite(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Profile>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.rotate_invite(&p)?))
}
async fn invite(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(code): Path<String>,
) -> Result<Json<Value>> {
    s.principal(&h, false)?;
    let (id, name) = s.db()?.invite_profile(&code)?;
    Ok(Json(json!({"id":id,"name":name})))
}
#[derive(Deserialize)]
struct ResolveInput {
    contact: String,
    agent: Option<String>,
}
async fn resolve(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<ResolveInput>,
) -> Result<Json<Value>> {
    let p = s.principal(&h, true)?;
    let (owner_id, agent_id) = s.db()?.resolve(&p, &v.contact, v.agent.as_deref())?;
    Ok(Json(json!({"owner_id":owner_id,"agent_id":agent_id})))
}
#[derive(Deserialize)]
struct PageQuery {
    before_time: Option<i64>,
    before_id: Option<String>,
}
async fn tasks(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<PageQuery>,
) -> Result<Json<Vec<TaskView>>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.tasks(&p, q.before_time.zip(q.before_id))?))
}
async fn new_task(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<NewTask>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.create_task(&p, v)?))
}
async fn task(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.get_task(&p, &id)?))
}
#[derive(Deserialize)]
struct PrepareInput {
    lease: Option<String>,
    draft: DraftInput,
}
async fn prepare(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<PrepareInput>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.prepare(
        &p,
        &id,
        v.lease.as_deref(),
        v.draft,
    )?))
}
async fn stage_result(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<PrepareInput>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.stage_result(
        &p,
        &id,
        v.lease.as_deref().unwrap_or(""),
        v.draft,
    )?))
}
#[derive(Deserialize)]
struct DecisionInput {
    version: u32,
    digest: String,
    decision: Decision,
    #[serde(default)]
    read_only: bool,
}
async fn decide(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<DecisionInput>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.decide_access(
        &p,
        &id,
        v.version,
        &v.digest,
        v.decision,
        v.read_only,
    )?))
}
async fn cancel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.cancel(&p, &id)?))
}
async fn received(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.acknowledge(&p, &id)?))
}
#[derive(Deserialize)]
struct LeaseInput {
    lease: String,
}
#[derive(Deserialize)]
struct PermissionInput {
    lease: String,
    permission: ToolPermissionInput,
}
#[derive(Deserialize)]
struct PermissionDecision {
    digest: String,
    decision: Decision,
}
async fn request_permission(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<PermissionInput>,
) -> Result<Json<ToolPermission>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.request_permission(
        &p,
        &id,
        &v.lease,
        v.permission,
    )?))
}
async fn permission_status(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, permission)): Path<(String, String)>,
    Json(v): Json<LeaseInput>,
) -> Result<Json<ToolPermission>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.permission_status(
        &p,
        &id,
        &v.lease,
        &permission,
    )?))
}
async fn decide_permission(
    State(s): State<AppState>,
    h: HeaderMap,
    Path((id, permission)): Path<(String, String)>,
    Json(v): Json<PermissionDecision>,
) -> Result<Json<ToolPermission>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.decide_permission(
        &p,
        &id,
        &permission,
        &v.digest,
        v.decision,
    )?))
}
async fn heartbeat(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<LeaseInput>,
) -> Result<Json<Status>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.heartbeat(&p, &id, &v.lease)?))
}
#[derive(Deserialize)]
struct FinishInput {
    lease: String,
    receipt: Receipt,
}
async fn finish(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<FinishInput>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.finish(&p, &id, &v.lease, v.receipt)?))
}
#[derive(Deserialize)]
struct UploadInput {
    lease: Option<String>,
    name: String,
    media_type: String,
    data: String,
}
async fn upload(
    State(s): State<AppState>,
    Path(id): Path<String>,
    request: axum::extract::Request,
) -> Result<Json<Artifact>> {
    let headers = request.headers().clone();
    let p = s.principal(&headers, true)?;
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("multipart/form-data"))
    {
        let mut parts = Multipart::from_request(request, &s)
            .await
            .map_err(|_| Error::Invalid("Provide a multipart file upload.".into()))?;
        let mut metadata = parts
            .next_field()
            .await
            .map_err(|_| Error::Invalid("The upload was interrupted.".into()))?
            .ok_or_else(|| Error::Invalid("Provide the file metadata first.".into()))?;
        validate(
            metadata.name() == Some("metadata"),
            "Provide the file metadata first.",
        )?;
        #[derive(Deserialize)]
        struct Metadata {
            lease: Option<String>,
            name: String,
            media_type: String,
        }
        let mut metadata_bytes = Vec::new();
        while let Some(chunk) = metadata
            .chunk()
            .await
            .map_err(|_| Error::Invalid("Invalid file metadata.".into()))?
        {
            validate(
                metadata_bytes.len().saturating_add(chunk.len()) <= 64 * 1024,
                "File metadata is too large.",
            )?;
            metadata_bytes.extend_from_slice(&chunk);
        }
        drop(metadata);
        let metadata: Metadata = serde_json::from_slice(&metadata_bytes)
            .map_err(|_| Error::Invalid("Provide valid JSON file metadata.".into()))?;
        let mut upload = s.db()?.begin_upload(
            &p,
            &id,
            metadata.lease.as_deref(),
            &metadata.name,
            &metadata.media_type,
        )?;
        let mut file = parts
            .next_field()
            .await
            .map_err(|_| Error::Invalid("The upload was interrupted.".into()))?
            .ok_or_else(|| Error::Invalid("Provide a file.".into()))?;
        validate(file.name() == Some("file"), "Provide a file.")?;
        while let Some(chunk) = file
            .chunk()
            .await
            .map_err(|_| Error::Invalid("The upload was interrupted.".into()))?
        {
            upload.write(&chunk)?;
        }
        drop(file);
        validate(
            parts
                .next_field()
                .await
                .map_err(|_| Error::Invalid("The upload was interrupted.".into()))?
                .is_none(),
            "Send one file per upload.",
        )?;
        let current = s.principal(&headers, true)?;
        return Ok(Json(s.db()?.finish_upload(
            &current,
            metadata.lease.as_deref(),
            upload,
        )?));
    }
    // Compatibility for existing API callers; native and browser clients stream multipart.
    let bytes = axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| Error::Invalid("Use multipart upload for files larger than 1 MiB.".into()))?;
    let v: UploadInput = serde_json::from_slice(&bytes)
        .map_err(|_| Error::Invalid("Provide a valid file upload.".into()))?;
    let data = STANDARD
        .decode(v.data)
        .map_err(|_| Error::Invalid("The file encoding is invalid.".into()))?;
    let current = s.principal(&headers, true)?;
    Ok(Json(s.db()?.upload(
        &current,
        &id,
        v.lease.as_deref(),
        &v.name,
        &v.media_type,
        &data,
    )?))
}
async fn download(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Response> {
    let p = s.principal(&h, false)?;
    let file = s.db()?.download_metadata(&p, &id)?;
    let expected = file.clone();
    let stream = futures_util::stream::try_unfold(
        (s, h, id, 0u64, 0u64, Sha256::new()),
        move |(s, headers, id, sequence, size, mut digest)| {
            let expected = expected.clone();
            async move {
                let p = s.principal(&headers, false)?;
                let chunk = s.db()?.download_chunk(&p, &id, sequence)?;
                let Some(chunk) = chunk else {
                    require(
                        size == expected.size && hex::encode(digest.finalize()) == expected.sha256,
                        "The file failed its integrity check.",
                    )?;
                    return Ok::<_, Error>(None);
                };
                let size = size
                    .checked_add(chunk.len() as u64)
                    .ok_or_else(|| Error::Invalid("The file size is invalid.".into()))?;
                require(
                    size <= expected.size,
                    "The file failed its integrity check.",
                )?;
                digest.update(&chunk);
                // Check the complete digest before releasing the final chunk.
                if size == expected.size {
                    require(
                        hex::encode(digest.clone().finalize()) == expected.sha256,
                        "The file failed its integrity check.",
                    )?;
                }
                Ok(Some((
                    Bytes::from(chunk),
                    (s, headers, id, sequence + 1, size, digest),
                )))
            }
        },
    );
    let download_name = crate::client::download_name(&file);
    let safe_name = download_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "._- ".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    let mut response = Body::from_stream(stream).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_LENGTH, HeaderValue::from(file.size));
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{safe_name}\"; filename*=UTF-8''{}",
            url::form_urlencoded::byte_serialize(download_name.as_bytes())
                .collect::<String>()
                // Extended header values use percent-encoded spaces and stars,
                // unlike application/x-www-form-urlencoded bodies.
                .replace('+', "%20")
                .replace('*', "%2A")
        ))
        .map_err(|e| Error::Internal(e.into()))?,
    );
    headers.insert(
        "x-content-sha256",
        HeaderValue::from_str(&file.sha256).map_err(|e| Error::Internal(e.into()))?,
    );
    Ok(response)
}
#[derive(Deserialize)]
struct WorkerCapabilities {
    #[serde(default)]
    read_only: bool,
}
async fn claim(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<WorkerCapabilities>,
) -> Result<Json<Option<WorkItem>>> {
    let p = s.principal(&h, true)?;
    let agent = p.agent_id.as_deref().ok_or(Error::Unauthorized)?;
    Ok(Json(if q.read_only {
        s.db()?.claim_read_capable(&p, agent)?
    } else {
        s.db()?.claim(&p, agent)?
    }))
}
async fn claim_preparation(
    State(s): State<AppState>,
    h: HeaderMap,
) -> Result<Json<Option<WorkItem>>> {
    let p = s.principal(&h, true)?;
    let agent = p.agent_id.as_deref().ok_or(Error::Unauthorized)?;
    Ok(Json(s.db()?.claim_preparation(&p, agent)?))
}
#[derive(Deserialize)]
struct EventQuery {
    #[serde(default)]
    after: i64,
}
async fn events(
    State(s): State<AppState>,
    h: HeaderMap,
    Query(q): Query<EventQuery>,
) -> Result<Json<Vec<Event>>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.events(&p, q.after)?))
}

async fn channels(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>> {
    let p = s.principal(&h, false)?;
    Ok(Json(serde_json::to_value(s.db()?.channels(&p)?)?))
}
async fn enable_phone(State(s): State<AppState>, h: HeaderMap) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    if !s.providers.phone_available() {
        return Err(Error::Unavailable(
            "Phone is not enabled on this server.".into(),
        ));
    }
    s.db()?.enable_phone(&p)?;
    Ok(StatusCode::NO_CONTENT)
}
#[derive(Deserialize)]
struct PhoneInput {
    phone: String,
}
async fn start_phone(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<PhoneInput>,
) -> Result<Json<crate::auth::OtpChallenge>> {
    let p = s.principal(&h, true)?;
    if !s.providers.phone_available() {
        return Err(Error::Unavailable(
            "Phone is not enabled on this server.".into(),
        ));
    }
    s.limit(&format!("phone:{}", p.user_id), 5)?;
    let (challenge, code) = s.db()?.begin_phone(&p, &v.phone)?;
    s.providers.sms(&v.phone, &format!("Your Litecom phone code is {code}. You requested this text. It expires in 10 minutes. Never share this code. Reply STOP to unsubscribe.")).await?;
    Ok(Json(challenge))
}
async fn verify_phone(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<EmailVerification>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.verify_phone(&p, &v.id, &v.code)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn people(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Vec<Contact>>> {
    let p = s.principal(&h, false)?;
    Ok(Json(s.db()?.people(&p)?))
}
async fn contact_person(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<Json<Contact>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.request_contact_to(&p, &id)?))
}
#[derive(Deserialize)]
struct EmailInput {
    email: String,
}
async fn start_email(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<EmailInput>,
) -> Result<Json<crate::auth::OtpChallenge>> {
    let p = s.principal(&h, true)?;
    if !s.providers.email_available() {
        return Err(Error::Unavailable(
            "Email delivery is not enabled on this server.".into(),
        ));
    }
    s.limit(&format!("email:{}", p.user_id), 5)?;
    let email = crate::channels::email_address(&v.email)?;
    let (challenge, code) = s.db()?.begin_email(&p, &email)?;
    s.providers.email(&email,"Connect email to Litecom",&format!("Your Litecom email code is {code}. It expires in 10 minutes. If you did not request this, ignore it.")).await?;
    Ok(Json(challenge))
}
#[derive(Deserialize)]
struct EmailVerification {
    id: String,
    code: String,
}
async fn verify_email(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<EmailVerification>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.verify_email(&p, &v.id, &v.code)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn disable_channel(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.disable_channel(&p, &id)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn start_slack(State(s): State<AppState>, h: HeaderMap) -> Result<Json<Value>> {
    let p = s.principal(&h, true)?;
    let slack = s
        .providers
        .slack
        .as_ref()
        .ok_or_else(|| Error::Unavailable("Slack is not enabled on this server.".into()))?;
    let start = s.db()?.begin_slack(&p)?;
    Ok(Json(json!({"url":slack.authorize_url(&s.origin,&start)})))
}
#[derive(Deserialize)]
struct SlackCallback {
    state: String,
    code: Option<String>,
    error: Option<String>,
}
async fn slack_callback(
    State(s): State<AppState>,
    Query(v): Query<SlackCallback>,
) -> Result<axum::response::Redirect> {
    s.db()?.take_slack_link(&v.state)?;
    if v.error.is_some() || v.code.is_none() {
        return Ok(axum::response::Redirect::to(
            "/slack/connected?error=cancelled",
        ));
    }
    let slack = s
        .providers
        .slack
        .as_ref()
        .ok_or_else(|| Error::Unavailable("Slack is not enabled on this server.".into()))?;
    match slack
        .exchange(
            s.providers.http(),
            &s.origin,
            v.code.as_deref().unwrap_or(""),
        )
        .await
    {
        Ok(destination) => {
            s.db()?.stage_slack_link(&v.state, destination)?;
            let encoded: String =
                url::form_urlencoded::byte_serialize(v.state.as_bytes()).collect();
            Ok(axum::response::Redirect::to(&format!(
                "/slack/connected?state={encoded}"
            )))
        }
        Err(_) => Ok(axum::response::Redirect::to(
            "/slack/connected?error=failed",
        )),
    }
}
#[derive(Deserialize)]
struct SlackFinish {
    state: String,
}
async fn finish_slack(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<SlackFinish>,
) -> Result<StatusCode> {
    let p = s.principal(&h, true)?;
    s.db()?.finish_slack_link(&p, &v.state)?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct RequestLookup {
    key: String,
}
async fn lookup_request(
    State(s): State<AppState>,
    h: HeaderMap,
    Json(v): Json<RequestLookup>,
) -> Result<Json<Option<TaskView>>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.lookup_request(&p, &v.key)?))
}

#[derive(Deserialize)]
struct DownloadCheck {
    sha256: String,
}
async fn download_received(
    State(s): State<AppState>,
    h: HeaderMap,
    Path(id): Path<String>,
    Json(v): Json<DownloadCheck>,
) -> Result<Json<TaskView>> {
    let p = s.principal(&h, true)?;
    Ok(Json(s.db()?.acknowledge_download(&p, &id, &v.sha256)?))
}
