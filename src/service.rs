use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use uuid::Uuid;

use crate::{
    config::Config,
    error::{Error, Result},
    local_file::{BoundedDownload, DownloadDurability},
    markdown::{MAX_MARKDOWN_BYTES, render_markdown},
    model::{
        ClientCountsPayload, Conversation, ConversationKind, ConversationPage,
        ConversationSearchReport, ConversationSearchTruncationReason, CustomEmoji, CustomEmojiKind,
        CustomEmojiList, DoctorReport, Draft, DraftCleanupWarning, DraftDeleteReport,
        DraftDestination, DraftPage, DraftSendReport, FileDownloadReport, FileReference, FileShare,
        FileShareVisibility, InboxConversation, InboxReport, Message, MessagePage,
        MessageSearchMatch, MessageSearchPage, RawAuthTestResponse, RawConversation,
        RawConversationsPage, RawDraft, RawDraftResponse, RawDraftRevision, RawDraftsPage,
        RawEmojiResponse, RawFile, RawFileResponse, RawMessage, RawMessagePage,
        RawMessageSearchMatch, RawMessageSearchResponse, RawMessagesList, RawMutationResponse,
        RawPostMessageResponse, RawReaction, RawReactionItemResponse, RawUnread, RawUser,
        RawUsersPage, Reaction, ReactionMutationReport, SentMessage, ThreadPage,
        UnreadConversation, UnreadReport, UnreadThreads, User, UserSearchReport,
        UserSearchTruncationReason,
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
const MAX_DRAFT_DESTINATION_USERS: usize = 100;
const MAX_ATTACHMENTS_PER_MESSAGE: usize = 100;
const MAX_FILES_PER_MESSAGE: usize = 100;
const MAX_FILE_SHARES: usize = 1_000;
const MAX_REACTIONS_PER_MESSAGE: usize = 100;
const MAX_REACTION_USERS: usize = 1_000;
const MAX_CUSTOM_EMOJI: usize = 10_000;

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
    ) -> Result<RawDraftResponse> {
        let _ = (client_msg_id, destinations, blocks);
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
    ) -> Result<RawDraftResponse> {
        let _ = (draft_id, last_updated_ts, destinations, blocks);
        Err(Error::InvalidResponse {
            method: "drafts.update",
        })
    }
    async fn drafts_delete(
        &self,
        draft_id: &str,
        last_updated_ts: &str,
    ) -> Result<RawMutationResponse> {
        let _ = (draft_id, last_updated_ts);
        Err(Error::InvalidResponse {
            method: "drafts.delete",
        })
    }
    async fn chat_post_message(
        &self,
        channel: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        client_msg_id: &str,
        text: &str,
        blocks: &[serde_json::Value],
    ) -> Result<RawPostMessageResponse> {
        let _ = (channel, thread_ts, broadcast, client_msg_id, text, blocks);
        Err(Error::InvalidResponse {
            method: "chat.postMessage",
        })
    }
}

#[derive(Clone)]
pub(crate) struct SlackService {
    api: Arc<dyn SlackApi>,
    team_id: String,
    workspace_url: String,
    now_millis: fn() -> Result<String>,
}

struct UserDirectory {
    users: HashMap<String, User>,
    complete: bool,
}

