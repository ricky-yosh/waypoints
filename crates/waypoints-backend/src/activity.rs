//! Bounded, transient ACP activity assembly for headless and graphical clients.

use std::collections::VecDeque;

use crate::{RunEvent, RunEventOrigin};

/// Maximum retained serialized activity bytes per client log.
pub const ACTIVITY_BYTES: usize = 128 * 1024;
/// Maximum retained activity blocks per client log.
pub const ACTIVITY_BLOCKS: usize = 512;

/// Independent agent text channels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Assistant,
    Thought,
}

/// Normalized activity, retaining unknown payloads rather than inventing semantics.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum Activity {
    Text(Stream, String),
    Tool { id: String, fields: serde_json::Value },
    Other(serde_json::Value),
}

/// A retained activity block; its identity is the first contributing event.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ActivityBlock {
    pub sequence: u64,
    pub origin: RunEventOrigin,
    pub activity: Activity,
}

/// Changes to a log's rows, suitable for adapting a retained scrolling view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ActivityDelta {
    /// Leading rows removed by retention.
    pub removed: usize,
    /// New rows appended after retention.
    pub appended: usize,
    /// Existing row indices needing remeasurement, after leading removal.
    pub changed: Vec<usize>,
}

/// Client-owned bounded activity log; no transcript is persisted.
#[derive(Debug, Clone, Default)]
pub struct ActivityLog {
    blocks: VecDeque<ActivityBlock>,
    break_stream: bool,
    trimmed: bool,
}

impl ActivityLog {
    /// Retained rows in event order.
    pub fn blocks(&self) -> &VecDeque<ActivityBlock> { &self.blocks }

    /// Whether retention has omitted any content since the log was created.
    pub fn is_trimmed(&self) -> bool { self.trimmed }

    /// Applies a poll page. A gap prevents merging text across missing events.
    pub fn apply(&mut self, events: Vec<RunEvent>, gap: bool) -> ActivityDelta {
        self.break_stream |= gap;
        let previous: Vec<_> = self.blocks.iter().map(|block| block.sequence).collect();
        let mut changed = Vec::new();
        for event in events {
            let sequence = self.push(event.sequence, event.origin, event.update);
            if let Ok(index) = previous.binary_search(&sequence) { changed.push(index); }
        }
        let removed = self.blocks.front().map_or(previous.len(), |first| {
            previous.partition_point(|sequence| *sequence < first.sequence)
        });
        let appended = self.blocks.len() - (previous.len() - removed);
        changed.sort_unstable();
        changed.dedup();
        ActivityDelta {
            removed,
            appended,
            changed: changed.into_iter().filter_map(|index| index.checked_sub(removed)).collect(),
        }
    }

    /// Applies one event and returns the identity of its affected block.
    pub fn push(&mut self, sequence: u64, origin: RunEventOrigin, update: serde_json::Value) -> u64 {
        let mut changed = sequence;
        let kind = update.get("sessionUpdate").and_then(serde_json::Value::as_str);
        let stream = match kind {
            Some("agent_message_chunk") => Some(Stream::Assistant),
            Some("agent_thought_chunk") => Some(Stream::Thought),
            _ => None,
        };
        if let Some(stream) = stream
            && let Some(text) = update.pointer("/content/text").and_then(serde_json::Value::as_str)
        {
            if !self.break_stream
                && let Some(previous) = self.blocks.back_mut()
                && previous.origin == origin
                && let Activity::Text(previous_stream, previous_text) = &mut previous.activity
                && *previous_stream == stream
            {
                previous_text.push_str(text);
                changed = previous.sequence;
            } else {
                self.add(sequence, origin, Activity::Text(stream, text.into()));
            }
            self.break_stream = false;
        } else if matches!(kind, Some("tool_call" | "tool_call_update"))
            && let Some(id) = update.get("toolCallId").and_then(serde_json::Value::as_str)
        {
            self.break_stream = true;
            let existing = self.blocks.iter_mut().rev().find(|block| {
                block.origin == origin && matches!(&block.activity, Activity::Tool { id: existing, .. } if existing == id)
            });
            if let Some(block) = existing {
                changed = block.sequence;
                if let Activity::Tool { fields, .. } = &mut block.activity
                    && let (Some(fields), Some(update)) = (fields.as_object_mut(), update.as_object())
                {
                    fields.extend(update.iter().map(|(key, value)| (key.clone(), value.clone())));
                }
            } else {
                self.add(sequence, origin, Activity::Tool { id: id.to_owned(), fields: update });
            }
        } else {
            self.break_stream = true;
            self.add(sequence, origin, Activity::Other(update));
        }
        self.trim();
        changed
    }

