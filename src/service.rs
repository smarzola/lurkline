use std::{collections::HashSet, sync::Arc};

use async_trait::async_trait;

use crate::{
    config::Config,
    error::{Error, Result},
    model::{
        ClientCountsPayload, ConversationKind, DoctorReport, FileReference, Message, MessagePage,
        RawMessage, RawMessagePage, RawMessagesList, RawUnread, RawUser, RawUsersPage, Reaction,
        ThreadPage, UnreadConversation, UnreadReport, UnreadThreads, User, UserSearchReport,
    },
};

const MAX_MESSAGES: usize = 200;
const MAX_USERS: usize = 100;
const USERS_PAGE_SIZE: usize = 200;
const MAX_USER_PAGES: usize = 20;

#[async_trait]
pub(crate) trait SlackApi: Send + Sync {
    async fn client_counts(&self) -> Result<ClientCountsPayload>;
    async fn conversation_history(&self, channel: &str, limit: usize) -> Result<RawMessagePage>;
    async fn conversation_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        limit: usize,
    ) -> Result<RawMessagePage>;
    async fn messages_list(&self, channel: &str, message_ts: &str) -> Result<RawMessagesList>;
    async fn users_list(&self, cursor: Option<&str>, limit: usize) -> Result<RawUsersPage>;
}

#[derive(Clone)]
pub(crate) struct SlackService {
    api: Arc<dyn SlackApi>,
    team_id: String,
    workspace_url: String,
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
        let mut conversations = Vec::new();
        append_unreads(
            &mut conversations,
            counts.channels,
            ConversationKind::Channel,
        );
        append_unreads(
            &mut conversations,
            counts.ims,
            ConversationKind::DirectMessage,
        );
        append_unreads(
            &mut conversations,
            counts.mpims,
            ConversationKind::GroupDirectMessage,
        );
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

    pub(crate) async fn read_channel(&self, channel: &str, limit: usize) -> Result<MessagePage> {
        validate_channel(channel)?;
        validate_limit("limit", limit, MAX_MESSAGES)?;
        let raw = self.api.conversation_history(channel, limit).await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor = nonempty(raw.response_metadata.next_cursor);
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
        limit: usize,
    ) -> Result<ThreadPage> {
        validate_channel(channel)?;
        validate_timestamp("thread_ts", thread_ts)?;
        validate_limit("limit", limit, MAX_MESSAGES)?;
        let raw = self
            .api
            .conversation_replies(channel, thread_ts, limit)
            .await?;
        let locally_truncated = raw.messages.len() > limit;
        let next_cursor = nonempty(raw.response_metadata.next_cursor);
        Ok(ThreadPage {
            channel_id: channel.to_owned(),
            thread_ts: thread_ts.to_owned(),
            messages: normalize_messages(channel, raw.messages, limit, "conversations.replies")?,
            has_more: raw.has_more || next_cursor.is_some() || locally_truncated,
            next_cursor,
        })
    }

    pub(crate) async fn get_message(&self, channel: &str, message_ts: &str) -> Result<Message> {
        validate_channel(channel)?;
        validate_timestamp("message_ts", message_ts)?;
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
        candidates
            .into_iter()
            .find(|message| message.ts == message_ts)
            .map(|message| normalize_message(channel, message))
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

        for page_index in 0..MAX_USER_PAGES {
            let page = self
                .api
                .users_list(cursor.as_deref(), USERS_PAGE_SIZE)
                .await?;
            for raw_user in page.members {
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
                        });
                    }
                    users.push(normalize_user(raw_user));
                }
            }
            let next = nonempty(page.response_metadata.next_cursor);
            let Some(next) = next else {
                return Ok(UserSearchReport {
                    query,
                    users,
                    truncated: false,
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
                });
            }
        }
        unreachable!("bounded user page loop always returns")
    }
}

