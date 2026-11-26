use std::collections::HashSet;

use serde_json::{json, Value};

use crate::api::ZaloClient;
use crate::error::{ProtoError, Result};
use crate::types::{
    AddPollOptionsInput, CreatePollInput, PollOptionView, PollView, PollViewContext, PollVoter,
};

impl ZaloClient {
    /// Creates a poll in a group (`POST /api/poll/create`).
    pub async fn create_poll(&self, input: &CreatePollInput, group_id: &str) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "group_id": group_id,
            "question": input.question,
            "options": input.options,
            "expired_time": input.expires_at,
            "pinAct": false,
            "allow_multi_choices": input.allow_multiple,
            "allow_add_new_option": input.allow_add_options,
            "is_hide_vote_preview": input.hide_vote_preview,
            "is_anonymous": input.anonymous,
            "poll_type": 0,
            "src": 1,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/poll/create"), &params, &[])
            .await
    }

    /// Fetches poll detail (`POST /api/poll/detail`).
    pub async fn get_poll_detail(&self, poll_id: i64) -> Result<Value> {
        if poll_id <= 0 {
            return Err(ProtoError::InvalidArg("Missing poll id".into()));
        }
        let base = self.require_service("group")?;
        let params = json!({
            "poll_id": poll_id,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/poll/detail"), &params, &[])
            .await
    }

    /// Votes (or unvotes with empty `option_ids`) on a poll (`GET /api/poll/vote`).
    pub async fn vote_poll(&self, poll_id: i64, option_ids: &[i64]) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "poll_id": poll_id,
            "option_ids": option_ids,
            "imei": self.ctx.imei,
        });
        self.http
            .get_encrypted(&self.ctx, &format!("{base}/api/poll/vote"), &params, &[])
            .await
    }

    /// Adds new options to a poll (`GET /api/poll/option/add`).
    pub async fn add_poll_options(&self, input: &AddPollOptionsInput) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "poll_id": input.poll_id,
            "new_options": serde_json::to_string(&input.options)?,
            "voted_option_ids": input.voted_option_ids,
        });
        self.http
            .get_encrypted(
                &self.ctx,
                &format!("{base}/api/poll/option/add"),
                &params,
                &[],
            )
            .await
    }

    /// Locks/closes a poll (`POST /api/poll/end`).
    pub async fn lock_poll(&self, poll_id: i64) -> Result<Value> {
        let base = self.require_service("group")?;
        let params = json!({
            "poll_id": poll_id,
            "imei": self.ctx.imei,
        });
        self.http
            .post_encrypted(&self.ctx, &format!("{base}/api/poll/end"), &params, &[])
            .await
    }
}

/// Converts Zalo's raw `PollDetail` JSON into [`PollView`] (exact port of `src/polls.ts:buildPollView`).
pub fn build_poll_view(detail: &Value, ctx: &PollViewContext<'_>) -> PollView {
    let expires_raw = val_i64(detail.get("expired_time"));
    let expires_at = if expires_raw > 0 { expires_raw } else { 0 };
    let expired = expires_at > 0 && expires_at <= ctx.now;
    let closed = val_bool(detail.get("closed")) || expired;

    let options_arr = detail
        .get("options")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let my_votes: Vec<i64> = options_arr
        .iter()
        .filter(|o| val_bool(o.get("voted")))
        .map(|o| val_i64(o.get("option_id")))
        .collect();
    let has_voted = !my_votes.is_empty() || val_bool(detail.get("joined"));
    let results_hidden = val_bool(detail.get("is_hide_vote_preview")) && !has_voted && !closed;
    let anonymous = val_bool(detail.get("is_anonymous"));

    let name_of = |id: &str| -> String {
        ctx.names
            .and_then(|m| m.get(id))
            .cloned()
            .unwrap_or_else(|| id.to_string())
    };

    let mut everyone: HashSet<String> = HashSet::new();
    let mut views = Vec::with_capacity(options_arr.len());

    for o in options_arr {
        let voter_ids: Vec<String> = o
            .get("voters")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().map(val_str).collect())
            .unwrap_or_default();
        for v in &voter_ids {
            everyone.insert(v.clone());
        }
        let voters = if anonymous || results_hidden {
            Vec::new()
        } else {
            voter_ids
                .into_iter()
                .map(|id| {
                    let name = name_of(&id);
                    PollVoter { id, name }
                })
                .collect()
        };
        views.push(PollOptionView {
            id: val_i64(o.get("option_id")),
            text: o
                .get("content")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            votes: if results_hidden {
                0
            } else {
                val_i64(o.get("votes")).max(0) as u64
            },
            voted: val_bool(o.get("voted")),
            voters,
        });
    }

    let creator_id = detail.get("creator").map(val_str).unwrap_or_default();
    let is_creator = ctx
        .own_id
        .map(|own| !own.is_empty() && own == creator_id)
        .unwrap_or(false);
    let allow_add_options = val_bool(detail.get("allow_add_new_option"));

    PollView {
        poll_id: val_i64(detail.get("poll_id")),
        question: detail
            .get("question")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        creator_name: name_of(&creator_id),
        creator_id,
        closed,
        expired,
        expires_at,
        created_at: val_i64(detail.get("created_time")),
        allow_multiple: val_bool(detail.get("allow_multi_choices")),
        allow_add_options,
        anonymous,
        results_hidden,
        total_votes: if results_hidden {
            0
        } else {
            val_i64(detail.get("num_vote")).max(0) as u64
        },
        voter_count: if results_hidden { 0 } else { everyone.len() },
        my_votes,
        can_vote: !closed,
        can_add_options: !closed && allow_add_options,
        can_close: !closed && (is_creator || ctx.is_admin),
        options: views,
    }
}