    fn add(&mut self, sequence: u64, origin: RunEventOrigin, activity: Activity) {
        self.blocks.push_back(ActivityBlock { sequence, origin, activity });
    }

    fn trim(&mut self) {
        let mut remaining = ACTIVITY_BYTES;
        let mut keep = 0;
        for block in self.blocks.iter_mut().rev().take(ACTIVITY_BLOCKS) {
            let size = match &mut block.activity {
                Activity::Text(_, text) => {
                    if keep == 0 && text.len() > remaining {
                        text.truncate(text.floor_char_boundary(remaining));
                        self.trimmed = true;
                        self.break_stream = true;
                    }
                    text.len()
                }
                Activity::Tool { fields, .. } | Activity::Other(fields) => fields.to_string().len(),
            };
            if size > remaining { break; }
            remaining -= size;
            keep += 1;
        }
        let drop = self.blocks.len() - keep;
        if drop > 0 {
            self.blocks.drain(..drop);
            self.trimmed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(sequence: u64, origin: RunEventOrigin, text: &str) -> RunEvent {
        RunEvent { sequence, origin, update: json!({"sessionUpdate":"agent_message_chunk","content":{"text":text}}) }
    }

    #[test]
    fn merges_text_but_keeps_replay_and_gaps_distinct() {
        let mut log = ActivityLog::default();
        let delta = log.apply(vec![text(1, RunEventOrigin::Replay, "old"), text(2, RunEventOrigin::Live, "a")], false);
        assert_eq!(delta.appended, 2);
        let delta = log.apply(vec![text(3, RunEventOrigin::Live, "b")], false);
        assert_eq!(delta.changed, vec![1]);
        assert_eq!(log.blocks()[1].activity, Activity::Text(Stream::Assistant, "ab".into()));
        log.apply(vec![text(5, RunEventOrigin::Live, "after gap")], true);
        assert_eq!(log.blocks().len(), 3);
    }

    #[test]
    fn merges_tools_by_id_and_origin_and_bounds_unknown_payloads() {
        let mut log = ActivityLog::default();
        log.push(1, RunEventOrigin::Live, json!({"sessionUpdate":"tool_call","toolCallId":"a","title":"Test"}));
        log.push(2, RunEventOrigin::Live, json!({"sessionUpdate":"tool_call_update","toolCallId":"a","status":"completed"}));
        assert!(matches!(&log.blocks()[0].activity, Activity::Tool { fields, .. } if fields["title"] == "Test" && fields["status"] == "completed"));
        log.push(3, RunEventOrigin::Replay, json!({"sessionUpdate":"tool_call","toolCallId":"a"}));
        assert_eq!(log.blocks().len(), 2);
        for sequence in 4..600 {
            log.push(sequence, RunEventOrigin::Live, json!({"future":"unknown"}));
        }
        assert_eq!(log.blocks().len(), ACTIVITY_BLOCKS);
        assert!(log.is_trimmed());
        log.push(600, RunEventOrigin::Live, json!({"huge":"x".repeat(ACTIVITY_BYTES + 1)}));
        assert!(log.blocks().is_empty());
    }
}
