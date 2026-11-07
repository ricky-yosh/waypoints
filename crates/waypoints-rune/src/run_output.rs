//! Structured run output: Markdown prose, reasoning, replay/retention notices,
//! and stable expandable tool rows.

use rune::{Color, El, ScrollSpec, Tx, col, el, row, text};
use rune_kit::{Button, ButtonVariant, Kit, Markdown};
use waypoints_backend::RunEventOrigin;
use waypoints_backend::activity::{Activity, ActivityBlock, Stream};

use crate::State;

pub fn view(s: &State, k: &Kit) -> El {
    let blocks = s.run_log.blocks();
    let active = s.selected_run().is_some_and(crate::run_is_active);
    let last = blocks.len().saturating_sub(1);
    let mut rows = Vec::new();

    if s.run_log.is_trimmed() {
        rows.push(notice(
            "run-trimmed-notice",
            "Older output was trimmed to keep this transcript bounded.",
            k,
            k.c.warning.fg,
        ));
    }
    if s.run_gap {
        rows.push(notice(
            "run-gap-notice",
            "Some output expired before it could be read; this is not a complete replay.",
            k,
            k.c.warning.fg,
        ));
    }

    let mut previous_replay = false;
    for (index, block) in blocks.iter().enumerate() {
        let replay = block.origin == RunEventOrigin::Replay;
        if replay && !previous_replay {
            rows.push(notice(
                format!("run-replay-notice-{}", block.sequence),
                "Replayed earlier output",
                k,
                k.c.text_faint,
            ));
        }
        previous_replay = replay;
        rows.push(block_view(s, block, index == last, active, k));
    }

    let mut column = col().w_full().grow(1.0).min_h(0.0).gap(4.0);
    if !s.run_output_at_bottom {
        column = column.child(
            row().w_full().justify_end().child(
                Button::new("Jump to latest")
                    .variant(ButtonVariant::GhostMuted)
                    .on_press_tx(|s: &mut State, tx: &mut Tx| {
                        tx.scroll_to_bottom_instant("run-output-log");
                        s.run_output_at_bottom = true;
                    })
                    .build(k)
                    .key("run-jump-latest"),
            ),
        );
    }
    column.child(
        col()
            .key("run-output-log")
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .padding(9.0)
            .gap(7.0)
            .radius(k.radius.md)
            .fill(k.c.layer_01)
            .border(1.0, k.c.border_base)
            .scroll(ScrollSpec { stick_bottom: true })
            .on_at_bottom(|s: &mut State, at| s.run_output_at_bottom = at)
            .children(rows),
    )
}

fn block_view(s: &State, block: &ActivityBlock, is_last: bool, active: bool, k: &Kit) -> El {
    let key = format!("run-block-{}", block.sequence);
    match &block.activity {
        Activity::Text(Stream::Assistant, text) => col().key(key).w_full().min_w(0.0).child(
            Markdown::new(text.clone())
                .selectable(true)
                .streaming(active && is_last)
                .build(k),
        ),
        Activity::Text(Stream::Thought, body) => col()
            .key(key)
            .w_full()
            .min_w(0.0)
            .gap(2.0)
            .child(
                text("Reasoning")
                    .font(k.ty.small_medium)
                    .color(k.c.text_faint),
            )
            .child(
                el().w_full().opacity(0.78).child(
                    Markdown::new(body.clone())
                        .selectable(true)
                        .streaming(active && is_last)
                        .build(k),
                ),
            ),
        Activity::Tool { id, fields } => tool_view(s, block.sequence, id, fields, k),
        Activity::Other(update) => other_view(s, block.sequence, update, k),
        _ => col().key(key).w_full().child(
            text("Activity updated")
                .font(k.ty.small)
                .color(k.c.text_muted),
        ),
    }
}

fn tool_view(s: &State, sequence: u64, id: &str, fields: &serde_json::Value, k: &Kit) -> El {
    let expanded = s.expanded_tools.contains(&sequence);
    let title = tool_heading(fields, "Tool call");
    let status = fields
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map(activity_status_label);
    let summary = tool_summary(fields);
    let status_color = fields
        .get("status")
        .and_then(serde_json::Value::as_str)
        .map_or(k.c.text_faint, |status| match status {
            "completed" => k.c.success.fg,
            "failed" | "error" => k.c.danger.fg,
            "in_progress" | "running" | "pending" => k.c.info.fg,
            _ => k.c.text_faint,
        });

    let toggle = Button::new(title)
        .variant(ButtonVariant::GhostMuted)
        .icon(if expanded {
            k.glyph.chevron_down
        } else {
            k.glyph.caret_right
        })
        .on_press(move |s: &mut State| toggle_tool(s, sequence))
        .build(k)
        .key(format!("run-tool-{sequence}"));

    let mut header = row().w_full().items_center().gap(6.0).child(toggle);
    if let Some(status) = status {
        header = header.child(text(status).font(k.ty.small).color(status_color));
    }
    if let Some(summary) = summary {
        header = header.child(
            text(summary)
                .font(k.ty.small)
                .color(k.c.text_muted)
                .truncate()
                .grow(1.0),
        );
    }

    let mut column = col()
        .key(format!("run-block-{sequence}"))
        .w_full()
        .min_w(0.0)
        .gap(3.0)
        .child(header);
    if expanded {
        column = column.child(
            col()
                .key(format!("run-tool-details-{sequence}"))
                .w_full()
                .child(
                    Markdown::new(fenced_code("json", &json_pretty(fields)))
                        .selectable(true)
                        .build(k),
                ),
        );
    }
    column.label(format!("Tool {id}"))
}

