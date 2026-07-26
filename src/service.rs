use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_trait::async_trait;

use crate::{
    config::Config,
    error::{Error, Result},
    model::{
        ClientCountsPayload, Conversation, ConversationKind, ConversationPage,
        ConversationSearchReport, ConversationSearchTruncationReason, DoctorReport, FileReference,
        InboxConversation, InboxReport, Message, MessagePage, MessageSearchMatch,
        MessageSearchPage, RawConversation, RawConversationsPage, RawMessage, RawMessagePage,
        RawMessageSearchMatch, RawMessageSearchResponse, RawMessagesList, RawUnread, RawUser,
        RawUsersPage, Reaction, ThreadPage, UnreadConversation, UnreadReport, UnreadThreads, User,
        UserSearchReport, UserSearchTruncationReason,
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
}

#[derive(Clone)]
pub(crate) struct SlackService {
    api: Arc<dyn SlackApi>,
    team_id: String,
    workspace_url: String,
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
            let messages = self.read_channel(&unread.id, None, message_limit).await?;
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
        let raw = self
            .api
            .conversation_history(&channel, cursor, limit)
            .await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor =
            response_cursor("conversations.history", raw.response_metadata.next_cursor)?;
        reject_repeated_cursor("conversations.history", cursor, next_cursor.as_deref())?;
        Ok(MessagePage {
            channel_id: channel.clone(),
            messages: normalize_messages(&channel, raw.messages, limit, "conversations.history")?,
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
            .map(|message| normalize_message(&channel, message))
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
        if is_valid_any_conversation_id(reference) {
            return Ok(reference.to_owned());
        }
        Ok(self.resolve_named_conversation(reference).await?.id)
    }

    async fn resolve_search_conversation(&self, reference: &str) -> Result<Conversation> {
        if is_valid_any_conversation_id(reference) {
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
    Ok(messages
        .into_iter()
        .take(limit)
        .map(|message| normalize_message(channel, message))
        .collect())
}

fn normalize_message(channel: &str, message: RawMessage) -> Message {
    Message {
        channel_id: channel.to_owned(),
        ts: message.ts,
        thread_ts: message.thread_ts,
        author_id: message.user.or(message.bot_id),
        author_name: message.username,
        text: message.text,
        reply_count: message.reply_count,
        latest_reply: message.latest_reply,
        reactions: message
            .reactions
            .into_iter()
            .map(|reaction| Reaction {
                name: reaction.name,
                count: reaction.count,
            })
            .collect(),
        files: message
            .files
            .into_iter()
            .map(|file| FileReference {
                id: file.id,
                name: file.name,
                mimetype: file.mimetype,
                size: file.size,
                download_url: file.url_private_download,
            })
            .collect(),
    }
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
        ConversationKind::GroupDirectMessage => id.starts_with('G'),
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
        }
    }

    fn service(api: impl SlackApi + 'static) -> SlackService {
        let config = Config::for_test(Url::parse("http://127.0.0.1:1234").unwrap(), 1024);
        SlackService::new(api, &config)
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
            }],
            files: vec![RawFile {
                id: "F123".into(),
                name: "note.txt".into(),
                mimetype: "text/plain".into(),
                size: 12,
                url_private_download: Some("https://files.slack.com/note.txt".into()),
            }],
            ..RawMessage::default()
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
                    ..raw_conversation("GTEAM", "mpdm-alice--bob-1")
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
            .list_conversations(Some("start-page"), 3)
            .await
            .unwrap();
        assert_eq!(page.conversations.len(), 3);
        assert_eq!(page.conversations[0].name, "general");
        assert_eq!(page.conversations[1].kind, ConversationKind::DirectMessage);
        assert_eq!(page.conversations[1].name, "alice");
        assert_eq!(page.conversations[1].display_name, "Alice Example");
        assert_eq!(page.conversations[1].user_id.as_deref(), Some("WALI"));
        assert_eq!(
            page.conversations[2].kind,
            ConversationKind::GroupDirectMessage
        );
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next-page"));
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
            .read_channel("CGENERAL", None, 1)
            .await
            .unwrap();
        assert_eq!(id_page.channel_id, "CGENERAL");

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
                id: "DALI".into(),
                is_im: true,
                user: Some("UALI".into()),
                ..RawConversation::default()
            }],
            ..RawConversationsPage::default()
        }]));
        let page = service(api)
            .search_messages("incident", Some("DALI"), None, None, None, 20)
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
                "reply_count": 0,
                "latest_reply": null,
                "reactions": [{"name": "eyes", "count": 2}],
                "files": [{
                    "id": "F123",
                    "name": "note.txt",
                    "mimetype": "text/plain",
                    "size": 12,
                    "download_url": "https://files.slack.com/note.txt"
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
