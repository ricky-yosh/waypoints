//! Rune canvas frontend for Waypoints.
//!
//! With no `--project`, the app runs the offline fixture canvas described in
//! `docs/port-info`. With `--project PATH`, it talks only to the existing
//! `waypoints_backend::Client`; it never opens the database or invokes jj.

mod activity;
mod backend;
mod canvas;
mod diff_review;
mod fixture;
mod history_view;
mod preferences;
mod project;
mod run_output;
mod settings;
mod startup;
mod state;
mod workbench;

pub(crate) use activity::run_is_active;
pub(crate) use state::*;

use rune::tok;
use rune::{AppSpec, Cx, El, Key, KeyEvent, MenuItem, Mods, Shortcut, Tx, el};
use rune_kit::Kit;

use activity::{poll, seed_output};
use startup::{init_state, setup, startup};
use state::{Mode, State};

pub(crate) const WINDOW_SIZE: (f32, f32) = (1280.0, 820.0);
const WINDOW_MIN_SIZE: (f32, f32) = (720.0, 480.0);

fn on_key(s: &mut State, event: &KeyEvent, tx: &mut Tx) -> bool {
    if s.settings_open {
        if matches!(event.key, Key::Escape) {
            s.settings_open = false;
            return true;
        }
        return false;
    }
    if s.workbench_open && s.review.tab == WorkbenchTab::Changes && s.review.diff.key(event, tx) {
        return true;
    }

    if let Key::Command('b') = event.key
        && s.mode == Mode::Backend
        && !s.display_fixture
    {
        s.toggle_sidebar();
        return true;
    }

    if let Key::Command('o') = event.key
        && s.mode == Mode::Backend
        && !s.display_fixture
    {
        s.set_sidebar_open(true);
        project::open_add_dialog(tx);
        return true;
    }

    if let Key::Command(digit @ '1'..='4') = event.key
        && let Some(node_id) = s.selected
    {
        s.workbench_open = true;
        s.ensure_draft_for(node_id);
        s.set_workbench_tab(match digit {
            '1' => WorkbenchTab::Assignment,
            '2' => WorkbenchTab::Output,
            '3' => WorkbenchTab::Changes,
            _ => WorkbenchTab::History,
        });
        return true;
    }

    if s.workbench_open {
        if s.review.tab == WorkbenchTab::History
            && !event.mods.cmd
            && !event.mods.ctrl
            && !event.mods.alt
            && let Key::Text(value) = &event.key
        {
            match value.as_str() {
                "r" => {
                    s.set_history_kind(crate::state::HistoryKind::Runs);
                    return true;
                }
                "n" => {
                    s.set_history_kind(crate::state::HistoryKind::Revisions);
                    return true;
                }
                _ => {}
            }
        }
        if event.is_command('j') {
            tx.scroll_to_bottom_instant("run-output-log");
            s.run_output_at_bottom = true;
            return true;
        }
    }

    if event.is_command('r') {
        if s.workbench_open {
            match s.review.tab {
                WorkbenchTab::History => match s.review.history.kind {
                    crate::state::HistoryKind::Runs => s.load_history_runs(true),
                    crate::state::HistoryKind::Revisions => s.load_history_revisions(true),
                },
                WorkbenchTab::Changes => s.request_review(s.review.target.clone()),
                _ => s.load_workflows(),
            }
            return true;
        }
        if s.mode == Mode::Backend {
            s.load_workflows();
            s.load_graph();
            return true;
        }
        return false;
    }

    if matches!(event.key, Key::Enter)
        && !s.workbench_open
        && s.selected.is_some()
        && tx
            .focused()
            .is_some_and(|focused| focused.starts_with("node-") || focused.contains("/node-"))
    {
        if event.mods.shift {
            s.preview_node = if s.preview_node == s.selected {
                None
            } else {
                s.selected
            };
            return true;
        }
        s.preview_node = None;
        s.workbench_open = true;
        if let Some(node_id) = s.selected {
            s.ensure_draft_for(node_id);
        }
        return true;
    }

    match &event.key {
        Key::Escape => {
            if s.settings_open {
                s.settings_open = false;
                true
            } else if s.workbench_open {
                s.workbench_open = false;
                true
            } else if s.preview_node.take().is_some() {
                true
            } else {
                s.selected.take().is_some()
            }
        }
        Key::Command(',') => {
            s.settings_open = true;
            true
        }
        Key::Command('0') => {
            s.camera.reset();
            true
        }
        Key::Command('=') | Key::Command('+') => {
            let zoom = s.preferences.interface_zoom;
            s.camera.zoom_about(
                canvas::camera_center([WINDOW_SIZE.0 / zoom, WINDOW_SIZE.1 / zoom]),
                1.2,
            );
            true
        }
        Key::Command('-') => {
            let zoom = s.preferences.interface_zoom;
            s.camera.zoom_about(
                canvas::camera_center([WINDOW_SIZE.0 / zoom, WINDOW_SIZE.1 / zoom]),
                1.0 / 1.2,
            );
            true
        }
        Key::Text(value) if value == " " && plain(event.mods) => {
            s.space_held = true;
            true
        }
        _ => false,
    }
}

fn plain(mods: Mods) -> bool {
    !mods.shift && !mods.cmd && !mods.alt && !mods.ctrl && !mods.logo
}

fn view(s: &State, cx: &mut Cx<State>) -> El {
    s.scale.set(cx.env().scale);
    cx.reduced_motion(s.preferences.reduced_motion);
    let dark = s.preferences.appearance.resolve(cx.system_dark());
    let theme = tok::theme(dark);
    let kit = Kit::neutral(dark);
    let window_size = cx
        .layout_size("app-root")
        .unwrap_or([WINDOW_SIZE.0, WINDOW_SIZE.1]);
    let zoom = s.preferences.interface_zoom;
    let content_size = [window_size[0] / zoom, window_size[1] / zoom];
    let chrome = cx.chrome_insets();
    let traffic_top = if chrome.leading > 0.0 { 28.0 } else { 0.0 };
    let insets = [chrome.leading / zoom, traffic_top / zoom];
    let mut content = el()
        .key("ui-content")
        .w(content_size[0])
        .h(content_size[1])
        .origin(0.0, 0.0)
        .scale(zoom)
        .child(canvas::viewport(s, &kit, dark, content_size, insets));
    if s.mode == Mode::Backend && !s.display_fixture {
        content = content.child(project::view(s, &kit));
    }
    if s.workbench_open && s.selected.is_some() {
        content = content.child(workbench::view(s, &kit, content_size[0], content_size[1]));
    }
    if s.settings_open {
        content = content.child(settings::view(s, &kit));
    }
    el().key("app-root")
        .w_full()
        .h_full()
        .clip_soft(0.0)
        .fill(theme.bg)
        .on_screen_layout(|s: &mut State, rect| s.observe_window_geometry(rect))
        .on_key_up(|s: &mut State, event, _| {
            if matches!(&event.key, Key::Text(value) if value == " ") {
                s.space_held = false;
                true
            } else {
                false
            }
        })
        .child(content)
}

fn fixture_scene(s: &mut State) {
    s.mode = Mode::Fixture;
    s.backend_enabled = false;
    s.display_fixture = false;
    s.project_path = None;
    s.selected_workflow = None;
    s.preferences.selected_project = None;
    s.preferences.sidebar_open = true;
    s.graph = fixture::small_graph();
    s.graph_state = LoadState::Idle;
}

fn backend_display_scene(s: &mut State) {
    s.backend_enabled = false;
    s.display_fixture = true;
    s.mode = Mode::Backend;
    s.project_path = None;
    s.preferences.selected_project = None;
    s.preferences.sidebar_open = true;
    s.graph = fixture::small_graph();
    s.graph_state = LoadState::Idle;
}

fn scene_graph_preview(s: &mut State) {
    fixture_scene(s);
    s.selected = Some(4);
    s.preview_node = Some(4);
    s.camera.set_pan([286.0, 44.0]);
}

fn scene_workbench(s: &mut State) {
    fixture_scene(s);
    s.selected = Some(4);
    s.workbench_open = true;
    s.camera.set_pan([10.0, 12.0]);
    s.camera.zoom.set(1.12);
    seed_output(s, 9);
}

fn scene_interface_zoom(s: &mut State) {
    scene_workbench(s);
    s.preferences.interface_zoom = 1.15;
}

fn scene_large_workbench(s: &mut State) {
    fixture_scene(s);
    s.graph = fixture::large_graph();
    s.selected = Some(10);
    s.workbench_open = true;
    s.camera.set_pan([8.0, 26.0]);
    s.camera.zoom.set(0.74);
    seed_output(s, 14);
    s.streaming = true;
    s.next_output_at = 0.0;
}

fn scene_zoomed(s: &mut State) {
    fixture_scene(s);
    s.selected = Some(6);
    s.camera.set_pan([-240.0, -36.0]);
    s.camera.zoom.set(1.65);
}

