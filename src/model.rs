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

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawResponseMetadata {
    #[serde(default)]
    pub next_cursor: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessagePage {
    pub messages: Vec<RawMessage>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub response_metadata: RawResponseMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessagesList {
    #[serde(default)]
    pub messages: BTreeMap<String, RawMessage>,
    #[serde(default)]
    pub messages_data: BTreeMap<String, RawChannelMessages>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawChannelMessages {
    pub messages: Vec<RawMessage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessage {
    pub ts: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub bot_id: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub reply_count: u64,
    #[serde(default)]
    pub latest_reply: Option<String>,
    #[serde(default)]
    pub reactions: Vec<RawReaction>,
    #[serde(default)]
    pub files: Vec<RawFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawReaction {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub count: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub mimetype: String,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub url_private_download: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawUsersPage {
    pub members: Vec<RawUser>,
    #[serde(default)]
    pub response_metadata: RawResponseMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawUser {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub deleted: bool,
    #[serde(default)]
    pub is_bot: bool,
    #[serde(default)]
    pub tz: Option<String>,
    #[serde(default)]
    pub profile: RawUserProfile,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawUserProfile {
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub real_name: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub image_72: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Message {
    pub channel_id: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub author_id: Option<String>,
    pub author_name: Option<String>,
    pub text: String,
    pub reply_count: u64,
    pub latest_reply: Option<String>,
    pub reactions: Vec<Reaction>,
    pub files: Vec<FileReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Reaction {
    pub name: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FileReference {
    pub id: String,
    pub name: String,
    pub mimetype: String,
    pub size: u64,
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MessagePage {
    pub channel_id: String,
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ThreadPage {
    pub channel_id: String,
    pub thread_ts: String,
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct User {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub real_name: String,
    pub title: String,
    pub deleted: bool,
    pub is_bot: bool,
    pub timezone: Option<String>,
    pub image_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserSearchReport {
    pub query: String,
    pub users: Vec<User>,
    pub truncated: bool,
}
