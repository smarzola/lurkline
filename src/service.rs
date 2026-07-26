use async_trait::async_trait;

use crate::{
    config::Config,
    error::Result,
    model::{
        ClientCountsPayload, ConversationKind, DoctorReport, RawUnread, UnreadConversation,
        UnreadReport, UnreadThreads,
    },
};

#[async_trait]
pub(crate) trait SlackApi: Send + Sync {
    async fn client_counts(&self) -> Result<ClientCountsPayload>;
}

pub(crate) struct SlackService<A> {
    api: A,
    team_id: String,
    workspace_url: String,
}

impl<A> SlackService<A>
where
    A: SlackApi,
{
    pub(crate) fn new(api: A, config: &Config) -> Self {
        Self {
            api,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use url::Url;

    use super::*;
    use crate::{
        error::Error,
        model::{RawThreadCounts, RawUnread},
    };

    struct FakeApi {
        result: Arc<std::result::Result<ClientCountsPayload, Error>>,
    }

    #[async_trait]
    impl SlackApi for FakeApi {
        async fn client_counts(&self) -> Result<ClientCountsPayload> {
            match self.result.as_ref() {
                Ok(value) => Ok(value.clone()),
                Err(_) => Err(Error::Authentication),
            }
        }
    }

    fn service(payload: ClientCountsPayload) -> SlackService<FakeApi> {
        let config = Config::for_test(Url::parse("http://127.0.0.1:1234").unwrap(), 1024);
        SlackService::new(
            FakeApi {
                result: Arc::new(Ok(payload)),
            },
            &config,
        )
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

    #[tokio::test]
    async fn normalizes_only_explicit_slack_unreads_across_kinds() {
        let report = service(ClientCountsPayload {
            channels: vec![entry("C_READ", false, 9), entry("C_UNREAD", true, 1)],
            ims: vec![entry("D_UNREAD", true, 3)],
            mpims: vec![entry("G_UNREAD", true, 0)],
            threads: RawThreadCounts {
                has_unreads: true,
                mention_count: 2,
                unread_count_by_channel: std::collections::BTreeMap::from([("C_UNREAD".into(), 4)]),
            },
        })
        .unreads()
        .await
        .unwrap();

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
        let report = service(ClientCountsPayload {
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
                unread_count_by_channel: std::collections::BTreeMap::from([
                    ("C_NULL".into(), 2),
                    ("D_ONE".into(), 1),
                ]),
            },
        })
        .unreads()
        .await
        .unwrap();

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
        let config = Config::for_test(Url::parse("http://127.0.0.1:1234").unwrap(), 1024);
        let service = SlackService::new(
            FakeApi {
                result: Arc::new(Err(Error::Authentication)),
            },
            &config,
        );
        assert!(matches!(service.doctor().await, Err(Error::Authentication)));
    }
}
