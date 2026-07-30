use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ClientCountsPayload {
    pub channels: Vec<RawUnread>,
    pub ims: Vec<RawUnread>,
    pub mpims: Vec<RawUnread>,
    pub threads: RawThreadCounts,
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct RawUnread {
    pub id: String,
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
    pub blocks: Option<Vec<Value>>,
    #[serde(default)]
    pub attachments: Option<Vec<Value>>,
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
    #[serde(default)]
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawFile {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub alt_txt: Option<String>,
    #[serde(default)]
    pub mimetype: Option<String>,
    #[serde(default)]
    pub filetype: Option<String>,
    #[serde(default)]
    pub pretty_type: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub file_access: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub created: Option<u64>,
    #[serde(default)]
    pub timestamp: Option<u64>,
    #[serde(default)]
    pub editable: Option<bool>,
    #[serde(default)]
    pub is_external: Option<bool>,
    #[serde(default)]
    pub is_public: Option<bool>,
    #[serde(default)]
    pub public_url_shared: Option<bool>,
    #[serde(default)]
    pub url_private: Option<String>,
    #[serde(default)]
    pub url_private_download: Option<String>,
    #[serde(default)]
    pub permalink: Option<String>,
    #[serde(default)]
    pub channels: Option<Vec<String>>,
    #[serde(default)]
    pub groups: Option<Vec<String>>,
    #[serde(default)]
    pub ims: Option<Vec<String>>,
    #[serde(default)]
    pub shares: Option<RawFileShares>,
    #[serde(default)]
    pub has_more_shares: Option<bool>,
    #[serde(default)]
    pub skipped_shares: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawFileShares {
    #[serde(default)]
    pub public: BTreeMap<String, Vec<RawFileShare>>,
    #[serde(default)]
    pub private: BTreeMap<String, Vec<RawFileShare>>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawFileShare {
    #[serde(default)]
    pub ts: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawFileResponse {
    pub file: RawFile,
}

#[derive(Clone, Deserialize)]
pub(crate) struct RawFileUploadAllocation {
    #[serde(default, deserialize_with = "deserialize_optional_string_lossy")]
    pub upload_url: Option<String>,
    #[serde(
        default,
        rename = "file",
        deserialize_with = "deserialize_optional_string_lossy"
    )]
    pub file_id: Option<String>,
}

fn deserialize_optional_string_lossy<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_str().map(str::to_owned))
}

#[derive(Clone, Deserialize)]
pub(crate) struct RawFileUploadCompletion {
    pub files: Vec<RawFile>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawEmojiResponse {
    #[serde(default)]
    pub emoji: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawAuthTestResponse {
    #[serde(default)]
    pub user_id: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawReactionItemResponse {
    #[serde(default, rename = "type")]
    pub item_type: String,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub message: Option<RawMessage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawUsersPage {
    pub members: Vec<RawUser>,
    #[serde(default)]
    pub response_metadata: RawResponseMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawConversationsPage {
    pub channels: Vec<RawConversation>,
    #[serde(default)]
    pub response_metadata: RawResponseMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawConversation {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(default)]
    pub is_private: bool,
    #[serde(default)]
    pub is_member: bool,
    #[serde(default)]
    pub is_im: bool,
    #[serde(default)]
    pub is_mpim: bool,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub num_members: Option<u64>,
    /// First-party responses may include the newest message; activity reads only its timestamp.
    #[serde(default)]
    pub latest: Option<Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessageSearchResponse {
    #[serde(default)]
    pub query: String,
    pub messages: RawMessageSearchMatches,
    #[serde(default)]
    pub response_metadata: RawResponseMetadata,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessageSearchMatches {
    pub matches: Vec<RawMessageSearchMatch>,
    pub total: u64,
    #[serde(default)]
    pub pagination: RawMessageSearchPagination,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessageSearchPagination {
    #[serde(default)]
    pub next_cursor: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessageSearchMatch {
    pub channel: RawMessageSearchChannel,
    pub ts: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
    #[serde(default)]
    pub user: Option<String>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub blocks: Option<Vec<Value>>,
    #[serde(default)]
    pub attachments: Option<Vec<Value>>,
    #[serde(default)]
    pub reactions: Vec<RawReaction>,
    #[serde(default)]
    pub files: Vec<RawFile>,
    #[serde(default)]
    pub permalink: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMessageSearchChannel {
    pub id: String,
    #[serde(default)]
    pub name: String,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawDraftsPage {
    #[serde(default)]
    pub drafts: Vec<RawDraft>,
    #[serde(default)]
    pub files: Vec<Value>,
    #[serde(default)]
    pub has_more: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawDraftResponse {
    pub draft: RawDraft,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawMutationResponse {}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawPostMessageResponse {
    #[serde(default)]
    pub channel: String,
    #[serde(default)]
    pub ts: String,
    pub message: RawMessage,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub(crate) struct RawDraft {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub client_msg_id: Option<String>,
    #[serde(default)]
    pub last_updated_ts: Option<RawDraftRevision>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub blocks: Option<Vec<Value>>,
    #[serde(default)]
    pub destinations: Vec<DraftDestination>,
    #[serde(default)]
    pub file_ids: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<Value>,
    #[serde(default)]
    pub is_from_composer: bool,
    #[serde(default)]
    pub is_deleted: bool,
    #[serde(default)]
    pub is_sent: bool,
    #[serde(default)]
    pub date_created: Option<u64>,
    #[serde(default)]
    pub date_scheduled: Option<u64>,
    #[serde(default)]
    pub last_updated_client: Option<String>,
    #[serde(default)]
    pub team_id: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum RawDraftRevision {
    String(String),
    Number(serde_json::Number),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationKind {
    Channel,
    DirectMessage,
    GroupDirectMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationNameResolution {
    Resolved,
    Incomplete,
    Inaccessible,
    Unnamed,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnreadConversation {
    pub id: String,
    pub kind: ConversationKind,
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub name_resolution: ConversationNameResolution,
    pub has_unreads: bool,
    pub mention_count: u64,
    pub last_read: Option<String>,
    pub latest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnreadThreads {
    pub has_unreads: bool,
    pub mention_count: u64,
    pub unread_count_by_channel: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UnreadReport {
    pub team_id: String,
    pub conversations: Vec<UnreadConversation>,
    pub threads: UnreadThreads,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DoctorReport {
    pub authenticated: bool,
    pub team_id: String,
    pub workspace_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Conversation {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub name_is_fallback: bool,
    /// Whether archive, membership, privacy, and member-count metadata came from Slack discovery.
    pub metadata_is_complete: bool,
    pub kind: ConversationKind,
    pub is_private: bool,
    pub is_archived: bool,
    pub is_member: bool,
    pub member_count: Option<u64>,
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConversationPage {
    pub conversations: Vec<Conversation>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConversationSearchReport {
    pub query: String,
    pub conversations: Vec<Conversation>,
    pub truncated: bool,
    pub truncation_reason: Option<ConversationSearchTruncationReason>,
    pub scanned_conversations: usize,
    pub scan_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSearchTruncationReason {
    ResultLimit,
    ScanLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MentionResolution {
    /// The selected message representation contains no user mentions.
    NotNeeded,
    /// Mentions were detected but this acknowledgement path does not perform enrichment.
    NotAttempted,
    /// Every detected mention has a safe rendered identity.
    Complete,
    /// At least one mention could not be rendered after a complete or bounded lookup.
    Partial,
    /// At least one mention remained unresolved after an interrupted or conflicting lookup.
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermalinkResolution {
    /// Every link applicable to this message is available.
    Complete,
    /// At least one applicable link is available.
    Partial,
    /// No applicable link could be constructed safely.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MessageMention {
    pub id: String,
    pub username: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MessageSearchMatch {
    pub channel_id: String,
    pub channel_name: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub permalink: Option<String>,
    pub thread_root_permalink: Option<String>,
    pub permalink_resolution: PermalinkResolution,
    pub author_id: Option<String>,
    /// Slack username or message-supplied author name when known.
    pub author_name: Option<String>,
    /// Slack profile display name, falling back to the profile's real name.
    pub author_display_name: Option<String>,
    pub author_resolution: AuthorResolution,
    /// Canonical Slack-native message text.
    pub text: String,
    /// Read-only human rendering. Outbound operations never use this value.
    pub rendered_text: String,
    pub mention_resolution: MentionResolution,
    /// Unique user mentions in first-encounter order.
    pub mentions: Vec<MessageMention>,
    /// Raw Slack Block Kit JSON. `None` means Slack omitted `blocks`; an empty
    /// vector means Slack explicitly returned an empty array.
    pub blocks: Option<Vec<Value>>,
    /// Raw Slack legacy-attachment JSON. `None` means Slack omitted the field;
    /// an empty vector means Slack explicitly returned an empty array.
    pub attachments: Option<Vec<Value>>,
    pub reactions: Vec<Reaction>,
    pub files: Vec<FileReference>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MessageSearchPage {
    pub query: String,
    pub matches: Vec<MessageSearchMatch>,
    pub total: u64,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct RenderedMessage {
    /// Plain-text fallback used for notifications and accessibility.
    pub text: String,
    /// Slack `rich_text` blocks generated from the Markdown source.
    pub blocks: Vec<Value>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct DraftDestination {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub broadcast: bool,
    /// Slack-provided participant metadata for direct-message destinations.
    /// Lurkline preserves this value but routes only by `channel_id`.
    #[serde(
        default,
        deserialize_with = "deserialize_present_user_ids",
        skip_serializing_if = "Option::is_none"
    )]
    pub user_ids: Option<Vec<String>>,
    /// Unknown private-API destination fields are retained for diagnostics but
    /// make the draft unsupported for mutation or publication.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn is_false(value: &bool) -> bool {
    !value
}

fn deserialize_present_user_ids<'de, D>(
    deserializer: D,
) -> std::result::Result<Option<Vec<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<Vec<String>>::deserialize(deserializer)?
        .map(Some)
        .ok_or_else(|| serde::de::Error::custom("user_ids must be an array"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileDraftAssociation {
    /// The response identified one file but didn't perform a complete ownership proof.
    Unverified,
    /// Complete Slack state proved exclusive private-file ownership.
    Verified,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Draft {
    pub id: String,
    pub client_msg_id: Option<String>,
    /// Slack's server revision, used as the drafts.list continuation cursor.
    pub last_updated_ts: String,
    /// Browser-compatible timestamp derived from the server revision for deletion.
    pub client_last_updated_ts: String,
    pub text: String,
    pub blocks: Option<Vec<Value>>,
    pub destinations: Vec<DraftDestination>,
    pub file_ids: Vec<String>,
    pub attachments: Vec<Value>,
    pub is_from_composer: bool,
    /// Ownership-proof state for a one-file draft. Absent for text-only drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_association: Option<FileDraftAssociation>,
    /// Whether Lurkline can safely update, delete, or publish this draft.
    pub is_supported: bool,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) has_unknown_fields: bool,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) file_shape_supported: bool,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) date_created: Option<u64>,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) date_scheduled: Option<u64>,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) last_updated_client: Option<String>,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) team_id: Option<String>,
    #[serde(skip)]
    #[schemars(skip)]
    pub(crate) user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DraftPage {
    pub drafts: Vec<Draft>,
    pub has_more: bool,
    /// Pass this private-API timestamp to the next list request.
    pub next_ts: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DraftDeleteReport {
    pub id: String,
    pub deleted: bool,
    /// File associated with the deleted draft. Absent for text-only drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
    /// Always false for one-file drafts because deletion preserves the file to
    /// avoid a cross-process ownership race. Absent for text-only drafts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_deleted: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct SentMessage {
    /// Client-generated UUID v4 used to make the publication identifiable.
    pub client_msg_id: String,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DraftCleanupWarning {
    /// Draft whose post-publication deletion wasn't confirmed.
    pub draft_id: String,
    /// Server revision observed immediately before publication.
    pub last_updated_ts: String,
    /// Secret-safe reason the post-success cleanup failed.
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DraftSendReport {
    pub sent: SentMessage,
    pub draft_id: String,
    /// True only when Slack acknowledged deletion or a bounded reread proved absence.
    pub draft_deleted: bool,
    /// Present when Slack acknowledged the message but draft deletion wasn't confirmed.
    pub cleanup_warning: Option<DraftCleanupWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InboxConversation {
    pub conversation: Conversation,
    pub unread: UnreadConversation,
    pub messages: MessagePage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InboxTruncationReason {
    ConversationLimit,
    ByteLimit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct InboxReport {
    pub team_id: String,
    pub conversations: Vec<InboxConversation>,
    pub total_unread_conversations: usize,
    pub has_more_conversations: bool,
    /// Why unread conversations were omitted; absent when this report is complete.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub truncation_reason: Option<InboxTruncationReason>,
    pub threads: UnreadThreads,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityOrder {
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ActivityConversationStatus {
    Complete,
    MessageLimit,
    Inaccessible,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActivityConversationResult {
    pub conversation: Conversation,
    pub status: ActivityConversationStatus,
    pub messages_sampled: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActivityItem {
    pub conversation_id: String,
    pub conversation_name: String,
    pub conversation_display_name: String,
    pub conversation_kind: ConversationKind,
    pub message: Message,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ActivityReport {
    pub team_id: String,
    /// Canonical UTC lower bound. The interval includes this instant.
    pub effective_after: String,
    /// Canonical UTC upper bound. The interval excludes this instant.
    pub effective_before: String,
    pub order: ActivityOrder,
    pub items: Vec<ActivityItem>,
    pub conversation_results: Vec<ActivityConversationResult>,
    pub scanned_conversations: usize,
    pub selected_conversations: usize,
    pub conversation_limit: usize,
    pub per_conversation_limit: usize,
    pub limit: usize,
    pub selection_truncated: bool,
    pub partial: bool,
    pub response_byte_limit_reached: bool,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Message {
    pub channel_id: String,
    pub ts: String,
    pub thread_ts: Option<String>,
    pub permalink: Option<String>,
    pub thread_root_permalink: Option<String>,
    pub permalink_resolution: PermalinkResolution,
    pub author_id: Option<String>,
    /// Slack username or message-supplied author name when known.
    pub author_name: Option<String>,
    /// Slack profile display name, falling back to the profile's real name.
    pub author_display_name: Option<String>,
    pub author_resolution: AuthorResolution,
    /// Canonical Slack-native message text.
    pub text: String,
    /// Read-only human rendering. Outbound operations never use this value.
    pub rendered_text: String,
    pub mention_resolution: MentionResolution,
    /// Unique user mentions in first-encounter order.
    pub mentions: Vec<MessageMention>,
    /// Raw Slack Block Kit JSON. Unknown block and element fields are retained.
    pub blocks: Option<Vec<Value>>,
    /// Raw Slack legacy-attachment JSON. Unknown nested fields are retained.
    /// `None` means Slack omitted the field; an empty vector means Slack
    /// explicitly returned an empty array.
    pub attachments: Option<Vec<Value>>,
    pub reply_count: u64,
    pub latest_reply: Option<String>,
    pub reactions: Vec<Reaction>,
    pub files: Vec<FileReference>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthorResolution {
    /// Slack supplied the author name directly on the message.
    Provided,
    /// Lurkline resolved the author through the bounded user directory.
    Directory,
    /// This path preserved the author ID without requesting directory enrichment.
    NotAttempted,
    /// A complete directory did not contain a usable identity for this ID.
    Unresolved,
    /// The bounded directory ended before every Slack user could be scanned.
    Incomplete,
    /// The auxiliary directory request failed; the message itself is intact.
    Unavailable,
    /// Slack supplied neither an author ID nor an author name.
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Reaction {
    pub name: String,
    pub count: u64,
    /// Slack may return fewer user IDs than `count`.
    pub user_ids: Vec<String>,
    pub user_ids_complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileReference {
    pub id: String,
    pub name: Option<String>,
    pub title: Option<String>,
    /// Slack-provided alternative text. `None` means Slack omitted the field.
    pub alt_text: Option<String>,
    pub mimetype: Option<String>,
    pub filetype: Option<String>,
    pub pretty_type: Option<String>,
    pub mode: Option<String>,
    pub file_access: Option<String>,
    pub uploader_id: Option<String>,
    pub size: Option<u64>,
    pub created: Option<u64>,
    pub timestamp: Option<u64>,
    pub editable: Option<bool>,
    pub is_external: Option<bool>,
    pub is_public: Option<bool>,
    pub public_url_shared: Option<bool>,
    pub private_url: Option<String>,
    pub download_url: Option<String>,
    pub permalink: Option<String>,
    /// Public-channel IDs. `None` means Slack omitted the field.
    pub channel_ids: Option<Vec<String>>,
    /// Private-channel IDs. `None` means Slack omitted the field.
    pub group_ids: Option<Vec<String>>,
    /// Direct-message IDs. `None` means Slack omitted the field.
    pub im_ids: Option<Vec<String>>,
    /// `None` means Slack omitted share metadata; `Some([])` is explicitly empty.
    pub shares: Option<Vec<FileShare>>,
    /// Whether `shares` is present and Slack did not report omitted entries.
    pub shares_complete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FileShareVisibility {
    Public,
    Private,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
/// A normalized share proof. Direct-message upload verification may synthesize
/// this record from exact file membership and message-route state.
pub struct FileShare {
    pub visibility: FileShareVisibility,
    pub channel_id: String,
    pub ts: String,
    pub thread_ts: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CustomEmojiKind {
    Image,
    Alias,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CustomEmoji {
    pub name: String,
    pub kind: CustomEmojiKind,
    pub image_url: Option<String>,
    pub alias_for: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CustomEmojiList {
    pub emoji: Vec<CustomEmoji>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ReactionMutationReport {
    pub channel_id: String,
    pub message_ts: String,
    pub name: String,
    pub target_present: bool,
    pub present: bool,
    pub changed: bool,
    pub reconciled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct FileDownloadReport {
    pub file: FileReference,
    pub output_path: String,
    pub bytes_written: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub durability_warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum FileUploadReport {
    /// Slack may have allocated an upload, but returned no safe recovery key.
    AllocationUncertain,
    /// Slack allocated a file ID, but no file bytes were sent.
    Allocated { file_id: String },
    /// Slack allocated a file ID, but the local source changed before completion.
    SourceChanged { file_id: String },
    /// Slack allocated a file ID, but byte acceptance cannot be proven.
    TransferUncertain { file_id: String },
    /// Slack received the bytes, but target sharing cannot be proven.
    CompletionUncertain { file_id: String },
    /// Exact Slack file membership and message-route state prove the requested target.
    Shared {
        file: Box<FileReference>,
        share: FileShare,
        reconciled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "stage", rename_all = "snake_case")]
pub enum FileDraftCreateReport {
    /// Slack may have allocated an upload, but returned no safe recovery key.
    AllocationUncertain,
    /// Slack allocated a file ID, but no file bytes were sent.
    Allocated { file_id: String },
    /// Slack allocated a file ID, but the local source changed.
    SourceChanged { file_id: String },
    /// Slack byte acceptance cannot be proven.
    TransferUncertain { file_id: String },
    /// Slack received the bytes, but private completion cannot be proven.
    FileCompletionUncertain { file_id: String },
    /// Slack definitively rejected draft creation after the private file was ready.
    DraftNotCreated { file_id: String, reason: String },
    /// Draft creation may have succeeded; do not retry automatically.
    DraftCreationUncertain {
        file_id: String,
        client_msg_id: String,
    },
    /// Exact cross-process Slack state proves one draft exclusively owns the file.
    Created {
        draft: Box<Draft>,
        file: Box<FileReference>,
        reconciled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct MessagePage {
    pub channel_id: String,
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ThreadPage {
    pub channel_id: String,
    pub thread_ts: String,
    pub messages: Vec<Message>,
    pub has_more: bool,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct UserSearchReport {
    pub query: String,
    pub users: Vec<User>,
    pub truncated: bool,
    pub truncation_reason: Option<UserSearchTruncationReason>,
    pub scanned_users: usize,
    pub scan_limit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UserSearchTruncationReason {
    ResultLimit,
    ScanLimit,
}
