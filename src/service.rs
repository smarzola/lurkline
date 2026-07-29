use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    config::Config,
    error::{Error, Result},
    local_file::{BoundedDownload, DownloadDurability, UploadPass, UploadSource},
    markdown::{MAX_MARKDOWN_BYTES, render_markdown},
    model::{
        AuthorResolution, ClientCountsPayload, Conversation, ConversationKind, ConversationPage,
        ConversationSearchReport, ConversationSearchTruncationReason, CustomEmoji, CustomEmojiKind,
        CustomEmojiList, DoctorReport, Draft, DraftCleanupWarning, DraftDeleteReport,
        DraftDestination, DraftPage, DraftSendReport, FileDownloadReport, FileDraftAssociation,
        FileDraftCreateReport, FileReference, FileShare, FileShareVisibility, FileUploadReport,
        InboxConversation, InboxReport, InboxTruncationReason, Message, MessagePage,
        MessageSearchMatch, MessageSearchPage, RawAuthTestResponse, RawConversation,
        RawConversationsPage, RawDraft, RawDraftResponse, RawDraftRevision, RawDraftsPage,
        RawEmojiResponse, RawFile, RawFileResponse, RawFileUploadAllocation,
        RawFileUploadCompletion, RawMessage, RawMessagePage, RawMessageSearchMatch,
        RawMessageSearchResponse, RawMessagesList, RawMutationResponse, RawPostMessageResponse,
        RawReaction, RawReactionItemResponse, RawUnread, RawUser, RawUsersPage, Reaction,
        ReactionMutationReport, SentMessage, ThreadPage, UnreadConversation, UnreadReport,
        UnreadThreads, User, UserSearchReport, UserSearchTruncationReason,
    },
};

const MAX_MESSAGES: usize = 200;
pub(crate) const MAX_INBOX_CONVERSATIONS: usize = 50;
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

