use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClientCountsPayload {
    #[serde(default)]
    pub channels: Vec<RawUnread>,
    #[serde(default)]
    pub ims: Vec<RawUnread>,
    #[serde(default)]
    pub mpims: Vec<RawUnread>,
    #[serde(default)]
    pub threads: RawThreadCounts,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawUnread {
    pub id: String,
    #[serde(default)]
    pub has_unreads: bool,
    #[serde(default)]
    pub mention_count: u64,
    #[serde(default)]
    pub last_read: Option<String>,
    #[serde(default)]
    pub latest: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawThreadCounts {
    #[serde(default)]
    pub has_unreads: bool,
    #[serde(default)]
    pub mention_count: u64,
    #[serde(default)]
    pub unread_count_by_channel: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Channel,
    DirectMessage,
    GroupDirectMessage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadConversation {
    pub id: String,
    pub kind: ConversationKind,
    pub has_unreads: bool,
    pub mention_count: u64,
    pub last_read: Option<String>,
    pub latest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadThreads {
    pub has_unreads: bool,
    pub mention_count: u64,
    pub unread_count_by_channel: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnreadReport {
    pub team_id: String,
    pub conversations: Vec<UnreadConversation>,
    pub threads: UnreadThreads,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DoctorReport {
    pub authenticated: bool,
    pub team_id: String,
    pub workspace_url: String,
}