impl SlackService {
    pub(crate) fn new(api: impl SlackApi + 'static, config: &Config) -> Self {
        Self {
            api: Arc::new(api),
            team_id: config.team_id.clone(),
            workspace_url: config.base_url.origin().ascii_serialization(),
            now_millis: system_unix_milliseconds,
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
            Err(error) if reaction_error_is_ambiguous(&error) => true,
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
        let response = self
            .api
            .drafts_create(
                &Uuid::new_v4().to_string(),
                std::slice::from_ref(&destination),
                &rendered.blocks,
            )
            .await?;
        let draft = normalize_draft(response.draft, "drafts.create")?;
        require_supported_draft(&draft)?;
        if draft.destinations.len() != 1 || !same_draft_route(&draft.destinations[0], &destination)
        {
            return Err(Error::InvalidResponse {
                method: "drafts.create",
            });
        }
        Ok(draft)
    }

    pub(crate) async fn update_draft(&self, draft_id: &str, markdown: &str) -> Result<Draft> {
        validate_draft_id(draft_id)?;
        let rendered = render_markdown(markdown)?;
        let current = self.get_draft(draft_id).await?;
        require_supported_draft(&current)?;
        let client_last_updated_ts = (self.now_millis)()?;
        let response = self
            .api
            .drafts_update(
                &current.id,
                &client_last_updated_ts,
                &current.destinations,
                &rendered.blocks,
            )
            .await?;
        let updated = normalize_draft(response.draft, "drafts.update")?;
        require_supported_draft(&updated)?;
        if updated.id != current.id
            || updated.destinations.len() != 1
            || !same_draft_route(&updated.destinations[0], &current.destinations[0])
        {
            return Err(Error::InvalidResponse {
                method: "drafts.update",
            });
        }
        Ok(updated)
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
        self.api
            .drafts_delete(&current.id, &current.client_last_updated_ts)
            .await?;
        Ok(DraftDeleteReport {
            id: current.id,
            deleted: true,
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
        let sent = self
            .post_rich_message(
                channel_id,
                destination.thread_ts.as_deref(),
                destination.broadcast,
                &fallback,
                blocks,
            )
            .await?;

        match self
            .api
            .drafts_delete(&draft.id, &draft.client_last_updated_ts)
            .await
        {
            Ok(_) => Ok(DraftSendReport {
                sent,
                draft_id: draft.id,
                draft_deleted: true,
                cleanup_warning: None,
            }),
            Err(error) => Ok(DraftSendReport {
                sent,
                draft_id: draft.id.clone(),
                draft_deleted: false,
                cleanup_warning: Some(DraftCleanupWarning {
                    draft_id: draft.id,
                    last_updated_ts: draft.last_updated_ts,
                    reason: error.to_string(),
                }),
            }),
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
        let response = self
            .api
            .chat_post_message(
                channel_id,
                thread_ts,
                broadcast,
                &client_msg_id,
                text,
                blocks,
            )
            .await
            .map_err(|error| classify_publication_error(&client_msg_id, error))?;
        normalize_sent_message(channel_id, thread_ts, client_msg_id.clone(), response)
            .map_err(|_| Error::PublicationUncertain { client_msg_id })
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
        let selected_ids = selected
            .iter()
            .map(|unread| unread.id.clone())
            .collect::<HashSet<_>>();
        let directory = self.load_conversations_by_id(&selected_ids).await?;
        let mut conversations = Vec::with_capacity(selected.len());
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
            conversations.push(InboxConversation {
                conversation,
                unread,
                messages,
            });
        }
        Ok(InboxReport {
            team_id: self.team_id.clone(),
            has_more_conversations: total_unread_conversations > conversations.len(),
            total_unread_conversations,
            conversations,
            threads: unreads.threads,
        })
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
        if let Some(reference) = conversation {
            let conversation = self.resolve_search_conversation(reference).await?;
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
        let matches = normalize_search_matches(raw.messages.matches)?;
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
        let channel = self.resolve_conversation_id(channel).await?;
        self.read_channel_by_id(&channel, cursor, limit).await
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
        let channel = self.resolve_conversation_id(channel).await?;
        let raw = self
            .api
            .conversation_replies(&channel, thread_ts, cursor, limit)
            .await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor =
            response_cursor("conversations.replies", raw.response_metadata.next_cursor)?;
        reject_repeated_cursor("conversations.replies", cursor, next_cursor.as_deref())?;
        Ok(ThreadPage {
            channel_id: channel.clone(),
            thread_ts: thread_ts.to_owned(),
            messages: normalize_messages(&channel, raw.messages, limit, "conversations.replies")?,
            has_more: raw.has_more || next_cursor.is_some() || locally_truncated,
            next_cursor,
        })
    }

    pub(crate) async fn get_message(&self, channel: &str, message_ts: &str) -> Result<Message> {
        validate_timestamp("message_ts", message_ts)?;
        let channel = self.resolve_conversation_id(channel).await?;
        let raw = self.api.messages_list(&channel, message_ts).await?;
        let mut candidates = raw.messages.into_values().collect::<Vec<_>>();
        if let Some(channel_messages) = raw.messages_data.get(&channel) {
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
        candidates
            .into_iter()
            .find(|message| message.ts == message_ts)
            .map(|message| normalize_message(&channel, message, "messages.list"))
            .transpose()?
            .ok_or(Error::NotFound {
                resource: "Slack message",
            })
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

    async fn resolve_search_conversation(&self, reference: &str) -> Result<Conversation> {
        if is_slack_shaped_conversation_id(reference) {
            return self.find_conversation_by_id(reference).await;
        }
        self.resolve_named_conversation(reference).await
    }

    async fn resolve_named_conversation(&self, reference: &str) -> Result<Conversation> {
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
                return matched.ok_or(Error::NotFound {
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

    async fn load_user_directory(&self) -> Result<UserDirectory> {
        let mut users = HashMap::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = HashSet::new();

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
                if !is_valid_user_id(&raw_user.id) {
                    return Err(Error::InvalidResponse {
                        method: "users.list",
                    });
                }
                let user = normalize_user(raw_user);
                users.insert(user.id.clone(), user);
            }
            let next = response_cursor("users.list", page.response_metadata.next_cursor)?;
            let Some(next) = next else {
                return Ok(UserDirectory {
                    users,
                    complete: true,
                });
            };
            if !seen_cursors.insert(next.clone()) {
                return Err(Error::InvalidResponse {
                    method: "users.list",
                });
            }
            cursor = Some(next);
            if page_index + 1 == MAX_USER_PAGES {
                return Ok(UserDirectory {
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
            let author_name = raw.username.filter(|value| !value.is_empty());
            if author_name
                .as_deref()
                .is_some_and(|value| value.len() > 256 || value.chars().any(char::is_control))
            {
                return Err(Error::InvalidResponse {
                    method: "search.messages",
                });
            }
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
    }) || raw.file_ids.iter().any(|file_id| {
        file_id.is_empty() || file_id.len() > 128 || file_id.chars().any(char::is_control)
    }) {
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
    let is_supported = raw.destinations.len() == 1
        && raw.destinations[0].channel_id.is_some()
        && raw.destinations[0].extra.is_empty()
        && raw.file_ids.is_empty()
        && raw.attachments.is_empty()
        && !raw.is_deleted
        && !raw.is_sent
        && raw
            .blocks
            .as_ref()
            .is_some_and(|blocks| is_rich_text_blocks(blocks));
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
        is_supported,
    })
}

fn same_draft_route(actual: &DraftDestination, requested: &DraftDestination) -> bool {
    actual.channel_id == requested.channel_id
        && actual.thread_ts == requested.thread_ts
        && actual.broadcast == requested.broadcast
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
    match error {
        Error::HttpStatus { .. }
        | Error::ResponseTooLarge { .. }
        | Error::InvalidResponse { .. }
        | Error::Timeout { .. }
        | Error::Transport { .. } => Error::PublicationUncertain {
            client_msg_id: client_msg_id.to_owned(),
        },
        definitive => definitive,
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
    reactions
        .into_iter()
        .map(|reaction| {
            let name = validate_reaction_name(&reaction.name)
                .map_err(|_| Error::InvalidResponse { method })?;
            if reaction.users.len() > MAX_REACTION_USERS
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

    let share_channels = raw
        .shares
        .public
        .len()
        .saturating_add(raw.shares.private.len());
    let share_count = raw
        .shares
        .public
        .values()
        .chain(raw.shares.private.values())
        .map(Vec::len)
        .sum::<usize>();
    if share_channels > MAX_FILE_SHARES || share_count > MAX_FILE_SHARES {
        return Err(Error::InvalidResponse { method });
    }
    let mut shares = Vec::with_capacity(share_count);
    append_file_shares(
        &mut shares,
        raw.shares.public,
        FileShareVisibility::Public,
        method,
    )?;
    append_file_shares(
        &mut shares,
        raw.shares.private,
        FileShareVisibility::Private,
        method,
    )?;
    shares.sort_by(|left, right| {
        left.channel_id
            .cmp(&right.channel_id)
            .then_with(|| left.ts.cmp(&right.ts))
            .then_with(|| left.thread_ts.cmp(&right.thread_ts))
    });

    Ok(FileReference {
        id: raw.id,
        name: raw.name,
        title: raw.title,
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
        shares,
    })
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

fn normalize_message(channel: &str, message: RawMessage, method: &'static str) -> Result<Message> {
    Ok(Message {
        channel_id: channel.to_owned(),
        ts: message.ts,
        thread_ts: message.thread_ts,
        author_id: message.user.or(message.bot_id),
        author_name: message.username,
        text: message.text,
        blocks: message.blocks,
        attachments: normalize_attachments(message.attachments, method)?,
        reply_count: message.reply_count,
        latest_reply: message.latest_reply,
        reactions: normalize_reactions(message.reactions, method)?,
        files: normalize_files(message.files, method)?,
    })
}

fn reaction_error_is_ambiguous(error: &Error) -> bool {
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
        replies: RawMessagePage,
        message_list: RawMessagesList,
        search: RawMessageSearchResponse,
        search_calls: Arc<Mutex<Vec<SearchCall>>>,
        history_calls: Arc<Mutex<Vec<HistoryCall>>>,
        reply_calls: Arc<Mutex<Vec<ReplyCall>>>,
        conversation_calls: Arc<Mutex<Vec<ConversationCall>>>,
        conversation_pages: Mutex<VecDeque<RawConversationsPage>>,
        user_pages: Mutex<VecDeque<RawUsersPage>>,
        drafts_page: RawDraftsPage,
        draft_info: RawDraftResponse,
        draft_create: RawDraftResponse,
        draft_update: RawDraftResponse,
        draft_delete_error: bool,
        draft_calls: Arc<Mutex<Vec<DraftCall>>>,
        post_response: Option<RawPostMessageResponse>,
        post_error: Option<String>,
        post_calls: Arc<Mutex<Vec<PostCall>>>,
        emoji_response: RawEmojiResponse,
        file_response: RawFileResponse,
        reaction_present: Arc<Mutex<bool>>,
        reaction_name: Arc<Mutex<String>>,
        reaction_error: Option<&'static str>,
        reaction_apply_before_error: bool,
        reaction_get_error_after: Option<usize>,
        reaction_get_count: Arc<Mutex<usize>>,
        reaction_calls: Arc<Mutex<Vec<String>>>,
        download_bytes: Vec<u8>,
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
        },
        Update {
            draft_id: String,
            last_updated_ts: String,
            destinations: Vec<DraftDestination>,
            blocks: Vec<serde_json::Value>,
        },
        Delete {
            draft_id: String,
            last_updated_ts: String,
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
            Ok(self.history.clone())
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
            Ok(self.replies.clone())
        }

        async fn messages_list(
            &self,
            _channel: &str,
            _message_ts: &str,
        ) -> Result<RawMessagesList> {
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

        async fn users_list(&self, _cursor: Option<&str>, _limit: usize) -> Result<RawUsersPage> {
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

        async fn files_info(&self, _file_id: &str) -> Result<RawFileResponse> {
            Ok(self.file_response.clone())
        }

        async fn reactions_get(
            &self,
            _channel: &str,
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
            Ok(RawReactionItemResponse {
                message: Some(RawMessage {
                    ts: message_ts.into(),
                    reactions: present
                        .then(|| RawReaction {
                            name,
                            count: 1,
                            users: vec!["U123".into()],
                        })
                        .into_iter()
                        .collect(),
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

        async fn drafts_list(&self, next_ts: Option<&str>, limit: usize) -> Result<RawDraftsPage> {
            self.draft_calls.lock().unwrap().push(DraftCall::List {
                next_ts: next_ts.map(str::to_owned),
                limit,
            });
            Ok(self.drafts_page.clone())
        }

        async fn drafts_info(&self, draft_id: &str) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Info {
                draft_id: draft_id.into(),
            });
            Ok(self.draft_info.clone())
        }

        async fn drafts_create(
            &self,
            client_msg_id: &str,
            destinations: &[DraftDestination],
            blocks: &[serde_json::Value],
        ) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Create {
                client_msg_id: client_msg_id.into(),
                destinations: destinations.to_vec(),
                blocks: blocks.to_vec(),
            });
            Ok(self.draft_create.clone())
        }

        async fn drafts_update(
            &self,
            draft_id: &str,
            last_updated_ts: &str,
            destinations: &[DraftDestination],
            blocks: &[serde_json::Value],
        ) -> Result<RawDraftResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Update {
                draft_id: draft_id.into(),
                last_updated_ts: last_updated_ts.into(),
                destinations: destinations.to_vec(),
                blocks: blocks.to_vec(),
            });
            Ok(self.draft_update.clone())
        }

        async fn drafts_delete(
            &self,
            draft_id: &str,
            last_updated_ts: &str,
        ) -> Result<RawMutationResponse> {
            self.draft_calls.lock().unwrap().push(DraftCall::Delete {
                draft_id: draft_id.into(),
                last_updated_ts: last_updated_ts.into(),
            });
            if self.draft_delete_error {
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
            channel: &str,
            thread_ts: Option<&str>,
            broadcast: bool,
            client_msg_id: &str,
            text: &str,
            blocks: &[serde_json::Value],
        ) -> Result<RawPostMessageResponse> {
            self.post_calls.lock().unwrap().push(PostCall {
                channel: channel.into(),
                thread_ts: thread_ts.map(str::to_owned),
                broadcast,
                client_msg_id: client_msg_id.into(),
                text: text.into(),
                blocks: blocks.to_vec(),
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
                    channel: channel.into(),
                    ts: "7000.000001".into(),
                    message: RawMessage {
                        ts: "7000.000001".into(),
                        thread_ts: thread_ts.map(str::to_owned),
                        text: text.into(),
                        blocks: Some(blocks.to_vec()),
                        ..RawMessage::default()
                    },
                }))
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
            replies: RawMessagePage::default(),
            message_list: RawMessagesList::default(),
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
            drafts_page: RawDraftsPage::default(),
            draft_info: RawDraftResponse::default(),
            draft_create: RawDraftResponse::default(),
            draft_update: RawDraftResponse::default(),
            draft_delete_error: false,
            draft_calls: Arc::new(Mutex::new(Vec::new())),
            post_response: None,
            post_error: None,
            post_calls: Arc::new(Mutex::new(Vec::new())),
            emoji_response: RawEmojiResponse::default(),
            file_response: RawFileResponse::default(),
            reaction_present: Arc::new(Mutex::new(false)),
            reaction_name: Arc::new(Mutex::new("eyes".into())),
            reaction_error: None,
            reaction_apply_before_error: false,
            reaction_get_error_after: None,
            reaction_get_count: Arc::new(Mutex::new(0)),
            reaction_calls: Arc::new(Mutex::new(Vec::new())),
            download_bytes: b"safe".to_vec(),
        }
    }

    fn service(api: impl SlackApi + 'static) -> SlackService {
        let config = Config::for_test(Url::parse("http://127.0.0.1:1234").unwrap(), 1024);
        let mut service = SlackService::new(api, &config);
        service.now_millis = || Ok("9000123".into());
        service
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
            ..RawDraft::default()
        }
    }

    fn raw_self_dm_draft(id: &str, revision: &str, text: &str) -> RawDraft {
        let mut draft = raw_draft(id, revision, "D123", text);
        draft.destinations[0].user_ids = Some(vec!["U123".into()]);
        draft
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
            draft: raw_draft("DR-existing", "3000.5", "C123", "old"),
        };
        api.draft_create = RawDraftResponse {
            draft: raw_draft("DR-created", "4000", "C123", "created"),
        };
        api.draft_update = RawDraftResponse {
            draft: raw_draft("DR-existing", "3001", "C123", "updated"),
        };
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
                deleted: true
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
        } = &calls[1]
        else {
            panic!("expected draft creation call");
        };
        assert_eq!(Uuid::parse_str(client_msg_id).unwrap().get_version_num(), 4);
        assert_eq!(destinations[0].channel_id.as_deref(), Some("C123"));
        assert_eq!(blocks[0]["type"], "rich_text");
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
                },
                DraftCall::Info {
                    draft_id: "DR-existing".into()
                },
                DraftCall::Delete {
                    draft_id: "DR-existing".into(),
                    last_updated_ts: "3000500".into(),
                }
            ]
        );
    }

    #[tokio::test]
    async fn drafts_accept_and_preserve_valid_self_dm_user_ids() {
        let mut api = fake_api();
        api.drafts_page = RawDraftsPage {
            drafts: vec![raw_self_dm_draft("DR-list", "1000", "listed")],
            ..RawDraftsPage::default()
        };
        api.draft_info = RawDraftResponse {
            draft: raw_self_dm_draft("DR-existing", "2000", "existing"),
        };
        api.draft_create = RawDraftResponse {
            draft: raw_self_dm_draft("DR-created", "3000", "created"),
        };
        api.draft_update = RawDraftResponse {
            draft: raw_self_dm_draft("DR-existing", "2001", "updated"),
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
        assert_eq!(
            destinations[0].user_ids.as_deref(),
            Some(["U123".to_owned()].as_slice())
        );
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
            api.draft_create = RawDraftResponse { draft };
            assert!(matches!(
                service(api)
                    .create_draft("D123", requested_thread, requested_broadcast, "synthetic")
                    .await,
                Err(Error::InvalidResponse {
                    method: "drafts.create"
                })
            ));
        }
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
            api.draft_info = RawDraftResponse {
                draft: raw_self_dm_draft("DR-existing", "2000", "existing"),
            };
            api.draft_update = RawDraftResponse { draft };
            assert!(matches!(
                service(api).update_draft("DR-existing", "synthetic").await,
                Err(Error::InvalidResponse {
                    method: "drafts.update"
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
                DraftCall::Info {
                    draft_id: "DR-attached".into()
                }
            ]
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
        let mut api = fake_api();
        api.draft_info = RawDraftResponse {
            draft: raw_draft("DR-send", "8000.5", "C123", "draft body"),
        };
        api.draft_delete_error = true;
        let draft_calls = api.draft_calls.clone();
        let post_calls = api.post_calls.clone();
        let service = service(api);

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
        assert_eq!(
            draft_calls.lock().unwrap().as_slice(),
            [
                DraftCall::Info {
                    draft_id: "DR-send".into()
                },
                DraftCall::Delete {
                    draft_id: "DR-send".into(),
                    last_updated_ts: "8000500".into()
                }
            ]
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
                shares: crate::model::RawFileShares {
                    private: BTreeMap::from([(
                        "C123".into(),
                        vec![crate::model::RawFileShare {
                            ts: "100.000001".into(),
                            thread_ts: None,
                        }],
                    )]),
                    ..crate::model::RawFileShares::default()
                },
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
        assert_eq!(file.shares.len(), 1);
        let emoji = service.list_custom_emoji().await.unwrap();
        assert_eq!(emoji.emoji.len(), 2);
        assert_eq!(emoji.emoji[0].kind, CustomEmojiKind::Image);
        assert_eq!(emoji.emoji[1].alias_for.as_deref(), Some("party"));
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
            shares: vec![],
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

        for ambiguous_error in ["timeout", "fatal_error", "internal_error"] {
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
        let message = service(api)
            .get_message("C123", "100.000002")
            .await
            .unwrap();
        assert_eq!(message.text, "target");
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::json!({
                "channel_id": "C123",
                "ts": "100.000002",
                "thread_ts": null,
                "author_id": "U123",
                "author_name": null,
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
                    "shares": []
                }]
            })
        );
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