pub(crate) struct FileDraftCreateRequest<'a> {
    pub(crate) conversation: &'a str,
    pub(crate) thread_ts: Option<&'a str>,
    pub(crate) broadcast: bool,
    pub(crate) markdown: &'a str,
    pub(crate) title: Option<&'a str>,
    pub(crate) alt_text: Option<&'a str>,
    pub(crate) confirmed: bool,
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
        target: &mut BoundedDownload,
    ) -> Result<()> {
        let _ = (download_url, target);
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
    workspace_url: String,
    inbox_byte_limit: usize,
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
    complete: bool,
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

impl SlackService {
    pub(crate) fn new(api: impl SlackApi + 'static, config: &Config) -> Self {
        Self {
            api: Arc::new(api),
            team_id: config.team_id.clone(),
            workspace_url: config.base_url.origin().ascii_serialization(),
            inbox_byte_limit: config.max_response_bytes,
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
            workspace_url: self.workspace_url.clone(),
        })
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
            .download_private_file(download_url, &mut target)
            .await?;
        if target.bytes_written() != expected_size {
            return Err(Error::InvalidResponse {
                method: "files.download",
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
            let messages =
                normalize_messages(channel_id, raw.messages, MAX_MESSAGES, method).ok()?;
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
        let normalized = normalize_message(channel_id, message, "reactions.get")?;
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
        let rendered = render_markdown(markdown)?;
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
        let rendered = render_markdown(markdown)?;
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
        let rendered = render_markdown(markdown)?;
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
        let rendered = render_markdown(markdown)?;
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
        normalize_sent_message(channel_id, thread_ts, client_msg_id.clone(), response)
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
        Ok(UnreadReport {
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
        let unreads = self.unreads().await?;
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
        let directory = self.load_conversations_by_id(&selected_ids).await?;
        let mut report = InboxReport {
            team_id: self.team_id.clone(),
            conversations: Vec::with_capacity(selected.len()),
            total_unread_conversations,
            has_more_conversations: total_unread_conversations > 0,
            truncation_reason: (total_unread_conversations > 0)
                .then_some(InboxTruncationReason::ByteLimit),
            threads: unreads.threads,
        };
        if !serialized_json_fits(&report, self.inbox_byte_limit) {
            return Err(Error::ResponseTooLarge {
                method: "inbox",
                limit: self.inbox_byte_limit,
            });
        }
        let mut byte_limited = false;
        for unread in selected {
            let conversation = directory
                .get(&unread.id)
                .cloned()
                .unwrap_or_else(|| fallback_conversation(&unread));
            if conversation.kind != unread.kind {
                return Err(Error::InvalidResponse {
                    method: "conversations.list",
                });
            }
            let messages = self
                .read_channel_by_id(&unread.id, None, message_limit)
                .await?;
            report.conversations.push(InboxConversation {
                conversation,
                unread,
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
            if !serialized_json_fits(&report, self.inbox_byte_limit) {
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
        if !serialized_json_fits(&report, self.inbox_byte_limit) {
            return Err(Error::ResponseTooLarge {
                method: "inbox",
                limit: self.inbox_byte_limit,
            });
        }
        Ok(report)
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
        let mut matches = normalize_search_matches(raw.messages.matches)?;
        self.enrich_search_authors(&mut matches, user_directory)
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
        self.enrich_message_authors(&mut page.messages, user_directory)
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
            messages: normalize_messages(channel, raw.messages, limit, "conversations.history")?,
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
        let mut messages =
            normalize_messages(&channel, raw.messages, limit, "conversations.replies")?;
        self.enrich_message_authors(&mut messages, user_directory)
            .await;
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
        self.enrich_message_authors(std::slice::from_mut(&mut message), user_directory)
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
        let first = normalize_message(channel, first, "messages.list")?;
        for duplicate in matches {
            if normalize_message(channel, duplicate, "messages.list")? != first {
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

    async fn load_conversations_by_id(
        &self,
        ids: &HashSet<String>,
    ) -> Result<HashMap<String, Conversation>> {
        if ids.is_empty() {
            return Ok(HashMap::new());
        }

        let mut matched = Vec::new();
        let mut matched_ids = HashSet::new();
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
                break;
            }

            let next = response_cursor("conversations.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
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

        let users = if matched.iter().any(|conversation| conversation.is_im) {
            self.load_user_directory().await?.users
        } else {
            HashMap::new()
        };
        Ok(normalize_conversations(matched, &users)?
            .into_iter()
            .map(|conversation| (conversation.id.clone(), conversation))
            .collect())
    }

    async fn enrich_message_authors(
        &self,
        messages: &mut [Message],
        user_directory: Option<UserDirectory>,
    ) {
        if !messages.iter().any(message_author_needs_directory) {
            return;
        }
        let directory = self.author_directory(user_directory).await;
        for message in messages
            .iter_mut()
            .filter(|message| message_author_needs_directory(message))
        {
            enrich_author(
                message.author_id.as_deref(),
                &mut message.author_name,
                &mut message.author_display_name,
                &mut message.author_resolution,
                &directory,
            );
        }
    }

    async fn enrich_search_authors(
        &self,
        messages: &mut [MessageSearchMatch],
        user_directory: Option<UserDirectory>,
    ) {
        if !messages.iter().any(search_author_needs_directory) {
            return;
        }
        let directory = self.author_directory(user_directory).await;
        for message in messages
            .iter_mut()
            .filter(|message| search_author_needs_directory(message))
        {
            enrich_author(
                message.author_id.as_deref(),
                &mut message.author_name,
                &mut message.author_display_name,
                &mut message.author_resolution,
                &directory,
            );
        }
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
                            complete: false,
                        },
                        error,
                    };
                }
            };
            for user in page_users {
                users.insert(user.id.clone(), user);
            }
            let Some(next) = next else {
                return UserDirectoryScan::Finished(UserDirectory {
                    users,
                    complete: true,
                });
            };
            seen_cursors.insert(next.clone());
            cursor = Some(next);
            if page_index + 1 == MAX_USER_PAGES {
                return UserDirectoryScan::Finished(UserDirectory {
                    users,
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
                        .map(|user| user.name.trim())
                        .filter(|name| is_valid_conversation_name(name))
                        .map(str::to_owned);
                    let name_is_fallback = loaded_name.is_none();
                    let name = loaded_name.unwrap_or_else(|| user_id.clone());
                    let display_name = user
                        .and_then(|user| {
                            [
                                user.display_name.trim(),
                                user.real_name.trim(),
                                user.name.trim(),
                            ]
                            .into_iter()
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

fn fallback_conversation(unread: &UnreadConversation) -> Conversation {
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
            let permalink = raw.permalink.filter(|value| !value.is_empty());
            if permalink
                .as_deref()
                .is_some_and(|value| value.len() > 8192 || value.chars().any(char::is_control))
            {
                return Err(Error::InvalidResponse {
                    method: "search.messages",
                });
            }
            Ok(MessageSearchMatch {
                channel_id: raw.channel.id,
                channel_name,
                ts: raw.ts,
                thread_ts: raw.thread_ts,
                author_id,
                author_name,
                author_display_name: None,
                author_resolution,
                text: raw.text,
                blocks: raw.blocks,
                attachments: normalize_attachments(raw.attachments, "search.messages")?,
                reactions: normalize_reactions(raw.reactions, "search.messages")?,
                files: normalize_files(raw.files, "search.messages")?,
                permalink,
            })
        })
        .collect()
}

fn append_unreads(
    target: &mut Vec<UnreadConversation>,
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
            target.push(UnreadConversation {
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
        .map(|message| normalize_message(channel, message, method))
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
        message: normalize_message(channel_id, response.message, "chat.postMessage")?,
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

fn message_author_needs_directory(message: &Message) -> bool {
    message.author_resolution == AuthorResolution::NotAttempted
}

fn search_author_needs_directory(message: &MessageSearchMatch) -> bool {
    message.author_resolution == AuthorResolution::NotAttempted
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
    if let Some(user) = directory.users.get(author_id) {
        *author_name = directory_author_label(&user.name);
        *author_display_name = directory_author_label(&user.display_name)
            .or_else(|| directory_author_label(&user.real_name));
        if author_name.is_some() || author_display_name.is_some() {
            *author_resolution = AuthorResolution::Directory;
            return;
        }
    }
    *author_resolution = missing_resolution;
}

fn directory_author_label(value: &str) -> Option<String> {
    let value = value.trim();
    is_valid_author_label(value).then(|| value.to_owned())
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

fn normalize_message(channel: &str, message: RawMessage, method: &'static str) -> Result<Message> {
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
    Ok(Message {
        channel_id: channel.to_owned(),
        ts: message.ts,
        thread_ts: message.thread_ts,
        author_id,
        author_name,
        author_display_name: None,
        author_resolution,
        text: message.text,
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
    let real_name = if raw.profile.real_name.is_empty() {
        raw.real_name
    } else {
        raw.profile.real_name
    };
    User {
        id: raw.id,
        name: raw.name,
        display_name: raw.profile.display_name,
        real_name,
        title: raw.profile.title,
        deleted: raw.deleted,
        is_bot: raw.is_bot,
        timezone: raw.tz,
        image_url: raw.profile.image_72,
    }
}

fn user_matches(user: &RawUser, needle: &str) -> bool {
    [
        user.id.as_str(),
        user.name.as_str(),
        user.real_name.as_str(),
        user.profile.display_name.as_str(),
        user.profile.real_name.as_str(),
        user.profile.title.as_str(),
    ]
    .iter()
    .any(|candidate| candidate.to_lowercase().contains(needle))
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
        history: RawMessagePage,
        history_pages: Mutex<VecDeque<RawMessagePage>>,
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
            history: RawMessagePage::default(),
            history_pages: Mutex::new(VecDeque::new()),
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
            name: name.into(),
            real_name: "Fallback Name".into(),
            profile: RawUserProfile {
                display_name: display_name.into(),
                real_name: "Profile Name".into(),
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
        let service = service(api);

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
        assert!(matches!(
            service.delete_draft("DR-valid", false).await,
            Err(Error::ConfirmationRequired {
                action: "draft deletion"
            })
        ));
        assert!(calls.lock().unwrap().is_empty());
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
        let fixture = UploadFixture::new(b"synthetic");
        let mut api = fake_api();
        let mut draft = raw_file_draft("DR-created-file", "700", "C123", "body", "FUPLOAD");
        draft.client_msg_id = Some(REQUEST_CLIENT_MSG_ID.into());
        draft.blocks = Some(render_markdown("**body**").unwrap().blocks);
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
        let upload_calls = api.upload_calls.clone();
        let draft_calls = api.draft_calls.clone();
        let service = service(api);

        let report = service
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
            .send_message("C123", None, false, "**root**", true)
            .await
            .unwrap();
        let reply = service
            .send_message("C123", Some("6000.000001"), true, "reply", true)
            .await
            .unwrap();
        assert_eq!(root.message.channel_id, "C123");
        assert_eq!(root.message.thread_ts, None);
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

        let calls = post_calls.lock().unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].channel, "C123");
        assert_eq!(calls[0].thread_ts, None);
        assert!(!calls[0].broadcast);
        assert_eq!(calls[0].text, "root");
        assert_eq!(calls[0].blocks[0]["type"], "rich_text");
        assert_eq!(calls[1].thread_ts.as_deref(), Some("6000.000001"));
        assert!(calls[1].broadcast);
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
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "GONE",
                        "kind": "group_direct_message",
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "CNULL",
                        "kind": "channel",
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
            members: vec![raw_user("WALI", "alice", "Alice Example")],
            ..RawUsersPage::default()
        }]));
        api.history.messages = vec![raw_message("100.000001", "recent")];
        let history_calls = api.history_calls.clone();

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
        assert_eq!(report.conversations[0].messages.messages.len(), 1);
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
            api
        };

        let full_report = service(make_api()).inbox(3, 1).await.unwrap();
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
        bounded_service.inbox_byte_limit = byte_limit;
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
        bounded_service.inbox_byte_limit = boundary_limit;
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
        bounded_service.inbox_byte_limit = 1;

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
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next"));

        let thread = service
            .read_thread("C123", "100.000001", None, 2)
            .await
            .unwrap();
        assert_eq!(thread.messages.len(), 2);
        assert_eq!(thread.thread_ts, "100.000001");
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
            file_access: None,
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

        let mut mismatch = fake_api();
        mismatch.download_bytes = b"short".to_vec();
        let target = root
            .prepare_download(std::path::Path::new("mismatch"), 10)
            .unwrap();
        assert!(matches!(
            service(mismatch)
                .download_file(file.clone(), target, "mismatch".into())
                .await,
            Err(Error::InvalidResponse {
                method: "files.download"
            })
        ));
        assert!(!directory.join("mismatch").exists());

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
        let mut message = raw_message("100.000001", "still readable");
        message.username = Some("unsafe\nname".into());
        api.history.messages = vec![message];
        api.user_list_error = true;
        let user_calls = api.user_calls.clone();

        let page = service(api).read_channel("C123", None, 1).await.unwrap();

        assert_single_user_directory_call(&user_calls);
        assert_eq!(page.messages[0].text, "still readable");
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
    async fn unenriched_inbox_and_sent_messages_report_not_attempted() {
        let mut inbox_api = fake_api();
        inbox_api.counts.channels = vec![entry("C123", true, 1)];
        inbox_api.conversation_pages = Mutex::new(VecDeque::from([RawConversationsPage {
            channels: vec![raw_conversation("C123", "general")],
            ..RawConversationsPage::default()
        }]));
        inbox_api.history.messages = vec![raw_message("100.000001", "inbox")];
        let user_calls = inbox_api.user_calls.clone();

        let inbox = service(inbox_api).inbox(1, 1).await.unwrap();
        assert!(user_calls.lock().unwrap().is_empty());
        let inbox_json = serde_json::to_value(&inbox).unwrap();
        assert_eq!(
            inbox_json["conversations"][0]["messages"]["messages"][0]["author_resolution"],
            "not_attempted"
        );

        let sent = normalize_sent_message(
            "C123",
            None,
            "00000000-0000-4000-8000-000000000001".into(),
            RawPostMessageResponse {
                channel: "C123".into(),
                ts: "100.000001".into(),
                message: raw_message("100.000001", "sent"),
            },
        )
        .unwrap();
        let sent_json = serde_json::to_value(sent).unwrap();
        assert_eq!(sent_json["message"]["author_resolution"], "not_attempted");
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
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::json!({
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
            })
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