fn other_view(s: &State, sequence: u64, update: &serde_json::Value, k: &Kit) -> El {
    let expanded = s.expanded_tools.contains(&sequence);
    let (title, summary) = other_heading(update);
    let toggle = Button::new(title)
        .variant(ButtonVariant::GhostMuted)
        .icon(if expanded {
            k.glyph.chevron_down
        } else {
            k.glyph.caret_right
        })
        .on_press(move |s: &mut State| toggle_tool(s, sequence))
        .build(k)
        .key(format!("run-other-{sequence}"));
    let mut header = row().w_full().items_center().gap(6.0).child(toggle);
    if let Some(summary) = summary {
        header = header.child(
            text(summary)
                .font(k.ty.small)
                .color(k.c.text_faint)
                .truncate()
                .grow(1.0),
        );
    }
    let mut column = col()
        .key(format!("run-block-{sequence}"))
        .w_full()
        .min_w(0.0)
        .gap(3.0)
        .child(header);
    if expanded {
        column = column.child(
            col()
                .key(format!("run-other-details-{sequence}"))
                .w_full()
                .child(
                    Markdown::new(fenced_code("json", &json_pretty(update)))
                        .selectable(true)
                        .build(k),
                ),
        );
    }
    column
}

fn toggle_tool(state: &mut State, sequence: u64) {
    if !state.expanded_tools.insert(sequence) {
        state.expanded_tools.remove(&sequence);
    }
}

fn notice(key: impl Into<String>, label: &str, k: &Kit, color: Color) -> El {
    row()
        .key(key)
        .w_full()
        .gap(5.0)
        .items_center()
        .child(text("•").font(k.ty.small).color(color))
        .child(text(label).font(k.ty.small).color(color).w_full().wrap())
}

fn tool_heading(fields: &serde_json::Value, fallback: &str) -> String {
    if let Some(title) = fields.get("title").and_then(serde_json::Value::as_str) {
        return title.to_owned();
    }
    fields
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .map_or_else(|| fallback.to_owned(), humanize)
}

fn humanize(value: &str) -> String {
    let text = value.trim().replace('_', " ");
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + characters.as_str(),
        None => String::new(),
    }
}

fn activity_status_label(status: &str) -> String {
    match status {
        "pending" => "Pending".into(),
        "in_progress" | "running" => "In progress".into(),
        "completed" => "Completed".into(),
        "failed" | "error" => "Failed".into(),
        other => humanize(other),
    }
}

fn tool_summary(fields: &serde_json::Value) -> Option<String> {
    if let Some(paths) = fields
        .get("locations")
        .and_then(serde_json::Value::as_array)
    {
        let paths = paths
            .iter()
            .filter_map(|location| location.get("path").and_then(serde_json::Value::as_str))
            .collect::<Vec<_>>();
        if let Some(first) = paths.first() {
            return Some(if paths.len() > 1 {
                format!("{first} +{}", paths.len() - 1)
            } else {
                (*first).to_owned()
            });
        }
    }
    if let Some(command) = fields
        .pointer("/rawInput/command")
        .and_then(serde_json::Value::as_str)
        .and_then(|command| command.lines().find(|line| !line.trim().is_empty()))
    {
        return Some(truncate(command, 120));
    }
    if let Some(kind) = fields.get("kind").and_then(serde_json::Value::as_str) {
        return Some(humanize(kind));
    }
    if let Some(items) = fields.get("content").and_then(serde_json::Value::as_array) {
        let text = items.iter().find_map(|item| {
            item.pointer("/content/text")
                .and_then(serde_json::Value::as_str)
                .or_else(|| item.get("text").and_then(serde_json::Value::as_str))
        });
        if let Some(line) = text.and_then(|text| text.lines().find(|line| !line.trim().is_empty()))
        {
            return Some(truncate(line, 120));
        }
        if !items.is_empty() {
            return Some(format!(
                "{} content {}",
                items.len(),
                if items.len() == 1 { "item" } else { "items" }
            ));
        }
    }
    None
}

fn other_heading(update: &serde_json::Value) -> (String, Option<String>) {
    match update
        .get("sessionUpdate")
        .and_then(serde_json::Value::as_str)
    {
        Some("plan") => (
            "Plan updated".into(),
            Some(format!(
                "{} steps",
                update
                    .get("entries")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len)
            )),
        ),
        Some("available_commands_update") => (
            "Available commands updated".into(),
            update
                .get("availableCommands")
                .and_then(serde_json::Value::as_array)
                .map(|commands| format!("{} commands", commands.len())),
        ),
        Some("usage_update") => {
            let used = update.get("used").and_then(serde_json::Value::as_u64);
            let size = update.get("size").and_then(serde_json::Value::as_u64);
            (
                "Usage updated".into(),
                match (used, size) {
                    (Some(used), Some(size)) => Some(format!("{used} of {size} tokens")),
                    (Some(used), None) => Some(format!("{used} tokens used")),
                    _ => None,
                },
            )
        }
        Some("session_info_update") => (
            "Session details updated".into(),
            update
                .get("title")
                .and_then(serde_json::Value::as_str)
                .filter(|title| !title.trim().is_empty())
                .map(str::to_owned),
        ),
        Some(name) => (humanize(name), None),
        None => ("Activity updated".into(), None),
    }
}

fn json_pretty(value: &serde_json::Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

fn fenced_code(language: &str, code: &str) -> String {
    let mut fence = 3;
    let mut run = 0;
    for character in code.chars() {
        if character == '`' {
            run += 1;
            fence = fence.max(run + 1);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat(fence);
    format!(
        "{fence}{language}\n{}\n{fence}",
        code.trim_end_matches('\n')
    )
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}