fn append_unreads(
    target: &mut Vec<UnreadConversation>,
    source: Vec<RawUnread>,
    kind: ConversationKind,
) {
    target.extend(
        source
            .into_iter()
            .filter(|entry| entry.has_unreads)
            .map(|entry| UnreadConversation {
                id: entry.id,
                kind,
                has_unreads: entry.has_unreads,
                mention_count: entry.mention_count,
                last_read: entry.last_read,
                latest: entry.latest,
            }),
    );
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

fn validate_channel(channel: &str) -> Result<()> {
    if !(2..=64).contains(&channel.len())
        || !matches!(channel.as_bytes().first(), Some(b'C' | b'D' | b'G'))
        || !channel.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::invalid_input(
            "channel_id",
            "must be a Slack channel, DM, or group-DM ID",
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

fn validate_limit(field: &'static str, limit: usize, maximum: usize) -> Result<()> {
    if !(1..=maximum).contains(&limit) {
        return Err(Error::invalid_input(field, "is outside the allowed range"));
    }
    Ok(())
}

fn nonempty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
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
        RawChannelMessages, RawFile, RawReaction, RawResponseMetadata, RawThreadCounts, RawUnread,
        RawUserProfile,
    };

    struct FakeApi {
        counts: ClientCountsPayload,
        history: RawMessagePage,
        replies: RawMessagePage,
        message_list: RawMessagesList,
        user_pages: Mutex<VecDeque<RawUsersPage>>,
    }

    #[async_trait]
    impl SlackApi for FakeApi {
        async fn client_counts(&self) -> Result<ClientCountsPayload> {
            Ok(self.counts.clone())
        }

        async fn conversation_history(
            &self,
            _channel: &str,
            _limit: usize,
        ) -> Result<RawMessagePage> {
            Ok(self.history.clone())
        }

        async fn conversation_replies(
            &self,
            _channel: &str,
            _thread_ts: &str,
            _limit: usize,
        ) -> Result<RawMessagePage> {
            Ok(self.replies.clone())
        }

        async fn messages_list(
            &self,
            _channel: &str,
            _message_ts: &str,
        ) -> Result<RawMessagesList> {
            Ok(self.message_list.clone())
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
            _limit: usize,
        ) -> Result<RawMessagePage> {
            Err(Error::Authentication)
        }

        async fn conversation_replies(
            &self,
            _channel: &str,
            _thread_ts: &str,
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

    #[tokio::test]
    async fn normalizes_only_explicit_slack_unreads_across_kinds() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![entry("C_READ", false, 9), entry("C_UNREAD", true, 1)],
            ims: vec![entry("D_UNREAD", true, 3)],
            mpims: vec![entry("G_UNREAD", true, 0)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 2,
                unread_count_by_channel: BTreeMap::from([("C_UNREAD".into(), 4)]),
            },
        };
        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            report
                .conversations
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["D_UNREAD", "C_UNREAD", "G_UNREAD"]
        );
        assert_eq!(
            report.conversations[0].kind,
            ConversationKind::DirectMessage
        );
        assert!(report.threads.has_unreads);
        assert_eq!(report.threads.mention_count, 2);
    }

    #[tokio::test]
    async fn unread_json_schema_is_stable_and_typed() {
        let mut api = fake_api();
        api.counts = ClientCountsPayload {
            channels: vec![RawUnread {
                id: "C_NULL".into(),
                has_unreads: true,
                mention_count: 0,
                last_read: None,
                latest: None,
            }],
            ims: vec![entry("D_ONE", true, 1)],
            mpims: vec![entry("G_ONE", true, 1)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 3,
                unread_count_by_channel: BTreeMap::from([
                    ("C_NULL".into(), 2),
                    ("D_ONE".into(), 1),
                ]),
            },
        };
        let report = service(api).unreads().await.unwrap();

        assert_eq!(
            serde_json::to_value(report).unwrap(),
            serde_json::json!({
                "team_id": "T000TEST",
                "conversations": [
                    {
                        "id": "D_ONE",
                        "kind": "direct_message",
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "G_ONE",
                        "kind": "group_direct_message",
                        "has_unreads": true,
                        "mention_count": 1,
                        "last_read": "100.0",
                        "latest": "200.0"
                    },
                    {
                        "id": "C_NULL",
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
                        "C_NULL": 2,
                        "D_ONE": 1
                    }
                }
            })
        );
    }

    #[tokio::test]
    async fn doctor_probes_the_api() {
        assert!(matches!(
            service(FailApi).doctor().await,
            Err(Error::Authentication)
        ));
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

        let page = service.read_channel("C123", 1).await.unwrap();
        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].text, "first");
        assert!(page.has_more);
        assert_eq!(page.next_cursor.as_deref(), Some("next"));

        let thread = service.read_thread("C123", "100.000001", 2).await.unwrap();
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

        let channel = service.read_channel("C123", 1).await.unwrap();
        assert_eq!(channel.messages.len(), 1);
        assert!(channel.has_more);
        assert_eq!(channel.next_cursor, None);

        let thread = service.read_thread("C123", "100.000001", 1).await.unwrap();
        assert_eq!(thread.messages.len(), 1);
        assert!(thread.has_more);
        assert_eq!(thread.next_cursor, None);
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
    async fn validates_all_external_inputs() {
        let service = service(fake_api());
        assert!(matches!(
            service.read_channel("bad", 1).await,
            Err(Error::InvalidInput {
                field: "channel_id",
                ..
            })
        ));
        assert!(matches!(
            service.read_thread("C123", "bad", 1).await,
            Err(Error::InvalidInput {
                field: "thread_ts",
                ..
            })
        ));
        assert!(matches!(
            service.read_channel("C123", 201).await,
            Err(Error::InvalidInput { field: "limit", .. })
        ));
        assert!(matches!(
            service.find_users("\n", 1).await,
            Err(Error::InvalidInput { field: "query", .. })
        ));
    }

    #[tokio::test]
    async fn rejects_empty_essential_response_identifiers() {
        let mut message_api = fake_api();
        message_api.history.messages = vec![raw_message("", "bad")];
        assert!(matches!(
            service(message_api).read_channel("C123", 1).await,
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
