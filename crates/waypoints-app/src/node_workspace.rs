use gpui_kit::TestSupportExt as _;
use gpui_kit::base::{Disableable as _, StyledExt as _};
#[path = "node_history.rs"]
mod node_history;
#[path = "node_runs.rs"]
mod node_runs;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffFile, DiffFileStatus, DiffMode, DiffState},
    input::{Textarea, TextareaState},
    text::TextView,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, ClipboardItem, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, Window, div, relative,
};
use waypoints_backend::{Client, RunStatus};
use waypoints_core::storage::{Node, NodeRevisionPage};

use super::{Load, Workspace, message, run_backend, workflow_graph::node_title};

type NodeKey = (u64, i64, i64);

#[derive(Default, Clone, Copy, PartialEq, Eq)]
enum Tab {
    #[default]
    Assignment,
    Output,
    Changes,
    History,
}

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Current,
    Run(i64),
    Revision(String),
}

struct Review {
    label: String,
    commit: String,
    conflicted: bool,
    files: Vec<(SharedString, Entity<DiffState>)>,
    selected: usize,
}

pub(super) struct NodeWorkspace {
    run: node_runs::RunPanel,
    saved: String,
    draft: Entity<TextareaState>,
    _watch_draft: Subscription,
    assignment_preview: bool,
    assignment_preview_scroll: ScrollHandle,
    saving: bool,
    save_error: Option<String>,
    tab: Tab,
    target: Target,
    review: Load<Review>,
    review_generation: u64,
    current_review_stale: bool,
    review_wrap: bool,
    review_mode: DiffMode,
    review_files_scroll: ScrollHandle,
    history: Load<(Vec<RunStatus>, Result<NodeRevisionPage, String>)>,
    history_scroll: gpui_kit::ListState,
    history_stale: bool,
    history_generation: u64,
    loading_older: bool,
    older_error: Option<String>,
}

fn short_id(id: &str) -> String {
    id.chars().take(12).collect()
}

fn target_label(target: &Target) -> String {
    match target {
        Target::Current => "Current recorded revision".into(),
        Target::Run(id) => format!("Run #{id} snapshot"),
        Target::Revision(commit) => format!("Native revision {}", short_id(commit)),
    }
}

fn split_path(path: &str) -> (&str, Option<&str>) {
    path.rsplit_once('/')
        .map_or((path, None), |(directory, name)| (name, Some(directory)))
}

fn status_label(status: DiffFileStatus) -> &'static str {
    match status {
        DiffFileStatus::Added => "A",
        DiffFileStatus::Deleted => "D",
        DiffFileStatus::Modified => "M",
        DiffFileStatus::Renamed => "R",
        DiffFileStatus::Copied => "C",
        DiffFileStatus::Unchanged => "=",
        DiffFileStatus::Conflicted => "!",
        _ => "?",
    }
}

fn markdown_with_copy(id: impl Into<gpui_kit::ElementId>, text: String) -> TextView {
    TextView::markdown(id, text)
        .selectable(true)
        .code_block_actions(|block, _, _cx| {
            let code = block.code();
            Button::new("copy-code")
                .ghost()
                .small()
                .label("Copy")
                .accessibility_label("Copy code")
                .on_click(move |_, _, cx| {
                    cx.write_to_clipboard(ClipboardItem::new_string(code.to_string()));
                })
        })
}

#[cfg(feature = "screenshot-fixtures")]
#[derive(Clone, Copy)]
pub(crate) enum ScreenshotSurface {
    Assignment,
    Output,
    Changes,
    History,
}

#[cfg(feature = "screenshot-fixtures")]
const SCREENSHOT_ASSIGNMENT_SAVED: &str = "Polish the node workspace.";

#[cfg(feature = "screenshot-fixtures")]
const SCREENSHOT_ASSIGNMENT_DRAFT: &str = r#"# Polish the node workspace

Improve the existing Waypoints node workspace so that reading assignments, following agent work, reviewing code, and understanding history feel deliberate and polished.

## Scope

- Preserve the sidebar, dependency graph, and per-node workspace.
- Keep the existing Diff component and its retained per-file state.
- Make the comparison target explicit and keep historical targets pinned.
- Keep permission requests from stealing navigation.

## Review expectations

The exact target should be obvious. Current review is invalidated when a run records a new revision, but a deliberately pinned run snapshot must not move.

```rust
fn target_label(target: &Target) -> String {
    match target {
        Target::Current => "Current recorded revision".into(),
        Target::Run(id) => format!("Run #{id} snapshot"),
        Target::Revision(commit) => format!("Native revision {}", short_id(commit)),
    }
}
```

