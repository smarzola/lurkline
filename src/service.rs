use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet, hash_map::Entry},
    io::{self, Write},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    config::Config,
    error::{Error, Result},
    local_file::{BoundedDownload, DownloadDurability, UploadPass, UploadSource},
    markdown::{
        MAX_MARKDOWN_BYTES, ResolvedOutboundUser, outbound_mention_references, render_markdown,
        render_markdown_with_mentions,
    },
    model::{
        ActivityContinuationKind, ActivityConversationResult, ActivityConversationStatus,
        ActivityItem, ActivityOrder, ActivityReport, AuthorResolution, ClientCountsPayload,
        Conversation, ConversationKind, ConversationNameResolution, ConversationPage,
        ConversationSearchReport, ConversationSearchTruncationReason, CustomEmoji, CustomEmojiKind,
        CustomEmojiList, DoctorReport, Draft, DraftCleanupWarning, DraftDeleteReport,
        DraftDestination, DraftPage, DraftSendReport, FileDownloadReport, FileDraftAssociation,
        FileDraftCreateReport, FileReference, FileShare, FileShareVisibility, FileUploadReport,
        InboxConversation, InboxReport, InboxTruncationReason, MentionResolution, Message,
        MessageMention, MessagePage, MessageSearchMatch, MessageSearchPage,
        OutboundMentionResolution, PermalinkResolution, RawAuthTestResponse, RawConversation,
        RawConversationsPage, RawDraft, RawDraftResponse, RawDraftRevision, RawDraftsPage,
        RawEmojiResponse, RawFile, RawFileResponse, RawFileUploadAllocation,
        RawFileUploadCompletion, RawMessage, RawMessagePage, RawMessageSearchMatch,
        RawMessageSearchResponse, RawMessagesList, RawMutationResponse, RawPostMessageResponse,
        RawReaction, RawReactionItemResponse, RawUnread, RawUser, RawUsersPage, Reaction,
        ReactionMutationReport, RenderedMessage, SentMessage, ThreadPage, UnreadConversation,
        UnreadReport, UnreadThreads, User, UserSearchReport, UserSearchTruncationReason,
    },
};

const MAX_MESSAGES: usize = 200;
pub(crate) const MAX_INBOX_CONVERSATIONS: usize = 50;
pub(crate) const MAX_ACTIVITY_CONVERSATIONS: usize = 50;
pub(crate) const MAX_ACTIVITY_MESSAGES: usize = 100;
pub(crate) const MAX_ACTIVITY_PER_CONVERSATION: usize = 200;
const DEFAULT_ACTIVITY_CONVERSATIONS: usize = 10;
const DEFAULT_ACTIVITY_MESSAGES: usize = 50;
const DEFAULT_ACTIVITY_PER_CONVERSATION: usize = 20;
const MAX_ACTIVITY_DURATION_SECONDS: i64 = 365 * 24 * 60 * 60;
const MAX_ACTIVITY_SELECTORS: usize = 50;
const MAX_ACTIVITY_CURSOR_LENGTH: usize = 8_192;
const ACTIVITY_CURSOR_VERSION: u8 = 2;
const ACTIVITY_CURSOR_PREFIX: &str = "activity-v2";
const ACTIVITY_CURSOR_DOMAIN: &[u8] = b"lurkline-activity-cursor-v2\0";
pub(crate) const MAX_USERS: usize = 100;
pub(crate) const MAX_CONVERSATIONS: usize = 100;
pub(crate) const CONVERSATIONS_PAGE_SIZE: usize = 200;
pub(crate) const MAX_SEARCH_MESSAGES: usize = 100;
const MAX_CONVERSATION_PAGES: usize = 20;
const USERS_PAGE_SIZE: usize = 200;
const MAX_USER_PAGES: usize = 20;
pub(crate) const MAX_DRAFTS: usize = 100;
pub(crate) const DEFAULT_FILE_DOWNLOAD_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const MAX_FILE_DOWNLOAD_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const DEFAULT_FILE_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
pub(crate) const MAX_FILE_UPLOAD_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_FILE_UPLOAD_NAME_BYTES: usize = 255;
const MAX_FILE_UPLOAD_TITLE_BYTES: usize = 255;
const MAX_FILE_UPLOAD_ALT_TEXT_BYTES: usize = 1_000;
const UPLOAD_RECONCILIATION_DELAYS_MS: &[u64] = &[0, 100, 250, 500, 1_000, 2_000];
const DRAFT_RECONCILIATION_DELAYS_MS: &[u64] = &[0, 250, 500, 1_000, 2_000, 4_000];
const MAX_DRAFT_OWNERSHIP_SCAN_PAGES: usize = 10;
const MAX_DRAFT_DESTINATION_USERS: usize = 100;
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 100;
const MAX_FILES_PER_MESSAGE: usize = 100;
const MAX_FILE_CONVERSATIONS: usize = 1_000;
const MAX_FILE_SHARES: usize = 1_000;
const MAX_FILE_SHARE_SCAN_PAGES: usize = 10;
const MAX_REACTIONS_PER_MESSAGE: usize = 100;
const MAX_REACTION_USERS: usize = 1_000;
const MAX_CUSTOM_EMOJI: usize = 10_000;
const MAX_MESSAGE_MENTIONS: usize = 256;
const MAX_RICH_TEXT_RENDER_NODES: usize = 4_096;
const MAX_RICH_TEXT_RENDER_DEPTH: usize = 64;

pub(crate) struct FileDraftCreateRequest<'a> {
    pub(crate) conversation: &'a str,
    pub(crate) thread_ts: Option<&'a str>,
    pub(crate) broadcast: bool,
    pub(crate) markdown: &'a str,
    pub(crate) title: Option<&'a str>,
    pub(crate) alt_text: Option<&'a str>,
    pub(crate) confirmed: bool,
}

pub(crate) struct ActivityRequest<'a> {
    pub(crate) since: Option<&'a str>,
    pub(crate) after: Option<&'a str>,
    pub(crate) before: Option<&'a str>,
    pub(crate) include: &'a [String],
    pub(crate) exclude: &'a [String],
    pub(crate) kinds: &'a [ConversationKind],
    pub(crate) order: Option<ActivityOrder>,
    pub(crate) conversation_limit: Option<usize>,
    pub(crate) per_conversation_limit: Option<usize>,
    pub(crate) limit: Option<usize>,
    pub(crate) cursor: Option<&'a str>,
}

pub(crate) struct ChatPostMessageRequest<'a> {
    pub(crate) channel: &'a str,
    pub(crate) thread_ts: Option<&'a str>,
    pub(crate) broadcast: bool,
    pub(crate) client_msg_id: &'a str,
    pub(crate) text: &'a str,
    pub(crate) blocks: &'a [serde_json::Value],
}

pub(crate) struct FileShareRequest<'a> {
    pub(crate) channel: &'a str,
    pub(crate) thread_ts: Option<&'a str>,
    pub(crate) broadcast: bool,
    pub(crate) client_msg_id: &'a str,
    pub(crate) draft_id: &'a str,
    pub(crate) blocks: &'a [serde_json::Value],
    pub(crate) file_id: &'a str,
}

#[async_trait]
pub(crate) trait SlackApi: Send + Sync {
    async fn client_counts(&self) -> Result<ClientCountsPayload>;
    async fn conversation_history(
        &self,
        channel: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessagePage>;
    async fn activity_history(
        &self,
        channel: &str,
        oldest: &str,
        latest: &str,
        limit: usize,
    ) -> Result<RawMessagePage> {
        let _ = (oldest, latest);
        self.conversation_history(channel, None, limit).await
    }
    async fn conversation_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessagePage>;
    async fn messages_list(&self, channel: &str, message_ts: &str) -> Result<RawMessagesList>;
    async fn conversations_list(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawConversationsPage>;
    async fn search_messages(
        &self,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessageSearchResponse>;
    async fn users_list(&self, cursor: Option<&str>, limit: usize) -> Result<RawUsersPage>;
    async fn auth_test(&self) -> Result<RawAuthTestResponse> {
        Err(Error::InvalidResponse {
            method: "auth.test",
        })
    }
    async fn emoji_list(&self) -> Result<RawEmojiResponse> {
        Err(Error::InvalidResponse {
            method: "emoji.list",
        })
    }
    async fn files_info(&self, file_id: &str) -> Result<RawFileResponse> {
        let _ = file_id;
        Err(Error::InvalidResponse {
            method: "files.info",
        })
    }
    async fn reactions_get(
        &self,
        channel: &str,
        message_ts: &str,
    ) -> Result<RawReactionItemResponse> {
        let _ = (channel, message_ts);
        Err(Error::InvalidResponse {
            method: "reactions.get",
        })
    }
    async fn reactions_add(
        &self,
        channel: &str,
        message_ts: &str,
        name: &str,
    ) -> Result<RawMutationResponse> {
        let _ = (channel, message_ts, name);
        Err(Error::InvalidResponse {
            method: "reactions.add",
        })
    }
    async fn reactions_remove(
        &self,
        channel: &str,
        message_ts: &str,
        name: &str,
    ) -> Result<RawMutationResponse> {
        let _ = (channel, message_ts, name);
        Err(Error::InvalidResponse {
            method: "reactions.remove",
        })
    }
    async fn download_private_file(
        &self,
        download_url: &str,
        expected_size: u64,
        expected_mimetype: Option<&str>,
        target: &mut BoundedDownload,
    ) -> Result<()> {
        let _ = (download_url, expected_size, expected_mimetype, target);
        Err(Error::InvalidResponse {
            method: "files.download",
        })
    }
    async fn files_get_upload_url(
        &self,
        filename: &str,
        length: u64,
        alt_text: Option<&str>,
    ) -> Result<RawFileUploadAllocation> {
        let _ = (filename, length, alt_text);
        Err(Error::InvalidResponse {
            method: "files.getUploadURL",
        })
    }
    async fn upload_edge_file(
        &self,
        upload_url: &str,
        source: &mut UploadSource,
    ) -> Result<UploadPass> {
        let _ = (upload_url, source);
        Err(Error::InvalidResponse {
            method: "files.uploadEdge",
        })
    }
    async fn files_complete_upload(
        &self,
        file_id: &str,
        title: Option<&str>,
        channel_id: &str,
        thread_ts: Option<&str>,
        client_msg_id: &str,
    ) -> Result<RawFileUploadCompletion> {
        let _ = (file_id, title, channel_id, thread_ts, client_msg_id);
        Err(Error::InvalidResponse {
            method: "files.completeUpload",
        })
    }
    async fn files_complete_draft_upload(
        &self,
        file_id: &str,
        title: Option<&str>,
    ) -> Result<RawFileUploadCompletion> {
        let _ = (file_id, title);
        Err(Error::InvalidResponse {
            method: "files.completeUpload",
        })
    }
    async fn drafts_list(&self, next_ts: Option<&str>, limit: usize) -> Result<RawDraftsPage> {
        let _ = (next_ts, limit);
        Err(Error::InvalidResponse {
            method: "drafts.list",
        })
    }
    async fn drafts_info(&self, draft_id: &str) -> Result<RawDraftResponse> {
        let _ = draft_id;
        Err(Error::InvalidResponse {
            method: "drafts.info",
        })
    }
    async fn drafts_create(
        &self,
        client_msg_id: &str,
        destinations: &[DraftDestination],
        blocks: &[serde_json::Value],
        file_ids: &[String],
    ) -> Result<RawDraftResponse> {
        let _ = (client_msg_id, destinations, blocks, file_ids);
        Err(Error::InvalidResponse {
            method: "drafts.create",
        })
    }
    async fn drafts_update(
        &self,
        draft_id: &str,
        last_updated_ts: &str,
        destinations: &[DraftDestination],
        blocks: &[serde_json::Value],
        file_ids: &[String],
    ) -> Result<RawDraftResponse> {
        let _ = (draft_id, last_updated_ts, destinations, blocks, file_ids);
        Err(Error::InvalidResponse {
            method: "drafts.update",
        })
    }
    async fn drafts_delete(
        &self,
        draft_id: &str,
        last_updated_ts: &str,
        skip_file_deletion: bool,
    ) -> Result<RawMutationResponse> {
        let _ = (draft_id, last_updated_ts, skip_file_deletion);
        Err(Error::InvalidResponse {
            method: "drafts.delete",
        })
    }
    async fn chat_post_message(
        &self,
        request: &ChatPostMessageRequest<'_>,
    ) -> Result<RawPostMessageResponse> {
        let _ = (
            request.channel,
            request.thread_ts,
            request.broadcast,
            request.client_msg_id,
            request.text,
            request.blocks,
        );
        Err(Error::InvalidResponse {
            method: "chat.postMessage",
        })
    }
    async fn files_share(&self, request: &FileShareRequest<'_>) -> Result<RawMutationResponse> {
        let _ = (
            request.channel,
            request.thread_ts,
            request.broadcast,
            request.client_msg_id,
            request.draft_id,
            request.blocks,
            request.file_id,
        );
        Err(Error::InvalidResponse {
            method: "files.share",
        })
    }
}

#[derive(Clone)]
pub(crate) struct SlackService {
    api: Arc<dyn SlackApi>,
    team_id: String,
    workspace_url: url::Url,
    max_response_bytes: usize,
    now_millis: fn() -> Result<String>,
    upload_reconciliation_delays_ms: &'static [u64],
    draft_reconciliation_delays_ms: &'static [u64],
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UploadPreparation {
    Ready { file_id: String },
    AllocationUncertain,
    Allocated { file_id: String },
    SourceChanged { file_id: String },
    TransferUncertain { file_id: String },
}

impl UploadPreparation {
    fn into_file_upload(self) -> std::result::Result<String, FileUploadReport> {
        match self {
            Self::Ready { file_id } => Ok(file_id),
            Self::AllocationUncertain => Err(FileUploadReport::AllocationUncertain),
            Self::Allocated { file_id } => Err(FileUploadReport::Allocated { file_id }),
            Self::SourceChanged { file_id } => Err(FileUploadReport::SourceChanged { file_id }),
            Self::TransferUncertain { file_id } => {
                Err(FileUploadReport::TransferUncertain { file_id })
            }
        }
    }

    fn into_file_draft(self) -> std::result::Result<String, FileDraftCreateReport> {
        match self {
            Self::Ready { file_id } => Ok(file_id),
            Self::AllocationUncertain => Err(FileDraftCreateReport::AllocationUncertain),
            Self::Allocated { file_id } => Err(FileDraftCreateReport::Allocated { file_id }),
            Self::SourceChanged { file_id } => {
                Err(FileDraftCreateReport::SourceChanged { file_id })
            }
            Self::TransferUncertain { file_id } => {
                Err(FileDraftCreateReport::TransferUncertain { file_id })
            }
        }
    }
}

struct ByteLimitWriter {
    bytes_written: usize,
    limit: usize,
}

impl ByteLimitWriter {
    fn new(limit: usize) -> Self {
        Self {
            bytes_written: 0,
            limit,
        }
    }
}

impl Write for ByteLimitWriter {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let Some(next_size) = self.bytes_written.checked_add(buffer.len()) else {
            return Err(io::Error::other("serialized output exceeds byte limit"));
        };
        if next_size > self.limit {
            return Err(io::Error::other("serialized output exceeds byte limit"));
        }
        self.bytes_written = next_size;
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn serialized_json_fits(value: &impl serde::Serialize, limit: usize) -> bool {
    serde_json::to_writer_pretty(ByteLimitWriter::new(limit), value).is_ok()
}

struct VerifiedFileDraft {
    draft: Draft,
    file: FileReference,
}

struct UserDirectory {
    users: HashMap<String, User>,
    conflicting_ids: HashSet<String>,
    complete: bool,
}

struct UnreadCountSnapshot {
    team_id: String,
    conversations: Vec<UnreadCount>,
    threads: UnreadThreads,
}

struct UnreadCount {
    id: String,
    kind: ConversationKind,
    has_unreads: bool,
    mention_count: u64,
    last_read: Option<String>,
    latest: Option<String>,
}

struct ResolvedUnreadName {
    name: Option<String>,
    display_name: Option<String>,
    resolution: ConversationNameResolution,
}

#[derive(Clone, Copy)]
enum DirectoryCompletion {
    Complete,
    Incomplete,
    Unavailable,
}

struct UnreadConversationDirectory {
    conversations: HashMap<String, RawConversation>,
    conflicting_ids: HashSet<String>,
    completion: DirectoryCompletion,
    user_directory: Option<UnreadUserDirectory>,
}

struct UnreadUserDirectory {
    users: HashMap<String, User>,
    conflicting_ids: HashSet<String>,
    completion: DirectoryCompletion,
}

struct MentionSelection {
    ids: Vec<String>,
    truncated: bool,
    source: MentionSource,
}

enum MentionSource {
    Canonical,
    RichText,
}

#[derive(Clone, Copy)]
enum RichTextNodeContext {
    Root,
    BlockElement,
    ListItem,
    Inline,
}

struct RichTextMentionRender<'a> {
    output: String,
    labels: Option<&'a HashMap<String, String>>,
    ids: Vec<String>,
    seen_ids: HashSet<String>,
    mentions_truncated: bool,
    render_limited: bool,
    nodes: usize,
}

struct RichTextMentionOutput {
    rendered_text: Option<String>,
    ids: Vec<String>,
    mentions_truncated: bool,
}

struct MessagePermalinks {
    permalink: Option<String>,
    thread_root_permalink: Option<String>,
    resolution: PermalinkResolution,
}

enum SearchPermalinkRoute {
    Root,
    Reply { thread_ts: String },
}

struct LoadedInboxConversations {
    conversations: HashMap<String, Conversation>,
    author_directory: Option<AuthorDirectory>,
    conversation_scan_complete: bool,
}

struct ResolvedNamedConversation {
    conversation: Conversation,
    user_directory: UserDirectory,
}

enum AuthorDirectory {
    Loaded(UserDirectory),
    Interrupted(UserDirectory),
}

enum UserDirectoryScan {
    Finished(UserDirectory),
    Interrupted {
        directory: UserDirectory,
        error: Error,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct ActivityKey {
    ts: String,
    conversation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
struct ActivityCursor {
    version: u8,
    team_id: String,
    after_nanos: i64,
    before_nanos: i64,
    order: ActivityOrder,
    conversation_kinds: Vec<ConversationKind>,
    include_ids: Vec<String>,
    exclude_ids: Vec<String>,
    conversation_limit: usize,
    per_conversation_limit: usize,
    limit: usize,
    eligible_conversations: usize,
    conversation_scan_truncated: bool,
    scope_digest: String,
    position: ActivityCursorPosition,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ActivityCursorPosition {
    Messages {
        scope_offset: usize,
        last_key: ActivityKey,
        snapshot_digest: String,
    },
    ConversationScope {
        scope_offset: usize,
    },
}

struct ActivityScope {
    candidates: Vec<ActivityConversationCandidate>,
    include_ids: Vec<String>,
    exclude_ids: Vec<String>,
    scanned_conversations: usize,
    scan_truncated: bool,
    digest: String,
}

struct ActivityConversationCandidate {
    conversation: Conversation,
}

struct ActivityConversationDirectory {
    candidates: Vec<ActivityConversationCandidate>,
    scanned_conversations: usize,
    scan_truncated: bool,
}

impl SlackService {
    pub(crate) fn new(api: impl SlackApi + 'static, config: &Config) -> Self {
        Self {
            api: Arc::new(api),
            team_id: config.team_id.clone(),
            workspace_url: config.workspace_url.clone(),
            max_response_bytes: config.max_response_bytes,
            now_millis: system_unix_milliseconds,
            upload_reconciliation_delays_ms: UPLOAD_RECONCILIATION_DELAYS_MS,
            draft_reconciliation_delays_ms: DRAFT_RECONCILIATION_DELAYS_MS,
        }
    }

    pub(crate) async fn doctor(&self) -> Result<DoctorReport> {
        self.api.client_counts().await?;
        Ok(DoctorReport {
            authenticated: true,
            team_id: self.team_id.clone(),
            workspace_url: self.workspace_url.origin().ascii_serialization(),
        })
    }

    pub(crate) async fn render_markdown(&self, source: &str) -> Result<RenderedMessage> {
        let references = outbound_mention_references(source)?;
        if references.is_empty() {
            return render_markdown(source);
        }
        let resolved = match self.scan_user_directory().await {
            UserDirectoryScan::Finished(directory) => {
                resolve_outbound_users(&references, &directory)?
            }
            UserDirectoryScan::Interrupted { directory, error } => {
                match resolve_outbound_users(&references, &directory) {
                    Ok(resolved) => resolved,
                    Err(resolution_error)
                        if interrupted_outbound_error_is_definitive(
                            &resolution_error,
                            &directory,
                        ) =>
                    {
                        return Err(resolution_error);
                    }
                    Err(_) => return Err(error),
                }
            }
        };
        render_markdown_with_mentions(source, &resolved)
    }

    pub(crate) async fn list_custom_emoji(&self) -> Result<CustomEmojiList> {
        let raw = self.api.emoji_list().await?;
        if raw.emoji.len() > MAX_CUSTOM_EMOJI {
            return Err(Error::InvalidResponse {
                method: "emoji.list",
            });
        }
        let mut emoji = raw
            .emoji
            .into_iter()
            .map(|(name, value)| normalize_custom_emoji(name, value))
            .collect::<Result<Vec<_>>>()?;
        emoji.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(CustomEmojiList { emoji })
    }

    pub(crate) async fn get_file(&self, file_id: &str) -> Result<FileReference> {
        validate_file_id(file_id)?;
        let file = normalize_file(self.api.files_info(file_id).await?.file, "files.info")?;
        if file.id != file_id {
            return Err(Error::InvalidResponse {
                method: "files.info",
            });
        }
        Ok(file)
    }

    pub(crate) async fn download_file(
        &self,
        file: FileReference,
        mut target: BoundedDownload,
        output_path: String,
    ) -> Result<FileDownloadReport> {
        if file.is_external == Some(true)
            || file.mode.as_deref().is_some_and(|mode| mode != "hosted")
        {
            return Err(Error::Unsupported {
                resource: "non-hosted Slack file downloads",
            });
        }
        if file
            .file_access
            .as_deref()
            .is_some_and(|access| access != "visible")
        {
            return Err(Error::Authorization {
                resource: "the requested Slack file",
            });
        }
        let expected_size = file.size.ok_or(Error::NotFound {
            resource: "Slack file size",
        })?;
        if expected_size > MAX_FILE_DOWNLOAD_BYTES {
            return Err(Error::invalid_input(
                "max_bytes",
                "Slack file is larger than the 1 GiB hard limit",
            ));
        }
        let download_url = file.download_url.as_deref().ok_or(Error::NotFound {
            resource: "Slack file download",
        })?;
        self.api
            .download_private_file(
                download_url,
                expected_size,
                file.mimetype.as_deref(),
                &mut target,
            )
            .await?;
        if target.bytes_written() != expected_size {
            return Err(Error::FileDownloadSizeMismatch {
                expected: expected_size,
                actual: target.bytes_written(),
            });
        }
        let commit = target.commit()?;
        Ok(FileDownloadReport {
            file,
            output_path,
            bytes_written: commit.bytes_written,
            durability_warning: (commit.durability == DownloadDurability::DirectorySyncWarning)
                .then(|| "file committed, but the parent directory could not be synced".into()),
        })
    }

    async fn prepare_upload(
        &self,
        source: &mut UploadSource,
        alt_text: Option<&str>,
    ) -> Result<UploadPreparation> {
        if source.size() > MAX_FILE_UPLOAD_BYTES {
            return Err(Error::invalid_input(
                "max_bytes",
                "Slack file is larger than the 1 GiB hard limit",
            ));
        }
        let raw_allocation = match self
            .api
            .files_get_upload_url(source.file_name(), source.size(), alt_text)
            .await
        {
            Ok(allocation) => allocation,
            Err(error) if mutation_error_is_ambiguous(&error) => {
                return Ok(UploadPreparation::AllocationUncertain);
            }
            Err(error) => return Err(error),
        };
        let Some(file_id) = raw_allocation
            .file_id
            .filter(|file_id| is_valid_file_id(file_id))
        else {
            return Ok(UploadPreparation::AllocationUncertain);
        };
        let Some(raw_upload_url) = raw_allocation.upload_url else {
            return Ok(UploadPreparation::Allocated { file_id });
        };
        let upload_url = Zeroizing::new(raw_upload_url);
        if !is_safe_upload_url(&upload_url) {
            return Ok(UploadPreparation::Allocated { file_id });
        }
        if !matches!(source.is_stable(), Ok(true)) {
            return Ok(UploadPreparation::SourceChanged { file_id });
        }
        let upload_pass = match self.api.upload_edge_file(&upload_url, source).await {
            Ok(upload_pass) => upload_pass,
            Err(_) => return Ok(UploadPreparation::TransferUncertain { file_id }),
        };
        if !matches!(source.upload_pass_matches(&upload_pass), Ok(true)) {
            return Ok(UploadPreparation::SourceChanged { file_id });
        }
        Ok(UploadPreparation::Ready { file_id })
    }

    pub(crate) async fn upload_file(
        &self,
        conversation: &str,
        thread_ts: Option<&str>,
        title: Option<&str>,
        alt_text: Option<&str>,
        mut source: UploadSource,
        confirm: bool,
    ) -> Result<FileUploadReport> {
        if !confirm {
            return Err(Error::ConfirmationRequired {
                action: "file upload",
            });
        }
        Self::validate_upload_request(
            conversation,
            thread_ts,
            title,
            alt_text,
            source.file_name(),
        )?;
        let channel_id = self.resolve_conversation_id(conversation).await?;
        if let Some(thread_ts) = thread_ts {
            let root = self.get_message_by_id(&channel_id, thread_ts).await?;
            if !is_exact_file_route(&root.ts, root.thread_ts.as_deref(), None) {
                return Err(Error::invalid_input(
                    "thread_ts",
                    "must identify a root message in the target conversation",
                ));
            }
        }
        let file_id = match self
            .prepare_upload(&mut source, alt_text)
            .await?
            .into_file_upload()
        {
            Ok(file_id) => file_id,
            Err(report) => return Ok(report),
        };

        let completion_client_msg_id = Uuid::new_v4().to_string();
        let completion = self
            .api
            .files_complete_upload(
                &file_id,
                title,
                &channel_id,
                thread_ts,
                &completion_client_msg_id,
            )
            .await;
        let completion_acknowledged = completion
            .as_ref()
            .is_ok_and(|completion| completion_has_exact_file(completion, &file_id));
        let reconciled = !completion_acknowledged;

        for delay_ms in self.upload_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let file = match self.api.files_info(&file_id).await {
                Ok(raw) if raw.file.id == file_id => {
                    let Ok(file) = normalize_file(raw.file, "files.info") else {
                        continue;
                    };
                    file
                }
                _ => continue,
            };
            if alt_text.is_some_and(|expected| file.alt_text.as_deref() != Some(expected)) {
                continue;
            }
            let share = if channel_id.starts_with('D') {
                if !file
                    .im_ids
                    .as_ref()
                    .is_some_and(|im_ids| im_ids.iter().any(|im_id| im_id == &channel_id))
                {
                    None
                } else {
                    self.find_direct_message_file_share(&channel_id, thread_ts, &file_id)
                        .await
                }
            } else {
                exact_file_share(&file, &channel_id, thread_ts)
            };
            if let Some(share) = share {
                return Ok(FileUploadReport::Shared {
                    file: Box::new(file),
                    share,
                    reconciled,
                });
            }
        }
        Ok(FileUploadReport::CompletionUncertain { file_id })
    }

    async fn find_direct_message_file_share(
        &self,
        channel_id: &str,
        thread_ts: Option<&str>,
        file_id: &str,
    ) -> Option<FileShare> {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        let mut match_ts = None;

        for _ in 0..MAX_FILE_SHARE_SCAN_PAGES {
            let raw = match thread_ts {
                Some(thread_ts) => {
                    self.api
                        .conversation_replies(
                            channel_id,
                            thread_ts,
                            cursor.as_deref(),
                            MAX_MESSAGES,
                        )
                        .await
                }
                None => {
                    self.api
                        .conversation_history(channel_id, cursor.as_deref(), MAX_MESSAGES)
                        .await
                }
            }
            .ok()?;
            if raw.messages.len() > MAX_MESSAGES {
                return None;
            }
            let method = if thread_ts.is_some() {
                "conversations.replies"
            } else {
                "conversations.history"
            };
            let messages = normalize_messages(
                &self.workspace_url,
                channel_id,
                raw.messages,
                MAX_MESSAGES,
                method,
            )
            .ok()?;
            for message in messages {
                let exact_route =
                    is_exact_file_route(&message.ts, message.thread_ts.as_deref(), thread_ts);
                let occurrences = message
                    .files
                    .iter()
                    .filter(|file| file.id == file_id)
                    .count();
                if occurrences > 1 || (occurrences == 1 && !exact_route) {
                    return None;
                }
                if occurrences == 1 && match_ts.replace(message.ts).is_some() {
                    return None;
                }
            }

            let next_cursor = response_cursor(method, raw.response_metadata.next_cursor).ok()?;
            let has_more = raw.has_more || next_cursor.is_some();
            if !has_more {
                return match_ts.map(|ts| FileShare {
                    visibility: FileShareVisibility::Private,
                    channel_id: channel_id.to_owned(),
                    ts,
                    thread_ts: thread_ts.map(str::to_owned),
                });
            }
            let next_cursor = next_cursor?;
            if cursor.as_deref() == Some(next_cursor.as_str())
                || !seen_cursors.insert(next_cursor.clone())
            {
                return None;
            }
            cursor = Some(next_cursor);
        }
        None
    }

    async fn complete_active_draft_snapshot(&self) -> Result<Vec<Draft>> {
        let mut cursor = None;
        let mut seen_cursors = HashSet::new();
        let mut seen_draft_ids = HashSet::new();
        let mut drafts = Vec::new();

        for page_index in 0..MAX_DRAFT_OWNERSHIP_SCAN_PAGES {
            let raw = self.api.drafts_list(cursor.as_deref(), MAX_DRAFTS).await?;
            if raw.drafts.len() > MAX_DRAFTS || raw.files.len() > 1_000 {
                return Err(Error::InvalidResponse {
                    method: "drafts.list",
                });
            }
            let page = raw
                .drafts
                .into_iter()
                .map(|draft| normalize_draft(draft, "drafts.list"))
                .collect::<Result<Vec<_>>>()?;
            if page
                .iter()
                .any(|draft| !seen_draft_ids.insert(draft.id.clone()))
            {
                return Err(Error::InvalidResponse {
                    method: "drafts.list",
                });
            }
            let next_cursor = raw
                .has_more
                .then(|| {
                    page.last()
                        .map(|draft| draft.last_updated_ts.clone())
                        .ok_or(Error::InvalidResponse {
                            method: "drafts.list",
                        })
                })
                .transpose()?;
            drafts.extend(page);
            let Some(next_cursor) = next_cursor else {
                return Ok(drafts);
            };
            if page_index + 1 == MAX_DRAFT_OWNERSHIP_SCAN_PAGES {
                return Err(Error::ScanLimit {
                    resource: "complete active Slack draft ownership",
                    limit: MAX_DRAFT_OWNERSHIP_SCAN_PAGES * MAX_DRAFTS,
                });
            }
            if cursor.as_deref() == Some(next_cursor.as_str())
                || !seen_cursors.insert(next_cursor.clone())
            {
                return Err(Error::InvalidResponse {
                    method: "drafts.list",
                });
            }
            cursor = Some(next_cursor);
        }
        Err(Error::ScanLimit {
            resource: "complete active Slack draft ownership",
            limit: MAX_DRAFT_OWNERSHIP_SCAN_PAGES * MAX_DRAFTS,
        })
    }

    async fn prove_exclusive_file_draft(
        &self,
        file_id: &str,
        client_msg_id: &str,
        expected: Option<&Draft>,
        expected_destination: Option<&DraftDestination>,
        required_blocks: Option<&[serde_json::Value]>,
    ) -> Result<VerifiedFileDraft> {
        validate_file_id(file_id)?;
        let drafts = self.complete_active_draft_snapshot().await?;
        let occurrences = drafts
            .iter()
            .flat_map(|draft| draft.file_ids.iter())
            .filter(|candidate| candidate.as_str() == file_id)
            .count();
        if occurrences != 1 {
            return Err(Error::invalid_input(
                "draft",
                "does not exclusively own its Slack file",
            ));
        }
        let snapshot = drafts
            .into_iter()
            .find(|draft| {
                draft.client_msg_id.as_deref() == Some(client_msg_id)
                    && draft.file_ids.len() == 1
                    && draft.file_ids[0] == file_id
            })
            .ok_or_else(|| {
                Error::invalid_input(
                    "draft",
                    "does not have a complete exclusive private-file association",
                )
            })?;
        if !snapshot.file_shape_supported
            || snapshot.team_id.as_deref() != Some(self.team_id.as_str())
            || expected_destination.is_some_and(|destination| {
                snapshot.destinations.len() != 1
                    || !same_draft_route(&snapshot.destinations[0], destination)
            })
            || expected.is_some_and(|expected| !same_draft_snapshot(&snapshot, expected))
            || required_blocks.is_some_and(|blocks| {
                !snapshot
                    .blocks
                    .as_deref()
                    .is_some_and(|actual| same_rendered_draft_blocks(actual, blocks))
            })
        {
            return Err(Error::invalid_input(
                "draft",
                "changed route, revision, content, or file association",
            ));
        }

        let info = normalize_draft(
            self.api.drafts_info(&snapshot.id).await?.draft,
            "drafts.info",
        )?;
        if !same_draft_snapshot(&info, &snapshot) || !info.file_shape_supported {
            return Err(Error::invalid_input(
                "draft",
                "does not match its complete active-draft snapshot",
            ));
        }
        let raw_file = self.api.files_info(file_id).await?;
        if raw_file.file.id != file_id {
            return Err(Error::InvalidResponse {
                method: "files.info",
            });
        }
        let file = normalize_file(raw_file.file, "files.info")?;
        if !is_private_unshared_file(&file) {
            return Err(Error::invalid_input(
                "draft",
                "file is shared, public, or has incomplete ownership state",
            ));
        }
        let mut draft = info;
        draft.file_association = Some(FileDraftAssociation::Verified);
        draft.is_supported = true;
        Ok(VerifiedFileDraft { draft, file })
    }

    async fn reconcile_draft_absence(&self, draft_id: &str) -> bool {
        for delay_ms in self.draft_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let Ok(drafts) = self.complete_active_draft_snapshot().await else {
                continue;
            };
            if drafts.iter().all(|draft| draft.id != draft_id) {
                return true;
            }
        }
        false
    }

    async fn reconcile_text_draft_creation(
        &self,
        client_msg_id: &str,
        destination: &DraftDestination,
        blocks: &[serde_json::Value],
    ) -> Option<Draft> {
        for delay_ms in self.draft_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let Ok(drafts) = self.complete_active_draft_snapshot().await else {
                continue;
            };
            let mut matches = drafts
                .into_iter()
                .filter(|draft| is_exact_text_draft(draft, client_msg_id, destination, blocks));
            let Some(draft) = matches.next() else {
                continue;
            };
            if matches.next().is_none() {
                return Some(draft);
            }
        }
        None
    }

    pub(crate) fn validate_upload_request(
        conversation: &str,
        thread_ts: Option<&str>,
        title: Option<&str>,
        alt_text: Option<&str>,
        file_name: &str,
    ) -> Result<()> {
        validate_conversation_reference(conversation)?;
        validate_upload_input("filename", file_name, MAX_FILE_UPLOAD_NAME_BYTES)?;
        if let Some(title) = title {
            validate_upload_input("title", title, MAX_FILE_UPLOAD_TITLE_BYTES)?;
        }
        if let Some(alt_text) = alt_text {
            validate_upload_input("alt_text", alt_text, MAX_FILE_UPLOAD_ALT_TEXT_BYTES)?;
        }
        if let Some(thread_ts) = thread_ts {
            validate_timestamp("thread_ts", thread_ts)?;
        }
        Ok(())
    }

    pub(crate) fn validate_file_draft_request(
        conversation: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        title: Option<&str>,
        alt_text: Option<&str>,
        file_name: &str,
    ) -> Result<()> {
        validate_draft_destination(thread_ts, broadcast)?;
        Self::validate_upload_request(conversation, thread_ts, title, alt_text, file_name)
    }

    pub(crate) async fn add_reaction(
        &self,
        conversation: &str,
        message_ts: &str,
        name: &str,
        confirmed: bool,
    ) -> Result<ReactionMutationReport> {
        self.set_reaction(conversation, message_ts, name, true, confirmed)
            .await
    }

    pub(crate) async fn remove_reaction(
        &self,
        conversation: &str,
        message_ts: &str,
        name: &str,
        confirmed: bool,
    ) -> Result<ReactionMutationReport> {
        self.set_reaction(conversation, message_ts, name, false, confirmed)
            .await
    }

    async fn set_reaction(
        &self,
        conversation: &str,
        message_ts: &str,
        name: &str,
        target_present: bool,
        confirmed: bool,
    ) -> Result<ReactionMutationReport> {
        require_confirmation("reaction mutation", confirmed)?;
        validate_timestamp("message_ts", message_ts)?;
        let name = validate_reaction_name(name)?;
        let channel_id = self.resolve_conversation_id(conversation).await?;
        let user_id = self.api.auth_test().await?.user_id;
        if !is_valid_user_id(&user_id) {
            return Err(Error::InvalidResponse {
                method: "auth.test",
            });
        }
        let before = self
            .reaction_is_present(&channel_id, message_ts, &name, &user_id)
            .await?;
        if before == target_present {
            return Ok(ReactionMutationReport {
                channel_id,
                message_ts: message_ts.to_owned(),
                name,
                target_present,
                present: target_present,
                changed: false,
                reconciled: false,
            });
        }

        let mutation = if target_present {
            self.api.reactions_add(&channel_id, message_ts, &name).await
        } else {
            self.api
                .reactions_remove(&channel_id, message_ts, &name)
                .await
        };
        let ambiguous_mutation = match mutation {
            Ok(_) => false,
            Err(Error::SlackApi { method, code })
                if (method == "reactions.add" && code == "already_reacted")
                    || (method == "reactions.remove" && code == "no_reaction") =>
            {
                return Ok(ReactionMutationReport {
                    channel_id,
                    message_ts: message_ts.to_owned(),
                    name,
                    target_present,
                    present: target_present,
                    changed: false,
                    reconciled: true,
                });
            }
            Err(error) if mutation_error_is_ambiguous(&error) => true,
            Err(error) => return Err(error),
        };

        match self
            .reaction_is_present(&channel_id, message_ts, &name, &user_id)
            .await
        {
            Ok(present) if present == target_present => Ok(ReactionMutationReport {
                channel_id,
                message_ts: message_ts.to_owned(),
                name,
                target_present,
                present,
                changed: true,
                reconciled: ambiguous_mutation,
            }),
            Ok(_) => Err(Error::ReactionNotApplied {
                channel_id,
                message_ts: message_ts.to_owned(),
                name,
            }),
            Err(_) => Err(Error::ReactionUncertain {
                channel_id,
                message_ts: message_ts.to_owned(),
                name,
            }),
        }
    }

    async fn reaction_is_present(
        &self,
        channel_id: &str,
        message_ts: &str,
        name: &str,
        user_id: &str,
    ) -> Result<bool> {
        let response = self.api.reactions_get(channel_id, message_ts).await?;
        if response.item_type != "message" || response.channel.as_deref() != Some(channel_id) {
            return Err(Error::InvalidResponse {
                method: "reactions.get",
            });
        }
        let message = response.message.ok_or(Error::InvalidResponse {
            method: "reactions.get",
        })?;
        if message.ts != message_ts {
            return Err(Error::InvalidResponse {
                method: "reactions.get",
            });
        }
        let normalized =
            normalize_message(&self.workspace_url, channel_id, message, "reactions.get")?;
        Ok(normalized
            .reactions
            .iter()
            .find(|reaction| reaction.name == name)
            .is_some_and(|reaction| reaction.user_ids.iter().any(|id| id == user_id)))
    }

    pub(crate) async fn list_drafts(
        &self,
        next_ts: Option<&str>,
        limit: usize,
    ) -> Result<DraftPage> {
        validate_limit("limit", limit, MAX_DRAFTS)?;
        validate_draft_revision_input("next_ts", next_ts)?;
        let raw = self.api.drafts_list(next_ts, limit).await?;
        if raw.drafts.len() > limit || raw.files.len() > 1_000 {
            return Err(Error::InvalidResponse {
                method: "drafts.list",
            });
        }
        let drafts = raw
            .drafts
            .into_iter()
            .map(|draft| normalize_draft(draft, "drafts.list"))
            .collect::<Result<Vec<_>>>()?;
        let continuation = raw
            .has_more
            .then(|| {
                drafts
                    .last()
                    .map(|draft| draft.last_updated_ts.clone())
                    .ok_or(Error::InvalidResponse {
                        method: "drafts.list",
                    })
            })
            .transpose()?;
        if continuation
            .as_deref()
            .zip(next_ts)
            .is_some_and(|(continuation, current)| continuation == current)
        {
            return Err(Error::InvalidResponse {
                method: "drafts.list",
            });
        }
        Ok(DraftPage {
            drafts,
            has_more: raw.has_more,
            next_ts: continuation,
        })
    }

    pub(crate) async fn get_draft(&self, draft_id: &str) -> Result<Draft> {
        validate_draft_id(draft_id)?;
        let draft = normalize_draft(self.api.drafts_info(draft_id).await?.draft, "drafts.info")?;
        if draft.id != draft_id {
            return Err(Error::InvalidResponse {
                method: "drafts.info",
            });
        }
        if draft.file_shape_supported
            && let (Some(client_msg_id), Some(file_id), Some(destination)) = (
                draft.client_msg_id.as_deref(),
                draft.file_ids.first(),
                draft.destinations.first(),
            )
            && let Ok(proof) = self
                .prove_exclusive_file_draft(
                    file_id,
                    client_msg_id,
                    Some(&draft),
                    Some(destination),
                    None,
                )
                .await
        {
            return Ok(proof.draft);
        }
        Ok(draft)
    }

    pub(crate) async fn create_draft(
        &self,
        conversation: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        markdown: &str,
    ) -> Result<Draft> {
        let rendered = self.render_markdown(markdown).await?;
        validate_draft_destination(thread_ts, broadcast)?;
        let channel_id = self.resolve_conversation_id(conversation).await?;
        let destination = DraftDestination {
            channel_id: Some(channel_id),
            thread_ts: thread_ts.map(str::to_owned),
            broadcast,
            ..DraftDestination::default()
        };
        let client_msg_id = Uuid::new_v4().to_string();
        let response = self
            .api
            .drafts_create(
                &client_msg_id,
                std::slice::from_ref(&destination),
                &rendered.blocks,
                &[],
            )
            .await;
        match response {
            Ok(response) => {
                if let Ok(draft) = normalize_draft(response.draft, "drafts.create")
                    && is_exact_text_draft(&draft, &client_msg_id, &destination, &rendered.blocks)
                {
                    return Ok(draft);
                }
            }
            Err(error) if mutation_error_is_ambiguous(&error) => {}
            Err(error) => return Err(error),
        }
        if let Some(draft) = self
            .reconcile_text_draft_creation(&client_msg_id, &destination, &rendered.blocks)
            .await
        {
            return Ok(draft);
        }
        Err(Error::DraftCreationUncertain { client_msg_id })
    }

    pub(crate) async fn create_file_draft(
        &self,
        request: FileDraftCreateRequest<'_>,
        mut source: UploadSource,
    ) -> Result<FileDraftCreateReport> {
        let FileDraftCreateRequest {
            conversation,
            thread_ts,
            broadcast,
            markdown,
            title,
            alt_text,
            confirmed,
        } = request;
        require_confirmation("file draft creation", confirmed)?;
        let rendered = self.render_markdown(markdown).await?;
        Self::validate_file_draft_request(
            conversation,
            thread_ts,
            broadcast,
            title,
            alt_text,
            source.file_name(),
        )?;
        let expected_file_name = source.file_name().to_owned();
        let expected_file_size = source.size();
        let channel_id = self.resolve_conversation_id(conversation).await?;
        if let Some(thread_ts) = thread_ts {
            let root = self.get_message_by_id(&channel_id, thread_ts).await?;
            if !is_exact_file_route(&root.ts, root.thread_ts.as_deref(), None) {
                return Err(Error::invalid_input(
                    "thread_ts",
                    "must identify a root message in the target conversation",
                ));
            }
        }
        let destination = DraftDestination {
            channel_id: Some(channel_id),
            thread_ts: thread_ts.map(str::to_owned),
            broadcast,
            ..DraftDestination::default()
        };

        let file_id = match self
            .prepare_upload(&mut source, alt_text)
            .await?
            .into_file_draft()
        {
            Ok(file_id) => file_id,
            Err(report) => return Ok(report),
        };

        let completion = self.api.files_complete_draft_upload(&file_id, title).await;
        if !completion
            .as_ref()
            .is_ok_and(|completion| completion_has_exact_file(completion, &file_id))
        {
            return Ok(FileDraftCreateReport::FileCompletionUncertain { file_id });
        }
        let mut private_file_ready = false;
        for delay_ms in self.upload_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let Ok(raw) = self.api.files_info(&file_id).await else {
                continue;
            };
            if raw.file.id != file_id {
                continue;
            }
            let Ok(file) = normalize_file(raw.file, "files.info") else {
                continue;
            };
            if is_expected_private_draft_file(
                &file,
                &file_id,
                &expected_file_name,
                expected_file_size,
                alt_text,
            ) {
                private_file_ready = true;
                break;
            }
        }
        if !private_file_ready {
            return Ok(FileDraftCreateReport::FileCompletionUncertain { file_id });
        }

        let client_msg_id = Uuid::new_v4().to_string();
        let file_ids = vec![file_id.clone()];
        let response = self
            .api
            .drafts_create(
                &client_msg_id,
                std::slice::from_ref(&destination),
                &rendered.blocks,
                &file_ids,
            )
            .await;
        let acknowledged = match response {
            Ok(response) => normalize_draft(response.draft, "drafts.create")
                .ok()
                .filter(|draft| {
                    draft.client_msg_id.as_deref() == Some(client_msg_id.as_str())
                        && draft.file_ids == file_ids
                        && draft.destinations.len() == 1
                        && same_draft_route(&draft.destinations[0], &destination)
                        && draft.blocks.as_deref().is_some_and(|actual| {
                            same_rendered_draft_blocks(actual, &rendered.blocks)
                        })
                }),
            Err(error) if mutation_error_is_ambiguous(&error) => None,
            Err(error) => {
                return Ok(FileDraftCreateReport::DraftNotCreated {
                    file_id,
                    reason: error.to_string(),
                });
            }
        };
        for delay_ms in self.draft_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let proof = self
                .prove_exclusive_file_draft(
                    &file_id,
                    &client_msg_id,
                    None,
                    Some(&destination),
                    Some(&rendered.blocks),
                )
                .await;
            if let Ok(proof) = proof {
                if !is_expected_private_draft_file(
                    &proof.file,
                    &file_id,
                    &expected_file_name,
                    expected_file_size,
                    alt_text,
                ) {
                    continue;
                }
                let reconciled = acknowledged
                    .as_ref()
                    .is_none_or(|acknowledged| !same_draft_snapshot(&proof.draft, acknowledged));
                return Ok(FileDraftCreateReport::Created {
                    draft: Box::new(proof.draft),
                    file: Box::new(proof.file),
                    reconciled,
                });
            }
        }
        Ok(FileDraftCreateReport::DraftCreationUncertain {
            file_id,
            client_msg_id,
        })
    }

    pub(crate) async fn update_draft(&self, draft_id: &str, markdown: &str) -> Result<Draft> {
        validate_draft_id(draft_id)?;
        let rendered = self.render_markdown(markdown).await?;
        let current = self.get_draft(draft_id).await?;
        require_supported_draft(&current)?;
        let current_destination = current.destinations.first().ok_or(Error::InvalidResponse {
            method: "drafts.info",
        })?;
        let mutation_destination = draft_mutation_destination(current_destination);
        let client_last_updated_ts = (self.now_millis)()?;
        let response = self
            .api
            .drafts_update(
                &current.id,
                &client_last_updated_ts,
                std::slice::from_ref(&mutation_destination),
                &rendered.blocks,
                &current.file_ids,
            )
            .await;
        match response {
            Ok(_) => {}
            Err(error) if mutation_error_is_ambiguous(&error) => {}
            Err(error) => return Err(error),
        }
        for delay_ms in self.draft_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            if let Ok(updated) = self.get_draft(&current.id).await
                && updated.is_supported
                && updated.id == current.id
                && updated.client_msg_id == current.client_msg_id
                && updated.last_updated_ts != current.last_updated_ts
                && updated.destinations == current.destinations
                && updated.file_ids == current.file_ids
                && updated
                    .blocks
                    .as_deref()
                    .is_some_and(|actual| same_rendered_draft_blocks(actual, &rendered.blocks))
            {
                return Ok(updated);
            }
        }
        Err(Error::DraftMutationUncertain {
            draft_id: current.id,
            action: "update",
        })
    }

    pub(crate) async fn delete_draft(
        &self,
        draft_id: &str,
        confirmed: bool,
    ) -> Result<DraftDeleteReport> {
        require_confirmation("draft deletion", confirmed)?;
        validate_draft_id(draft_id)?;
        let current = self.get_draft(draft_id).await?;
        require_supported_draft(&current)?;
        let file_id = current.file_ids.first().cloned();
        let delete = self
            .api
            .drafts_delete(
                &current.id,
                &current.client_last_updated_ts,
                file_id.is_some(),
            )
            .await;
        match delete {
            Ok(_) => {
                return Ok(DraftDeleteReport {
                    id: current.id,
                    deleted: true,
                    file_deleted: file_id.as_ref().map(|_| false),
                    file_id,
                });
            }
            Err(error) if mutation_error_is_ambiguous(&error) => {}
            Err(error) => return Err(error),
        }
        if self.reconcile_draft_absence(&current.id).await {
            return Ok(DraftDeleteReport {
                id: current.id,
                deleted: true,
                file_deleted: file_id.as_ref().map(|_| false),
                file_id,
            });
        }
        Err(Error::DraftMutationUncertain {
            draft_id: current.id,
            action: "delete",
        })
    }

    pub(crate) async fn send_message(
        &self,
        conversation: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        markdown: &str,
        confirmed: bool,
    ) -> Result<SentMessage> {
        require_confirmation("message publication", confirmed)?;
        validate_draft_destination(thread_ts, broadcast)?;
        let rendered = self.render_markdown(markdown).await?;
        let channel_id = self.resolve_conversation_id(conversation).await?;
        self.post_rich_message(
            &channel_id,
            thread_ts,
            broadcast,
            &rendered.text,
            &rendered.blocks,
        )
        .await
    }

    pub(crate) async fn send_draft(
        &self,
        draft_id: &str,
        confirmed: bool,
    ) -> Result<DraftSendReport> {
        require_confirmation("draft publication", confirmed)?;
        validate_draft_id(draft_id)?;
        let draft = self.get_draft(draft_id).await?;
        require_supported_draft(&draft)?;
        let destination = draft.destinations.first().ok_or(Error::InvalidResponse {
            method: "drafts.info",
        })?;
        let channel_id = destination
            .channel_id
            .as_deref()
            .ok_or(Error::InvalidResponse {
                method: "drafts.info",
            })?;
        let blocks = draft.blocks.as_deref().ok_or(Error::InvalidResponse {
            method: "drafts.info",
        })?;
        let fallback = if is_valid_message_fallback(&draft.text) {
            draft.text.clone()
        } else {
            rich_text_fallback(blocks).ok_or_else(|| {
                Error::invalid_input(
                    "draft",
                    "does not contain publishable bounded Slack rich text",
                )
            })?
        };
        let authored_blocks = authored_draft_blocks(blocks).ok_or(Error::InvalidResponse {
            method: "drafts.info",
        })?;
        let sent = if let Some(file_id) = draft.file_ids.first() {
            let client_msg_id = Uuid::new_v4().to_string();
            let request = FileShareRequest {
                channel: channel_id,
                thread_ts: destination.thread_ts.as_deref(),
                broadcast: destination.broadcast,
                client_msg_id: &client_msg_id,
                draft_id: &draft.id,
                blocks: &authored_blocks,
                file_id,
            };
            self.share_file_draft(&request).await?
        } else {
            self.post_rich_message(
                channel_id,
                destination.thread_ts.as_deref(),
                destination.broadcast,
                &fallback,
                &authored_blocks,
            )
            .await?
        };

        if !draft.file_ids.is_empty()
            && self
                .complete_active_draft_snapshot()
                .await
                .is_ok_and(|drafts| drafts.iter().all(|candidate| candidate.id != draft.id))
        {
            return Ok(DraftSendReport {
                sent,
                draft_id: draft.id,
                draft_deleted: true,
                cleanup_warning: None,
            });
        }

        let cleanup = self
            .api
            .drafts_delete(
                &draft.id,
                &draft.client_last_updated_ts,
                !draft.file_ids.is_empty(),
            )
            .await;
        match cleanup {
            Ok(_) => Ok(DraftSendReport {
                sent,
                draft_id: draft.id,
                draft_deleted: true,
                cleanup_warning: None,
            }),
            Err(error) => {
                if self.reconcile_draft_absence(&draft.id).await {
                    return Ok(DraftSendReport {
                        sent,
                        draft_id: draft.id,
                        draft_deleted: true,
                        cleanup_warning: None,
                    });
                }
                let reason = if mutation_error_is_ambiguous(&error) {
                    Error::DraftMutationUncertain {
                        draft_id: draft.id.clone(),
                        action: "post-publication cleanup",
                    }
                    .to_string()
                } else {
                    error.to_string()
                };
                Ok(DraftSendReport {
                    sent,
                    draft_id: draft.id.clone(),
                    draft_deleted: false,
                    cleanup_warning: Some(DraftCleanupWarning {
                        draft_id: draft.id.clone(),
                        last_updated_ts: draft.last_updated_ts,
                        reason,
                    }),
                })
            }
        }
    }

    async fn post_rich_message(
        &self,
        channel_id: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        text: &str,
        blocks: &[serde_json::Value],
    ) -> Result<SentMessage> {
        validate_draft_destination(thread_ts, broadcast)?;
        if !is_valid_message_fallback(text) || blocks.is_empty() {
            return Err(Error::invalid_input(
                "message",
                "must include bounded non-empty text and rich-text blocks",
            ));
        }
        let client_msg_id = Uuid::new_v4().to_string();
        let request = ChatPostMessageRequest {
            channel: channel_id,
            thread_ts,
            broadcast,
            client_msg_id: &client_msg_id,
            text,
            blocks,
        };
        let response = self
            .api
            .chat_post_message(&request)
            .await
            .map_err(|error| classify_publication_error(&client_msg_id, error))?;
        normalize_sent_message(
            &self.workspace_url,
            channel_id,
            thread_ts,
            client_msg_id.clone(),
            response,
        )
        .map_err(|_| Error::PublicationUncertain { client_msg_id })
    }

    async fn share_file_draft(&self, request: &FileShareRequest<'_>) -> Result<SentMessage> {
        match self.api.files_share(request).await {
            Ok(_) => {}
            Err(error) => match classify_publication_error(request.client_msg_id, error) {
                Error::PublicationUncertain { .. } => {}
                definitive => return Err(definitive),
            },
        }
        for delay_ms in self.draft_reconciliation_delays_ms {
            if *delay_ms != 0 {
                tokio::time::sleep(Duration::from_millis(*delay_ms)).await;
            }
            let Ok(raw) = self.api.files_info(request.file_id).await else {
                continue;
            };
            if raw.file.id != request.file_id {
                continue;
            }
            let Ok(file) = normalize_file(raw.file, "files.info") else {
                continue;
            };
            let share = if request.channel.starts_with('D') {
                match exact_file_share(&file, request.channel, request.thread_ts) {
                    Some(share) => Some(share),
                    None if file.shares_complete
                        && file.shares.as_ref().is_some_and(Vec::is_empty)
                        && file
                            .im_ids
                            .as_ref()
                            .is_some_and(|ids| ids.len() == 1 && ids[0] == request.channel) =>
                    {
                        self.find_direct_message_file_share(
                            request.channel,
                            request.thread_ts,
                            request.file_id,
                        )
                        .await
                    }
                    None => None,
                }
            } else {
                exact_file_share(&file, request.channel, request.thread_ts)
            };
            let Some(share) = share else {
                continue;
            };
            if !is_valid_timestamp(&share.ts) {
                continue;
            }
            let Ok(message) = self.get_message_by_id(request.channel, &share.ts).await else {
                continue;
            };
            if !is_exact_file_route(&message.ts, message.thread_ts.as_deref(), request.thread_ts)
                || message.files.len() != 1
                || message.files[0].id != request.file_id
                || !message
                    .blocks
                    .as_deref()
                    .is_some_and(|actual| same_rendered_draft_blocks(actual, request.blocks))
            {
                continue;
            }
            if is_exact_published_file(&file, request.channel, &message.ts, request.thread_ts) {
                return Ok(SentMessage {
                    client_msg_id: request.client_msg_id.to_owned(),
                    message,
                });
            }
        }
        Err(Error::PublicationUncertain {
            client_msg_id: request.client_msg_id.to_owned(),
        })
    }

    pub(crate) async fn unreads(&self) -> Result<UnreadReport> {
        let counts = self.unread_counts().await?;
        let mut names = self
            .resolve_unread_conversation_names(&counts.conversations)
            .await;
        Ok(UnreadReport {
            team_id: counts.team_id,
            conversations: counts
                .conversations
                .into_iter()
                .map(|unread| {
                    let name = names
                        .remove(&unread.id)
                        .unwrap_or_else(unavailable_unread_name);
                    named_unread(unread, name)
                })
                .collect(),
            threads: counts.threads,
        })
    }

    async fn unread_counts(&self) -> Result<UnreadCountSnapshot> {
        let counts = self.api.client_counts().await?;
        if counts
            .threads
            .unread_count_by_channel
            .keys()
            .any(|id| !is_valid_any_conversation_id(id))
        {
            return Err(Error::InvalidResponse {
                method: "client.counts",
            });
        }
        let mut conversations = Vec::new();
        let mut seen_ids = HashSet::new();
        append_unreads(
            &mut conversations,
            &mut seen_ids,
            counts.channels,
            ConversationKind::Channel,
        )?;
        append_unreads(
            &mut conversations,
            &mut seen_ids,
            counts.ims,
            ConversationKind::DirectMessage,
        )?;
        append_unreads(
            &mut conversations,
            &mut seen_ids,
            counts.mpims,
            ConversationKind::GroupDirectMessage,
        )?;
        conversations.sort_by(|left, right| {
            right
                .mention_count
                .cmp(&left.mention_count)
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(UnreadCountSnapshot {
            team_id: self.team_id.clone(),
            conversations,
            threads: UnreadThreads {
                has_unreads: counts.threads.has_unreads,
                mention_count: counts.threads.mention_count,
                unread_count_by_channel: counts.threads.unread_count_by_channel,
            },
        })
    }

    pub(crate) async fn inbox(
        &self,
        conversation_limit: usize,
        message_limit: usize,
    ) -> Result<InboxReport> {
        validate_limit(
            "conversation_limit",
            conversation_limit,
            MAX_INBOX_CONVERSATIONS,
        )?;
        validate_limit("message_limit", message_limit, MAX_MESSAGES)?;
        let unreads = self.unread_counts().await?;
        let total_unread_conversations = unreads.conversations.len();
        let selected = unreads
            .conversations
            .into_iter()
            .take(conversation_limit)
            .collect::<Vec<_>>();
        let selected_count = selected.len();
        let selected_ids = selected
            .iter()
            .map(|unread| unread.id.clone())
            .collect::<HashSet<_>>();
        let mut loaded = self.load_conversations_by_id(&selected_ids).await?;
        let mut report = InboxReport {
            team_id: self.team_id.clone(),
            conversations: Vec::with_capacity(selected.len()),
            total_unread_conversations,
            has_more_conversations: total_unread_conversations > 0,
            truncation_reason: (total_unread_conversations > 0)
                .then_some(InboxTruncationReason::ByteLimit),
            threads: unreads.threads,
        };
        if !serialized_json_fits(&report, self.max_response_bytes) {
            return Err(Error::ResponseTooLarge {
                method: "inbox",
                limit: self.max_response_bytes,
            });
        }
        let mut byte_limited = false;
        for unread in selected {
            let resolved_name = loaded
                .conversations
                .get(&unread.id)
                .map(|conversation| {
                    loaded_inbox_conversation_name(conversation, loaded.author_directory.as_ref())
                })
                .unwrap_or_else(|| unresolved_unread_name(loaded.conversation_scan_complete));
            let conversation = loaded
                .conversations
                .get(&unread.id)
                .cloned()
                .unwrap_or_else(|| fallback_conversation(&unread));
            if conversation.kind != unread.kind {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            let mut messages = self
                .read_channel_by_id(&unread.id, None, message_limit)
                .await?;
            if messages.messages.iter().any(message_needs_directory) {
                if loaded.author_directory.is_none() {
                    loaded.author_directory = Some(self.author_directory(None).await);
                }
                if let Some(directory) = &loaded.author_directory {
                    enrich_messages_from_directory(&mut messages.messages, directory);
                }
            }
            report.conversations.push(InboxConversation {
                conversation,
                unread: named_unread(unread, resolved_name),
                messages,
            });
            report.has_more_conversations = true;
            report.truncation_reason = if report.conversations.len() == selected_count
                && total_unread_conversations > selected_count
            {
                Some(InboxTruncationReason::ConversationLimit)
            } else {
                Some(InboxTruncationReason::ByteLimit)
            };
            if !serialized_json_fits(&report, self.max_response_bytes) {
                report.conversations.pop();
                byte_limited = true;
                break;
            }
        }
        report.has_more_conversations = total_unread_conversations > report.conversations.len();
        report.truncation_reason = if byte_limited {
            Some(InboxTruncationReason::ByteLimit)
        } else if report.has_more_conversations {
            Some(InboxTruncationReason::ConversationLimit)
        } else {
            None
        };
        if !serialized_json_fits(&report, self.max_response_bytes) {
            return Err(Error::ResponseTooLarge {
                method: "inbox",
                limit: self.max_response_bytes,
            });
        }
        Ok(report)
    }

    pub(crate) async fn activity(&self, request: ActivityRequest<'_>) -> Result<ActivityReport> {
        let cursor = match request.cursor {
            Some(cursor) => {
                if request.since.is_some()
                    || request.after.is_some()
                    || request.before.is_some()
                    || !request.include.is_empty()
                    || !request.exclude.is_empty()
                    || !request.kinds.is_empty()
                    || request.order.is_some()
                    || request.conversation_limit.is_some()
                    || request.per_conversation_limit.is_some()
                    || request.limit.is_some()
                {
                    return Err(Error::invalid_input(
                        "cursor",
                        "must be used without interval, filter, ordering, or limit options",
                    ));
                }
                let cursor = decode_activity_cursor(cursor)?;
                validate_activity_cursor(&cursor, &self.team_id)?;
                Some(cursor)
            }
            None => None,
        };

        let (
            after_nanos,
            before_nanos,
            order,
            conversation_kinds,
            conversation_limit,
            per_conversation_limit,
            limit,
        ) = if let Some(cursor) = &cursor {
            (
                cursor.after_nanos,
                cursor.before_nanos,
                cursor.order,
                cursor.conversation_kinds.clone(),
                cursor.conversation_limit,
                cursor.per_conversation_limit,
                cursor.limit,
            )
        } else {
            let (after_nanos, before_nanos) =
                self.resolve_activity_interval(request.since, request.after, request.before)?;
            let conversation_limit = request
                .conversation_limit
                .unwrap_or(DEFAULT_ACTIVITY_CONVERSATIONS);
            let per_conversation_limit = request
                .per_conversation_limit
                .unwrap_or(DEFAULT_ACTIVITY_PER_CONVERSATION);
            let limit = request.limit.unwrap_or(DEFAULT_ACTIVITY_MESSAGES);
            validate_limit(
                "conversation_limit",
                conversation_limit,
                MAX_ACTIVITY_CONVERSATIONS,
            )?;
            validate_limit(
                "per_conversation_limit",
                per_conversation_limit,
                MAX_ACTIVITY_PER_CONVERSATION,
            )?;
            validate_limit("limit", limit, MAX_ACTIVITY_MESSAGES)?;
            (
                after_nanos,
                before_nanos,
                request.order.unwrap_or(ActivityOrder::NewestFirst),
                normalize_activity_kinds(request.kinds),
                conversation_limit,
                per_conversation_limit,
                limit,
            )
        };

        let author_directory = match self.scan_user_directory().await {
            UserDirectoryScan::Finished(directory) => AuthorDirectory::Loaded(directory),
            UserDirectoryScan::Interrupted { directory, error } => {
                if matches!(error, Error::Authentication) {
                    return Err(error);
                }
                AuthorDirectory::Interrupted(directory)
            }
        };
        let all_conversations = self
            .scan_activity_conversations(author_directory_users(&author_directory))
            .await?;
        let scope = match &cursor {
            Some(cursor) => {
                let scope = rebuild_activity_scope(
                    all_conversations,
                    &conversation_kinds,
                    &cursor.include_ids,
                    &cursor.exclude_ids,
                )?;
                if cursor.eligible_conversations != scope.candidates.len()
                    || cursor.conversation_scan_truncated != scope.scan_truncated
                    || cursor.scope_digest != scope.digest
                    || cursor.include_ids != scope.include_ids
                    || cursor.exclude_ids != scope.exclude_ids
                {
                    return Err(stale_activity_cursor());
                }
                scope
            }
            None => select_activity_scope(
                all_conversations,
                &conversation_kinds,
                request.include,
                request.exclude,
            )?,
        };
        let scope_offset = cursor
            .as_ref()
            .map(|cursor| activity_cursor_scope_offset(&cursor.position))
            .unwrap_or(0);
        if scope_offset > scope.candidates.len() {
            return Err(stale_activity_cursor());
        }
        let scope_end = scope_offset
            .checked_add(conversation_limit)
            .ok_or(Error::Output)?
            .min(scope.candidates.len());
        let mut conversations = scope.candidates[scope_offset..scope_end]
            .iter()
            .map(|candidate| candidate.conversation.clone())
            .collect::<Vec<_>>();
        conversations.sort_by(|left, right| left.id.cmp(&right.id));
        if let Some(ActivityCursorPosition::Messages { last_key, .. }) =
            cursor.as_ref().map(|cursor| &cursor.position)
            && !conversations
                .iter()
                .any(|conversation| conversation.id == last_key.conversation_id)
        {
            return Err(stale_activity_cursor());
        }

        let slack_bounds = activity_slack_bounds(after_nanos, before_nanos)?;
        let mut items = Vec::new();
        let mut conversation_results = Vec::with_capacity(conversations.len());
        let mut seen_keys = HashSet::new();
        for conversation in &conversations {
            let Some((oldest, latest)) = &slack_bounds else {
                conversation_results.push(ActivityConversationResult {
                    conversation: conversation.clone(),
                    status: ActivityConversationStatus::Complete,
                    messages_sampled: 0,
                });
                continue;
            };
            let raw = match self
                .api
                .activity_history(&conversation.id, oldest, latest, per_conversation_limit)
                .await
            {
                Ok(raw) => raw,
                Err(error) if matches!(error, Error::Authentication) => return Err(error),
                Err(error) => {
                    conversation_results.push(ActivityConversationResult {
                        conversation: conversation.clone(),
                        status: activity_error_status(&error),
                        messages_sampled: 0,
                    });
                    continue;
                }
            };
            let next_cursor = match response_cursor(
                "conversations.history",
                raw.response_metadata.next_cursor.clone(),
            ) {
                Ok(next_cursor) => next_cursor,
                Err(_) => {
                    conversation_results.push(ActivityConversationResult {
                        conversation: conversation.clone(),
                        status: ActivityConversationStatus::Unavailable,
                        messages_sampled: 0,
                    });
                    continue;
                }
            };
            let status = if raw.messages.len() > per_conversation_limit
                || raw.has_more
                || next_cursor.is_some()
            {
                ActivityConversationStatus::MessageLimit
            } else {
                ActivityConversationStatus::Complete
            };
            let mut messages = match normalize_messages(
                &self.workspace_url,
                &conversation.id,
                raw.messages,
                per_conversation_limit,
                "conversations.history",
            ) {
                Ok(messages) => messages,
                Err(_) => {
                    conversation_results.push(ActivityConversationResult {
                        conversation: conversation.clone(),
                        status: ActivityConversationStatus::Unavailable,
                        messages_sampled: 0,
                    });
                    continue;
                }
            };
            messages.retain(|message| {
                timestamp_in_activity_interval(&message.ts, after_nanos, before_nanos)
            });
            enrich_messages_from_directory(&mut messages, &author_directory);
            for message in messages {
                let key = (
                    conversation.id.clone(),
                    canonical_activity_timestamp(&message.ts),
                );
                if !seen_keys.insert(key) {
                    return Err(Error::InvalidResponse { method: "activity" });
                }
                items.push(ActivityItem {
                    conversation_id: conversation.id.clone(),
                    conversation_name: conversation.name.clone(),
                    conversation_display_name: conversation.display_name.clone(),
                    conversation_kind: conversation.kind,
                    message,
                });
            }
            conversation_results.push(ActivityConversationResult {
                conversation: conversation.clone(),
                status,
                messages_sampled: items
                    .iter()
                    .filter(|item| item.conversation_id == conversation.id)
                    .count(),
            });
        }
        items.sort_by(|left, right| {
            compare_activity_keys(
                &left.message.ts,
                &left.conversation_id,
                &right.message.ts,
                &right.conversation_id,
            )
        });
        if order == ActivityOrder::NewestFirst {
            items.reverse();
        }
        conversation_results
            .sort_by(|left, right| left.conversation.id.cmp(&right.conversation.id));

        let snapshot_digest = activity_snapshot_digest(&items, &conversation_results)?;
        let start = match cursor.as_ref().map(|cursor| &cursor.position) {
            Some(ActivityCursorPosition::Messages {
                last_key,
                snapshot_digest: expected_digest,
                ..
            }) => {
                if expected_digest != &snapshot_digest {
                    return Err(stale_activity_cursor());
                }
                items
                    .iter()
                    .position(|item| activity_item_key(item) == *last_key)
                    .map(|index| index + 1)
                    .ok_or_else(stale_activity_cursor)?
            }
            Some(ActivityCursorPosition::ConversationScope { .. }) | None => 0,
        };
        let desired_end = start.saturating_add(limit).min(items.len());
        let mut page_items = items[start..desired_end].to_vec();
        let remaining_conversations = scope.candidates.len() - scope_end;
        let scope_has_more = remaining_conversations > 0;
        let selection_truncated =
            scope.scan_truncated || conversations.len() < scope.candidates.len();
        let partial_without_bytes = selection_truncated
            || conversation_results
                .iter()
                .any(|result| result.status != ActivityConversationStatus::Complete);
        let mut byte_limited = false;

        loop {
            let end = start + page_items.len();
            let messages_have_more = end < items.len();
            let (continuation_kind, position) = if messages_have_more {
                let last = page_items.last().ok_or(Error::ResponseTooLarge {
                    method: "activity",
                    limit: self.max_response_bytes,
                })?;
                (
                    Some(ActivityContinuationKind::Messages),
                    Some(ActivityCursorPosition::Messages {
                        scope_offset,
                        last_key: activity_item_key(last),
                        snapshot_digest: snapshot_digest.clone(),
                    }),
                )
            } else if scope_has_more {
                (
                    Some(ActivityContinuationKind::ConversationScope),
                    Some(ActivityCursorPosition::ConversationScope {
                        scope_offset: scope_end,
                    }),
                )
            } else {
                (None, None)
            };
            let next_cursor = position
                .map(|position| {
                    encode_activity_cursor(&ActivityCursor {
                        version: ACTIVITY_CURSOR_VERSION,
                        team_id: self.team_id.clone(),
                        after_nanos,
                        before_nanos,
                        order,
                        conversation_kinds: conversation_kinds.clone(),
                        include_ids: scope.include_ids.clone(),
                        exclude_ids: scope.exclude_ids.clone(),
                        conversation_limit,
                        per_conversation_limit,
                        limit,
                        eligible_conversations: scope.candidates.len(),
                        conversation_scan_truncated: scope.scan_truncated,
                        scope_digest: scope.digest.clone(),
                        position,
                    })
                })
                .transpose()?;
            let has_more = next_cursor.is_some();
            let report = ActivityReport {
                team_id: self.team_id.clone(),
                effective_after: format_activity_instant(after_nanos)?,
                effective_before: format_activity_instant(before_nanos)?,
                order,
                conversation_kinds: conversation_kinds.clone(),
                items: page_items.clone(),
                conversation_results: conversation_results.clone(),
                scanned_conversations: scope.scanned_conversations,
                eligible_conversations: scope.candidates.len(),
                scope_offset,
                selected_conversations: conversations.len(),
                remaining_conversations,
                conversation_limit,
                per_conversation_limit,
                limit,
                conversation_scan_truncated: scope.scan_truncated,
                selection_truncated,
                scope_has_more,
                partial: partial_without_bytes || byte_limited,
                response_byte_limit_reached: byte_limited,
                has_more,
                continuation_kind,
                next_cursor,
            };
            if serialized_json_fits(&report, self.max_response_bytes) {
                return Ok(report);
            }
            if page_items.pop().is_none() || page_items.is_empty() {
                return Err(Error::ResponseTooLarge {
                    method: "activity",
                    limit: self.max_response_bytes,
                });
            }
            byte_limited = true;
        }
    }

    fn resolve_activity_interval(
        &self,
        since: Option<&str>,
        after: Option<&str>,
        before: Option<&str>,
    ) -> Result<(i64, i64)> {
        match (since, after, before) {
            (Some(since), None, None) => {
                let duration = parse_activity_duration(since)?;
                let now_millis = (self.now_millis)()?
                    .parse::<i64>()
                    .map_err(|_| Error::SystemClock)?;
                let before = now_millis
                    .checked_mul(1_000_000)
                    .ok_or(Error::SystemClock)?;
                let after = before
                    .checked_sub(
                        duration
                            .num_nanoseconds()
                            .ok_or_else(|| Error::invalid_input("since", "is too large"))?,
                    )
                    .ok_or_else(|| Error::invalid_input("since", "is too large"))?;
                if after < 0 {
                    return Err(Error::invalid_input(
                        "since",
                        "extends earlier than the Unix epoch",
                    ));
                }
                Ok((after, before))
            }
            (None, Some(after), Some(before)) => {
                let after = parse_activity_rfc3339("after", after)?;
                let before = parse_activity_rfc3339("before", before)?;
                if after >= before {
                    return Err(Error::invalid_input("after", "must be earlier than before"));
                }
                Ok((after, before))
            }
            _ => Err(Error::invalid_input(
                "interval",
                "provide either since or both after and before",
            )),
        }
    }

    async fn scan_activity_conversations(
        &self,
        users: &HashMap<String, User>,
    ) -> Result<ActivityConversationDirectory> {
        let mut candidates = Vec::new();
        let mut seen_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await?;
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            for raw in page.channels {
                let conversation = normalize_conversations(vec![raw], users)?.pop().ok_or(
                    Error::InvalidResponse {
                        method: "conversations.list",
                    },
                )?;
                if !conversation.is_archived && !seen_ids.insert(conversation.id.clone()) {
                    return Err(Error::InvalidResponse {
                        method: "conversations.list",
                    });
                }
                if !conversation.is_archived {
                    candidates.push(ActivityConversationCandidate { conversation });
                }
            }
            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                return Ok(ActivityConversationDirectory {
                    scanned_conversations: candidates.len(),
                    candidates,
                    scan_truncated: false,
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                return Ok(ActivityConversationDirectory {
                    scanned_conversations: candidates.len(),
                    candidates,
                    scan_truncated: true,
                });
            }
        }
        unreachable!("bounded activity conversation scan always returns")
    }

    pub(crate) async fn list_conversations(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ConversationPage> {
        validate_limit("limit", limit, CONVERSATIONS_PAGE_SIZE)?;
        validate_cursor(cursor)?;
        let raw = self.api.conversations_list(cursor, limit).await?;
        if raw.channels.len() > limit {
            return Err(Error::InvalidResponse {
                method: "conversations.list",
            });
        }
        let user_directory = if raw.channels.iter().any(|conversation| conversation.is_im) {
            self.load_user_directory().await?
        } else {
            UserDirectory {
                users: HashMap::new(),
                conflicting_ids: HashSet::new(),
                complete: true,
            }
        };
        let next_cursor = response_cursor("conversations.list", raw.response_metadata.next_cursor)?;
        reject_repeated_cursor("conversations.list", cursor, next_cursor.as_deref())?;
        Ok(ConversationPage {
            conversations: normalize_conversations(raw.channels, &user_directory.users)?,
            has_more: next_cursor.is_some(),
            next_cursor,
        })
    }

    pub(crate) async fn find_conversations(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<ConversationSearchReport> {
        let query = validate_query(query)?;
        validate_limit("limit", limit, MAX_CONVERSATIONS)?;
        let needle = query.to_lowercase();
        let user_directory = self.load_user_directory().await?;
        let mut conversations = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut scanned_conversations = 0;

        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await?;
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            scanned_conversations += page.channels.len();
            for conversation in normalize_conversations(page.channels, &user_directory.users)? {
                if conversation_matches(&conversation, &needle) {
                    if conversations.len() == limit {
                        return Ok(ConversationSearchReport {
                            query,
                            conversations,
                            truncated: true,
                            truncation_reason: Some(
                                ConversationSearchTruncationReason::ResultLimit,
                            ),
                            scanned_conversations,
                            scan_limit: CONVERSATIONS_PAGE_SIZE * MAX_CONVERSATION_PAGES,
                        });
                    }
                    conversations.push(conversation);
                }
            }
            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                let truncated = !user_directory.complete;
                return Ok(ConversationSearchReport {
                    query,
                    conversations,
                    truncated,
                    truncation_reason: truncated
                        .then_some(ConversationSearchTruncationReason::ScanLimit),
                    scanned_conversations,
                    scan_limit: CONVERSATIONS_PAGE_SIZE * MAX_CONVERSATION_PAGES,
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                return Ok(ConversationSearchReport {
                    query,
                    conversations,
                    truncated: true,
                    truncation_reason: Some(ConversationSearchTruncationReason::ScanLimit),
                    scanned_conversations,
                    scan_limit: CONVERSATIONS_PAGE_SIZE * MAX_CONVERSATION_PAGES,
                });
            }
        }
        unreachable!("bounded conversation page loop always returns")
    }

    pub(crate) async fn search_messages(
        &self,
        query: &str,
        conversation: Option<&str>,
        after: Option<&str>,
        before: Option<&str>,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MessageSearchPage> {
        let mut applied_query = validate_search_query(query)?;
        validate_limit("limit", limit, MAX_SEARCH_MESSAGES)?;
        validate_cursor(cursor)?;
        let after = after
            .map(|value| validate_date("after", value))
            .transpose()?;
        let before = before
            .map(|value| validate_date("before", value))
            .transpose()?;
        if after
            .as_deref()
            .zip(before.as_deref())
            .is_some_and(|(after, before)| after > before)
        {
            return Err(Error::invalid_input(
                "after",
                "must not be later than before",
            ));
        }
        let mut user_directory = None;
        if let Some(reference) = conversation {
            let (conversation, resolved_user_directory) = self
                .resolve_search_conversation_for_message_read(reference)
                .await?;
            user_directory = resolved_user_directory;
            let target = if conversation.kind == ConversationKind::DirectMessage {
                format!(
                    "<@{}>",
                    conversation
                        .user_id
                        .as_deref()
                        .ok_or(Error::InvalidResponse {
                            method: "conversations.list",
                        })?
                )
            } else {
                conversation.name
            };
            applied_query.push_str(" in:");
            applied_query.push_str(&target);
        }
        if let Some(after) = &after {
            applied_query.push_str(" after:");
            applied_query.push_str(after);
        }
        if let Some(before) = &before {
            applied_query.push_str(" before:");
            applied_query.push_str(before);
        }

        let raw = self
            .api
            .search_messages(&applied_query, cursor, limit)
            .await?;
        if raw.messages.matches.len() > limit
            || raw.messages.total < raw.messages.matches.len() as u64
        {
            return Err(Error::InvalidResponse {
                method: "search.messages",
            });
        }
        if !raw.query.is_empty()
            && (raw.query.len() > 2048 || raw.query.chars().any(char::is_control))
        {
            return Err(Error::InvalidResponse {
                method: "search.messages",
            });
        }
        let metadata_cursor =
            response_cursor("search.messages", raw.response_metadata.next_cursor)?;
        let pagination_cursor =
            response_cursor("search.messages", raw.messages.pagination.next_cursor)?;
        if metadata_cursor
            .as_deref()
            .zip(pagination_cursor.as_deref())
            .is_some_and(|(metadata, pagination)| metadata != pagination)
        {
            return Err(Error::InvalidResponse {
                method: "search.messages",
            });
        }
        let next_cursor = metadata_cursor.or(pagination_cursor);
        if next_cursor
            .as_deref()
            .is_some_and(|next| next == cursor.unwrap_or("*"))
        {
            return Err(Error::InvalidResponse {
                method: "search.messages",
            });
        }
        let mut matches = normalize_search_matches(&self.workspace_url, raw.messages.matches)?;
        self.enrich_search_messages(&mut matches, user_directory)
            .await;
        Ok(MessageSearchPage {
            query: applied_query,
            has_more: next_cursor.is_some(),
            total: raw.messages.total,
            matches,
            next_cursor,
        })
    }

    pub(crate) async fn read_channel(
        &self,
        channel: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MessagePage> {
        validate_limit("limit", limit, MAX_MESSAGES)?;
        validate_cursor(cursor)?;
        let (channel, user_directory) = self.resolve_conversation_for_message_read(channel).await?;
        let mut page = self.read_channel_by_id(&channel, cursor, limit).await?;
        self.enrich_messages(&mut page.messages, user_directory)
            .await;
        Ok(page)
    }

    async fn read_channel_by_id(
        &self,
        channel: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<MessagePage> {
        let raw = self
            .api
            .conversation_history(channel, cursor, limit)
            .await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor =
            response_cursor("conversations.history", raw.response_metadata.next_cursor)?;
        reject_repeated_cursor("conversations.history", cursor, next_cursor.as_deref())?;
        Ok(MessagePage {
            channel_id: channel.to_owned(),
            messages: normalize_messages(
                &self.workspace_url,
                channel,
                raw.messages,
                limit,
                "conversations.history",
            )?,
            has_more: raw.has_more || next_cursor.is_some() || locally_truncated,
            next_cursor,
        })
    }

    pub(crate) async fn read_thread(
        &self,
        channel: &str,
        thread_ts: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<ThreadPage> {
        validate_timestamp("thread_ts", thread_ts)?;
        validate_limit("limit", limit, MAX_MESSAGES)?;
        validate_cursor(cursor)?;
        let (channel, user_directory) = self.resolve_conversation_for_message_read(channel).await?;
        let raw = self
            .api
            .conversation_replies(&channel, thread_ts, cursor, limit)
            .await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor =
            response_cursor("conversations.replies", raw.response_metadata.next_cursor)?;
        reject_repeated_cursor("conversations.replies", cursor, next_cursor.as_deref())?;
        let mut messages = normalize_messages(
            &self.workspace_url,
            &channel,
            raw.messages,
            limit,
            "conversations.replies",
        )?;
        self.enrich_messages(&mut messages, user_directory).await;
        Ok(ThreadPage {
            channel_id: channel.clone(),
            thread_ts: thread_ts.to_owned(),
            messages,
            has_more: raw.has_more || next_cursor.is_some() || locally_truncated,
            next_cursor,
        })
    }

    pub(crate) async fn get_message(&self, channel: &str, message_ts: &str) -> Result<Message> {
        validate_timestamp("message_ts", message_ts)?;
        let (channel, user_directory) = self.resolve_conversation_for_message_read(channel).await?;
        let mut message = self.get_message_by_id(&channel, message_ts).await?;
        self.enrich_messages(std::slice::from_mut(&mut message), user_directory)
            .await;
        Ok(message)
    }

    async fn get_message_by_id(&self, channel: &str, message_ts: &str) -> Result<Message> {
        let raw = self.api.messages_list(channel, message_ts).await?;
        let mut candidates = raw.messages.into_values().collect::<Vec<_>>();
        if let Some(channel_messages) = raw.messages_data.get(channel) {
            candidates.extend(channel_messages.messages.iter().cloned());
        }
        if candidates
            .iter()
            .any(|message| !is_valid_timestamp(&message.ts))
        {
            return Err(Error::InvalidResponse {
                method: "messages.list",
            });
        }
        let mut matches = candidates
            .into_iter()
            .filter(|message| message.ts == message_ts);
        let Some(first) = matches.next() else {
            return Err(Error::NotFound {
                resource: "Slack message",
            });
        };
        let first = normalize_message(&self.workspace_url, channel, first, "messages.list")?;
        for duplicate in matches {
            if normalize_message(&self.workspace_url, channel, duplicate, "messages.list")? != first
            {
                return Err(Error::InvalidResponse {
                    method: "messages.list",
                });
            }
        }
        Ok(first)
    }

    pub(crate) async fn find_users(&self, query: &str, limit: usize) -> Result<UserSearchReport> {
        let query = validate_query(query)?;
        validate_limit("limit", limit, MAX_USERS)?;
        let needle = query.to_lowercase();
        let mut users = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut scanned_users = 0;

        for page_index in 0..MAX_USER_PAGES {
            let page = self
                .api
                .users_list(cursor.as_deref(), USERS_PAGE_SIZE)
                .await?;
            if page.members.len() > USERS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "users.list",
                });
            }
            for raw_user in page.members {
                scanned_users += 1;
                if !is_valid_user_id(&raw_user.id) {
                    return Err(Error::InvalidResponse {
                        method: "users.list",
                    });
                }
                if user_matches(&raw_user, &needle) {
                    if users.len() == limit {
                        return Ok(UserSearchReport {
                            query,
                            users,
                            truncated: true,
                            truncation_reason: Some(UserSearchTruncationReason::ResultLimit),
                            scanned_users,
                            scan_limit: USERS_PAGE_SIZE * MAX_USER_PAGES,
                        });
                    }
                    users.push(normalize_user(raw_user));
                }
            }
            let next = response_cursor("users.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                return Ok(UserSearchReport {
                    query,
                    users,
                    truncated: false,
                    truncation_reason: None,
                    scanned_users,
                    scan_limit: USERS_PAGE_SIZE * MAX_USER_PAGES,
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "users.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_USER_PAGES {
                return Ok(UserSearchReport {
                    query,
                    users,
                    truncated: true,
                    truncation_reason: Some(UserSearchTruncationReason::ScanLimit),
                    scanned_users,
                    scan_limit: USERS_PAGE_SIZE * MAX_USER_PAGES,
                });
            }
        }
        unreachable!("bounded user page loop always returns")
    }

    async fn resolve_conversation_id(&self, reference: &str) -> Result<String> {
        if is_slack_shaped_conversation_id(reference) {
            return Ok(reference.to_owned());
        }
        Ok(self.resolve_named_conversation(reference).await?.id)
    }

    async fn resolve_conversation_for_message_read(
        &self,
        reference: &str,
    ) -> Result<(String, Option<UserDirectory>)> {
        if is_slack_shaped_conversation_id(reference) {
            return Ok((reference.to_owned(), None));
        }
        let resolved = self
            .resolve_named_conversation_with_directory(reference)
            .await?;
        Ok((resolved.conversation.id, Some(resolved.user_directory)))
    }

    async fn resolve_search_conversation_for_message_read(
        &self,
        reference: &str,
    ) -> Result<(Conversation, Option<UserDirectory>)> {
        if is_slack_shaped_conversation_id(reference) {
            return Ok((self.find_conversation_by_id(reference).await?, None));
        }
        let resolved = self
            .resolve_named_conversation_with_directory(reference)
            .await?;
        Ok((resolved.conversation, Some(resolved.user_directory)))
    }

    async fn resolve_named_conversation(&self, reference: &str) -> Result<Conversation> {
        Ok(self
            .resolve_named_conversation_with_directory(reference)
            .await?
            .conversation)
    }

    async fn resolve_named_conversation_with_directory(
        &self,
        reference: &str,
    ) -> Result<ResolvedNamedConversation> {
        let needle = validate_conversation_reference(reference)?.to_lowercase();
        let user_directory = self.load_user_directory().await?;
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut matched: Option<Conversation> = None;

        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await?;
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            for conversation in normalize_conversations(page.channels, &user_directory.users)? {
                if conversation_matches_exactly(&conversation, &needle) {
                    if matched
                        .as_ref()
                        .is_some_and(|matched| matched.id != conversation.id)
                    {
                        return Err(Error::invalid_input(
                            "conversation",
                            "matches more than one Slack conversation; use its ID",
                        ));
                    }
                    matched = Some(conversation);
                }
            }
            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                if !user_directory.complete {
                    return Err(Error::ScanLimit {
                        resource: "Slack conversation",
                        limit: USERS_PAGE_SIZE * MAX_USER_PAGES,
                    });
                }
                return Ok(ResolvedNamedConversation {
                    conversation: matched.ok_or(Error::NotFound {
                        resource: "Slack conversation",
                    })?,
                    user_directory,
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                return Err(Error::ScanLimit {
                    resource: "Slack conversation",
                    limit: CONVERSATIONS_PAGE_SIZE * MAX_CONVERSATION_PAGES,
                });
            }
        }
        unreachable!("bounded conversation resolver loop always returns")
    }

    async fn find_conversation_by_id(&self, id: &str) -> Result<Conversation> {
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await?;
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            if let Some(conversation) = normalize_conversations(page.channels, &HashMap::new())?
                .into_iter()
                .find(|conversation| conversation.id == id)
            {
                return Ok(conversation);
            }
            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                return Err(Error::NotFound {
                    resource: "Slack conversation",
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                return Err(Error::ScanLimit {
                    resource: "Slack conversation",
                    limit: CONVERSATIONS_PAGE_SIZE * MAX_CONVERSATION_PAGES,
                });
            }
        }
        unreachable!("bounded conversation ID lookup always returns")
    }

    async fn resolve_unread_conversation_names(
        &self,
        unreads: &[UnreadCount],
    ) -> HashMap<String, ResolvedUnreadName> {
        if unreads.is_empty() {
            return HashMap::new();
        }
        let wanted = unreads
            .iter()
            .map(|unread| (unread.id.clone(), unread.kind))
            .collect::<HashMap<_, _>>();
        let mut conversations = HashMap::new();
        let mut conflicting_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut completion = DirectoryCompletion::Unavailable;

        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = match self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await
            {
                Ok(page) => page,
                Err(_) => break,
            };
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                break;
            }
            for raw in page.channels {
                if !wanted.contains_key(&raw.id) || conflicting_ids.contains(&raw.id) {
                    continue;
                }
                let id = raw.id.clone();
                match conversations.entry(id) {
                    Entry::Vacant(entry) => {
                        entry.insert(raw);
                    }
                    Entry::Occupied(entry) => {
                        let (id, _) = entry.remove_entry();
                        conflicting_ids.insert(id);
                    }
                }
            }
            if conversations.len() + conflicting_ids.len() == wanted.len() {
                completion = DirectoryCompletion::Complete;
                break;
            }
            let next =
                match response_cursor("conversations.list", page.response_metadata.next_cursor) {
                    Ok(next) => next,
                    Err(_) => break,
                };
            let Some(next) = next else {
                completion = DirectoryCompletion::Complete;
                break;
            };
            if !seen_cursors.insert(next.clone()) {
                break;
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                completion = DirectoryCompletion::Incomplete;
            }
        }

        let wanted_users = conversations
            .iter()
            .filter(|(id, conversation)| {
                wanted.get(*id) == Some(&ConversationKind::DirectMessage)
                    && conversation.is_im
                    && !conversation.is_mpim
            })
            .filter_map(|(_, conversation)| {
                conversation
                    .user
                    .as_deref()
                    .filter(|id| is_valid_user_id(id))
            })
            .map(str::to_owned)
            .collect::<HashSet<_>>();
        let user_directory = if wanted_users.is_empty() {
            None
        } else {
            Some(self.resolve_unread_user_directory(&wanted_users).await)
        };
        let directory = UnreadConversationDirectory {
            conversations,
            conflicting_ids,
            completion,
            user_directory,
        };
        unreads
            .iter()
            .map(|unread| (unread.id.clone(), resolved_unread_name(unread, &directory)))
            .collect()
    }

    async fn resolve_unread_user_directory(&self, wanted: &HashSet<String>) -> UnreadUserDirectory {
        let mut users = HashMap::new();
        let mut conflicting_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut completion = DirectoryCompletion::Unavailable;

        for page_index in 0..MAX_USER_PAGES {
            let page = match self
                .api
                .users_list(cursor.as_deref(), USERS_PAGE_SIZE)
                .await
            {
                Ok(page) => page,
                Err(_) => break,
            };
            let (page_users, next) = match normalize_user_directory_page(page, &seen_cursors) {
                Ok(page) => page,
                Err(_) => break,
            };
            for user in page_users {
                if !wanted.contains(&user.id) || conflicting_ids.contains(&user.id) {
                    continue;
                }
                let id = user.id.clone();
                match users.entry(id) {
                    Entry::Vacant(entry) => {
                        entry.insert(user);
                    }
                    Entry::Occupied(entry) => {
                        let (id, _) = entry.remove_entry();
                        conflicting_ids.insert(id);
                    }
                }
            }
            if users.len() + conflicting_ids.len() == wanted.len() {
                completion = DirectoryCompletion::Complete;
                break;
            }
            let Some(next) = next else {
                completion = DirectoryCompletion::Complete;
                break;
            };
            seen_cursors.insert(next.clone());
            cursor = Some(next);
            if page_index + 1 == MAX_USER_PAGES {
                completion = DirectoryCompletion::Incomplete;
            }
        }

        UnreadUserDirectory {
            users,
            conflicting_ids,
            completion,
        }
    }

    async fn load_conversations_by_id(
        &self,
        ids: &HashSet<String>,
    ) -> Result<LoadedInboxConversations> {
        if ids.is_empty() {
            return Ok(LoadedInboxConversations {
                conversations: HashMap::new(),
                author_directory: None,
                conversation_scan_complete: true,
            });
        }

        let mut matched = Vec::new();
        let mut matched_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();
        let mut conversation_scan_complete = false;

        for page_index in 0..MAX_CONVERSATION_PAGES {
            let page = self
                .api
                .conversations_list(cursor.as_deref(), CONVERSATIONS_PAGE_SIZE)
                .await?;
            if page.channels.len() > CONVERSATIONS_PAGE_SIZE {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }

            normalize_conversations(page.channels.clone(), &HashMap::new())?;
            for raw in page.channels {
                if ids.contains(&raw.id) {
                    if !matched_ids.insert(raw.id.clone()) {
                        return Err(Error::InvalidResponse {
                            method: "conversations.list",
                        });
                    }
                    matched.push(raw);
                }
            }
            if matched_ids.len() == ids.len() {
                conversation_scan_complete = true;
                break;
            }

            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                conversation_scan_complete = true;
                break;
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_CONVERSATION_PAGES {
                break;
            }
        }

        let author_directory = if matched.iter().any(|conversation| conversation.is_im) {
            Some(self.author_directory(None).await)
        } else {
            None
        };
        let empty_users = HashMap::new();
        let users = author_directory
            .as_ref()
            .map(author_directory_users)
            .unwrap_or(&empty_users);
        Ok(LoadedInboxConversations {
            conversations: normalize_conversations(matched, users)?
                .into_iter()
                .map(|conversation| (conversation.id.clone(), conversation))
                .collect(),
            author_directory,
            conversation_scan_complete,
        })
    }

    async fn enrich_messages(
        &self,
        messages: &mut [Message],
        user_directory: Option<UserDirectory>,
    ) {
        if !messages.iter().any(message_needs_directory) {
            return;
        }
        let directory = self.author_directory(user_directory).await;
        enrich_messages_from_directory(messages, &directory);
    }

    async fn enrich_search_messages(
        &self,
        messages: &mut [MessageSearchMatch],
        user_directory: Option<UserDirectory>,
    ) {
        if !messages.iter().any(search_message_needs_directory) {
            return;
        }
        let directory = self.author_directory(user_directory).await;
        enrich_search_messages_from_directory(messages, &directory);
    }

    async fn author_directory(&self, user_directory: Option<UserDirectory>) -> AuthorDirectory {
        if let Some(user_directory) = user_directory {
            return AuthorDirectory::Loaded(user_directory);
        }
        match self.scan_user_directory().await {
            UserDirectoryScan::Finished(user_directory) => AuthorDirectory::Loaded(user_directory),
            UserDirectoryScan::Interrupted { directory, .. } => {
                AuthorDirectory::Interrupted(directory)
            }
        }
    }

    async fn load_user_directory(&self) -> Result<UserDirectory> {
        match self.scan_user_directory().await {
            UserDirectoryScan::Finished(directory) => Ok(directory),
            UserDirectoryScan::Interrupted { error, .. } => Err(error),
        }
    }

    async fn scan_user_directory(&self) -> UserDirectoryScan {
        let mut users = HashMap::new();
        let mut conflicting_ids = HashSet::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

        for page_index in 0..MAX_USER_PAGES {
            let page = match self
                .api
                .users_list(cursor.as_deref(), USERS_PAGE_SIZE)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    return UserDirectoryScan::Interrupted {
                        directory: UserDirectory {
                            users,
                            conflicting_ids,
                            complete: false,
                        },
                        error,
                    };
                }
            };
            let (page_users, next) = match normalize_user_directory_page(page, &seen_cursors) {
                Ok(page) => page,
                Err(error) => {
                    return UserDirectoryScan::Interrupted {
                        directory: UserDirectory {
                            users,
                            conflicting_ids,
                            complete: false,
                        },
                        error,
                    };
                }
            };
            for user in page_users {
                if conflicting_ids.contains(&user.id) {
                    continue;
                }
                let id = user.id.clone();
                match users.entry(id) {
                    Entry::Vacant(entry) => {
                        entry.insert(user);
                    }
                    Entry::Occupied(entry) => {
                        let (id, _) = entry.remove_entry();
                        conflicting_ids.insert(id);
                    }
                }
            }
            let Some(next) = next else {
                return UserDirectoryScan::Finished(UserDirectory {
                    users,
                    conflicting_ids,
                    complete: true,
                });
            };
            seen_cursors.insert(next.clone());
            cursor = Some(next);
            if page_index + 1 == MAX_USER_PAGES {
                return UserDirectoryScan::Finished(UserDirectory {
                    users,
                    conflicting_ids,
                    complete: false,
                });
            }
        }
        unreachable!("bounded user directory loop always returns")
    }
}

fn normalize_conversations(
    conversations: Vec<RawConversation>,
    users: &HashMap<String, User>,
) -> Result<Vec<Conversation>> {
    conversations
        .into_iter()
        .map(|raw| {
            if raw.is_im && raw.is_mpim {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            let kind = if raw.is_im {
                ConversationKind::DirectMessage
            } else if raw.is_mpim {
                ConversationKind::GroupDirectMessage
            } else {
                ConversationKind::Channel
            };
            if !is_valid_conversation_id(&raw.id, kind) {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }

            let (name, display_name, name_is_fallback, user_id) =
                if kind == ConversationKind::DirectMessage {
                    let user_id = raw.user.filter(|id| is_valid_user_id(id)).ok_or(
                        Error::InvalidResponse {
                            method: "conversations.list",
                        },
                    )?;
                    let user = users.get(&user_id);
                    let loaded_name = user
                        .and_then(|user| user.name.as_deref())
                        .map(str::trim)
                        .filter(|name| is_valid_conversation_name(name))
                        .map(str::to_owned);
                    let name_is_fallback = loaded_name.is_none();
                    let name = loaded_name.unwrap_or_else(|| user_id.clone());
                    let display_name = user
                        .and_then(|user| {
                            [
                                user.display_name.as_deref(),
                                user.real_name.as_deref(),
                                user.name.as_deref(),
                            ]
                            .into_iter()
                            .flatten()
                            .map(str::trim)
                            .find(|value| is_valid_conversation_name(value))
                        })
                        .unwrap_or(&user_id)
                        .to_owned();
                    (name, display_name, name_is_fallback, Some(user_id))
                } else {
                    let name = raw.name.trim();
                    if !is_valid_conversation_name(name) {
                        return Err(Error::InvalidResponse {
                            method: "conversations.list",
                        });
                    }
                    (name.to_owned(), name.to_owned(), false, None)
                };

            Ok(Conversation {
                id: raw.id,
                name,
                display_name,
                name_is_fallback,
                metadata_is_complete: true,
                kind,
                is_private: raw.is_private || kind != ConversationKind::Channel,
                is_archived: raw.is_archived,
                is_member: raw.is_member,
                member_count: raw.num_members,
                user_id,
            })
        })
        .collect()
}

fn fallback_conversation(unread: &UnreadCount) -> Conversation {
    Conversation {
        id: unread.id.clone(),
        name: unread.id.clone(),
        display_name: unread.id.clone(),
        name_is_fallback: true,
        metadata_is_complete: false,
        kind: unread.kind,
        is_private: unread.kind != ConversationKind::Channel,
        is_archived: false,
        is_member: false,
        member_count: None,
        user_id: None,
    }
}

fn named_unread(unread: UnreadCount, name: ResolvedUnreadName) -> UnreadConversation {
    UnreadConversation {
        id: unread.id,
        kind: unread.kind,
        name: name.name,
        display_name: name.display_name,
        name_resolution: name.resolution,
        has_unreads: unread.has_unreads,
        mention_count: unread.mention_count,
        last_read: unread.last_read,
        latest: unread.latest,
    }
}

fn unavailable_unread_name() -> ResolvedUnreadName {
    ResolvedUnreadName {
        name: None,
        display_name: None,
        resolution: ConversationNameResolution::Unavailable,
    }
}

fn unread_name_with_resolution(resolution: ConversationNameResolution) -> ResolvedUnreadName {
    ResolvedUnreadName {
        name: None,
        display_name: None,
        resolution,
    }
}

fn unresolved_unread_name(conversation_scan_complete: bool) -> ResolvedUnreadName {
    unread_name_with_resolution(if conversation_scan_complete {
        ConversationNameResolution::Inaccessible
    } else {
        ConversationNameResolution::Incomplete
    })
}

fn resolved_unread_name(
    unread: &UnreadCount,
    directory: &UnreadConversationDirectory,
) -> ResolvedUnreadName {
    if directory.conflicting_ids.contains(&unread.id) {
        return unavailable_unread_name();
    }
    let Some(conversation) = directory.conversations.get(&unread.id) else {
        return unread_name_with_resolution(match directory.completion {
            DirectoryCompletion::Complete => ConversationNameResolution::Inaccessible,
            DirectoryCompletion::Incomplete => ConversationNameResolution::Incomplete,
            DirectoryCompletion::Unavailable => ConversationNameResolution::Unavailable,
        });
    };
    let Some(kind) = raw_conversation_kind(conversation) else {
        return unavailable_unread_name();
    };
    if kind != unread.kind || !is_valid_conversation_id(&conversation.id, unread.kind) {
        return unavailable_unread_name();
    }
    match kind {
        ConversationKind::Channel => resolved_plain_conversation_name(&conversation.name),
        ConversationKind::GroupDirectMessage => {
            let Some(display_name) = readable_group_dm_name(&conversation.name) else {
                return unread_name_with_resolution(ConversationNameResolution::Unnamed);
            };
            ResolvedUnreadName {
                name: Some(display_name.clone()),
                display_name: Some(display_name),
                resolution: ConversationNameResolution::Resolved,
            }
        }
        ConversationKind::DirectMessage => {
            let Some(user_id) = conversation
                .user
                .as_deref()
                .filter(|id| is_valid_user_id(id))
            else {
                return unavailable_unread_name();
            };
            let Some(user_directory) = &directory.user_directory else {
                return unavailable_unread_name();
            };
            if user_directory.conflicting_ids.contains(user_id) {
                return unavailable_unread_name();
            }
            if let Some(user) = user_directory.users.get(user_id) {
                return resolved_user_conversation_name(user);
            }
            missing_unread_user_name(user_directory)
        }
    }
}

fn raw_conversation_kind(conversation: &RawConversation) -> Option<ConversationKind> {
    if conversation.is_im && conversation.is_mpim {
        None
    } else if conversation.is_im {
        Some(ConversationKind::DirectMessage)
    } else if conversation.is_mpim {
        Some(ConversationKind::GroupDirectMessage)
    } else {
        Some(ConversationKind::Channel)
    }
}

fn resolved_plain_conversation_name(value: &str) -> ResolvedUnreadName {
    let value = value.trim();
    if !is_valid_conversation_name(value) {
        return unread_name_with_resolution(ConversationNameResolution::Unnamed);
    }
    ResolvedUnreadName {
        name: Some(value.to_owned()),
        display_name: Some(value.to_owned()),
        resolution: ConversationNameResolution::Resolved,
    }
}

fn resolved_user_conversation_name(user: &User) -> ResolvedUnreadName {
    let name = user.name.clone();
    let display_name = user
        .display_name
        .clone()
        .or_else(|| user.real_name.clone())
        .or_else(|| name.clone());
    if name.is_none() && display_name.is_none() {
        return unread_name_with_resolution(ConversationNameResolution::Unnamed);
    }
    ResolvedUnreadName {
        name,
        display_name,
        resolution: ConversationNameResolution::Resolved,
    }
}

fn missing_user_conversation_name(directory: &AuthorDirectory) -> ResolvedUnreadName {
    unread_name_with_resolution(match directory {
        AuthorDirectory::Loaded(directory) if directory.complete => {
            ConversationNameResolution::Unnamed
        }
        AuthorDirectory::Loaded(_) => ConversationNameResolution::Incomplete,
        AuthorDirectory::Interrupted(_) => ConversationNameResolution::Unavailable,
    })
}

fn missing_unread_user_name(directory: &UnreadUserDirectory) -> ResolvedUnreadName {
    unread_name_with_resolution(match directory.completion {
        DirectoryCompletion::Complete => ConversationNameResolution::Unnamed,
        DirectoryCompletion::Incomplete => ConversationNameResolution::Incomplete,
        DirectoryCompletion::Unavailable => ConversationNameResolution::Unavailable,
    })
}

fn loaded_inbox_conversation_name(
    conversation: &Conversation,
    author_directory: Option<&AuthorDirectory>,
) -> ResolvedUnreadName {
    match conversation.kind {
        ConversationKind::Channel => resolved_plain_conversation_name(&conversation.name),
        ConversationKind::GroupDirectMessage => {
            let Some(display_name) = readable_group_dm_name(&conversation.name) else {
                return unread_name_with_resolution(ConversationNameResolution::Unnamed);
            };
            ResolvedUnreadName {
                name: Some(display_name.clone()),
                display_name: Some(display_name),
                resolution: ConversationNameResolution::Resolved,
            }
        }
        ConversationKind::DirectMessage if !conversation.name_is_fallback => ResolvedUnreadName {
            name: Some(conversation.name.clone()),
            display_name: Some(conversation.display_name.clone()),
            resolution: ConversationNameResolution::Resolved,
        },
        ConversationKind::DirectMessage => {
            if conversation.user_id.as_deref().is_some_and(|user_id| {
                author_directory
                    .is_some_and(|directory| author_directory_conflicts(directory, user_id))
            }) {
                return unavailable_unread_name();
            }
            if conversation
                .user_id
                .as_deref()
                .is_some_and(|user_id| conversation.display_name != user_id)
                && is_valid_conversation_name(&conversation.display_name)
            {
                return ResolvedUnreadName {
                    name: None,
                    display_name: Some(conversation.display_name.clone()),
                    resolution: ConversationNameResolution::Resolved,
                };
            }
            author_directory
                .map(missing_user_conversation_name)
                .unwrap_or_else(unavailable_unread_name)
        }
    }
}

fn readable_group_dm_name(value: &str) -> Option<String> {
    let value = value.trim();
    if !is_valid_conversation_name(value) {
        return None;
    }
    let Some(encoded) = value.strip_prefix("mpdm-") else {
        return Some(value.to_owned());
    };
    let (participants, suffix) = encoded.rsplit_once('-')?;
    if suffix.is_empty() || suffix.len() > 10 || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let participants = participants.split("--").collect::<Vec<_>>();
    if !(2..=32).contains(&participants.len())
        || participants
            .iter()
            .any(|participant| !is_valid_conversation_name(participant))
    {
        return None;
    }
    let display_name = participants.join(", ");
    is_valid_conversation_name(&display_name).then_some(display_name)
}

fn conversation_matches(conversation: &Conversation, needle: &str) -> bool {
    [
        conversation.id.as_str(),
        conversation.name.as_str(),
        conversation.display_name.as_str(),
        conversation.user_id.as_deref().unwrap_or_default(),
    ]
    .iter()
    .any(|candidate| candidate.to_lowercase().contains(needle))
}

fn conversation_matches_exactly(conversation: &Conversation, needle: &str) -> bool {
    [
        conversation.name.as_str(),
        conversation.display_name.as_str(),
    ]
    .iter()
    .any(|candidate| candidate.to_lowercase() == needle)
}

fn normalize_search_matches(
    workspace_url: &url::Url,
    matches: Vec<RawMessageSearchMatch>,
) -> Result<Vec<MessageSearchMatch>> {
    matches
        .into_iter()
        .map(|raw| {
            if !is_valid_any_conversation_id(&raw.channel.id)
                || !is_valid_timestamp(&raw.ts)
                || raw
                    .thread_ts
                    .as_deref()
                    .is_some_and(|timestamp| !is_valid_timestamp(timestamp))
            {
                return Err(Error::InvalidResponse {
                    method: "search.messages",
                });
            }
            let channel_name = if raw.channel.name.trim().is_empty() {
                raw.channel.id.clone()
            } else if is_valid_conversation_name(raw.channel.name.trim()) {
                raw.channel.name.trim().to_owned()
            } else {
                return Err(Error::InvalidResponse {
                    method: "search.messages",
                });
            };
            let author_id = raw.user.filter(|value| !value.is_empty());
            if author_id
                .as_deref()
                .is_some_and(|value| !is_valid_user_id(value))
            {
                return Err(Error::InvalidResponse {
                    method: "search.messages",
                });
            }
            let author_name = normalize_message_author_name(raw.username);
            let author_resolution = initial_author_resolution(&author_id, &author_name);
            let (rendered_text, mention_resolution, mentions) =
                initial_mention_fields(&raw.text, raw.blocks.as_deref());
            // Search does not guarantee a separate thread timestamp. Treat its URL only as
            // corroborating route metadata, then always emit a locally reconstructed URL.
            let permalink_route = raw.permalink.as_deref().and_then(|permalink| {
                search_permalink_route(workspace_url, &raw.channel.id, &raw.ts, permalink)
            });
            let (thread_ts, permalinks) = match (raw.thread_ts, permalink_route) {
                (Some(thread_ts), _) => {
                    let permalinks = message_permalinks(
                        workspace_url,
                        &raw.channel.id,
                        &raw.ts,
                        Some(&thread_ts),
                    );
                    (Some(thread_ts), permalinks)
                }
                (None, Some(SearchPermalinkRoute::Reply { thread_ts })) => {
                    let permalinks = message_permalinks(
                        workspace_url,
                        &raw.channel.id,
                        &raw.ts,
                        Some(&thread_ts),
                    );
                    (Some(thread_ts), permalinks)
                }
                (None, Some(SearchPermalinkRoute::Root)) => (
                    None,
                    message_permalinks(workspace_url, &raw.channel.id, &raw.ts, None),
                ),
                (None, None) => (
                    None,
                    MessagePermalinks {
                        permalink: None,
                        thread_root_permalink: None,
                        resolution: PermalinkResolution::Unavailable,
                    },
                ),
            };
            Ok(MessageSearchMatch {
                channel_id: raw.channel.id,
                channel_name,
                ts: raw.ts,
                thread_ts,
                permalink: permalinks.permalink,
                thread_root_permalink: permalinks.thread_root_permalink,
                permalink_resolution: permalinks.resolution,
                author_id,
                author_name,
                author_display_name: None,
                author_resolution,
                text: raw.text,
                rendered_text,
                mention_resolution,
                mentions,
                blocks: raw.blocks,
                attachments: normalize_attachments(raw.attachments, "search.messages")?,
                reactions: normalize_reactions(raw.reactions, "search.messages")?,
                files: normalize_files(raw.files, "search.messages")?,
            })
        })
        .collect()
}

fn append_unreads(
    target: &mut Vec<UnreadCount>,
    seen_ids: &mut HashSet<String>,
    source: Vec<RawUnread>,
    kind: ConversationKind,
) -> Result<()> {
    for entry in source {
        if !is_valid_conversation_id(&entry.id, kind) || !seen_ids.insert(entry.id.clone()) {
            return Err(Error::InvalidResponse {
                method: "client.counts",
            });
        }
        if entry.has_unreads {
            target.push(UnreadCount {
                id: entry.id,
                kind,
                has_unreads: entry.has_unreads,
                mention_count: entry.mention_count,
                last_read: entry.last_read,
                latest: entry.latest,
            });
        }
    }
    Ok(())
}

fn normalize_messages(
    workspace_url: &url::Url,
    channel: &str,
    messages: Vec<RawMessage>,
    limit: usize,
    method: &'static str,
) -> Result<Vec<Message>> {
    if messages
        .iter()
        .any(|message| !is_valid_timestamp(&message.ts))
    {
        return Err(Error::InvalidResponse { method });
    }
    messages
        .into_iter()
        .take(limit)
        .map(|message| normalize_message(workspace_url, channel, message, method))
        .collect::<Result<Vec<_>>>()
}

fn normalize_draft(raw: RawDraft, method: &'static str) -> Result<Draft> {
    if !is_valid_draft_id(&raw.id) {
        return Err(Error::InvalidResponse { method });
    }
    let last_updated_ts = raw
        .last_updated_ts
        .map(|revision| match revision {
            RawDraftRevision::String(value) => value,
            RawDraftRevision::Number(value) => value.to_string(),
        })
        .filter(|revision| is_valid_draft_revision(revision))
        .ok_or(Error::InvalidResponse { method })?;
    let client_last_updated_ts = server_revision_to_client_timestamp(&last_updated_ts)
        .ok_or(Error::InvalidResponse { method })?;
    if raw.client_msg_id.as_deref().is_some_and(|client_msg_id| {
        client_msg_id.is_empty()
            || client_msg_id.len() > 128
            || client_msg_id.chars().any(char::is_control)
    }) || raw
        .file_ids
        .iter()
        .any(|file_id| !is_valid_file_id(file_id))
    {
        return Err(Error::InvalidResponse { method });
    }
    for destination in &raw.destinations {
        if destination
            .channel_id
            .as_deref()
            .is_some_and(|channel_id| !is_valid_any_conversation_id(channel_id))
            || destination
                .thread_ts
                .as_deref()
                .is_some_and(|thread_ts| !is_valid_timestamp(thread_ts))
            || (destination.broadcast && destination.thread_ts.is_none())
            || destination.user_ids.as_ref().is_some_and(|user_ids| {
                !(1..=MAX_DRAFT_DESTINATION_USERS).contains(&user_ids.len())
                    || user_ids.iter().any(|user_id| !is_valid_user_id(user_id))
            })
        {
            return Err(Error::InvalidResponse { method });
        }
    }
    let has_unknown_fields = !raw.extra.is_empty();
    let supported_shape = raw.destinations.len() == 1
        && raw.destinations[0].channel_id.is_some()
        && raw.destinations[0].extra.is_empty()
        && raw.attachments.is_empty()
        && !raw.is_deleted
        && !raw.is_sent
        && raw
            .blocks
            .as_ref()
            .is_some_and(|blocks| is_rich_text_blocks(blocks));
    let is_supported = supported_shape && raw.file_ids.is_empty();
    let file_identity_supported = raw.date_created.is_some_and(|created| created > 0)
        && raw.date_scheduled == Some(0)
        && raw
            .last_updated_client
            .as_deref()
            .is_some_and(is_valid_draft_client)
        && raw.team_id.as_deref().is_some_and(is_valid_team_id)
        && raw.user_id.as_deref().is_some_and(is_valid_user_id);
    let file_shape_supported = supported_shape
        && file_identity_supported
        && !has_unknown_fields
        && raw.file_ids.len() == 1;
    let file_association = (raw.file_ids.len() == 1).then_some(FileDraftAssociation::Unverified);
    Ok(Draft {
        id: raw.id,
        client_msg_id: raw.client_msg_id,
        last_updated_ts,
        client_last_updated_ts,
        text: raw.text,
        blocks: raw.blocks,
        destinations: raw.destinations,
        file_ids: raw.file_ids,
        attachments: raw.attachments,
        is_from_composer: raw.is_from_composer,
        file_association,
        is_supported,
        has_unknown_fields,
        file_shape_supported,
        date_created: raw.date_created,
        date_scheduled: raw.date_scheduled,
        last_updated_client: raw.last_updated_client,
        team_id: raw.team_id,
        user_id: raw.user_id,
    })
}

fn same_draft_route(actual: &DraftDestination, requested: &DraftDestination) -> bool {
    actual.channel_id == requested.channel_id
        && actual.thread_ts == requested.thread_ts
        && actual.broadcast == requested.broadcast
}

fn is_exact_text_draft(
    draft: &Draft,
    client_msg_id: &str,
    destination: &DraftDestination,
    blocks: &[serde_json::Value],
) -> bool {
    draft.is_supported
        && draft.client_msg_id.as_deref() == Some(client_msg_id)
        && draft.file_ids.is_empty()
        && draft.destinations.len() == 1
        && same_draft_route(&draft.destinations[0], destination)
        && draft
            .blocks
            .as_deref()
            .is_some_and(|actual| same_rendered_draft_blocks(actual, blocks))
}

fn draft_mutation_destination(destination: &DraftDestination) -> DraftDestination {
    DraftDestination {
        channel_id: destination.channel_id.clone(),
        thread_ts: destination.thread_ts.clone(),
        broadcast: destination.broadcast,
        ..DraftDestination::default()
    }
}

fn same_draft_snapshot(actual: &Draft, expected: &Draft) -> bool {
    actual.id == expected.id
        && actual.client_msg_id == expected.client_msg_id
        && actual.last_updated_ts == expected.last_updated_ts
        && actual.client_last_updated_ts == expected.client_last_updated_ts
        && actual.text == expected.text
        && actual.blocks == expected.blocks
        && actual.destinations == expected.destinations
        && actual.file_ids == expected.file_ids
        && actual.attachments == expected.attachments
        && actual.is_from_composer == expected.is_from_composer
        && actual.has_unknown_fields == expected.has_unknown_fields
        && actual.file_shape_supported == expected.file_shape_supported
        && actual.date_created == expected.date_created
        && actual.date_scheduled == expected.date_scheduled
        && actual.last_updated_client == expected.last_updated_client
        && actual.team_id == expected.team_id
        && actual.user_id == expected.user_id
}

fn is_private_unshared_file(file: &FileReference) -> bool {
    file.is_external == Some(false)
        && file.mode.as_deref() != Some("external")
        && file.is_public == Some(false)
        && file.public_url_shared == Some(false)
        && file.channel_ids.as_ref().is_some_and(Vec::is_empty)
        && file.group_ids.as_ref().is_some_and(Vec::is_empty)
        && file.im_ids.as_ref().is_some_and(Vec::is_empty)
        && file.shares.as_ref().is_some_and(Vec::is_empty)
        && file.shares_complete
}

fn is_expected_private_draft_file(
    file: &FileReference,
    file_id: &str,
    expected_name: &str,
    expected_size: u64,
    expected_alt_text: Option<&str>,
) -> bool {
    file.id == file_id
        && file.name.as_deref() == Some(expected_name)
        && file.size == Some(expected_size)
        && expected_alt_text.is_none_or(|expected| file.alt_text.as_deref() == Some(expected))
        && is_private_unshared_file(file)
}

fn is_exact_published_file(
    file: &FileReference,
    channel_id: &str,
    message_ts: &str,
    thread_ts: Option<&str>,
) -> bool {
    if file.public_url_shared != Some(false)
        || file.is_external != Some(false)
        || file.mode.as_deref() == Some("external")
        || file.is_public.is_none()
        || !file.shares_complete
    {
        return false;
    }
    let channel_ids = file.channel_ids.as_deref();
    let group_ids = file.group_ids.as_deref();
    let im_ids = file.im_ids.as_deref();
    let shares = file.shares.as_deref();
    if channel_id.starts_with('D') {
        return channel_ids == Some(&[])
            && group_ids == Some(&[])
            && im_ids.is_some_and(|ids| ids == [channel_id])
            && shares.is_some_and(|shares| {
                shares.is_empty()
                    || (shares.len() == 1
                        && shares[0].channel_id == channel_id
                        && shares[0].ts == message_ts
                        && is_exact_file_route(
                            &shares[0].ts,
                            shares[0].thread_ts.as_deref(),
                            thread_ts,
                        ))
            });
    }
    let has_exact_membership = (channel_ids.is_some_and(|ids| ids == [channel_id])
        && group_ids == Some(&[]))
        || (group_ids.is_some_and(|ids| ids == [channel_id]) && channel_ids == Some(&[]));
    has_exact_membership
        && im_ids == Some(&[])
        && shares.is_some_and(|shares| {
            shares.len() == 1
                && shares[0].channel_id == channel_id
                && shares[0].ts == message_ts
                && is_exact_file_route(&shares[0].ts, shares[0].thread_ts.as_deref(), thread_ts)
        })
}

fn normalize_sent_message(
    workspace_url: &url::Url,
    channel_id: &str,
    thread_ts: Option<&str>,
    client_msg_id: String,
    response: RawPostMessageResponse,
) -> Result<SentMessage> {
    if response.channel != channel_id
        || !is_valid_timestamp(&response.ts)
        || response.message.ts != response.ts
        || response.message.thread_ts.as_deref() != thread_ts
    {
        return Err(Error::InvalidResponse {
            method: "chat.postMessage",
        });
    }
    Ok(SentMessage {
        client_msg_id,
        message: normalize_message(
            workspace_url,
            channel_id,
            response.message,
            "chat.postMessage",
        )?,
    })
}

fn is_valid_message_fallback(text: &str) -> bool {
    !text.trim().is_empty()
        && text.len() <= MAX_MARKDOWN_BYTES
        && !text.chars().any(|character| character == '\0')
}

fn is_rich_text_blocks(blocks: &[serde_json::Value]) -> bool {
    !blocks.is_empty()
        && blocks.iter().all(|block| {
            block
                .as_object()
                .and_then(|object| object.get("type"))
                .and_then(serde_json::Value::as_str)
                == Some("rich_text")
        })
}

fn same_rendered_draft_blocks(
    actual: &[serde_json::Value],
    requested: &[serde_json::Value],
) -> bool {
    authored_draft_blocks(actual).as_deref() == Some(requested)
}

fn authored_draft_blocks(blocks: &[serde_json::Value]) -> Option<Vec<serde_json::Value>> {
    if !is_rich_text_blocks(blocks) {
        return None;
    }
    blocks
        .iter()
        .map(|block| {
            let mut block = block.as_object()?.clone();
            if let Some(block_id) = block.remove("block_id")
                && !block_id.as_str().is_some_and(|block_id| {
                    (1..=255).contains(&block_id.len()) && !block_id.chars().any(char::is_control)
                })
            {
                return None;
            }
            Some(serde_json::Value::Object(block))
        })
        .collect()
}

fn rich_text_fallback(blocks: &[serde_json::Value]) -> Option<String> {
    if !is_rich_text_blocks(blocks) {
        return None;
    }
    let mut output = String::new();
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 && !push_bounded(&mut output, "\n") {
            return None;
        }
        if !append_rich_text_node(block, &mut output, 0) {
            return None;
        }
    }
    let output = output.trim().to_owned();
    is_valid_message_fallback(&output).then_some(output)
}

fn append_rich_text_node(node: &serde_json::Value, output: &mut String, depth: usize) -> bool {
    if depth > 64 {
        return false;
    }
    let kind = node.get("type").and_then(serde_json::Value::as_str);
    match kind {
        Some("text") => node
            .get("text")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| push_bounded(output, text)),
        Some("link") => node
            .get("text")
            .or_else(|| node.get("url"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| push_bounded(output, text)),
        Some("emoji") => node
            .get("name")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|name| push_bounded(output, &format!(":{name}:"))),
        Some("user") => node
            .get("user_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| push_bounded(output, &format!("<@{id}>"))),
        Some("channel") => node
            .get("channel_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| push_bounded(output, &format!("<#{id}>"))),
        Some("usergroup") => node
            .get("usergroup_id")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| push_bounded(output, &format!("<!subteam^{id}>"))),
        Some("broadcast") => node
            .get("range")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|range| push_bounded(output, &format!("@{range}"))),
        Some("date") => node
            .get("fallback")
            .or_else(|| node.get("timestamp"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| push_bounded(output, text)),
        Some("color") => node
            .get("value")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|text| push_bounded(output, text)),
        Some("rich_text_list") => {
            let Some(elements) = node.get("elements").and_then(serde_json::Value::as_array) else {
                return false;
            };
            let ordered = node.get("style").and_then(serde_json::Value::as_str) == Some("ordered");
            let indent = node
                .get("indent")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0)
                .min(8) as usize;
            for (index, element) in elements.iter().enumerate() {
                if index > 0 && !push_bounded(output, "\n") {
                    return false;
                }
                if !push_bounded(output, &"  ".repeat(indent)) {
                    return false;
                }
                let marker = if ordered {
                    format!("{}. ", index + 1)
                } else {
                    "- ".into()
                };
                if !push_bounded(output, &marker)
                    || !append_rich_text_node(element, output, depth + 1)
                {
                    return false;
                }
            }
            true
        }
        Some("rich_text") => append_rich_text_children(node, output, depth, "\n"),
        Some("rich_text_section" | "rich_text_quote" | "rich_text_preformatted") => {
            append_rich_text_children(node, output, depth, "")
        }
        _ => {
            if let Some(text) = node.get("text").and_then(serde_json::Value::as_str) {
                push_bounded(output, text)
            } else {
                append_rich_text_children(node, output, depth, "")
            }
        }
    }
}

fn append_rich_text_children(
    node: &serde_json::Value,
    output: &mut String,
    depth: usize,
    separator: &str,
) -> bool {
    let Some(elements) = node.get("elements").and_then(serde_json::Value::as_array) else {
        return false;
    };
    for (index, element) in elements.iter().enumerate() {
        if index > 0 && !separator.is_empty() && !push_bounded(output, separator) {
            return false;
        }
        if !append_rich_text_node(element, output, depth + 1) {
            return false;
        }
    }
    true
}

fn push_bounded(output: &mut String, value: &str) -> bool {
    if output.len().saturating_add(value.len()) > MAX_MARKDOWN_BYTES {
        false
    } else {
        output.push_str(value);
        true
    }
}

fn classify_publication_error(client_msg_id: &str, error: Error) -> Error {
    if mutation_error_is_ambiguous(&error) {
        Error::PublicationUncertain {
            client_msg_id: client_msg_id.to_owned(),
        }
    } else {
        error
    }
}

fn require_supported_draft(draft: &Draft) -> Result<()> {
    if draft.is_supported {
        Ok(())
    } else {
        Err(Error::invalid_input(
            "draft",
            "uses unsupported destinations, files, attachments, or content",
        ))
    }
}

fn normalize_attachments(
    attachments: Option<Vec<serde_json::Value>>,
    method: &'static str,
) -> Result<Option<Vec<serde_json::Value>>> {
    if attachments
        .as_ref()
        .is_some_and(|attachments| attachments.len() > MAX_ATTACHMENTS_PER_MESSAGE)
    {
        return Err(Error::InvalidResponse { method });
    }
    Ok(attachments)
}

fn normalize_reactions(reactions: Vec<RawReaction>, method: &'static str) -> Result<Vec<Reaction>> {
    if reactions.len() > MAX_REACTIONS_PER_MESSAGE {
        return Err(Error::InvalidResponse { method });
    }
    let mut names = HashSet::with_capacity(reactions.len());
    reactions
        .into_iter()
        .map(|reaction| {
            let name = validate_reaction_name(&reaction.name)
                .map_err(|_| Error::InvalidResponse { method })?;
            if !names.insert(name.clone())
                || reaction.users.len() > MAX_REACTION_USERS
                || reaction.count < reaction.users.len() as u64
                || reaction
                    .users
                    .iter()
                    .any(|user_id| !is_valid_user_id(user_id))
            {
                return Err(Error::InvalidResponse { method });
            }
            let mut user_ids = reaction.users;
            user_ids.sort();
            user_ids.dedup();
            if reaction.count < user_ids.len() as u64 {
                return Err(Error::InvalidResponse { method });
            }
            Ok(Reaction {
                name,
                count: reaction.count,
                user_ids_complete: reaction.count == user_ids.len() as u64,
                user_ids,
            })
        })
        .collect()
}

fn normalize_files(files: Vec<RawFile>, method: &'static str) -> Result<Vec<FileReference>> {
    if files.len() > MAX_FILES_PER_MESSAGE {
        return Err(Error::InvalidResponse { method });
    }
    files
        .into_iter()
        .map(|file| normalize_file(file, method))
        .collect()
}

fn normalize_file(raw: RawFile, method: &'static str) -> Result<FileReference> {
    if !is_valid_file_id(&raw.id)
        || !is_bounded_optional_text(raw.name.as_deref(), 1_024)
        || !is_bounded_optional_text(raw.title.as_deref(), 1_024)
        || !is_bounded_optional_text(raw.alt_txt.as_deref(), MAX_FILE_UPLOAD_ALT_TEXT_BYTES)
        || !is_bounded_optional_text(raw.mimetype.as_deref(), 256)
        || !is_bounded_optional_text(raw.filetype.as_deref(), 128)
        || !is_bounded_optional_text(raw.pretty_type.as_deref(), 256)
        || !is_bounded_optional_text(raw.mode.as_deref(), 64)
        || !is_bounded_optional_text(raw.file_access.as_deref(), 128)
        || raw
            .user
            .as_deref()
            .is_some_and(|user_id| !user_id.is_empty() && !is_valid_user_id(user_id))
    {
        return Err(Error::InvalidResponse { method });
    }
    for candidate in [
        raw.url_private.as_deref(),
        raw.url_private_download.as_deref(),
        raw.permalink.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        if !is_safe_metadata_url(candidate) {
            return Err(Error::InvalidResponse { method });
        }
    }

    let channel_ids = normalize_file_conversation_ids(raw.channels, b"C", method)?;
    let group_ids = normalize_file_conversation_ids(raw.groups, b"CG", method)?;
    let im_ids = normalize_file_conversation_ids(raw.ims, b"D", method)?;
    let shares = if let Some(raw_shares) = raw.shares {
        let share_channels = raw_shares
            .public
            .len()
            .saturating_add(raw_shares.private.len());
        let share_count = raw_shares
            .public
            .values()
            .chain(raw_shares.private.values())
            .map(Vec::len)
            .sum::<usize>();
        if share_channels > MAX_FILE_SHARES || share_count > MAX_FILE_SHARES {
            return Err(Error::InvalidResponse { method });
        }
        let mut shares = Vec::with_capacity(share_count);
        append_file_shares(
            &mut shares,
            raw_shares.public,
            FileShareVisibility::Public,
            method,
        )?;
        append_file_shares(
            &mut shares,
            raw_shares.private,
            FileShareVisibility::Private,
            method,
        )?;
        shares.sort_by(|left, right| {
            left.channel_id
                .cmp(&right.channel_id)
                .then_with(|| left.ts.cmp(&right.ts))
                .then_with(|| left.thread_ts.cmp(&right.thread_ts))
        });
        Some(shares)
    } else {
        None
    };
    let shares_complete =
        shares.is_some() && raw.has_more_shares != Some(true) && raw.skipped_shares != Some(true);

    Ok(FileReference {
        id: raw.id,
        name: raw.name,
        title: raw.title,
        alt_text: raw.alt_txt,
        mimetype: raw.mimetype,
        filetype: raw.filetype,
        pretty_type: raw.pretty_type,
        mode: raw.mode,
        file_access: raw.file_access,
        uploader_id: raw.user.filter(|user_id| !user_id.is_empty()),
        size: raw.size,
        created: raw.created,
        timestamp: raw.timestamp,
        editable: raw.editable,
        is_external: raw.is_external,
        is_public: raw.is_public,
        public_url_shared: raw.public_url_shared,
        private_url: raw.url_private,
        download_url: raw.url_private_download,
        permalink: raw.permalink,
        channel_ids,
        group_ids,
        im_ids,
        shares,
        shares_complete,
    })
}

fn normalize_file_conversation_ids(
    ids: Option<Vec<String>>,
    prefixes: &[u8],
    method: &'static str,
) -> Result<Option<Vec<String>>> {
    let Some(ids) = ids else {
        return Ok(None);
    };
    if ids.len() > MAX_FILE_CONVERSATIONS {
        return Err(Error::InvalidResponse { method });
    }
    let mut seen = HashSet::with_capacity(ids.len());
    for id in &ids {
        if !id
            .as_bytes()
            .first()
            .is_some_and(|prefix| prefixes.contains(prefix))
            || !is_valid_any_conversation_id(id)
            || !seen.insert(id.clone())
        {
            return Err(Error::InvalidResponse { method });
        }
    }
    Ok(Some(ids))
}

fn exact_file_share(
    file: &FileReference,
    channel_id: &str,
    thread_ts: Option<&str>,
) -> Option<FileShare> {
    let mut matches = file.shares.as_ref()?.iter().filter(|share| {
        share.channel_id == channel_id
            && is_exact_file_route(&share.ts, share.thread_ts.as_deref(), thread_ts)
    });
    let first = matches.next()?.clone();
    matches.next().is_none().then_some(first)
}

fn is_exact_file_route(
    message_ts: &str,
    actual_thread_ts: Option<&str>,
    requested_thread_ts: Option<&str>,
) -> bool {
    match requested_thread_ts {
        Some(requested_thread_ts) => {
            actual_thread_ts == Some(requested_thread_ts) && message_ts != requested_thread_ts
        }
        None => actual_thread_ts.is_none() || actual_thread_ts == Some(message_ts),
    }
}

fn append_file_shares(
    target: &mut Vec<FileShare>,
    source: std::collections::BTreeMap<String, Vec<crate::model::RawFileShare>>,
    visibility: FileShareVisibility,
    method: &'static str,
) -> Result<()> {
    for (channel_id, shares) in source {
        if !is_valid_any_conversation_id(&channel_id) {
            return Err(Error::InvalidResponse { method });
        }
        for share in shares {
            if !is_valid_timestamp(&share.ts)
                || share
                    .thread_ts
                    .as_deref()
                    .is_some_and(|thread_ts| !is_valid_timestamp(thread_ts))
            {
                return Err(Error::InvalidResponse { method });
            }
            target.push(FileShare {
                visibility,
                channel_id: channel_id.clone(),
                ts: share.ts,
                thread_ts: share.thread_ts,
            });
        }
    }
    Ok(())
}

fn normalize_custom_emoji(name: String, value: String) -> Result<CustomEmoji> {
    let name = validate_emoji_name(&name).map_err(|_| Error::InvalidResponse {
        method: "emoji.list",
    })?;
    if let Some(alias) = value.strip_prefix("alias:") {
        return Ok(CustomEmoji {
            name,
            kind: CustomEmojiKind::Alias,
            image_url: None,
            alias_for: Some(
                validate_emoji_name(alias).map_err(|_| Error::InvalidResponse {
                    method: "emoji.list",
                })?,
            ),
        });
    }
    if !is_safe_metadata_url(&value) {
        return Err(Error::InvalidResponse {
            method: "emoji.list",
        });
    }
    Ok(CustomEmoji {
        name,
        kind: CustomEmojiKind::Image,
        image_url: Some(value),
        alias_for: None,
    })
}

fn initial_mention_fields(
    text: &str,
    blocks: Option<&[serde_json::Value]>,
) -> (String, MentionResolution, Vec<MessageMention>) {
    let selection = select_message_mentions(text, blocks);
    let resolution = if selection.ids.is_empty() && !selection.truncated {
        MentionResolution::NotNeeded
    } else {
        MentionResolution::NotAttempted
    };
    let mentions = selection
        .ids
        .into_iter()
        .map(|id| MessageMention {
            id,
            username: None,
            display_name: None,
        })
        .collect();
    (text.to_owned(), resolution, mentions)
}

fn select_message_mentions(text: &str, blocks: Option<&[serde_json::Value]>) -> MentionSelection {
    if let Some(blocks) = blocks
        && let Some(rendered) = render_rich_text_mentions(blocks, None)
    {
        return MentionSelection {
            ids: rendered.ids,
            truncated: rendered.mentions_truncated,
            source: MentionSource::RichText,
        };
    }
    let (ids, truncated) = scan_canonical_mentions(text);
    MentionSelection {
        ids,
        truncated,
        source: MentionSource::Canonical,
    }
}

fn scan_canonical_mentions(text: &str) -> (Vec<String>, bool) {
    if text.len() > MAX_MARKDOWN_BYTES {
        return (Vec::new(), true);
    }
    let mut ids = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut truncated = false;
    let mut index = 0;
    while index < text.len() {
        if text.as_bytes()[index] == b'`' {
            let run = backtick_run(text, index);
            if let Some(close) = matching_backtick_run(text, index + run, run) {
                index = close + run;
                continue;
            }
            index += run;
            continue;
        }
        if text[index..].starts_with("<@")
            && let Some(relative_end) = text[index + 2..].find('>')
        {
            let end = index + 2 + relative_end;
            let id = &text[index + 2..end];
            if is_valid_user_id(id) {
                record_mention_id(&mut ids, &mut seen_ids, id, &mut truncated);
                index = end + 1;
                continue;
            }
        }
        index += text[index..]
            .chars()
            .next()
            .expect("index stays on a character boundary")
            .len_utf8();
    }
    (ids, truncated)
}

fn backtick_run(text: &str, start: usize) -> usize {
    text.as_bytes()[start..]
        .iter()
        .take_while(|byte| **byte == b'`')
        .count()
}

fn matching_backtick_run(text: &str, mut index: usize, wanted: usize) -> Option<usize> {
    while index < text.len() {
        if text.as_bytes()[index] == b'`' {
            let run = backtick_run(text, index);
            if run == wanted {
                return Some(index);
            }
            index += run;
        } else {
            index += text[index..]
                .chars()
                .next()
                .expect("index stays on a character boundary")
                .len_utf8();
        }
    }
    None
}

fn record_mention_id(
    ids: &mut Vec<String>,
    seen_ids: &mut HashSet<String>,
    id: &str,
    truncated: &mut bool,
) {
    if !seen_ids.insert(id.to_owned()) {
        return;
    }
    if ids.len() == MAX_MESSAGE_MENTIONS {
        *truncated = true;
    } else {
        ids.push(id.to_owned());
    }
}

fn render_canonical_mentions(text: &str, labels: &HashMap<String, String>) -> Option<String> {
    if text.len() > MAX_MARKDOWN_BYTES {
        return None;
    }
    let mut output = String::with_capacity(text.len());
    let mut index = 0;
    while index < text.len() {
        if text.as_bytes()[index] == b'`' {
            let run = backtick_run(text, index);
            if let Some(close) = matching_backtick_run(text, index + run, run) {
                if !push_bounded(&mut output, &text[index..close + run]) {
                    return None;
                }
                index = close + run;
                continue;
            }
            if !push_bounded(&mut output, &text[index..index + run]) {
                return None;
            }
            index += run;
            continue;
        }
        if text[index..].starts_with("<@")
            && let Some(relative_end) = text[index + 2..].find('>')
        {
            let end = index + 2 + relative_end;
            let id = &text[index + 2..end];
            if is_valid_user_id(id) {
                let rendered = labels
                    .get(id)
                    .map(|label| format!("@{label}"))
                    .unwrap_or_else(|| text[index..=end].to_owned());
                if !push_bounded(&mut output, &rendered) {
                    return None;
                }
                index = end + 1;
                continue;
            }
        }
        let character = text[index..]
            .chars()
            .next()
            .expect("index stays on a character boundary");
        if !push_bounded(&mut output, &text[index..index + character.len_utf8()]) {
            return None;
        }
        index += character.len_utf8();
    }
    Some(output)
}

fn render_rich_text_mentions(
    blocks: &[serde_json::Value],
    labels: Option<&HashMap<String, String>>,
) -> Option<RichTextMentionOutput> {
    if !is_rich_text_blocks(blocks) {
        return None;
    }
    let mut render = RichTextMentionRender {
        output: String::new(),
        labels,
        ids: Vec::new(),
        seen_ids: HashSet::new(),
        mentions_truncated: false,
        render_limited: false,
        nodes: 0,
    };
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            render.push("\n");
        }
        if !append_strict_rich_text_node(block, &mut render, 0, false, RichTextNodeContext::Root) {
            return None;
        }
    }
    Some(RichTextMentionOutput {
        rendered_text: (!render.render_limited).then_some(render.output),
        ids: render.ids,
        mentions_truncated: render.mentions_truncated,
    })
}

impl RichTextMentionRender<'_> {
    fn push(&mut self, value: &str) {
        if self.render_limited {
            return;
        }
        if self.output.len().saturating_add(value.len()) > MAX_MARKDOWN_BYTES {
            self.render_limited = true;
            self.output.clear();
        } else {
            self.output.push_str(value);
        }
    }

    fn record(&mut self, id: &str) {
        record_mention_id(
            &mut self.ids,
            &mut self.seen_ids,
            id,
            &mut self.mentions_truncated,
        );
    }
}

fn append_strict_rich_text_node(
    node: &serde_json::Value,
    render: &mut RichTextMentionRender<'_>,
    depth: usize,
    in_code: bool,
    context: RichTextNodeContext,
) -> bool {
    if depth > MAX_RICH_TEXT_RENDER_DEPTH || render.nodes == MAX_RICH_TEXT_RENDER_NODES {
        return false;
    }
    render.nodes += 1;
    let Some(kind) = node.get("type").and_then(serde_json::Value::as_str) else {
        return false;
    };
    let valid_placement = match context {
        RichTextNodeContext::Root => kind == "rich_text",
        RichTextNodeContext::BlockElement => matches!(
            kind,
            "rich_text_section" | "rich_text_list" | "rich_text_quote" | "rich_text_preformatted"
        ),
        RichTextNodeContext::ListItem => kind == "rich_text_section",
        RichTextNodeContext::Inline => matches!(
            kind,
            "text"
                | "link"
                | "emoji"
                | "user"
                | "channel"
                | "usergroup"
                | "broadcast"
                | "date"
                | "color"
        ),
    };
    if !valid_placement {
        return false;
    }
    let style_is_code = if matches!(context, RichTextNodeContext::Inline) {
        let Some(style_is_code) = strict_inline_code_style(node) else {
            return false;
        };
        style_is_code
    } else {
        false
    };
    let in_code = in_code || style_is_code;
    match kind {
        "text" => {
            let Some(text) = node.get("text").and_then(serde_json::Value::as_str) else {
                return false;
            };
            render.push(text);
            true
        }
        "link" => {
            let Some(text) = node
                .get("text")
                .or_else(|| node.get("url"))
                .and_then(serde_json::Value::as_str)
            else {
                return false;
            };
            render.push(text);
            true
        }
        "emoji" => {
            let Some(name) = node.get("name").and_then(serde_json::Value::as_str) else {
                return false;
            };
            render.push(&format!(":{name}:"));
            true
        }
        "user" => {
            let Some(id) = node.get("user_id").and_then(serde_json::Value::as_str) else {
                return false;
            };
            if !is_valid_user_id(id) {
                return false;
            }
            if !in_code {
                render.record(id);
            }
            let value = if in_code {
                format!("<@{id}>")
            } else {
                render
                    .labels
                    .and_then(|labels| labels.get(id))
                    .map(|label| format!("@{label}"))
                    .unwrap_or_else(|| format!("<@{id}>"))
            };
            render.push(&value);
            true
        }
        "channel" => {
            let Some(id) = node
                .get("channel_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| is_valid_any_conversation_id(id))
            else {
                return false;
            };
            render.push(&format!("<#{id}>"));
            true
        }
        "usergroup" => {
            let Some(id) = node
                .get("usergroup_id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| {
                    !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control)
                })
            else {
                return false;
            };
            render.push(&format!("<!subteam^{id}>"));
            true
        }
        "broadcast" => {
            let Some(range) = node
                .get("range")
                .and_then(serde_json::Value::as_str)
                .filter(|value| is_valid_author_label(value))
            else {
                return false;
            };
            render.push(&format!("@{range}"));
            true
        }
        "date" => {
            let Some(text) = node
                .get("fallback")
                .or_else(|| node.get("timestamp"))
                .and_then(serde_json::Value::as_str)
            else {
                return false;
            };
            render.push(text);
            true
        }
        "color" => {
            let Some(value) = node.get("value").and_then(serde_json::Value::as_str) else {
                return false;
            };
            render.push(value);
            true
        }
        "rich_text_list" => {
            let Some(elements) = node.get("elements").and_then(serde_json::Value::as_array) else {
                return false;
            };
            let Some(style) = node.get("style").and_then(serde_json::Value::as_str) else {
                return false;
            };
            if !matches!(style, "ordered" | "bullet") {
                return false;
            }
            let offset = match (style, node.get("offset")) {
                ("ordered", Some(offset)) => {
                    let Some(offset) = offset.as_u64() else {
                        return false;
                    };
                    offset
                }
                ("ordered", None) | ("bullet", None) => 0,
                ("bullet", Some(_)) => return false,
                _ => unreachable!("validated rich-text list style"),
            };
            let indent = match node.get("indent") {
                Some(indent) => {
                    let Some(indent) = indent.as_u64().filter(|indent| *indent <= 8) else {
                        return false;
                    };
                    indent as usize
                }
                None => 0,
            };
            for (index, element) in elements.iter().enumerate() {
                if index > 0 {
                    render.push("\n");
                }
                render.push(&"  ".repeat(indent));
                if style == "ordered" {
                    let Some(number) = offset
                        .checked_add(index as u64)
                        .and_then(|value| value.checked_add(1))
                    else {
                        return false;
                    };
                    render.push(&format!("{number}. "));
                } else {
                    render.push("- ");
                }
                if !append_strict_rich_text_node(
                    element,
                    render,
                    depth + 1,
                    in_code,
                    RichTextNodeContext::ListItem,
                ) {
                    return false;
                }
            }
            true
        }
        "rich_text" => append_strict_rich_text_children(
            node,
            render,
            depth,
            in_code,
            "\n",
            RichTextNodeContext::BlockElement,
        ),
        "rich_text_section" | "rich_text_quote" => append_strict_rich_text_children(
            node,
            render,
            depth,
            in_code,
            "",
            RichTextNodeContext::Inline,
        ),
        "rich_text_preformatted" => append_strict_rich_text_children(
            node,
            render,
            depth,
            true,
            "",
            RichTextNodeContext::Inline,
        ),
        _ => false,
    }
}

fn strict_inline_code_style(node: &serde_json::Value) -> Option<bool> {
    let Some(style) = node.get("style") else {
        return Some(false);
    };
    let style = style.as_object()?;
    if style.values().any(|value| !value.is_boolean()) {
        return None;
    }
    Some(
        style
            .get("code")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
    )
}

fn append_strict_rich_text_children(
    node: &serde_json::Value,
    render: &mut RichTextMentionRender<'_>,
    depth: usize,
    in_code: bool,
    separator: &str,
    child_context: RichTextNodeContext,
) -> bool {
    let Some(elements) = node.get("elements").and_then(serde_json::Value::as_array) else {
        return false;
    };
    for (index, element) in elements.iter().enumerate() {
        if index > 0 && !separator.is_empty() {
            render.push(separator);
        }
        if !append_strict_rich_text_node(element, render, depth + 1, in_code, child_context) {
            return false;
        }
    }
    true
}

fn enrich_mentions(
    text: &str,
    blocks: Option<&[serde_json::Value]>,
    rendered_text: &mut String,
    mention_resolution: &mut MentionResolution,
    mentions: &mut Vec<MessageMention>,
    directory: &AuthorDirectory,
) {
    let selection = select_message_mentions(text, blocks);
    if selection.ids.is_empty() && !selection.truncated {
        *rendered_text = text.to_owned();
        *mention_resolution = MentionResolution::NotNeeded;
        mentions.clear();
        return;
    }

    let (users, interrupted) = match directory {
        AuthorDirectory::Loaded(directory) => (directory, false),
        AuthorDirectory::Interrupted(directory) => (directory, true),
    };
    let mut labels = HashMap::new();
    let mut partial = selection.truncated;
    let mut unavailable = false;
    *mentions = selection
        .ids
        .iter()
        .map(|id| {
            if users.conflicting_ids.contains(id) {
                unavailable = true;
                return MessageMention {
                    id: id.clone(),
                    username: None,
                    display_name: None,
                };
            }
            let Some(user) = users.users.get(id) else {
                if interrupted {
                    unavailable = true;
                } else {
                    partial = true;
                }
                return MessageMention {
                    id: id.clone(),
                    username: None,
                    display_name: None,
                };
            };
            let username = user.name.clone();
            let display_name = user.display_name.clone().or_else(|| user.real_name.clone());
            if let Some(label) = username.as_ref().or(display_name.as_ref()) {
                labels.insert(id.clone(), label.clone());
            } else {
                partial = true;
            }
            MessageMention {
                id: id.clone(),
                username,
                display_name,
            }
        })
        .collect();

    let rendered = if selection.truncated {
        None
    } else {
        match selection.source {
            MentionSource::Canonical => render_canonical_mentions(text, &labels),
            MentionSource::RichText => blocks
                .and_then(|blocks| render_rich_text_mentions(blocks, Some(&labels)))
                .and_then(|rendered| rendered.rendered_text),
        }
    };
    if let Some(rendered) = rendered {
        *rendered_text = rendered;
    } else {
        *rendered_text = text.to_owned();
        partial = true;
    }
    *mention_resolution = if unavailable {
        MentionResolution::Unavailable
    } else if partial {
        MentionResolution::Partial
    } else {
        MentionResolution::Complete
    };
}

fn message_author_needs_directory(message: &Message) -> bool {
    message.author_resolution == AuthorResolution::NotAttempted
}

fn message_mentions_need_directory(message: &Message) -> bool {
    message.mention_resolution == MentionResolution::NotAttempted
}

fn message_needs_directory(message: &Message) -> bool {
    message_author_needs_directory(message) || message_mentions_need_directory(message)
}

fn author_directory_users(directory: &AuthorDirectory) -> &HashMap<String, User> {
    match directory {
        AuthorDirectory::Loaded(directory) | AuthorDirectory::Interrupted(directory) => {
            &directory.users
        }
    }
}

fn author_directory_conflicts(directory: &AuthorDirectory, user_id: &str) -> bool {
    match directory {
        AuthorDirectory::Loaded(directory) | AuthorDirectory::Interrupted(directory) => {
            directory.conflicting_ids.contains(user_id)
        }
    }
}

fn enrich_messages_from_directory(messages: &mut [Message], directory: &AuthorDirectory) {
    for message in messages {
        if message_author_needs_directory(message) {
            enrich_author(
                message.author_id.as_deref(),
                &mut message.author_name,
                &mut message.author_display_name,
                &mut message.author_resolution,
                directory,
            );
        }
        if message_mentions_need_directory(message) {
            enrich_mentions(
                &message.text,
                message.blocks.as_deref(),
                &mut message.rendered_text,
                &mut message.mention_resolution,
                &mut message.mentions,
                directory,
            );
        }
    }
}

fn search_author_needs_directory(message: &MessageSearchMatch) -> bool {
    message.author_resolution == AuthorResolution::NotAttempted
}

fn search_mentions_need_directory(message: &MessageSearchMatch) -> bool {
    message.mention_resolution == MentionResolution::NotAttempted
}

fn search_message_needs_directory(message: &MessageSearchMatch) -> bool {
    search_author_needs_directory(message) || search_mentions_need_directory(message)
}

fn enrich_search_messages_from_directory(
    messages: &mut [MessageSearchMatch],
    directory: &AuthorDirectory,
) {
    for message in messages {
        if search_author_needs_directory(message) {
            enrich_author(
                message.author_id.as_deref(),
                &mut message.author_name,
                &mut message.author_display_name,
                &mut message.author_resolution,
                directory,
            );
        }
        if search_mentions_need_directory(message) {
            enrich_mentions(
                &message.text,
                message.blocks.as_deref(),
                &mut message.rendered_text,
                &mut message.mention_resolution,
                &mut message.mentions,
                directory,
            );
        }
    }
}

fn enrich_author(
    author_id: Option<&str>,
    author_name: &mut Option<String>,
    author_display_name: &mut Option<String>,
    author_resolution: &mut AuthorResolution,
    directory: &AuthorDirectory,
) {
    let Some(author_id) = author_id else {
        *author_resolution = AuthorResolution::Unknown;
        return;
    };
    let (directory, missing_resolution) = match directory {
        AuthorDirectory::Loaded(directory) => (
            directory,
            if directory.complete {
                AuthorResolution::Unresolved
            } else {
                AuthorResolution::Incomplete
            },
        ),
        AuthorDirectory::Interrupted(directory) => (directory, AuthorResolution::Unavailable),
    };
    if directory.conflicting_ids.contains(author_id) {
        *author_resolution = AuthorResolution::Unavailable;
        return;
    }
    if let Some(user) = directory.users.get(author_id) {
        *author_name = user.name.clone();
        *author_display_name = user.display_name.clone().or_else(|| user.real_name.clone());
        if author_name.is_some() || author_display_name.is_some() {
            *author_resolution = AuthorResolution::Directory;
            return;
        }
    }
    *author_resolution = missing_resolution;
}

fn normalize_message_author_name(author_name: Option<String>) -> Option<String> {
    let author_name = author_name?;
    if author_name.trim().is_empty() || !is_valid_author_label(&author_name) {
        return None;
    }
    Some(author_name)
}

fn is_valid_author_label(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
}

fn initial_author_resolution(
    author_id: &Option<String>,
    author_name: &Option<String>,
) -> AuthorResolution {
    if author_name.is_some() {
        AuthorResolution::Provided
    } else if author_id.is_some() {
        AuthorResolution::NotAttempted
    } else {
        AuthorResolution::Unknown
    }
}

fn canonical_permalink_timestamp(timestamp: &str) -> Option<String> {
    if !is_valid_timestamp(timestamp) {
        return None;
    }
    let (seconds, fraction) = timestamp.split_once('.')?;
    if fraction.len() > 6 {
        return None;
    }
    let mut canonical = String::with_capacity(seconds.len() + 7);
    canonical.push_str(seconds);
    canonical.push('.');
    canonical.push_str(fraction);
    canonical.extend(std::iter::repeat_n('0', 6 - fraction.len()));
    Some(canonical)
}

fn canonical_permalink(
    workspace_url: &url::Url,
    channel_id: &str,
    message_ts: &str,
    thread_ts: Option<&str>,
) -> Option<String> {
    if !crate::config::is_valid_workspace_origin(workspace_url)
        || !is_valid_any_conversation_id(channel_id)
    {
        return None;
    }
    let message_ts = canonical_permalink_timestamp(message_ts)?;
    let mut permalink = workspace_url.clone();
    let path_timestamp = message_ts.replace('.', "");
    permalink
        .path_segments_mut()
        .ok()?
        .clear()
        .push("archives")
        .push(channel_id)
        .push(&format!("p{path_timestamp}"));
    permalink.set_query(None);
    permalink.set_fragment(None);
    if let Some(thread_ts) = thread_ts {
        let thread_ts = canonical_permalink_timestamp(thread_ts)?;
        permalink
            .query_pairs_mut()
            .append_pair("thread_ts", &thread_ts)
            .append_pair("cid", channel_id);
    }
    Some(permalink.into())
}

fn search_permalink_route(
    workspace_url: &url::Url,
    channel_id: &str,
    message_ts: &str,
    candidate: &str,
) -> Option<SearchPermalinkRoute> {
    if candidate.is_empty()
        || candidate.len() > 8_192
        || candidate.chars().any(char::is_control)
        || !crate::config::is_valid_workspace_origin(workspace_url)
        || !is_valid_any_conversation_id(channel_id)
    {
        return None;
    }
    let message_ts = canonical_permalink_timestamp(message_ts)?;
    let candidate = url::Url::parse(candidate).ok()?;
    if candidate.origin() != workspace_url.origin()
        || !candidate.username().is_empty()
        || candidate.password().is_some()
        || candidate.fragment().is_some()
    {
        return None;
    }
    let expected_path = format!("/archives/{channel_id}/p{}", message_ts.replace('.', ""));
    if candidate.path() != expected_path {
        return None;
    }
    let Some(query) = candidate.query() else {
        return Some(SearchPermalinkRoute::Root);
    };
    if query.is_empty() {
        return None;
    }
    let mut thread_ts = None;
    let mut cid_matches = false;
    for (name, value) in candidate.query_pairs() {
        match name.as_ref() {
            "thread_ts" if thread_ts.is_none() => {
                thread_ts = canonical_permalink_timestamp(&value);
                thread_ts.as_ref()?;
            }
            "cid" if !cid_matches && value == channel_id => cid_matches = true,
            _ => return None,
        }
    }
    let thread_ts = thread_ts?;
    if !cid_matches || thread_ts == message_ts {
        return None;
    }
    Some(SearchPermalinkRoute::Reply { thread_ts })
}

fn permalink_resolution(
    permalink: &Option<String>,
    thread_root_permalink: &Option<String>,
    thread_root_applicable: bool,
) -> PermalinkResolution {
    match (
        permalink.is_some(),
        !thread_root_applicable || thread_root_permalink.is_some(),
        thread_root_permalink.is_some(),
    ) {
        (true, true, _) => PermalinkResolution::Complete,
        (true, false, _) | (false, _, true) => PermalinkResolution::Partial,
        (false, _, false) => PermalinkResolution::Unavailable,
    }
}

fn message_permalinks(
    workspace_url: &url::Url,
    channel_id: &str,
    message_ts: &str,
    thread_ts: Option<&str>,
) -> MessagePermalinks {
    let canonical_message_ts = canonical_permalink_timestamp(message_ts);
    let canonical_thread_ts = thread_ts.and_then(canonical_permalink_timestamp);
    let is_self_threaded_root = thread_ts.is_some_and(|thread_ts| {
        thread_ts == message_ts
            || canonical_message_ts
                .as_deref()
                .zip(canonical_thread_ts.as_deref())
                .is_some_and(|(message, thread)| message == thread)
    });
    let thread_root_ts = if thread_ts.is_some() && !is_self_threaded_root {
        thread_ts
    } else {
        None
    };
    let permalink = canonical_permalink(workspace_url, channel_id, message_ts, thread_root_ts);
    let thread_root_permalink =
        thread_root_ts.and_then(|root| canonical_permalink(workspace_url, channel_id, root, None));
    let resolution =
        permalink_resolution(&permalink, &thread_root_permalink, thread_root_ts.is_some());
    MessagePermalinks {
        permalink,
        thread_root_permalink,
        resolution,
    }
}

fn normalize_message(
    workspace_url: &url::Url,
    channel: &str,
    message: RawMessage,
    method: &'static str,
) -> Result<Message> {
    let author_id = message
        .user
        .or(message.bot_id)
        .filter(|value| !value.is_empty());
    if author_id
        .as_deref()
        .is_some_and(|value| !is_valid_user_id(value))
    {
        return Err(Error::InvalidResponse { method });
    }
    let author_name = normalize_message_author_name(message.username);
    let author_resolution = initial_author_resolution(&author_id, &author_name);
    let (rendered_text, mention_resolution, mentions) =
        initial_mention_fields(&message.text, message.blocks.as_deref());
    let permalinks = message_permalinks(
        workspace_url,
        channel,
        &message.ts,
        message.thread_ts.as_deref(),
    );
    Ok(Message {
        channel_id: channel.to_owned(),
        ts: message.ts,
        thread_ts: message.thread_ts,
        permalink: permalinks.permalink,
        thread_root_permalink: permalinks.thread_root_permalink,
        permalink_resolution: permalinks.resolution,
        author_id,
        author_name,
        author_display_name: None,
        author_resolution,
        text: message.text,
        rendered_text,
        mention_resolution,
        mentions,
        blocks: message.blocks,
        attachments: normalize_attachments(message.attachments, method)?,
        reply_count: message.reply_count,
        latest_reply: message.latest_reply,
        reactions: normalize_reactions(message.reactions, method)?,
        files: normalize_files(message.files, method)?,
    })
}

fn mutation_error_is_ambiguous(error: &Error) -> bool {
    matches!(
        error,
        Error::HttpStatus { .. }
            | Error::ResponseTooLarge { .. }
            | Error::InvalidResponse { .. }
            | Error::Timeout { .. }
            | Error::Transport { .. }
    ) || matches!(
        error,
        Error::SlackApi { code, .. } if matches!(code.as_str(), "fatal_error" | "internal_error")
    )
}

fn validate_file_id(file_id: &str) -> Result<()> {
    if is_valid_file_id(file_id) {
        Ok(())
    } else {
        Err(Error::invalid_input(
            "file_id",
            "must be a Slack file identifier",
        ))
    }
}

fn is_valid_file_id(file_id: &str) -> bool {
    file_id.starts_with('F')
        && (2..=64).contains(&file_id.len())
        && file_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn validate_emoji_name(name: &str) -> Result<String> {
    let name = name.trim_matches(':');
    if is_valid_basic_emoji_name(name) {
        Ok(name.to_owned())
    } else {
        Err(Error::invalid_input(
            "name",
            "must be a 1 to 100 character Slack emoji name",
        ))
    }
}

fn validate_reaction_name(name: &str) -> Result<String> {
    let normalized = name.trim_matches(':');
    let valid = if let Some((base, tone)) = normalized.split_once("::skin-tone-") {
        !base.contains(':')
            && is_valid_basic_emoji_name(base)
            && matches!(tone.as_bytes(), [b'2'..=b'6'])
    } else {
        is_valid_basic_emoji_name(normalized)
    };
    if valid && normalized.len() <= 120 {
        Ok(normalized.to_owned())
    } else {
        Err(Error::invalid_input(
            "name",
            "must be a Slack emoji name, optionally followed by ::skin-tone-2 through ::skin-tone-6",
        ))
    }
}

fn is_valid_basic_emoji_name(name: &str) -> bool {
    (1..=100).contains(&name.len())
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'+')
        })
}

fn is_bounded_optional_text(value: Option<&str>, maximum: usize) -> bool {
    value.is_none_or(|value| is_bounded_untrusted_text(value, maximum))
}

fn is_bounded_untrusted_text(value: &str, maximum: usize) -> bool {
    value.len() <= maximum && !value.chars().any(|character| character == '\0')
}

fn is_safe_metadata_url(value: &str) -> bool {
    value.len() <= 8_192
        && !value.chars().any(char::is_control)
        && url::Url::parse(value)
            .ok()
            .is_some_and(|url| url.scheme() == "https" && url.host_str().is_some())
}

fn is_safe_upload_url(value: &str) -> bool {
    if value.len() > 8_192 || value.chars().any(char::is_control) {
        return false;
    }
    url::Url::parse(value).ok().is_some_and(|url| {
        url.scheme() == "https"
            && url.host_str() == Some("files.slack.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none()
            && url
                .path()
                .strip_prefix("/upload/v1/")
                .is_some_and(|suffix| !suffix.is_empty())
    })
}

fn validate_upload_input(field: &'static str, value: &str, maximum: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(Error::invalid_input(
            field,
            "must contain bounded non-control text",
        ));
    }
    Ok(())
}

fn completion_has_exact_file(completion: &RawFileUploadCompletion, file_id: &str) -> bool {
    completion.files.len() == 1 && completion.files[0].id == file_id
}

fn normalize_user_directory_page(
    page: RawUsersPage,
    seen_cursors: &HashSet<String>,
) -> Result<(Vec<User>, Option<String>)> {
    if page.members.len() > USERS_PAGE_SIZE
        || page
            .members
            .iter()
            .any(|raw_user| !is_valid_user_id(&raw_user.id))
    {
        return Err(Error::InvalidResponse {
            method: "users.list",
        });
    }
    let next = response_cursor("users.list", page.response_metadata.next_cursor)?;
    if next
        .as_ref()
        .is_some_and(|next| seen_cursors.contains(next))
    {
        return Err(Error::InvalidResponse {
            method: "users.list",
        });
    }
    Ok((page.members.into_iter().map(normalize_user).collect(), next))
}

fn normalize_user(raw: RawUser) -> User {
    let real_name = normalize_optional_identity_label(raw.profile.real_name)
        .or_else(|| normalize_optional_identity_label(raw.real_name));
    User {
        id: raw.id,
        name: normalize_optional_identity_label(raw.name),
        display_name: normalize_optional_identity_label(raw.profile.display_name),
        real_name,
        title: raw.profile.title,
        deleted: raw.deleted,
        is_bot: raw.is_bot,
        timezone: raw.tz,
        image_url: raw.profile.image_72,
    }
}

fn resolve_outbound_users(
    references: &[String],
    directory: &UserDirectory,
) -> Result<Vec<ResolvedOutboundUser>> {
    references
        .iter()
        .map(|reference| resolve_outbound_user(reference, directory))
        .collect()
}

fn resolve_outbound_user(
    reference: &str,
    directory: &UserDirectory,
) -> Result<ResolvedOutboundUser> {
    if directory.conflicting_ids.contains(reference) {
        return Err(Error::OutboundMention {
            reference: reference.to_owned(),
            reason: "Slack returned conflicting records for this ID; inspect the user directory and retry",
        });
    }
    if let Some(user) = directory.users.get(reference) {
        if user.deleted {
            return Err(Error::OutboundMention {
                reference: reference.to_owned(),
                reason: "the user is deleted; choose an active Slack user",
            });
        }
        return Ok(ResolvedOutboundUser {
            reference: reference.to_owned(),
            user_id: user.id.clone(),
            resolution: OutboundMentionResolution::UserId,
        });
    }

    if !directory.complete {
        return Err(Error::OutboundMention {
            reference: reference.to_owned(),
            reason: if is_slack_shaped_user_reference(reference) {
                "the bounded user directory ended before this ID could be verified; retry with a verified active ID"
            } else {
                "name resolution requires a complete bounded user directory; use an exact verified user ID"
            },
        });
    }
    if !directory.conflicting_ids.is_empty() {
        return Err(Error::OutboundMention {
            reference: reference.to_owned(),
            reason: "Slack returned conflicting user records; use an exact non-conflicting user ID",
        });
    }

    if let Some(resolved) = resolve_outbound_user_by_label(
        reference,
        directory,
        |user| user.name.as_deref(),
        OutboundMentionResolution::Username,
    )? {
        return Ok(resolved);
    }
    if let Some(resolved) = resolve_outbound_user_by_label(
        reference,
        directory,
        |user| user.display_name.as_deref(),
        OutboundMentionResolution::DisplayName,
    )? {
        return Ok(resolved);
    }

    Err(Error::OutboundMention {
        reference: reference.to_owned(),
        reason: if is_slack_shaped_user_reference(reference) {
            "no active user has this exact ID, username, or display name; use `lurkline users find`"
        } else {
            "no active user has this exact username or display name; use `lurkline users find` or an exact user ID"
        },
    })
}

fn interrupted_outbound_error_is_definitive(error: &Error, directory: &UserDirectory) -> bool {
    let Error::OutboundMention { reference, .. } = error else {
        return false;
    };
    directory.users.contains_key(reference) || directory.conflicting_ids.contains(reference)
}

fn resolve_outbound_user_by_label(
    reference: &str,
    directory: &UserDirectory,
    label: impl Fn(&User) -> Option<&str>,
    resolution: OutboundMentionResolution,
) -> Result<Option<ResolvedOutboundUser>> {
    let folded_reference = reference.to_lowercase();
    let mut active = directory.users.values().filter(|user| {
        !user.deleted
            && label(user).is_some_and(|candidate| candidate.to_lowercase() == folded_reference)
    });
    let first = active.next();
    if active.next().is_some() {
        return Err(Error::OutboundMention {
            reference: reference.to_owned(),
            reason: "multiple active users match; use an exact Slack user ID",
        });
    }
    if let Some(user) = first {
        return Ok(Some(ResolvedOutboundUser {
            reference: reference.to_owned(),
            user_id: user.id.clone(),
            resolution,
        }));
    }
    if directory.users.values().any(|user| {
        user.deleted
            && label(user).is_some_and(|candidate| candidate.to_lowercase() == folded_reference)
    }) {
        return Err(Error::OutboundMention {
            reference: reference.to_owned(),
            reason: "the matching user is deleted; choose an active Slack user",
        });
    }
    Ok(None)
}

fn is_slack_shaped_user_reference(reference: &str) -> bool {
    matches!(reference.as_bytes().first(), Some(b'U' | b'W'))
        && (2..=64).contains(&reference.len())
        && reference
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn user_matches(user: &RawUser, needle: &str) -> bool {
    [
        Some(user.id.as_str()),
        user.name.as_deref(),
        user.real_name.as_deref(),
        user.profile.display_name.as_deref(),
        user.profile.real_name.as_deref(),
        Some(user.profile.title.as_str()),
    ]
    .iter()
    .flatten()
    .any(|candidate| candidate.to_lowercase().contains(needle))
}

fn normalize_optional_identity_label(value: Option<String>) -> Option<String> {
    let value = value?;
    let value = value.trim();
    is_valid_author_label(value).then(|| value.to_owned())
}

fn validate_conversation_reference(reference: &str) -> Result<String> {
    let reference = reference.trim();
    let reference = reference
        .strip_prefix('#')
        .or_else(|| reference.strip_prefix('@'))
        .unwrap_or(reference)
        .trim();
    if !is_valid_conversation_name(reference) {
        return Err(Error::invalid_input(
            "conversation",
            "must be a Slack conversation ID or a 1 to 128 character name",
        ));
    }
    Ok(reference.to_owned())
}

fn is_valid_conversation_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
}

fn validate_cursor(cursor: Option<&str>) -> Result<()> {
    if cursor.is_some_and(|cursor| {
        cursor.trim().is_empty()
            || cursor.len() > 2048
            || cursor.chars().any(|character| character.is_control())
    }) {
        return Err(Error::invalid_input(
            "cursor",
            "must contain 1 to 2048 non-control characters",
        ));
    }
    Ok(())
}

fn validate_timestamp(field: &'static str, timestamp: &str) -> Result<()> {
    if !is_valid_timestamp(timestamp) {
        return Err(Error::invalid_input(
            field,
            "must be a Slack message timestamp",
        ));
    }
    Ok(())
}

fn validate_draft_destination(thread_ts: Option<&str>, broadcast: bool) -> Result<()> {
    if let Some(thread_ts) = thread_ts {
        validate_timestamp("thread_ts", thread_ts)?;
    }
    if broadcast && thread_ts.is_none() {
        return Err(Error::invalid_input(
            "broadcast",
            "is valid only for a thread reply",
        ));
    }
    Ok(())
}

fn require_confirmation(action: &'static str, confirmed: bool) -> Result<()> {
    if confirmed {
        Ok(())
    } else {
        Err(Error::ConfirmationRequired { action })
    }
}

fn system_unix_milliseconds() -> Result<String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().to_string())
        .map_err(|_| Error::SystemClock)
}

fn parse_activity_duration(value: &str) -> Result<TimeDelta> {
    if value.is_empty() || value.len() > 64 || value.chars().any(char::is_whitespace) {
        return Err(Error::invalid_input(
            "since",
            "must be a positive duration such as 6h or 1d12h",
        ));
    }
    let mut total_seconds = 0_i64;
    let mut number = 0_i64;
    let mut has_digits = false;
    for character in value.chars() {
        if let Some(digit) = character.to_digit(10) {
            has_digits = true;
            number = number
                .checked_mul(10)
                .and_then(|number| number.checked_add(i64::from(digit)))
                .ok_or_else(|| Error::invalid_input("since", "is too large"))?;
            continue;
        }
        if !has_digits || number == 0 {
            return Err(Error::invalid_input(
                "since",
                "must be a positive duration such as 6h or 1d12h",
            ));
        }
        let multiplier = match character {
            's' => 1,
            'm' => 60,
            'h' => 60 * 60,
            'd' => 24 * 60 * 60,
            'w' => 7 * 24 * 60 * 60,
            _ => {
                return Err(Error::invalid_input(
                    "since",
                    "must use s, m, h, d, or w units",
                ));
            }
        };
        total_seconds = total_seconds
            .checked_add(
                number
                    .checked_mul(multiplier)
                    .ok_or_else(|| Error::invalid_input("since", "is too large"))?,
            )
            .ok_or_else(|| Error::invalid_input("since", "is too large"))?;
        if total_seconds > MAX_ACTIVITY_DURATION_SECONDS {
            return Err(Error::invalid_input("since", "must not exceed 365 days"));
        }
        number = 0;
        has_digits = false;
    }
    if has_digits || total_seconds == 0 {
        return Err(Error::invalid_input(
            "since",
            "must be a positive duration such as 6h or 1d12h",
        ));
    }
    TimeDelta::try_seconds(total_seconds)
        .ok_or_else(|| Error::invalid_input("since", "is too large"))
}

fn parse_activity_rfc3339(field: &'static str, value: &str) -> Result<i64> {
    let parsed = DateTime::parse_from_rfc3339(value).map_err(|_| {
        Error::invalid_input(
            field,
            "must be RFC 3339 with Z or an explicit numeric offset",
        )
    })?;
    let nanos = parsed.timestamp_nanos_opt().ok_or_else(|| {
        Error::invalid_input(field, "must be within the supported Unix timestamp range")
    })?;
    if nanos < 0 {
        return Err(Error::invalid_input(
            field,
            "must not be earlier than the Unix epoch",
        ));
    }
    Ok(nanos)
}

fn format_activity_instant(nanos: i64) -> Result<String> {
    let seconds = nanos.div_euclid(1_000_000_000);
    let subsecond = nanos.rem_euclid(1_000_000_000) as u32;
    DateTime::<Utc>::from_timestamp(seconds, subsecond)
        .map(|instant| instant.to_rfc3339_opts(SecondsFormat::AutoSi, true))
        .ok_or(Error::InvalidResponse { method: "activity" })
}

fn activity_slack_bounds(after_nanos: i64, before_nanos: i64) -> Result<Option<(String, String)>> {
    if after_nanos < 0 || after_nanos >= before_nanos {
        return Err(Error::InvalidResponse { method: "activity" });
    }
    let oldest_micros = ceil_activity_microseconds(after_nanos)?;
    let latest_micros = ceil_activity_microseconds(before_nanos)?
        .checked_sub(1)
        .ok_or(Error::InvalidResponse { method: "activity" })?;
    if oldest_micros > latest_micros {
        return Ok(None);
    }
    Ok(Some((
        format_slack_microseconds(oldest_micros),
        format_slack_microseconds(latest_micros),
    )))
}

fn ceil_activity_microseconds(nanos: i64) -> Result<i64> {
    (nanos / 1_000)
        .checked_add(i64::from(nanos % 1_000 != 0))
        .ok_or(Error::InvalidResponse { method: "activity" })
}

fn format_slack_microseconds(total_micros: i64) -> String {
    format!(
        "{}.{:06}",
        total_micros / 1_000_000,
        total_micros % 1_000_000
    )
}

fn timestamp_in_activity_interval(timestamp: &str, after: i64, before: i64) -> bool {
    compare_slack_timestamp_to_nanos(timestamp, after) != Some(Ordering::Less)
        && compare_slack_timestamp_to_nanos(timestamp, before) == Some(Ordering::Less)
}

fn compare_slack_timestamp_to_nanos(timestamp: &str, nanos: i64) -> Option<Ordering> {
    let (seconds, fraction) = timestamp.split_once('.')?;
    let seconds = seconds.parse::<i64>().ok()?;
    let bound_seconds = nanos.div_euclid(1_000_000_000);
    match seconds.cmp(&bound_seconds) {
        Ordering::Equal => {
            let bound_fraction = format!("{:09}", nanos.rem_euclid(1_000_000_000));
            Some(compare_decimal_fractions(fraction, &bound_fraction))
        }
        ordering => Some(ordering),
    }
}

fn compare_decimal_fractions(left: &str, right: &str) -> Ordering {
    let length = left.len().max(right.len());
    let left = left.as_bytes();
    let right = right.as_bytes();
    for index in 0..length {
        let left = left.get(index).copied().unwrap_or(b'0');
        let right = right.get(index).copied().unwrap_or(b'0');
        match left.cmp(&right) {
            Ordering::Equal => {}
            ordering => return ordering,
        }
    }
    Ordering::Equal
}

fn compare_slack_timestamps(left: &str, right: &str) -> Ordering {
    let (left_seconds, left_fraction) = left.split_once('.').unwrap_or((left, ""));
    let (right_seconds, right_fraction) = right.split_once('.').unwrap_or((right, ""));
    let left_seconds = trim_decimal_leading_zeroes(left_seconds);
    let right_seconds = trim_decimal_leading_zeroes(right_seconds);
    left_seconds
        .len()
        .cmp(&right_seconds.len())
        .then_with(|| left_seconds.cmp(right_seconds))
        .then_with(|| compare_decimal_fractions(left_fraction, right_fraction))
}

fn trim_decimal_leading_zeroes(value: &str) -> &str {
    let trimmed = value.trim_start_matches('0');
    if trimmed.is_empty() { "0" } else { trimmed }
}

fn canonical_activity_timestamp(timestamp: &str) -> String {
    let (seconds, fraction) = timestamp.split_once('.').unwrap_or((timestamp, ""));
    let fraction = fraction.trim_end_matches('0');
    format!(
        "{}.{}",
        trim_decimal_leading_zeroes(seconds),
        if fraction.is_empty() { "0" } else { fraction }
    )
}

fn compare_activity_keys(
    left_ts: &str,
    left_conversation_id: &str,
    right_ts: &str,
    right_conversation_id: &str,
) -> Ordering {
    compare_slack_timestamps(left_ts, right_ts)
        .then_with(|| left_conversation_id.cmp(right_conversation_id))
}

fn activity_item_key(item: &ActivityItem) -> ActivityKey {
    ActivityKey {
        ts: item.message.ts.clone(),
        conversation_id: item.conversation_id.clone(),
    }
}

fn activity_snapshot_digest(
    items: &[ActivityItem],
    conversation_results: &[ActivityConversationResult],
) -> Result<String> {
    let encoded = serde_json::to_vec(&(items, conversation_results)).map_err(|_| Error::Output)?;
    Ok(sha256_hex(&encoded))
}

fn sha256_hex(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn encode_activity_cursor(cursor: &ActivityCursor) -> Result<String> {
    let payload = serde_json::to_vec(cursor).map_err(|_| Error::Output)?;
    let mut checked = Vec::with_capacity(ACTIVITY_CURSOR_DOMAIN.len() + payload.len());
    checked.extend_from_slice(ACTIVITY_CURSOR_DOMAIN);
    checked.extend_from_slice(&payload);
    let encoded = format!(
        "{ACTIVITY_CURSOR_PREFIX}.{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        sha256_hex(&checked)
    );
    if encoded.len() > MAX_ACTIVITY_CURSOR_LENGTH {
        return Err(Error::Output);
    }
    Ok(encoded)
}

fn decode_activity_cursor(value: &str) -> Result<ActivityCursor> {
    if value.trim().is_empty()
        || value.len() > MAX_ACTIVITY_CURSOR_LENGTH
        || value.chars().any(char::is_control)
    {
        return Err(invalid_activity_cursor());
    }
    let mut parts = value.split('.');
    let prefix = parts.next();
    let payload = parts.next();
    let checksum = parts.next();
    if prefix != Some(ACTIVITY_CURSOR_PREFIX)
        || payload.is_none()
        || checksum.is_none()
        || parts.next().is_some()
    {
        return Err(invalid_activity_cursor());
    }
    let payload = URL_SAFE_NO_PAD
        .decode(payload.unwrap_or_default())
        .map_err(|_| invalid_activity_cursor())?;
    let mut checked = Vec::with_capacity(ACTIVITY_CURSOR_DOMAIN.len() + payload.len());
    checked.extend_from_slice(ACTIVITY_CURSOR_DOMAIN);
    checked.extend_from_slice(&payload);
    let expected_checksum = sha256_hex(&checked);
    if checksum != Some(expected_checksum.as_str()) {
        return Err(invalid_activity_cursor());
    }
    serde_json::from_slice(&payload).map_err(|_| invalid_activity_cursor())
}

fn validate_activity_cursor(cursor: &ActivityCursor, team_id: &str) -> Result<()> {
    if cursor.version != ACTIVITY_CURSOR_VERSION
        || cursor.team_id != team_id
        || cursor.after_nanos < 0
        || cursor.after_nanos >= cursor.before_nanos
        || !activity_kinds_are_canonical(&cursor.conversation_kinds)
        || cursor
            .include_ids
            .len()
            .saturating_add(cursor.exclude_ids.len())
            > MAX_ACTIVITY_SELECTORS
        || !activity_ids_are_canonical(&cursor.include_ids)
        || !activity_ids_are_canonical(&cursor.exclude_ids)
        || cursor
            .include_ids
            .iter()
            .any(|id| cursor.exclude_ids.binary_search(id).is_ok())
        || !(1..=MAX_ACTIVITY_CONVERSATIONS).contains(&cursor.conversation_limit)
        || !(1..=MAX_ACTIVITY_PER_CONVERSATION).contains(&cursor.per_conversation_limit)
        || !(1..=MAX_ACTIVITY_MESSAGES).contains(&cursor.limit)
        || cursor.eligible_conversations == 0
        || cursor.eligible_conversations > MAX_CONVERSATION_PAGES * CONVERSATIONS_PAGE_SIZE
        || !is_lower_hex_digest(&cursor.scope_digest)
    {
        return Err(invalid_activity_cursor());
    }
    let scope_offset = activity_cursor_scope_offset(&cursor.position);
    if scope_offset >= cursor.eligible_conversations
        || !scope_offset.is_multiple_of(cursor.conversation_limit)
        || scope_offset
            .checked_add(cursor.conversation_limit)
            .is_none()
    {
        return Err(invalid_activity_cursor());
    }
    match &cursor.position {
        ActivityCursorPosition::Messages {
            last_key,
            snapshot_digest,
            ..
        } if !is_valid_timestamp(&last_key.ts)
            || !is_valid_any_conversation_id(&last_key.conversation_id)
            || !is_lower_hex_digest(snapshot_digest) =>
        {
            return Err(invalid_activity_cursor());
        }
        ActivityCursorPosition::ConversationScope { scope_offset } if *scope_offset == 0 => {
            return Err(invalid_activity_cursor());
        }
        _ => {}
    }
    Ok(())
}

fn activity_cursor_scope_offset(position: &ActivityCursorPosition) -> usize {
    match position {
        ActivityCursorPosition::Messages { scope_offset, .. }
        | ActivityCursorPosition::ConversationScope { scope_offset } => *scope_offset,
    }
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn activity_kinds_are_canonical(kinds: &[ConversationKind]) -> bool {
    !kinds.is_empty() && kinds.len() <= 3 && kinds.windows(2).all(|pair| pair[0] < pair[1])
}

fn activity_ids_are_canonical(ids: &[String]) -> bool {
    ids.iter().all(|id| is_valid_any_conversation_id(id))
        && ids.windows(2).all(|pair| pair[0] < pair[1])
}

fn invalid_activity_cursor() -> Error {
    Error::invalid_input("cursor", "is not a valid activity continuation cursor")
}

fn stale_activity_cursor() -> Error {
    Error::invalid_input(
        "cursor",
        "is stale because the bounded Slack snapshot changed; start a new activity query",
    )
}

fn normalize_activity_kinds(kinds: &[ConversationKind]) -> Vec<ConversationKind> {
    let mut kinds = if kinds.is_empty() {
        vec![
            ConversationKind::Channel,
            ConversationKind::DirectMessage,
            ConversationKind::GroupDirectMessage,
        ]
    } else {
        kinds.to_vec()
    };
    kinds.sort_unstable();
    kinds.dedup();
    kinds
}

fn select_activity_scope(
    all: ActivityConversationDirectory,
    kinds: &[ConversationKind],
    include: &[String],
    exclude: &[String],
) -> Result<ActivityScope> {
    if include.len().saturating_add(exclude.len()) > MAX_ACTIVITY_SELECTORS {
        return Err(Error::invalid_input(
            "include",
            "include and exclude accept at most 50 selectors in total",
        ));
    }
    let include_ids = resolve_activity_selectors(&all.candidates, kinds, include, "include")?;
    let exclude_ids = resolve_activity_selectors(&all.candidates, kinds, exclude, "exclude")?;
    if include_ids
        .iter()
        .any(|id| exclude_ids.binary_search(id).is_ok())
    {
        return Err(Error::invalid_input(
            "include",
            "must not select the same conversation as exclude",
        ));
    }
    build_activity_scope(all, kinds, include_ids, exclude_ids)
}

fn rebuild_activity_scope(
    all: ActivityConversationDirectory,
    kinds: &[ConversationKind],
    include_ids: &[String],
    exclude_ids: &[String],
) -> Result<ActivityScope> {
    if include_ids.iter().chain(exclude_ids).any(|id| {
        !all.candidates.iter().any(|candidate| {
            candidate.conversation.id == *id
                && kinds.binary_search(&candidate.conversation.kind).is_ok()
        })
    }) {
        return Err(stale_activity_cursor());
    }
    build_activity_scope(all, kinds, include_ids.to_vec(), exclude_ids.to_vec())
}

fn build_activity_scope(
    all: ActivityConversationDirectory,
    kinds: &[ConversationKind],
    mut include_ids: Vec<String>,
    mut exclude_ids: Vec<String>,
) -> Result<ActivityScope> {
    include_ids.sort();
    include_ids.dedup();
    exclude_ids.sort();
    exclude_ids.dedup();
    let include = include_ids.iter().cloned().collect::<HashSet<_>>();
    let exclude = exclude_ids.iter().cloned().collect::<HashSet<_>>();
    let mut candidates = all
        .candidates
        .into_iter()
        .filter(|candidate| {
            let conversation = &candidate.conversation;
            if kinds.binary_search(&conversation.kind).is_err() {
                return false;
            }
            let included = if include.is_empty() {
                conversation.kind != ConversationKind::Channel || conversation.is_member
            } else {
                include.contains(&conversation.id)
            };
            included && !exclude.contains(&conversation.id)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.conversation.id.cmp(&right.conversation.id));
    let digest = activity_scope_digest(&candidates)?;
    Ok(ActivityScope {
        candidates,
        include_ids,
        exclude_ids,
        scanned_conversations: all.scanned_conversations,
        scan_truncated: all.scan_truncated,
        digest,
    })
}

fn activity_scope_digest(candidates: &[ActivityConversationCandidate]) -> Result<String> {
    let identity = candidates
        .iter()
        .map(|candidate| {
            (
                candidate.conversation.id.as_str(),
                candidate.conversation.kind,
            )
        })
        .collect::<Vec<_>>();
    let encoded = serde_json::to_vec(&identity).map_err(|_| Error::Output)?;
    Ok(sha256_hex(&encoded))
}

fn resolve_activity_selectors(
    candidates: &[ActivityConversationCandidate],
    kinds: &[ConversationKind],
    selectors: &[String],
    field: &'static str,
) -> Result<Vec<String>> {
    let mut ids = HashSet::new();
    for selector in selectors {
        let needle = validate_activity_selector(field, selector)?;
        let lowered = needle.to_lowercase();
        let matches = if is_valid_any_conversation_id(&needle) {
            candidates
                .iter()
                .filter(|candidate| candidate.conversation.id == needle)
                .collect::<Vec<_>>()
        } else {
            candidates
                .iter()
                .filter(|candidate| {
                    let conversation = &candidate.conversation;
                    conversation.name.to_lowercase() == lowered
                        || conversation.display_name.to_lowercase() == lowered
                })
                .collect::<Vec<_>>()
        };
        let allowed_matches = matches
            .iter()
            .copied()
            .filter(|candidate| kinds.binary_search(&candidate.conversation.kind).is_ok())
            .collect::<Vec<_>>();
        match allowed_matches.as_slice() {
            [] => {
                if !matches.is_empty() {
                    return Err(Error::invalid_input(
                        field,
                        "matches a conversation excluded by the kind filter",
                    ));
                }
                return Err(Error::invalid_input(
                    field,
                    "does not match an accessible Slack conversation",
                ));
            }
            [candidate] => {
                ids.insert(candidate.conversation.id.clone());
            }
            _ => {
                return Err(Error::invalid_input(
                    field,
                    "matches more than one Slack conversation; use its ID",
                ));
            }
        }
    }
    let mut ids = ids.into_iter().collect::<Vec<_>>();
    ids.sort();
    Ok(ids)
}

fn validate_activity_selector(field: &'static str, selector: &str) -> Result<String> {
    let selector = selector.trim();
    let selector = selector
        .strip_prefix('#')
        .or_else(|| selector.strip_prefix('@'))
        .unwrap_or(selector)
        .trim();
    if !is_valid_conversation_name(selector) {
        return Err(Error::invalid_input(
            field,
            "must be a Slack conversation ID or a 1 to 128 character exact name",
        ));
    }
    Ok(selector.to_owned())
}

fn activity_error_status(error: &Error) -> ActivityConversationStatus {
    match error {
        Error::Authorization { .. } | Error::NotFound { .. } => {
            ActivityConversationStatus::Inaccessible
        }
        Error::SlackApi { code, .. }
            if matches!(
                code.as_str(),
                "access_denied"
                    | "channel_is_limited_access"
                    | "channel_not_found"
                    | "no_permission"
                    | "not_in_channel"
            ) =>
        {
            ActivityConversationStatus::Inaccessible
        }
        _ => ActivityConversationStatus::Unavailable,
    }
}

fn validate_draft_id(draft_id: &str) -> Result<()> {
    if is_valid_draft_id(draft_id) {
        Ok(())
    } else {
        Err(Error::invalid_input(
            "draft_id",
            "must contain 1 to 128 safe identifier characters",
        ))
    }
}

fn is_valid_draft_id(draft_id: &str) -> bool {
    (1..=128).contains(&draft_id.len())
        && draft_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_valid_draft_client(client: &str) -> bool {
    client.len() <= 64
        && client
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':'))
}

fn is_valid_team_id(team_id: &str) -> bool {
    team_id.starts_with('T')
        && (2..=64).contains(&team_id.len())
        && team_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn validate_draft_revision_input(field: &'static str, revision: Option<&str>) -> Result<()> {
    if revision.is_some_and(|revision| !is_valid_draft_revision(revision)) {
        Err(Error::invalid_input(
            field,
            "must be a Slack draft revision timestamp",
        ))
    } else {
        Ok(())
    }
}

fn is_valid_draft_revision(revision: &str) -> bool {
    let mut decimal_points = 0;
    !revision.is_empty()
        && revision.len() <= 32
        && revision.bytes().all(|byte| {
            if byte == b'.' {
                decimal_points += 1;
                decimal_points == 1
            } else {
                byte.is_ascii_digit()
            }
        })
        && revision.bytes().any(|byte| byte.is_ascii_digit())
        && !revision.starts_with('.')
        && !revision.ends_with('.')
}

/// Slack's web client turns the server's seconds revision into milliseconds
/// before sending it back as `client_last_updated_ts`.
fn server_revision_to_client_timestamp(revision: &str) -> Option<String> {
    if !is_valid_draft_revision(revision) {
        return None;
    }
    let (whole, fraction) = revision.split_once('.').unwrap_or((revision, ""));
    let milliseconds_len = fraction.len().min(3);
    let mut integer = String::with_capacity(whole.len() + 3);
    integer.push_str(whole);
    integer.push_str(&fraction[..milliseconds_len]);
    integer.extend(std::iter::repeat_n('0', 3 - milliseconds_len));
    let integer = integer.trim_start_matches('0');
    let integer = if integer.is_empty() { "0" } else { integer };
    let remainder = fraction[milliseconds_len..].trim_end_matches('0');
    if remainder.is_empty() {
        Some(integer.to_owned())
    } else {
        Some(format!("{integer}.{remainder}"))
    }
}

fn is_valid_timestamp(timestamp: &str) -> bool {
    let mut parts = timestamp.split('.');
    let seconds = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    !(timestamp.len() > 32
        || seconds.is_empty()
        || fraction.is_empty()
        || parts.next().is_some()
        || !seconds.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit()))
}

fn is_valid_user_id(user_id: &str) -> bool {
    (2..=64).contains(&user_id.len()) && user_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_valid_conversation_id(id: &str, kind: ConversationKind) -> bool {
    let valid_prefix = match kind {
        ConversationKind::Channel => matches!(id.as_bytes().first(), Some(b'C' | b'G')),
        ConversationKind::DirectMessage => id.starts_with('D'),
        ConversationKind::GroupDirectMessage => {
            matches!(id.as_bytes().first(), Some(b'C' | b'G'))
        }
    };
    valid_prefix
        && (2..=64).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_valid_any_conversation_id(id: &str) -> bool {
    matches!(id.as_bytes().first(), Some(b'C' | b'D' | b'G'))
        && (2..=64).contains(&id.len())
        && id.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn is_slack_shaped_conversation_id(id: &str) -> bool {
    is_valid_any_conversation_id(id)
        && id
            .bytes()
            .skip(1)
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn validate_query(query: &str) -> Result<String> {
    let query = query.trim();
    if query.is_empty()
        || query.len() > 128
        || query.chars().any(|character| character.is_control())
    {
        return Err(Error::invalid_input(
            "query",
            "must contain 1 to 128 non-control characters",
        ));
    }
    Ok(query.to_owned())
}

fn validate_search_query(query: &str) -> Result<String> {
    let query = query.trim();
    if query.is_empty()
        || query.len() > 512
        || query.chars().any(|character| character.is_control())
    {
        return Err(Error::invalid_input(
            "query",
            "must contain 1 to 512 non-control characters",
        ));
    }
    Ok(query.to_owned())
}

fn validate_date(field: &'static str, value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let valid_shape = bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit());
    if valid_shape {
        let year = value[0..4].parse::<u32>().unwrap_or_default();
        let month = value[5..7].parse::<u32>().unwrap_or_default();
        let day = value[8..10].parse::<u32>().unwrap_or_default();
        let leap_year =
            year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
        let days = match month {
            1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
            4 | 6 | 9 | 11 => 30,
            2 if leap_year => 29,
            2 => 28,
            _ => 0,
        };
        if year > 0 && day > 0 && day <= days {
            return Ok(value.to_owned());
        }
    }
    Err(Error::invalid_input(
        field,
        "must be a valid calendar date in YYYY-MM-DD format",
    ))
}

fn validate_limit(field: &'static str, limit: usize, maximum: usize) -> Result<()> {
    if !(1..=maximum).contains(&limit) {
        return Err(Error::invalid_input(field, "is outside the allowed range"));
    }
    Ok(())
}

fn response_cursor(method: &'static str, value: String) -> Result<Option<String>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(Error::InvalidResponse { method });
    }
    Ok(Some(value))
}

fn reject_repeated_cursor(
    method: &'static str,
    current: Option<&str>,
    next: Option<&str>,
) -> Result<()> {
    if current
        .zip(next)
        .is_some_and(|(current, next)| current == next)
    {
        return Err(Error::InvalidResponse { method });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, VecDeque},
        sync::Mutex,
    };

    use serde_json::json;
    use url::Url;

    use futures_util::StreamExt;

    use super::*;
    use crate::model::{
        RawChannelMessages, RawConversation, RawConversationsPage, RawFile,
        RawMessageSearchChannel, RawMessageSearchMatch, RawMessageSearchMatches,
        RawMessageSearchPagination, RawMessageSearchResponse, RawReaction, RawResponseMetadata,
        RawThreadCounts, RawUnread, RawUserProfile,
    };

    struct FakeApi {
        counts: ClientCountsPayload,
        count_calls: Arc<Mutex<usize>>,
        history: RawMessagePage,
        history_pages: Mutex<VecDeque<RawMessagePage>>,
        activity_results: Mutex<VecDeque<Result<RawMessagePage>>>,
        activity_calls: Arc<Mutex<Vec<ActivityCall>>>,
        replies: RawMessagePage,
        reply_pages: Mutex<VecDeque<RawMessagePage>>,
        message_list: RawMessagesList,
        message_list_calls: Arc<Mutex<Vec<(String, String)>>>,
        search: RawMessageSearchResponse,
        search_calls: Arc<Mutex<Vec<SearchCall>>>,
        history_calls: Arc<Mutex<Vec<HistoryCall>>>,
        reply_calls: Arc<Mutex<Vec<ReplyCall>>>,
        conversation_calls: Arc<Mutex<Vec<ConversationCall>>>,
        conversation_pages: Mutex<VecDeque<RawConversationsPage>>,
        user_pages: Mutex<VecDeque<RawUsersPage>>,
        user_calls: Arc<Mutex<Vec<UserCall>>>,
        user_list_error: bool,
        user_list_error_after: Option<usize>,
        drafts_page: RawDraftsPage,
        draft_pages: Mutex<VecDeque<RawDraftsPage>>,
        draft_info: RawDraftResponse,
        draft_infos: Mutex<VecDeque<RawDraftResponse>>,
        draft_create: RawDraftResponse,
        draft_create_error: Option<&'static str>,
        draft_update: RawDraftResponse,
        draft_update_error: Option<&'static str>,
        draft_delete_error: bool,
        draft_delete_ambiguous: bool,
        draft_calls: Arc<Mutex<Vec<DraftCall>>>,
        post_response: Option<RawPostMessageResponse>,
        post_error: Option<String>,
        post_calls: Arc<Mutex<Vec<PostCall>>>,
        file_share_error: Option<String>,
        file_share_transport_error: bool,
        file_share_calls: Arc<Mutex<Vec<FileShareCall>>>,
        emoji_response: RawEmojiResponse,
        file_response: RawFileResponse,
        file_responses: Mutex<VecDeque<RawFileResponse>>,
        file_info_results: Mutex<VecDeque<Result<RawFileResponse>>>,
        file_info_calls: Arc<Mutex<Vec<String>>>,
        reaction_present: Arc<Mutex<bool>>,
        reaction_name: Arc<Mutex<String>>,
        reaction_error: Option<&'static str>,
        reaction_apply_before_error: bool,
        reaction_get_error_after: Option<usize>,
        reaction_wrong_channel_after: Option<usize>,
        reaction_wrong_type_after: Option<usize>,
        reaction_duplicate_after: Option<usize>,
        reaction_get_count: Arc<Mutex<usize>>,
        reaction_calls: Arc<Mutex<Vec<String>>>,
        download_bytes: Vec<u8>,
        upload_allocation: RawFileUploadAllocation,
        upload_allocation_error: Option<&'static str>,
        upload_transfer_error: bool,
        upload_transfer_invalid_ack: bool,
        upload_mutate_pass: bool,
        upload_completion: RawFileUploadCompletion,
        upload_completion_error: bool,
        upload_calls: Arc<Mutex<Vec<&'static str>>>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct SearchCall {
        query: String,
        cursor: Option<String>,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct HistoryCall {
        channel: String,
        cursor: Option<String>,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ActivityCall {
        channel: String,
        oldest: String,
        latest: String,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ReplyCall {
        channel: String,
        thread_ts: String,
        cursor: Option<String>,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct ConversationCall {
        cursor: Option<String>,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct UserCall {
        cursor: Option<String>,
        limit: usize,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum DraftCall {
        List {
            next_ts: Option<String>,
            limit: usize,
        },
        Info {
            draft_id: String,
        },
        Create {
            client_msg_id: String,
            destinations: Vec<DraftDestination>,
            blocks: Vec<serde_json::Value>,
            file_ids: Vec<String>,
        },
        Update {
            draft_id: String,
            last_updated_ts: String,
            destinations: Vec<DraftDestination>,
            blocks: Vec<serde_json::Value>,
            file_ids: Vec<String>,
        },
        Delete {
            draft_id: String,
            last_updated_ts: String,
            skip_file_deletion: bool,
        },
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct PostCall {
        channel: String,
        thread_ts: Option<String>,
        broadcast: bool,
        client_msg_id: String,
        text: String,
        blocks: Vec<serde_json::Value>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct FileShareCall {
        channel: String,
        thread_ts: Option<String>,
        broadcast: bool,
        client_msg_id: String,
        draft_id: String,
        blocks: Vec<serde_json::Value>,
        file_id: String,
    }

    const REQUEST_CLIENT_MSG_ID: &str = "__from_request__";

    fn hydrate_test_client_msg_id(draft: &mut RawDraft, client_msg_id: &str) {
        if draft.client_msg_id.as_deref() == Some(REQUEST_CLIENT_MSG_ID) {
            draft.client_msg_id = Some(client_msg_id.to_owned());
        }
    }

    fn last_created_client_msg_id(calls: &Mutex<Vec<DraftCall>>) -> Option<String> {
        calls.lock().unwrap().iter().rev().find_map(|call| {
            if let DraftCall::Create { client_msg_id, .. } = call {
                Some(client_msg_id.clone())
            } else {
                None
            }
        })
    }

    fn assert_creation_uncertain_matches_request(error: Error, calls: &Mutex<Vec<DraftCall>>) {
        let Error::DraftCreationUncertain { client_msg_id } = error else {
            panic!("expected a structured uncertain creation");
        };
        assert_eq!(
            last_created_client_msg_id(calls).as_deref(),
            Some(client_msg_id.as_str())
        );
    }

    #[async_trait]
    impl SlackApi for FakeApi {
        async fn client_counts(&self) -> Result<ClientCountsPayload> {
            *self.count_calls.lock().unwrap() += 1;
            Ok(self.counts.clone())
        }

        async fn conversation_history(
            &self,
            channel: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<RawMessagePage> {
            self.history_calls.lock().unwrap().push(HistoryCall {
                channel: channel.into(),
                cursor: cursor.map(str::to_owned),
                limit,
            });
            Ok(self
                .history_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.history.clone()))
        }

        async fn activity_history(
            &self,
            channel: &str,
            oldest: &str,
            latest: &str,
            limit: usize,
        ) -> Result<RawMessagePage> {
            self.activity_calls.lock().unwrap().push(ActivityCall {
                channel: channel.into(),
                oldest: oldest.into(),
                latest: latest.into(),
                limit,
            });
            self.activity_results
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Ok(self.history.clone()))
        }

        async fn conversation_replies(
            &self,
            channel: &str,
            thread_ts: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<RawMessagePage> {
            self.reply_calls.lock().unwrap().push(ReplyCall {
                channel: channel.into(),
                thread_ts: thread_ts.into(),
                cursor: cursor.map(str::to_owned),
                limit,
            });
            Ok(self
                .reply_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.replies.clone()))
        }

        async fn messages_list(&self, channel: &str, message_ts: &str) -> Result<RawMessagesList> {
            self.message_list_calls
                .lock()
                .unwrap()
                .push((channel.into(), message_ts.into()));
            Ok(self.message_list.clone())
        }

        async fn conversations_list(
            &self,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<RawConversationsPage> {
            self.conversation_calls
                .lock()
                .unwrap()
                .push(ConversationCall {
                    cursor: cursor.map(str::to_owned),
                    limit,
                });
            Ok(self
                .conversation_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }

        async fn search_messages(
            &self,
            query: &str,
            cursor: Option<&str>,
            limit: usize,
        ) -> Result<RawMessageSearchResponse> {
            self.search_calls.lock().unwrap().push(SearchCall {
                query: query.into(),
                cursor: cursor.map(str::to_owned),
                limit,
            });
            Ok(self.search.clone())
        }

        async fn users_list(&self, cursor: Option<&str>, limit: usize) -> Result<RawUsersPage> {
            let mut calls = self.user_calls.lock().unwrap();
            let call_index = calls.len();
            calls.push(UserCall {
                cursor: cursor.map(str::to_owned),
                limit,
            });
            drop(calls);
            if self.user_list_error
                || self
                    .user_list_error_after
                    .is_some_and(|after| call_index >= after)
            {
                return Err(Error::Authentication);
            }
            Ok(self
                .user_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_default())
        }

        async fn auth_test(&self) -> Result<RawAuthTestResponse> {
            Ok(RawAuthTestResponse {
                user_id: "U123".into(),
            })
        }

        async fn emoji_list(&self) -> Result<RawEmojiResponse> {
            Ok(self.emoji_response.clone())
        }

        async fn files_info(&self, file_id: &str) -> Result<RawFileResponse> {
            self.file_info_calls
                .lock()
                .unwrap()
                .push(file_id.to_owned());
            if let Some(result) = self.file_info_results.lock().unwrap().pop_front() {
                return result;
            }
            Ok(self
                .file_responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.file_response.clone()))
        }

        async fn reactions_get(
            &self,
            channel: &str,
            message_ts: &str,
        ) -> Result<RawReactionItemResponse> {
            self.reaction_calls.lock().unwrap().push("get".into());
            let mut get_count = self.reaction_get_count.lock().unwrap();
            *get_count += 1;
            if self
                .reaction_get_error_after
                .is_some_and(|successful_reads| *get_count > successful_reads)
            {
                return Err(Error::Transport {
                    method: "reactions.get",
                });
            }
            let present = *self.reaction_present.lock().unwrap();
            let name = self.reaction_name.lock().unwrap().clone();
            let is_after = |threshold: Option<usize>| {
                threshold.is_some_and(|successful_reads| *get_count > successful_reads)
            };
            let mut reactions = present
                .then(|| RawReaction {
                    name: name.clone(),
                    count: 1,
                    users: vec!["U123".into()],
                })
                .into_iter()
                .collect::<Vec<_>>();
            if is_after(self.reaction_duplicate_after) {
                reactions.push(RawReaction {
                    name,
                    count: 1,
                    users: vec![],
                });
            }
            Ok(RawReactionItemResponse {
                item_type: if is_after(self.reaction_wrong_type_after) {
                    "file".into()
                } else {
                    "message".into()
                },
                channel: Some(
                    if is_after(self.reaction_wrong_channel_after) {
                        "COTHER"
                    } else {
                        channel
                    }
                    .into(),
                ),
                message: Some(RawMessage {
                    ts: message_ts.into(),
                    reactions,
                    ..RawMessage::default()
                }),
            })
        }

        async fn reactions_add(
            &self,
            _channel: &str,
            _message_ts: &str,
            name: &str,
        ) -> Result<RawMutationResponse> {
            self.reaction_calls.lock().unwrap().push("add".into());
            *self.reaction_name.lock().unwrap() = name.into();
            if self.reaction_error.is_none()
                || self.reaction_error == Some("already_reacted")
                || self.reaction_apply_before_error
            {
                *self.reaction_present.lock().unwrap() = true;
            }
            match self.reaction_error {
                Some("timeout") => Err(Error::Timeout {
                    method: "reactions.add",
                }),
                Some("invalid_response") => Err(Error::InvalidResponse {
                    method: "reactions.add",
                }),
                Some(code) => Err(Error::SlackApi {
                    method: "reactions.add",
                    code: code.into(),
                }),
                None => Ok(RawMutationResponse::default()),
            }
        }

        async fn reactions_remove(
            &self,
            _channel: &str,
            _message_ts: &str,
            name: &str,
        ) -> Result<RawMutationResponse> {
            self.reaction_calls.lock().unwrap().push("remove".into());
            *self.reaction_name.lock().unwrap() = name.into();
            if self.reaction_error.is_none()
                || self.reaction_error == Some("no_reaction")
                || self.reaction_apply_before_error
            {
                *self.reaction_present.lock().unwrap() = false;
            }
            match self.reaction_error {
                Some("timeout") => Err(Error::Timeout {
                    method: "reactions.remove",
                }),
                Some("invalid_response") => Err(Error::InvalidResponse {
                    method: "reactions.remove",
                }),
                Some(code) => Err(Error::SlackApi {
                    method: "reactions.remove",
                    code: code.into(),
                }),
                None => Ok(RawMutationResponse::default()),
            }
        }

        async fn download_private_file(
            &self,
            _download_url: &str,
            _expected_size: u64,
            _expected_mimetype: Option<&str>,
            target: &mut BoundedDownload,
        ) -> Result<()> {
            for chunk in self.download_bytes.chunks(2) {
                target.write_chunk(chunk)?;
            }
            Ok(())
        }

        async fn files_get_upload_url(
            &self,
            _filename: &str,
            _length: u64,
            _alt_text: Option<&str>,
        ) -> Result<RawFileUploadAllocation> {
            self.upload_calls.lock().unwrap().push("allocate");
            match self.upload_allocation_error {
                Some("timeout") => Err(Error::Timeout {
                    method: "files.getUploadURL",
                }),
                Some("denied") => Err(Error::SlackApi {
                    method: "files.getUploadURL",
                    code: "not_allowed".into(),
                }),
                Some(_) => Err(Error::InvalidResponse {
                    method: "files.getUploadURL",
                }),
                None => Ok(self.upload_allocation.clone()),
            }
        }

        async fn upload_edge_file(
            &self,
            _upload_url: &str,
            source: &mut UploadSource,
        ) -> Result<UploadPass> {
            self.upload_calls.lock().unwrap().push("transfer");
            if self.upload_transfer_error {
                return Err(Error::Transport {
                    method: "files.uploadEdge",
                });
            }
            let (mut stream, receipt) = source.upload_stream()?;
            while let Some(chunk) = stream.next().await {
                chunk.map_err(|_| Error::Transport {
                    method: "files.uploadEdge",
                })?;
            }
            let mut pass = receipt.await.map_err(|_| Error::InvalidResponse {
                method: "files.uploadEdge",
            })?;
            if self.upload_transfer_invalid_ack {
                return Err(Error::InvalidResponse {
                    method: "files.uploadEdge",
                });
            }
            if self.upload_mutate_pass {
                pass.digest[0] ^= 0xff;
            }
            Ok(pass)
        }

        async fn files_complete_upload(
            &self,
            _file_id: &str,
            _title: Option<&str>,
            _channel_id: &str,
            _thread_ts: Option<&str>,
            _client_msg_id: &str,
        ) -> Result<RawFileUploadCompletion> {
            self.upload_calls.lock().unwrap().push("complete");
            if self.upload_completion_error {
                Err(Error::Timeout {
                    method: "files.completeUpload",
                })
            } else {
                Ok(self.upload_completion.clone())
            }
        }

        async fn files_complete_draft_upload(
            &self,
            _file_id: &str,
            _title: Option<&str>,
        ) -> Result<RawFileUploadCompletion> {
            self.upload_calls.lock().unwrap().push("complete");
            if self.upload_completion_error {
                Err(Error::Timeout {
                    method: "files.completeUpload",
                })
            } else {
                Ok(self.upload_completion.clone())
            }
        }

        async fn drafts_list(&self, next_ts: Option<&str>, limit: usize) -> Result<RawDraftsPage> {
            self.draft_calls.lock().unwrap().push(DraftCall::List {
                next_ts: next_ts.map(str::to_owned),
                limit,
            });
            let mut page = self
                .draft_pages
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.drafts_page.clone());
            if let Some(client_msg_id) = last_created_client_msg_id(&self.draft_calls) {
                for draft in &mut page.drafts {
                    hydrate_test_client_msg_id(draft, &client_msg_id);
                }
            }
            Ok(page)
        }

        async fn drafts_info(&self, draft_id: &str) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Info {
                draft_id: draft_id.into(),
            });
            let mut response = self
                .draft_infos
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| self.draft_info.clone());
            if let Some(client_msg_id) = last_created_client_msg_id(&self.draft_calls) {
                hydrate_test_client_msg_id(&mut response.draft, &client_msg_id);
            }
            Ok(response)
        }

        async fn drafts_create(
            &self,
            client_msg_id: &str,
            destinations: &[DraftDestination],
            blocks: &[serde_json::Value],
            file_ids: &[String],
        ) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Create {
                client_msg_id: client_msg_id.into(),
                destinations: destinations.to_vec(),
                blocks: blocks.to_vec(),
                file_ids: file_ids.to_vec(),
            });
            match self.draft_create_error {
                Some("timeout") => Err(Error::Timeout {
                    method: "drafts.create",
                }),
                Some(code) => Err(Error::SlackApi {
                    method: "drafts.create",
                    code: code.into(),
                }),
                None => {
                    let mut response = self.draft_create.clone();
                    hydrate_test_client_msg_id(&mut response.draft, client_msg_id);
                    Ok(response)
                }
            }
        }

        async fn drafts_update(
            &self,
            draft_id: &str,
            last_updated_ts: &str,
            destinations: &[DraftDestination],
            blocks: &[serde_json::Value],
            file_ids: &[String],
        ) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Update {
                draft_id: draft_id.into(),
                last_updated_ts: last_updated_ts.into(),
                destinations: destinations.to_vec(),
                blocks: blocks.to_vec(),
                file_ids: file_ids.to_vec(),
            });
            match self.draft_update_error {
                Some("timeout") => Err(Error::Timeout {
                    method: "drafts.update",
                }),
                Some(code) => Err(Error::SlackApi {
                    method: "drafts.update",
                    code: code.into(),
                }),
                None => Ok(self.draft_update.clone()),
            }
        }

        async fn drafts_delete(
            &self,
            draft_id: &str,
            last_updated_ts: &str,
            skip_file_deletion: bool,
        ) -> Result<RawMutationResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Delete {
                draft_id: draft_id.into(),
                last_updated_ts: last_updated_ts.into(),
                skip_file_deletion,
            });
            if self.draft_delete_ambiguous {
                Err(Error::Timeout {
                    method: "drafts.delete",
                })
            } else if self.draft_delete_error {
                Err(Error::SlackApi {
                    method: "drafts.delete",
                    code: "draft_conflict".into(),
                })
            } else {
                Ok(RawMutationResponse::default())
            }
        }

        async fn chat_post_message(
            &self,
            request: &ChatPostMessageRequest<'_>,
        ) -> Result<RawPostMessageResponse> {
            self.post_calls.lock().unwrap().push(PostCall {
                channel: request.channel.into(),
                thread_ts: request.thread_ts.map(str::to_owned),
                broadcast: request.broadcast,
                client_msg_id: request.client_msg_id.into(),
                text: request.text.into(),
                blocks: request.blocks.to_vec(),
            });
            if let Some(code) = &self.post_error {
                return Err(Error::SlackApi {
                    method: "chat.postMessage",
                    code: code.clone(),
                });
            }
            Ok(self
                .post_response
                .clone()
                .unwrap_or_else(|| RawPostMessageResponse {
                    channel: request.channel.into(),
                    ts: "7000.000001".into(),
                    message: RawMessage {
                        ts: "7000.000001".into(),
                        thread_ts: request.thread_ts.map(str::to_owned),
                        text: request.text.into(),
                        blocks: Some(request.blocks.to_vec()),
                        ..RawMessage::default()
                    },
                }))
        }

        async fn files_share(&self, request: &FileShareRequest<'_>) -> Result<RawMutationResponse> {
            self.file_share_calls.lock().unwrap().push(FileShareCall {
                channel: request.channel.into(),
                thread_ts: request.thread_ts.map(str::to_owned),
                broadcast: request.broadcast,
                client_msg_id: request.client_msg_id.into(),
                draft_id: request.draft_id.into(),
                blocks: request.blocks.to_vec(),
                file_id: request.file_id.into(),
            });
            if self.file_share_transport_error {
                return Err(Error::Timeout {
                    method: "files.share",
                });
            }
            if let Some(code) = &self.file_share_error {
                return Err(Error::SlackApi {
                    method: "files.share",
                    code: code.clone(),
                });
            }
            Ok(RawMutationResponse::default())
        }
    }

    struct FailApi;

    #[async_trait]
    impl SlackApi for FailApi {
        async fn client_counts(&self) -> Result<ClientCountsPayload> {
            Err(Error::Authentication)
        }

        async fn conversation_history(
            &self,
            _channel: &str,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<RawMessagePage> {
            Err(Error::Authentication)
        }

        async fn conversation_replies(
            &self,
            _channel: &str,
            _thread_ts: &str,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<RawMessagePage> {
            Err(Error::Authentication)
        }

        async fn messages_list(
            &self,
            _channel: &str,
            _message_ts: &str,
        ) -> Result<RawMessagesList> {
            Err(Error::Authentication)
        }

        async fn conversations_list(
            &self,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<RawConversationsPage> {
            Err(Error::Authentication)
        }

        async fn search_messages(
            &self,
            _query: &str,
            _cursor: Option<&str>,
            _limit: usize,
        ) -> Result<RawMessageSearchResponse> {
            Err(Error::Authentication)
        }

        async fn users_list(&self, _cursor: Option<&str>, _limit: usize) -> Result<RawUsersPage> {
            Err(Error::Authentication)
        }
    }

    fn empty_counts() -> ClientCountsPayload {
        ClientCountsPayload {
            channels: vec![],
            ims: vec![],
            mpims: vec![],
            threads: RawThreadCounts::default(),
        }
    }

    fn fake_api() -> FakeApi {
        FakeApi {
            counts: empty_counts(),
            count_calls: Arc::new(Mutex::new(0)),
            history: RawMessagePage::default(),
            history_pages: Mutex::new(VecDeque::new()),
            activity_results: Mutex::new(VecDeque::new()),
            activity_calls: Arc::new(Mutex::new(Vec::new())),
            replies: RawMessagePage::default(),
            reply_pages: Mutex::new(VecDeque::new()),
            message_list: RawMessagesList::default(),
            message_list_calls: Arc::new(Mutex::new(Vec::new())),
            search: RawMessageSearchResponse {
                messages: RawMessageSearchMatches {
                    matches: vec![],
                    total: 0,
                    ..RawMessageSearchMatches::default()
                },
                ..RawMessageSearchResponse::default()
            },
            search_calls: Arc::new(Mutex::new(Vec::new())),
            history_calls: Arc::new(Mutex::new(Vec::new())),
            reply_calls: Arc::new(Mutex::new(Vec::new())),
            conversation_calls: Arc::new(Mutex::new(Vec::new())),
            conversation_pages: Mutex::new(VecDeque::new()),
            user_pages: Mutex::new(VecDeque::new()),
            user_calls: Arc::new(Mutex::new(Vec::new())),
            user_list_error: false,
            user_list_error_after: None,
            drafts_page: RawDraftsPage::default(),
            draft_pages: Mutex::new(VecDeque::new()),
            draft_info: RawDraftResponse::default(),
            draft_infos: Mutex::new(VecDeque::new()),
            draft_create: RawDraftResponse::default(),
            draft_create_error: None,
            draft_update: RawDraftResponse::default(),
            draft_update_error: None,
            draft_delete_error: false,
            draft_delete_ambiguous: false,
            draft_calls: Arc::new(Mutex::new(Vec::new())),
            post_response: None,
            post_error: None,
            post_calls: Arc::new(Mutex::new(Vec::new())),
            file_share_error: None,
            file_share_transport_error: false,
            file_share_calls: Arc::new(Mutex::new(Vec::new())),
            emoji_response: RawEmojiResponse::default(),
            file_response: RawFileResponse::default(),
            file_responses: Mutex::new(VecDeque::new()),
            file_info_results: Mutex::new(VecDeque::new()),
            file_info_calls: Arc::new(Mutex::new(Vec::new())),
            reaction_present: Arc::new(Mutex::new(false)),
            reaction_name: Arc::new(Mutex::new("eyes".into())),
            reaction_error: None,
            reaction_apply_before_error: false,
            reaction_get_error_after: None,
            reaction_wrong_channel_after: None,
            reaction_wrong_type_after: None,
            reaction_duplicate_after: None,
            reaction_get_count: Arc::new(Mutex::new(0)),
            reaction_calls: Arc::new(Mutex::new(Vec::new())),
            download_bytes: b"safe".to_vec(),
            upload_allocation: RawFileUploadAllocation {
                upload_url: Some("https://files.slack.com/upload/v1/FUPLOAD?sig=synthetic".into()),
                file_id: Some("FUPLOAD".into()),
            },
            upload_allocation_error: None,
            upload_transfer_error: false,
            upload_transfer_invalid_ack: false,
            upload_mutate_pass: false,
            upload_completion: RawFileUploadCompletion {
                files: vec![RawFile {
                    id: "FUPLOAD".into(),
                    ..RawFile::default()
                }],
            },
            upload_completion_error: false,
            upload_calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn service(api: impl SlackApi + 'static) -> SlackService {
        let config = Config::for_test(Url::parse("http://127.0.0.1:1234").unwrap(), 1024 * 1024);
        let mut service = SlackService::new(api, &config);
        service.now_millis = || Ok("9000123".into());
        service.upload_reconciliation_delays_ms = &[0];
        service.draft_reconciliation_delays_ms = &[0];
        service
    }

    struct UploadFixture(std::path::PathBuf);

    impl UploadFixture {
        fn new(bytes: &[u8]) -> Self {
            let path = std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "lurkline-service-upload-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
            std::fs::create_dir(&path).unwrap();
            std::fs::write(path.join("source.txt"), bytes).unwrap();
            Self(path)
        }

        fn source(&self) -> UploadSource {
            let root = crate::local_file::McpFileRoot::open(&self.0).unwrap();
            root.prepare_upload(std::path::Path::new("source.txt"), 1024)
                .unwrap()
        }
    }

    impl Drop for UploadFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    async fn create_file_draft_report(
        api: FakeApi,
        fixture: &UploadFixture,
    ) -> FileDraftCreateReport {
        service(api)
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "C123",
                    thread_ts: None,
                    broadcast: false,
                    markdown: "**body**",
                    title: None,
                    alt_text: Some("Synthetic test file"),
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap()
    }

    fn uploaded_file(thread_ts: Option<&str>) -> RawFile {
        RawFile {
            id: "FUPLOAD".into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            shares: Some(crate::model::RawFileShares {
                private: BTreeMap::from([(
                    "C123".into(),
                    vec![crate::model::RawFileShare {
                        ts: "200.000001".into(),
                        thread_ts: thread_ts.map(str::to_owned),
                    }],
                )]),
                ..crate::model::RawFileShares::default()
            }),
            ..RawFile::default()
        }
    }

    fn dm_uploaded_file(im_ids: Option<Vec<String>>) -> RawFile {
        RawFile {
            id: "FUPLOAD".into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            ims: im_ids,
            shares: Some(crate::model::RawFileShares::default()),
            ..RawFile::default()
        }
    }

    fn upload_message(ts: &str, thread_ts: Option<&str>) -> RawMessage {
        RawMessage {
            ts: ts.into(),
            thread_ts: thread_ts.map(str::to_owned),
            user: Some("U123".into()),
            text: "synthetic file upload".into(),
            files: vec![RawFile {
                id: "FUPLOAD".into(),
                ..RawFile::default()
            }],
            ..RawMessage::default()
        }
    }

    fn allow_upload_thread(api: &mut FakeApi) {
        api.message_list.messages.insert(
            "upload-thread-root".into(),
            RawMessage {
                ts: "100.000001".into(),
                user: Some("U123".into()),
                text: "synthetic upload thread root".into(),
                ..RawMessage::default()
            },
        );
    }

    fn entry(id: &str, has_unreads: bool, mentions: u64) -> RawUnread {
        RawUnread {
            id: id.into(),
            has_unreads,
            mention_count: mentions,
            last_read: Some("100.0".into()),
            latest: Some("200.0".into()),
        }
    }

    fn unread_entry<'a>(report: &'a UnreadReport, id: &str) -> &'a UnreadConversation {
        report
            .conversations
            .iter()
            .find(|conversation| conversation.id == id)
            .unwrap_or_else(|| panic!("missing unread conversation {id}"))
    }

    fn raw_message(ts: &str, text: &str) -> RawMessage {
        RawMessage {
            ts: ts.into(),
            user: Some("U123".into()),
            text: text.into(),
            reactions: vec![RawReaction {
                name: "eyes".into(),
                count: 2,
                users: vec![],
            }],
            files: vec![RawFile {
                id: "F123".into(),
                name: Some("note.txt".into()),
                mimetype: Some("text/plain".into()),
                size: Some(12),
                url_private_download: Some("https://files.slack.com/note.txt".into()),
                ..RawFile::default()
            }],
            ..RawMessage::default()
        }
    }

    fn raw_draft(id: &str, revision: &str, channel_id: &str, text: &str) -> RawDraft {
        RawDraft {
            id: id.into(),
            client_msg_id: Some("00000000-0000-4000-8000-000000000001".into()),
            last_updated_ts: Some(RawDraftRevision::String(revision.into())),
            text: text.into(),
            blocks: Some(vec![json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_section",
                    "elements": [{"type": "text", "text": text}]
                }]
            })]),
            destinations: vec![DraftDestination {
                channel_id: Some(channel_id.into()),
                ..DraftDestination::default()
            }],
            is_from_composer: true,
            date_created: Some(1_700_000_000),
            date_scheduled: Some(0),
            last_updated_client: Some(String::new()),
            team_id: Some("T000TEST".into()),
            user_id: Some("U123".into()),
            ..RawDraft::default()
        }
    }

    fn raw_self_dm_draft(id: &str, revision: &str, text: &str) -> RawDraft {
        let mut draft = raw_draft(id, revision, "D123", text);
        draft.destinations[0].user_ids = Some(vec!["U123".into()]);
        draft
    }

    fn raw_file_draft(
        id: &str,
        revision: &str,
        channel_id: &str,
        text: &str,
        file_id: &str,
    ) -> RawDraft {
        let mut draft = raw_draft(id, revision, channel_id, text);
        draft.file_ids = vec![file_id.into()];
        draft
    }

    fn private_draft_file(file_id: &str) -> RawFile {
        RawFile {
            id: file_id.into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            channels: Some(vec![]),
            groups: Some(vec![]),
            ims: Some(vec![]),
            shares: Some(crate::model::RawFileShares::default()),
            ..RawFile::default()
        }
    }

    fn published_dm_file(file_id: &str, message_ts: &str) -> RawFile {
        RawFile {
            id: file_id.into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            channels: Some(vec![]),
            groups: Some(vec![]),
            ims: Some(vec!["D123".into()]),
            shares: Some(crate::model::RawFileShares {
                private: BTreeMap::from([(
                    "D123".into(),
                    vec![crate::model::RawFileShare {
                        ts: message_ts.into(),
                        thread_ts: None,
                    }],
                )]),
                ..crate::model::RawFileShares::default()
            }),
            ..RawFile::default()
        }
    }

    fn published_dm_file_without_share(file_id: &str) -> RawFile {
        RawFile {
            id: file_id.into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            channels: Some(vec![]),
            groups: Some(vec![]),
            ims: Some(vec!["D123".into()]),
            shares: Some(crate::model::RawFileShares::default()),
            ..RawFile::default()
        }
    }

    fn published_channel_file(
        file_id: &str,
        message_ts: &str,
        actual_thread_ts: Option<&str>,
    ) -> RawFile {
        RawFile {
            id: file_id.into(),
            name: Some("source.txt".into()),
            alt_txt: Some("Synthetic test file".into()),
            size: Some(9),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            channels: Some(vec!["C123".into()]),
            groups: Some(vec![]),
            ims: Some(vec![]),
            shares: Some(crate::model::RawFileShares {
                private: BTreeMap::from([(
                    "C123".into(),
                    vec![crate::model::RawFileShare {
                        ts: message_ts.into(),
                        thread_ts: actual_thread_ts.map(str::to_owned),
                    }],
                )]),
                ..crate::model::RawFileShares::default()
            }),
            ..RawFile::default()
        }
    }

    fn active_drafts(drafts: Vec<RawDraft>) -> RawDraftsPage {
        RawDraftsPage {
            drafts,
            files: vec![],
            has_more: false,
        }
    }

    fn raw_user(id: &str, name: &str, display_name: &str) -> RawUser {
        RawUser {
            id: id.into(),
            name: Some(name.into()),
            real_name: Some("Fallback Name".into()),
            profile: RawUserProfile {
                display_name: Some(display_name.into()),
                real_name: Some("Profile Name".into()),
                title: "Engineer".into(),
                ..RawUserProfile::default()
            },
            ..RawUser::default()
        }
    }

    fn assert_single_user_directory_call(calls: &Mutex<Vec<UserCall>>) {
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[UserCall {
                cursor: None,
                limit: USERS_PAGE_SIZE,
            }]
        );
    }

    #[tokio::test]
    async fn outbound_mentions_resolve_id_username_and_display_name_in_one_scan() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("UALICE", "alice", "Alice Example"),
                raw_user("UOPS", "", "Operations Team"),
            ],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();
        let rendered = service(api)
            .render_markdown(concat!(
                "Hello [@Alice](slack-user:alice), ",
                "[@Operations](<slack-user:Operations Team>), and ",
                "[@Alice by ID](slack-user:UALICE).",
            ))
            .await
            .unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(
            rendered
                .outbound_mentions
                .iter()
                .map(|mention| (
                    mention.user_id.as_str(),
                    mention.resolution,
                    mention.label.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("UALICE", OutboundMentionResolution::Username, "@Alice"),
                (
                    "UOPS",
                    OutboundMentionResolution::DisplayName,
                    "@Operations"
                ),
                ("UALICE", OutboundMentionResolution::UserId, "@Alice by ID"),
            ]
        );
        let encoded = serde_json::to_string(&rendered.blocks).unwrap();
        assert_eq!(encoded.matches("\"type\":\"user\"").count(), 3);
        assert_eq!(encoded.matches("\"user_id\":\"UALICE\"").count(), 2);
        assert!(encoded.contains("\"user_id\":\"UOPS\""));
    }

    #[tokio::test]
    async fn verified_user_ids_survive_an_interrupted_directory_scan() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("UALICE", "alice", "Alice Example")],
            response_metadata: RawResponseMetadata {
                next_cursor: "next-page".into(),
            },
        }]));
        api.user_list_error_after = Some(1);
        let rendered = service(api)
            .render_markdown("Hello [@Alice](slack-user:UALICE).")
            .await
            .unwrap();
        assert_eq!(
            rendered.outbound_mentions[0].resolution,
            OutboundMentionResolution::UserId
        );
        assert_eq!(
            rendered.blocks[0]["elements"][0]["elements"][1],
            json!({"type": "user", "user_id": "UALICE"})
        );

        let mut unresolved = fake_api();
        unresolved.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("UALICE", "alice", "Alice Example")],
            response_metadata: RawResponseMetadata {
                next_cursor: "next-page".into(),
            },
        }]));
        unresolved.user_list_error_after = Some(1);
        assert!(matches!(
            service(unresolved)
                .render_markdown("Hello [@Alice](slack-user:alice).")
                .await,
            Err(Error::Authentication)
        ));

        let mut unavailable = fake_api();
        unavailable.user_list_error = true;
        assert!(matches!(
            service(unavailable)
                .render_markdown("Hello [@Alice](slack-user:alice).")
                .await,
            Err(Error::Authentication)
        ));
    }

    #[tokio::test]
    async fn mention_looking_text_and_code_render_without_a_user_scan() {
        let api = fake_api();
        let user_calls = api.user_calls.clone();
        let rendered = service(api)
            .render_markdown(concat!(
                "Literal @alice, alice@example.com, and <@UALICE>.\n\n",
                "`[@Alice](slack-user:alice)`\n\n",
                "```text\n[@Alice](slack-user:alice)\n```",
            ))
            .await
            .unwrap();
        assert!(user_calls.lock().unwrap().is_empty());
        assert!(rendered.outbound_mentions.is_empty());
        assert!(
            !serde_json::to_string(&rendered.blocks)
                .unwrap()
                .contains("\"type\":\"user\"")
        );
    }

    #[test]
    fn outbound_mention_resolution_is_exact_deterministic_and_fail_closed() {
        let alice = normalize_user(raw_user("UALICE", "alice", "Shared"));
        let display_only = normalize_user(raw_user("UDISPLAY", "", "Display Only"));
        let shadow = normalize_user(raw_user("USHADOW", "shadow", "alice"));
        let complete = UserDirectory {
            users: [alice.clone(), display_only.clone(), shadow]
                .into_iter()
                .map(|user| (user.id.clone(), user))
                .collect(),
            conflicting_ids: HashSet::new(),
            complete: true,
        };
        assert_eq!(
            resolve_outbound_user("alice", &complete)
                .unwrap()
                .resolution,
            OutboundMentionResolution::Username
        );
        assert_eq!(
            resolve_outbound_user("display only", &complete)
                .unwrap()
                .resolution,
            OutboundMentionResolution::DisplayName
        );

        let id_shaped_labels = UserDirectory {
            users: [
                raw_user("UUSERNAME", "WALLACE", "Other"),
                raw_user("UDISPLAY2", "someone_else", "UPPERDISPLAY"),
            ]
            .into_iter()
            .map(normalize_user)
            .map(|user| (user.id.clone(), user))
            .collect(),
            conflicting_ids: HashSet::new(),
            complete: true,
        };
        assert_eq!(
            resolve_outbound_user("WALLACE", &id_shaped_labels)
                .unwrap()
                .resolution,
            OutboundMentionResolution::Username
        );
        assert_eq!(
            resolve_outbound_user("UPPERDISPLAY", &id_shaped_labels)
                .unwrap()
                .resolution,
            OutboundMentionResolution::DisplayName
        );

        let mut duplicate = raw_user("UOTHER", "ALICE", "Other");
        duplicate.profile.display_name = Some("Other".into());
        let ambiguous = UserDirectory {
            users: [alice.clone(), normalize_user(duplicate)]
                .into_iter()
                .map(|user| (user.id.clone(), user))
                .collect(),
            conflicting_ids: HashSet::new(),
            complete: true,
        };
        assert!(matches!(
            resolve_outbound_user("alice", &ambiguous),
            Err(Error::OutboundMention {
                reason: "multiple active users match; use an exact Slack user ID",
                ..
            })
        ));

        let incomplete = UserDirectory {
            users: [(alice.id.clone(), alice.clone())].into(),
            conflicting_ids: HashSet::new(),
            complete: false,
        };
        assert_eq!(
            resolve_outbound_user("UALICE", &incomplete)
                .unwrap()
                .resolution,
            OutboundMentionResolution::UserId
        );
        assert!(matches!(
            resolve_outbound_user("alice", &incomplete),
            Err(Error::OutboundMention {
                reason: "name resolution requires a complete bounded user directory; use an exact verified user ID",
                ..
            })
        ));
        assert!(resolve_outbound_user("UUNKNOWN", &incomplete).is_err());

        let mut deleted = raw_user("UDELETED", "former", "Former User");
        deleted.deleted = true;
        let deleted = normalize_user(deleted);
        let deleted_directory = UserDirectory {
            users: [(deleted.id.clone(), deleted)].into(),
            conflicting_ids: HashSet::new(),
            complete: true,
        };
        assert!(matches!(
            resolve_outbound_user("former", &deleted_directory),
            Err(Error::OutboundMention {
                reason: "the matching user is deleted; choose an active Slack user",
                ..
            })
        ));
        assert!(resolve_outbound_user("missing", &complete).is_err());

        let conflict = UserDirectory {
            users: HashMap::new(),
            conflicting_ids: HashSet::from(["UCONFLICT".into()]),
            complete: true,
        };
        assert!(resolve_outbound_user("UCONFLICT", &conflict).is_err());
        assert!(resolve_outbound_user("conflict", &conflict).is_err());
    }

    #[test]
    fn nullable_user_identity_normalization_preserves_a_literal_null_string() {
        let overlong = "x".repeat(257);
        for raw in [
            json!({"id": "UOMITTED", "profile": {}}),
            json!({
                "id": "UNULL",
                "name": null,
                "real_name": null,
                "profile": {
                    "display_name": null,
                    "real_name": null
                }
            }),
            json!({
                "id": "UEMPTY",
                "name": "",
                "real_name": " ",
                "profile": {
                    "display_name": "\t",
                    "real_name": "\n"
                }
            }),
            json!({
                "id": "UCONTROL",
                "name": "bad\nname",
                "real_name": "bad\u{7}name",
                "profile": {
                    "display_name": "bad\tname",
                    "real_name": "bad\u{1b}name"
                }
            }),
            json!({
                "id": "UOVERLONG",
                "name": overlong,
                "real_name": overlong,
                "profile": {
                    "display_name": overlong,
                    "real_name": overlong
                }
            }),
        ] {
            let user = normalize_user(serde_json::from_value(raw).unwrap());
            assert_eq!(user.name, None);
            assert_eq!(user.display_name, None);
            assert_eq!(user.real_name, None);
            let json = serde_json::to_value(user).unwrap();
            assert_eq!(json["name"], serde_json::Value::Null);
            assert_eq!(json["display_name"], serde_json::Value::Null);
            assert_eq!(json["real_name"], serde_json::Value::Null);
        }

        let user = normalize_user(
            serde_json::from_value(json!({
                "id": "ULITERAL",
                "name": " null ",
                "real_name": " Real Name ",
                "profile": {
                    "display_name": " null ",
                    "real_name": " Profile Name "
                }
            }))
            .unwrap(),
        );
        assert_eq!(user.name.as_deref(), Some("null"));
        assert_eq!(user.display_name.as_deref(), Some("null"));
        assert_eq!(user.real_name.as_deref(), Some("Profile Name"));
        let json = serde_json::to_value(user).unwrap();
        assert_eq!(json["name"], "null");
        assert_eq!(json["display_name"], "null");

        let user = normalize_user(
            serde_json::from_value(json!({
                "id": "UTOPLEVEL",
                "real_name": " Top Level Name ",
                "profile": {
                    "real_name": " "
                }
            }))
            .unwrap(),
        );
        assert_eq!(user.real_name.as_deref(), Some("Top Level Name"));
    }

    #[test]
    fn nullable_directory_identities_flow_to_author_and_mention_json() {
        let absent_display = normalize_user(RawUser {
            id: "UABSENT".into(),
            name: Some("alice".into()),
            profile: RawUserProfile {
                display_name: None,
                real_name: None,
                ..RawUserProfile::default()
            },
            ..RawUser::default()
        });
        let literal_null = normalize_user(RawUser {
            id: "ULITERAL".into(),
            name: Some("example".into()),
            profile: RawUserProfile {
                display_name: Some("null".into()),
                real_name: None,
                ..RawUserProfile::default()
            },
            ..RawUser::default()
        });
        let unsafe_user = normalize_user(RawUser {
            id: "UUNSAFE".into(),
            name: Some("bad\nname".into()),
            real_name: Some("bad\u{7}name".into()),
            profile: RawUserProfile {
                display_name: Some("x".repeat(257)),
                real_name: Some("bad\u{1b}name".into()),
                ..RawUserProfile::default()
            },
            ..RawUser::default()
        });
        assert_eq!(unsafe_user.name, None);
        assert_eq!(unsafe_user.display_name, None);
        assert_eq!(unsafe_user.real_name, None);
        let unsafe_json = serde_json::to_value(&unsafe_user).unwrap();
        assert_eq!(unsafe_json["name"], serde_json::Value::Null);
        assert_eq!(unsafe_json["display_name"], serde_json::Value::Null);
        assert_eq!(unsafe_json["real_name"], serde_json::Value::Null);

        let unread = resolved_user_conversation_name(&unsafe_user);
        assert_eq!(unread.name, None);
        assert_eq!(unread.display_name, None);
        assert_eq!(unread.resolution, ConversationNameResolution::Unnamed);

        let directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([
                ("UABSENT".into(), absent_display),
                ("ULITERAL".into(), literal_null),
                ("UUNSAFE".into(), unsafe_user),
            ]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });

        let mut author_name = None;
        let mut author_display_name = None;
        let mut author_resolution = AuthorResolution::NotAttempted;
        enrich_author(
            Some("UABSENT"),
            &mut author_name,
            &mut author_display_name,
            &mut author_resolution,
            &directory,
        );
        assert_eq!(author_name.as_deref(), Some("alice"));
        assert_eq!(author_display_name, None);
        assert_eq!(author_resolution, AuthorResolution::Directory);

        let mut unsafe_author_name = None;
        let mut unsafe_author_display_name = None;
        let mut unsafe_author_resolution = AuthorResolution::NotAttempted;
        enrich_author(
            Some("UUNSAFE"),
            &mut unsafe_author_name,
            &mut unsafe_author_display_name,
            &mut unsafe_author_resolution,
            &directory,
        );
        assert_eq!(unsafe_author_name, None);
        assert_eq!(unsafe_author_display_name, None);
        assert_eq!(unsafe_author_resolution, AuthorResolution::Unresolved);

        let text = "<@UABSENT> <@ULITERAL> <@UUNSAFE>";
        let (mut rendered_text, mut mention_resolution, mut mentions) =
            initial_mention_fields(text, None);
        enrich_mentions(
            text,
            None,
            &mut rendered_text,
            &mut mention_resolution,
            &mut mentions,
            &directory,
        );
        assert_eq!(mention_resolution, MentionResolution::Partial);
        assert_eq!(mentions[0].display_name, None);
        assert_eq!(mentions[1].display_name.as_deref(), Some("null"));
        assert_eq!(mentions[2].username, None);
        assert_eq!(mentions[2].display_name, None);
        let json = serde_json::to_value(mentions).unwrap();
        assert_eq!(json[0]["display_name"], serde_json::Value::Null);
        assert_eq!(json[1]["display_name"], "null");
        assert_eq!(json[2]["username"], serde_json::Value::Null);
        assert_eq!(json[2]["display_name"], serde_json::Value::Null);
    }

    fn raw_conversation(id: &str, name: &str) -> RawConversation {
        RawConversation {
            id: id.into(),
            name: name.into(),
            is_member: true,
            num_members: Some(7),
            ..RawConversation::default()
        }
    }

    #[tokio::test]
    async fn drafts_lifecycle_uses_bounded_pages_and_server_concurrency_revisions() {
        let existing = raw_draft("DR-existing", "3000.5", "C123", "old");
        let updated = raw_draft("DR-existing", "3001", "C123", "replacement");
        let mut api = fake_api();
        api.drafts_page = RawDraftsPage {
            drafts: vec![
                raw_draft("DR-list-1", "1000", "C123", "first"),
                raw_draft("DR-list-2", "2000", "C123", "second"),
            ],
            files: vec![],
            has_more: true,
        };
        api.draft_info = RawDraftResponse {
            draft: existing.clone(),
        };
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse { draft: existing },
            RawDraftResponse {
                draft: updated.clone(),
            },
            RawDraftResponse {
                draft: updated.clone(),
            },
        ]));
        let mut created = raw_draft("DR-created", "4000", "C123", "created");
        created.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        created.blocks = Some(render_markdown("**created**").unwrap().blocks);
        api.draft_create = RawDraftResponse { draft: created };
        api.draft_update = RawDraftResponse { draft: updated };
        let calls = api.draft_calls.clone();
        let service = service(api);

        let page = service.list_drafts(Some("500"), 2).await.unwrap();
        assert_eq!(page.drafts.len(), 2);
        assert!(page.drafts.iter().all(|draft| draft.is_supported));
        assert!(page.has_more);
        assert_eq!(page.next_ts.as_deref(), Some("2000"));

        let created = service
            .create_draft("C123", None, false, "**created**")
            .await
            .unwrap();
        assert_eq!(created.id, "DR-created");

        let updated = service
            .update_draft("DR-existing", "replacement")
            .await
            .unwrap();
        assert_eq!(updated.last_updated_ts, "3001");
        assert_eq!(updated.client_last_updated_ts, "3001000");

        let deleted = service.delete_draft("DR-existing", true).await.unwrap();
        assert_eq!(
            deleted,
            DraftDeleteReport {
                id: "DR-existing".into(),
                deleted: true,
                file_id: None,
                file_deleted: None,
            }
        );

        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0],
            DraftCall::List {
                next_ts: Some("500".into()),
                limit: 2
            }
        );
        let DraftCall::Create {
            client_msg_id,
            destinations,
            blocks,
            file_ids,
        } = &calls[1]
        else {
            panic!("expected draft creation call");
        };
        assert_eq!(Uuid::parse_str(client_msg_id).unwrap().get_version_num(), 4);
        assert_eq!(destinations[0].channel_id.as_deref(), Some("C123"));
        assert_eq!(blocks[0]["type"], "rich_text");
        assert!(file_ids.is_empty());
        assert_eq!(
            calls[2..],
            [
                DraftCall::Info {
                    draft_id: "DR-existing".into()
                },
                DraftCall::Update {
                    draft_id: "DR-existing".into(),
                    last_updated_ts: "9000123".into(),
                    destinations: vec![DraftDestination {
                        channel_id: Some("C123".into()),
                        ..DraftDestination::default()
                    }],
                    blocks: render_markdown("replacement").unwrap().blocks,
                    file_ids: vec![],
                },
                DraftCall::Info {
                    draft_id: "DR-existing".into()
                },
                DraftCall::Info {
                    draft_id: "DR-existing".into()
                },
                DraftCall::Delete {
                    draft_id: "DR-existing".into(),
                    last_updated_ts: "3001000".into(),
                    skip_file_deletion: false,
                }
            ]
        );
    }

    #[tokio::test]
    async fn drafts_accept_and_preserve_valid_self_dm_user_ids() {
        let existing = raw_self_dm_draft("DR-existing", "2000", "existing");
        let updated = raw_self_dm_draft("DR-existing", "2001", "updated");
        let mut api = fake_api();
        api.drafts_page = RawDraftsPage {
            drafts: vec![raw_self_dm_draft("DR-list", "1000", "listed")],
            ..RawDraftsPage::default()
        };
        api.draft_info = RawDraftResponse {
            draft: existing.clone(),
        };
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: existing.clone(),
            },
            RawDraftResponse {
                draft: updated.clone(),
            },
            RawDraftResponse { draft: updated },
            RawDraftResponse { draft: existing },
        ]));
        let mut created = raw_self_dm_draft("DR-created", "3000", "created");
        created.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        api.draft_create = RawDraftResponse { draft: created };
        api.draft_update = RawDraftResponse {
            draft: raw_self_dm_draft("DR-existing", "2001", "transient"),
        };
        let draft_calls = api.draft_calls.clone();
        let post_calls = api.post_calls.clone();
        let service = service(api);

        let page = service.list_drafts(None, 25).await.unwrap();
        assert!(page.drafts[0].is_supported);
        assert_eq!(
            page.drafts[0].destinations[0].user_ids.as_deref(),
            Some(["U123".to_owned()].as_slice())
        );
        assert_eq!(
            serde_json::to_value(&page.drafts[0].destinations[0]).unwrap(),
            json!({"channel_id": "D123", "user_ids": ["U123"]})
        );
        assert_eq!(
            serde_json::to_value(DraftDestination {
                channel_id: Some("D123".into()),
                ..DraftDestination::default()
            })
            .unwrap(),
            json!({"channel_id": "D123"})
        );

        let created = service
            .create_draft("D123", None, false, "created")
            .await
            .unwrap();
        assert!(created.is_supported);
        assert_eq!(
            created.destinations[0].user_ids.as_deref(),
            Some(["U123".to_owned()].as_slice())
        );

        let updated = service
            .update_draft("DR-existing", "updated")
            .await
            .unwrap();
        assert!(updated.is_supported);
        assert_eq!(
            updated.destinations[0].user_ids.as_deref(),
            Some(["U123".to_owned()].as_slice())
        );

        assert!(
            service
                .delete_draft("DR-existing", true)
                .await
                .unwrap()
                .deleted
        );
        assert!(
            service
                .send_draft("DR-existing", true)
                .await
                .unwrap()
                .draft_deleted
        );

        let calls = draft_calls.lock().unwrap();
        let DraftCall::Create { destinations, .. } = &calls[1] else {
            panic!("expected draft creation");
        };
        assert_eq!(destinations[0].user_ids, None);
        let DraftCall::Update { destinations, .. } = &calls[3] else {
            panic!("expected draft update");
        };
        assert_eq!(destinations[0].channel_id.as_deref(), Some("D123"));
        assert_eq!(destinations[0].user_ids, None);
        let posts = post_calls.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].channel, "D123");
        assert_eq!(posts[0].thread_ts, None);
        assert!(!posts[0].broadcast);
    }

    #[test]
    fn draft_user_ids_are_bounded_validated_and_keep_unknown_fields_unsupported() {
        let mut maximum = raw_self_dm_draft("DR-maximum", "1000", "maximum");
        maximum.destinations[0].user_ids = Some(
            (0..MAX_DRAFT_DESTINATION_USERS)
                .map(|index| format!("U{index:03}"))
                .collect(),
        );
        assert!(
            normalize_draft(maximum, "drafts.info")
                .unwrap()
                .is_supported
        );

        for user_ids in [
            Vec::new(),
            vec!["invalid-user".into()],
            (0..=MAX_DRAFT_DESTINATION_USERS)
                .map(|index| format!("U{index:03}"))
                .collect(),
        ] {
            let mut malformed = raw_self_dm_draft("DR-invalid", "1000", "invalid");
            malformed.destinations[0].user_ids = Some(user_ids);
            assert!(matches!(
                normalize_draft(malformed, "drafts.info"),
                Err(Error::InvalidResponse {
                    method: "drafts.info"
                })
            ));
        }

        let empty_user_ids = serde_json::from_value::<RawDraftResponse>(json!({
            "draft": {
                "id": "DR-empty-users",
                "last_updated_ts": "1000",
                "blocks": [{"type": "rich_text", "elements": []}],
                "destinations": [{
                    "channel_id": "D123",
                    "user_ids": []
                }]
            }
        }))
        .unwrap();
        assert!(matches!(
            normalize_draft(empty_user_ids.draft, "drafts.info"),
            Err(Error::InvalidResponse {
                method: "drafts.info"
            })
        ));

        for malformed_user_ids in [json!("U123"), json!(null)] {
            assert!(
                serde_json::from_value::<RawDraftResponse>(json!({
                    "draft": {
                        "id": "DR-invalid-type",
                        "last_updated_ts": "1000",
                        "blocks": [{"type": "rich_text", "elements": []}],
                        "destinations": [{
                            "channel_id": "D123",
                            "user_ids": malformed_user_ids
                        }]
                    }
                }))
                .is_err()
            );
        }

        let mut future = raw_self_dm_draft("DR-future", "1000", "future");
        future.destinations[0]
            .extra
            .insert("future_route".into(), json!(true));
        let normalized = normalize_draft(future, "drafts.info").unwrap();
        assert!(!normalized.is_supported);
        assert_eq!(
            normalized.destinations[0].extra.get("future_route"),
            Some(&json!(true))
        );
    }

    #[tokio::test]
    async fn draft_creation_rejects_changed_route_despite_valid_user_enrichment() {
        let mut cases = [
            (
                raw_self_dm_draft("DR-channel", "1000", "channel"),
                None,
                false,
            ),
            (
                raw_self_dm_draft("DR-thread", "1000", "thread"),
                Some("1000.000001"),
                false,
            ),
            (
                raw_self_dm_draft("DR-broadcast", "1000", "broadcast"),
                Some("1000.000001"),
                false,
            ),
        ];
        cases[0].0.destinations[0].channel_id = Some("D999".into());
        cases[1].0.destinations[0].thread_ts = Some("2000.000001".into());
        cases[2].0.destinations[0].thread_ts = Some("1000.000001".into());
        cases[2].0.destinations[0].broadcast = true;

        for (draft, requested_thread, requested_broadcast) in cases {
            let mut api = fake_api();
            let mut draft = draft;
            draft.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
            api.draft_create = RawDraftResponse { draft };
            let calls = api.draft_calls.clone();
            let error = service(api)
                .create_draft("D123", requested_thread, requested_broadcast, "synthetic")
                .await
                .unwrap_err();
            assert_creation_uncertain_matches_request(error, &calls);
        }
    }

    #[tokio::test]
    async fn text_draft_creation_rejects_each_mismatched_acknowledgement_dimension() {
        for mismatch in ["client_msg_id", "file_ids", "blocks"] {
            let mut response = raw_draft("DR-created", "1000", "C123", "synthetic");
            response.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
            match mismatch {
                "client_msg_id" => {
                    response.client_msg_id = Some("00000000-0000-4000-8000-000000000002".into());
                }
                "file_ids" => response.file_ids = vec!["F123".into()],
                "blocks" => {
                    response.blocks = Some(render_markdown("different").unwrap().blocks);
                }
                _ => unreachable!(),
            }
            let mut api = fake_api();
            api.draft_create = RawDraftResponse { draft: response };
            let calls = api.draft_calls.clone();

            let error = service(api)
                .create_draft("C123", None, false, "synthetic")
                .await
                .unwrap_err();

            assert_creation_uncertain_matches_request(error, &calls);
        }
    }

    #[tokio::test]
    async fn text_draft_creation_reconciles_ambiguous_or_mismatched_acknowledgements() {
        for response in ["timeout", "mismatched"] {
            let mut exact = raw_draft("DR-created", "1000", "C123", "synthetic");
            exact.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
            let mut api = fake_api();
            api.drafts_page = active_drafts(vec![exact]);
            if response == "timeout" {
                api.draft_create_error = Some("timeout");
            } else {
                api.draft_create = RawDraftResponse {
                    draft: raw_draft("DR-unrelated", "1000", "C999", "unrelated"),
                };
            }
            let calls = api.draft_calls.clone();

            let created = service(api)
                .create_draft("C123", None, false, "synthetic")
                .await
                .unwrap();

            assert_eq!(created.id, "DR-created");
            let calls = calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| matches!(call, DraftCall::Create { .. }))
                    .count(),
                1
            );
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| matches!(call, DraftCall::List { .. }))
                    .count(),
                1
            );
        }
    }

    #[tokio::test]
    async fn unresolved_text_draft_creation_retains_the_client_id_without_retrying() {
        let mut api = fake_api();
        api.draft_create_error = Some("timeout");
        let calls = api.draft_calls.clone();

        let error = service(api)
            .create_draft("C123", None, false, "synthetic")
            .await
            .unwrap_err();
        assert_creation_uncertain_matches_request(error, &calls);

        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::Create { .. }))
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn duplicate_text_draft_creation_reconciliation_stays_uncertain() {
        let mut first = raw_draft("DR-first", "1000", "C123", "synthetic");
        first.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        let mut second = raw_draft("DR-second", "1001", "C123", "synthetic");
        second.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        let mut api = fake_api();
        api.draft_create_error = Some("timeout");
        api.drafts_page = active_drafts(vec![first, second]);
        let calls = api.draft_calls.clone();

        let error = service(api)
            .create_draft("C123", None, false, "synthetic")
            .await
            .unwrap_err();

        assert_creation_uncertain_matches_request(error, &calls);
    }

    #[tokio::test]
    async fn draft_update_rejects_changed_route_despite_valid_user_enrichment() {
        let mut cases = [
            raw_self_dm_draft("DR-existing", "2001", "channel"),
            raw_self_dm_draft("DR-existing", "2001", "thread"),
            raw_self_dm_draft("DR-existing", "2001", "broadcast"),
        ];
        cases[0].destinations[0].channel_id = Some("D999".into());
        cases[1].destinations[0].thread_ts = Some("2000.000001".into());
        cases[2].destinations[0].thread_ts = Some("1000.000001".into());
        cases[2].destinations[0].broadcast = true;

        for draft in cases {
            let mut api = fake_api();
            let current = raw_self_dm_draft("DR-existing", "2000", "existing");
            api.draft_info = RawDraftResponse {
                draft: current.clone(),
            };
            api.draft_infos = Mutex::new(VecDeque::from([
                RawDraftResponse { draft: current },
                RawDraftResponse {
                    draft: draft.clone(),
                },
            ]));
            api.draft_update = RawDraftResponse { draft };
            assert!(matches!(
                service(api).update_draft("DR-existing", "synthetic").await,
                Err(Error::DraftMutationUncertain {
                    action: "update",
                    ..
                })
            ));
        }
    }

    #[test]
    fn converts_server_draft_revisions_to_browser_mutation_timestamps() {
        assert_eq!(
            server_revision_to_client_timestamp("3000.5").as_deref(),
            Some("3000500")
        );
        assert_eq!(
            server_revision_to_client_timestamp("1.234567").as_deref(),
            Some("1234.567")
        );
        assert_eq!(
            server_revision_to_client_timestamp("0001.000000").as_deref(),
            Some("1000")
        );
        assert_eq!(
            server_revision_to_client_timestamp("0.000001").as_deref(),
            Some("0.001")
        );
    }

    #[test]
    fn classifies_only_ambiguous_post_outcomes_as_publication_uncertain() {
        for error in [
            Error::HttpStatus {
                method: "chat.postMessage",
                status: 502,
            },
            Error::ResponseTooLarge {
                method: "chat.postMessage",
                limit: 1024,
            },
            Error::InvalidResponse {
                method: "chat.postMessage",
            },
            Error::Timeout {
                method: "chat.postMessage",
            },
            Error::Transport {
                method: "chat.postMessage",
            },
            Error::SlackApi {
                method: "chat.postMessage",
                code: "fatal_error".into(),
            },
            Error::SlackApi {
                method: "chat.postMessage",
                code: "internal_error".into(),
            },
        ] {
            assert!(matches!(
                classify_publication_error("client-id", error),
                Error::PublicationUncertain { client_msg_id } if client_msg_id == "client-id"
            ));
        }
        assert!(matches!(
            classify_publication_error(
                "client-id",
                Error::SlackApi {
                    method: "chat.postMessage",
                    code: "restricted_action".into()
                }
            ),
            Error::SlackApi { code, .. } if code == "restricted_action"
        ));
    }

    #[tokio::test]
    async fn drafts_write_gate_validation_and_confirmation_fail_before_network_io() {
        let api = fake_api();
        let calls = api.draft_calls.clone();
        let upload_calls = api.upload_calls.clone();
        let service = service(api);
        let fixture = UploadFixture::new(b"synthetic");

        assert!(service.list_drafts(None, 0).await.is_err());
        assert!(service.list_drafts(Some("."), 25).await.is_err());
        assert!(service.get_draft("bad id").await.is_err());
        assert!(
            service
                .create_draft("C123", None, true, "content")
                .await
                .is_err()
        );
        assert!(
            service
                .create_draft("C123", None, false, "content\u{0}")
                .await
                .is_err()
        );
        assert!(
            service
                .update_draft("DR-valid", "content\u{0}")
                .await
                .is_err()
        );
        for result in [
            service
                .create_draft("C123", None, false, "<https://example.com/draft|Draft>")
                .await,
            service
                .update_draft("DR-valid", "<https://example.com/draft|Updated draft>")
                .await,
        ] {
            assert!(matches!(
                result,
                Err(Error::InvalidInput {
                    field: "markdown",
                    ..
                })
            ));
        }
        assert!(matches!(
            service
                .create_file_draft(
                    FileDraftCreateRequest {
                        conversation: "C123",
                        thread_ts: None,
                        broadcast: false,
                        markdown: "<https://example.com/file|File draft>",
                        title: None,
                        alt_text: Some("Synthetic test file"),
                        confirmed: true,
                    },
                    fixture.source(),
                )
                .await,
            Err(Error::InvalidInput {
                field: "markdown",
                ..
            })
        ));
        assert!(matches!(
            service.delete_draft("DR-valid", false).await,
            Err(Error::ConfirmationRequired {
                action: "draft deletion"
            })
        ));
        assert!(calls.lock().unwrap().is_empty());
        assert!(upload_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn drafts_reject_stale_pagination_and_unsupported_mutation_shapes() {
        let mut repeated = fake_api();
        repeated.drafts_page = RawDraftsPage {
            drafts: vec![raw_draft("DR-one", "500", "C123", "one")],
            has_more: true,
            ..RawDraftsPage::default()
        };
        assert!(matches!(
            service(repeated).list_drafts(Some("500"), 25).await,
            Err(Error::InvalidResponse {
                method: "drafts.list"
            })
        ));

        let mut unsupported = fake_api();
        let mut attached = raw_draft("DR-attached", "600", "C123", "attached");
        attached.file_ids.push("F123".into());
        unsupported.draft_info = RawDraftResponse { draft: attached };
        let calls = unsupported.draft_calls.clone();
        let service = service(unsupported);
        assert!(matches!(
            service.update_draft("DR-attached", "new").await,
            Err(Error::InvalidInput { field: "draft", .. })
        ));
        assert!(matches!(
            service.delete_draft("DR-attached", true).await,
            Err(Error::InvalidInput { field: "draft", .. })
        ));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                DraftCall::Info {
                    draft_id: "DR-attached".into()
                },
                DraftCall::List {
                    next_ts: None,
                    limit: MAX_DRAFTS,
                },
                DraftCall::Info {
                    draft_id: "DR-attached".into()
                },
                DraftCall::List {
                    next_ts: None,
                    limit: MAX_DRAFTS,
                },
            ]
        );
    }

    #[tokio::test]
    async fn one_file_draft_read_requires_global_exclusive_private_file_proof() {
        let mut api = fake_api();
        let draft = raw_file_draft("DR-file", "610", "C123", "attached", "FPRIVATE");
        api.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        api.drafts_page = active_drafts(vec![draft.clone()]);
        api.file_response = RawFileResponse {
            file: private_draft_file("FPRIVATE"),
        };
        let service = service(api);

        let verified = service.get_draft("DR-file").await.unwrap();
        assert!(verified.is_supported);
        assert_eq!(verified.file_ids, ["FPRIVATE"]);
        assert_eq!(
            verified.file_association,
            Some(FileDraftAssociation::Verified)
        );
    }

    #[tokio::test]
    async fn one_file_draft_read_fails_closed_on_duplicate_unknown_or_incomplete_state() {
        let draft = raw_file_draft("DR-file", "620", "C123", "attached", "FPRIVATE");
        let mut duplicate = fake_api();
        duplicate.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        duplicate.draft_pages = Mutex::new(VecDeque::from([
            RawDraftsPage {
                drafts: vec![draft.clone()],
                has_more: true,
                ..RawDraftsPage::default()
            },
            active_drafts(vec![raw_file_draft(
                "DR-other", "619", "C123", "other", "FPRIVATE",
            )]),
        ]));
        let duplicate = service(duplicate).get_draft("DR-file").await.unwrap();
        assert!(!duplicate.is_supported);
        assert_eq!(
            duplicate.file_association,
            Some(FileDraftAssociation::Unverified)
        );

        let mut unknown = draft.clone();
        unknown.extra.insert("future_field".into(), json!(true));
        let mut api = fake_api();
        api.draft_info = RawDraftResponse { draft: unknown };
        let unknown = service(api).get_draft("DR-file").await.unwrap();
        assert!(!unknown.is_supported);

        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        api.drafts_page = active_drafts(vec![draft.clone()]);
        api.file_response = RawFileResponse {
            file: RawFile {
                id: "FPRIVATE".into(),
                is_public: Some(false),
                public_url_shared: Some(false),
                ..RawFile::default()
            },
        };
        let incomplete = service(api).get_draft("DR-file").await.unwrap();
        assert!(!incomplete.is_supported);

        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        api.drafts_page = active_drafts(vec![draft]);
        let mut external = private_draft_file("FPRIVATE");
        external.is_external = Some(true);
        external.mode = Some("external".into());
        api.file_response = RawFileResponse { file: external };
        let external = service(api).get_draft("DR-file").await.unwrap();
        assert!(!external.is_supported);
        assert_eq!(
            external.file_association,
            Some(FileDraftAssociation::Unverified)
        );
    }

    #[tokio::test]
    async fn file_draft_ownership_scan_fails_closed_on_cursor_loops_and_scan_caps() {
        let target = raw_file_draft("DR-file", "630", "C123", "attached", "FPRIVATE");
        let mut looped = fake_api();
        looped.draft_info = RawDraftResponse {
            draft: target.clone(),
        };
        looped.draft_pages = Mutex::new(VecDeque::from([
            RawDraftsPage {
                drafts: vec![target.clone()],
                has_more: true,
                ..RawDraftsPage::default()
            },
            RawDraftsPage {
                drafts: vec![raw_draft("DR-loop", "630", "C123", "loop")],
                has_more: true,
                ..RawDraftsPage::default()
            },
        ]));
        let loop_calls = looped.draft_calls.clone();
        let looped = service(looped).get_draft("DR-file").await.unwrap();
        assert!(!looped.is_supported);
        assert_eq!(
            loop_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            2
        );

        let mut capped = fake_api();
        capped.draft_info = RawDraftResponse {
            draft: target.clone(),
        };
        capped.draft_pages = Mutex::new(
            (0..MAX_DRAFT_OWNERSHIP_SCAN_PAGES)
                .map(|index| RawDraftsPage {
                    drafts: vec![if index == 0 {
                        target.clone()
                    } else {
                        raw_draft(
                            &format!("DR-cap-{index}"),
                            &format!("{}", 630 + index),
                            "C123",
                            "cap",
                        )
                    }],
                    has_more: true,
                    ..RawDraftsPage::default()
                })
                .collect::<VecDeque<_>>(),
        );
        let cap_calls = capped.draft_calls.clone();
        let capped = service(capped).get_draft("DR-file").await.unwrap();
        assert!(!capped.is_supported);
        assert_eq!(
            capped.file_association,
            Some(FileDraftAssociation::Unverified)
        );
        assert_eq!(
            cap_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            MAX_DRAFT_OWNERSHIP_SCAN_PAGES
        );
    }

    #[tokio::test]
    async fn creates_one_file_draft_only_after_private_cross_process_proof() {
        const MARKDOWN: &str = "Review with [@Alice](slack-user:alice).";
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        let mut draft = raw_file_draft("DR-created-file", "700", "C123", "body", "FUPLOAD");
        draft.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        draft.blocks = Some(
            render_markdown_with_mentions(
                MARKDOWN,
                &[ResolvedOutboundUser {
                    reference: "alice".into(),
                    user_id: "UALICE".into(),
                    resolution: OutboundMentionResolution::Username,
                }],
            )
            .unwrap()
            .blocks,
        );
        draft.blocks.as_mut().unwrap()[0]["block_id"] = json!("B1234");
        api.draft_create = RawDraftResponse {
            draft: draft.clone(),
        };
        api.drafts_page = active_drafts(vec![draft.clone()]);
        api.draft_info = RawDraftResponse { draft };
        let initial_file = private_draft_file("FUPLOAD");
        let mut enriched_file = initial_file.clone();
        enriched_file.title = Some("Asynchronously enriched title".into());
        enriched_file.mimetype = Some("text/plain".into());
        enriched_file.timestamp = Some(1_700_000_000);
        enriched_file.url_private = Some("https://files.slack.com/files-pri/synthetic".into());
        api.file_responses = Mutex::new(VecDeque::from([
            RawFileResponse { file: initial_file },
            RawFileResponse {
                file: enriched_file,
            },
        ]));
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("UALICE", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        let upload_calls = api.upload_calls.clone();
        let draft_calls = api.draft_calls.clone();
        let service = service(api);

        let report = service
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "C123",
                    thread_ts: None,
                    broadcast: false,
                    markdown: MARKDOWN,
                    title: None,
                    alt_text: Some("Synthetic test file"),
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap();
        let FileDraftCreateReport::Created {
            draft,
            file,
            reconciled,
        } = report
        else {
            panic!("expected a proved file draft");
        };
        assert_eq!(draft.id, "DR-created-file");
        assert_eq!(draft.file_association, Some(FileDraftAssociation::Verified));
        assert_eq!(file.id, "FUPLOAD");
        assert_eq!(file.title.as_deref(), Some("Asynchronously enriched title"));
        assert_eq!(file.mimetype.as_deref(), Some("text/plain"));
        assert!(!reconciled);
        assert_eq!(
            draft.blocks.as_ref().unwrap()[0]["elements"][0]["elements"][1],
            json!({"type": "user", "user_id": "UALICE"})
        );
        assert_eq!(
            upload_calls.lock().unwrap().as_slice(),
            ["allocate", "transfer", "complete"]
        );
        let calls = draft_calls.lock().unwrap();
        assert!(matches!(
            &calls[0],
            DraftCall::Create { file_ids, .. } if file_ids == &["FUPLOAD"]
        ));
        assert!(matches!(&calls[1], DraftCall::List { .. }));
        assert!(matches!(&calls[2], DraftCall::Info { .. }));
    }

    #[tokio::test]
    async fn file_draft_creation_reports_every_recoverable_pre_draft_stage() {
        let fixture = UploadFixture::new(b"synthetic");

        let mut allocation_uncertain = fake_api();
        allocation_uncertain.upload_allocation_error = Some("timeout");
        assert!(matches!(
            create_file_draft_report(allocation_uncertain, &fixture).await,
            FileDraftCreateReport::AllocationUncertain
        ));

        let mut allocated = fake_api();
        allocated.upload_allocation.upload_url = None;
        assert!(matches!(
            create_file_draft_report(allocated, &fixture).await,
            FileDraftCreateReport::Allocated { file_id } if file_id == "FUPLOAD"
        ));

        let mut source_changed = fake_api();
        source_changed.upload_mutate_pass = true;
        assert!(matches!(
            create_file_draft_report(source_changed, &fixture).await,
            FileDraftCreateReport::SourceChanged { file_id } if file_id == "FUPLOAD"
        ));

        let mut transfer_uncertain = fake_api();
        transfer_uncertain.upload_transfer_error = true;
        assert!(matches!(
            create_file_draft_report(transfer_uncertain, &fixture).await,
            FileDraftCreateReport::TransferUncertain { file_id } if file_id == "FUPLOAD"
        ));

        let mut completion_uncertain = fake_api();
        completion_uncertain.upload_completion_error = true;
        assert!(matches!(
            create_file_draft_report(completion_uncertain, &fixture).await,
            FileDraftCreateReport::FileCompletionUncertain { file_id }
                if file_id == "FUPLOAD"
        ));

        let mut draft_rejected = fake_api();
        draft_rejected.file_response = RawFileResponse {
            file: private_draft_file("FUPLOAD"),
        };
        draft_rejected.draft_create_error = Some("restricted_action");
        let calls = draft_rejected.draft_calls.clone();
        let report = create_file_draft_report(draft_rejected, &fixture).await;
        assert!(matches!(
            report,
            FileDraftCreateReport::DraftNotCreated { file_id, reason }
                if file_id == "FUPLOAD" && reason.contains("restricted_action")
        ));
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Create { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn file_draft_creation_requires_exact_requested_blocks() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        let mut response = raw_file_draft("DR-created-file", "710", "C123", "body", "FUPLOAD");
        response.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        response.blocks = Some(render_markdown("**body**").unwrap().blocks);
        api.draft_create = RawDraftResponse {
            draft: response.clone(),
        };
        let mut wrong = response;
        wrong.blocks = Some(render_markdown("different").unwrap().blocks);
        api.drafts_page = active_drafts(vec![wrong.clone()]);
        api.draft_info = RawDraftResponse { draft: wrong };
        api.file_response = RawFileResponse {
            file: private_draft_file("FUPLOAD"),
        };
        let calls = api.draft_calls.clone();

        let report = service(api)
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "C123",
                    thread_ts: None,
                    broadcast: false,
                    markdown: "**body**",
                    title: None,
                    alt_text: Some("Synthetic test file"),
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap();
        assert!(matches!(
            report,
            FileDraftCreateReport::DraftCreationUncertain {
                file_id,
                client_msg_id: _
            } if file_id == "FUPLOAD"
        ));
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Create { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn ambiguous_file_draft_creation_reconciles_exact_state_or_stays_uncertain() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut exact_api = fake_api();
        let mut exact = raw_file_draft("DR-created-file", "720", "C123", "body", "FUPLOAD");
        exact.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        exact.blocks = Some(render_markdown("**body**").unwrap().blocks);
        exact_api.draft_create_error = Some("timeout");
        exact_api.drafts_page = active_drafts(vec![exact.clone()]);
        exact_api.draft_info = RawDraftResponse { draft: exact };
        exact_api.file_response = RawFileResponse {
            file: private_draft_file("FUPLOAD"),
        };
        let exact_calls = exact_api.draft_calls.clone();
        let report = service(exact_api)
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "C123",
                    thread_ts: None,
                    broadcast: false,
                    markdown: "**body**",
                    title: None,
                    alt_text: Some("Synthetic test file"),
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap();
        assert!(matches!(
            report,
            FileDraftCreateReport::Created {
                reconciled: true,
                ..
            }
        ));
        assert_eq!(
            exact_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Create { .. }))
                .count(),
            1
        );

        let mut absent_api = fake_api();
        absent_api.draft_create_error = Some("timeout");
        absent_api.drafts_page = active_drafts(vec![]);
        absent_api.file_response = RawFileResponse {
            file: private_draft_file("FUPLOAD"),
        };
        let absent_calls = absent_api.draft_calls.clone();
        let report = service(absent_api)
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "C123",
                    thread_ts: None,
                    broadcast: false,
                    markdown: "**body**",
                    title: None,
                    alt_text: Some("Synthetic test file"),
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap();
        assert!(matches!(
            report,
            FileDraftCreateReport::DraftCreationUncertain {
                file_id,
                client_msg_id: _
            } if file_id == "FUPLOAD"
        ));
        assert_eq!(
            absent_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Create { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn updates_one_file_draft_with_the_exact_file_and_reproves_state() {
        let current = raw_file_draft("DR-file", "800", "C123", "old", "FPRIVATE");
        let mut updated = raw_file_draft("DR-file", "801", "C123", "replacement", "FPRIVATE");
        updated.blocks = Some(render_markdown("replacement").unwrap().blocks);
        updated.blocks.as_mut().unwrap()[0]["block_id"] = json!("B1234");
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: current.clone(),
            },
            RawDraftResponse {
                draft: current.clone(),
            },
            RawDraftResponse {
                draft: updated.clone(),
            },
            RawDraftResponse {
                draft: updated.clone(),
            },
            RawDraftResponse {
                draft: updated.clone(),
            },
        ]));
        api.draft_pages = Mutex::new(VecDeque::from([
            active_drafts(vec![current.clone()]),
            active_drafts(vec![current]),
            active_drafts(vec![updated.clone()]),
        ]));
        let mut transient = updated;
        transient.blocks = Some(render_markdown("transient").unwrap().blocks);
        transient
            .extra
            .insert("client_last_updated_ts".into(), json!("synthetic"));
        api.draft_update = RawDraftResponse { draft: transient };
        api.file_response = RawFileResponse {
            file: private_draft_file("FPRIVATE"),
        };
        let calls = api.draft_calls.clone();
        let mut service = service(api);
        service.draft_reconciliation_delays_ms = &[0, 0];

        let updated = service
            .update_draft("DR-file", "replacement")
            .await
            .unwrap();
        assert!(updated.is_supported);
        assert_eq!(
            updated.file_association,
            Some(FileDraftAssociation::Verified)
        );
        assert!(calls.lock().unwrap().iter().any(|call| {
            matches!(
                call,
                DraftCall::Update { file_ids, .. } if file_ids == &["FPRIVATE"]
            )
        }));
    }

    #[tokio::test]
    async fn ambiguous_one_file_draft_update_is_not_retried() {
        let current = raw_file_draft("DR-file", "810", "C123", "old", "FPRIVATE");
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: current.clone(),
            },
            RawDraftResponse {
                draft: current.clone(),
            },
        ]));
        api.drafts_page = active_drafts(vec![current]);
        api.file_response = RawFileResponse {
            file: private_draft_file("FPRIVATE"),
        };
        api.draft_update_error = Some("timeout");
        let calls = api.draft_calls.clone();

        assert!(matches!(
            service(api).update_draft("DR-file", "replacement").await,
            Err(Error::DraftMutationUncertain {
                action: "update",
                ..
            })
        ));
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Update { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn one_file_draft_deletion_always_preserves_the_file() {
        for ambiguous in [false, true] {
            let draft = raw_file_draft("DR-file", "900", "C123", "delete", "FPRIVATE");
            let concurrent = raw_file_draft("DR-concurrent", "901", "C123", "other", "FPRIVATE");
            let mut api = fake_api();
            api.draft_infos = Mutex::new(VecDeque::from([
                RawDraftResponse {
                    draft: draft.clone(),
                },
                RawDraftResponse {
                    draft: draft.clone(),
                },
            ]));
            api.draft_pages = Mutex::new(VecDeque::from([
                active_drafts(vec![draft]),
                active_drafts(vec![concurrent]),
            ]));
            api.file_response = RawFileResponse {
                file: private_draft_file("FPRIVATE"),
            };
            api.draft_delete_ambiguous = ambiguous;
            let calls = api.draft_calls.clone();
            let file_info_calls = api.file_info_calls.clone();
            let service = service(api);

            let report = service.delete_draft("DR-file", true).await.unwrap();
            assert_eq!(report.file_id.as_deref(), Some("FPRIVATE"));
            assert_eq!(report.file_deleted, Some(false));
            let calls = calls.lock().unwrap();
            assert_eq!(
                calls
                    .iter()
                    .filter(|call| matches!(call, DraftCall::Delete { .. }))
                    .count(),
                1
            );
            assert!(calls.iter().any(|call| {
                matches!(
                    call,
                    DraftCall::Delete {
                        skip_file_deletion: true,
                        ..
                    }
                )
            }));
            assert_eq!(file_info_calls.lock().unwrap().as_slice(), ["FPRIVATE"]);
        }
    }

    #[tokio::test]
    async fn unresolved_one_file_draft_delete_is_bounded_and_not_retried() {
        let draft = raw_file_draft("DR-file", "910", "C123", "delete", "FPRIVATE");
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.draft_pages = Mutex::new(
            std::iter::repeat_with(|| active_drafts(vec![draft.clone()]))
                .take(7)
                .collect::<VecDeque<_>>(),
        );
        api.file_response = RawFileResponse {
            file: private_draft_file("FPRIVATE"),
        };
        api.draft_delete_ambiguous = true;
        let calls = api.draft_calls.clone();
        let mut service = service(api);
        service.draft_reconciliation_delays_ms = &[0, 0, 0, 0, 0, 0];

        assert!(matches!(
            service.delete_draft("DR-file", true).await,
            Err(Error::DraftMutationUncertain {
                action: "delete",
                ..
            })
        ));
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            7
        );
        assert!(calls.iter().any(|call| {
            matches!(
                call,
                DraftCall::Delete {
                    skip_file_deletion: true,
                    ..
                }
            )
        }));
    }

    #[tokio::test]
    async fn text_draft_delete_reconciles_ambiguity_or_reports_uncertain() {
        let current = raw_draft("DR-text", "920", "C123", "delete");
        let mut absent = fake_api();
        absent.draft_info = RawDraftResponse {
            draft: current.clone(),
        };
        absent.drafts_page = active_drafts(vec![]);
        absent.draft_delete_ambiguous = true;
        let calls = absent.draft_calls.clone();

        let report = service(absent).delete_draft("DR-text", true).await.unwrap();
        assert!(report.deleted);
        assert_eq!(report.file_id, None);
        assert_eq!(report.file_deleted, None);
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            1
        );

        let mut unresolved = fake_api();
        unresolved.draft_info = RawDraftResponse {
            draft: current.clone(),
        };
        unresolved.drafts_page = active_drafts(vec![current]);
        unresolved.draft_delete_ambiguous = true;
        let calls = unresolved.draft_calls.clone();
        let mut service = service(unresolved);
        service.draft_reconciliation_delays_ms = &[0, 0, 0, 0, 0, 0];

        assert!(matches!(
            service.delete_draft("DR-text", true).await,
            Err(Error::DraftMutationUncertain {
                action: "delete",
                ..
            })
        ));
        assert_eq!(
            calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn publishes_one_file_draft_once_then_preserves_file_during_cleanup() {
        let mut draft = raw_file_draft("DR-file", "1000", "D123", "publish", "FPRIVATE");
        draft.blocks.as_mut().unwrap()[0]["block_id"] = json!("server-block-id");
        let blocks = draft.blocks.clone();
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.drafts_page = active_drafts(vec![draft]);
        api.file_info_results = Mutex::new(VecDeque::from([
            Ok(RawFileResponse {
                file: private_draft_file("FPRIVATE"),
            }),
            Ok(RawFileResponse {
                file: published_dm_file("FPRIVATE", "7000.000001"),
            }),
        ]));
        api.message_list.messages.insert(
            "published".into(),
            RawMessage {
                ts: "7000.000001".into(),
                text: "publish".into(),
                blocks,
                files: vec![RawFile {
                    id: "FPRIVATE".into(),
                    ..RawFile::default()
                }],
                ..RawMessage::default()
            },
        );
        let file_share_calls = api.file_share_calls.clone();
        let post_calls = api.post_calls.clone();
        let draft_calls = api.draft_calls.clone();
        let service = service(api);

        let report = service.send_draft("DR-file", true).await.unwrap();
        assert!(report.draft_deleted);
        let shares = file_share_calls.lock().unwrap();
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].channel, "D123");
        assert_eq!(shares[0].thread_ts, None);
        assert!(!shares[0].broadcast);
        assert_eq!(shares[0].draft_id, "DR-file");
        assert_eq!(shares[0].file_id, "FPRIVATE");
        assert_eq!(shares[0].blocks[0]["type"], "rich_text");
        assert!(shares[0].blocks[0].get("block_id").is_none());
        assert_eq!(
            Uuid::parse_str(&shares[0].client_msg_id)
                .unwrap()
                .get_version_num(),
            4
        );
        assert!(post_calls.lock().unwrap().is_empty());
        assert!(draft_calls.lock().unwrap().iter().any(|call| {
            matches!(
                call,
                DraftCall::Delete {
                    skip_file_deletion: true,
                    ..
                }
            )
        }));
    }

    #[tokio::test]
    async fn one_file_draft_publication_requires_exact_readback_before_cleanup() {
        let draft = raw_file_draft("DR-file", "1010", "D123", "publish", "FPRIVATE");
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.drafts_page = active_drafts(vec![draft]);
        api.file_info_results = Mutex::new(VecDeque::from([
            Ok(RawFileResponse {
                file: private_draft_file("FPRIVATE"),
            }),
            Ok(RawFileResponse {
                file: published_dm_file("FPRIVATE", "7000.000001"),
            }),
        ]));
        api.message_list.messages.insert(
            "published".into(),
            RawMessage {
                ts: "7000.000001".into(),
                text: "wrong content".into(),
                blocks: Some(vec![json!({
                    "type": "rich_text",
                    "elements": [{
                        "type": "rich_text_section",
                        "elements": [{"type": "text", "text": "wrong content"}]
                    }]
                })]),
                files: vec![RawFile {
                    id: "FPRIVATE".into(),
                    ..RawFile::default()
                }],
                ..RawMessage::default()
            },
        );
        let file_share_calls = api.file_share_calls.clone();
        let draft_calls = api.draft_calls.clone();
        let mut service = service(api);
        service.draft_reconciliation_delays_ms = &[0, 0, 0, 0, 0, 0];

        assert!(matches!(
            service.send_draft("DR-file", true).await,
            Err(Error::PublicationUncertain { .. })
        ));
        assert_eq!(file_share_calls.lock().unwrap().len(), 1);
        assert_eq!(
            draft_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn ambiguous_file_share_reconciles_once_and_accepts_atomic_draft_removal() {
        let draft = raw_file_draft("DR-file", "1020", "D123", "publish", "FPRIVATE");
        let blocks = draft.blocks.clone();
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.draft_pages = Mutex::new(VecDeque::from([
            active_drafts(vec![draft]),
            active_drafts(vec![]),
        ]));
        api.file_info_results = Mutex::new(VecDeque::from([
            Ok(RawFileResponse {
                file: private_draft_file("FPRIVATE"),
            }),
            Ok(RawFileResponse {
                file: published_dm_file_without_share("FPRIVATE"),
            }),
        ]));
        let published = RawMessage {
            ts: "7000.000001".into(),
            text: "publish".into(),
            blocks,
            files: vec![RawFile {
                id: "FPRIVATE".into(),
                ..RawFile::default()
            }],
            ..RawMessage::default()
        };
        api.history.messages = vec![published.clone()];
        api.message_list
            .messages
            .insert("published".into(), published);
        api.file_share_error = Some("internal_error".into());
        let file_share_calls = api.file_share_calls.clone();
        let post_calls = api.post_calls.clone();
        let draft_calls = api.draft_calls.clone();
        let history_calls = api.history_calls.clone();

        let report = service(api).send_draft("DR-file", true).await.unwrap();
        assert!(report.draft_deleted);
        assert!(report.cleanup_warning.is_none());
        assert_eq!(file_share_calls.lock().unwrap().len(), 1);
        assert!(post_calls.lock().unwrap().is_empty());
        assert_eq!(history_calls.lock().unwrap().len(), 1);
        assert_eq!(
            draft_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn file_draft_publication_accepts_a_self_threaded_channel_root() {
        let draft = raw_file_draft("DR-file", "1021", "C123", "publish", "FPRIVATE");
        let blocks = draft.blocks.clone();
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.draft_pages = Mutex::new(VecDeque::from([
            active_drafts(vec![draft]),
            active_drafts(vec![]),
        ]));
        api.file_info_results = Mutex::new(VecDeque::from([
            Ok(RawFileResponse {
                file: private_draft_file("FPRIVATE"),
            }),
            Ok(RawFileResponse {
                file: published_channel_file("FPRIVATE", "7001.000001", Some("7001.000001")),
            }),
        ]));
        api.message_list.messages.insert(
            "published".into(),
            RawMessage {
                ts: "7001.000001".into(),
                thread_ts: Some("7001.000001".into()),
                text: "publish".into(),
                blocks,
                files: vec![RawFile {
                    id: "FPRIVATE".into(),
                    ..RawFile::default()
                }],
                ..RawMessage::default()
            },
        );
        let file_share_calls = api.file_share_calls.clone();
        let draft_calls = api.draft_calls.clone();

        let report = service(api).send_draft("DR-file", true).await.unwrap();
        assert!(report.draft_deleted);
        assert!(report.cleanup_warning.is_none());
        assert_eq!(
            report.sent.message.thread_ts.as_deref(),
            Some("7001.000001")
        );
        assert_eq!(file_share_calls.lock().unwrap().len(), 1);
        assert_eq!(
            draft_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn definitive_file_share_rejection_does_not_delete_or_fallback_post() {
        let draft = raw_file_draft("DR-file", "1030", "D123", "publish", "FPRIVATE");
        let mut api = fake_api();
        api.draft_infos = Mutex::new(VecDeque::from([
            RawDraftResponse {
                draft: draft.clone(),
            },
            RawDraftResponse {
                draft: draft.clone(),
            },
        ]));
        api.drafts_page = active_drafts(vec![draft]);
        api.file_response = RawFileResponse {
            file: private_draft_file("FPRIVATE"),
        };
        api.file_share_error = Some("restricted_action".into());
        let file_share_calls = api.file_share_calls.clone();
        let post_calls = api.post_calls.clone();
        let draft_calls = api.draft_calls.clone();

        assert!(matches!(
            service(api).send_draft("DR-file", true).await,
            Err(Error::SlackApi {
                method: "files.share",
                code
            }) if code == "restricted_action"
        ));
        assert_eq!(file_share_calls.lock().unwrap().len(), 1);
        assert!(post_calls.lock().unwrap().is_empty());
        assert_eq!(
            draft_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            0
        );
    }

    #[tokio::test]
    async fn drafts_info_rejects_a_mismatched_response_id_before_mutation() {
        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: raw_draft("DR-other", "700", "C123", "other"),
        };
        let calls = api.draft_calls.clone();
        let service = service(api);

        assert!(matches!(
            service.update_draft("DR-requested", "new").await,
            Err(Error::InvalidResponse {
                method: "drafts.info"
            })
        ));
        assert!(matches!(
            service.delete_draft("DR-requested", true).await,
            Err(Error::InvalidResponse {
                method: "drafts.info"
            })
        ));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            [
                DraftCall::Info {
                    draft_id: "DR-requested".into()
                },
                DraftCall::Info {
                    draft_id: "DR-requested".into()
                }
            ]
        );
    }

    #[tokio::test]
    async fn send_root_and_reply_use_fresh_uuid_forms_and_shared_validation() {
        let api = fake_api();
        let post_calls = api.post_calls.clone();
        let service = service(api);

        let root = service
            .send_message("C123", None, false, "root <@U456>", true)
            .await
            .unwrap();
        let reply = service
            .send_message("C123", Some("6000.000001"), true, "reply", true)
            .await
            .unwrap();
        assert_eq!(root.message.channel_id, "C123");
        assert_eq!(root.message.thread_ts, None);
        assert_eq!(root.message.text, "root <@U456>");
        assert_eq!(root.message.rendered_text, "root <@U456>");
        assert_eq!(
            root.message.mention_resolution,
            MentionResolution::NotNeeded
        );
        assert!(root.message.mentions.is_empty());
        assert_eq!(reply.message.thread_ts.as_deref(), Some("6000.000001"));
        assert_ne!(root.client_msg_id, reply.client_msg_id);
        for id in [&root.client_msg_id, &reply.client_msg_id] {
            assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 4);
        }

        assert!(matches!(
            service
                .send_message("C123", None, false, "unconfirmed", false)
                .await,
            Err(Error::ConfirmationRequired {
                action: "message publication"
            })
        ));
        assert!(matches!(
            service
                .send_message("C123", None, true, "bad broadcast", true)
                .await,
            Err(Error::InvalidInput {
                field: "broadcast",
                ..
            })
        ));
        for (thread_ts, broadcast) in [(None, false), (Some("6000.000001"), true)] {
            assert!(matches!(
                service
                    .send_message(
                        "C123",
                        thread_ts,
                        broadcast,
                        "<https://example.com/write|Unsupported write>",
                        true,
                    )
                    .await,
                Err(Error::InvalidInput {
                    field: "markdown",
                    ..
                })
            ));
        }

        let calls = post_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].channel, "C123");
        assert_eq!(calls[0].thread_ts, None);
        assert!(!calls[0].broadcast);
        assert_eq!(calls[0].text, "root <@U456>");
        assert_eq!(calls[0].blocks[0]["type"], "rich_text");
        assert_eq!(calls[1].thread_ts.as_deref(), Some("6000.000001"));
        assert!(calls[1].broadcast);
    }

    #[tokio::test]
    async fn root_and_reply_publication_preserve_resolved_user_elements() {
        let users = RawUsersPage {
            members: vec![raw_user("UALICE", "alice", "Alice Example")],
            ..RawUsersPage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([users.clone(), users]));
        let user_calls = api.user_calls.clone();
        let post_calls = api.post_calls.clone();
        let service = service(api);
        let markdown = "Hello [@Alice](slack-user:alice).";

        let root = service
            .send_message("C123", None, false, markdown, true)
            .await
            .unwrap();
        let reply = service
            .send_message("C123", Some("6000.000001"), false, markdown, true)
            .await
            .unwrap();

        assert_eq!(user_calls.lock().unwrap().len(), 2);
        let calls = post_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        for call in calls.iter() {
            assert_eq!(call.text, "Hello @Alice.");
            assert_eq!(
                call.blocks[0]["elements"][0]["elements"][1],
                json!({"type": "user", "user_id": "UALICE"})
            );
        }
        for sent in [&root, &reply] {
            assert_eq!(
                sent.message.mention_resolution,
                MentionResolution::NotAttempted
            );
            assert_eq!(sent.message.mentions.len(), 1);
            assert_eq!(sent.message.mentions[0].id, "UALICE");
            assert_eq!(
                sent.message.blocks.as_ref().unwrap()[0]["elements"][0]["elements"][1],
                json!({"type": "user", "user_id": "UALICE"})
            );
        }
        assert_eq!(root.message.thread_ts, None);
        assert_eq!(reply.message.thread_ts.as_deref(), Some("6000.000001"));
    }

    #[tokio::test]
    async fn unresolved_outbound_mentions_fail_before_conversation_or_publication() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        let user_calls = api.user_calls.clone();
        let conversation_calls = api.conversation_calls.clone();
        let post_calls = api.post_calls.clone();
        let error = service(api)
            .send_message(
                "unresolved-conversation",
                None,
                false,
                "Hello [@Missing](slack-user:missing).",
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, Error::OutboundMention { .. }));
        assert_single_user_directory_call(&user_calls);
        assert!(conversation_calls.lock().unwrap().is_empty());
        assert!(post_calls.lock().unwrap().is_empty());

        let mut file_api = fake_api();
        file_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        let file_user_calls = file_api.user_calls.clone();
        let file_conversation_calls = file_api.conversation_calls.clone();
        let file_upload_calls = file_api.upload_calls.clone();
        let file_draft_calls = file_api.draft_calls.clone();
        let fixture = UploadFixture::new(b"synthetic");
        let file_error = service(file_api)
            .create_file_draft(
                FileDraftCreateRequest {
                    conversation: "unresolved-conversation",
                    thread_ts: None,
                    broadcast: false,
                    markdown: "Hello [@Missing](slack-user:missing).",
                    title: None,
                    alt_text: None,
                    confirmed: true,
                },
                fixture.source(),
            )
            .await
            .unwrap_err();
        assert!(matches!(file_error, Error::OutboundMention { .. }));
        assert_single_user_directory_call(&file_user_calls);
        assert!(file_conversation_calls.lock().unwrap().is_empty());
        assert!(file_upload_calls.lock().unwrap().is_empty());
        assert!(file_draft_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn send_rejects_malformed_acknowledgements_and_preserves_slack_errors() {
        let mut malformed = fake_api();
        malformed.post_response = Some(RawPostMessageResponse {
            channel: "C-other".into(),
            ts: "7000.000001".into(),
            message: RawMessage {
                ts: "7000.000001".into(),
                text: "synthetic".into(),
                ..RawMessage::default()
            },
        });
        let post_calls = malformed.post_calls.clone();
        let error = service(malformed)
            .send_message("C123", None, false, "synthetic", true)
            .await
            .unwrap_err();
        let Error::PublicationUncertain { client_msg_id } = error else {
            panic!("malformed acknowledgement must be publication-uncertain");
        };
        assert_eq!(post_calls.lock().unwrap()[0].client_msg_id, client_msg_id);

        let mut rejected = fake_api();
        rejected.post_error = Some("restricted_action".into());
        assert!(matches!(
            service(rejected)
                .send_message("C123", None, false, "synthetic", true)
                .await,
            Err(Error::SlackApi {
                method: "chat.postMessage",
                code
            }) if code == "restricted_action"
        ));
    }

    #[tokio::test]
    async fn send_draft_reports_post_success_cleanup_failure_without_reposting() {
        let draft = raw_draft("DR-send", "8000.5", "C123", "draft body");
        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        api.drafts_page = active_drafts(vec![draft]);
        api.draft_delete_error = true;
        let draft_calls = api.draft_calls.clone();
        let post_calls = api.post_calls.clone();
        let mut service = service(api);
        service.draft_reconciliation_delays_ms = &[0, 0, 0, 0, 0, 0];

        assert!(matches!(
            service.send_draft("DR-send", false).await,
            Err(Error::ConfirmationRequired {
                action: "draft publication"
            })
        ));
        let report = service.send_draft("DR-send", true).await.unwrap();
        assert_eq!(report.draft_id, "DR-send");
        assert!(!report.draft_deleted);
        let warning = report.cleanup_warning.expect("cleanup warning");
        assert_eq!(warning.draft_id, "DR-send");
        assert_eq!(warning.last_updated_ts, "8000.5");
        assert!(warning.reason.contains("draft_conflict"));
        assert_eq!(post_calls.lock().unwrap().len(), 1);
        let calls = draft_calls.lock().unwrap();
        assert_eq!(
            &calls[..2],
            &[
                DraftCall::Info {
                    draft_id: "DR-send".into()
                },
                DraftCall::Delete {
                    draft_id: "DR-send".into(),
                    last_updated_ts: "8000500".into(),
                    skip_file_deletion: false,
                }
            ]
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            6
        );
    }

    #[tokio::test]
    async fn send_draft_reconciles_ambiguous_cleanup_without_reposting() {
        let mut absent = fake_api();
        absent.draft_info = RawDraftResponse {
            draft: raw_draft("DR-send", "8050", "C123", "draft body"),
        };
        absent.draft_delete_ambiguous = true;
        absent.drafts_page = active_drafts(vec![]);
        let absent_posts = absent.post_calls.clone();
        let absent_calls = absent.draft_calls.clone();
        let report = service(absent).send_draft("DR-send", true).await.unwrap();
        assert!(report.draft_deleted);
        assert!(report.cleanup_warning.is_none());
        assert_eq!(absent_posts.lock().unwrap().len(), 1);
        assert_eq!(
            absent_calls
                .lock()
                .unwrap()
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            1
        );

        let draft = raw_draft("DR-send", "8051", "C123", "draft body");
        let mut unresolved = fake_api();
        unresolved.draft_info = RawDraftResponse {
            draft: draft.clone(),
        };
        unresolved.drafts_page = active_drafts(vec![draft]);
        unresolved.draft_delete_ambiguous = true;
        let unresolved_posts = unresolved.post_calls.clone();
        let unresolved_calls = unresolved.draft_calls.clone();
        let mut service = service(unresolved);
        service.draft_reconciliation_delays_ms = &[0, 0, 0, 0, 0, 0];
        let report = service.send_draft("DR-send", true).await.unwrap();
        assert!(!report.draft_deleted);
        assert!(
            report
                .cleanup_warning
                .expect("cleanup warning")
                .reason
                .contains("outcome is unknown")
        );
        assert_eq!(unresolved_posts.lock().unwrap().len(), 1);
        let calls = unresolved_calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::Delete { .. }))
                .count(),
            1
        );
        assert_eq!(
            calls
                .iter()
                .filter(|call| matches!(call, DraftCall::List { .. }))
                .count(),
            6
        );
    }

    #[tokio::test]
    async fn send_draft_exposes_client_id_and_does_not_delete_on_ambiguous_post() {
        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: raw_draft("DR-unsent", "8100", "C123", "draft body"),
        };
        api.post_response = Some(RawPostMessageResponse {
            channel: "C123".into(),
            ts: "7000.000001".into(),
            message: RawMessage {
                ts: "different.000001".into(),
                text: "draft body".into(),
                ..RawMessage::default()
            },
        });
        let draft_calls = api.draft_calls.clone();
        let post_calls = api.post_calls.clone();
        let service = service(api);

        let error = service.send_draft("DR-unsent", true).await.unwrap_err();
        let rendered_error = error.to_string();
        let Error::PublicationUncertain { client_msg_id } = error else {
            panic!("ambiguous draft post must expose a publication-uncertain result");
        };
        assert_eq!(post_calls.lock().unwrap()[0].client_msg_id, client_msg_id);
        assert!(rendered_error.contains("do not retry automatically"));
        assert_eq!(
            draft_calls.lock().unwrap().as_slice(),
            [DraftCall::Info {
                draft_id: "DR-unsent".into()
            }]
        );
    }

    #[tokio::test]
    async fn send_draft_derives_block_only_fallback_and_rejects_non_rich_blocks() {
        let mut block_only = fake_api();
        let mut draft = raw_draft("DR-block-only", "8200", "C123", "derived text");
        draft.text.clear();
        block_only.draft_info = RawDraftResponse { draft };
        let post_calls = block_only.post_calls.clone();
        let report = service(block_only)
            .send_draft("DR-block-only", true)
            .await
            .unwrap();
        assert!(report.draft_deleted);
        assert_eq!(post_calls.lock().unwrap()[0].text, "derived text");

        let mut non_rich = fake_api();
        let mut draft = raw_draft("DR-block-kit", "8300", "C123", "not allowed");
        draft.blocks = Some(vec![json!({
            "type": "section",
            "text": {"type": "mrkdwn", "text": "not allowed"}
        })]);
        non_rich.draft_info = RawDraftResponse { draft };
        let post_calls = non_rich.post_calls.clone();
        let draft_calls = non_rich.draft_calls.clone();
        assert!(matches!(
            service(non_rich).send_draft("DR-block-kit", true).await,
            Err(Error::InvalidInput { field: "draft", .. })
        ));
        assert!(post_calls.lock().unwrap().is_empty());
        assert_eq!(
            draft_calls.lock().unwrap().as_slice(),
            [DraftCall::Info {
                draft_id: "DR-block-kit".into()
            }]
        );
    }

    #[tokio::test]
    async fn normalizes_only_explicit_slack_unreads_across_kinds() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![entry("CREAD", false, 9), entry("CUNREAD", true, 1)],
            ims: vec![entry("DUNREAD", true, 3)],
            mpims: vec![entry("GUNREAD", true, 0)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 2,
                unread_count_by_channel: BTreeMap::from([("CUNREAD".into(), 4)]),
            },
        };
        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            report
                .conversations
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["DUNREAD", "CUNREAD", "GUNREAD"]
        );
        assert_eq!(
            report.conversations[0].kind,
            ConversationKind::DirectMessage
        );
        assert!(report.threads.has_unreads);
        assert_eq!(report.threads.mention_count, 2);
    }

    #[tokio::test]
    async fn rejects_duplicate_unread_ids_within_or_across_count_kinds() {
        let mut same_list = fake_api();
        same_list.counts.channels = vec![entry("CGENERAL", true, 1), entry("CGENERAL", true, 1)];
        assert!(matches!(
            service(same_list).unreads().await,
            Err(Error::InvalidResponse {
                method: "client.counts"
            })
        ));

        let mut cross_list = fake_api();
        cross_list.counts.channels = vec![entry("GTEAM", true, 1)];
        cross_list.counts.mpims = vec![entry("GTEAM", true, 1)];
        assert!(matches!(
            service(cross_list).unreads().await,
            Err(Error::InvalidResponse {
                method: "client.counts"
            })
        ));
    }

    #[tokio::test]
    async fn unread_json_schema_is_stable_and_typed() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![RawUnread {
                id: "CNULL".into(),
                has_unreads: true,
                mention_count: 0,
                last_read: None,
                latest: None,
            }],
            ims: vec![entry("DONE", true, 1)],
            mpims: vec![entry("GONE", true, 1)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 3,
                unread_count_by_channel: BTreeMap::from([("CNULL".into(), 2), ("DONE".into(), 1)]),
            },
        };
        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "team_id": "T000TEST",
                "conversations": [
                    {
                        "id": "DONE",
                        "kind": "direct_message",
                        "name": null,
                        "display_name": null,
                        "name_resolution": "inaccessible",
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "GONE",
                        "kind": "group_direct_message",
                        "name": null,
                        "display_name": null,
                        "name_resolution": "inaccessible",
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "CNULL",
                        "kind": "channel",
                        "name": null,
                        "display_name": null,
                        "name_resolution": "inaccessible",
                        "has_unreads": true,
                        "mention_count": 0,
                        "last_read": null,
                        "latest": null
                    }
                ],
                "threads": {
                    "has_unreads": true,
                    "mention_count": 3,
                    "unread_count_by_channel": {
                        "CNULL": 2,
                        "DONE": 1
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn unread_names_resolve_all_kinds_with_one_bounded_shared_scan() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![entry("CGENERAL", true, 2)],
            ims: vec![entry("DALI", true, 4), entry("DBOB", true, 3)],
            mpims: vec![entry("GTEAM", true, 1)],
            threads: RawThreadCounts::default(),
        };
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CGENERAL", "general"),
                RawConversation {
                    id: "DALI".into(),
                    is_im: true,
                    user: Some("WALI".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    id: "DBOB".into(),
                    is_im: true,
                    user: Some("WBOB".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    is_mpim: true,
                    ..raw_conversation("GTEAM", "mpdm-alice--bob-1")
                },
            ],
            response_metadata: RawResponseMetadata {
                next_cursor: "unused-because-all-counted".into(),
            },
        }]));
        let mut display_only = raw_user("WBOB", "bad\nname", "Bob Example");
        display_only.real_name = None;
        display_only.profile.real_name = None;
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("WALI", "alice", "Alice Example"), display_only],
            response_metadata: RawResponseMetadata {
                next_cursor: "unused-because-all-users-found".into(),
            },
        }]));
        let count_calls = api.count_calls.clone();
        let conversation_calls = api.conversation_calls.clone();
        let user_calls = api.user_calls.clone();

        let report = service(api).unreads().await.unwrap();

        assert_eq!(*count_calls.lock().unwrap(), 1);
        assert_eq!(conversation_calls.lock().unwrap().len(), 1);
        assert_single_user_directory_call(&user_calls);
        assert_eq!(
            (
                unread_entry(&report, "CGENERAL").name.as_deref(),
                unread_entry(&report, "CGENERAL").display_name.as_deref(),
                unread_entry(&report, "CGENERAL").name_resolution,
            ),
            (
                Some("general"),
                Some("general"),
                ConversationNameResolution::Resolved,
            )
        );
        assert_eq!(
            (
                unread_entry(&report, "DALI").name.as_deref(),
                unread_entry(&report, "DALI").display_name.as_deref(),
                unread_entry(&report, "DALI").name_resolution,
            ),
            (
                Some("alice"),
                Some("Alice Example"),
                ConversationNameResolution::Resolved,
            )
        );
        assert_eq!(
            (
                unread_entry(&report, "DBOB").name.as_deref(),
                unread_entry(&report, "DBOB").display_name.as_deref(),
                unread_entry(&report, "DBOB").name_resolution,
            ),
            (
                None,
                Some("Bob Example"),
                ConversationNameResolution::Resolved,
            )
        );
        assert_eq!(
            (
                unread_entry(&report, "GTEAM").name.as_deref(),
                unread_entry(&report, "GTEAM").display_name.as_deref(),
                unread_entry(&report, "GTEAM").name_resolution,
            ),
            (
                Some("alice, bob"),
                Some("alice, bob"),
                ConversationNameResolution::Resolved,
            )
        );
    }

    #[tokio::test]
    async fn unread_dm_name_resolution_rejects_target_user_conflicts() {
        let mut same_page_api = fake_api();
        same_page_api.counts.ims = vec![entry("DCONFLICT", true, 1)];
        same_page_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "DCONFLICT".into(),
                is_im: true,
                user: Some("WCONFLICT".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        same_page_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("WCONFLICT", "first", "First User"),
                raw_user("WCONFLICT", "second", "Second User"),
            ],
            ..RawUsersPage::default()
        }]));
        let same_page = service(same_page_api).unreads().await.unwrap();
        assert_eq!(
            unread_entry(&same_page, "DCONFLICT").name_resolution,
            ConversationNameResolution::Unavailable
        );

        let mut cross_page_api = fake_api();
        cross_page_api.counts.ims = vec![entry("DCONFLICT", true, 2), entry("DOTHER", true, 1)];
        cross_page_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                RawConversation {
                    id: "DCONFLICT".into(),
                    is_im: true,
                    user: Some("WCONFLICT".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    id: "DOTHER".into(),
                    is_im: true,
                    user: Some("WOTHER".into()),
                    ..RawConversation::default()
                },
            ],
            ..RawConversationsPage::default()
        }]));
        cross_page_api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage {
                members: vec![raw_user("WCONFLICT", "first", "First User")],
                response_metadata: RawResponseMetadata {
                    next_cursor: "users-2".into(),
                },
            },
            RawUsersPage {
                members: vec![
                    raw_user("WCONFLICT", "second", "Second User"),
                    raw_user("WOTHER", "other", "Other User"),
                ],
                response_metadata: RawResponseMetadata {
                    next_cursor: "unused-because-all-users-accounted".into(),
                },
            },
        ]));
        let cross_page_calls = cross_page_api.user_calls.clone();
        let cross_page = service(cross_page_api).unreads().await.unwrap();
        assert_eq!(cross_page_calls.lock().unwrap().len(), 2);
        assert_eq!(
            unread_entry(&cross_page, "DCONFLICT").name_resolution,
            ConversationNameResolution::Unavailable
        );
        assert_eq!(
            (
                unread_entry(&cross_page, "DOTHER").name.as_deref(),
                unread_entry(&cross_page, "DOTHER").name_resolution,
            ),
            (Some("other"), ConversationNameResolution::Resolved,)
        );
    }

    #[tokio::test]
    async fn unread_name_resolution_preserves_valid_matches_and_marks_unsafe_names() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CGENERAL", true, 2), entry("CUNNAMED", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CGENERAL", "general"),
                raw_conversation("CUNNAMED", "bad\nname"),
            ],
            ..RawConversationsPage::default()
        }]));

        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            unread_entry(&report, "CGENERAL").name_resolution,
            ConversationNameResolution::Resolved
        );
        let unnamed = unread_entry(&report, "CUNNAMED");
        assert_eq!(unnamed.name, None);
        assert_eq!(unnamed.display_name, None);
        assert_eq!(unnamed.name_resolution, ConversationNameResolution::Unnamed);
    }

    #[tokio::test]
    async fn unread_name_resolution_reports_conversation_scan_bounds() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CMISSING", true, 1)];
        api.conversation_pages = Mutex::new(
            (0..MAX_CONVERSATION_PAGES)
                .map(|page| RawConversationsPage {
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("conversation-page-{page}"),
                    },
                    ..RawConversationsPage::default()
                })
                .collect(),
        );
        let conversation_calls = api.conversation_calls.clone();

        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            unread_entry(&report, "CMISSING").name_resolution,
            ConversationNameResolution::Incomplete
        );
        assert_eq!(
            conversation_calls.lock().unwrap().len(),
            MAX_CONVERSATION_PAGES
        );
    }

    #[tokio::test]
    async fn unread_name_resolution_retains_matches_when_discovery_becomes_unavailable() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CKNOWN", true, 2), entry("CMISSING", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([
            RawConversationsPage {
                channels: vec![raw_conversation("CKNOWN", "known")],
                response_metadata: RawResponseMetadata {
                    next_cursor: "page-2".into(),
                },
            },
            RawConversationsPage {
                response_metadata: RawResponseMetadata {
                    next_cursor: "page-2".into(),
                },
                ..RawConversationsPage::default()
            },
        ]));

        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            unread_entry(&report, "CKNOWN").name_resolution,
            ConversationNameResolution::Resolved
        );
        assert_eq!(
            unread_entry(&report, "CMISSING").name_resolution,
            ConversationNameResolution::Unavailable
        );
    }

    #[tokio::test]
    async fn unread_name_resolution_degrades_conflicting_and_malformed_matches() {
        let mut api = fake_api();
        api.counts.channels = vec![
            entry("CCONFLICT", true, 3),
            entry("CMISMATCH", true, 2),
            entry("CBROKEN", true, 1),
        ];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CCONFLICT", "first"),
                raw_conversation("CCONFLICT", "second"),
                RawConversation {
                    id: "CMISMATCH".into(),
                    is_im: true,
                    user: Some("WALI".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    id: "CBROKEN".into(),
                    is_im: true,
                    is_mpim: true,
                    ..RawConversation::default()
                },
            ],
            ..RawConversationsPage::default()
        }]));

        let report = service(api).unreads().await.unwrap();

        assert_eq!(report.conversations.len(), 3);
        for id in ["CCONFLICT", "CMISMATCH", "CBROKEN"] {
            assert_eq!(
                unread_entry(&report, id).name_resolution,
                ConversationNameResolution::Unavailable,
                "{id} should retain its authoritative count identity"
            );
        }
    }

    #[tokio::test]
    async fn unread_dm_name_resolution_distinguishes_complete_bounded_and_interrupted_users() {
        let mut complete_api = fake_api();
        complete_api.counts.ims = vec![entry("DMISSING", true, 1)];
        complete_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "DMISSING".into(),
                is_im: true,
                user: Some("WMISSING".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        complete_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        let complete = service(complete_api).unreads().await.unwrap();
        assert_eq!(
            unread_entry(&complete, "DMISSING").name_resolution,
            ConversationNameResolution::Unnamed
        );

        let mut bounded_api = fake_api();
        bounded_api.counts.ims = vec![entry("DKNOWN", true, 2), entry("DMISSING", true, 1)];
        bounded_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                RawConversation {
                    id: "DKNOWN".into(),
                    is_im: true,
                    user: Some("WKNOWN".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    id: "DMISSING".into(),
                    is_im: true,
                    user: Some("WMISSING".into()),
                    ..RawConversation::default()
                },
            ],
            ..RawConversationsPage::default()
        }]));
        bounded_api.user_pages = Mutex::new(
            (0..MAX_USER_PAGES)
                .map(|page| RawUsersPage {
                    members: (page == 0)
                        .then(|| raw_user("WKNOWN", "known", "Known User"))
                        .into_iter()
                        .collect(),
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("users-{page}"),
                    },
                })
                .collect(),
        );
        let bounded_user_calls = bounded_api.user_calls.clone();
        let bounded = service(bounded_api).unreads().await.unwrap();
        assert_eq!(
            unread_entry(&bounded, "DKNOWN").name_resolution,
            ConversationNameResolution::Resolved
        );
        assert_eq!(
            unread_entry(&bounded, "DMISSING").name_resolution,
            ConversationNameResolution::Incomplete
        );
        assert_eq!(bounded_user_calls.lock().unwrap().len(), MAX_USER_PAGES);

        let mut interrupted_api = fake_api();
        interrupted_api.counts.ims = vec![entry("DKNOWN", true, 2), entry("DMISSING", true, 1)];
        interrupted_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                RawConversation {
                    id: "DKNOWN".into(),
                    is_im: true,
                    user: Some("WKNOWN".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    id: "DMISSING".into(),
                    is_im: true,
                    user: Some("WMISSING".into()),
                    ..RawConversation::default()
                },
            ],
            ..RawConversationsPage::default()
        }]));
        interrupted_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("WKNOWN", "known", "Known User")],
            response_metadata: RawResponseMetadata {
                next_cursor: "users-2".into(),
            },
        }]));
        interrupted_api.user_list_error_after = Some(1);
        let interrupted = service(interrupted_api).unreads().await.unwrap();
        assert_eq!(
            unread_entry(&interrupted, "DKNOWN").name_resolution,
            ConversationNameResolution::Resolved
        );
        assert_eq!(
            unread_entry(&interrupted, "DMISSING").name_resolution,
            ConversationNameResolution::Unavailable
        );
    }

    #[test]
    fn group_dm_names_decode_only_the_bounded_known_shape() {
        assert_eq!(
            readable_group_dm_name("mpdm-alice--bob-1").as_deref(),
            Some("alice, bob")
        );
        assert_eq!(
            readable_group_dm_name("project-chat").as_deref(),
            Some("project-chat")
        );
        for unsafe_name in [
            "mpdm-alice-1",
            "mpdm-alice--bob-x",
            "mpdm-alice--bad\nname-1",
            "mpdm-alice--bob-12345678901",
        ] {
            assert_eq!(
                readable_group_dm_name(unsafe_name),
                None,
                "{unsafe_name:?} should never be displayed"
            );
        }
    }

    #[tokio::test]
    async fn inbox_selects_sorted_unreads_enriches_names_and_bounds_history() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![entry("CGENERAL", true, 1)],
            ims: vec![entry("DALI", true, 3)],
            mpims: vec![entry("GTEAM", true, 0)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 2,
                unread_count_by_channel: BTreeMap::from([("CGENERAL".into(), 2)]),
            },
        };
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CGENERAL", "general"),
                RawConversation {
                    id: "DALI".into(),
                    is_im: true,
                    user: Some("WALI".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    is_mpim: true,
                    ..raw_conversation("GTEAM", "mpdm-alice--bob-1")
                },
            ],
            ..RawConversationsPage::default()
        }]));
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("WALI", "alice", "Alice Example"),
                raw_user("U123", "carol", "Carol Example"),
            ],
            ..RawUsersPage::default()
        }]));
        api.history.messages = vec![raw_message("100.000001", "recent <@WALI>")];
        let count_calls = api.count_calls.clone();
        let conversation_calls = api.conversation_calls.clone();
        let history_calls = api.history_calls.clone();
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(2, 7).await.unwrap();

        assert_eq!(report.total_unread_conversations, 3);
        assert!(report.has_more_conversations);
        assert_eq!(
            report.truncation_reason,
            Some(InboxTruncationReason::ConversationLimit)
        );
        assert_eq!(
            report
                .conversations
                .iter()
                .map(|entry| entry.conversation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["DALI", "CGENERAL"]
        );
        assert_eq!(report.conversations[0].conversation.name, "alice");
        assert_eq!(
            report.conversations[0].conversation.display_name,
            "Alice Example"
        );
        assert!(!report.conversations[0].conversation.name_is_fallback);
        assert!(report.conversations[0].conversation.metadata_is_complete);
        assert_eq!(report.conversations[1].conversation.name, "general");
        assert_eq!(
            (
                report.conversations[0].unread.name.as_deref(),
                report.conversations[0].unread.display_name.as_deref(),
                report.conversations[0].unread.name_resolution,
            ),
            (
                Some("alice"),
                Some("Alice Example"),
                ConversationNameResolution::Resolved,
            )
        );
        assert_eq!(
            (
                report.conversations[1].unread.name.as_deref(),
                report.conversations[1].unread.display_name.as_deref(),
                report.conversations[1].unread.name_resolution,
            ),
            (
                Some("general"),
                Some("general"),
                ConversationNameResolution::Resolved,
            )
        );
        assert_eq!(report.conversations[0].messages.messages.len(), 1);
        assert!(report.conversations.iter().all(|entry| {
            let message = &entry.messages.messages[0];
            message.author_name.as_deref() == Some("carol")
                && message.author_display_name.as_deref() == Some("Carol Example")
                && message.author_resolution == AuthorResolution::Directory
                && message.rendered_text == "recent @alice"
                && message.mention_resolution == MentionResolution::Complete
        }));
        assert_eq!(*count_calls.lock().unwrap(), 1);
        assert_eq!(conversation_calls.lock().unwrap().len(), 1);
        assert_single_user_directory_call(&user_calls);
        assert!(report.threads.has_unreads);
        assert_eq!(report.threads.mention_count, 2);
        assert_eq!(
            *history_calls.lock().unwrap(),
            vec![
                HistoryCall {
                    channel: "DALI".into(),
                    cursor: None,
                    limit: 7,
                },
                HistoryCall {
                    channel: "CGENERAL".into(),
                    cursor: None,
                    limit: 7,
                },
            ]
        );
    }

    #[tokio::test]
    async fn inbox_never_uses_conflicting_user_metadata_for_dm_or_message_names() {
        let mut api = fake_api();
        api.counts.ims = vec![entry("DCONFLICT", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "DCONFLICT".into(),
                is_im: true,
                user: Some("WCONFLICT".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("WCONFLICT", "first", "First User"),
                raw_user("WCONFLICT", "second", "Second User"),
            ],
            ..RawUsersPage::default()
        }]));
        let mut message = raw_message("100.000001", "synthetic <@WCONFLICT>");
        message.user = Some("WCONFLICT".into());
        api.history.messages = vec![message];
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(1, 1).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        let entry = &report.conversations[0];
        assert!(entry.conversation.name_is_fallback);
        assert_eq!(entry.unread.name, None);
        assert_eq!(entry.unread.display_name, None);
        assert_eq!(
            entry.unread.name_resolution,
            ConversationNameResolution::Unavailable
        );
        assert_eq!(entry.messages.messages[0].author_name, None);
        assert_eq!(
            entry.messages.messages[0].author_resolution,
            AuthorResolution::Unavailable
        );
        assert_eq!(
            entry.messages.messages[0].rendered_text,
            "synthetic <@WCONFLICT>"
        );
        assert_eq!(
            entry.messages.messages[0].mention_resolution,
            MentionResolution::Unavailable
        );
    }

    #[tokio::test]
    async fn inbox_resolves_complete_misses_without_losing_json_identity_metadata() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CGENERAL", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CGENERAL", "general")],
            ..RawConversationsPage::default()
        }]));
        api.history.messages = vec![raw_message("100.000001", "unknown author")];
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(1, 1).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        let message = &report.conversations[0].messages.messages[0];
        assert_eq!(message.author_id.as_deref(), Some("U123"));
        assert_eq!(message.author_name, None);
        assert_eq!(message.author_display_name, None);
        assert_eq!(message.author_resolution, AuthorResolution::Unresolved);
        let json = serde_json::to_value(message).unwrap();
        assert_eq!(json["author_id"], "U123");
        assert_eq!(json["author_name"], serde_json::Value::Null);
        assert_eq!(json["author_display_name"], serde_json::Value::Null);
        assert_eq!(json["author_resolution"], "unresolved");
    }

    #[tokio::test]
    async fn inbox_reuses_valid_users_when_the_shared_directory_is_interrupted() {
        let mut api = fake_api();
        api.counts.ims = vec![entry("DALI", true, 3)];
        api.counts.channels = vec![entry("CGENERAL", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                RawConversation {
                    id: "DALI".into(),
                    is_im: true,
                    user: Some("WALI".into()),
                    ..RawConversation::default()
                },
                raw_conversation("CGENERAL", "general"),
            ],
            ..RawConversationsPage::default()
        }]));
        let known = raw_message("100.000001", "known");
        let mut unavailable = raw_message("100.000002", "unavailable");
        unavailable.user = Some("U999".into());
        api.history.messages = vec![known, unavailable];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("WALI", "alice", "Alice Example"),
                raw_user("U123", "carol", "Carol Example"),
            ],
            response_metadata: RawResponseMetadata {
                next_cursor: "users-2".into(),
            },
        }]));
        api.user_list_error_after = Some(1);
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(2, 2).await.unwrap();

        assert_eq!(
            user_calls.lock().unwrap().as_slice(),
            &[
                UserCall {
                    cursor: None,
                    limit: USERS_PAGE_SIZE,
                },
                UserCall {
                    cursor: Some("users-2".into()),
                    limit: USERS_PAGE_SIZE,
                },
            ]
        );
        assert_eq!(
            report
                .conversations
                .iter()
                .map(|entry| entry.conversation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["DALI", "CGENERAL"]
        );
        assert_eq!(report.conversations[0].conversation.name, "alice");
        assert_eq!(
            report.conversations[0].conversation.display_name,
            "Alice Example"
        );
        for entry in &report.conversations {
            let messages = &entry.messages.messages;
            assert_eq!(messages[0].author_name.as_deref(), Some("carol"));
            assert_eq!(
                messages[0].author_display_name.as_deref(),
                Some("Carol Example")
            );
            assert_eq!(messages[0].author_resolution, AuthorResolution::Directory);
            assert_eq!(messages[1].author_name, None);
            assert_eq!(messages[1].author_resolution, AuthorResolution::Unavailable);
        }
    }

    #[tokio::test]
    async fn inbox_marks_unscanned_authors_incomplete_at_the_shared_directory_bound() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CGENERAL", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CGENERAL", "general")],
            ..RawConversationsPage::default()
        }]));
        let known = raw_message("100.000001", "known");
        let mut incomplete = raw_message("100.000002", "incomplete");
        incomplete.user = Some("U999".into());
        api.history.messages = vec![known, incomplete];
        api.user_pages = Mutex::new(
            (0..MAX_USER_PAGES)
                .map(|page| RawUsersPage {
                    members: if page == 0 {
                        vec![raw_user("U123", "alice", "Alice Example")]
                    } else {
                        vec![]
                    },
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("users-{}", page + 1),
                    },
                })
                .collect(),
        );
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(1, 2).await.unwrap();

        assert_eq!(user_calls.lock().unwrap().len(), MAX_USER_PAGES);
        let messages = &report.conversations[0].messages.messages;
        assert_eq!(messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(messages[0].author_resolution, AuthorResolution::Directory);
        assert_eq!(messages[1].author_name, None);
        assert_eq!(messages[1].author_resolution, AuthorResolution::Incomplete);
    }

    #[tokio::test]
    async fn inbox_skips_the_shared_directory_for_supplied_and_authorless_messages() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CGENERAL", true, 2), entry("CRANDOM", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CGENERAL", "general"),
                raw_conversation("CRANDOM", "random"),
            ],
            ..RawConversationsPage::default()
        }]));
        let mut supplied = raw_message("100.000001", "bot");
        supplied.user = None;
        supplied.bot_id = Some("B123".into());
        supplied.username = Some("build-bot".into());
        let authorless = RawMessage {
            ts: "100.000002".into(),
            text: "system event".into(),
            ..RawMessage::default()
        };
        api.history.messages = vec![supplied, authorless];
        let user_calls = api.user_calls.clone();

        let report = service(api).inbox(2, 2).await.unwrap();

        assert!(user_calls.lock().unwrap().is_empty());
        assert_eq!(report.conversations.len(), 2);
        for entry in report.conversations {
            assert_eq!(
                entry.messages.messages[0].author_resolution,
                AuthorResolution::Provided
            );
            assert_eq!(
                entry.messages.messages[0].author_name.as_deref(),
                Some("build-bot")
            );
            assert_eq!(
                entry.messages.messages[1].author_resolution,
                AuthorResolution::Unknown
            );
            assert_eq!(entry.messages.messages[1].author_id, None);
        }
    }

    #[tokio::test]
    async fn inbox_accepts_a_c_prefixed_mpim_from_counts_and_discovery() {
        let mut api = fake_api();
        api.counts.mpims = vec![entry("CTEAM", true, 2)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                is_mpim: true,
                ..raw_conversation("CTEAM", "mpdm-alice--bob-1")
            }],
            ..RawConversationsPage::default()
        }]));
        api.history.messages = vec![raw_message("100.000001", "recent")];
        let history_calls = api.history_calls.clone();

        let report = service(api).inbox(1, 3).await.unwrap();

        assert_eq!(report.total_unread_conversations, 1);
        assert!(!report.has_more_conversations);
        assert_eq!(report.truncation_reason, None);
        assert_eq!(report.conversations.len(), 1);
        assert_eq!(
            report.conversations[0].conversation.kind,
            ConversationKind::GroupDirectMessage
        );
        assert_eq!(report.conversations[0].messages.messages.len(), 1);
        assert_eq!(
            *history_calls.lock().unwrap(),
            vec![HistoryCall {
                channel: "CTEAM".into(),
                cursor: None,
                limit: 3,
            }]
        );
    }

    #[tokio::test]
    async fn empty_inbox_skips_discovery_and_history_but_preserves_thread_unreads() {
        let mut api = fake_api();
        api.counts.threads = RawThreadCounts {
            has_unreads: true,
            mention_count: 4,
            unread_count_by_channel: BTreeMap::from([("CGENERAL".into(), 4)]),
        };
        let conversation_calls = api.conversation_calls.clone();
        let history_calls = api.history_calls.clone();

        let report = service(api).inbox(10, 20).await.unwrap();

        assert!(report.conversations.is_empty());
        assert_eq!(report.total_unread_conversations, 0);
        assert!(!report.has_more_conversations);
        assert!(report.threads.has_unreads);
        assert_eq!(report.threads.mention_count, 4);
        assert_eq!(report.truncation_reason, None);
        assert!(conversation_calls.lock().unwrap().is_empty());
        assert!(history_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn inbox_caps_the_complete_report_and_stops_after_the_first_oversized_history() {
        let make_api = || {
            let mut api = fake_api();
            api.counts.channels = vec![
                entry("CAAA", true, 3),
                entry("CBBB", true, 2),
                entry("CCCC", true, 1),
            ];
            api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
                channels: vec![
                    raw_conversation("CAAA", "alpha"),
                    raw_conversation("CBBB", "beta"),
                    raw_conversation("CCCC", "gamma"),
                ],
                ..RawConversationsPage::default()
            }]));
            api.history.messages = vec![raw_message(
                "100.000001",
                "a bounded synthetic message whose serialized size is intentionally nontrivial",
            )];
            api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
                members: vec![raw_user("U123", "alice", "Alice Example")],
                ..RawUsersPage::default()
            }]));
            api
        };

        let full_api = make_api();
        let full_user_calls = full_api.user_calls.clone();
        let full_report = service(full_api).inbox(3, 1).await.unwrap();
        assert_single_user_directory_call(&full_user_calls);
        assert!(full_report.conversations.iter().all(|entry| {
            entry.messages.messages[0].author_resolution == AuthorResolution::Directory
        }));
        assert!(full_report.conversations.iter().all(|entry| {
            entry.messages.messages[0]
                .permalink
                .as_deref()
                .is_some_and(|link| {
                    link.starts_with("https://example.slack.com/archives/")
                        && !link.contains("127.0.0.1")
                })
        }));
        let mut one_conversation_report = full_report.clone();
        one_conversation_report.conversations.truncate(1);
        one_conversation_report.has_more_conversations = true;
        one_conversation_report.truncation_reason = Some(InboxTruncationReason::ByteLimit);
        let byte_limit = serde_json::to_vec_pretty(&one_conversation_report)
            .unwrap()
            .len();

        let api = make_api();
        let history_calls = api.history_calls.clone();
        let mut bounded_service = service(api);
        bounded_service.max_response_bytes = byte_limit;
        let report = bounded_service.inbox(3, 1).await.unwrap();

        assert_eq!(report.conversations.len(), 1);
        assert!(report.has_more_conversations);
        assert_eq!(
            report.truncation_reason,
            Some(InboxTruncationReason::ByteLimit)
        );
        assert!(serde_json::to_vec_pretty(&report).unwrap().len() <= byte_limit);
        assert_eq!(
            history_calls
                .lock()
                .unwrap()
                .iter()
                .map(|call| call.channel.as_str())
                .collect::<Vec<_>>(),
            vec!["CAAA", "CBBB"]
        );

        let mut two_conversation_report = full_report;
        two_conversation_report.conversations.truncate(2);
        two_conversation_report.has_more_conversations = true;
        two_conversation_report.truncation_reason = Some(InboxTruncationReason::ByteLimit);
        let boundary_limit = serde_json::to_vec_pretty(&two_conversation_report)
            .unwrap()
            .len();
        two_conversation_report.truncation_reason = Some(InboxTruncationReason::ConversationLimit);
        assert!(
            serde_json::to_vec_pretty(&two_conversation_report)
                .unwrap()
                .len()
                > boundary_limit
        );

        let api = make_api();
        let history_calls = api.history_calls.clone();
        let mut bounded_service = service(api);
        bounded_service.max_response_bytes = boundary_limit;
        let report = bounded_service.inbox(2, 1).await.unwrap();

        assert_eq!(report.conversations.len(), 1);
        assert_eq!(
            report.truncation_reason,
            Some(InboxTruncationReason::ByteLimit)
        );
        assert!(serde_json::to_vec_pretty(&report).unwrap().len() <= boundary_limit);
        assert_eq!(history_calls.lock().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn inbox_fails_closed_when_even_empty_metadata_exceeds_the_byte_limit() {
        let api = fake_api();
        let conversation_calls = api.conversation_calls.clone();
        let history_calls = api.history_calls.clone();
        let mut bounded_service = service(api);
        bounded_service.max_response_bytes = 1;

        assert!(matches!(
            bounded_service.inbox(1, 1).await,
            Err(Error::ResponseTooLarge {
                method: "inbox",
                limit: 1
            })
        ));
        assert!(conversation_calls.lock().unwrap().is_empty());
        assert!(history_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn inbox_uses_an_explicit_id_fallback_when_discovery_metadata_is_missing() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("CMISSING", true, 1)];
        let history_calls = api.history_calls.clone();

        let report = service(api).inbox(10, 5).await.unwrap();

        let entry = &report.conversations[0];
        assert_eq!(entry.conversation.id, "CMISSING");
        assert_eq!(entry.conversation.name, "CMISSING");
        assert_eq!(entry.conversation.display_name, "CMISSING");
        assert!(entry.conversation.name_is_fallback);
        assert!(!entry.conversation.metadata_is_complete);
        assert_eq!(
            *history_calls.lock().unwrap(),
            vec![HistoryCall {
                channel: "CMISSING".into(),
                cursor: None,
                limit: 5,
            }]
        );
    }

    #[tokio::test]
    async fn inbox_rejects_unread_and_discovery_kind_disagreement_before_history() {
        let mut api = fake_api();
        api.counts.channels = vec![entry("GTEAM", true, 1)];
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                is_mpim: true,
                ..raw_conversation("GTEAM", "mpdm-alice--bob-1")
            }],
            ..RawConversationsPage::default()
        }]));
        let history_calls = api.history_calls.clone();

        assert!(matches!(
            service(api).inbox(10, 5).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));
        assert!(history_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn inbox_rejects_invalid_bounds_before_any_slack_request() {
        for (conversation_limit, message_limit, field) in [
            (0, 1, "conversation_limit"),
            (MAX_INBOX_CONVERSATIONS + 1, 1, "conversation_limit"),
            (1, 0, "message_limit"),
            (1, MAX_MESSAGES + 1, "message_limit"),
        ] {
            assert!(matches!(
                service(FailApi)
                    .inbox(conversation_limit, message_limit)
                    .await,
                Err(Error::InvalidInput {
                    field: actual,
                    ..
                }) if actual == field
            ));
        }
    }

    #[tokio::test]
    async fn doctor_probes_the_api() {
        assert!(matches!(
            service(FailApi).doctor().await,
            Err(Error::Authentication)
        ));
    }

    #[tokio::test]
    async fn lists_and_normalizes_all_conversation_kinds() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                RawConversation {
                    is_private: true,
                    ..raw_conversation("CGENERAL", "general")
                },
                RawConversation {
                    id: "DALI".into(),
                    is_im: true,
                    user: Some("WALI".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    is_mpim: true,
                    ..raw_conversation("CTEAM", "mpdm-alice--bob-1")
                },
                RawConversation {
                    is_mpim: true,
                    ..raw_conversation("GTEAM", "mpdm-carol--dan-1")
                },
            ],
            response_metadata: RawResponseMetadata {
                next_cursor: "next-page".into(),
            },
        }]));
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("WALI", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));

        let page = service(api)
            .list_conversations(Some("start-page"), 4)
            .await
            .unwrap();
        assert_eq!(page.conversations.len(), 4);
        assert_eq!(page.conversations[0].name, "general");
        assert_eq!(page.conversations[1].kind, ConversationKind::DirectMessage);
        assert_eq!(page.conversations[1].name, "alice");
        assert_eq!(page.conversations[1].display_name, "Alice Example");
        assert_eq!(page.conversations[1].user_id.as_deref(), Some("WALI"));
        assert_eq!(
            page.conversations[2].kind,
            ConversationKind::GroupDirectMessage
        );
        assert_eq!(
            page.conversations[3].kind,
            ConversationKind::GroupDirectMessage
        );
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next-page"));
    }

    #[tokio::test]
    async fn finds_a_c_prefixed_mpim_conversation() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                is_mpim: true,
                ..raw_conversation("CTEAM", "mpdm-alice--bob-1")
            }],
            ..RawConversationsPage::default()
        }]));

        let report = service(api).find_conversations("alice", 1).await.unwrap();

        assert_eq!(report.conversations.len(), 1);
        assert_eq!(
            report.conversations[0].kind,
            ConversationKind::GroupDirectMessage
        );
    }

    #[tokio::test]
    async fn rejects_invalid_mpim_conversation_ids() {
        for id in ["DTEAM", "C-TEAM"] {
            let mut api = fake_api();
            api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
                channels: vec![RawConversation {
                    is_mpim: true,
                    ..raw_conversation(id, "mpdm-alice--bob-1")
                }],
                ..RawConversationsPage::default()
            }]));

            assert!(matches!(
                service(api).list_conversations(None, 1).await,
                Err(Error::InvalidResponse {
                    method: "conversations.list"
                })
            ));
        }
    }

    #[tokio::test]
    async fn dm_without_loaded_user_metadata_has_a_stable_fallback() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "DMISSING".into(),
                is_im: true,
                user: Some("UMISSING".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        let page = service(api).list_conversations(None, 1).await.unwrap();
        assert_eq!(page.conversations[0].name, "UMISSING");
        assert_eq!(page.conversations[0].display_name, "UMISSING");
    }

    #[tokio::test]
    async fn finds_conversations_across_pages_and_reports_result_truncation() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([
            RawConversationsPage {
                channels: vec![raw_conversation("COTHER", "other")],
                response_metadata: RawResponseMetadata {
                    next_cursor: "page-2".into(),
                },
            },
            RawConversationsPage {
                channels: vec![
                    raw_conversation("CTARGET1", "target-one"),
                    raw_conversation("CTARGET2", "target-two"),
                ],
                ..RawConversationsPage::default()
            },
        ]));
        let report = service(api)
            .find_conversations(" target ", 1)
            .await
            .unwrap();
        assert_eq!(report.query, "target");
        assert_eq!(report.conversations[0].id, "CTARGET1");
        assert!(report.truncated);
        assert_eq!(
            report.truncation_reason,
            Some(ConversationSearchTruncationReason::ResultLimit)
        );
    }

    #[tokio::test]
    async fn rejects_repeated_conversation_cursors() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([
            RawConversationsPage {
                response_metadata: RawResponseMetadata {
                    next_cursor: "same".into(),
                },
                ..RawConversationsPage::default()
            },
            RawConversationsPage {
                response_metadata: RawResponseMetadata {
                    next_cursor: "same".into(),
                },
                ..RawConversationsPage::default()
            },
        ]));
        assert!(matches!(
            service(api).find_conversations("missing", 10).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));
    }

    #[tokio::test]
    async fn list_conversations_rejects_the_supplied_cursor_as_its_continuation() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            response_metadata: RawResponseMetadata {
                next_cursor: "same".into(),
            },
            ..RawConversationsPage::default()
        }]));
        assert!(matches!(
            service(api).list_conversations(Some("same"), 10).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));
    }

    #[tokio::test]
    async fn rejects_malformed_slack_response_cursors() {
        let mut list_api = fake_api();
        list_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            response_metadata: RawResponseMetadata {
                next_cursor: "bad\ncursor".into(),
            },
            ..RawConversationsPage::default()
        }]));
        assert!(matches!(
            service(list_api).list_conversations(None, 10).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));

        let mut find_api = fake_api();
        find_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            response_metadata: RawResponseMetadata {
                next_cursor: "x".repeat(2049),
            },
            ..RawConversationsPage::default()
        }]));
        assert!(matches!(
            service(find_api).find_conversations("missing", 10).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));
    }

    #[tokio::test]
    async fn incomplete_user_enrichment_keeps_find_results_truthfully_truncated() {
        fn incomplete_user_pages() -> VecDeque<RawUsersPage> {
            (0..MAX_USER_PAGES)
                .map(|page| RawUsersPage {
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("user-page-{page}"),
                    },
                    ..RawUsersPage::default()
                })
                .collect()
        }

        let mut empty_api = fake_api();
        empty_api.user_pages = Mutex::new(incomplete_user_pages());
        empty_api.conversation_pages =
            Mutex::new(VecDeque::from([RawConversationsPage::default()]));
        let empty = service(empty_api)
            .find_conversations("missing", 10)
            .await
            .unwrap();
        assert!(empty.conversations.is_empty());
        assert!(empty.truncated);
        assert_eq!(
            empty.truncation_reason,
            Some(ConversationSearchTruncationReason::ScanLimit)
        );

        let mut retained_api = fake_api();
        retained_api.user_pages = Mutex::new(incomplete_user_pages());
        retained_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CTARGET", "target")],
            ..RawConversationsPage::default()
        }]));
        let retained = service(retained_api)
            .find_conversations("target", 10)
            .await
            .unwrap();
        assert_eq!(retained.conversations.len(), 1);
        assert!(retained.truncated);
        assert_eq!(
            retained.truncation_reason,
            Some(ConversationSearchTruncationReason::ScanLimit)
        );
    }

    #[tokio::test]
    async fn resolver_never_claims_certainty_past_either_scan_cap() {
        let mut incomplete_users_api = fake_api();
        incomplete_users_api.user_pages = Mutex::new(
            (0..MAX_USER_PAGES)
                .map(|page| RawUsersPage {
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("user-page-{page}"),
                    },
                    ..RawUsersPage::default()
                })
                .collect(),
        );
        incomplete_users_api.conversation_pages =
            Mutex::new(VecDeque::from([RawConversationsPage {
                channels: vec![raw_conversation("CGENERAL", "general")],
                ..RawConversationsPage::default()
            }]));
        assert!(matches!(
            service(incomplete_users_api)
                .read_channel("general", None, 1)
                .await,
            Err(Error::ScanLimit {
                resource: "Slack conversation",
                ..
            })
        ));

        let mut conversation_pages = VecDeque::new();
        for page in 0..MAX_CONVERSATION_PAGES {
            conversation_pages.push_back(RawConversationsPage {
                channels: (page == 0)
                    .then(|| raw_conversation("CGENERAL", "general"))
                    .into_iter()
                    .collect(),
                response_metadata: RawResponseMetadata {
                    next_cursor: format!("conversation-page-{page}"),
                },
            });
        }
        let mut incomplete_conversations_api = fake_api();
        incomplete_conversations_api.conversation_pages = Mutex::new(conversation_pages);
        assert!(matches!(
            service(incomplete_conversations_api)
                .read_channel("general", None, 1)
                .await,
            Err(Error::ScanLimit {
                resource: "Slack conversation",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn resolves_exact_names_but_keeps_ids_on_the_fast_path() {
        let mut id_api = fake_api();
        id_api.history.messages = vec![raw_message("100.000001", "by id")];
        let id_page = service(id_api)
            .read_channel("CABCDEFGH", None, 1)
            .await
            .unwrap();
        assert_eq!(id_page.channel_id, "CABCDEFGH");

        let mut name_api = fake_api();
        name_api.history.messages = vec![raw_message("100.000001", "by name")];
        name_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CGENERAL", "general")],
            ..RawConversationsPage::default()
        }]));
        let name_page = service(name_api)
            .read_channel("#GENERAL", None, 1)
            .await
            .unwrap();
        assert_eq!(name_page.channel_id, "CGENERAL");
        assert_eq!(name_page.messages[0].text, "by name");
    }

    #[tokio::test]
    async fn unprefixed_case_variants_resolve_across_reads_and_search() {
        let mut channel_api = fake_api();
        channel_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C00000001", "general")],
            ..RawConversationsPage::default()
        }]));
        let channel = service(channel_api)
            .read_channel("General", None, 1)
            .await
            .unwrap();
        assert_eq!(channel.channel_id, "C00000001");

        let mut thread_api = fake_api();
        thread_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C00000002", "design")],
            ..RawConversationsPage::default()
        }]));
        let thread = service(thread_api)
            .read_thread("Design", "100.000001", None, 1)
            .await
            .unwrap();
        assert_eq!(thread.channel_id, "C00000002");

        let mut message_api = fake_api();
        message_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C00000003", "customersuccess")],
            ..RawConversationsPage::default()
        }]));
        message_api.message_list.messages =
            BTreeMap::from([("target".into(), raw_message("100.000001", "by exact name"))]);
        let message = service(message_api)
            .get_message("CustomerSuccess", "100.000001")
            .await
            .unwrap();
        assert_eq!(message.channel_id, "C00000003");

        let mut search_api = fake_api();
        search_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C00000004", "general")],
            ..RawConversationsPage::default()
        }]));
        let search = service(search_api)
            .search_messages("deploy", Some("General"), None, None, None, 20)
            .await
            .unwrap();
        assert_eq!(search.query, "deploy in:general");
    }

    #[tokio::test]
    async fn slack_shaped_inputs_take_id_precedence_and_prefixes_force_names() {
        let id_page = service(fake_api())
            .read_channel("GENERAL2", None, 1)
            .await
            .unwrap();
        assert_eq!(id_page.channel_id, "GENERAL2");

        let mut name_api = fake_api();
        name_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C00000005", "general2")],
            ..RawConversationsPage::default()
        }]));
        let name_page = service(name_api)
            .read_channel("#GENERAL2", None, 1)
            .await
            .unwrap();
        assert_eq!(name_page.channel_id, "C00000005");
    }

    #[tokio::test]
    async fn rejects_ambiguous_or_missing_conversation_names() {
        let mut ambiguous_api = fake_api();
        ambiguous_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("CONE", "shared"),
                raw_conversation("CTWO", "SHARED"),
            ],
            ..RawConversationsPage::default()
        }]));
        assert!(matches!(
            service(ambiguous_api).read_channel("shared", None, 1).await,
            Err(Error::InvalidInput {
                field: "conversation",
                ..
            })
        ));

        assert!(matches!(
            service(fake_api()).read_channel("missing", None, 1).await,
            Err(Error::NotFound {
                resource: "Slack conversation"
            })
        ));
    }

    #[tokio::test]
    async fn validates_conversation_pages_and_inputs() {
        let slack_service = service(fake_api());
        assert!(matches!(
            slack_service.list_conversations(Some(""), 1).await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
        assert!(matches!(
            slack_service.list_conversations(None, 201).await,
            Err(Error::InvalidInput { field: "limit", .. })
        ));

        let mut malformed_api = fake_api();
        malformed_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "DBAD".into(),
                is_im: true,
                user: None,
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        assert!(matches!(
            service(malformed_api).list_conversations(None, 1).await,
            Err(Error::InvalidResponse {
                method: "conversations.list"
            })
        ));
    }

    #[tokio::test]
    async fn searches_messages_with_conversation_date_and_cursor_filters() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CGENERAL", "general")],
            ..RawConversationsPage::default()
        }]));
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        api.search = RawMessageSearchResponse {
            query: "ignored server echo".into(),
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "CGENERAL".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    thread_ts: Some("100.000000".into()),
                    user: Some("U123".into()),
                    text: "deploy complete".into(),
                    permalink: Some(
                        "https://example.slack.com/archives/CGENERAL/p100000001".into(),
                    ),
                    ..RawMessageSearchMatch::default()
                }],
                total: 2,
                pagination: RawMessageSearchPagination {
                    next_cursor: "next-search-page".into(),
                },
            },
            ..RawMessageSearchResponse::default()
        };
        let calls = api.search_calls.clone();
        let user_calls = api.user_calls.clone();

        let page = service(api)
            .search_messages(
                " deploy ",
                Some("#GENERAL"),
                Some("2024-02-29"),
                Some("2024-03-01"),
                Some("current-page"),
                1,
            )
            .await
            .unwrap();
        assert_eq!(
            page.query,
            "deploy in:general after:2024-02-29 before:2024-03-01"
        );
        assert_eq!(page.matches.len(), 1);
        assert_eq!(page.matches[0].channel_name, "general");
        assert_eq!(page.matches[0].thread_ts.as_deref(), Some("100.000000"));
        assert_eq!(page.matches[0].author_name.as_deref(), Some("alice"));
        assert_eq!(
            page.matches[0].author_display_name.as_deref(),
            Some("Alice Example")
        );
        assert_eq!(
            page.matches[0].author_resolution,
            AuthorResolution::Directory
        );
        assert_eq!(page.total, 2);
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next-search-page"));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[SearchCall {
                query: page.query,
                cursor: Some("current-page".into()),
                limit: 1,
            }]
        );
        assert_single_user_directory_call(&user_calls);
    }

    #[tokio::test]
    async fn searches_a_dm_by_conversation_id_using_the_participant_modifier() {
        let mut api = fake_api();
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "D01234567".into(),
                is_im: true,
                user: Some("UALI".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        let page = service(api)
            .search_messages("incident", Some("D01234567"), None, None, None, 20)
            .await
            .unwrap();
        assert_eq!(page.query, "incident in:<@UALI>");
    }

    #[tokio::test]
    async fn validates_search_query_dates_cursor_and_limit_before_searching() {
        assert!(matches!(
            service(fake_api())
                .search_messages("\n", None, None, None, None, 20)
                .await,
            Err(Error::InvalidInput { field: "query", .. })
        ));
        assert!(matches!(
            service(fake_api())
                .search_messages("deploy", None, Some("2025-02-29"), None, None, 20)
                .await,
            Err(Error::InvalidInput { field: "after", .. })
        ));
        assert!(matches!(
            service(fake_api())
                .search_messages(
                    "deploy",
                    None,
                    Some("2025-03-02"),
                    Some("2025-03-01"),
                    None,
                    20,
                )
                .await,
            Err(Error::InvalidInput { field: "after", .. })
        ));
        assert!(matches!(
            service(fake_api())
                .search_messages("deploy", None, None, None, Some(""), 20)
                .await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
        assert!(matches!(
            service(fake_api())
                .search_messages("deploy", None, None, None, Some("   "), 20)
                .await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
        assert!(matches!(
            service(fake_api())
                .search_messages("deploy", None, None, None, None, 101)
                .await,
            Err(Error::InvalidInput { field: "limit", .. })
        ));
    }

    #[tokio::test]
    async fn rejects_malformed_or_over_limit_search_responses() {
        let mut malformed_api = fake_api();
        malformed_api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "CGENERAL".into(),
                        name: "general".into(),
                    },
                    ts: "bad".into(),
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        assert!(matches!(
            service(malformed_api)
                .search_messages("deploy", None, None, None, None, 20)
                .await,
            Err(Error::InvalidResponse {
                method: "search.messages"
            })
        ));

        let mut oversized_api = fake_api();
        oversized_api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![
                    RawMessageSearchMatch::default(),
                    RawMessageSearchMatch::default(),
                ],
                total: 2,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        assert!(matches!(
            service(oversized_api)
                .search_messages("deploy", None, None, None, None, 1)
                .await,
            Err(Error::InvalidResponse {
                method: "search.messages"
            })
        ));

        let mut conflicting_api = fake_api();
        conflicting_api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![],
                total: 0,
                pagination: RawMessageSearchPagination {
                    next_cursor: "nested".into(),
                },
            },
            response_metadata: RawResponseMetadata {
                next_cursor: "top-level".into(),
            },
            ..RawMessageSearchResponse::default()
        };
        assert!(matches!(
            service(conflicting_api)
                .search_messages("deploy", None, None, None, None, 20)
                .await,
            Err(Error::InvalidResponse {
                method: "search.messages"
            })
        ));
    }

    #[tokio::test]
    async fn search_continuation_depends_on_the_cursor_not_workspace_total() {
        let mut api = fake_api();
        api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "CGENERAL".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    ..RawMessageSearchMatch::default()
                }],
                total: 50,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        let page = service(api)
            .search_messages("deploy", None, None, None, Some("final-page"), 20)
            .await
            .unwrap();
        assert!(!page.has_more);
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.total, 50);
    }

    #[tokio::test]
    async fn search_rejects_a_repeated_first_or_explicit_cursor() {
        for (current, repeated) in [(None, "*"), (Some("page-2"), "page-2")] {
            let mut api = fake_api();
            api.search = RawMessageSearchResponse {
                messages: RawMessageSearchMatches {
                    matches: vec![],
                    total: 0,
                    pagination: RawMessageSearchPagination {
                        next_cursor: repeated.into(),
                    },
                },
                ..RawMessageSearchResponse::default()
            };
            assert!(matches!(
                service(api)
                    .search_messages("deploy", None, None, None, current, 20)
                    .await,
                Err(Error::InvalidResponse {
                    method: "search.messages"
                })
            ));
        }
    }

    #[tokio::test]
    async fn normalizes_bounded_channel_and_thread_messages() {
        let mut api = fake_api();
        api.history = RawMessagePage {
            messages: vec![
                raw_message("100.000001", "first"),
                raw_message("100.000002", "second"),
            ],
            has_more: false,
            response_metadata: RawResponseMetadata {
                next_cursor: "next".into(),
            },
        };
        api.replies = api.history.clone();
        let service = service(api);

        let page = service.read_channel("C123", None, 1).await.unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].text, "first");
        assert_eq!(
            page.messages[0].permalink.as_deref(),
            Some("https://example.slack.com/archives/C123/p100000001")
        );
        assert_eq!(
            page.messages[0].permalink_resolution,
            PermalinkResolution::Complete
        );
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next"));

        let thread = service
            .read_thread("C123", "100.000001", None, 2)
            .await
            .unwrap();
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.thread_ts, "100.000001");
    }

    #[test]
    fn canonical_permalinks_cover_conversation_kinds_roots_and_replies() {
        let workspace = url::Url::parse("https://example.slack.com").unwrap();
        for channel in ["C123", "D123", "G123"] {
            let root = message_permalinks(&workspace, channel, "100.1", None);
            assert_eq!(
                root.permalink.as_deref(),
                Some(format!("https://example.slack.com/archives/{channel}/p100100000").as_str())
            );
            assert_eq!(root.thread_root_permalink, None);
            assert_eq!(root.resolution, PermalinkResolution::Complete);

            let reply = message_permalinks(&workspace, channel, "101.000002", Some("100.100000"));
            assert_eq!(
                reply.permalink.as_deref(),
                Some(
                    format!(
                        "https://example.slack.com/archives/{channel}/p101000002?thread_ts=100.100000&cid={channel}"
                    )
                    .as_str()
                )
            );
            assert_eq!(
                reply.thread_root_permalink.as_deref(),
                Some(format!("https://example.slack.com/archives/{channel}/p100100000").as_str())
            );
            assert_eq!(reply.resolution, PermalinkResolution::Complete);
        }
    }

    #[test]
    fn canonical_permalinks_handle_self_roots_and_partial_degradation() {
        let workspace = url::Url::parse("https://example.slack.com").unwrap();
        let self_root = message_permalinks(&workspace, "C123", "100.1", Some("100.100000"));
        assert_eq!(
            self_root.permalink.as_deref(),
            Some("https://example.slack.com/archives/C123/p100100000")
        );
        assert_eq!(self_root.thread_root_permalink, None);
        assert_eq!(self_root.resolution, PermalinkResolution::Complete);

        let root_only = message_permalinks(&workspace, "C123", "101.0000001", Some("100.000001"));
        assert_eq!(root_only.permalink, None);
        assert_eq!(
            root_only.thread_root_permalink.as_deref(),
            Some("https://example.slack.com/archives/C123/p100000001")
        );
        assert_eq!(root_only.resolution, PermalinkResolution::Partial);

        let unavailable = message_permalinks(&workspace, "C123", "100.0000001", None);
        assert_eq!(unavailable.permalink, None);
        assert_eq!(unavailable.thread_root_permalink, None);
        assert_eq!(unavailable.resolution, PermalinkResolution::Unavailable);

        assert_eq!(
            permalink_resolution(&Some("exact".into()), &None, true),
            PermalinkResolution::Partial
        );
    }

    #[test]
    fn canonical_permalinks_reject_unsafe_origins_identifiers_and_timestamps() {
        for origin in [
            "http://example.slack.com",
            "https://example.com",
            "https://one.two.slack.com",
            "https://user@example.slack.com",
            "https://example.slack.com:444",
            "https://example.slack.com/path",
            "https://example.slack.com/?query=1",
            "https://example.slack.com/#fragment",
        ] {
            let links = message_permalinks(
                &url::Url::parse(origin).unwrap(),
                "C123",
                "100.000001",
                None,
            );
            assert_eq!(links.resolution, PermalinkResolution::Unavailable);
            assert_eq!(links.permalink, None);
        }
        for (channel, timestamp) in [
            ("X123", "100.000001"),
            ("C12/3", "100.000001"),
            ("C123", "bad"),
            ("C123", "100."),
            ("C123", "100.1234567"),
        ] {
            let links = message_permalinks(
                &url::Url::parse("https://example.slack.com").unwrap(),
                channel,
                timestamp,
                None,
            );
            assert_eq!(links.resolution, PermalinkResolution::Unavailable);
            assert_eq!(links.permalink, None);
        }
    }

    #[test]
    fn search_permalinks_are_always_canonical_local_serializations() {
        let workspace = url::Url::parse("https://example.slack.com").unwrap();
        let raw_candidates = vec![
            None,
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123"
                    .into(),
            ),
            Some(
                "https://example.slack.com/archives/C123/p101000002?cid=C123&thread_ts=100.000001"
                    .into(),
            ),
            Some("https://other.slack.com/archives/C123/p101000002".into()),
            Some("https://example.slack.com/archives/D123/p101000002".into()),
            Some("https://example.slack.com/archives/C123/p999000002".into()),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=999.000001&cid=C123"
                    .into(),
            ),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=D123"
                    .into(),
            ),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&thread_ts=100.000001&cid=C123".into(),
            ),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123&cid=C123".into(),
            ),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123&tracking=1".into(),
            ),
            Some("https://example.slack.com/archives%2FC123%2Fp101000002".into()),
            Some("https://example.slack.com/archives/C123/p101000002#fragment".into()),
            Some("https://example.slack.com/archives/C123/p101000002\nunsafe".into()),
            Some("x".repeat(9_000)),
        ];
        for candidate in raw_candidates {
            let matches = normalize_search_matches(
                &workspace,
                vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "101.000002".into(),
                    thread_ts: Some("100.000001".into()),
                    permalink: candidate,
                    ..RawMessageSearchMatch::default()
                }],
            )
            .unwrap();
            assert_eq!(
                matches[0].permalink.as_deref(),
                Some(
                    "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123"
                )
            );
            assert_eq!(
                matches[0].thread_root_permalink.as_deref(),
                Some("https://example.slack.com/archives/C123/p100000001")
            );
            assert_eq!(
                matches[0].permalink_resolution,
                PermalinkResolution::Complete
            );
        }
    }

    #[test]
    fn search_permalinks_safely_recover_missing_reply_context() {
        let workspace = url::Url::parse("https://example.slack.com").unwrap();
        let normalize = |thread_ts: Option<&str>, permalink: &str| {
            normalize_search_matches(
                &workspace,
                vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "101.000002".into(),
                    thread_ts: thread_ts.map(str::to_owned),
                    permalink: Some(permalink.into()),
                    ..RawMessageSearchMatch::default()
                }],
            )
            .unwrap()
            .remove(0)
        };

        let derived = normalize(
            None,
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.1&cid=C123",
        );
        assert_eq!(derived.thread_ts.as_deref(), Some("100.100000"));
        assert_eq!(
            derived.permalink.as_deref(),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.100000&cid=C123"
            )
        );
        assert_eq!(
            derived.thread_root_permalink.as_deref(),
            Some("https://example.slack.com/archives/C123/p100100000")
        );
        assert_eq!(derived.permalink_resolution, PermalinkResolution::Complete);

        let root = normalize(None, "https://example.slack.com/archives/C123/p101000002");
        assert_eq!(root.thread_ts, None);
        assert_eq!(
            root.permalink.as_deref(),
            Some("https://example.slack.com/archives/C123/p101000002")
        );
        assert_eq!(root.thread_root_permalink, None);
        assert_eq!(root.permalink_resolution, PermalinkResolution::Complete);

        let missing = normalize_search_matches(
            &workspace,
            vec![RawMessageSearchMatch {
                channel: RawMessageSearchChannel {
                    id: "C123".into(),
                    name: "general".into(),
                },
                ts: "101.000002".into(),
                ..RawMessageSearchMatch::default()
            }],
        )
        .unwrap()
        .remove(0);
        assert_eq!(missing.thread_ts, None);
        assert_eq!(missing.permalink, None);
        assert_eq!(missing.thread_root_permalink, None);
        assert_eq!(
            missing.permalink_resolution,
            PermalinkResolution::Unavailable
        );

        let mismatch = normalize(
            Some("100.000001"),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=999.000001&cid=C123",
        );
        assert_eq!(mismatch.thread_ts.as_deref(), Some("100.000001"));
        assert_eq!(
            mismatch.permalink.as_deref(),
            Some(
                "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123"
            )
        );
    }

    #[test]
    fn malformed_search_permalinks_cannot_supply_thread_context() {
        let workspace = url::Url::parse("https://example.slack.com").unwrap();
        let candidates = vec![
            "https://other.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/D123/p101000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/C123/p999000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=D123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123&tracking=1".into(),
            "https://example.slack.com/archives%2FC123%2Fp101000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123#fragment".into(),
            "https://user@example.slack.com/archives/C123/p101000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com:444/archives/C123/p101000002?thread_ts=100.000001&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002?thread_ts=101.000002&cid=C123".into(),
            "https://example.slack.com/archives/C123/p101000002\nunsafe".into(),
            "x".repeat(9_000),
        ];
        for candidate in candidates {
            let message = normalize_search_matches(
                &workspace,
                vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "101.000002".into(),
                    permalink: Some(candidate),
                    ..RawMessageSearchMatch::default()
                }],
            )
            .unwrap()
            .remove(0);
            assert_eq!(message.thread_ts, None);
            assert_eq!(message.permalink, None);
            assert_eq!(message.thread_root_permalink, None);
            assert_eq!(
                message.permalink_resolution,
                PermalinkResolution::Unavailable
            );
        }
    }

    #[tokio::test]
    async fn lossless_rich_text_reads_preserve_unknown_missing_and_empty_blocks() {
        let unknown_blocks = vec![json!({
            "type": "rich_text",
            "block_id": "synthetic",
            "future_top_level": {"keep": [true, 7, null]},
            "elements": [{
                "type": "rich_text_section",
                "future_element": "retained",
                "elements": [{
                    "type": "text",
                    "text": "hello",
                    "style": {"bold": true, "future_style": "kept"}
                }]
            }]
        })];
        let mut first = raw_message("100.000001", "fallback");
        first.blocks = Some(unknown_blocks.clone());
        let mut second = raw_message("100.000002", "explicitly empty");
        second.blocks = Some(vec![]);
        let third = raw_message("100.000003", "omitted");

        let mut api = fake_api();
        api.history.messages = vec![first, second, third];
        api.search = RawMessageSearchResponse {
            query: "fallback".into(),
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    text: "fallback".into(),
                    blocks: Some(unknown_blocks.clone()),
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        let service = service(api);

        let history = service.read_channel("C123", None, 3).await.unwrap();
        assert_eq!(history.messages[0].blocks, Some(unknown_blocks.clone()));
        assert_eq!(history.messages[1].blocks, Some(vec![]));
        assert_eq!(history.messages[2].blocks, None);

        let search = service
            .search_messages("fallback", None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(search.matches[0].blocks, Some(unknown_blocks));
    }

    #[test]
    fn canonical_mentions_are_unique_ordered_and_never_resolve_inside_code() {
        let text = concat!(
            "é <@UALICE> and <@UBOB> and <@UALICE> ",
            "`inline <@UCODE1>` ``paired <@UCODE2>`` ```fenced <@UCODE3>``` ",
            "<@bad-id> <@UALICE|alice> unmatched ` then <@UAFTER>"
        );

        let (ids, truncated) = scan_canonical_mentions(text);
        assert_eq!(ids, ["UALICE", "UBOB", "UAFTER"]);
        assert!(!truncated);

        let rendered = render_canonical_mentions(
            text,
            &HashMap::from([
                ("UALICE".into(), "alice".into()),
                ("UBOB".into(), "Bob Example".into()),
                ("UCODE1".into(), "must-not-render".into()),
                ("UCODE2".into(), "must-not-render".into()),
                ("UCODE3".into(), "must-not-render".into()),
                ("UAFTER".into(), "after".into()),
            ]),
        )
        .unwrap();
        assert_eq!(
            rendered,
            concat!(
                "é @alice and @Bob Example and @alice ",
                "`inline <@UCODE1>` ``paired <@UCODE2>`` ```fenced <@UCODE3>``` ",
                "<@bad-id> <@UALICE|alice> unmatched ` then @after"
            )
        );
    }

    #[test]
    fn strict_rich_text_mentions_render_names_and_keep_literal_code_untouched() {
        let blocks = vec![json!({
            "type": "rich_text",
            "elements": [
                {
                    "type": "rich_text_section",
                    "elements": [
                        {"type": "text", "text": "Hello "},
                        {"type": "user", "user_id": "UALICE"},
                        {"type": "text", "text": " and <@ULITERAL> "},
                        {
                            "type": "user",
                            "user_id": "USTYLED",
                            "style": {"code": true}
                        }
                    ]
                },
                {
                    "type": "rich_text_preformatted",
                    "elements": [{"type": "user", "user_id": "UCODE"}]
                },
                {
                    "type": "rich_text_quote",
                    "elements": [{"type": "user", "user_id": "UBOB"}]
                }
            ]
        })];
        let canonical = "canonical <@UCANON>";
        let (mut rendered_text, mut resolution, mut mentions) =
            initial_mention_fields(canonical, Some(&blocks));
        assert_eq!(resolution, MentionResolution::NotAttempted);
        assert_eq!(
            mentions
                .iter()
                .map(|mention| mention.id.as_str())
                .collect::<Vec<_>>(),
            ["UALICE", "UBOB"]
        );

        let mut display_only = raw_user("UBOB", "\n", "Bob Example");
        display_only.real_name = None;
        display_only.profile.real_name = None;
        let directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([
                (
                    "UALICE".into(),
                    normalize_user(raw_user("UALICE", "alice", "Alice Example")),
                ),
                ("UBOB".into(), normalize_user(display_only)),
            ]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });
        enrich_mentions(
            canonical,
            Some(&blocks),
            &mut rendered_text,
            &mut resolution,
            &mut mentions,
            &directory,
        );

        assert_eq!(
            rendered_text,
            "Hello @alice and <@ULITERAL> <@USTYLED>\n<@UCODE>\n@Bob Example"
        );
        assert_eq!(resolution, MentionResolution::Complete);
        assert_eq!(
            mentions,
            [
                MessageMention {
                    id: "UALICE".into(),
                    username: Some("alice".into()),
                    display_name: Some("Alice Example".into()),
                },
                MessageMention {
                    id: "UBOB".into(),
                    username: None,
                    display_name: Some("Bob Example".into()),
                },
            ]
        );
    }

    #[test]
    fn unsupported_rich_text_falls_back_to_canonical_mentions_without_losing_blocks() {
        let blocks = vec![json!({
            "type": "rich_text",
            "elements": [{
                "type": "future_rich_text_node",
                "user_id": "UNOTTRUSTED"
            }]
        })];
        let canonical = "Fallback <@UALICE>";
        let (mut rendered_text, mut resolution, mut mentions) =
            initial_mention_fields(canonical, Some(&blocks));
        let directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([(
                "UALICE".into(),
                normalize_user(raw_user("UALICE", "alice", "Alice Example")),
            )]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });

        enrich_mentions(
            canonical,
            Some(&blocks),
            &mut rendered_text,
            &mut resolution,
            &mut mentions,
            &directory,
        );

        assert_eq!(rendered_text, "Fallback @alice");
        assert_eq!(resolution, MentionResolution::Complete);
        assert_eq!(mentions.len(), 1);
        assert_eq!(mentions[0].id, "UALICE");
        assert_eq!(blocks[0]["elements"][0]["type"], "future_rich_text_node");
    }

    #[test]
    fn malformed_rich_text_placement_and_styles_fall_back_to_canonical_mentions() {
        let malformed_blocks = [
            vec![json!({
                "type": "rich_text",
                "elements": [{"type": "user", "user_id": "UWRONG"}]
            })],
            vec![json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_list",
                    "style": "bullet",
                    "elements": [{"type": "user", "user_id": "UWRONG"}]
                }]
            })],
            vec![json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_section",
                    "elements": [{
                        "type": "rich_text_section",
                        "elements": [{"type": "user", "user_id": "UWRONG"}]
                    }]
                }]
            })],
            vec![json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_section",
                    "elements": [{
                        "type": "user",
                        "user_id": "UWRONG",
                        "style": {"code": "true"}
                    }]
                }]
            })],
        ];
        let directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([(
                "UALICE".into(),
                normalize_user(raw_user("UALICE", "alice", "Alice Example")),
            )]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });
        let canonical = "Hi <@UALICE>";

        for blocks in malformed_blocks {
            assert!(render_rich_text_mentions(&blocks, None).is_none());
            let original = blocks.clone();
            let (mut rendered, mut resolution, mut mentions) =
                initial_mention_fields(canonical, Some(&blocks));
            assert_eq!(resolution, MentionResolution::NotAttempted);
            assert_eq!(mentions[0].id, "UALICE");

            enrich_mentions(
                canonical,
                Some(&blocks),
                &mut rendered,
                &mut resolution,
                &mut mentions,
                &directory,
            );

            assert_eq!(rendered, "Hi @alice");
            assert_eq!(resolution, MentionResolution::Complete);
            assert_eq!(blocks, original);
        }
    }

    #[test]
    fn rich_text_rendering_is_bounded_by_nodes_and_bytes() {
        let oversized_output = vec![json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_section",
                "elements": [
                    {"type": "text", "text": "x".repeat(MAX_MARKDOWN_BYTES)},
                    {"type": "user", "user_id": "UALICE"}
                ]
            }]
        })];
        let selection = render_rich_text_mentions(&oversized_output, None).unwrap();
        assert_eq!(selection.ids, ["UALICE"]);
        assert_eq!(selection.rendered_text, None);

        let too_many_nodes = vec![json!({
            "type": "rich_text",
            "elements": (0..MAX_RICH_TEXT_RENDER_NODES)
                .map(|_| json!({
                    "type": "rich_text_section",
                    "elements": []
                }))
                .collect::<Vec<_>>()
        })];
        assert!(render_rich_text_mentions(&too_many_nodes, None).is_none());

        let canonical = "Fallback <@UALICE>";
        let (rendered, resolution, mentions) =
            initial_mention_fields(canonical, Some(&too_many_nodes));
        assert_eq!(rendered, canonical);
        assert_eq!(resolution, MentionResolution::NotAttempted);
        assert_eq!(mentions[0].id, "UALICE");
    }

    #[test]
    fn ordered_rich_text_mentions_honor_validated_list_offsets() {
        let blocks = vec![json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_list",
                "style": "ordered",
                "offset": 2,
                "elements": [
                    {
                        "type": "rich_text_section",
                        "elements": [{"type": "user", "user_id": "UALICE"}]
                    },
                    {
                        "type": "rich_text_section",
                        "elements": [{"type": "text", "text": "done"}]
                    }
                ]
            }]
        })];
        let labels = HashMap::from([("UALICE".into(), "alice".into())]);
        let rendered = render_rich_text_mentions(&blocks, Some(&labels)).unwrap();
        assert_eq!(
            rendered.rendered_text.as_deref(),
            Some("3. @alice\n4. done")
        );
        assert_eq!(rendered.ids, ["UALICE"]);

        for invalid in [
            json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_list",
                    "style": "bullet",
                    "offset": 2,
                    "elements": []
                }]
            }),
            json!({
                "type": "rich_text",
                "elements": [{
                    "type": "rich_text_list",
                    "style": "ordered",
                    "offset": -1,
                    "elements": []
                }]
            }),
        ] {
            assert!(render_rich_text_mentions(&[invalid], None).is_none());
        }
    }

    #[test]
    fn mention_resolution_reports_partial_unavailable_conflict_and_bounds() {
        let complete_directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([(
                "UALICE".into(),
                normalize_user(raw_user("UALICE", "alice", "Alice Example")),
            )]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });
        let text = "<@UALICE> and <@UMISSING>";
        let (mut rendered, mut resolution, mut mentions) = initial_mention_fields(text, None);
        enrich_mentions(
            text,
            None,
            &mut rendered,
            &mut resolution,
            &mut mentions,
            &complete_directory,
        );
        assert_eq!(rendered, "@alice and <@UMISSING>");
        assert_eq!(resolution, MentionResolution::Partial);

        let interrupted_directory = AuthorDirectory::Interrupted(UserDirectory {
            users: HashMap::from([(
                "UALICE".into(),
                normalize_user(raw_user("UALICE", "alice", "Alice Example")),
            )]),
            conflicting_ids: HashSet::new(),
            complete: false,
        });
        let (mut rendered, mut resolution, mut mentions) = initial_mention_fields(text, None);
        enrich_mentions(
            text,
            None,
            &mut rendered,
            &mut resolution,
            &mut mentions,
            &interrupted_directory,
        );
        assert_eq!(rendered, "@alice and <@UMISSING>");
        assert_eq!(resolution, MentionResolution::Unavailable);

        let conflict_directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::new(),
            conflicting_ids: HashSet::from(["UALICE".into()]),
            complete: true,
        });
        let (mut rendered, mut resolution, mut mentions) =
            initial_mention_fields("<@UALICE>", None);
        enrich_mentions(
            "<@UALICE>",
            None,
            &mut rendered,
            &mut resolution,
            &mut mentions,
            &conflict_directory,
        );
        assert_eq!(rendered, "<@UALICE>");
        assert_eq!(resolution, MentionResolution::Unavailable);

        let bounded = (0..=MAX_MESSAGE_MENTIONS)
            .map(|index| format!("<@U{index}>"))
            .collect::<Vec<_>>()
            .join(" ");
        let (mut rendered, mut resolution, mut mentions) = initial_mention_fields(&bounded, None);
        assert_eq!(mentions.len(), MAX_MESSAGE_MENTIONS);
        enrich_mentions(
            &bounded,
            None,
            &mut rendered,
            &mut resolution,
            &mut mentions,
            &complete_directory,
        );
        assert_eq!(rendered, bounded);
        assert_eq!(mentions.len(), MAX_MESSAGE_MENTIONS);
        assert_eq!(resolution, MentionResolution::Partial);

        let mut unsafe_user = raw_user("UUNSAFE", "\n", "\t");
        unsafe_user.real_name = Some("\r".into());
        unsafe_user.profile.real_name = Some("\r".into());
        let unsafe_directory = AuthorDirectory::Loaded(UserDirectory {
            users: HashMap::from([("UUNSAFE".into(), normalize_user(unsafe_user))]),
            conflicting_ids: HashSet::new(),
            complete: true,
        });
        let (mut rendered, mut resolution, mut mentions) =
            initial_mention_fields("<@UUNSAFE>", None);
        enrich_mentions(
            "<@UUNSAFE>",
            None,
            &mut rendered,
            &mut resolution,
            &mut mentions,
            &unsafe_directory,
        );
        assert_eq!(rendered, "<@UUNSAFE>");
        assert_eq!(resolution, MentionResolution::Partial);
        assert_eq!(mentions[0].username, None);
        assert_eq!(mentions[0].display_name, None);
    }

    #[tokio::test]
    async fn message_authors_and_mentions_share_one_directory_scan() {
        let mut first = raw_message("100.000001", "Hi <@U456> and <@U789>");
        first.attachments = Some(vec![json!({"future": {"keep": true}})]);
        let mut second = raw_message("100.000002", "Again <@U456>");
        second.user = Some("U456".into());
        let original_attachments = first.attachments.clone();
        let mut api = fake_api();
        api.history.messages = vec![first, second];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("U123", "alice", "Alice Example"),
                raw_user("U456", "bob", "Bob Example"),
                raw_user("U789", "carol", "Carol Example"),
            ],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 2).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.messages[0].text, "Hi <@U456> and <@U789>");
        assert_eq!(page.messages[0].rendered_text, "Hi @bob and @carol");
        assert_eq!(
            page.messages[0].mention_resolution,
            MentionResolution::Complete
        );
        assert_eq!(
            page.messages[0]
                .mentions
                .iter()
                .map(|mention| mention.id.as_str())
                .collect::<Vec<_>>(),
            ["U456", "U789"]
        );
        assert_eq!(page.messages[0].attachments, original_attachments);
        assert_eq!(page.messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(page.messages[1].author_name.as_deref(), Some("bob"));
        assert_eq!(page.messages[1].rendered_text, "Again @bob");
    }

    #[tokio::test]
    async fn message_search_uses_the_same_typed_mention_resolution() {
        let blocks = vec![json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_section",
                "elements": [
                    {"type": "text", "text": "Found "},
                    {"type": "user", "user_id": "U456"}
                ]
            }]
        })];
        let mut api = fake_api();
        api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    user: Some("U123".into()),
                    text: "Found <@U456>".into(),
                    blocks: Some(blocks.clone()),
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("U123", "alice", "Alice Example"),
                raw_user("U456", "bob", "Bob Example"),
            ],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();

        let page = service(api)
            .search_messages("Found", None, None, None, None, 1)
            .await
            .unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.matches[0].text, "Found <@U456>");
        assert_eq!(page.matches[0].rendered_text, "Found @bob");
        assert_eq!(
            page.matches[0].mention_resolution,
            MentionResolution::Complete
        );
        assert_eq!(page.matches[0].mentions[0].id, "U456");
        assert_eq!(page.matches[0].blocks, Some(blocks));
    }

    #[tokio::test]
    async fn lossless_attachments_cross_history_thread_exact_inbox_and_search_paths() {
        let unknown = vec![json!({
            "fallback": "synthetic",
            "future": {"nested": [1, true, null]},
            "fields": [{"title": "kept", "unknown": {"x": "y"}}]
        })];
        let mut with_unknown = raw_message("100.000001", "fallback");
        with_unknown.attachments = Some(unknown.clone());
        let mut with_empty = raw_message("100.000002", "empty");
        with_empty.attachments = Some(vec![]);
        let omitted = raw_message("100.000003", "omitted");

        let mut api = fake_api();
        api.history.messages = vec![with_unknown.clone(), with_empty, omitted];
        api.replies.messages = vec![with_unknown.clone()];
        api.message_list
            .messages
            .insert("exact".into(), with_unknown.clone());
        api.counts.channels = vec![entry("C123", true, 1)];
        api.conversation_pages
            .get_mut()
            .unwrap()
            .push_back(RawConversationsPage {
                channels: vec![raw_conversation("C123", "general")],
                ..RawConversationsPage::default()
            });
        api.search = RawMessageSearchResponse {
            query: "fallback".into(),
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    text: "fallback".into(),
                    attachments: Some(unknown.clone()),
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        let service = service(api);

        let history = service.read_channel("C123", None, 3).await.unwrap();
        assert_eq!(history.messages[0].attachments, Some(unknown.clone()));
        assert_eq!(history.messages[1].attachments, Some(vec![]));
        assert_eq!(history.messages[2].attachments, None);
        assert_eq!(
            service
                .read_thread("C123", "100.000001", None, 1)
                .await
                .unwrap()
                .messages[0]
                .attachments,
            Some(unknown.clone())
        );
        assert_eq!(
            service
                .get_message("C123", "100.000001")
                .await
                .unwrap()
                .attachments,
            Some(unknown.clone())
        );
        assert_eq!(
            service.inbox(1, 1).await.unwrap().conversations[0]
                .messages
                .messages[0]
                .attachments,
            Some(unknown.clone())
        );
        assert_eq!(
            service
                .search_messages("fallback", None, None, None, None, 1)
                .await
                .unwrap()
                .matches[0]
                .attachments,
            Some(unknown)
        );
    }

    #[tokio::test]
    async fn skin_tone_reactions_and_sparse_file_access_cross_every_read_path() {
        let sparse_file: RawFile = serde_json::from_value(json!({
            "id": "FCONNECT",
            "name": null,
            "mode": "file_access",
            "file_access": "check_file_info",
            "created": 0,
            "timestamp": null,
            "user": ""
        }))
        .unwrap();
        let skin_tone = RawReaction {
            name: "thumbsup::skin-tone-6".into(),
            count: 1,
            users: vec!["U123".into()],
        };
        let mut message = raw_message("100.000001", "synthetic");
        message.reactions = vec![skin_tone.clone()];
        message.files = vec![sparse_file.clone()];

        let mut api = fake_api();
        api.history.messages = vec![message.clone()];
        api.replies.messages = vec![message.clone()];
        api.message_list
            .messages
            .insert("exact".into(), message.clone());
        api.counts.channels = vec![entry("C123", true, 1)];
        api.conversation_pages
            .get_mut()
            .unwrap()
            .push_back(RawConversationsPage {
                channels: vec![raw_conversation("C123", "general")],
                ..RawConversationsPage::default()
            });
        api.search = RawMessageSearchResponse {
            query: "synthetic".into(),
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    text: "synthetic".into(),
                    reactions: vec![skin_tone],
                    files: vec![sparse_file.clone()],
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        api.file_response = RawFileResponse { file: sparse_file };
        let service = service(api);

        let assert_message = |message: &Message| {
            assert_eq!(message.reactions[0].name, "thumbsup::skin-tone-6");
            assert_eq!(message.reactions[0].user_ids, ["U123"]);
            let file = &message.files[0];
            assert_eq!(file.id, "FCONNECT");
            assert_eq!(file.name, None);
            assert_eq!(file.size, None);
            assert_eq!(file.timestamp, None);
            assert_eq!(file.uploader_id, None);
            assert_eq!(file.mode.as_deref(), Some("file_access"));
            assert_eq!(file.file_access.as_deref(), Some("check_file_info"));
        };

        assert_message(
            &service
                .read_channel("C123", None, 1)
                .await
                .unwrap()
                .messages[0],
        );
        assert_message(
            &service
                .read_thread("C123", "100.000001", None, 1)
                .await
                .unwrap()
                .messages[0],
        );
        assert_message(&service.get_message("C123", "100.000001").await.unwrap());
        assert_message(
            &service.inbox(1, 1).await.unwrap().conversations[0]
                .messages
                .messages[0],
        );
        let search = service
            .search_messages("synthetic", None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(search.matches[0].reactions[0].name, "thumbsup::skin-tone-6");
        assert_eq!(search.matches[0].files[0].size, None);
        assert_eq!(
            search.matches[0].files[0].file_access.as_deref(),
            Some("check_file_info")
        );
        let exact_file = service.get_file("FCONNECT").await.unwrap();
        assert_eq!(exact_file.name, None);
        assert_eq!(exact_file.timestamp, None);
        assert_eq!(exact_file.size, None);
        assert_eq!(exact_file.uploader_id, None);
    }

    #[tokio::test]
    async fn file_reaction_and_custom_emoji_reads_are_typed_bounded_and_truthful() {
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: RawFile {
                id: "F123".into(),
                name: Some("note.txt".into()),
                title: Some("Note".into()),
                alt_txt: Some("A short note".into()),
                mimetype: Some("text/plain".into()),
                filetype: Some("text".into()),
                pretty_type: Some("Plain Text".into()),
                mode: Some("hosted".into()),
                user: Some("U123".into()),
                size: Some(4),
                created: Some(1),
                timestamp: Some(2),
                url_private: Some("https://files.slack.com/private".into()),
                url_private_download: Some("https://files.slack.com/download".into()),
                permalink: Some("https://sferait.slack.com/files/U123/F123".into()),
                shares: Some(crate::model::RawFileShares {
                    private: BTreeMap::from([(
                        "C123".into(),
                        vec![crate::model::RawFileShare {
                            ts: "100.000001".into(),
                            thread_ts: None,
                        }],
                    )]),
                    ..crate::model::RawFileShares::default()
                }),
                ..RawFile::default()
            },
        };
        api.emoji_response = RawEmojiResponse {
            emoji: BTreeMap::from([
                (
                    "party".into(),
                    "https://emoji.slack-edge.com/T/party/id".into(),
                ),
                ("shipit".into(), "alias:party".into()),
            ]),
        };
        let service = service(api);
        let file = service.get_file("F123").await.unwrap();
        assert_eq!(file.title.as_deref(), Some("Note"));
        assert_eq!(file.alt_text.as_deref(), Some("A short note"));
        assert_eq!(file.shares.as_ref().unwrap().len(), 1);
        assert!(file.shares_complete);
        let emoji = service.list_custom_emoji().await.unwrap();
        assert_eq!(emoji.emoji.len(), 2);
        assert_eq!(emoji.emoji[0].kind, CustomEmojiKind::Image);
        assert_eq!(emoji.emoji[1].alias_for.as_deref(), Some("party"));
    }

    #[test]
    fn file_share_metadata_distinguishes_omitted_complete_and_truncated_data() {
        let omitted = normalize_file(
            RawFile {
                id: "F123".into(),
                ..RawFile::default()
            },
            "files.info",
        )
        .unwrap();
        assert_eq!(omitted.shares, None);
        assert!(!omitted.shares_complete);
        assert_eq!(omitted.channel_ids, None);
        assert_eq!(omitted.group_ids, None);
        assert_eq!(omitted.im_ids, None);

        let complete = normalize_file(
            RawFile {
                id: "F123".into(),
                channels: Some(vec![]),
                groups: Some(vec!["G123".into()]),
                ims: Some(vec!["D123".into()]),
                shares: Some(crate::model::RawFileShares::default()),
                ..RawFile::default()
            },
            "files.info",
        )
        .unwrap();
        assert_eq!(complete.shares, Some(vec![]));
        assert!(complete.shares_complete);
        assert_eq!(complete.channel_ids, Some(vec![]));
        assert_eq!(complete.group_ids, Some(vec!["G123".into()]));
        assert_eq!(complete.im_ids, Some(vec!["D123".into()]));

        for raw in [
            RawFile {
                id: "F123".into(),
                channels: Some(vec!["D123".into()]),
                ..RawFile::default()
            },
            RawFile {
                id: "F123".into(),
                groups: Some(vec!["G123".into(), "G123".into()]),
                ..RawFile::default()
            },
            RawFile {
                id: "F123".into(),
                ims: Some(vec!["D123".into(); MAX_FILE_CONVERSATIONS + 1]),
                ..RawFile::default()
            },
            RawFile {
                id: "F123".into(),
                alt_txt: Some("bad\0alt".into()),
                ..RawFile::default()
            },
            RawFile {
                id: "F123".into(),
                alt_txt: Some("a".repeat(MAX_FILE_UPLOAD_ALT_TEXT_BYTES + 1)),
                ..RawFile::default()
            },
        ] {
            assert!(matches!(
                normalize_file(raw, "files.info"),
                Err(Error::InvalidResponse {
                    method: "files.info"
                })
            ));
        }

        for (has_more_shares, skipped_shares) in [(Some(true), None), (None, Some(true))] {
            let truncated = normalize_file(
                RawFile {
                    id: "F123".into(),
                    shares: Some(crate::model::RawFileShares {
                        private: BTreeMap::from([(
                            "C123".into(),
                            vec![crate::model::RawFileShare {
                                ts: "100.000001".into(),
                                thread_ts: None,
                            }],
                        )]),
                        ..crate::model::RawFileShares::default()
                    }),
                    has_more_shares,
                    skipped_shares,
                    ..RawFile::default()
                },
                "files.info",
            )
            .unwrap();
            assert_eq!(truncated.shares.as_ref().unwrap().len(), 1);
            assert!(!truncated.shares_complete);
        }
    }

    #[tokio::test]
    async fn download_requires_exact_metadata_size_before_atomic_commit() {
        let directory = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "lurkline-service-download-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
        std::fs::create_dir(&directory).unwrap();
        let root = crate::local_file::McpFileRoot::open(&directory).unwrap();

        let file = FileReference {
            id: "F123".into(),
            name: Some("note.txt".into()),
            title: Some("Note".into()),
            alt_text: None,
            mimetype: Some("text/plain".into()),
            filetype: Some("text".into()),
            pretty_type: Some("Plain Text".into()),
            mode: Some("hosted".into()),
            file_access: Some("visible".into()),
            uploader_id: Some("U123".into()),
            size: Some(4),
            created: Some(1),
            timestamp: Some(2),
            editable: Some(false),
            is_external: Some(false),
            is_public: Some(false),
            public_url_shared: Some(false),
            private_url: None,
            download_url: Some("https://files.slack.com/download".into()),
            permalink: None,
            channel_ids: None,
            group_ids: None,
            im_ids: None,
            shares: Some(vec![]),
            shares_complete: true,
        };
        let target = root
            .prepare_download(std::path::Path::new("output"), 10)
            .unwrap();
        let report = service(fake_api())
            .download_file(file.clone(), target, "output".into())
            .await
            .unwrap();
        assert_eq!(report.bytes_written, 4);
        assert_eq!(std::fs::read(directory.join("output")).unwrap(), b"safe");

        for (path, mode, file_access) in [
            ("missing-mode", None, Some("visible".into())),
            ("missing-file-access", Some("hosted".into()), None),
        ] {
            let mut legacy = file.clone();
            legacy.mode = mode;
            legacy.file_access = file_access;
            let target = root
                .prepare_download(std::path::Path::new(path), 10)
                .unwrap();
            let report = service(fake_api())
                .download_file(legacy, target, path.into())
                .await
                .unwrap();
            assert_eq!(report.bytes_written, 4);
            assert_eq!(std::fs::read(directory.join(path)).unwrap(), b"safe");
        }

        let mut mismatch = fake_api();
        mismatch.download_bytes = b"short".to_vec();
        let target = root
            .prepare_download(std::path::Path::new("mismatch"), 10)
            .unwrap();
        assert!(matches!(
            service(mismatch)
                .download_file(file.clone(), target, "mismatch".into())
                .await,
            Err(Error::FileDownloadSizeMismatch {
                expected: 4,
                actual: 5
            })
        ));
        assert!(!directory.join("mismatch").exists());

        let mut unsupported = file.clone();
        unsupported.mode = Some("external".into());
        unsupported.is_external = Some(true);
        let target = root
            .prepare_download(std::path::Path::new("unsupported"), 10)
            .unwrap();
        assert!(matches!(
            service(fake_api())
                .download_file(unsupported, target, "unsupported".into())
                .await,
            Err(Error::Unsupported {
                resource: "non-hosted Slack file downloads"
            })
        ));
        assert!(!directory.join("unsupported").exists());

        let mut inaccessible = file.clone();
        inaccessible.file_access = Some("check_file_info".into());
        let target = root
            .prepare_download(std::path::Path::new("inaccessible"), 10)
            .unwrap();
        assert!(matches!(
            service(fake_api())
                .download_file(inaccessible, target, "inaccessible".into())
                .await,
            Err(Error::Authorization {
                resource: "the requested Slack file"
            })
        ));
        assert!(!directory.join("inaccessible").exists());

        let mut oversized = file.clone();
        oversized.size = Some(MAX_FILE_DOWNLOAD_BYTES + 1);
        let target = root
            .prepare_download(std::path::Path::new("oversized"), 10)
            .unwrap();
        assert!(matches!(
            service(fake_api())
                .download_file(oversized, target, "oversized".into())
                .await,
            Err(Error::InvalidInput {
                field: "max_bytes",
                reason: "Slack file is larger than the 1 GiB hard limit"
            })
        ));
        assert!(!directory.join("oversized").exists());

        let mut unknown_size = file.clone();
        unknown_size.size = None;
        let target = root
            .prepare_download(std::path::Path::new("unknown-size"), 10)
            .unwrap();
        assert!(matches!(
            service(fake_api())
                .download_file(unknown_size, target, "unknown-size".into())
                .await,
            Err(Error::NotFound {
                resource: "Slack file size"
            })
        ));
        assert!(!directory.join("unknown-size").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn upload_verifies_exact_root_and_thread_shares() {
        for thread_ts in [None, Some("100.000001")] {
            let fixture = UploadFixture::new(b"synthetic");
            let mut api = fake_api();
            api.file_response = RawFileResponse {
                file: uploaded_file(thread_ts),
            };
            if thread_ts.is_some() {
                allow_upload_thread(&mut api);
            }
            let calls = api.upload_calls.clone();
            let message_list_calls = api.message_list_calls.clone();
            let report = service(api)
                .upload_file(
                    "C123",
                    thread_ts,
                    Some("Synthetic"),
                    Some("Synthetic test file"),
                    fixture.source(),
                    true,
                )
                .await
                .unwrap();

            let FileUploadReport::Shared {
                file,
                share,
                reconciled,
            } = report
            else {
                panic!("an exact files.info share must prove success");
            };
            assert_eq!(file.id, "FUPLOAD");
            assert_eq!(share.channel_id, "C123");
            assert_eq!(share.thread_ts.as_deref(), thread_ts);
            assert!(!reconciled);
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["allocate", "transfer", "complete"]
            );
            let message_list_calls = message_list_calls.lock().unwrap();
            if thread_ts.is_some() {
                assert_eq!(
                    message_list_calls.as_slice(),
                    [("C123".into(), "100.000001".into())]
                );
            } else {
                assert!(message_list_calls.is_empty());
            }
        }
    }

    #[tokio::test]
    async fn upload_verifies_dm_root_and_thread_with_membership_and_exact_messages() {
        for thread_ts in [None, Some("100.000001")] {
            let fixture = UploadFixture::new(b"synthetic");
            let mut api = fake_api();
            api.file_response = RawFileResponse {
                file: dm_uploaded_file(Some(vec!["D123".into()])),
            };
            let message = upload_message("200.000001", thread_ts);
            if thread_ts.is_some() {
                allow_upload_thread(&mut api);
                api.replies.messages = vec![message];
            } else {
                api.history.messages = vec![message];
            }
            let history_calls = api.history_calls.clone();
            let reply_calls = api.reply_calls.clone();
            let report = service(api)
                .upload_file(
                    "D123",
                    thread_ts,
                    Some("Synthetic"),
                    Some("Synthetic test file"),
                    fixture.source(),
                    true,
                )
                .await
                .unwrap();

            let FileUploadReport::Shared {
                file,
                share,
                reconciled,
            } = report
            else {
                panic!("exact DM membership and message state must prove success");
            };
            assert_eq!(file.im_ids, Some(vec!["D123".into()]));
            assert_eq!(share.visibility, FileShareVisibility::Private);
            assert_eq!(share.channel_id, "D123");
            assert_eq!(share.ts, "200.000001");
            assert_eq!(share.thread_ts.as_deref(), thread_ts);
            assert!(!reconciled);
            assert_eq!(
                history_calls.lock().unwrap().len(),
                usize::from(thread_ts.is_none())
            );
            assert_eq!(
                reply_calls.lock().unwrap().len(),
                usize::from(thread_ts.is_some())
            );
        }
    }

    #[tokio::test]
    async fn upload_classifies_dm_thread_parents_as_roots_not_replies() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: dm_uploaded_file(Some(vec!["D123".into()])),
        };
        api.history.messages = vec![upload_message("200.000001", Some("200.000001"))];
        let report = service(api)
            .upload_file("D123", None, None, None, fixture.source(), true)
            .await
            .unwrap();
        let FileUploadReport::Shared { share, .. } = report else {
            panic!("a self-threaded parent must prove a root upload");
        };
        assert_eq!(share.ts, "200.000001");
        assert_eq!(share.thread_ts, None);

        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: dm_uploaded_file(Some(vec!["D123".into()])),
        };
        allow_upload_thread(&mut api);
        api.replies.messages = vec![upload_message("100.000001", Some("100.000001"))];
        assert_eq!(
            service(api)
                .upload_file(
                    "D123",
                    Some("100.000001"),
                    None,
                    None,
                    fixture.source(),
                    true
                )
                .await
                .unwrap(),
            FileUploadReport::CompletionUncertain {
                file_id: "FUPLOAD".into()
            }
        );
    }

    #[test]
    fn file_route_proof_distinguishes_root_parents_from_replies() {
        assert!(is_exact_file_route("200.000001", None, None));
        assert!(is_exact_file_route("200.000001", Some("200.000001"), None));
        assert!(!is_exact_file_route(
            "100.000001",
            Some("100.000001"),
            Some("100.000001")
        ));
        assert!(is_exact_file_route(
            "200.000001",
            Some("100.000001"),
            Some("100.000001")
        ));
    }

    #[tokio::test]
    async fn upload_rejects_unproven_or_ambiguous_dm_shares() {
        for (im_ids, messages) in [
            (None, vec![upload_message("200.000001", None)]),
            (
                Some(vec!["DOTHER".into()]),
                vec![upload_message("200.000001", None)],
            ),
            (Some(vec!["D123".into()]), vec![]),
            (
                Some(vec!["D123".into()]),
                vec![
                    upload_message("200.000001", None),
                    upload_message("201.000001", None),
                ],
            ),
            (
                Some(vec!["D123".into()]),
                vec![upload_message("200.000001", Some("100.000001"))],
            ),
        ] {
            let fixture = UploadFixture::new(b"synthetic");
            let mut api = fake_api();
            api.file_response = RawFileResponse {
                file: dm_uploaded_file(im_ids),
            };
            api.history.messages = messages;
            assert_eq!(
                service(api)
                    .upload_file("D123", None, None, None, fixture.source(), true)
                    .await
                    .unwrap(),
                FileUploadReport::CompletionUncertain {
                    file_id: "FUPLOAD".into()
                }
            );
        }
    }

    #[tokio::test]
    async fn upload_rejects_unresolved_thread_targets_before_allocation() {
        let cases = [
            RawMessagesList::default(),
            RawMessagesList {
                messages: BTreeMap::from([(
                    "reply".into(),
                    RawMessage {
                        ts: "100.000001".into(),
                        thread_ts: Some("99.000001".into()),
                        ..RawMessage::default()
                    },
                )]),
                ..RawMessagesList::default()
            },
            RawMessagesList {
                messages: BTreeMap::from([(
                    "malformed".into(),
                    RawMessage {
                        ts: "malformed".into(),
                        ..RawMessage::default()
                    },
                )]),
                ..RawMessagesList::default()
            },
        ];

        for (index, message_list) in cases.into_iter().enumerate() {
            let fixture = UploadFixture::new(b"synthetic");
            let mut api = fake_api();
            api.message_list = message_list;
            let upload_calls = api.upload_calls.clone();
            let message_list_calls = api.message_list_calls.clone();
            let result = service(api)
                .upload_file(
                    "C123",
                    Some("100.000001"),
                    None,
                    None,
                    fixture.source(),
                    true,
                )
                .await;
            match index {
                0 => assert!(matches!(result, Err(Error::NotFound { .. }))),
                1 => assert!(matches!(
                    result,
                    Err(Error::InvalidInput {
                        field: "thread_ts",
                        ..
                    })
                )),
                _ => assert!(matches!(
                    result,
                    Err(Error::InvalidResponse {
                        method: "messages.list"
                    })
                )),
            }
            assert!(upload_calls.lock().unwrap().is_empty());
            assert_eq!(
                message_list_calls.lock().unwrap().as_slice(),
                [("C123".into(), "100.000001".into())]
            );
        }
    }

    #[tokio::test]
    async fn upload_dm_thread_verification_is_bounded_and_cursor_safe() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: dm_uploaded_file(Some(vec!["D123".into()])),
        };
        allow_upload_thread(&mut api);
        api.reply_pages.get_mut().unwrap().extend([
            RawMessagePage {
                messages: vec![],
                has_more: true,
                response_metadata: RawResponseMetadata {
                    next_cursor: "page-2".into(),
                },
            },
            RawMessagePage {
                messages: vec![upload_message("200.000001", Some("100.000001"))],
                ..RawMessagePage::default()
            },
        ]);
        let reply_calls = api.reply_calls.clone();
        assert!(matches!(
            service(api)
                .upload_file(
                    "D123",
                    Some("100.000001"),
                    None,
                    None,
                    fixture.source(),
                    true
                )
                .await
                .unwrap(),
            FileUploadReport::Shared { .. }
        ));
        assert_eq!(
            reply_calls.lock().unwrap().as_slice(),
            [
                ReplyCall {
                    channel: "D123".into(),
                    thread_ts: "100.000001".into(),
                    cursor: None,
                    limit: MAX_MESSAGES,
                },
                ReplyCall {
                    channel: "D123".into(),
                    thread_ts: "100.000001".into(),
                    cursor: Some("page-2".into()),
                    limit: MAX_MESSAGES,
                },
            ]
        );

        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: dm_uploaded_file(Some(vec!["D123".into()])),
        };
        allow_upload_thread(&mut api);
        api.replies = RawMessagePage {
            messages: vec![upload_message("200.000001", Some("100.000001"))],
            has_more: true,
            response_metadata: RawResponseMetadata {
                next_cursor: "loop".into(),
            },
        };
        assert_eq!(
            service(api)
                .upload_file(
                    "D123",
                    Some("100.000001"),
                    None,
                    None,
                    fixture.source(),
                    true
                )
                .await
                .unwrap(),
            FileUploadReport::CompletionUncertain {
                file_id: "FUPLOAD".into()
            }
        );
    }

    #[tokio::test]
    async fn upload_returns_secret_safe_recovery_stages_without_blind_retries() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.upload_allocation_error = Some("timeout");
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::AllocationUncertain
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate"]);

        let mut api = fake_api();
        api.upload_allocation.file_id = Some("invalid".into());
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::AllocationUncertain
        );

        let mut api = fake_api();
        api.upload_allocation_error = Some("denied");
        assert!(matches!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await,
            Err(Error::SlackApi {
                method: "files.getUploadURL",
                ..
            })
        ));

        let mut api = fake_api();
        api.upload_allocation.upload_url = None;
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::Allocated {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate"]);

        let mut api = fake_api();
        api.upload_allocation.upload_url = Some("https://example.com/upload/v1/FUPLOAD".into());
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::Allocated {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate"]);

        let mut api = fake_api();
        api.upload_transfer_error = true;
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::TransferUncertain {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate", "transfer"]);

        let mut api = fake_api();
        api.upload_transfer_invalid_ack = true;
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::TransferUncertain {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate", "transfer"]);

        let mut api = fake_api();
        api.upload_mutate_pass = true;
        let calls = api.upload_calls.clone();
        assert_eq!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::SourceChanged {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(calls.lock().unwrap().as_slice(), ["allocate", "transfer"]);
    }

    #[tokio::test]
    async fn upload_reconciles_ambiguous_completion_once_with_exact_file_state() {
        for malformed_ack in [false, true] {
            let fixture = UploadFixture::new(b"synthetic");
            let mut api = fake_api();
            api.file_response = RawFileResponse {
                file: uploaded_file(None),
            };
            if malformed_ack {
                api.upload_completion.files = vec![RawFile {
                    id: "FOTHER".into(),
                    ..RawFile::default()
                }];
            } else {
                api.upload_completion_error = true;
            }
            let calls = api.upload_calls.clone();
            let report = service(api)
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap();

            assert!(matches!(
                report,
                FileUploadReport::Shared {
                    reconciled: true,
                    ..
                }
            ));
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                ["allocate", "transfer", "complete"]
            );
        }

        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: RawFile {
                id: "FUPLOAD".into(),
                shares: Some(crate::model::RawFileShares::default()),
                ..RawFile::default()
            },
        };
        let report = service(api)
            .upload_file("C123", None, None, None, fixture.source(), true)
            .await
            .unwrap();
        assert_eq!(
            report,
            FileUploadReport::CompletionUncertain {
                file_id: "FUPLOAD".into()
            }
        );
    }

    #[tokio::test]
    async fn upload_waits_for_exact_eventually_consistent_alt_text_and_share() {
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        let mut pending = uploaded_file(None);
        pending.alt_txt = None;
        api.file_responses.get_mut().unwrap().extend([
            RawFileResponse { file: pending },
            RawFileResponse {
                file: uploaded_file(None),
            },
        ]);
        let mut service = service(api);
        service.upload_reconciliation_delays_ms = &[0, 0];

        let report = service
            .upload_file(
                "C123",
                None,
                Some("Synthetic"),
                Some("Synthetic test file"),
                fixture.source(),
                true,
            )
            .await
            .unwrap();

        let FileUploadReport::Shared { file, .. } = report else {
            panic!("the second exact read must prove the processed upload");
        };
        assert_eq!(file.alt_text.as_deref(), Some("Synthetic test file"));
    }

    #[test]
    fn upload_and_draft_reconciliation_schedules_are_bounded() {
        assert_eq!(UPLOAD_RECONCILIATION_DELAYS_MS.len(), 6);
        assert_eq!(UPLOAD_RECONCILIATION_DELAYS_MS.iter().sum::<u64>(), 3_850);
        assert_eq!(UPLOAD_RECONCILIATION_DELAYS_MS.first(), Some(&0));
        assert_eq!(DRAFT_RECONCILIATION_DELAYS_MS.len(), 6);
        assert_eq!(DRAFT_RECONCILIATION_DELAYS_MS.iter().sum::<u64>(), 7_750);
        assert_eq!(DRAFT_RECONCILIATION_DELAYS_MS.first(), Some(&0));
    }

    #[tokio::test]
    async fn upload_stops_after_six_unproved_reads_without_repeating_completion() {
        const SIX_IMMEDIATE_READS: &[u64] = &[0; 6];

        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        api.file_response = RawFileResponse {
            file: RawFile {
                id: "FUPLOAD".into(),
                shares: Some(crate::model::RawFileShares::default()),
                ..RawFile::default()
            },
        };
        let file_info_calls = api.file_info_calls.clone();
        let upload_calls = api.upload_calls.clone();
        let mut service = service(api);
        service.upload_reconciliation_delays_ms = SIX_IMMEDIATE_READS;

        assert_eq!(
            service
                .upload_file("C123", None, None, None, fixture.source(), true)
                .await
                .unwrap(),
            FileUploadReport::CompletionUncertain {
                file_id: "FUPLOAD".into()
            }
        );
        assert_eq!(file_info_calls.lock().unwrap().len(), 6);
        assert_eq!(
            upload_calls.lock().unwrap().as_slice(),
            ["allocate", "transfer", "complete"]
        );
    }

    #[tokio::test]
    async fn upload_confirmation_precedes_all_slack_mutations() {
        let fixture = UploadFixture::new(b"synthetic");
        let api = fake_api();
        let calls = api.upload_calls.clone();
        assert!(matches!(
            service(api)
                .upload_file("C123", None, None, None, fixture.source(), false)
                .await,
            Err(Error::ConfirmationRequired {
                action: "file upload"
            })
        ));
        assert!(calls.lock().unwrap().is_empty());
    }

    #[test]
    fn upload_request_validation_is_pure_and_enforces_exact_metadata_bounds() {
        let file_name = "f".repeat(MAX_FILE_UPLOAD_NAME_BYTES);
        let title = "t".repeat(MAX_FILE_UPLOAD_TITLE_BYTES);
        let alt_text = "a".repeat(MAX_FILE_UPLOAD_ALT_TEXT_BYTES);
        assert!(
            SlackService::validate_upload_request(
                "C123",
                Some("100.000001"),
                Some(&title),
                Some(&alt_text),
                &file_name,
            )
            .is_ok()
        );

        for (field, result) in [
            (
                "conversation",
                SlackService::validate_upload_request("\n", None, None, None, "source.txt"),
            ),
            (
                "thread_ts",
                SlackService::validate_upload_request(
                    "C123",
                    Some("invalid"),
                    None,
                    None,
                    "source.txt",
                ),
            ),
            (
                "filename",
                SlackService::validate_upload_request(
                    "C123",
                    None,
                    None,
                    None,
                    &"f".repeat(MAX_FILE_UPLOAD_NAME_BYTES + 1),
                ),
            ),
            (
                "title",
                SlackService::validate_upload_request(
                    "C123",
                    None,
                    Some(&"t".repeat(MAX_FILE_UPLOAD_TITLE_BYTES + 1)),
                    None,
                    "source.txt",
                ),
            ),
            (
                "alt_text",
                SlackService::validate_upload_request(
                    "C123",
                    None,
                    None,
                    Some(&"a".repeat(MAX_FILE_UPLOAD_ALT_TEXT_BYTES + 1)),
                    "source.txt",
                ),
            ),
        ] {
            assert!(
                matches!(result, Err(Error::InvalidInput { field: actual, .. }) if actual == field)
            );
        }

        for (field, value) in [
            ("filename", "bad\nname"),
            ("title", "bad\ttitle"),
            ("alt_text", "bad\0alt"),
        ] {
            let result = match field {
                "filename" => {
                    SlackService::validate_upload_request("C123", None, None, None, value)
                }
                "title" => SlackService::validate_upload_request(
                    "C123",
                    None,
                    Some(value),
                    None,
                    "source.txt",
                ),
                "alt_text" => SlackService::validate_upload_request(
                    "C123",
                    None,
                    None,
                    Some(value),
                    "source.txt",
                ),
                _ => unreachable!(),
            };
            assert!(
                matches!(result, Err(Error::InvalidInput { field: actual, .. }) if actual == field)
            );
        }
    }

    #[tokio::test]
    async fn reaction_mutations_require_confirmation_are_idempotent_and_reconcile_ambiguity() {
        let api = fake_api();
        let calls = api.reaction_calls.clone();
        let state = api.reaction_present.clone();
        let slack = service(api);
        assert!(matches!(
            slack
                .add_reaction("C123", "100.000001", "eyes", false)
                .await,
            Err(Error::ConfirmationRequired { .. })
        ));
        assert!(calls.lock().unwrap().is_empty());

        let added = slack
            .add_reaction("C123", "100.000001", ":eyes:", true)
            .await
            .unwrap();
        assert!(added.target_present);
        assert!(added.present);
        assert!(added.changed);
        assert!(!added.reconciled);
        assert_eq!(&*calls.lock().unwrap(), &["get", "add", "get"]);

        calls.lock().unwrap().clear();
        let unchanged = slack
            .add_reaction("C123", "100.000001", "eyes", true)
            .await
            .unwrap();
        assert!(unchanged.target_present);
        assert!(!unchanged.changed);
        assert_eq!(&*calls.lock().unwrap(), &["get"]);
        assert!(*state.lock().unwrap());

        let mut already = fake_api();
        already.reaction_error = Some("already_reacted");
        let calls = already.reaction_calls.clone();
        let report = service(already)
            .add_reaction("C123", "100.000001", "eyes", true)
            .await
            .unwrap();
        assert!(report.target_present);
        assert!(report.present);
        assert!(!report.changed);
        assert!(report.reconciled);
        assert_eq!(&*calls.lock().unwrap(), &["get", "add"]);

        let mut no_reaction = fake_api();
        *no_reaction.reaction_present.lock().unwrap() = true;
        no_reaction.reaction_error = Some("no_reaction");
        let calls = no_reaction.reaction_calls.clone();
        let report = service(no_reaction)
            .remove_reaction("C123", "100.000001", "eyes", true)
            .await
            .unwrap();
        assert!(!report.target_present);
        assert!(!report.present);
        assert!(!report.changed);
        assert!(report.reconciled);
        assert_eq!(&*calls.lock().unwrap(), &["get", "remove"]);

        let skin_tone = service(fake_api())
            .add_reaction("C123", "100.000001", ":thumbsup::skin-tone-6:", true)
            .await
            .unwrap();
        assert_eq!(skin_tone.name, "thumbsup::skin-tone-6");
        assert!(skin_tone.present);
        let skin_api = fake_api();
        *skin_api.reaction_present.lock().unwrap() = true;
        *skin_api.reaction_name.lock().unwrap() = "thumbsup::skin-tone-6".into();
        let removed_skin_tone = service(skin_api)
            .remove_reaction("C123", "100.000001", ":thumbsup::skin-tone-6:", true)
            .await
            .unwrap();
        assert_eq!(removed_skin_tone.name, "thumbsup::skin-tone-6");
        assert!(!removed_skin_tone.present);

        for ambiguous_error in [
            "timeout",
            "invalid_response",
            "fatal_error",
            "internal_error",
        ] {
            let mut applied = fake_api();
            applied.reaction_error = Some(ambiguous_error);
            applied.reaction_apply_before_error = true;
            let reconciled = service(applied)
                .add_reaction("C123", "100.000001", "eyes", true)
                .await
                .unwrap();
            assert!(reconciled.target_present);
            assert!(reconciled.present);
            assert!(reconciled.changed);
            assert!(reconciled.reconciled);
        }

        for ambiguous_error in [
            "timeout",
            "invalid_response",
            "fatal_error",
            "internal_error",
        ] {
            let mut not_applied = fake_api();
            not_applied.reaction_error = Some(ambiguous_error);
            assert!(matches!(
                service(not_applied)
                    .add_reaction("C123", "100.000001", "eyes", true)
                    .await,
                Err(Error::ReactionNotApplied {
                    channel_id,
                    message_ts,
                    name
                }) if channel_id == "C123"
                    && message_ts == "100.000001"
                    && name == "eyes"
            ));

            let mut unreadable = fake_api();
            unreadable.reaction_error = Some(ambiguous_error);
            unreadable.reaction_apply_before_error = true;
            unreadable.reaction_get_error_after = Some(1);
            assert!(matches!(
                service(unreadable)
                    .add_reaction("C123", "100.000001", "eyes", true)
                    .await,
                Err(Error::ReactionUncertain {
                    channel_id,
                    message_ts,
                    name
                }) if channel_id == "C123"
                    && message_ts == "100.000001"
                && name == "eyes"
            ));
        }

        for malformed_kind in 0..3 {
            let mut malformed = fake_api();
            match malformed_kind {
                0 => malformed.reaction_wrong_channel_after = Some(0),
                1 => malformed.reaction_wrong_type_after = Some(0),
                2 => {
                    *malformed.reaction_present.lock().unwrap() = true;
                    malformed.reaction_duplicate_after = Some(0);
                }
                _ => unreachable!(),
            }
            let calls = malformed.reaction_calls.clone();
            assert!(matches!(
                service(malformed)
                    .add_reaction("C123", "100.000001", "eyes", true)
                    .await,
                Err(Error::InvalidResponse {
                    method: "reactions.get"
                })
            ));
            assert_eq!(&*calls.lock().unwrap(), &["get"]);
        }

        for malformed_kind in 0..3 {
            let mut malformed = fake_api();
            malformed.reaction_error = Some("timeout");
            malformed.reaction_apply_before_error = true;
            match malformed_kind {
                0 => malformed.reaction_wrong_channel_after = Some(1),
                1 => malformed.reaction_wrong_type_after = Some(1),
                2 => malformed.reaction_duplicate_after = Some(1),
                _ => unreachable!(),
            }
            assert!(matches!(
                service(malformed)
                    .add_reaction("C123", "100.000001", "eyes", true)
                    .await,
                Err(Error::ReactionUncertain {
                    channel_id,
                    message_ts,
                    name
                }) if channel_id == "C123"
                    && message_ts == "100.000001"
                    && name == "eyes"
            ));
        }
    }

    #[tokio::test]
    async fn local_message_truncation_always_sets_has_more() {
        let mut api = fake_api();
        api.history.messages = vec![
            raw_message("100.000001", "first"),
            raw_message("100.000002", "second"),
        ];
        api.replies = api.history.clone();
        let service = service(api);

        let channel = service.read_channel("C123", None, 1).await.unwrap();
        assert_eq!(channel.messages.len(), 1);
        assert!(channel.has_more);
        assert_eq!(channel.next_cursor, None);

        let thread = service
            .read_thread("C123", "100.000001", None, 1)
            .await
            .unwrap();
        assert_eq!(thread.messages.len(), 1);
        assert!(thread.has_more);
        assert_eq!(thread.next_cursor, None);
    }

    #[tokio::test]
    async fn channel_and_thread_reads_forward_cursors_and_reject_repetition() {
        let mut api = fake_api();
        api.history.response_metadata.next_cursor = "channel-next".into();
        api.replies.response_metadata.next_cursor = "thread-next".into();
        let history_calls = api.history_calls.clone();
        let reply_calls = api.reply_calls.clone();
        let subject = service(api);

        subject
            .read_channel("C123", Some("channel-current"), 3)
            .await
            .unwrap();
        subject
            .read_thread("C123", "100.000001", Some("thread-current"), 4)
            .await
            .unwrap();

        assert_eq!(
            *history_calls.lock().unwrap(),
            vec![HistoryCall {
                channel: "C123".into(),
                cursor: Some("channel-current".into()),
                limit: 3,
            }]
        );
        assert_eq!(
            *reply_calls.lock().unwrap(),
            vec![ReplyCall {
                channel: "C123".into(),
                thread_ts: "100.000001".into(),
                cursor: Some("thread-current".into()),
                limit: 4,
            }]
        );

        let mut repeated_history = fake_api();
        repeated_history.history.response_metadata.next_cursor = "same".into();
        assert!(matches!(
            service(repeated_history)
                .read_channel("C123", Some("same"), 1)
                .await,
            Err(Error::InvalidResponse {
                method: "conversations.history"
            })
        ));

        let mut repeated_thread = fake_api();
        repeated_thread.replies.response_metadata.next_cursor = "same".into();
        assert!(matches!(
            service(repeated_thread)
                .read_thread("C123", "100.000001", Some("same"), 1)
                .await,
            Err(Error::InvalidResponse {
                method: "conversations.replies"
            })
        ));
    }

    #[tokio::test]
    async fn channel_cursor_validation_happens_before_network_io() {
        let api = fake_api();
        let history_calls = api.history_calls.clone();
        assert!(matches!(
            service(api).read_channel("C123", Some(" \t"), 1).await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
        assert!(history_calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolves_message_authors_in_one_bounded_directory_batch() {
        let mut api = fake_api();
        let first = raw_message("100.000001", "first");
        let mut second = raw_message("100.000002", "second");
        second.user = Some("U456".into());
        api.history.messages = vec![first, second];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("U123", "alice", "Alice Example"),
                raw_user("U456", "bob", "Bob Example"),
            ],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 2).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.messages[0].author_id.as_deref(), Some("U123"));
        assert_eq!(page.messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(
            page.messages[0].author_display_name.as_deref(),
            Some("Alice Example")
        );
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Directory
        );
        assert_eq!(page.messages[1].author_name.as_deref(), Some("bob"));
        assert_eq!(
            page.messages[1].author_display_name.as_deref(),
            Some("Bob Example")
        );
        assert_eq!(
            page.messages[1].author_resolution,
            AuthorResolution::Directory
        );
    }

    #[tokio::test]
    async fn resolves_thread_authors_without_per_message_requests() {
        let mut api = fake_api();
        let first = raw_message("100.000001", "root");
        let mut second = raw_message("100.000002", "reply");
        second.thread_ts = Some("100.000001".into());
        api.replies.messages = vec![first, second];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();

        let page = service(api)
            .read_thread("C123", "100.000001", None, 2)
            .await
            .unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.messages.len(), 2);
        assert!(
            page.messages
                .iter()
                .all(|message| message.author_name.as_deref() == Some("alice"))
        );
    }

    #[tokio::test]
    async fn directly_named_and_authorless_messages_skip_auxiliary_resolution() {
        let mut api = fake_api();
        let mut provided = raw_message("100.000001", "provided");
        provided.user = None;
        provided.bot_id = Some("B123".into());
        provided.username = Some("build-bot".into());
        let authorless = RawMessage {
            ts: "100.000002".into(),
            text: "system event".into(),
            ..RawMessage::default()
        };
        api.history.messages = vec![provided, authorless];
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 2).await.unwrap();

        assert!(user_calls.lock().unwrap().is_empty());
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Provided
        );
        assert_eq!(page.messages[0].author_name.as_deref(), Some("build-bot"));
        assert_eq!(
            page.messages[1].author_resolution,
            AuthorResolution::Unknown
        );
        assert_eq!(page.messages[1].author_id, None);
    }

    #[tokio::test]
    async fn auxiliary_author_failure_preserves_an_addressable_message() {
        let mut api = fake_api();
        let mut message = raw_message("100.000001", "still readable <@U456>");
        message.username = Some("unsafe\nname".into());
        api.history.messages = vec![message];
        api.user_list_error = true;
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 1).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.messages[0].text, "still readable <@U456>");
        assert_eq!(page.messages[0].rendered_text, "still readable <@U456>");
        assert_eq!(
            page.messages[0].mention_resolution,
            MentionResolution::Unavailable
        );
        assert_eq!(page.messages[0].author_id.as_deref(), Some("U123"));
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Unavailable
        );
    }

    #[tokio::test]
    async fn interrupted_directory_retains_validated_users_and_marks_only_misses_unavailable() {
        let mut api = fake_api();
        let first = raw_message("100.000001", "known before interruption");
        let mut missing = raw_message("100.000002", "not reached");
        missing.user = Some("U999".into());
        api.history.messages = vec![first, missing];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            response_metadata: RawResponseMetadata {
                next_cursor: "users-2".into(),
            },
        }]));
        api.user_list_error_after = Some(1);
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 2).await.unwrap();

        assert_eq!(
            user_calls.lock().unwrap().as_slice(),
            &[
                UserCall {
                    cursor: None,
                    limit: USERS_PAGE_SIZE,
                },
                UserCall {
                    cursor: Some("users-2".into()),
                    limit: USERS_PAGE_SIZE,
                },
            ]
        );
        assert_eq!(page.messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Directory
        );
        assert_eq!(page.messages[1].author_name, None);
        assert_eq!(
            page.messages[1].author_resolution,
            AuthorResolution::Unavailable
        );
    }

    #[tokio::test]
    async fn malformed_later_directory_page_keeps_only_earlier_validated_users() {
        let malformed_pages = [
            RawUsersPage {
                members: vec![
                    raw_user("U999", "bob", "Bob Example"),
                    raw_user("bad-id", "invalid", "Invalid"),
                ],
                ..RawUsersPage::default()
            },
            RawUsersPage {
                members: vec![raw_user("U999", "bob", "Bob Example")],
                response_metadata: RawResponseMetadata {
                    next_cursor: "users-2".into(),
                },
            },
        ];
        for malformed_page in malformed_pages {
            let mut api = fake_api();
            let first = raw_message("100.000001", "known before malformed page");
            let mut missing = raw_message("100.000002", "malformed page user");
            missing.user = Some("U999".into());
            api.history.messages = vec![first, missing];
            api.user_pages = Mutex::new(VecDeque::from([
                RawUsersPage {
                    members: vec![raw_user("U123", "alice", "Alice Example")],
                    response_metadata: RawResponseMetadata {
                        next_cursor: "users-2".into(),
                    },
                },
                malformed_page,
            ]));

            let page = service(api).read_channel("C123", None, 2).await.unwrap();

            assert_eq!(page.messages[0].author_name.as_deref(), Some("alice"));
            assert_eq!(
                page.messages[0].author_resolution,
                AuthorResolution::Directory
            );
            assert_eq!(page.messages[1].author_name, None);
            assert_eq!(
                page.messages[1].author_resolution,
                AuthorResolution::Unavailable
            );
        }
    }

    #[tokio::test]
    async fn incomplete_directory_resolves_scanned_users_and_marks_misses() {
        let mut api = fake_api();
        let first = raw_message("100.000001", "found");
        let mut missing = raw_message("100.000002", "not scanned");
        missing.user = Some("U999".into());
        api.history.messages = vec![first, missing];
        api.user_pages = Mutex::new(
            (0..MAX_USER_PAGES)
                .map(|page| RawUsersPage {
                    members: if page == 0 {
                        vec![raw_user("U123", "alice", "Alice Example")]
                    } else {
                        vec![]
                    },
                    response_metadata: RawResponseMetadata {
                        next_cursor: format!("users-{}", page + 1),
                    },
                })
                .collect(),
        );
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 2).await.unwrap();

        assert_eq!(user_calls.lock().unwrap().len(), MAX_USER_PAGES);
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Directory
        );
        assert_eq!(page.messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(
            page.messages[1].author_resolution,
            AuthorResolution::Incomplete
        );
        assert_eq!(page.messages[1].author_name, None);
    }

    #[tokio::test]
    async fn display_only_directory_identity_is_usable_and_bounded() {
        let mut api = fake_api();
        api.history.messages = vec![raw_message("100.000001", "display only")];
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "\n", "Alice Example")],
            ..RawUsersPage::default()
        }]));

        let page = service(api).read_channel("C123", None, 1).await.unwrap();

        assert_eq!(page.messages[0].author_name, None);
        assert_eq!(
            page.messages[0].author_display_name.as_deref(),
            Some("Alice Example")
        );
        assert_eq!(
            page.messages[0].author_resolution,
            AuthorResolution::Directory
        );
    }

    #[tokio::test]
    async fn unusable_supplied_author_names_fall_back_without_losing_messages() {
        let mut channel_api = fake_api();
        let mut channel_message = raw_message("100.000001", "safe primary data");
        channel_message.username = Some("bad\nname".into());
        channel_api.history.messages = vec![channel_message];
        channel_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        let channel = service(channel_api)
            .read_channel("C123", None, 1)
            .await
            .unwrap();
        assert_eq!(channel.messages[0].text, "safe primary data");
        assert_eq!(channel.messages[0].author_name.as_deref(), Some("alice"));
        assert_eq!(
            channel.messages[0].author_resolution,
            AuthorResolution::Directory
        );

        let mut search_api = fake_api();
        search_api.search = RawMessageSearchResponse {
            messages: RawMessageSearchMatches {
                matches: vec![RawMessageSearchMatch {
                    channel: RawMessageSearchChannel {
                        id: "C123".into(),
                        name: "general".into(),
                    },
                    ts: "100.000001".into(),
                    user: Some("U999".into()),
                    username: Some("x".repeat(257)),
                    text: "search result survives".into(),
                    ..RawMessageSearchMatch::default()
                }],
                total: 1,
                ..RawMessageSearchMatches::default()
            },
            ..RawMessageSearchResponse::default()
        };
        let search = service(search_api)
            .search_messages("survives", None, None, None, None, 1)
            .await
            .unwrap();
        assert_eq!(search.matches[0].text, "search result survives");
        assert_eq!(search.matches[0].author_name, None);
        assert_eq!(
            search.matches[0].author_resolution,
            AuthorResolution::Unresolved
        );
    }

    #[tokio::test]
    async fn named_channel_and_dm_reads_reuse_the_routing_directory() {
        let mut channel_api = fake_api();
        channel_api.history.messages = vec![raw_message("100.000001", "channel")];
        channel_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        channel_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("CGENERAL", "general")],
            ..RawConversationsPage::default()
        }]));
        let channel_user_calls = channel_api.user_calls.clone();
        let channel = service(channel_api)
            .read_channel("general", None, 1)
            .await
            .unwrap();
        assert_single_user_directory_call(&channel_user_calls);
        assert_eq!(channel.messages[0].author_name.as_deref(), Some("alice"));

        let mut dm_api = fake_api();
        dm_api.history.messages = vec![raw_message("100.000001", "dm")];
        dm_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        dm_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![RawConversation {
                id: "D123".into(),
                is_im: true,
                user: Some("U123".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        let dm_user_calls = dm_api.user_calls.clone();
        let dm = service(dm_api)
            .read_channel("@alice", None, 1)
            .await
            .unwrap();
        assert_single_user_directory_call(&dm_user_calls);
        assert_eq!(dm.messages[0].author_name.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn named_route_failure_remains_a_routing_error() {
        let mut api = fake_api();
        api.history.messages = vec![raw_message("100.000001", "not addressable")];
        api.user_list_error = true;
        assert!(matches!(
            service(api).read_channel("general", None, 1).await,
            Err(Error::Authentication)
        ));

        let mut later_failure = fake_api();
        later_failure.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            response_metadata: RawResponseMetadata {
                next_cursor: "users-2".into(),
            },
        }]));
        later_failure.user_list_error_after = Some(1);
        assert!(matches!(
            service(later_failure)
                .read_channel("general", None, 1)
                .await,
            Err(Error::Authentication)
        ));
    }

    #[tokio::test]
    async fn unenriched_sent_messages_report_not_attempted() {
        let sent = normalize_sent_message(
            &url::Url::parse("https://example.slack.com").unwrap(),
            "C123",
            None,
            "00000000-0000-4000-8000-000000000001".into(),
            RawPostMessageResponse {
                channel: "C123".into(),
                ts: "100.000001".into(),
                message: raw_message("100.000001", "sent <@U456>"),
            },
        )
        .unwrap();
        let sent_json = serde_json::to_value(sent).unwrap();
        assert_eq!(sent_json["message"]["author_resolution"], "not_attempted");
        assert_eq!(sent_json["message"]["text"], "sent <@U456>");
        assert_eq!(sent_json["message"]["rendered_text"], "sent <@U456>");
        assert_eq!(sent_json["message"]["mention_resolution"], "not_attempted");
        assert_eq!(sent_json["message"]["mentions"][0]["id"], "U456");
        assert_eq!(
            sent_json["message"]["permalink"],
            "https://example.slack.com/archives/C123/p100000001"
        );
        assert_eq!(sent_json["message"]["thread_root_permalink"], json!(null));
        assert_eq!(sent_json["message"]["permalink_resolution"], "complete");

        let mut reply_message = raw_message("101.000002", "reply");
        reply_message.thread_ts = Some("100.000001".into());
        let reply = normalize_sent_message(
            &url::Url::parse("https://example.slack.com").unwrap(),
            "D123",
            Some("100.000001"),
            "00000000-0000-4000-8000-000000000002".into(),
            RawPostMessageResponse {
                channel: "D123".into(),
                ts: "101.000002".into(),
                message: reply_message,
            },
        )
        .unwrap();
        assert_eq!(
            reply.message.permalink.as_deref(),
            Some(
                "https://example.slack.com/archives/D123/p101000002?thread_ts=100.000001&cid=D123"
            )
        );
        assert_eq!(
            reply.message.thread_root_permalink.as_deref(),
            Some("https://example.slack.com/archives/D123/p100000001")
        );
    }

    #[tokio::test]
    async fn gets_an_exact_message_from_hydrated_channel_data() {
        let mut api = fake_api();
        api.message_list.messages_data = BTreeMap::from([(
            "C123".into(),
            RawChannelMessages {
                messages: vec![
                    raw_message("100.000001", "wrong"),
                    raw_message("100.000002", "target"),
                ],
            },
        )]);
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("U123", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        let user_calls = api.user_calls.clone();
        let message = service(api)
            .get_message("C123", "100.000002")
            .await
            .unwrap();
        assert_eq!(message.text, "target");
        assert_single_user_directory_call(&user_calls);
        let mut expected = serde_json::json!({
            "channel_id": "C123",
            "ts": "100.000002",
            "thread_ts": null,
            "author_id": "U123",
            "author_name": "alice",
            "author_display_name": "Alice Example",
            "author_resolution": "directory",
            "text": "target",
            "blocks": null,
            "attachments": null,
            "reply_count": 0,
            "latest_reply": null,
            "reactions": [{
                "name": "eyes",
                "count": 2,
                "user_ids": [],
                "user_ids_complete": false
            }],
            "files": [{
                "id": "F123",
                "name": "note.txt",
                "title": null,
                "alt_text": null,
                "mimetype": "text/plain",
                "filetype": null,
                "pretty_type": null,
                "mode": null,
                "file_access": null,
                "uploader_id": null,
                "size": 12,
                "created": null,
                "timestamp": null,
                "editable": null,
                "is_external": null,
                "is_public": null,
                "public_url_shared": null,
                "private_url": null,
                "download_url": "https://files.slack.com/note.txt",
                "permalink": null,
                "channel_ids": null,
                "group_ids": null,
                "im_ids": null,
                "shares": null,
                "shares_complete": false
            }]
        });
        let expected = expected.as_object_mut().unwrap();
        expected.insert(
            "permalink".into(),
            json!("https://example.slack.com/archives/C123/p100000002"),
        );
        expected.insert("thread_root_permalink".into(), json!(null));
        expected.insert("permalink_resolution".into(), json!("complete"));
        expected.insert("rendered_text".into(), json!("target"));
        expected.insert("mention_resolution".into(), json!("not_needed"));
        expected.insert("mentions".into(), json!([]));
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::Value::Object(expected.clone())
        );
    }

    #[tokio::test]
    async fn exact_message_accepts_identical_duplicate_representations() {
        let mut api = fake_api();
        let message = raw_message("100.000002", "target");
        api.message_list.messages = BTreeMap::from([("target".into(), message.clone())]);
        api.message_list.messages_data = BTreeMap::from([(
            "C123".into(),
            RawChannelMessages {
                messages: vec![message],
            },
        )]);

        let message = service(api)
            .get_message("C123", "100.000002")
            .await
            .unwrap();
        assert_eq!(message.text, "target");
        assert_eq!(message.files.len(), 1);
    }

    #[tokio::test]
    async fn exact_message_rejects_conflicting_duplicate_representations() {
        let mut api = fake_api();
        let sparse = raw_message("100.000002", "target");
        let mut hydrated = sparse.clone();
        hydrated.attachments = Some(vec![serde_json::json!({
            "fallback": "synthetic attachment"
        })]);
        api.message_list.messages = BTreeMap::from([("target".into(), sparse)]);
        api.message_list.messages_data = BTreeMap::from([(
            "C123".into(),
            RawChannelMessages {
                messages: vec![hydrated],
            },
        )]);

        assert!(matches!(
            service(api).get_message("C123", "100.000002").await,
            Err(Error::InvalidResponse {
                method: "messages.list"
            })
        ));
    }

    #[tokio::test]
    async fn gets_an_exact_message_from_top_level_data() {
        let mut api = fake_api();
        api.message_list.messages = BTreeMap::from([
            ("first".into(), raw_message("100.000001", "wrong")),
            ("target".into(), raw_message("100.000002", "target")),
        ]);
        let message = service(api)
            .get_message("C123", "100.000002")
            .await
            .unwrap();
        assert_eq!(message.text, "target");
    }

    #[tokio::test]
    async fn reports_missing_exact_message() {
        assert!(matches!(
            service(fake_api()).get_message("C123", "100.000002").await,
            Err(Error::NotFound { .. })
        ));
    }

    #[tokio::test]
    async fn searches_users_across_pages_and_reports_truncation() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage {
                members: vec![raw_user("U1", "alpha", "No Match")],
                response_metadata: RawResponseMetadata {
                    next_cursor: "cursor-2".into(),
                },
            },
            RawUsersPage {
                members: vec![
                    raw_user("U2", "one", "Target One"),
                    raw_user("U3", "two", "Target Two"),
                ],
                ..RawUsersPage::default()
            },
        ]));
        let report = service(api).find_users(" target ", 1).await.unwrap();
        assert_eq!(report.query, "target");
        assert_eq!(report.users[0].id, "U2");
        assert!(report.truncated);
    }

    #[tokio::test]
    async fn accepts_enterprise_w_user_ids_whether_or_not_they_match() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("W123ABC", "unrelated", "No Match"),
                raw_user("W456DEF", "target", "Target Enterprise User"),
            ],
            ..RawUsersPage::default()
        }]));
        let report = service(api).find_users("target", 10).await.unwrap();
        assert_eq!(report.users.len(), 1);
        assert_eq!(report.users[0].id, "W456DEF");
    }

    #[tokio::test]
    async fn empty_user_search_reports_the_scan_cap_as_incomplete() {
        let mut pages = VecDeque::new();
        for page in 0..MAX_USER_PAGES {
            pages.push_back(RawUsersPage {
                members: vec![],
                response_metadata: RawResponseMetadata {
                    next_cursor: format!("cursor-{page}"),
                },
            });
        }
        let mut api = fake_api();
        api.user_pages = Mutex::new(pages);
        let report = service(api).find_users("missing", 10).await.unwrap();
        assert!(report.users.is_empty());
        assert!(report.truncated);
        assert_eq!(
            report.truncation_reason,
            Some(UserSearchTruncationReason::ScanLimit)
        );
        assert_eq!(report.scan_limit, 4_000);
    }

    #[tokio::test]
    async fn rejects_user_pages_larger_than_the_requested_bound() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: (0..=USERS_PAGE_SIZE)
                .map(|index| raw_user(&format!("U{index}"), "user", "User"))
                .collect(),
            ..RawUsersPage::default()
        }]));
        assert!(matches!(
            service(api).find_users("user", 10).await,
            Err(Error::InvalidResponse {
                method: "users.list"
            })
        ));
    }

    #[test]
    fn activity_time_parsing_is_exact_and_timezone_independent() {
        assert_eq!(
            parse_activity_duration("1d12h30m").unwrap().num_seconds(),
            131_400
        );
        assert!(parse_activity_duration("0h").is_err());
        assert!(parse_activity_duration("366d").is_err());

        let before_fallback =
            parse_activity_rfc3339("before", "2026-10-25T02:30:00+01:00").unwrap();
        let after_fallback = parse_activity_rfc3339("after", "2026-10-25T02:30:00+02:00").unwrap();
        assert_eq!(before_fallback - after_fallback, 3_600_000_000_000);
        assert_eq!(
            format_activity_instant(after_fallback).unwrap(),
            "2026-10-25T00:30:00Z"
        );

        let submicro = parse_activity_rfc3339("after", "1970-01-01T00:01:40.000000501Z").unwrap();
        assert_eq!(
            activity_slack_bounds(
                parse_activity_rfc3339("after", "1970-01-01T00:01:40Z").unwrap(),
                parse_activity_rfc3339("before", "1970-01-01T00:01:42Z").unwrap(),
            )
            .unwrap(),
            Some(("100.000000".into(), "101.999999".into()))
        );
        assert_eq!(
            activity_slack_bounds(
                submicro,
                parse_activity_rfc3339("before", "1970-01-01T00:01:42.000000501Z").unwrap(),
            )
            .unwrap(),
            Some(("100.000001".into(), "102.000000".into()))
        );
        let maximum = parse_activity_rfc3339("before", "2262-04-11T23:47:16.854775807Z").unwrap();
        assert_eq!(maximum, i64::MAX);
        assert_eq!(
            ceil_activity_microseconds(maximum).unwrap(),
            i64::MAX / 1_000 + 1
        );
        assert!(!timestamp_in_activity_interval(
            "100.000000",
            submicro,
            submicro + 1_000
        ));
        assert!(timestamp_in_activity_interval(
            "100.000001",
            submicro,
            submicro + 1_000
        ));
    }

    #[tokio::test]
    async fn activity_selects_useful_defaults_and_preserves_boundaries_replies_and_ties() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![
                raw_user("U123", "writer", "Writer"),
                raw_user("U999", "self", "Self"),
            ],
            ..RawUsersPage::default()
        }]));
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("C001", "joined"),
                RawConversation {
                    is_member: false,
                    ..raw_conversation("C002", "visible")
                },
                RawConversation {
                    id: "D001".into(),
                    is_im: true,
                    is_private: true,
                    is_member: true,
                    user: Some("U999".into()),
                    ..RawConversation::default()
                },
            ],
            ..RawConversationsPage::default()
        }]));
        api.activity_results = Mutex::new(VecDeque::from([
            Ok(RawMessagePage {
                messages: vec![
                    RawMessage {
                        thread_ts: Some("100.000000".into()),
                        ..raw_message("101.000000", "reply")
                    },
                    raw_message("100.000000", "after boundary"),
                    raw_message("102.000000", "before boundary"),
                    raw_message("99.999999", "too old"),
                ],
                has_more: true,
                ..RawMessagePage::default()
            }),
            Ok(RawMessagePage {
                messages: vec![raw_message("101.000000", "same instant")],
                ..RawMessagePage::default()
            }),
        ]));
        let activity_calls = Arc::clone(&api.activity_calls);
        let user_calls = Arc::clone(&api.user_calls);
        let report = service(api)
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(10),
                limit: Some(10),
                cursor: None,
            })
            .await
            .unwrap();

        assert_eq!(
            report
                .items
                .iter()
                .map(|item| (item.conversation_id.as_str(), item.message.ts.as_str()))
                .collect::<Vec<_>>(),
            vec![
                ("D001", "101.000000"),
                ("C001", "101.000000"),
                ("C001", "100.000000"),
            ]
        );
        assert_eq!(
            report.items[1].message.thread_ts.as_deref(),
            Some("100.000000")
        );
        assert_eq!(
            report.conversation_results[0].status,
            ActivityConversationStatus::MessageLimit
        );
        assert_eq!(report.selected_conversations, 2);
        assert_eq!(
            activity_calls.lock().unwrap().as_slice(),
            &[
                ActivityCall {
                    channel: "C001".into(),
                    oldest: "100.000000".into(),
                    latest: "101.999999".into(),
                    limit: 10,
                },
                ActivityCall {
                    channel: "D001".into(),
                    oldest: "100.000000".into(),
                    latest: "101.999999".into(),
                    limit: 10,
                },
            ]
        );
        assert_single_user_directory_call(&user_calls);
    }

    #[tokio::test]
    async fn activity_exclusive_upper_bound_cannot_consume_the_history_cap() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        }]));
        api.activity_results = Mutex::new(VecDeque::from([Ok(RawMessagePage {
            // Slack's cap of one applies after the exact request latest. A message at
            // 102.000000 is not eligible to consume this slot.
            messages: vec![raw_message("101.999999", "last included microsecond")],
            has_more: true,
            ..RawMessagePage::default()
        })]));
        let calls = Arc::clone(&api.activity_calls);
        let report = service(api)
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(report.items[0].message.ts, "101.999999");
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[ActivityCall {
                channel: "C001".into(),
                oldest: "100.000000".into(),
                latest: "101.999999".into(),
                limit: 1,
            }]
        );
    }

    #[tokio::test]
    async fn activity_skips_history_when_the_interval_contains_no_slack_microsecond() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        }]));
        let calls = Arc::clone(&api.activity_calls);
        let report = service(api)
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40.000000001Z"),
                before: Some("1970-01-01T00:01:40.000000999Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert!(calls.lock().unwrap().is_empty());
        assert!(report.items.is_empty());
        assert_eq!(
            report.conversation_results[0].status,
            ActivityConversationStatus::Complete
        );
        assert!(!report.partial);
    }

    #[tokio::test]
    async fn activity_cursor_resumes_by_key_and_ignores_newer_than_frozen_window() {
        let conversation_page = RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        };
        let first_snapshot = RawMessagePage {
            messages: vec![
                raw_message("103.000000", "third"),
                raw_message("102.000000", "second"),
                raw_message("101.000000", "first"),
            ],
            ..RawMessagePage::default()
        };
        let mut second_snapshot = first_snapshot.clone();
        second_snapshot
            .messages
            .insert(0, raw_message("105.000001", "new after frozen before"));
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([
            conversation_page.clone(),
            conversation_page,
        ]));
        api.activity_results =
            Mutex::new(VecDeque::from([Ok(first_snapshot), Ok(second_snapshot)]));
        let service = service(api);
        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:45Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(10),
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(first.items.len(), 2);
        let cursor = first.next_cursor.clone().unwrap();
        let second = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: Some(&cursor),
            })
            .await
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].message.ts, "101.000000");
        assert!(!second.has_more);

        let mut tampered = cursor.into_bytes();
        let last = tampered.len() - 1;
        tampered[last] = if tampered[last] == b'a' { b'b' } else { b'a' };
        let tampered = String::from_utf8(tampered).unwrap();
        assert!(matches!(
            service
                .activity(ActivityRequest {
                    since: None,
                    after: None,
                    before: None,
                    include: &[],
                    exclude: &[],
                    kinds: &[],
                    order: None,
                    conversation_limit: None,
                    per_conversation_limit: None,
                    limit: None,
                    cursor: Some(&tampered),
                })
                .await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
    }

    #[test]
    fn activity_cursor_round_trips_the_maximum_valid_domain() {
        let conversation_ids = (0..MAX_ACTIVITY_CONVERSATIONS)
            .map(|index| format!("C{index:02}{}", "A".repeat(61)))
            .collect::<Vec<_>>();
        let team_id = format!("T{}", "A".repeat(63));
        let cursor = ActivityCursor {
            version: ACTIVITY_CURSOR_VERSION,
            team_id: team_id.clone(),
            after_nanos: 0,
            before_nanos: i64::MAX,
            order: ActivityOrder::OldestFirst,
            conversation_kinds: normalize_activity_kinds(&[]),
            include_ids: conversation_ids.clone(),
            exclude_ids: vec![],
            conversation_limit: MAX_ACTIVITY_CONVERSATIONS,
            per_conversation_limit: MAX_ACTIVITY_PER_CONVERSATION,
            limit: MAX_ACTIVITY_MESSAGES,
            eligible_conversations: MAX_ACTIVITY_CONVERSATIONS,
            conversation_scan_truncated: true,
            scope_digest: "a".repeat(64),
            position: ActivityCursorPosition::Messages {
                scope_offset: 0,
                last_key: ActivityKey {
                    ts: format!("{}.{}", "9".repeat(15), "9".repeat(16)),
                    conversation_id: conversation_ids[0].clone(),
                },
                snapshot_digest: "b".repeat(64),
            },
        };

        let encoded = encode_activity_cursor(&cursor).unwrap();
        assert!(encoded.len() > 2_048);
        assert!(encoded.len() <= MAX_ACTIVITY_CURSOR_LENGTH);
        let decoded = decode_activity_cursor(&encoded).unwrap();
        assert_eq!(decoded, cursor);
        validate_activity_cursor(&decoded, &team_id).unwrap();

        assert!(decode_activity_cursor("activity-v1.invalid.invalid").is_err());
        let mut invalid_offset = cursor.clone();
        invalid_offset.position = ActivityCursorPosition::ConversationScope {
            scope_offset: MAX_ACTIVITY_CONVERSATIONS,
        };
        assert!(validate_activity_cursor(&invalid_offset, &team_id).is_err());
        let mut duplicate_kinds = cursor;
        duplicate_kinds.conversation_kinds =
            vec![ConversationKind::Channel, ConversationKind::Channel];
        assert!(validate_activity_cursor(&duplicate_kinds, &team_id).is_err());
    }

    #[tokio::test]
    async fn activity_preserves_successes_beside_inaccessible_conversations() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![
                raw_conversation("C001", "available"),
                raw_conversation("C002", "inaccessible"),
            ],
            ..RawConversationsPage::default()
        }]));
        api.activity_results = Mutex::new(VecDeque::from([
            Ok(RawMessagePage {
                messages: vec![raw_message("101.000000", "kept")],
                ..RawMessagePage::default()
            }),
            Err(Error::SlackApi {
                method: "conversations.history",
                code: "not_in_channel".into(),
            }),
        ]));
        let report = service(api)
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: Some(ActivityOrder::OldestFirst),
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(report.items.len(), 1);
        assert!(report.partial);
        assert_eq!(
            report.conversation_results[1].status,
            ActivityConversationStatus::Inaccessible
        );
    }

    #[tokio::test]
    async fn activity_authentication_failure_is_not_reported_as_partial() {
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        }]));
        api.activity_results = Mutex::new(VecDeque::from([Err(Error::Authentication)]));
        assert!(matches!(
            service(api)
                .activity(ActivityRequest {
                    since: Some("1h"),
                    after: None,
                    before: None,
                    include: &[],
                    exclude: &[],
                    kinds: &[],
                    order: None,
                    conversation_limit: None,
                    per_conversation_limit: None,
                    limit: None,
                    cursor: None,
                })
                .await,
            Err(Error::Authentication)
        ));
    }

    #[tokio::test]
    async fn activity_rejects_a_changed_bounded_snapshot_as_stale() {
        let conversation_page = RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        };
        let first_snapshot = RawMessagePage {
            messages: vec![
                raw_message("103.000000", "third"),
                raw_message("102.000000", "second"),
            ],
            ..RawMessagePage::default()
        };
        let mut changed_snapshot = first_snapshot.clone();
        changed_snapshot.messages[1].text = "edited".into();
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([
            conversation_page.clone(),
            conversation_page,
        ]));
        api.activity_results =
            Mutex::new(VecDeque::from([Ok(first_snapshot), Ok(changed_snapshot)]));
        let service = service(api);
        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:45Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(10),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert!(matches!(
            service
                .activity(ActivityRequest {
                    since: None,
                    after: None,
                    before: None,
                    include: &[],
                    exclude: &[],
                    kinds: &[],
                    order: None,
                    conversation_limit: None,
                    per_conversation_limit: None,
                    limit: None,
                    cursor: first.next_cursor.as_deref(),
                })
                .await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
    }

    #[tokio::test]
    async fn activity_shortens_a_page_at_the_complete_response_byte_limit() {
        let conversation_page = RawConversationsPage {
            channels: vec![raw_conversation("C001", "joined")],
            ..RawConversationsPage::default()
        };
        let snapshot = RawMessagePage {
            messages: vec![
                raw_message("103.000000", &"x".repeat(500)),
                raw_message("102.000000", &"y".repeat(500)),
                raw_message("101.000000", &"z".repeat(500)),
            ],
            ..RawMessagePage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([
            conversation_page.clone(),
            conversation_page,
        ]));
        api.activity_results = Mutex::new(VecDeque::from([Ok(snapshot.clone()), Ok(snapshot)]));
        let mut service = service(api);
        let unbounded = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:45Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(10),
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();
        let full_size = serde_json::to_vec_pretty(&unbounded).unwrap().len();
        let mut one_item = unbounded.clone();
        one_item.items.pop();
        one_item.partial = true;
        one_item.response_byte_limit_reached = true;
        let one_size = serde_json::to_vec_pretty(&one_item).unwrap().len();
        assert!(full_size > one_size);
        service.max_response_bytes = one_size;

        let bounded = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:45Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: Some(10),
                limit: Some(2),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(bounded.items.len(), 1);
        assert!(bounded.response_byte_limit_reached);
        assert!(bounded.partial);
        assert!(bounded.has_more);
        assert_eq!(
            bounded.continuation_kind,
            Some(ActivityContinuationKind::Messages)
        );
        assert!(bounded.next_cursor.is_some());
        assert!(serialized_json_fits(&bounded, one_size));
    }

    #[test]
    fn activity_selection_resolves_exact_names_and_rejects_overlap() {
        let conversations = vec![
            raw_conversation("C001", "joined"),
            RawConversation {
                is_member: false,
                ..raw_conversation("C002", "visible")
            },
        ]
        .into_iter()
        .map(|raw| {
            normalize_conversations(vec![raw], &HashMap::new())
                .unwrap()
                .remove(0)
        })
        .collect::<Vec<_>>();
        let all = ActivityConversationDirectory {
            candidates: conversations
                .into_iter()
                .map(|conversation| ActivityConversationCandidate { conversation })
                .collect(),
            scanned_conversations: 2,
            scan_truncated: false,
        };
        let kinds = normalize_activity_kinds(&[]);
        let selected = select_activity_scope(all, &kinds, &["visible".into()], &[]).unwrap();
        assert_eq!(selected.candidates[0].conversation.id, "C002");

        let all = ActivityConversationDirectory {
            candidates: selected
                .candidates
                .into_iter()
                .map(|candidate| ActivityConversationCandidate {
                    conversation: candidate.conversation,
                })
                .collect(),
            scanned_conversations: 1,
            scan_truncated: false,
        };
        assert!(matches!(
            select_activity_scope(all, &kinds, &["C002".into()], &["visible".into()]),
            Err(Error::InvalidInput {
                field: "include",
                ..
            })
        ));
    }

    #[test]
    fn activity_scope_uses_stable_id_order() {
        let candidates = (1..=12)
            .map(|index| {
                let id = format!("C{index:03}");
                let conversation = normalize_conversations(
                    vec![raw_conversation(&id, &format!("channel-{index}"))],
                    &HashMap::new(),
                )
                .unwrap()
                .remove(0);
                ActivityConversationCandidate { conversation }
            })
            .collect::<Vec<_>>();
        let selected = select_activity_scope(
            ActivityConversationDirectory {
                candidates,
                scanned_conversations: 12,
                scan_truncated: false,
            },
            &normalize_activity_kinds(&[]),
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(
            selected
                .candidates
                .iter()
                .take(2)
                .map(|candidate| candidate.conversation.id.as_str())
                .collect::<Vec<_>>(),
            vec!["C001", "C002"]
        );
        assert_eq!(selected.candidates.len(), 12);
    }

    #[tokio::test]
    async fn activity_traverses_more_than_fifty_conversations_without_overlap() {
        let conversations = (0..55)
            .map(|index| raw_conversation(&format!("C{index:03}"), &format!("channel-{index:03}")))
            .collect::<Vec<_>>();
        let first_page = RawConversationsPage {
            channels: conversations.clone(),
            ..RawConversationsPage::default()
        };
        let mut renamed_page = first_page.clone();
        renamed_page.channels[50].name = "renamed-without-changing-eligibility".into();
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([first_page, renamed_page]));
        api.activity_results = Mutex::new((0..55).map(|_| Ok(RawMessagePage::default())).collect());
        let activity_calls = Arc::clone(&api.activity_calls);
        let service = service(api);

        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[ConversationKind::Channel],
                order: None,
                conversation_limit: Some(50),
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert!(first.items.is_empty());
        assert_eq!(first.eligible_conversations, 55);
        assert_eq!(first.scope_offset, 0);
        assert_eq!(first.selected_conversations, 50);
        assert_eq!(first.remaining_conversations, 5);
        assert!(first.selection_truncated);
        assert!(first.scope_has_more);
        assert_eq!(
            first.continuation_kind,
            Some(ActivityContinuationKind::ConversationScope)
        );
        let cursor = first.next_cursor.clone().unwrap();
        assert!(matches!(
            decode_activity_cursor(&cursor).unwrap().position,
            ActivityCursorPosition::ConversationScope { scope_offset: 50 }
        ));

        let second = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: Some(&cursor),
            })
            .await
            .unwrap();
        assert!(second.items.is_empty());
        assert_eq!(second.eligible_conversations, 55);
        assert_eq!(second.scope_offset, 50);
        assert_eq!(second.selected_conversations, 5);
        assert_eq!(second.remaining_conversations, 0);
        assert!(second.selection_truncated);
        assert!(!second.scope_has_more);
        assert!(!second.has_more);
        assert_eq!(second.continuation_kind, None);

        let ids = first
            .conversation_results
            .iter()
            .chain(&second.conversation_results)
            .map(|result| result.conversation.id.clone())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 55);
        assert_eq!(ids.iter().collect::<HashSet<_>>().len(), 55);
        assert_eq!(activity_calls.lock().unwrap().len(), 55);
    }

    #[tokio::test]
    async fn activity_traverses_more_than_fifty_mixed_conversation_kinds() {
        let mut conversations = (0..20)
            .map(|index| raw_conversation(&format!("C{index:03}"), &format!("channel-{index:03}")))
            .collect::<Vec<_>>();
        conversations.extend((0..20).map(|index| RawConversation {
            id: format!("D{index:03}"),
            is_im: true,
            is_private: true,
            is_member: true,
            user: Some(format!("U{index:03}")),
            ..RawConversation::default()
        }));
        conversations.extend((0..20).map(|index| RawConversation {
            is_mpim: true,
            is_private: true,
            ..raw_conversation(&format!("G{index:03}"), &format!("mpdm-U000--U001-{index}"))
        }));
        let users = RawUsersPage {
            members: (0..20)
                .map(|index| {
                    raw_user(
                        &format!("U{index:03}"),
                        &format!("user-{index:03}"),
                        &format!("User {index:03}"),
                    )
                })
                .collect(),
            ..RawUsersPage::default()
        };
        let page = RawConversationsPage {
            channels: conversations,
            ..RawConversationsPage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([users.clone(), users]));
        api.conversation_pages = Mutex::new(VecDeque::from([page.clone(), page]));
        api.activity_results = Mutex::new((0..60).map(|_| Ok(RawMessagePage::default())).collect());
        let activity_calls = Arc::clone(&api.activity_calls);
        let service = service(api);

        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: Some(50),
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        let cursor = first.next_cursor.clone().unwrap();
        assert_eq!(
            first.continuation_kind,
            Some(ActivityContinuationKind::ConversationScope)
        );
        assert_eq!(first.eligible_conversations, 60);
        assert_eq!(first.selected_conversations, 50);
        assert_eq!(first.remaining_conversations, 10);

        let second = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: Some(&cursor),
            })
            .await
            .unwrap();
        assert_eq!(second.scope_offset, 50);
        assert_eq!(second.selected_conversations, 10);
        assert_eq!(second.remaining_conversations, 0);
        assert_eq!(second.continuation_kind, None);

        let conversations = first
            .conversation_results
            .iter()
            .chain(&second.conversation_results)
            .map(|result| (result.conversation.id.clone(), result.conversation.kind))
            .collect::<Vec<_>>();
        assert_eq!(conversations.len(), 60);
        assert_eq!(
            conversations
                .iter()
                .map(|(id, _)| id)
                .collect::<HashSet<_>>()
                .len(),
            60
        );
        assert_eq!(
            conversations
                .iter()
                .filter(|(_, kind)| *kind == ConversationKind::Channel)
                .count(),
            20
        );
        assert_eq!(
            conversations
                .iter()
                .filter(|(_, kind)| *kind == ConversationKind::DirectMessage)
                .count(),
            20
        );
        assert_eq!(
            conversations
                .iter()
                .filter(|(_, kind)| *kind == ConversationKind::GroupDirectMessage)
                .count(),
            20
        );
        assert_eq!(activity_calls.lock().unwrap().len(), 60);
    }

    #[tokio::test]
    async fn activity_filters_kinds_before_the_slice_and_rejects_mismatched_selectors() {
        let page = RawConversationsPage {
            channels: vec![
                raw_conversation("C001", "channel"),
                RawConversation {
                    id: "D001".into(),
                    is_im: true,
                    is_private: true,
                    is_member: true,
                    user: Some("U001".into()),
                    ..RawConversation::default()
                },
                RawConversation {
                    is_mpim: true,
                    is_private: true,
                    ..raw_conversation("G001", "mpdm-U001--U002-1")
                },
            ],
            ..RawConversationsPage::default()
        };
        let users = RawUsersPage {
            members: vec![
                raw_user("U001", "one", "One"),
                raw_user("U002", "two", "Two"),
            ],
            ..RawUsersPage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([users.clone(), users]));
        api.conversation_pages = Mutex::new(VecDeque::from([page.clone(), page]));
        api.activity_results = Mutex::new(VecDeque::from([Ok(RawMessagePage::default())]));
        let activity_calls = Arc::clone(&api.activity_calls);
        let service = service(api);

        let report = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[
                    ConversationKind::DirectMessage,
                    ConversationKind::DirectMessage,
                ],
                order: None,
                conversation_limit: Some(1),
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(
            report.conversation_kinds,
            vec![ConversationKind::DirectMessage]
        );
        assert_eq!(report.eligible_conversations, 1);
        assert_eq!(report.conversation_results[0].conversation.id, "D001");
        assert_eq!(activity_calls.lock().unwrap()[0].channel, "D001");

        assert!(matches!(
            service
                .activity(ActivityRequest {
                    since: None,
                    after: Some("1970-01-01T00:01:40Z"),
                    before: Some("1970-01-01T00:01:42Z"),
                    include: &["D001".into()],
                    exclude: &[],
                    kinds: &[ConversationKind::Channel],
                    order: None,
                    conversation_limit: Some(1),
                    per_conversation_limit: Some(1),
                    limit: Some(1),
                    cursor: None,
                })
                .await,
            Err(Error::InvalidInput {
                field: "include",
                ..
            })
        ));
        assert_eq!(activity_calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn activity_pages_explicit_includes_and_documents_merge_order() {
        let page = RawConversationsPage {
            channels: vec![
                raw_conversation("C001", "first"),
                raw_conversation("C002", "second"),
            ],
            ..RawConversationsPage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([page.clone(), page]));
        api.activity_results = Mutex::new(VecDeque::from([
            Ok(RawMessagePage {
                messages: vec![raw_message("101.000000", "older")],
                ..RawMessagePage::default()
            }),
            Ok(RawMessagePage {
                messages: vec![raw_message("102.000000", "newer")],
                ..RawMessagePage::default()
            }),
        ]));
        let service = service(api);
        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:43Z"),
                include: &["C002".into(), "C001".into()],
                exclude: &[],
                kinds: &[ConversationKind::Channel],
                order: None,
                conversation_limit: Some(1),
                per_conversation_limit: Some(1),
                limit: Some(10),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(first.items[0].conversation_id, "C001");
        assert_eq!(
            first.continuation_kind,
            Some(ActivityContinuationKind::ConversationScope)
        );
        let cursor = first.next_cursor.clone().unwrap();
        let decoded = decode_activity_cursor(&cursor).unwrap();
        assert_eq!(decoded.include_ids, vec!["C001", "C002"]);

        let second = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: Some(&cursor),
            })
            .await
            .unwrap();
        assert_eq!(second.items[0].conversation_id, "C002");

        let mut merged = first
            .items
            .into_iter()
            .chain(second.items)
            .collect::<Vec<_>>();
        merged.sort_by(|left, right| {
            compare_activity_keys(
                &left.message.ts,
                &left.conversation_id,
                &right.message.ts,
                &right.conversation_id,
            )
        });
        merged.reverse();
        assert_eq!(
            merged
                .iter()
                .map(|item| item.conversation_id.as_str())
                .collect::<Vec<_>>(),
            vec!["C002", "C001"]
        );
    }

    #[tokio::test]
    async fn activity_finishes_message_pages_before_advancing_the_scope() {
        let page = RawConversationsPage {
            channels: vec![
                raw_conversation("C001", "first"),
                raw_conversation("C002", "second"),
            ],
            ..RawConversationsPage::default()
        };
        let first_snapshot = RawMessagePage {
            messages: vec![
                raw_message("103.000000", &"x".repeat(500)),
                raw_message("102.000000", &"y".repeat(500)),
                raw_message("101.000000", &"z".repeat(500)),
            ],
            ..RawMessagePage::default()
        };
        let second_snapshot = RawMessagePage {
            messages: vec![raw_message("104.000000", &"w".repeat(500))],
            ..RawMessagePage::default()
        };
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
            RawUsersPage::default(),
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([
            page.clone(),
            page.clone(),
            page.clone(),
            page.clone(),
            page,
        ]));
        api.activity_results = Mutex::new(VecDeque::from([
            Ok(first_snapshot.clone()),
            Ok(first_snapshot.clone()),
            Ok(first_snapshot.clone()),
            Ok(first_snapshot),
            Ok(second_snapshot),
        ]));
        let activity_calls = Arc::clone(&api.activity_calls);
        let mut service = service(api);
        let initial_request = || ActivityRequest {
            since: None,
            after: Some("1970-01-01T00:01:40Z"),
            before: Some("1970-01-01T00:01:45Z"),
            include: &[],
            exclude: &[],
            kinds: &[ConversationKind::Channel],
            order: Some(ActivityOrder::OldestFirst),
            conversation_limit: Some(1),
            per_conversation_limit: Some(10),
            limit: Some(3),
            cursor: None,
        };

        let baseline = service.activity(initial_request()).await.unwrap();
        assert_eq!(baseline.items.len(), 3);
        assert_eq!(
            baseline.continuation_kind,
            Some(ActivityContinuationKind::ConversationScope)
        );
        let mut one_item = baseline.clone();
        one_item.items.truncate(1);
        let snapshot_digest =
            activity_snapshot_digest(&baseline.items, &baseline.conversation_results).unwrap();
        let mut cursor = decode_activity_cursor(baseline.next_cursor.as_deref().unwrap()).unwrap();
        cursor.position = ActivityCursorPosition::Messages {
            scope_offset: 0,
            last_key: activity_item_key(&one_item.items[0]),
            snapshot_digest,
        };
        one_item.next_cursor = Some(encode_activity_cursor(&cursor).unwrap());
        one_item.continuation_kind = Some(ActivityContinuationKind::Messages);
        one_item.response_byte_limit_reached = true;
        one_item.partial = true;
        let one_item_size = serde_json::to_vec_pretty(&one_item).unwrap().len();
        service.max_response_bytes = one_item_size;

        let first = service.activity(initial_request()).await.unwrap();
        assert_eq!(first.items.len(), 1);
        assert!(first.response_byte_limit_reached);
        assert_eq!(
            first.continuation_kind,
            Some(ActivityContinuationKind::Messages)
        );

        let second = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: first.next_cursor.as_deref(),
            })
            .await
            .unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(
            second.continuation_kind,
            Some(ActivityContinuationKind::Messages)
        );

        let third = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: second.next_cursor.as_deref(),
            })
            .await
            .unwrap();
        assert_eq!(third.items.len(), 1);
        assert_eq!(
            third.continuation_kind,
            Some(ActivityContinuationKind::ConversationScope)
        );

        let fourth = service
            .activity(ActivityRequest {
                since: None,
                after: None,
                before: None,
                include: &[],
                exclude: &[],
                kinds: &[],
                order: None,
                conversation_limit: None,
                per_conversation_limit: None,
                limit: None,
                cursor: third.next_cursor.as_deref(),
            })
            .await
            .unwrap();
        assert_eq!(fourth.items.len(), 1);
        assert_eq!(fourth.items[0].conversation_id, "C002");
        assert_eq!(fourth.continuation_kind, None);
        assert!(fourth.next_cursor.is_none());

        let traversed = first
            .items
            .iter()
            .chain(&second.items)
            .chain(&third.items)
            .chain(&fourth.items)
            .map(|item| (item.conversation_id.clone(), item.message.ts.clone()))
            .collect::<Vec<_>>();
        assert_eq!(traversed.len(), 4);
        assert_eq!(traversed.iter().collect::<HashSet<_>>().len(), 4);
        assert_eq!(
            traversed,
            vec![
                ("C001".into(), "101.000000".into()),
                ("C001".into(), "102.000000".into()),
                ("C001".into(), "103.000000".into()),
                ("C002".into(), "104.000000".into()),
            ]
        );
        assert_eq!(activity_calls.lock().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn activity_scope_drift_is_stale_before_history() {
        let first_conversations = (0..51)
            .map(|index| raw_conversation(&format!("C{index:03}"), &format!("channel-{index:03}")))
            .collect::<Vec<_>>();
        let mut changed_conversations = first_conversations.clone();
        changed_conversations[50] = raw_conversation("C999", "replacement");
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([
            RawUsersPage::default(),
            RawUsersPage::default(),
        ]));
        api.conversation_pages = Mutex::new(VecDeque::from([
            RawConversationsPage {
                channels: first_conversations,
                ..RawConversationsPage::default()
            },
            RawConversationsPage {
                channels: changed_conversations,
                ..RawConversationsPage::default()
            },
        ]));
        api.activity_results = Mutex::new((0..50).map(|_| Ok(RawMessagePage::default())).collect());
        let activity_calls = Arc::clone(&api.activity_calls);
        let service = service(api);
        let first = service
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[ConversationKind::Channel],
                order: None,
                conversation_limit: Some(50),
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(activity_calls.lock().unwrap().len(), 50);
        assert!(matches!(
            service
                .activity(ActivityRequest {
                    since: None,
                    after: None,
                    before: None,
                    include: &[],
                    exclude: &[],
                    kinds: &[],
                    order: None,
                    conversation_limit: None,
                    per_conversation_limit: None,
                    limit: None,
                    cursor: first.next_cursor.as_deref(),
                })
                .await,
            Err(Error::InvalidInput {
                field: "cursor",
                ..
            })
        ));
        assert_eq!(activity_calls.lock().unwrap().len(), 50);
    }

    #[tokio::test]
    async fn activity_scan_limit_is_partial_and_has_no_synthetic_continuation() {
        let pages = (0..MAX_CONVERSATION_PAGES)
            .map(|index| RawConversationsPage {
                channels: vec![raw_conversation(
                    &format!("C{index:03}"),
                    &format!("channel-{index:03}"),
                )],
                response_metadata: RawResponseMetadata {
                    next_cursor: format!("page-{index}"),
                },
            })
            .collect::<VecDeque<_>>();
        let mut api = fake_api();
        api.user_pages = Mutex::new(VecDeque::from([RawUsersPage::default()]));
        api.conversation_pages = Mutex::new(pages);
        api.activity_results = Mutex::new(
            (0..MAX_CONVERSATION_PAGES)
                .map(|_| Ok(RawMessagePage::default()))
                .collect(),
        );
        let report = service(api)
            .activity(ActivityRequest {
                since: None,
                after: Some("1970-01-01T00:01:40Z"),
                before: Some("1970-01-01T00:01:42Z"),
                include: &[],
                exclude: &[],
                kinds: &[ConversationKind::Channel],
                order: None,
                conversation_limit: Some(50),
                per_conversation_limit: Some(1),
                limit: Some(1),
                cursor: None,
            })
            .await
            .unwrap();
        assert_eq!(report.scanned_conversations, MAX_CONVERSATION_PAGES);
        assert!(report.conversation_scan_truncated);
        assert!(report.selection_truncated);
        assert!(report.partial);
        assert!(!report.scope_has_more);
        assert!(!report.has_more);
        assert_eq!(report.continuation_kind, None);
        assert_eq!(report.next_cursor, None);
    }

    #[tokio::test]
    async fn validates_all_external_inputs() {
        let service = service(fake_api());
        assert!(matches!(
            service.read_channel("", None, 1).await,
            Err(Error::InvalidInput {
                field: "conversation",
                ..
            })
        ));
        assert!(matches!(
            service.read_thread("C123", "bad", None, 1).await,
            Err(Error::InvalidInput {
                field: "thread_ts",
                ..
            })
        ));
        assert!(matches!(
            service.read_channel("C123", None, 201).await,
            Err(Error::InvalidInput { field: "limit", .. })
        ));
        assert!(matches!(
            service.find_users("\n", 1).await,
            Err(Error::InvalidInput { field: "query", .. })
        ));
    }

    #[tokio::test]
    async fn rejects_empty_essential_response_identifiers() {
        let mut counts_api = fake_api();
        counts_api.counts.channels = vec![entry("", true, 1)];
        assert!(matches!(
            service(counts_api).unreads().await,
            Err(Error::InvalidResponse {
                method: "client.counts"
            })
        ));

        let mut thread_counts_api = fake_api();
        thread_counts_api
            .counts
            .threads
            .unread_count_by_channel
            .insert(String::new(), 1);
        assert!(matches!(
            service(thread_counts_api).unreads().await,
            Err(Error::InvalidResponse {
                method: "client.counts"
            })
        ));

        let mut message_api = fake_api();
        message_api.history.messages = vec![raw_message("", "bad")];
        assert!(matches!(
            service(message_api).read_channel("C123", None, 1).await,
            Err(Error::InvalidResponse {
                method: "conversations.history"
            })
        ));

        let mut user_api = fake_api();
        user_api.user_pages = Mutex::new(VecDeque::from([RawUsersPage {
            members: vec![raw_user("", "target", "Target")],
            ..RawUsersPage::default()
        }]));
        assert!(matches!(
            service(user_api).find_users("target", 1).await,
            Err(Error::InvalidResponse {
                method: "users.list"
            })
        ));
    }
}
