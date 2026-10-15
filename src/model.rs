use crate::error::{Result, require, validate};
use serde::{Deserialize, Serialize};

// These paths describe the remote agent's machine, not the relay's OS.
fn absolute_agent_path(path: &str) -> bool {
    let bytes = path.as_bytes();
    !path.contains('\0')
        && (path.starts_with('/')
            || path.starts_with("\\\\")
            || (bytes.len() >= 3
                && bytes[0].is_ascii_alphabetic()
                && bytes[1] == b':'
                && matches!(bytes[2], b'/' | b'\\')))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Human,
    Agent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Principal {
    pub user_id: String,
    pub role: Role,
    pub agent_id: Option<String>,
    pub credential_id: String,
}
impl Principal {
    pub fn human(&self) -> Result<()> {
        require(
            self.role == Role::Human,
            "Open Litecom in your browser to approve this change.",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub invite: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Local,
    Devin,
    A2a,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Agent {
    pub id: String,
    pub owner_id: String,
    pub name: String,
    pub kind: AgentKind,
    pub is_default: bool,
    pub revoked: bool,
    pub last_seen_at: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub status: String,
    pub incoming: bool,
    pub agents: Vec<Agent>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpConnection {
    pub executable: String,
    pub home: String,
    pub connection: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Command {
        title: String,
        description: String,
        argv: Vec<String>,
        cwd: String,
        acting_account: String,
    },
    Harness {
        title: String,
        description: String,
        runner: String,
        executable: String,
        #[serde(default)]
        arguments: Vec<String>,
        #[serde(default)]
        mcp: Option<McpConnection>,
        instruction: String,
        workspace: String,
        permissions: Vec<String>,
        acting_account: String,
    },
    Cloud {
        title: String,
        description: String,
        instruction: String,
        provider: AgentKind,
        acting_account: String,
        connection_revision: String,
    },
}

impl Action {
    pub fn validate(&self) -> Result<()> {
        let (title, description, account) = match self {
            Self::Command {
                title,
                description,
                argv,
                cwd,
                acting_account,
            } => {
                validate(
                    !argv.is_empty()
                        && argv.len() <= 256
                        && argv.iter().all(|v| !v.contains('\0') && v.len() <= 100_000),
                    "Provide a valid command and arguments.",
                )?;
                validate(
                    absolute_agent_path(cwd),
                    "The action's working folder must be an absolute path.",
                )?;
                (title, description, acting_account)
            }
            Self::Harness {
                title,
                description,
                instruction,
                workspace,
                permissions,
                acting_account,
                runner,
                executable,
                arguments,
                mcp,
            } => {
                validate(
                    matches!(
                        runner.as_str(),
                        "codex"
                            | "claude"
                            | "codex-acp"
                            | "claude-agent-acp"
                            | "acp"
                            | "opencode"
                            | "cursor"
                            | "pi"
                            | "openclaw"
                            | "hermes"
                    ) && absolute_agent_path(executable),
                    "Choose an installed agent executable.",
                )?;
                validate(
                    arguments.len() <= 256
                        && arguments
                            .iter()
                            .all(|a| a.len() <= 100_000 && !a.contains('\0')),
                    "Provide valid agent arguments.",
                )?;
                if let Some(mcp) = mcp {
                    validate(
                        absolute_agent_path(&mcp.executable)
                            && absolute_agent_path(&mcp.home)
                            && !mcp.connection.is_empty()
                            && mcp.connection.len() <= 64
                            && mcp
                                .connection
                                .bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'),
                        "Provide a valid Litecom MCP connection.",
                    )?;
                }
                validate(
                    !instruction.trim().is_empty() && instruction.len() <= 100_000,
                    "Describe the requested work.",
                )?;
                validate(
                    absolute_agent_path(workspace)
                        && !permissions.is_empty()
                        && permissions.len() <= 100,
                    "Provide the working folder and enforceable permissions.",
                )?;
                (title, description, acting_account)
            }
            Self::Cloud {
                title,
                description,
                instruction,
                provider,
                acting_account,
                connection_revision,
            } => {
                validate(
                    uuid::Uuid::parse_str(connection_revision).is_ok(),
                    "Connect the cloud agent before requesting a run.",
                )?;
                validate(
                    !instruction.trim().is_empty()
                        && instruction.len() <= 100_000
                        && *provider != AgentKind::Local,
                    "Provide a cloud provider and instruction.",
                )?;
                (title, description, acting_account)
            }
        };
        validate(
            !title.trim().is_empty()
                && title.len() <= 200
                && !description.trim().is_empty()
                && description.len() <= 10_000
                && !account.trim().is_empty()
                && account.len() <= 200,
            "Describe the action, its effects, and the acting account.",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub task_id: String,
    pub name: String,
    pub media_type: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct DraftInput {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    pub action: Option<Action>,
}
impl DraftInput {
    pub fn validate(&self) -> Result<()> {
        validate(
            !self.text.trim().is_empty()
                || !self.attachment_ids.is_empty()
                || self.action.is_some(),
            "Prepare a reply, file, or action first.",
        )?;
        validate(
            self.text.len() <= 2_000_000 && self.attachment_ids.len() <= 100,
            "The reply is too large.",
        )?;
        if let Some(action) = &self.action {
            action.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Draft {
    pub text: String,
    pub attachments: Vec<Artifact>,
    pub action: Option<Action>,
    pub version: u32,
    pub digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub outcome: Outcome,
    pub summary: String,
    #[serde(default)]
    pub urls: Vec<String>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Queued,
    Preparing,
    AwaitingApproval,
    Approved,
    Executing,
    CancelRequested,
    Completed,
    Declined,
    Cancelled,
    Failed,
    Expired,
}
impl Status {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Declined | Self::Cancelled | Self::Failed | Self::Expired
        )
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Preparing => "preparing",
            Self::AwaitingApproval => "awaiting_approval",
            Self::Approved => "approved",
            Self::Executing => "executing",
            Self::CancelRequested => "cancel_requested",
            Self::Completed => "completed",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Response {
    pub text: String,
    pub attachments: Vec<Artifact>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DownloadReceipt {
    pub artifact_id: String,
    pub verified_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TaskView {
    pub id: String,
    pub requester_id: String,
    pub owner_id: String,
    pub agent_id: String,
    pub status: Status,
    pub request: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub expires_at: i64,
    pub received_at: Option<i64>,
    #[serde(default)]
    pub downloads: Vec<DownloadReceipt>,
    pub revoked_at: Option<i64>,
    pub draft: Option<Draft>,
    pub response: Option<Response>,
    pub receipt: Option<Receipt>,
    pub error: Option<String>,
    /// Provider activity and links are visible only to the account owner.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<ExecutionProgress>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<ToolPermission>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolPermissionInput {
    pub id: String,
    pub title: String,
    pub details: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolPermission {
    pub id: String,
    pub title: String,
    pub details: serde_json::Value,
    pub digest: String,
    pub decision: Option<Decision>,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ExecutionProgress {
    pub message: String,
    pub url: Option<String>,
    #[serde(default)]
    pub needs_attention: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkItem {
    pub task_id: String,
    pub agent_id: String,
    pub phase: String,
    pub request: String,
    pub lease: String,
    pub lease_until: i64,
    pub draft: Option<Draft>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewTask {
    pub owner_id: String,
    pub agent_id: Option<String>,
    pub request: String,
    pub idempotency_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Approve,
    Decline,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Event {
    pub sequence: i64,
    pub task_id: String,
    pub kind: String,
    pub created_at: i64,
}