Use `cargo test -p waypoints-app --locked` for focused checks, then run the workspace-wide checks before reporting completion.

> Keep the view quiet: subdued structure, readable code, and no nested cards.

See the [GPUI Kit design guides](https://example.invalid/design-guides) for the intended density and spacing."#;

#[cfg(feature = "screenshot-fixtures")]
const SCREENSHOT_PATCH: &str = r#"diff --git a/crates/waypoints-app/src/node_workspace.rs b/crates/waypoints-app/src/node_workspace.rs
index 1111111..2222222 100644
--- a/crates/waypoints-app/src/node_workspace.rs
+++ b/crates/waypoints-app/src/node_workspace.rs
@@ -78,13 +78,18 @@ fn short_id(id: &str) -> String {
     id.chars().take(12).collect()
 }
 
-fn target_label(target: &Target) -> String {
+fn target_label(target: &Target) -> String {
     match target {
-        Target::Current => "Current".into(),
+        Target::Current => "Current recorded revision".into(),
         Target::Run(id) => format!("Run #{id} snapshot"),
-        Target::Revision(commit) => format!("Revision {commit}"),
+        Target::Revision(commit) => format!("Native revision {}", short_id(commit)),
     }
 }
+
+fn split_path(path: &str) -> (&str, Option<&str>) {
+    path.rsplit_once('/')
+        .map_or((path, None), |(directory, name)| (name, Some(directory)))
+}
 
 fn status_label(status: DiffFileStatus) -> &'static str {
     match status {
diff --git a/crates/waypoints-app/src/node_runs.rs b/crates/waypoints-app/src/node_runs.rs
index 3333333..4444444 100644
--- a/crates/waypoints-app/src/node_runs.rs
+++ b/crates/waypoints-app/src/node_runs.rs
@@ -997,6 +1003,19 @@ fn tool_heading(fields: &serde_json::Value, fallback: &str) -> String {
 }
 
 fn tool_summary(fields: &serde_json::Value) -> Option<String> {
+    if let Some(paths) = fields.get("locations").and_then(serde_json::Value::as_array) {
+        let paths = paths
+            .iter()
+            .filter_map(|location| location.get("path").and_then(serde_json::Value::as_str))
+            .collect::<Vec<_>>();
+        if let Some(first) = paths.first() {
+            return Some(if paths.len() > 1 {
+                format!("{first} +{}", paths.len() - 1)
+            } else {
+                (*first).to_owned()
+            });
+        }
+    }
     if let Some(command) = fields
         .pointer("/rawInput/command")
         .and_then(serde_json::Value::as_str)
diff --git a/crates/waypoints-app/src/node_history.rs b/crates/waypoints-app/src/node_history.rs
new file mode 100644
index 0000000..5555555
--- /dev/null
+++ b/crates/waypoints-app/src/node_history.rs
@@ -0,0 +1,22 @@
+use gpui_kit::{App, Context, ListState};
+
+const HISTORY_PAGE_SIZE: usize = 30;
+
+fn readable_timestamp(timestamp: &str) -> String {
+    let timestamp = timestamp.trim();
+    let Some((date, time)) = timestamp.split_once('T') else {
+        return timestamp.to_owned();
+    };
+    let time = time.trim_end_matches('Z');
+    let clock = time.get(..5).unwrap_or(time);
+    format!("{date} {clock} UTC")
+}
+
+fn describe_operation(operation: &str) -> String {
+    let operation = operation.trim();
+    let mut characters = operation.chars();
+    let Some(first) = characters.next() else {
+        return String::new();
+    };
+    first.to_uppercase().collect::<String>() + characters.as_str()
+}
diff --git a/docs/05-ui-and-diffs.md b/docs/05-ui-and-diffs.md
index 6666666..7777777 100644
--- a/docs/05-ui-and-diffs.md
+++ b/docs/05-ui-and-diffs.md
@@ -12,7 +12,13 @@ The node workspace has four tabs.
 
 ## Changes
 
-The Changes tab shows a patch.
+The Changes tab shows a patch with an explicit comparison target.
+
+- Current recorded revision
+- Saved run snapshot
+- Native revision
+
+Each file keeps its own retained diff state and scroll position.
 
 ## History
 
diff --git a/docs/ui-guide.md b/docs/new-ui-guide.md
similarity index 82%
rename from docs/ui-guide.md
rename to docs/new-ui-guide.md
index aaaaaaa..bbbbbbb 100644
--- a/docs/ui-guide.md
+++ b/docs/new-ui-guide.md
@@ -1,3 +1,3 @@
 # Working in the desktop app
 
-The old wording.
+The updated wording.
diff --git a/assets/icon.png b/assets/icon.png
index 1111111..2222222 100644
Binary files a/assets/icon.png and b/assets/icon.png differ
diff --git a/apps/web/src/features/review/components/ReviewDiffToolbarWithAnExtremelyLongName.tsx b/apps/web/src/features/review/components/ReviewDiffToolbarWithAnExtremelyLongName.tsx
index 8888888..9999999 100644
--- a/apps/web/src/features/review/components/ReviewDiffToolbarWithAnExtremelyLongName.tsx
+++ b/apps/web/src/features/review/components/ReviewDiffToolbarWithAnExtremelyLongName.tsx
@@ -1,5 +1,12 @@
 import { DiffViewer } from "./diff-viewer";
 
-export function ReviewDiffToolbar() {
-  return <DiffViewer file={file} />;
+export function ReviewDiffToolbar({ file, mode }: ReviewDiffToolbarProps) {
+  const label = mode === "split" ? "Split view" : "Unified view";
+
+  return (
+    <section aria-label="Diff toolbar">
+      <span>{label}</span>
+      <DiffViewer file={file} mode={mode} />
+    </section>
+  );
 }
"#;

#[cfg(feature = "screenshot-fixtures")]
impl NodeWorkspace {
    pub(super) fn install_screenshot_fixture(
        &mut self,
        surface: ScreenshotSurface,
        window: &mut Window,
        cx: &mut gpui_kit::App,
    ) {
        match surface {
            ScreenshotSurface::Assignment => {
                self.tab = Tab::Assignment;
                self.assignment_preview = true;
                self.saved = SCREENSHOT_ASSIGNMENT_SAVED.into();
                self.draft.update(cx, |draft, cx| {
                    draft.set_value(SCREENSHOT_ASSIGNMENT_DRAFT, window, cx)
                });
                self.saving = false;
                self.save_error = None;
            }
            ScreenshotSurface::Output => {
                self.tab = Tab::Output;
                self.run.install_screenshot_fixture(cx);
            }
            ScreenshotSurface::Changes => {
                self.tab = Tab::Changes;
                self.target = Target::Current;
                self.current_review_stale = true;
                self.review_wrap = false;
                self.review_mode = DiffMode::Unified;
                let files = DiffFile::parse(SCREENSHOT_PATCH)
                    .expect("screenshot patch parses as a native diff");
                self.review = Load::Ready(Review {
                    label: target_label(&self.target),
                    commit: "0123456789abcdef0123456789abcdef01234567".into(),
                    conflicted: false,
                    files: files
                        .into_iter()
                        .map(|file| {
                            let path = file.path().clone();
                            (
                                path,
                                cx.new(|cx| DiffState::new([file], cx).with_mode(self.review_mode)),
                            )
                        })
                        .collect(),
                    selected: 0,
                });
            }
            ScreenshotSurface::History => {
                self.tab = Tab::History;
                self.install_screenshot_history();
            }
        }
    }
}

fn current_review_changed(target: &Target, review: &Load<Review>, saved_commit: &str) -> bool {
    matches!(target, Target::Current)
        && match review {
            Load::Ready(review) => review.commit != saved_commit,
            Load::Loading => true,
            _ => false,
        }
}

fn review_needs_load(target: &Target, review: &Load<Review>, current_stale: bool) -> bool {
    matches!(review, Load::Idle) || (matches!(target, Target::Current) && current_stale)
}

impl Workspace {
    #[cfg(test)]
    pub(super) fn assignment_draft(&self, key: NodeKey, cx: &gpui_kit::App) -> SharedString {
        self.node_workspaces[&key].draft.read(cx).value()
    }

    pub(super) fn open_node(&mut self, id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let (Some((project, workflow)), Load::Ready(graph)) = (self.state.selected, &self.graph)
        else {
            return;
        };
        let Some(node) = graph.nodes.iter().find(|node| node.id == id) else {
            return;
        };
        let key = (project, workflow, id);
        self.selected_nodes.insert((project, workflow), id);
        self.node_workspaces.entry(key).or_insert_with(|| {
            let saved = node.prompt.clone();
            let draft = cx.new(|cx| TextareaState::new(window, cx).default_value(saved.clone()));
            let watch_draft = cx.observe(&draft, |_, _, cx| cx.notify());
            NodeWorkspace {
                run: node_runs::RunPanel::new(window, cx),
                saved,
                draft,
                _watch_draft: watch_draft,
                assignment_preview: false,
                assignment_preview_scroll: ScrollHandle::new(),
                saving: false,
                save_error: None,
                tab: Tab::default(),
                target: Target::Current,
                review: Load::Idle,
                review_generation: 0,
                current_review_stale: false,
                review_wrap: false,
                review_mode: DiffMode::Unified,
                review_files_scroll: ScrollHandle::new(),
                history: Load::Idle,
                history_scroll: gpui_kit::ListState::new(
                    1,
                    gpui_kit::ListAlignment::Top,
                    gpui_kit::px(400.),
                ),
                history_stale: false,
                history_generation: 0,
                loading_older: false,
                older_error: None,
            }
        });
        self.discover_run(key, cx);
        cx.notify();
    }

    fn save_assignment(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        let prompt = node.draft.read(cx).value().to_string();
        if node.saving || prompt == node.saved {
            return;
        }
        node.saving = true;
        node.save_error = None;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        client
                            .update_node(key.1, key.2, &prompt)
                            .await
                            .map_err(|error| error.to_string())
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| this.finish_assignment_save(key, result, cx));
        })
        .detach();
    }

    fn finish_assignment_save(
        &mut self,
        key: NodeKey,
        result: Result<Node, String>,
        cx: &mut Context<Self>,
    ) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        node.saving = false;
        match result {
            Ok(saved) => {
                node.saved.clone_from(&saved.prompt);
                if self.state.selected == Some((key.0, key.1))
                    && let Load::Ready(graph) = &mut self.graph
                    && let Some(entry) = graph.nodes.iter_mut().find(|entry| entry.id == key.2)
                {
                    *entry = saved;
                }
            }
            Err(error) => node.save_error = Some(error),
        }
        cx.notify();
    }

    fn open_review(&mut self, key: NodeKey, target: Target, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        node.tab = Tab::Changes;
        let previous = std::mem::replace(&mut node.review, Load::Loading);
        let previous = match previous {
            Load::Ready(review) if node.target == target => Some(review),
            _ => None,
        };
        if node.target != target {
            node.review_files_scroll = ScrollHandle::new();
        }
        node.target = target.clone();
        if target == Target::Current {
            node.current_review_stale = false;
        }
        node.review_generation += 1;
        let generation = node.review_generation;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    let review = run_backend(async |client: Client| {
                        match &target {
                            Target::Current => client.review_node(key.1, key.2).await,
                            Target::Run(id) => client.review_run(*id).await,
                            Target::Revision(commit) => {
                                client.review_node_revision(key.1, key.2, commit).await
                            }
                        }
                        .map_err(|error| error.to_string())
                    })?;
                    let files =
                        DiffFile::parse(&review.patch).map_err(|error| error.to_string())?;
                    let source = match &target {
                        Target::Current => "Current recorded revision".to_owned(),
                        Target::Run(id) => format!("Run #{id} snapshot"),
                        Target::Revision(commit) => {
                            format!("Native revision {}", short_id(commit))
                        }
                    };
                    Ok::<_, String>((
                        source,
                        review.revision.commit_id,
                        review.revision.conflicted,
                        files,
                    ))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key)
                    && node.review_generation == generation
                {
                    node.review = match result {
                        Ok((_, ref commit, _, _))
                            if previous
                                .as_ref()
                                .is_some_and(|review| review.commit == *commit) =>
                        {
                            previous.map_or(Load::Idle, Load::Ready)
                        }
                        Ok((label, commit, conflicted, files)) => {
                            let selected_path = previous.as_ref().and_then(|review| {
                                review.files.get(review.selected).map(|(path, _)| path)
                            });
                            let selected = files
                                .iter()
                                .position(|file| selected_path == Some(file.path()))
                                .unwrap_or(0);
                            Load::Ready(Review {
                                label,
                                commit,
                                conflicted,
                                files: files
                                    .into_iter()
                                    .map(|file| {
                                        let path = file.path().clone();
                                        (
                                            path,
                                            cx.new(|cx| {
                                                DiffState::new([file], cx)
                                                    .with_mode(node.review_mode)
                                            }),
                                        )
                                    })
                                    .collect(),
                                selected,
                            })
                        }
                        Err(error) => Load::Error(error),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_tab(&mut self, key: NodeKey, tab: Tab, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        node.tab = tab;
        if tab == Tab::Changes
            && review_needs_load(&node.target, &node.review, node.current_review_stale)
        {
            let target = node.target.clone();
            self.open_review(key, target, cx);
        } else if tab == Tab::History && (node.history_stale || matches!(node.history, Load::Idle))
        {
            self.load_history(key, cx);
        }
        cx.notify();
    }

    pub(super) fn render_node_workspace(&self, cx: &Context<Self>) -> impl IntoElement {
        let root = div()
            .size_full()
            .min_h_0()
            .min_w_0()
            .flex()
            .flex_col()
            .gap_2()
            .p_3();
        let Some((project, workflow)) = self.state.selected else {
            return root;
        };
        let Some(&id) = self.selected_nodes.get(&(project, workflow)) else {
            return root.child(message("Select a node to open its workspace."));
        };
        let key = (project, workflow, id);
        let Some(node) = self.node_workspaces.get(&key) else {
            return root;
        };
        let tabs = [
            ("Assignment", Tab::Assignment),
            ("Output", Tab::Output),
            ("Changes", Tab::Changes),
            ("History", Tab::History),
        ];
        let root = root
            .child(
                div()
                    .font_semibold()
                    .overflow_hidden()
                    .text_ellipsis()
                    .child(node_title(id, &node.saved)),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children(tabs.into_iter().map(|(label, tab)| {
                        Button::new(label)
                            .ghost()
                            .label(label)
                            .selected(node.tab == tab)
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.open_tab(key, tab, cx)),
                            )
                    })),
            );
        match node.tab {
            Tab::Assignment => {
                let draft = node.draft.read(cx).value();
                let dirty = draft.as_ref() != node.saved;
                let toolbar = div()
                    .flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .children(
                        [("Edit", false), ("Preview", true)].map(|(label, preview)| {
                            Button::new(label)
                                .ghost()
                                .label(label)
                                .selected(node.assignment_preview == preview)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(node) = this.node_workspaces.get_mut(&key) {
                                        node.assignment_preview = preview;
                                        cx.notify();
                                    }
                                }))
                        }),
                    )
                    .child(
                        Button::new("save-assignment")
                            .primary()
                            .label("Save assignment")
                            .disabled(!dirty || node.saving)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.save_assignment(key, cx);
                            })),
                    )
                    .child(message(if node.saving {
                        "Saving…"
                    } else if dirty {
                        "Unsaved edits"
                    } else {
                        "Saved assignment"
                    }));
                let root = root
                    .child(toolbar)
                    .when_some(node.save_error.clone(), |root, error| {
                        root.child(message(error).text_color(cx.theme().danger))
                    });
                if node.assignment_preview {
                    root.child(
                        div()
                            .id("assignment-preview")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .track_scroll(&node.assignment_preview_scroll)
                            .child(
                                div()
                                    .max_w(gpui_kit::rems(48.))
                                    .p_3()
                                    .when(draft.trim().is_empty(), |body| {
                                        body.child(
                                            message("This assignment is empty.")
                                                .text_color(cx.theme().muted_foreground),
                                        )
                                    })
                                    .when(!draft.trim().is_empty(), |body| {
                                        body.child(markdown_with_copy(
                                            format!("assignment-{}-{}-{}", key.0, key.1, key.2),
                                            draft.to_string(),
                                        ))
                                    }),
                            ),
                    )
                } else {
                    root.child(
                        message("Saved assignments apply to future runs.")
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div().flex_1().min_h_0().child(
                            Textarea::new(&node.draft)
                                .h(relative(1.))
                                .aria_label("Assignment"),
                        ),
                    )
                }
            }
            Tab::Output => self.render_run_panel(key, root, cx),
            Tab::Changes => {
                let target = target_label(&node.target);
                let root = root.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("current-review")
                                .ghost()
                                .label("Review current")
                                .disabled(node.target == Target::Current)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_review(key, Target::Current, cx)
                                })),
                        )
                        .child(
                            Button::new("reload-review")
                                .ghost()
                                .label("Reload review")
                                .disabled(matches!(node.review, Load::Loading))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(node) = this.node_workspaces.get(&key) {
                                        this.open_review(key, node.target.clone(), cx);
                                    }
                                })),
                        ),
                );
                match &node.review {
                    Load::Idle | Load::Loading => root
                        .child(div().font_semibold().child(target.clone()))
                        .child(message("Loading changes…").text_color(cx.theme().muted_foreground)),
                    Load::Error(error) => root
                        .child(div().font_semibold().child(target.clone()))
                        .child(message(error.clone()).text_color(cx.theme().danger))
                        .child(
                            Button::new("retry-review")
                                .ghost()
                                .small()
                                .label("Try again")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(node) = this.node_workspaces.get(&key) {
                                        this.open_review(key, node.target.clone(), cx);
                                    }
                                })),
                        ),
                    Load::Ready(review) => {
                        let commit = review.commit.clone();
                        let short_commit = short_id(&commit);
                        let mut header = div()
                            .flex()
                            .gap_2()
                            .flex_wrap()
                            .items_center()
                            .child(div().font_semibold().child(review.label.clone()))
                            .child(
                                Button::new("copy-review-id")
                                    .ghost()
                                    .small()
                                    .label(short_commit)
                                    .accessibility_label("Copy revision ID")
                                    .tooltip(format!("Copy revision {commit}"))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            commit.clone(),
                                        ));
                                    }),
                            )
                            .when(review.conflicted, |row| {
                                row.child(message("Conflicts").text_color(cx.theme().warning))
                            })
                            .when(
                                node.target == Target::Current && node.current_review_stale,
                                |row| {
                                    row.child(div().id("review-stale").test_support().child(
                                        message(
                                            "The recorded revision changed. Reload to review the latest.",
                                        )
                                        .text_color(cx.theme().warning),
                                    ))
                                },
                            )
                            .child(
                                Button::new("wrap-review")
                                    .ghost()
                                    .small()
                                    .label(if node.review_wrap {
                                        "Wrapping lines"
                                    } else {
                                        "Unwrapped lines"
                                    })
                                    .selected(node.review_wrap)
                                    .tooltip(if node.review_wrap {
                                        "Show long lines without wrapping"
                                    } else {
                                        "Wrap long lines"
                                    })
                                    .accessibility_label("Wrap long lines")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(node) = this.node_workspaces.get_mut(&key) {
                                            node.review_wrap = !node.review_wrap;
                                            cx.notify();
                                        }
                                    })),
                            )
                            .child(
                                Button::new("split-review")
                                    .ghost()
                                    .small()
                                    .label(match node.review_mode {
                                        DiffMode::Split => "Split view",
                                        _ => "Unified view",
                                    })
                                    .selected(node.review_mode == DiffMode::Split)
                                    .tooltip(match node.review_mode {
                                        DiffMode::Split => "Switch to unified view",
                                        _ => "Switch to split view",
                                    })
                                    .accessibility_label("Toggle split view")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        if let Some(node) = this.node_workspaces.get_mut(&key) {
                                            node.review_mode = match node.review_mode {
                                                DiffMode::Split => DiffMode::Unified,
                                                _ => DiffMode::Split,
                                            };
                                            if let Load::Ready(review) = &node.review {
                                                for (_, diff) in &review.files {
                                                    diff.update(cx, |diff, cx| {
                                                        diff.set_mode(node.review_mode, cx)
                                                    });
                                                }
                                            }
                                            cx.notify();
                                        }
                                    })),
                            );
                        if review.files.is_empty() {
                            header = header.child(
                                message("This revision has no file changes.")
                                    .text_color(cx.theme().muted_foreground),
                            );
                            return root.child(header);
                        }
                        let root = root.child(header);
                        let additions = review
                            .files
                            .iter()
                            .map(|(_, diff)| diff.read(cx).files()[0].additions())
                            .sum::<usize>();
                        let deletions = review
                            .files
                            .iter()
                            .map(|(_, diff)| diff.read(cx).files()[0].deletions())
                            .sum::<usize>();
                        let binary = review
                            .files
                            .iter()
                            .filter(|(_, diff)| diff.read(cx).files()[0].is_binary())
                            .count();
                        let files = review.files.len();
                        let Some((_, diff)) = review.files.get(review.selected) else {
                            return root.child(
                                message("Select a changed file to review.")
                                    .text_color(cx.theme().muted_foreground),
                            );
                        };
                        root.child(
                            div()
                                .flex()
                                .items_stretch()
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .w_56()
                                        .max_w(relative(0.36))
                                        .min_h_0()
                                        .border_r_1()
                                        .border_color(cx.theme().border)
                                        .pr_2()
                                        .child(
                                            div()
                                                .px_1()
                                                .pb_1()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!(
                                                    "{files} {} · +{additions} −{deletions}{}",
                                                    if files == 1 { "file" } else { "files" },
                                                    if binary > 0 {
                                                        format!(" · {binary} binary")
                                                    } else {
                                                        String::new()
                                                    }
                                                )),
                                        )
                                        .child(
                                            div()
                                                .id("review-files")
                                                .flex_1()
                                                .min_h_0()
                                                .overflow_y_scroll()
                                                .track_scroll(&node.review_files_scroll)
                                                .children(review.files.iter().enumerate().map(
                                                    |(index, (path, diff))| {
                                                        let file = &diff.read(cx).files()[0];
                                                        let status = file.status();
                                                        let (name, directory) = split_path(path);
                                                        let secondary = if status
                                                            == DiffFileStatus::Renamed
                                                        {
                                                            file.original_path()
                                                                .map(|old| {
                                                                    format!("{old} → {path}")
                                                                })
                                                                .unwrap_or_else(|| {
                                                                    directory.unwrap_or("").into()
                                                                })
                                                        } else {
                                                            directory.unwrap_or("").to_owned()
                                                        };
                                                        let counts = if file.is_binary() {
                                                            "Binary".to_owned()
                                                        } else if status
                                                            == DiffFileStatus::Renamed
                                                            && file.additions() == 0
                                                            && file.deletions() == 0
                                                        {
                                                            "Renamed".to_owned()
                                                        } else {
                                                            format!(
                                                                "+{} −{}",
                                                                file.additions(),
                                                                file.deletions()
                                                            )
                                                        };
                                                        let status_color = match status {
                                                            DiffFileStatus::Added => {
                                                                cx.theme().success
                                                            }
                                                            DiffFileStatus::Deleted => {
                                                                cx.theme().danger
                                                            }
                                                            DiffFileStatus::Conflicted => {
                                                                cx.theme().warning
                                                            }
                                                            _ => cx.theme().muted_foreground,
                                                        };
                                                        Button::new(path.clone())
                                                            .ghost()
                                                            .w_full()
                                                            .tooltip(path.to_string())
                                                            .selected(review.selected == index)
                                                            .accessibility_label(format!(
                                                                "Review {path}"
                                                            ))
                                                            .child(
                                                                div()
                                                                    .min_w_0()
                                                                    .flex_1()
                                                                    .text_left()
                                                                    .child(
                                                                        div()
                                                                            .flex()
                                                                            .items_center()
                                                                            .gap_2()
                                                                            .child(
                                                                                div()
                                                                                    .w_3()
                                                                                    .text_xs()
                                                                                    .text_color(
                                                                                        status_color,
                                                                                    )
                                                                                    .child(
                                                                                        status_label(
                                                                                            status,
                                                                                        ),
                                                                                    ),
                                                                            )
                                                                            .child(
                                                                                div()
                                                                                    .min_w_0()
                                                                                    .flex_1()
                                                                                    .truncate()
                                                                                    .child(
                                                                                        name.to_owned(),
                                                                                    ),
                                                                            )
                                                                            .child(
                                                                                div()
                                                                                    .text_xs()
                                                                                    .text_color(
                                                                                        cx.theme()
                                                                                            .muted_foreground,
                                                                                    )
                                                                                    .child(counts),
                                                                            ),
                                                                    )
                                                                    .when(
                                                                        !secondary.is_empty(),
                                                                        |row| {
                                                                            row.child(
                                                                                div()
                                                                                    .truncate()
                                                                                    .text_xs()
                                                                                    .text_color(
                                                                                        cx.theme()
                                                                                            .muted_foreground,
                                                                                    )
                                                                                    .child(secondary),
                                                                            )
                                                                        },
                                                                    ),
                                                            )
                                                            .on_click(cx.listener(
                                                                move |this, _, _, cx| {
                                                                    if let Some(node) = this
                                                                        .node_workspaces
                                                                        .get_mut(&key)
                                                                        && let Load::Ready(
                                                                            review,
                                                                        ) = &mut node.review
                                                                    {
                                                                        review.selected = index;
                                                                        cx.notify();
                                                                    }
                                                                },
                                                            ))
                                                    },
                                                )),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex_1()
                                        .min_h_0()
                                        .min_w_0()
                                        .child(Diff::new(diff).soft_wrap(node.review_wrap)),
                                ),
                        )
                    }
                }
            }
            Tab::History => self.render_node_history(key, node, root, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_navigation_separates_names_from_directories_and_labels_statuses() {
        assert_eq!(
            split_path("crates/waypoints-app/src/node_workspace.rs"),
            ("node_workspace.rs", Some("crates/waypoints-app/src"))
        );
        assert_eq!(split_path("README.md"), ("README.md", None));
        assert_eq!(split_path(""), ("", None));
        assert_eq!(target_label(&Target::Current), "Current recorded revision");
        assert_eq!(target_label(&Target::Run(12)), "Run #12 snapshot");
        assert_eq!(
            target_label(&Target::Revision("abcdef1234567890".into())),
            "Native revision abcdef123456"
        );
        for (status, label) in [
            (DiffFileStatus::Added, "A"),
            (DiffFileStatus::Deleted, "D"),
            (DiffFileStatus::Modified, "M"),
            (DiffFileStatus::Renamed, "R"),
            (DiffFileStatus::Copied, "C"),
            (DiffFileStatus::Unchanged, "="),
            (DiffFileStatus::Conflicted, "!"),
        ] {
            assert_eq!(status_label(status), label);
        }
    }

    #[test]
    fn snapshot_changes_invalidate_current_only_when_needed() {
        let review = Load::Ready(Review {
            label: "Recorded revision".into(),
            commit: "before".into(),
            conflicted: false,
            files: Vec::new(),
            selected: 0,
        });
        assert!(!current_review_changed(&Target::Current, &review, "before"));
        let stale = current_review_changed(&Target::Current, &review, "after");
        assert!(stale);
        assert!(review_needs_load(&Target::Current, &review, stale));
        assert!(current_review_changed(
            &Target::Current,
            &Load::Loading,
            "after"
        ));
        for target in [Target::Run(1), Target::Revision("before".into())] {
            assert!(!current_review_changed(&target, &review, "after"));
            assert!(!review_needs_load(&target, &review, stale));
        }
    }

    #[gpui_kit::test]
    fn stale_notice_shows_for_current_but_not_for_a_pinned_run(cx: &mut gpui_kit::TestAppContext) {
        use crate::workspace::workspace_tests::open_workspace;
        use gpui_kit::test::TestWindowExt as _;
        use serde_json::json;

        let (handle, workspace) = open_workspace(cx);
        let key = (1, 7, 1);
        cx.update_window(handle, |_, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": crate::workspace::workspace_tests::workflow("project-a", 7),
                        "nodes": [{"id": 1, "workflow_id": 7, "prompt": "Review"}],
                        "connections": []
                    }))
                    .expect("graph fixture matches the storage schema"),
                );
                workspace.open_node(1, window, cx);
                let node = workspace.node_workspaces.get_mut(&key).expect("open node");
                node.tab = Tab::Changes;
                node.target = Target::Current;
                node.current_review_stale = true;
                node.review = Load::Ready(Review {
                    label: "Current recorded revision".into(),
                    commit: "old".into(),
                    conflicted: false,
                    files: Vec::new(),
                    selected: 0,
                });
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("review-stale").visible());

            workspace.update(cx, |workspace, cx| {
                let node = workspace.node_workspaces.get_mut(&key).expect("open node");
                node.target = Target::Run(42);
                node.current_review_stale = false;
                node.review = Load::Ready(Review {
                    label: "Run #42 snapshot".into(),
                    commit: "old".into(),
                    conflicted: false,
                    files: Vec::new(),
                    selected: 0,
                });
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.try_find("review-stale").is_none());
            assert!(matches!(
                workspace.read(cx).node_workspaces[&key].target,
                Target::Run(42)
            ));
        })
        .expect("workspace window remains open");
    }
    #[gpui_kit::test]
    fn edits_while_saving_remain_a_draft_on_success_and_error(cx: &mut gpui_kit::TestAppContext) {
        use crate::workspace::workspace_tests::{open_workspace, workflow};
        use gpui_kit::test::TestWindowExt as _;
        use serde_json::json;

        let (handle, workspace) = open_workspace(cx);
        let key = (1, 7, 1);
        cx.update_window(handle, |_, window, cx| {
            let submitted: Node = serde_json::from_value(json!({
                "id": 1, "workflow_id": 7, "prompt": "Submitted assignment"
            }))
            .expect("node fixture matches the storage schema");
            let mut previous = submitted.clone();
            previous.prompt = "Previous assignment".into();
            workspace.update(cx, |workspace, cx| {
                workspace.graph = Load::Ready(
                    serde_json::from_value(json!({
                        "workflow": workflow("project-a", 7),
                        "nodes": [previous],
                        "connections": []
                    }))
                    .expect("graph fixture matches the storage schema"),
                );
                workspace.open_node(1, window, cx);
                workspace
                    .node_workspaces
                    .get_mut(&key)
                    .expect("open node")
                    .saving = true;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("input-state", cx);
            window.input("Later edit", cx);
            let edited = workspace.read(cx).assignment_draft(key, cx);
            assert!(edited.contains("Later edit"));

            workspace.update(cx, |workspace, cx| {
                workspace.finish_assignment_save(key, Ok(submitted), cx);
            });
            let node = &workspace.read(cx).node_workspaces[&key];
            assert!(!node.saving);
            assert_eq!(node.saved, "Submitted assignment");
            assert_eq!(node.draft.read(cx).value(), edited);

            workspace.update(cx, |workspace, cx| {
                workspace
                    .node_workspaces
                    .get_mut(&key)
                    .expect("open node")
                    .saving = true;
                workspace.finish_assignment_save(key, Err("Backend unavailable".into()), cx);
            });
            window.click("Preview", cx);
            let node = &workspace.read(cx).node_workspaces[&key];
            assert!(node.assignment_preview);
            assert!(!node.saving);
            assert_eq!(node.save_error.as_deref(), Some("Backend unavailable"));
            assert_eq!(node.draft.read(cx).value(), edited);
        })
        .expect("workspace window remains open");
    }
}
