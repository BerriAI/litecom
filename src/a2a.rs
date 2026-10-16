//! A2A v1 JSON-RPC transport over the same account and task authority as MCP.
use crate::{
    api::AppState,
    error::{Error, Result, require, validate},
    model::{Agent, NewTask, Principal, Status, TaskView},
};
use a2a_protocol_types::{
    agent_card::AgentCard,
    artifact::Artifact as A2aArtifact,
    jsonrpc::JsonRpcRequest,
    message::{MessageRole, Part, PartContent},
    params::{CancelTaskParams, MessageSendParams, TaskQueryParams},
    task::{Task, TaskState, TaskStatus},
};
use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde_json::{Value, json};

pub fn project(view: TaskView, origin: &str) -> Result<Task> {
    let state = match view.status {
        Status::Queued => TaskState::Submitted,
        Status::Preparing
        | Status::AwaitingApproval
        | Status::Approved
        | Status::Executing
        | Status::CancelRequested => TaskState::Working,
        Status::Completed => TaskState::Completed,
        Status::Declined => TaskState::Rejected,
        Status::Cancelled => TaskState::Canceled,
        Status::Failed | Status::Expired => TaskState::Failed,
    };
    let mut artifacts = vec![];
    if let Some(response) = view.response {
        if !response.text.is_empty() {
            artifacts.push(A2aArtifact::new(
                format!("{}-text", view.id),
                vec![Part::text(response.text)],
            ));
        }
        for file in response.attachments {
            let mut part = Part::url(format!("{origin}/api/files/{}", file.id));
            part.filename = Some(file.name.clone());
            part.media_type = Some(file.media_type);
            part.metadata = Some(
                json!({"sha256":file.sha256,"bytes":file.size,"authentication":"Use the same Litecom bearer credential."}),
            );
            let mut artifact = A2aArtifact::new(file.id, vec![part]);
            artifact.name = Some(file.name);
            artifacts.push(artifact);
        }
    }
    Ok(Task {
        id: view.id.clone().into(),
        context_id: view.id.clone().into(),
        status: TaskStatus {
            state,
            message: None,
            timestamp: None,
        },
        history: None,
        artifacts: (!artifacts.is_empty()).then_some(artifacts),
        metadata: Some(
            json!({"litecom":{"phase":view.status.as_str(),"receivedAt":view.received_at,"downloads":view.downloads,"revokedAt":view.revoked_at,"expiresAt":view.expires_at,"reviewUrl":format!("{origin}/request/{}",view.id)}}),
        ),
    })
}
fn destination(state: &AppState, p: &Principal, owner: &str, agent: &str) -> Result<Agent> {
    state
        .db()?
        .agents(p, owner)?
        .into_iter()
        .find(|a| a.id == agent)
        .ok_or(Error::NotFound)
}
fn card(agent: Agent, origin: &str) -> Result<AgentCard> {
    Ok(serde_json::from_value(json!({
        "name":agent.name,"description":"Request a response, files, or an approved action. The owner reviews what is shared. Poll GetTask for updates.","version":env!("CARGO_PKG_VERSION"),
        "supportedInterfaces":[{"url":format!("{origin}/a2a/{}/{}",agent.owner_id,agent.id),"protocolBinding":"JSONRPC","protocolVersion":"1.0"}],
        "capabilities":{"streaming":false,"pushNotifications":false,"extendedAgentCard":false},
        "defaultInputModes":["text/plain"],"defaultOutputModes":["text/plain","application/octet-stream"],
        "skills":[{"id":"request","name":"Ask an agent","description":"Prepare a response or action for owner approval.","tags":["sharing","approval"],"examples":["Send me the launch notes."]}],
        "securitySchemes":{"litecom":{"httpAuthSecurityScheme":{"scheme":"bearer","bearerFormat":"Litecom agent connection"}}},
        "securityRequirements":[{"schemes":{"litecom":{"list":[]}}}]
    }))?)
}
pub async fn agent_card(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, agent)): Path<(String, String)>,
) -> Result<Json<AgentCard>> {
    let p = state.principal(&headers, false)?;
    Ok(Json(card(
        destination(&state, &p, &owner, &agent)?,
        &state.origin,
    )?))
}
fn failure(id: Value, code: i32, message: &str) -> Json<Value> {
    Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
}
pub async fn rpc(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((owner, agent)): Path<(String, String)>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = state.principal(&headers, true)?;
    let request: JsonRpcRequest = match serde_json::from_value(body.clone()) {
        Ok(v) => v,
        Err(_) => {
            return Ok(failure(
                body.get("id").cloned().unwrap_or(Value::Null),
                -32600,
                "Invalid JSON-RPC 2.0 request.",
            ));
        }
    };
    let id = serde_json::to_value(&request.id)?;
    let params = request.params.unwrap_or_else(|| json!({}));
    let outcome: Result<Value> = async {
        destination(&state, &p, &owner, &agent)?;
        match request.method.as_str() {
            "SendMessage" => {
                let params: MessageSendParams = serde_json::from_value(params)
                    .map_err(|_| Error::Invalid("Use A2A v1 SendMessage parameters.".into()))?;
                validate(
                    params.message.role == MessageRole::User,
                    "Send a user message.",
                )?;
                validate(
                    params.message.task_id.is_none() && params.message.context_id.is_none(),
                    "Create a new request without taskId or contextId; use GetTask for an existing request.",
                )?;
                validate(!params.message.parts.is_empty(), "Provide a message.")?;
                if let Some(config) = &params.configuration {
                    validate(
                        config.task_push_notification_config.is_none(),
                        "Push notifications are not advertised. Use GetTask.",
                    )?;
                }
                let ask_only = params.message.metadata.as_ref().and_then(|m|m.get("litecom")).and_then(|m|m.get("ask_only")).map(|value|value.as_bool().ok_or_else(||Error::Invalid("litecom.ask_only must be a boolean.".into()))).transpose()?.unwrap_or(false);
                let mut texts = vec![];
                for part in params.message.parts {
                    match part.content{PartContent::Text(text)=>texts.push(text),_=>return Err(Error::Invalid("This agent accepts text requests. Approved responses may contain files.".into()))}
                }
                let key = params.message.id.0;
                let immediate = params.configuration.as_ref()
                    .and_then(|config| config.return_immediately).unwrap_or(false);
                let mut task = state.db()?.create_task(
                    &p,
                    NewTask {
                    ask_only,
                        owner_id: owner.clone(),
                        agent_id: Some(agent.clone()),
                        request: texts.join("\n"),
                        idempotency_key: key,
                    },
                )?;
                while !immediate && !task.status.terminal() {
                    // Do not hold the database lock while another request works
                    // or the owner reviews. Recheck authority on every read so a
                    // revoked connection cannot receive a later private result.
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    let current = state.principal(&headers, true)?;
                    destination(&state, &current, &owner, &agent)?;
                    task = state.db()?.get_task(&current, &task.id)?;
                }
                // SendMessage's result is the Task/Message union wrapper in A2A v1.
                Ok(json!({"task":project(task,&state.origin)?}))
            }
            "GetTask" => {
                let params: TaskQueryParams = serde_json::from_value(params)
                    .map_err(|_| Error::Invalid("Provide the task id.".into()))?;
                let task = state.db()?.get_task(&p, &params.id)?;
                require(
                    task.owner_id == owner && task.agent_id == agent,
                    "This task belongs to another agent.",
                )?;
                Ok(serde_json::to_value(project(task, &state.origin)?)?)
            }
            "CancelTask" => {
                let params: CancelTaskParams = serde_json::from_value(params)
                    .map_err(|_| Error::Invalid("Provide the task id.".into()))?;
                let task = state.db()?.get_task(&p, &params.id)?;
                require(
                    task.owner_id == owner && task.agent_id == agent,
                    "This task belongs to another agent.",
                )?;
                if task.status == Status::Cancelled {
                    return Ok(serde_json::to_value(project(task, &state.origin)?)?);
                }
                if task.status.terminal() {
                    return Err(Error::Conflict("This task is already finished. Use Litecom's revoke control to revoke future download access.".into()));
                }
                let task = state.db()?.cancel(&p, &params.id)?;
                Ok(serde_json::to_value(project(task, &state.origin)?)?)
            }
            _ => Err(Error::Unavailable(
                "This endpoint supports SendMessage, GetTask, and CancelTask.".into(),
            )),
        }
    }.await;
    match outcome {
        Ok(value) => Ok(Json(json!({"jsonrpc":"2.0","id":id,"result":value}))),
        Err(Error::Invalid(message)) => Ok(failure(id, -32602, &message)),
        Err(Error::NotFound | Error::Forbidden(_)) => {
            Ok(failure(id, -32001, "Task or agent not found."))
        }
        Err(Error::Conflict(message)) => Ok(failure(id, -32002, &message)),
        Err(Error::Unavailable(message)) => Ok(failure(id, -32004, &message)),
        Err(error) => Err(error),
    }
}
