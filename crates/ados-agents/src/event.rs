//! Plain Rust values for what an agent streams and asks, so callers never touch ACP types.
use std::sync::Arc;

use agent_client_protocol::v1::schema::{
    ContentBlock, PermissionOptionKind, RequestPermissionRequest, SessionNotification,
    SessionUpdate, ToolCallStatus, ToolKind,
};
use futures::future::BoxFuture;

/// One thing the agent reported. Every variant except `Crashed` names its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// A chunk of the agent's reply.
    Text { session_id: String, text: String },
    /// A chunk of the agent's visible reasoning.
    Thought { session_id: String, text: String },
    /// A chunk of a user message (agents replay these on `session/load`).
    UserText { session_id: String, text: String },
    /// The agent started a tool call.
    ToolCall { session_id: String, id: String, title: String, kind: String, status: ToolStatus },
    /// A tool call changed; `None` fields did not change.
    ToolCallUpdate {
        session_id: String,
        id: String,
        title: Option<String>,
        status: Option<ToolStatus>,
    },
    /// The agent process ended without being asked to. `message` is a plain sentence.
    Crashed { message: String },
}

impl AgentEvent {
    /// The session the event belongs to (`None` for `Crashed`, which is about the process).
    pub fn session_id(&self) -> Option<&str> {
        match self {
            Self::Text { session_id, .. }
            | Self::Thought { session_id, .. }
            | Self::UserText { session_id, .. }
            | Self::ToolCall { session_id, .. }
            | Self::ToolCallUpdate { session_id, .. } => Some(session_id),
            Self::Crashed { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// Why a prompt turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
    Other,
}

/// The agent asks before running a tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub session_id: String,
    pub tool_call_id: String,
    /// What the tool call is, e.g. "Write note.txt". May be empty.
    pub title: String,
    pub options: Vec<PermissionChoice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionChoice {
    pub id: String,
    pub name: String,
    pub kind: PermissionKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
    Other,
}

impl PermissionKind {
    pub fn is_allow(self) -> bool {
        matches!(self, Self::AllowOnce | Self::AllowAlways)
    }
}

/// Answers a permission request with the chosen option id, or `None` to deny.
/// Denying picks the agent's reject option when it offers one, else cancels the request.
pub type PermissionCallback =
    Arc<dyn Fn(PermissionRequest) -> BoxFuture<'static, Option<String>> + Send + Sync>;

/// The default permission callback: deny everything.
pub fn deny() -> PermissionCallback {
    Arc::new(|_| Box::pin(async { None }))
}

pub(crate) fn permission_request(req: &RequestPermissionRequest) -> PermissionRequest {
    PermissionRequest {
        session_id: req.session_id.0.to_string(),
        tool_call_id: req.tool_call.tool_call_id.0.to_string(),
        title: req.tool_call.fields.title.clone().unwrap_or_default(),
        options: req
            .options
            .iter()
            .map(|o| PermissionChoice {
                id: o.option_id.0.to_string(),
                name: o.name.clone(),
                kind: match o.kind {
                    PermissionOptionKind::AllowOnce => PermissionKind::AllowOnce,
                    PermissionOptionKind::AllowAlways => PermissionKind::AllowAlways,
                    PermissionOptionKind::RejectOnce => PermissionKind::RejectOnce,
                    PermissionOptionKind::RejectAlways => PermissionKind::RejectAlways,
                    _ => PermissionKind::Other,
                },
            })
            .collect(),
    }
}

pub(crate) fn stop_reason(reason: agent_client_protocol::v1::schema::StopReason) -> StopReason {
    use agent_client_protocol::v1::schema::StopReason as S;
    match reason {
        S::EndTurn => StopReason::EndTurn,
        S::MaxTokens => StopReason::MaxTokens,
        S::MaxTurnRequests => StopReason::MaxTurnRequests,
        S::Refusal => StopReason::Refusal,
        S::Cancelled => StopReason::Cancelled,
        _ => StopReason::Other,
    }
}

fn status(s: ToolCallStatus) -> ToolStatus {
    match s {
        ToolCallStatus::Pending => ToolStatus::Pending,
        ToolCallStatus::InProgress => ToolStatus::InProgress,
        ToolCallStatus::Completed => ToolStatus::Completed,
        _ => ToolStatus::Failed,
    }
}

fn kind_name(kind: ToolKind) -> String {
    // ToolKind serialises as a snake_case string ("read", "edit", "execute", ...).
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(s)) => s,
        _ => "other".into(),
    }
}

fn text(block: &ContentBlock) -> String {
    match block {
        ContentBlock::Text(t) => t.text.clone(),
        ContentBlock::Image(_) => "[image]".into(),
        ContentBlock::Audio(_) => "[audio]".into(),
        ContentBlock::ResourceLink(l) => format!("[{}]", l.uri),
        _ => "[resource]".into(),
    }
}

/// Turn one `session/update` into an event. Updates aDOs does not show yet
/// (plans, modes, commands, usage) give `None`.
pub(crate) fn from_notification(n: &SessionNotification) -> Option<AgentEvent> {
    let session_id = n.session_id.0.to_string();
    Some(match &n.update {
        SessionUpdate::AgentMessageChunk(c) => AgentEvent::Text { session_id, text: text(&c.content) },
        SessionUpdate::AgentThoughtChunk(c) => {
            AgentEvent::Thought { session_id, text: text(&c.content) }
        }
        SessionUpdate::UserMessageChunk(c) => {
            AgentEvent::UserText { session_id, text: text(&c.content) }
        }
        SessionUpdate::ToolCall(t) => AgentEvent::ToolCall {
            session_id,
            id: t.tool_call_id.0.to_string(),
            title: t.title.clone(),
            kind: kind_name(t.kind),
            status: status(t.status),
        },
        SessionUpdate::ToolCallUpdate(u) => AgentEvent::ToolCallUpdate {
            session_id,
            id: u.tool_call_id.0.to_string(),
            title: u.fields.title.clone(),
            status: u.fields.status.map(status),
        },
        _ => return None,
    })
}