fn history_run(id: i64, state: &str, snapshot: serde_json::Value) -> waypoints_backend::RunStatus {
    serde_json::from_value(serde_json::json!({
        "run": {
            "id": id,
            "workflow_id": 1,
            "node_id": 4,
            "prompt": if id == 11 { "Review the recorded output" } else { "Earlier implementation run" },
            "directory": "/tmp/waypoints-rune-fixture/workspace",
            "change_id": "qytulmmuwtvswxvznlktmxtvpxktvrvq",
            "executable": "/tmp/waypoints-rune-fixture/fake-acp",
            "session_id": "fixture-session",
            "continued_from": if id == 11 { serde_json::Value::from(10) } else { serde_json::Value::Null },
            "state": {"status": state},
            "snapshot": snapshot
        }
    }))
    .expect("history run fixture")
}

fn scene_project_sidebar(s: &mut State) {
    s.backend_enabled = false;
    s.preferences.sidebar_open = true;
    s.mode = Mode::Backend;
    s.display_fixture = false;
    s.backend_state = LoadState::Ready;
    s.project_path = Some("/tmp/waypoints-rune-fixture/alpha".into());
    s.preferences.projects = vec![
        "/tmp/waypoints-rune-fixture/alpha".into(),
        "/tmp/waypoints-rune-fixture/beta".into(),
    ];
    s.preferences.selected_project = s.project_path.clone();
    s.workflows = vec![
        serde_json::from_value(serde_json::json!({
            "id": 1,
            "project_path": "/tmp/waypoints-rune-fixture/alpha",
            "name": "Canvas migration",
            "base": null,
            "archived": false
        }))
        .expect("workflow fixture"),
        serde_json::from_value(serde_json::json!({
            "id": 2,
            "project_path": "/tmp/waypoints-rune-fixture/alpha",
            "name": "Backend integration",
            "base": null,
            "archived": false
        }))
        .expect("workflow fixture"),
    ];
    s.workflows_state = LoadState::Ready;
    s.selected_workflow = Some(1);
    s.graph = fixture::small_graph();
    s.graph_state = LoadState::Ready;
    s.selected = Some(4);
    s.camera.set_pan([286.0, 44.0]);
}

fn scene_assignment(s: &mut State) {
    backend_display_scene(s);
    s.selected_workflow = Some(1);
    s.selected = Some(4);
    s.workbench_open = true;
    s.workbench_expanded = false;
    s.review.tab = WorkbenchTab::Assignment;
    s.camera.set_pan([286.0, 44.0]);
    let mut draft =
        NodeDraft::new("## Plan migration\n\nSplit the work into reversible milestones.".into());
    draft.edit.replace_text(
        "## Plan migration\n\nSplit the work into reversible milestones.\n\nThe saved assignment changed elsewhere; this draft is kept unsaved.",
    );
    draft.notice = Some("Saved assignment changed elsewhere; unsaved draft kept".into());
    s.drafts.insert((1, 4), draft);
}

fn scene_assignment_preview(s: &mut State) {
    scene_assignment(s);
    if let Some(draft) = s.drafts.get_mut(&(1, 4)) {
        draft.preview = true;
    }
}

fn scene_settings(s: &mut State) {
    fixture_scene(s);
    s.settings_open = true;
    s.preferences.appearance = crate::preferences::Appearance::System;
}

fn scene_history_runs(s: &mut State) {
    backend_display_scene(s);
    s.selected_workflow = Some(1);
    s.selected = Some(4);
    s.workbench_open = true;
    s.workbench_expanded = true;
    s.camera.set_pan([286.0, 44.0]);
    s.drafts.insert(
        (1, 4),
        NodeDraft::new("## Review workspace\n\nHistory fixture.".into()),
    );
    s.review.tab = WorkbenchTab::History;
    let saved = serde_json::json!({
        "status": "saved",
        "revision": {
            "change_id": "qytulmmuwtvswxvznlktmxtvpxktvrvq",
            "commit_id": "bc1ff243d16d5db056ae1cf33564e10bba6be91c",
            "description": "Recorded review fixture",
            "conflicted": false
        }
    });
    let not_recorded = serde_json::json!({"status": "not_recorded"});
    s.review.history.kind = crate::state::HistoryKind::Runs;
    s.review.history.runs = vec![
        history_run(11, "completed", saved),
        history_run(10, "cancelled", not_recorded),
    ];
    s.review.history.runs_state = LoadState::Ready;
    s.review.history.selected_run = Some(11);
    s.review.history.runs_cursor = Some("older-runs".into());
}

fn scene_history_native(s: &mut State) {
    backend_display_scene(s);
    s.selected_workflow = Some(1);
    s.selected = Some(4);
    s.workbench_open = true;
    s.workbench_expanded = true;
    s.camera.set_pan([286.0, 44.0]);
    s.drafts.insert(
        (1, 4),
        NodeDraft::new("## Review workspace\n\nHistory fixture.".into()),
    );
    s.review.tab = WorkbenchTab::History;
    s.review.history.kind = crate::state::HistoryKind::Revisions;
    s.review.history.revisions = vec![
        serde_json::from_value(serde_json::json!({
            "evolution": {
                "revision": {
                    "change_id": "qytulmmuwtvswxvznlktmxtvpxktvrvq",
                    "commit_id": "bc1ff243d16d5db056ae1cf33564e10bba6be91c",
                    "description": "Recorded review fixture",
                    "conflicted": false
                },
                "predecessor_commit_ids": ["1111111111111111111111111111111111111111"],
                "operation_id": "opaque-native-operation",
                "operation_description": "record run snapshot",
                "timestamp": "2026-11-07T00:00:00Z"
            },
            "is_current": true,
            "run_ids": [11]
        }))
        .expect("native revision fixture"),
        serde_json::from_value(serde_json::json!({
            "evolution": {
                "revision": {
                    "change_id": "qytulmmuwtvswxvznlktmxtvpxktvrvq",
                    "commit_id": "2222222222222222222222222222222222222222",
                    "description": "Earlier evolution",
                    "conflicted": false
                },
                "predecessor_commit_ids": [],
                "operation_id": "opaque-native-operation-2",
                "operation_description": "describe node workspace",
                "timestamp": "2026-11-06T23:30:00Z"
            },
            "is_current": false,
            "run_ids": []
        }))
        .expect("native revision fixture"),
    ];
    s.review.history.revisions_state = LoadState::Ready;
    s.review.history.selected_revision = Some("bc1ff243d16d5db056ae1cf33564e10bba6be91c".into());
    s.review.history.revisions_cursor = Some("older-revisions".into());
}

fn scene_diff_review(s: &mut State) {
    backend_display_scene(s);
    s.selected_workflow = Some(1);
    s.selected = Some(4);
    s.workbench_open = true;
    s.workbench_expanded = true;
    s.camera.set_pan([286.0, 44.0]);
    s.drafts.insert(
        (1, 4),
        NodeDraft::new(
            "## Review workspace\n\nCompare the recorded revision with the run snapshot.".into(),
        ),
    );
    s.review.tab = WorkbenchTab::Changes;
    s.review.state = LoadState::Ready;
    s.review.target = crate::backend::ReviewTarget::Current;
    s.review.review = Some(
        serde_json::from_value(serde_json::json!({
            "revision": {
                "change_id": "qytulmmuwtvswxvznlktmxtvpxktvrvq",
                "commit_id": "bc1ff243d16d5db056ae1cf33564e10bba6be91c",
                "description": "Recorded review fixture",
                "conflicted": false
            },
            "parent_commit_ids": ["0000000000000000000000000000000000000000"],
            "summary": "M src/main.rs\nM src/workbench.rs\n",
            "patch": ""
        }))
        .expect("diff review fixture"),
    );
    s.review.loaded_commit = Some("bc1ff243d16d5db056ae1cf33564e10bba6be91c".into());
    s.review.diff = rune_kit::diff::DiffState::new(vec![
        rune_kit::diff::FileDiff::from_texts(
            "crates/waypoints-rune/src/main.rs",
            "fn main() {\n    run();\n}\n",
            "fn main() {\n    run_canvas();\n}\n",
        ),
        rune_kit::diff::FileDiff::from_texts(
            "crates/waypoints-rune/src/workbench.rs",
            "fn view() {\n    assignment();\n}\n",
            "fn view() {\n    assignment();\n    diff_review();\n}\n",
        ),
    ]);
    s.review.selected_file = Some(0);
}