/// Extracts a poll ID from a group board event (`data.groupTopic.type == 3`, exact port of `src/polls.ts:pollIdOfGroupEvent`).
pub fn poll_id_of_group_event(event: &Value) -> Option<i64> {
    let topic = event.get("data")?.get("groupTopic")?;
    if val_i64(topic.get("type")) != 3 {
        return None;
    }
    let params_val = match topic.get("params")? {
        Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
        other => other.clone(),
    };
    let id = params_val
        .get("pollId")
        .or_else(|| params_val.get("poll_id"))
        .map(|v| val_i64(Some(v)))
        .unwrap_or(0);
    if id > 0 {
        Some(id)
    } else {
        None
    }
}

fn val_i64(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_u64().map(|u| u as i64))
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Value::String(s)) => s.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn val_bool(v: Option<&Value>) -> bool {
    match v {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0) != 0,
        _ => false,
    }
}

fn val_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn build_poll_view_hidden_results_and_expiry() {
        let detail = json!({
            "poll_id": 42,
            "question": "Lunch?",
            "creator": "100",
            "closed": false,
            "expired_time": 2000,
            "created_time": 1000,
            "allow_multi_choices": true,
            "allow_add_new_option": true,
            "is_anonymous": false,
            "is_hide_vote_preview": true,
            "joined": false,
            "num_vote": 3,
            "options": [
                { "option_id": 1, "content": "Pho", "votes": 2, "voted": false, "voters": ["200", "300"] },
                { "option_id": 2, "content": "Bun", "votes": 1, "voted": false, "voters": ["200"] }
            ]
        });

        let mut names = HashMap::new();
        names.insert("100".to_string(), "Alice".to_string());
        names.insert("200".to_string(), "Bob".to_string());

        // Before voting and before expiry -> results hidden
        let view = build_poll_view(
            &detail,
            &PollViewContext {
                own_id: Some("100"),
                is_admin: false,
                now: 1500,
                names: Some(&names),
            },
        );
        assert!(view.results_hidden);
        assert!(!view.closed);
        assert!(!view.expired);
        assert!(view.can_vote);
        assert!(view.can_add_options);
        assert!(view.can_close); // creator
        assert_eq!(view.creator_name, "Alice");
        assert_eq!(view.total_votes, 0);
        assert_eq!(view.voter_count, 0);
        assert_eq!(view.options[0].votes, 0);
        assert!(view.options[0].voters.is_empty());

        // Check exact camelCase JSON keys matching src/polls.ts
        let serialized = serde_json::to_value(&view).unwrap();
        assert_eq!(serialized["pollId"], 42);
        assert_eq!(serialized["creatorId"], "100");
        assert_eq!(serialized["resultsHidden"], true);
        assert_eq!(serialized["canClose"], true);

        // After expiry -> expired & closed & results revealed
        let expired_view = build_poll_view(
            &detail,
            &PollViewContext {
                own_id: Some("999"),
                is_admin: false,
                now: 2500,
                names: Some(&names),
            },
        );
        assert!(expired_view.expired);
        assert!(expired_view.closed);
        assert!(!expired_view.results_hidden);
        assert!(!expired_view.can_vote);
        assert!(!expired_view.can_close);
        assert_eq!(expired_view.total_votes, 3);
        assert_eq!(expired_view.voter_count, 2); // unique voters "200", "300"
        assert_eq!(expired_view.options[0].votes, 2);
        assert_eq!(expired_view.options[0].voters[0].name, "Bob");
        assert_eq!(expired_view.options[0].voters[1].name, "300");
    }

    #[test]
    fn poll_id_of_group_event_parses_string_and_object_params() {
        let ev_str = json!({
            "type": "update_board",
            "data": {
                "groupTopic": {
                    "type": 3,
                    "params": "{\"pollId\":12345}"
                }
            }
        });
        assert_eq!(poll_id_of_group_event(&ev_str), Some(12345));

        let ev_obj = json!({
            "data": {
                "groupTopic": {
                    "type": 3,
                    "params": { "poll_id": 67890 }
                }
            }
        });
        assert_eq!(poll_id_of_group_event(&ev_obj), Some(67890));

        let ev_other = json!({
            "data": {
                "groupTopic": {
                    "type": 1,
                    "params": { "pollId": 111 }
                }
            }
        });
        assert_eq!(poll_id_of_group_event(&ev_other), None);
    }
}