fn scene_structured_output(s: &mut State) {
    backend_display_scene(s);
    use waypoints_backend::activity::ActivityLog;

    s.selected_workflow = Some(1);
    s.selected = Some(4);
    s.workbench_open = true;
    s.workbench_expanded = true;
    s.review.tab = WorkbenchTab::Output;
    s.camera.set_pan([286.0, 44.0]);
    s.drafts.insert(
        (1, 4),
        NodeDraft::new(
            "## Review workspace\n\nCompare the recorded revision with the run snapshot.".into(),
        ),
    );
    s.runs.insert(
        4,
        serde_json::from_value(serde_json::json!({
            "run": {
                "id": 11,
                "workflow_id": 1,
                "node_id": 4,
                "prompt": "structured output fixture",
                "directory": "/tmp/waypoints-rune-fixture/workspace",
                "change_id": "fixture-change",
                "executable": "/tmp/waypoints-rune-fixture/fake-acp",
                "session_id": "fixture-session",
                "state": {"status": "running"},
                "snapshot": {"status": "not_recorded"}
            }
        }))
        .expect("structured output run fixture"),
    );
    let mut log = ActivityLog::default();
    let event = |sequence: u64, origin: &str, update: serde_json::Value| {
        serde_json::from_value(serde_json::json!({
            "sequence": sequence,
            "origin": origin,
            "update": update,
        }))
        .expect("structured output event fixture")
    };
    log.apply(
        vec![
            event(
                1,
                "replay",
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "Earlier replay: inspected the retained diff helpers."}
                }),
            ),
            event(
                2,
                "live",
                serde_json::json!({
                    "sessionUpdate": "agent_message_chunk",
                    "content": {"type": "text", "text": "## Review workspace\n\nCompare the recorded revision with the run snapshot.\n\n```rust\nfn target_label(target: &Target) -> String {\n    target.describe()\n}\n```\n"}
                }),
            ),
            event(
                3,
                "live",
                serde_json::json!({
                    "sessionUpdate": "agent_thought_chunk",
                    "content": {"type": "text", "text": "Keep the retained state; adjust presentation and freshness only."}
                }),
            ),
            event(
                4,
                "live",
                serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "read-diff",
                    "title": "Read diff helpers",
                    "kind": "read",
                    "status": "pending",
                    "locations": [{"path": "crates/waypoints-app/src/node_workspace.rs"}]
                }),
            ),
            event(
                5,
                "live",
                serde_json::json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "read-diff",
                    "status": "completed",
                    "content": [{"type": "content", "content": {"type": "text", "text": "DiffState retained"}}]
                }),
            ),
            event(
                6,
                "live",
                serde_json::json!({
                    "sessionUpdate": "tool_call",
                    "toolCallId": "plan",
                    "title": "Plan updated",
                    "kind": "other",
                    "status": "completed"
                }),
            ),
        ],
        false,
    );
    s.run_log = log;
    s.run_gap = true;
    s.run_output_at_bottom = true;
}

fn scene_structured_output_expanded(s: &mut State) {
    scene_structured_output(s);
    s.expanded_tools.insert(4);
}

fn save_preferences_on_close(s: &mut State, _: &mut Tx) {
    s.flush_preferences();
}

thread_local! {
    static MENU_HANDLE: std::cell::RefCell<Option<rune::AppHandle<State>>> =
        const { std::cell::RefCell::new(None) };
}

fn shortcut(value: &str) -> Shortcut {
    value.parse().expect("valid menu shortcut")
}

fn menu_open_settings(s: &mut State, _: &mut Tx) {
    s.settings_open = true;
}

fn menu_add_project(s: &mut State, tx: &mut Tx) {
    s.set_sidebar_open(true);
    project::open_add_dialog(tx);
}

fn menu_close_window(_: &mut State, tx: &mut Tx) {
    tx.close_window();
}

fn menu_toggle_sidebar(s: &mut State, _: &mut Tx) {
    if s.mode == Mode::Backend && !s.display_fixture {
        s.toggle_sidebar();
    }
}

fn menu_refresh_project(s: &mut State, _: &mut Tx) {
    if s.mode == Mode::Backend {
        s.load_workflows();
        s.load_graph();
    }
}

fn menu_items(
    sidebar_open: bool,
    sidebar_available: bool,
    refresh_available: bool,
) -> Vec<MenuItem<State>> {
    vec![
        MenuItem::new("Settings…", menu_open_settings)
            .id("settings")
            .key_equivalent(shortcut("cmd+comma"))
            .keys_first(),
        MenuItem::separator(),
        MenuItem::new("Add Project…", menu_add_project)
            .id("add-project")
            .menu("File")
            .key_equivalent(shortcut("cmd+o"))
            .keys_first(),
        MenuItem::new("Close Window", menu_close_window)
            .id("close-window")
            .menu("File"),
        MenuItem::new("Show Projects Sidebar", menu_toggle_sidebar)
            .id("toggle-sidebar")
            .menu("View")
            .key_equivalent(shortcut("cmd+b"))
            .checked(sidebar_open)
            .enabled(sidebar_available),
        MenuItem::new("Refresh Project", menu_refresh_project)
            .id("refresh-project")
            .menu("View")
            .key_equivalent(shortcut("cmd+r"))
            .keys_first()
            .enabled(refresh_available),
    ]
}

fn menu_flags(s: &State) -> (bool, bool, bool) {
    (
        s.preferences.sidebar_open,
        s.mode == Mode::Backend && !s.display_fixture,
        s.mode == Mode::Backend && s.project_path.is_some(),
    )
}

pub(crate) fn sync_native_menus(s: &State) {
    let errors = MENU_HANDLE.with(|handle| {
        handle
            .borrow()
            .as_ref()
            .map(rune::AppHandle::take_menu_errors)
            .unwrap_or_default()
    });
    for error in errors {
        eprintln!("rune: native menu: {error}");
    }
    let flags = menu_flags(s);
    if s.menu_flags.get() == Some(flags) {
        return;
    }
    MENU_HANDLE.with(|handle| {
        let handle = handle.borrow();
        let Some(handle) = handle.as_ref() else {
            return;
        };
        match handle.set_menu(menu_items(flags.0, flags.1, flags.2)) {
            Ok(()) => s.menu_flags.set(Some(flags)),
            Err(rune::MenuError::NotRunning) => {}
            Err(_) => s.menu_flags.set(Some(flags)),
        }
    });
}

fn display_point_bounds(
    display: &rune::Display,
    _primary: &rune::Display,
    _scale: f32,
) -> [f32; 4] {
    let [x, y, width, height] = display.native_bounds();
    #[cfg(target_os = "macos")]
    {
        [x as f32, y as f32, width as f32, height as f32]
    }
    #[cfg(not(target_os = "macos"))]
    {
        let [primary_x, primary_y, _, _] = _primary.native_bounds();
        [
            ((x - primary_x) / f64::from(_scale)) as f32,
            ((y - primary_y) / f64::from(_scale)) as f32,
            (width / f64::from(_scale)) as f32,
            (height / f64::from(_scale)) as f32,
        ]
    }
}

fn restored_geometry(
    geometry: preferences::WindowGeometry,
    displays: &[rune::Display],
) -> Option<preferences::WindowGeometry> {
    let primary = displays.iter().find(|display| display.primary)?;
    let bounds: Vec<_> = displays
        .iter()
        .map(|display| display_point_bounds(display, primary, geometry.scale))
        .collect();
    preferences::clamp_window_geometry(geometry, &bounds)
}

fn restored_position(
    geometry: preferences::WindowGeometry,
    displays: &[rune::Display],
) -> Option<(i32, i32)> {
    let primary = displays.iter().find(|display| display.primary)?;
    let [primary_x, primary_y, _, _] = primary.native_bounds();
    #[cfg(target_os = "macos")]
    let base = (
        primary_x as f32 * geometry.scale,
        primary_y as f32 * geometry.scale,
    );
    #[cfg(not(target_os = "macos"))]
    let base = (primary_x as f32, primary_y as f32);
    Some((
        (base.0 + geometry.x * geometry.scale).round() as i32,
        (base.1 + geometry.y * geometry.scale).round() as i32,
    ))
}

fn main() {
    let _ = startup();
    let loaded = preferences::load();
    rune::run_windows_with(rune::RunOptions::default(), move |displays| {
        let geometry = loaded
            .preferences
            .window_geometry
            .and_then(|geometry| restored_geometry(geometry, displays));
        let position = geometry.and_then(|geometry| restored_position(geometry, displays));
        let size = geometry.map_or(WINDOW_SIZE, |geometry| (geometry.width, geometry.height));
        let backend = loaded.preferences.selected_project.is_some() || startup().project.is_some();
        let mut app = rune::App::new(AppSpec {
            title: "Waypoints · Rune canvas",
            size,
            min_size: Some(WINDOW_MIN_SIZE),
            scenes: &[
                ("workbench", scene_workbench),
                ("graph-preview", scene_graph_preview),
                ("large-workbench", scene_large_workbench),
                ("zoomed", scene_zoomed),
                ("structured-output", scene_structured_output),
                (
                    "structured-output-expanded",
                    scene_structured_output_expanded,
                ),
                ("diff-review", scene_diff_review),
                ("assignment", scene_assignment),
                ("assignment-preview", scene_assignment_preview),
                ("settings", scene_settings),
                ("project-sidebar", scene_project_sidebar),
                ("interface-zoom", scene_interface_zoom),
                ("history-runs", scene_history_runs),
                ("history-native", scene_history_native),
            ],
            setup: Some(setup),
            poll: Some(poll),
            on_key: Some(on_key),
            ..AppSpec::new("Waypoints · Rune canvas", size, init_state, view)
        });
        for item in menu_items(loaded.preferences.sidebar_open, backend, backend) {
            app = app.menu_item(item);
        }
        let app = app
            .standard_menus()
            .window_options(rune::WindowOptions {
                position,
                ..rune::WindowOptions::default()
            })
            .on_close(save_preferences_on_close);
        let handle = app.handle();
        MENU_HANDLE.with(|cell| *cell.borrow_mut() = Some(handle));
        vec![rune::WindowSpec::from_app("main", app)]
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use rune::{Env, MagnifyEvent, Runtime, WheelEvent};
    use waypoints_backend::Config;
    use waypoints_backend::activity::{Activity, Stream};
    use waypoints_core::run::{RunState, SnapshotState};

    use crate::activity::MAX_OUTPUT_LINES;
    use crate::backend::{Backend, Command, Update};
    use crate::fixture::Graph;

    fn test_runtime() -> Runtime<State> {
        let mut runtime = Runtime::new(State::default(), view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        runtime
    }

    fn click(runtime: &mut Runtime<State>, x: f32, y: f32) {
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
    }

    #[test]
    fn camera_events_skip_the_view_and_keep_the_pointer_anchor() {
        let mut runtime = test_runtime();
        // Settle the first layout report before measuring camera-only commits.
        runtime.frame(0.0);
        let before_commits = rune::commit_generation();
        let anchor = [620.0, 330.0];
        let world_before = runtime.state.camera.screen_to_world(anchor);

        runtime.magnify_event(MagnifyEvent::new(0.25).at(anchor[0], anchor[1]));
        runtime.frame(0.0);

        let world_after = runtime.state.camera.screen_to_world(anchor);
        assert!(runtime.state.camera.zoom() > 1.0, "pinch zooms in");
        assert!(
            (world_after[0] - world_before[0]).abs() < 0.01
                && (world_after[1] - world_before[1]).abs() < 0.01,
            "the world point under the pointer stays fixed: {world_before:?} -> {world_after:?}"
        );
        assert_eq!(
            rune::commit_generation(),
            before_commits,
            "a camera-only pinch does not commit the view"
        );
        assert!(runtime.live_only_events() >= 1);

        let pan_before = runtime.state.camera.pan();
        runtime.wheel_event(WheelEvent::new(34.0, 18.0).at(anchor[0], anchor[1]));
        runtime.frame(0.0);
        let pan_after = runtime.state.camera.pan();
        assert_eq!(pan_after, [pan_before[0] + 34.0, pan_before[1] + 18.0]);
        assert_eq!(
            rune::commit_generation(),
            before_commits,
            "wheel panning does not commit the view"
        );
        assert!(runtime.live_only_events() >= 2);
        assert!(runtime.live_only_warnings().is_empty());
    }

    #[test]
    fn hit_testing_follows_a_moved_and_zoomed_camera() {
        let mut runtime = test_runtime();
        let (x, y) = runtime.find("node-4").expect("fixture card");
        assert!(runtime.element_at(x, y).unwrap().path().contains("node-4"));

        runtime.state.camera.set_pan([-120.0, -36.0]);
        runtime.state.camera.zoom.set(1.4);
        runtime.frame(0.0);

        let (x, y) = runtime
            .find("node-4")
            .expect("fixture card after camera move");
        let picked = runtime.element_at(x, y).expect("card is still hittable");
        assert!(
            picked.path().contains("node-4"),
            "hit testing follows the drawn card"
        );
    }

    #[test]
    fn dragging_a_card_changes_presentation_only_and_edges_follow() {
        let mut runtime = test_runtime();
        let node_before = runtime.state.graph.node(4).unwrap().clone();
        let edges_before = runtime.state.graph.edges.clone();
        let (x, y) = runtime.find("node-4").expect("fixture card");

        runtime.pointer_down(x, y);
        runtime.pointer_move(x + 42.0, y + 26.0);
        runtime.frame(1.0 / 60.0);
        runtime.pointer_up(x + 42.0, y + 26.0);
        runtime.frame(1.0 / 60.0);

        let node_after = runtime.state.graph.node(4).unwrap().clone();
        let zoom = runtime.state.camera.zoom();
        assert_eq!(node_after.id, node_before.id, "node identity is stable");
        assert!(
            (node_after.x - (node_before.x + 42.0 / zoom)).abs() < 0.5,
            "presentation x follows the drag in world points: {} -> {}",
            node_before.x,
            node_after.x
        );
        assert!(
            (node_after.y - (node_before.y + 26.0 / zoom)).abs() < 0.5,
            "presentation y follows the drag in world points: {} -> {}",
            node_before.y,
            node_after.y
        );
        assert_eq!(
            runtime.state.graph.edges, edges_before,
            "dependency edges are unchanged"
        );
        let edge = *edges_before.iter().find(|edge| edge.target == 4).unwrap();
        let (_, target) = runtime.state.graph.edge_endpoints(edge).unwrap();
        assert_eq!(
            target[0], node_after.x,
            "the edge endpoint follows the dragged card"
        );
        assert_eq!(
            target[1],
            node_after.y + fixture::CARD_HEIGHT * 0.5,
            "the edge endpoint tracks the card's new row"
        );
        assert_eq!(
            runtime.state.selected,
            Some(4),
            "pressing a card selects it"
        );
    }

    #[test]
    fn workbench_stays_screen_space_and_streams_synthetic_output() {
        let mut runtime = test_runtime();
        let (x, y) = runtime.find("node-3").expect("fixture card");

        click(&mut runtime, x, y);
        assert_eq!(runtime.state.selected, Some(3));
        assert!(!runtime.state.workbench_open, "a single press only selects");

        for _ in 0..2 {
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.16);
        }
        assert!(
            runtime.state.workbench_open,
            "double press opens the workbench"
        );
        let (before, _) = runtime
            .presented("workbench")
            .expect("workbench is mounted");

        runtime.state.camera.zoom.set(1.7);
        runtime.frame(0.0);
        let (after, _) = runtime
            .presented("workbench")
            .expect("workbench is still mounted");
        assert!(
            (before[2] - after[2]).abs() < 0.01 && (before[3] - after[3]).abs() < 0.01,
            "the workbench keeps its screen-space size under camera zoom: {before:?} -> {after:?}"
        );
        let (assignment, _) = runtime
            .presented("workbench-assignment")
            .expect("assignment text");
        let y = assignment[1] + 9.0;
        runtime.pointer_down(assignment[0] + 6.0, y);
        runtime.pointer_move(assignment[0] + 150.0, y);
        runtime.pointer_up(assignment[0] + 150.0, y);
        runtime.frame(0.0);
        assert!(
            runtime
                .selected_text()
                .is_some_and(|text| text.contains("Plan migration")),
            "assignment text stays selectable under camera zoom: {:?}",
            runtime.selected_text()
        );

        let (sx, sy) = runtime.find("workbench-stream").expect("stream button");
        click(&mut runtime, sx, sy);
        assert!(
            runtime.state.streaming,
            "stream button starts the synthetic feed"
        );
        for _ in 0..12 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(
            !runtime.state.output.is_empty(),
            "synthetic output arrives while the workbench is open"
        );
        assert!(runtime.find("workbench-output-count").is_some());

        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(0.0);
        assert!(!runtime.state.workbench_open, "Escape closes the workbench");
        assert_eq!(
            runtime.state.selected,
            Some(3),
            "closing keeps the graph selection"
        );
    }

    #[test]
    fn large_fixture_stream_is_bounded() {
        let mut runtime = test_runtime();
        runtime.update(|s, _| scene_large_workbench(s));
        runtime.frame(0.0);
        assert_eq!(runtime.state.graph.nodes.len(), 24);
        assert!(runtime.state.workbench_open);
        for _ in 0..(MAX_OUTPUT_LINES * 4) {
            runtime.frame(1.0 / 30.0);
        }
        assert_eq!(
            runtime.state.output.len(),
            MAX_OUTPUT_LINES,
            "synthetic output is bounded"
        );
        assert!(runtime.find("workbench").is_some());
    }

    #[test]
    fn workbench_fits_a_narrow_window() {
        let mut runtime =
            Runtime::new(State::default(), view, (720.0, 480.0), Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.update(|s, _| scene_workbench(s));
        runtime.frame(0.0);
        runtime.frame(0.0);
        let (rect, _) = runtime
            .presented("workbench")
            .expect("workbench is mounted");
        assert!(rect[2] <= 688.01, "workbench leaves a margin: {rect:?}");
        assert!(
            rect[0] >= 16.0,
            "workbench stays inside the narrow window: {rect:?}"
        );
        assert!(rect[1] >= 0.0 && rect[1] + rect[3] <= 480.0);
    }

    struct TestServer {
        _directory: tempfile::TempDir,
        config: Config,
        handle: std::thread::JoinHandle<()>,
    }

    fn spawn_test_server() -> TestServer {
        let directory = tempfile::Builder::new()
            .prefix("wp-rune-")
            .tempdir_in("/tmp")
            .expect("short temporary directory");
        let config = Config::new(
            &directory.path().join("workflow.db"),
            &directory.path().join("backend.sock"),
        )
        .expect("test backend config");
        let server_config = config.clone();
        let handle = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test server runtime");
            let _ = runtime.block_on(waypoints_backend::serve(&server_config));
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while !config.socket().exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "test backend socket never appeared"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        TestServer {
            _directory: directory,
            config,
            handle,
        }
    }

    async fn connect_test_client(config: &Config) -> waypoints_backend::Client {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            match waypoints_backend::Client::connect(config).await {
                Ok(client) => return client,
                Err(error) if tokio::time::Instant::now() < deadline => {
                    let _ = error;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("test backend did not accept clients: {error}"),
            }
        }
    }

    fn wait_for_state(state: &mut State, timeout: Duration, condition: impl Fn(&State) -> bool) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let updates = state
                .backend
                .as_ref()
                .map(Backend::drain)
                .unwrap_or_default();
            for update in updates {
                state.apply_backend_update(update);
            }
            if !state.run_events_pending && state.selected_run().is_some_and(run_is_active) {
                state.request_run_events();
            }
            if condition(state) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "backend state did not reach the expected condition: backend={:?} workflows={:?} ({}) graph={:?} runs={:?} run={:?} error={:?}",
                state.backend_state,
                state.workflows_state,
                state.workflows.len(),
                state.graph_state,
                state.runs_state,
                state.selected_run().map(|run| run.run.state.clone()),
                state.run_error
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_for_runtime(
        runtime: &mut Runtime<State>,
        timeout: Duration,
        condition: impl Fn(&State) -> bool,
    ) {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            runtime.frame(1.0 / 60.0);
            if condition(&runtime.state) {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "runtime did not reach the expected backend state"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn backend_navigation_assignment_and_stale_replies_use_the_client() {
        let server = spawn_test_server();
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test client runtime");
        let (client, project, workflow, second_workflow, first_node, second_node) = tokio_rt
            .block_on(async {
                let client = connect_test_client(&server.config).await;
                let project = server._directory.path().join("project");
                std::fs::create_dir(&project).expect("project directory");
                let workflow = client
                    .create_workflow(&project, "Canvas workflow")
                    .await
                    .expect("create workflow");
                let second_workflow = client
                    .create_workflow(&project, "Second workflow")
                    .await
                    .expect("create second workflow");
                let first_node = client
                    .create_node(workflow.id, "# Plan migration\n\nFirst assignment")
                    .await
                    .expect("create first node");
                let second_node = client
                    .create_node(second_workflow.id, "# Build canvas\n\nSecond assignment")
                    .await
                    .expect("create second node");
                (
                    client,
                    project,
                    workflow,
                    second_workflow,
                    first_node,
                    second_node,
                )
            });

        let mut state = State {
            mode: Mode::Backend,
            project_path: Some(project.clone()),
            selected_workflow: Some(workflow.id),
            graph: Graph::default(),
            backend: Some(Backend::spawn(
                server.config.clone(),
                std::sync::Arc::new(|| {}),
            )),
            backend_state: LoadState::Loading,
            preferences_path: Some(server._directory.path().join("prefs.json")),
            preferences_writable: true,
            ..State::default()
        };
        state.camera.set_pan([286.0, 44.0]);
        state
            .backend
            .as_ref()
            .expect("backend")
            .send(Command::Connect);
        wait_for_state(&mut state, Duration::from_secs(10), |state| {
            state.workflows.len() == 2
                && state.graph_state == LoadState::Ready
                && state.selected_workflow == Some(workflow.id)
        });
        assert_eq!(state.graph.nodes.len(), 1);
        assert_eq!(state.graph.nodes[0].title, "# Plan migration");

        let old_token = state.graph_token;
        let old_graph = state.source_graph.clone().expect("first graph");
        state.select_workflow(second_workflow.id);
        assert_eq!(state.selected_workflow, Some(second_workflow.id));
        let saved = std::fs::read_to_string(server._directory.path().join("prefs.json"))
            .expect("selected workflow persisted");
        assert!(saved.contains(&format!("\"selected_workflow\": {}", second_workflow.id)));
        assert!(saved.contains(&project.display().to_string()));
        state.apply_backend_update(Update::Graph {
            token: old_token,
            workflow_id: workflow.id,
            result: Box::new(Ok(old_graph)),
        });
        assert_eq!(
            state.source_graph.as_ref().map(|graph| graph.workflow.id),
            Some(workflow.id),
            "a stale graph reply does not replace the newly selected workflow"
        );
        wait_for_state(&mut state, Duration::from_secs(10), |state| {
            state.graph_state == LoadState::Ready
                && state.selected_workflow == Some(second_workflow.id)
        });
        assert_eq!(
            state.source_graph.as_ref().map(|graph| graph.workflow.id),
            Some(second_workflow.id)
        );

        state.select_remote_node(second_node.id);
        state
            .drafts
            .get_mut(&(second_workflow.id, second_node.id))
            .expect("draft for the second node")
            .edit
            .replace_text("unsaved draft survives navigation");
        state.select_workflow(workflow.id);
        wait_for_state(&mut state, Duration::from_secs(10), |state| {
            state.graph_state == LoadState::Ready && state.selected_workflow == Some(workflow.id)
        });
        state.select_workflow(second_workflow.id);
        wait_for_state(&mut state, Duration::from_secs(10), |state| {
            state.graph_state == LoadState::Ready
                && state.selected_workflow == Some(second_workflow.id)
        });
        state.select_remote_node(second_node.id);
        assert_eq!(
            state
                .draft_for(second_node.id)
                .expect("preserved draft")
                .edit
                .text,
            "unsaved draft survives navigation"
        );

        let mut ui = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        ui.poll = Some(poll);
        ui.on_key = Some(on_key);
        ui.check_live_only(false);
        ui.frame(0.0);
        assert!(ui.find("project-panel").is_some());
        assert!(
            ui.find(&format!("workflow-{}", second_workflow.id))
                .is_some()
        );
        let (x, y) = ui
            .find(&format!("node-{}", second_node.id))
            .expect("remote graph card");
        click(&mut ui, x, y);
        for _ in 0..2 {
            ui.pointer_down(x, y);
            ui.pointer_up(x, y);
            ui.frame(0.16);
        }
        assert!(ui.state.workbench_open);
        assert!(ui.find("workbench-assignment-editor").is_some());
        let key = (second_workflow.id, second_node.id);
        ui.update(|state, _| {
            state
                .drafts
                .get_mut(&key)
                .expect("assignment draft")
                .preview = true;
        });
        ui.frame(0.0);
        assert!(
            ui.find("workbench-assignment-preview").is_some(),
            "assignment preview renders Markdown from the draft"
        );
        ui.update(|state, _| {
            state
                .drafts
                .get_mut(&key)
                .expect("assignment draft")
                .preview = false;
        });
        ui.frame(0.0);

        ui.update(|state, _| {
            state
                .drafts
                .get_mut(&key)
                .expect("assignment draft")
                .edit
                .replace_text("# Updated node\n\nChanged from Rune");
        });
        ui.frame(0.0);
        let (save_x, save_y) = ui.find("workbench-save").expect("save button");
        click(&mut ui, save_x, save_y);
        wait_for_runtime(&mut ui, Duration::from_secs(10), |state| {
            state.draft_for(second_node.id).is_some_and(|draft| {
                draft.pending.is_none()
                    && draft
                        .notice
                        .as_deref()
                        .is_some_and(|notice| notice.starts_with("Assignment saved"))
            })
        });
        assert!(
            ui.state
                .draft_for(second_node.id)
                .expect("draft")
                .baseline
                .starts_with("# Updated node")
        );
        let saved = tokio_rt
            .block_on(client.node(second_workflow.id, second_node.id))
            .expect("read saved node");
        assert_eq!(saved.prompt, "# Updated node\n\nChanged from Rune");
        assert_eq!(first_node.id, 1, "fixture node identity stays stable");

        drop(ui);
        let _ = tokio_rt.block_on(client.stop());
        let _ = server.handle.join();
    }

    fn git_jj(project: &std::path::Path, args: &[&str]) {
        let output = std::process::Command::new("jj")
            .current_dir(project)
            .env("JJ_CONFIG", "")
            .args([
                "--config",
                "user.name='Waypoints test'",
                "--config",
                "user.email='test@example.invalid'",
                "--no-pager",
                "--color=never",
            ])
            .args(args)
            .output()
            .expect("running jj");
        assert!(
            output.status.success(),
            "jj {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn backend_run_permission_cancel_and_snapshot_use_the_client() {
        use std::os::unix::fs::PermissionsExt;

        let server = spawn_test_server();
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test client runtime");
        let project = server._directory.path().join("run-project");
        std::fs::create_dir(&project).expect("project directory");
        git_jj(server._directory.path(), &["git", "init", "run-project"]);
        std::fs::write(project.join("baseline.txt"), "original\n").expect("baseline");
        git_jj(&project, &["describe", "-m", "Run baseline"]);
        let workspace = server._directory.path().join("agent-workspace");
        let agent = server._directory.path().join("fake-acp");
        std::fs::write(
            &agent,
            include_str!("../../waypoints-cli/tests/fixtures/acp_agent.py"),
        )
        .expect("fake ACP agent");
        std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o700))
            .expect("agent permissions");

        let (client, workflow, node, other_node) = tokio_rt.block_on(async {
            let client = connect_test_client(&server.config).await;
            let workflow = client
                .create_workflow(&project, "Run workflow")
                .await
                .expect("create workflow");
            let node = client
                .create_node(workflow.id, "permission")
                .await
                .expect("create node");
            let other_node = client
                .create_node(workflow.id, "other node")
                .await
                .expect("create other node");
            client
                .create_node_change(workflow.id, node.id, "root()")
                .await
                .expect("create node change");
            client
                .create_node_workspace(workflow.id, node.id, &workspace)
                .await
                .expect("create node workspace");
            (client, workflow, node, other_node)
        });

        let mut state = State {
            mode: Mode::Backend,
            project_path: Some(project.clone()),
            selected_workflow: Some(workflow.id),
            graph: Graph::default(),
            backend: Some(Backend::spawn(
                server.config.clone(),
                std::sync::Arc::new(|| {}),
            )),
            backend_state: LoadState::Loading,
            agent: Some(agent),
            ..State::default()
        };
        state.camera.set_pan([286.0, 44.0]);
        state
            .backend
            .as_ref()
            .expect("backend")
            .send(Command::Connect);
        wait_for_state(&mut state, Duration::from_secs(10), |state| {
            state.graph_state == LoadState::Ready
                && state.runs_state == LoadState::Ready
                && state.selected_workflow == Some(workflow.id)
        });
        state.select_remote_node(node.id);
        state.start_run();
        wait_for_state(&mut state, Duration::from_secs(15), |state| {
            state
                .selected_run()
                .is_some_and(|run| run.permission.is_some())
        });
        assert!(
            state.run_log.blocks().iter().any(|block| {
                matches!(
                    &block.activity,
                    Activity::Text(Stream::Assistant, text) if text == "event-0"
                )
            }),
            "structured output arrived: {:?}",
            state.run_log.blocks()
        );
        state.select_remote_node(other_node.id);
        assert!(state.selected_run().is_none());
        assert!(
            state.run_log.blocks().is_empty(),
            "live output belongs to the selected node"
        );
        state.select_remote_node(node.id);
        assert!(
            state
                .selected_run()
                .is_some_and(|run| run.permission.is_some()),
            "the pending permission survives node switching"
        );

        let (permission_id, option_id) = {
            let permission = state
                .selected_run()
                .and_then(|run| run.permission.as_ref())
                .expect("pending permission");
            let option = permission
                .options
                .iter()
                .find(|option| option.option_id == "allow")
                .expect("allow once option");
            (permission.id, option.option_id.clone())
        };
        state.reply_permission(permission_id, option_id);
        wait_for_state(&mut state, Duration::from_secs(20), |state| {
            state.selected_run().is_some_and(|run| {
                matches!(run.run.state, RunState::Completed)
                    && matches!(run.run.snapshot, SnapshotState::Saved { .. })
            })
        });
        assert_eq!(
            state.selected_run().map(|run| run.run.state.clone()),
            Some(RunState::Completed),
            "run outcome and saved snapshot are both present"
        );
        let completed_run_id = state.selected_run().expect("completed run").run.id;
        state.review_latest_run();
        wait_for_state(&mut state, Duration::from_secs(15), |state| {
            state.review.state == LoadState::Ready
        });
        let review = state.review.review.as_ref().expect("run snapshot review");
        assert!(
            review.patch.contains("approved.txt"),
            "review uses the run's exact saved snapshot: {}",
            review.patch
        );
        assert!(
            !state.review.diff.files().is_empty(),
            "review patch is parsed into diff files"
        );
        assert_eq!(
            state.review.target,
            crate::backend::ReviewTarget::Run(completed_run_id),
            "review target is pinned to the completed run"
        );

        state.start_run();
        wait_for_state(&mut state, Duration::from_secs(15), |state| {
            state
                .selected_run()
                .is_some_and(|run| run.permission.is_some())
        });
        state.cancel_run();
        wait_for_state(&mut state, Duration::from_secs(20), |state| {
            state
                .selected_run()
                .is_some_and(|run| matches!(run.run.state, RunState::Cancelled))
        });

        state.load_history_runs(true);
        wait_for_state(&mut state, Duration::from_secs(15), |state| {
            state.review.history.runs_state == LoadState::Ready
        });
        assert!(
            state.review.history.runs.len() >= 2,
            "run history retains both the completed and cancelled runs: {:?}",
            state
                .review
                .history
                .runs
                .iter()
                .map(|run| run.run.id)
                .collect::<Vec<_>>()
        );
        state.review.history.kind = crate::state::HistoryKind::Runs;
        state.review.history.selected_run = Some(completed_run_id);
        state.review_history_run();
        wait_for_state(&mut state, Duration::from_secs(15), |state| {
            state.review.state == LoadState::Ready
                && state.review.target == crate::backend::ReviewTarget::Run(completed_run_id)
        });
        assert!(
            state
                .review
                .review
                .as_ref()
                .is_some_and(|review| review.patch.contains("approved.txt"))
        );

        state.set_history_kind(crate::state::HistoryKind::Revisions);
        wait_for_state(&mut state, Duration::from_secs(20), |state| {
            state.review.history.revisions_state == LoadState::Ready
        });
        assert!(
            !state.review.history.revisions.is_empty(),
            "native revision history loads"
        );
        let revision = state.review.history.revisions[0]
            .evolution
            .revision
            .commit_id
            .clone();
        state.review.history.selected_revision = Some(revision.clone());
        state.review_history_revision();
        wait_for_state(&mut state, Duration::from_secs(20), |state| {
            state.review.state == LoadState::Ready
                && state.review.target == crate::backend::ReviewTarget::Revision(revision.clone())
        });

        drop(state);
        let _ = tokio_rt.block_on(client.stop());
        let _ = server.handle.join();
    }

    #[test]
    fn backend_failure_empty_and_retry_states_render() {
        let mut state = State {
            mode: Mode::Backend,
            project_path: Some("/tmp/waypoints-rune-unavailable".into()),
            backend_state: LoadState::Failed("backend unavailable".into()),
            workflows_state: LoadState::Ready,
            graph_state: LoadState::Idle,
            ..State::default()
        };
        state.graph = Graph::default();
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("backend-error").is_some());
        assert!(runtime.find("backend-retry").is_some());
        assert!(runtime.find("workflows-empty").is_some());

        runtime.update(|state, _| {
            state.backend_state = LoadState::Ready;
            state.workflows_state = LoadState::Failed("workflow list failed".into());
        });
        runtime.frame(0.0);
        assert!(runtime.find("workflows-error").is_some());
        assert!(runtime.find("workflows-retry").is_some());

        runtime.update(|state, _| {
            state.workflows_state = LoadState::Ready;
            state.graph_state = LoadState::Failed("graph load failed".into());
        });
        runtime.frame(0.0);
        assert!(runtime.find("graph-error").is_some());
        assert!(runtime.find("graph-retry").is_some());
    }

    fn structured_output_view(state: &State, _: &mut Cx<State>) -> El {
        rune::col()
            .key("structured-output-test")
            .w_full()
            .h(600.0)
            .child(crate::run_output::view(state, &Kit::neutral(false)))
    }

    #[test]
    fn structured_output_renders_markdown_tools_replay_and_notices() {
        use rune::Env;
        use waypoints_backend::activity::ActivityLog;

        let mut state = State {
            mode: Mode::Backend,
            selected_workflow: Some(1),
            selected: Some(1),
            run_gap: true,
            ..State::default()
        };
        state.run_log = ActivityLog::default();
        let event = |sequence: u64, origin: &str, update: serde_json::Value| {
            serde_json::from_value(serde_json::json!({
                "sequence": sequence,
                "origin": origin,
                "update": update,
            }))
            .expect("run event fixture")
        };
        state.run_log.apply(
            vec![
                event(
                    1,
                    "live",
                    serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "## Review workspace\n\nMarkdown body."}
                    }),
                ),
                event(
                    2,
                    "live",
                    serde_json::json!({
                        "sessionUpdate": "tool_call",
                        "toolCallId": "read",
                        "title": "Read source",
                        "status": "pending"
                    }),
                ),
                event(
                    3,
                    "live",
                    serde_json::json!({
                        "sessionUpdate": "tool_call_update",
                        "toolCallId": "read",
                        "status": "completed"
                    }),
                ),
                event(
                    4,
                    "replay",
                    serde_json::json!({
                        "sessionUpdate": "agent_message_chunk",
                        "content": {"type": "text", "text": "Earlier output."}
                    }),
                ),
            ],
            false,
        );
        let mut runtime = Runtime::new(
            state,
            structured_output_view,
            (760.0, 600.0),
            Env::new(false, 1.0),
        );
        runtime.check_live_only(false);
        runtime.frame(0.0);

        assert!(runtime.find("run-tool-2").is_some());
        assert!(runtime.find("run-replay-notice-4").is_some());
        assert!(runtime.find("run-gap-notice").is_some());
        assert!(
            runtime.elements().iter().any(|element| {
                element
                    .text
                    .as_deref()
                    .is_some_and(|text| text.contains("Review workspace"))
            }),
            "Markdown heading is rendered"
        );
        assert!(runtime.find("run-tool-details-2").is_none());
        let (x, y) = runtime.find("run-tool-2").expect("tool toggle");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(
            runtime.find("run-tool-details-2").is_some(),
            "tool details expand in place"
        );
        runtime.update(|state, _| state.run_output_at_bottom = false);
        runtime.frame(0.0);
        assert!(
            runtime.find("run-jump-latest").is_some(),
            "scrolling away from the tail exposes Jump to latest"
        );
    }

    #[test]
    fn diff_review_renders_targets_files_modes_and_staleness() {
        use rune::Env;
        use rune_kit::diff::DiffMode;

        let mut state = State::default();
        scene_diff_review(&mut state);
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("review-target-current").is_some());
        assert!(runtime.find("review-file-0").is_some());
        assert!(runtime.find("review-file-1").is_some());
        assert!(runtime.find("review-commit").is_some());
        assert!(runtime.find("review-diff").is_some());
        let (files, _) = runtime.presented("review-files").expect("file list");
        assert!(
            (files[2] - 190.0).abs() < 1.0,
            "the changed-files pane starts fixed: {files:?}"
        );
        let (grip_x, grip_y) = runtime
            .find("review-file-resize")
            .expect("file resize grip");
        runtime.pointer_move(grip_x, grip_y);
        runtime.frame(0.0);
        runtime.pointer_down(grip_x, grip_y);
        runtime.pointer_move(grip_x + 60.0, grip_y);
        runtime.pointer_up(grip_x + 60.0, grip_y);
        runtime.frame(0.0);
        assert!(
            (runtime.state.review.file_list_width - 250.0).abs() < 5.0,
            "dragging the grip resizes the file list: {}",
            runtime.state.review.file_list_width
        );

        let (x, y) = runtime.find("review-toggle-mode").expect("mode toggle");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert_eq!(runtime.state.review.diff.mode(), DiffMode::Split);

        let (x, y) = runtime.find("review-toggle-wrap").expect("wrap toggle");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.review.diff.wrap());

        let (x, y) = runtime.find("review-file-1").expect("second file");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert_eq!(runtime.state.review.selected_file, Some(1));

        runtime.update(|state, _| state.review.stale = true);
        runtime.frame(0.0);
        assert!(runtime.find("review-stale").is_some());
        assert!(runtime.find("review-reload-current").is_some());

        runtime.update(|state, _| {
            if let Some(review) = &mut state.review.review {
                review.revision.conflicted = true;
            }
        });
        runtime.frame(0.0);
        assert!(runtime.find("review-conflicted").is_some());
    }

    #[test]
    fn history_views_render_runs_native_and_keep_rows_on_older_page_failure() {
        use rune::Env;

        let mut state = State::default();
        scene_history_runs(&mut state);
        let newer = history_run(
            12,
            "completed",
            serde_json::json!({"status": "not_recorded"}),
        );
        state.runs.insert(4, newer.clone());
        state.apply_backend_update(Update::RunStatus {
            token: state.run_status_token,
            result: Box::new(Ok(newer)),
        });
        assert!(
            state.review.history.fresh,
            "a newly recorded run marks loaded history as fresh"
        );
        state.review.history.fresh = false;
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("history-run-11").is_some());
        assert!(runtime.find("history-run-10").is_some());
        assert!(runtime.find("history-review-run").is_some());
        assert!(runtime.find("history-load-older").is_some());

        let token = runtime.state.review.history.token;
        runtime.update(|state, _| {
            state.apply_backend_update(Update::RunsPage {
                token,
                workflow_id: 1,
                node_id: 4,
                append: true,
                result: Box::new(Err("older run page failed".into())),
            });
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.review.history.runs.len(), 2);
        assert_eq!(
            runtime.state.review.history.notice.as_deref(),
            Some("older run page failed")
        );
        assert!(runtime.find("history-notice").is_some());

        let mut state = State::default();
        scene_history_native(&mut state);
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("history-revision-bc1ff243d16d").is_some());
        assert!(runtime.find("history-review-revision").is_some());
        runtime.update(|state, _| state.review.history.fresh = true);
        runtime.frame(0.0);
        assert!(runtime.find("history-freshness").is_some());
        assert!(runtime.find("history-refresh-new").is_some());

        let token = runtime.state.review.history.token;
        runtime.update(|state, _| {
            state.apply_backend_update(Update::RevisionsPage {
                token,
                workflow_id: 1,
                node_id: 4,
                append: true,
                result: Box::new(Err("older revision page failed".into())),
            });
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.review.history.revisions.len(), 2);
        assert_eq!(
            runtime.state.review.history.notice.as_deref(),
            Some("older revision page failed")
        );
    }

    #[test]
    fn settings_overlay_applies_appearance_and_motion_preferences() {
        use rune::Env;

        let mut runtime = Runtime::new(State::default(), view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        let (x, y) = runtime.find("settings-open").expect("settings button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.find("settings-panel").is_some());
        assert!(runtime.find("settings-appearance").is_some());
        assert!(runtime.find("settings-reduced-motion").is_some());
        assert!(runtime.find("settings-shortcuts").is_some());
        let elements = runtime.elements();
        assert!(
            elements
                .iter()
                .any(|element| element.text.as_deref().is_some_and(
                    |text| text.contains("Workbench: Assignment, Output, Changes, History")
                )),
            "the settings sheet documents the workbench tab shortcuts"
        );
        assert!(
            elements
                .iter()
                .any(|element| element.text.as_deref().is_some_and(|text| text == "⌘B")),
            "the settings sheet documents the project shortcut"
        );

        assert!(runtime.find("segment0").is_some());
        assert!(runtime.find("segment1").is_some());
        assert!(runtime.find("segment2").is_some());
        runtime.update(|state, _| {
            state.set_appearance(crate::preferences::Appearance::Dark);
            state.set_reduced_motion(true);
        });
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.preferences.appearance,
            crate::preferences::Appearance::Dark
        );
        assert!(runtime.state.preferences.reduced_motion);

        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        for _ in 0..10 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(!runtime.state.settings_open);
        let focused = runtime
            .elements()
            .into_iter()
            .find(|element| element.focused)
            .and_then(|element| element.key);
        assert_eq!(
            focused.as_deref(),
            Some("settings-open"),
            "closing Settings returns focus to its opener"
        );
    }

    #[test]
    fn interface_zoom_scales_the_whole_ui_from_a_larger_layout() {
        use rune::Env;

        let mut state = State::default();
        state.preferences.interface_zoom = 1.15;
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        let content = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("ui-content"))
            .expect("scaled UI content");
        assert!(
            content.target[2] < WINDOW_SIZE.0,
            "content lays out in a larger logical window: {:?}",
            content.target
        );
        assert!(
            (content.rect[2] - WINDOW_SIZE.0).abs() < 1.0,
            "content draws across the window: {:?}",
            content.rect
        );
        assert!(runtime.find("canvas-hud").is_some());
        assert!(runtime.find("settings-open").is_some());
    }

    #[test]
    fn project_add_and_remove_use_the_folder_dialog_and_persist() {
        use rune::{Env, FileDialogKind};

        let directory = tempfile::tempdir().expect("preferences dir");
        let project = directory.path().join("project");
        std::fs::create_dir(&project).expect("project dir");
        let preferences_path = directory.path().join("prefs.json");
        let mut state = State {
            mode: Mode::Backend,
            preferences_path: Some(preferences_path.clone()),
            preferences_writable: true,
            ..State::default()
        };
        state.display_fixture = false;
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("projects-empty").is_some());
        let (x, y) = runtime.find("project-add").expect("add project");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        let (kind, _) = runtime.file_dialog().expect("folder dialog");
        assert_eq!(kind, FileDialogKind::Folder);
        runtime
            .resolve_file_dialog(Some(project.clone()))
            .expect("answer folder dialog");
        for _ in 0..5 {
            runtime.frame(1.0 / 60.0);
        }
        let canonical = std::fs::canonicalize(&project).expect("canonical project");
        assert_eq!(runtime.state.preferences.projects, vec![canonical.clone()]);
        assert_eq!(runtime.state.project_path.as_ref(), Some(&canonical));
        assert!(runtime.find("project-0").is_some());
        let saved = std::fs::read_to_string(&preferences_path).expect("saved preferences");
        assert!(saved.contains(&canonical.display().to_string()));

        let (x, y) = runtime.find("project-remove-0").expect("remove project");
        runtime.pointer_move(x, y);
        runtime.frame(0.0);
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.preferences.projects.is_empty());
        assert!(runtime.state.project_path.is_none());
        assert!(runtime.find("projects-empty").is_some());
        let saved = std::fs::read_to_string(&preferences_path).expect("saved preferences");
        assert!(!saved.contains(&canonical.display().to_string()));
    }

    #[test]
    fn assignment_edit_and_preview_render_draft_and_notice() {
        use rune::Env;

        let mut state = State::default();
        scene_assignment(&mut state);
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("workbench-assignment-editor").is_some());
        assert!(runtime.find("assignment-mode").is_some());
        let notice = runtime
            .elements()
            .into_iter()
            .find(|element| element.key.as_deref() == Some("assignment-notice"))
            .and_then(|element| element.text);
        assert_eq!(
            notice.as_deref(),
            Some("Saved assignment changed elsewhere; unsaved draft kept")
        );
        assert!(runtime.find("workbench-save").is_some());
        assert!(runtime.find("workbench-discard").is_some());
        let (save, _) = runtime.presented("workbench-save").expect("save button");
        let (close, _) = runtime
            .presented("workbench-close-button")
            .expect("footer close");
        assert!(
            save[1] + save[3] <= close[1],
            "compact assignment controls stay above the footer: {save:?} vs {close:?}"
        );

        let mut state = State::default();
        scene_assignment_preview(&mut state);
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("workbench-assignment-preview").is_some());
        let (rect, _) = runtime
            .presented("workbench-assignment-preview-scroll")
            .expect("preview viewport");
        assert!(
            rect[3] > 40.0 && rect[3] < 100.0,
            "the compact preview is clipped to its viewport: {rect:?}"
        );
        runtime.update(|state, _| state.workbench_expanded = true);
        for _ in 0..24 {
            runtime.frame(1.0 / 60.0);
        }
        let (rect, _) = runtime
            .presented("workbench-assignment-preview-scroll")
            .expect("expanded preview viewport");
        assert!(
            rect[3] > 200.0,
            "the expanded preview fills the workbench: {rect:?}"
        );
    }

    #[test]
    fn hud_icons_keep_accessible_labels() {
        use rune::Env;

        let state = State {
            mode: Mode::Backend,
            backend_enabled: false,
            ..State::default()
        };
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        for (key, label) in [
            ("zoom-in", "Zoom in"),
            ("zoom-out", "Zoom out"),
            ("zoom-reset", "Reset"),
            ("project-toggle", "Hide projects"),
            ("settings-open", "Settings"),
        ] {
            let element = runtime
                .elements()
                .into_iter()
                .find(|element| element.key.as_deref() == Some(key))
                .expect("icon button");
            assert_eq!(element.label.as_deref(), Some(label), "{key}");
            assert_eq!(element.role, "button", "{key}");
        }
    }

    #[test]
    fn pointer_hover_previews_a_card_and_clears_on_leave() {
        let mut runtime = test_runtime();
        let (x, y) = runtime.find("node-4").expect("fixture card");
        runtime.pointer_move(x, y);
        for _ in 0..20 {
            runtime.frame(1.0 / 60.0);
        }
        assert_eq!(runtime.state.preview_node, Some(4));
        assert!(runtime.find("graph-preview").is_some());

        runtime.pointer_move(5.0, 400.0);
        for _ in 0..20 {
            runtime.frame(1.0 / 60.0);
        }
        assert_eq!(runtime.state.preview_node, None);
        assert!(runtime.find("graph-preview").is_none());
    }

    #[test]
    fn window_geometry_is_throttled_persisted_and_flushed() {
        assert!(
            !rune::is_headless(),
            "cargo test is not a headless render process"
        );
        let directory = tempfile::tempdir().expect("preferences dir");
        let preferences_path = directory.path().join("prefs.json");
        let mut state = State {
            preferences_path: Some(preferences_path.clone()),
            preferences_writable: true,
            ..State::default()
        };
        state.scale.set(2.0);
        state.observe_window_geometry([120.0, 80.0, 1100.0, 760.0]);
        assert!(state.geometry_dirty);
        state.tick_preferences(0.2);
        assert!(!preferences_path.exists(), "saves are throttled");

        state.tick_preferences(0.6);
        assert!(!state.geometry_dirty);
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&preferences_path).expect("saved preferences"),
        )
        .expect("valid preferences JSON");
        assert_eq!(saved["window_geometry"]["x"], 120.0);
        assert_eq!(saved["window_geometry"]["width"], 1100.0);
        assert_eq!(saved["window_geometry"]["scale"], 2.0);

        state.observe_window_geometry([130.0, 90.0, 1100.0, 760.0]);
        assert!(state.geometry_dirty);
        state.flush_preferences();
        assert!(!state.geometry_dirty);
        let saved: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(&preferences_path).expect("flushed preferences"),
        )
        .expect("valid preferences JSON");
        assert_eq!(saved["window_geometry"]["x"], 130.0);

        state.observe_window_geometry([f32::NAN, 0.0, 100.0, 100.0]);
        assert_eq!(
            state.preferences.window_geometry.map(|geometry| geometry.x),
            Some(130.0),
            "invalid geometry is ignored"
        );
    }

    #[test]
    fn keyboard_toggles_sidebar_and_opens_add_project_dialog() {
        use rune::{Env, FileDialogKind};

        let directory = tempfile::tempdir().expect("preferences dir");
        let preferences_path = directory.path().join("prefs.json");
        let mut state = State {
            mode: Mode::Backend,
            preferences_path: Some(preferences_path),
            preferences_writable: true,
            ..State::default()
        };
        state.display_fixture = false;
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert!(runtime.find("project-panel").is_some());
        assert!(runtime.state.preferences.sidebar_open);

        runtime.key(KeyEvent {
            key: Key::Command('b'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert!(!runtime.state.preferences.sidebar_open);
        assert!(runtime.find("project-panel").is_none());
        assert!(runtime.find("project-toggle").is_some());

        runtime.key(KeyEvent {
            key: Key::Command('b'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert!(runtime.state.preferences.sidebar_open);
        assert!(runtime.find("project-panel").is_some());

        runtime.key(KeyEvent {
            key: Key::Command('o'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        let (kind, options) = runtime.file_dialog().expect("add project dialog");
        assert_eq!(kind, FileDialogKind::Folder);
        assert_eq!(options.title, "Add Waypoints project");
        runtime
            .resolve_file_dialog(None)
            .expect("cancel the folder dialog");
        runtime.frame(0.0);
        assert!(runtime.state.preferences.sidebar_open);
        assert!(runtime.state.preferences.projects.is_empty());
    }

    #[test]
    fn keyboard_opens_workbench_switches_tabs_and_returns_focus() {
        let mut runtime = test_runtime();
        let (x, y) = runtime.find("node-4").expect("fixture card");
        click(&mut runtime, x, y);
        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.preview_node, Some(4));
        assert!(runtime.find("graph-preview").is_some());
        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.preview_node, None);
        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods {
                shift: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.preview_node, Some(4));
        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.preview_node, None);
        assert_eq!(runtime.state.selected, Some(4));
        assert!(!runtime.state.workbench_open);

        runtime.key(KeyEvent {
            key: Key::Enter,
            mods: Mods::default(),
        });
        for _ in 0..24 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(
            runtime.state.workbench_open,
            "Return opens the focused graph card"
        );
        let (_, workbench_opacity) = runtime.presented("workbench").expect("workbench mounted");
        assert!(
            workbench_opacity > 0.99,
            "the workbench is fully visible before pointer use: {workbench_opacity}"
        );
        assert_eq!(runtime.state.review.tab, WorkbenchTab::Assignment);

        runtime.key(KeyEvent {
            key: Key::Command('2'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.review.tab, WorkbenchTab::Output);

        let (sx, sy) = runtime.find("workbench-stream").expect("stream button");
        runtime.pointer_move(sx, sy);
        runtime.frame(0.0);
        click(&mut runtime, sx, sy);
        assert!(runtime.state.streaming, "the stream button is pressed");
        let focused_before = runtime
            .elements()
            .into_iter()
            .find(|element| element.focused)
            .and_then(|element| element.key);
        assert_eq!(focused_before.as_deref(), Some("workbench-stream"));

        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(0.0);
        assert!(!runtime.state.workbench_open);
        let focused_after = runtime
            .elements()
            .into_iter()
            .find(|element| element.focused)
            .and_then(|element| element.key);
        assert_eq!(
            focused_after.as_deref(),
            Some("node-4"),
            "closing the workbench returns focus to its opener"
        );
    }

    #[test]
    fn keyboard_history_and_output_shortcuts_work() {
        use rune::Env;

        let mut state = State::default();
        scene_history_runs(&mut state);
        state.workbench_open = true;
        state.run_output_at_bottom = false;
        let mut runtime = Runtime::new(state, view, WINDOW_SIZE, Env::new(false, 1.0));
        runtime.poll = Some(poll);
        runtime.on_key = Some(on_key);
        runtime.check_live_only(false);
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.review.history.kind,
            crate::state::HistoryKind::Runs
        );
        runtime.key(KeyEvent {
            key: Key::Text("n".into()),
            mods: Mods::default(),
        });
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.review.history.kind,
            crate::state::HistoryKind::Revisions
        );
        runtime.key(KeyEvent {
            key: Key::Command('j'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert!(runtime.state.run_output_at_bottom);
    }

    #[test]
    fn keyboard_reset_and_selection_work() {
        let mut runtime = test_runtime();
        let (x, y) = runtime.find("node-5").unwrap();
        click(&mut runtime, x, y);
        assert_eq!(runtime.state.selected, Some(5));

        runtime.state.camera.zoom.set(1.8);
        runtime.state.camera.set_pan([-100.0, -50.0]);
        runtime.frame(0.0);
        runtime.key(KeyEvent {
            key: Key::Command('0'),
            mods: Mods {
                cmd: true,
                ..Mods::default()
            },
        });
        runtime.frame(0.0);
        assert_eq!(runtime.state.camera.zoom(), 0.88);
        assert_eq!(runtime.state.camera.pan(), [22.0, 44.0]);

        runtime.key(KeyEvent {
            key: Key::Escape,
            mods: Mods::default(),
        });
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.selected, None,
            "Escape clears an idle selection"
        );
    }
}
