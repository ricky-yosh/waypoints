use gpui_kit::base::{Disableable as _, StyledExt as _};
#[path = "node_runs.rs"]
mod node_runs;

use gpui_kit::component::{
    ActiveTheme as _, Selectable as _,
    button::{Button, ButtonVariants as _},
    diff::{Diff, DiffFile, DiffState},
    input::{Textarea, TextareaState},
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AppContext as _, Context, Entity, InteractiveElement as _, IntoElement, ParentElement as _,
    ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window,
    div, relative,
};
use waypoints_backend::{Client, RunStatus};
use waypoints_core::{run::SnapshotState, storage::NodeRevisionPage};

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

#[derive(Clone)]
enum Target {
    Current,
    Run(i64),
    Revision(String),
}

struct Review {
    label: String,
    files: Vec<(SharedString, Entity<DiffState>)>,
    selected: usize,
}

pub(super) struct NodeWorkspace {
    run: node_runs::RunPanel,
    saved: String,
    draft: Entity<TextareaState>,
    _watch_draft: Subscription,
    saving: bool,
    save_error: Option<String>,
    tab: Tab,
    target: Target,
    review: Load<Review>,
    review_generation: u64,
    history: Load<(Vec<RunStatus>, Result<NodeRevisionPage, String>)>,
    history_scroll: ScrollHandle,
    history_stale: bool,
}

impl Workspace {
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
                saving: false,
                save_error: None,
                tab: Tab::default(),
                target: Target::Current,
                review: Load::Idle,
                review_generation: 0,
                history: Load::Idle,
                history_scroll: ScrollHandle::new(),
                history_stale: false,
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
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key) {
                    node.saving = false;
                    match result {
                        Ok(saved) => {
                            node.saved.clone_from(&saved.prompt);
                            if this.state.selected == Some((key.0, key.1))
                                && let Load::Ready(graph) = &mut this.graph
                                && let Some(entry) =
                                    graph.nodes.iter_mut().find(|entry| entry.id == key.2)
                            {
                                *entry = saved;
                            }
                        }
                        Err(error) => node.save_error = Some(error),
                    }
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn open_review(&mut self, key: NodeKey, target: Target, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        node.tab = Tab::Changes;
        node.target = target.clone();
        node.review = Load::Loading;
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
                    let source = match target {
                        Target::Current => "Current recorded revision".to_owned(),
                        Target::Run(id) => format!("Run #{id} snapshot"),
                        Target::Revision(_) => "Native revision".to_owned(),
                    };
                    Ok::<_, String>((format!("{source} · {}", review.revision.commit_id), files))
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key)
                    && node.review_generation == generation
                {
                    node.review = match result {
                        Ok((label, files)) => Load::Ready(Review {
                            label,
                            files: files
                                .into_iter()
                                .map(|file| {
                                    let path = file.path().clone();
                                    (path, cx.new(|cx| DiffState::new([file], cx)))
                                })
                                .collect(),
                            selected: 0,
                        }),
                        Err(error) => Load::Error(error),
                    };
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn load_history(&mut self, key: NodeKey, cx: &mut Context<Self>) {
        let Some(node) = self.node_workspaces.get_mut(&key) else {
            return;
        };
        if matches!(node.history, Load::Loading) {
            return;
        }
        node.history = Load::Loading;
        node.history_stale = false;
        cx.notify();
        cx.spawn(async move |this, cx| {
            let result = cx
                .background_spawn(async move {
                    run_backend(|client: Client| async move {
                        let runs = client
                            .node_runs(key.1, key.2)
                            .await
                            .map_err(|error| error.to_string())?;
                        let revisions = client
                            .node_revisions(key.1, key.2, 30, None)
                            .await
                            .map_err(|error| error.to_string());
                        Ok((runs, revisions))
                    })
                })
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(node) = this.node_workspaces.get_mut(&key) {
                    node.history = match result {
                        Ok(history) => Load::Ready(history),
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
        if tab == Tab::Changes && matches!(node.review, Load::Idle) {
            self.open_review(key, Target::Current, cx);
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
                let dirty = node.draft.read(cx).value().as_ref() != node.saved;
                root.child(div().flex().gap_2().items_center().flex_wrap()
                    .child(Button::new("save-assignment").primary().label("Save assignment")
                        .disabled(!dirty || node.saving)
                        .on_click(cx.listener(move |this, _, _, cx| this.save_assignment(key, cx))))
                    .child(message(if node.saving { "Saving…" } else if dirty { "Unsaved edits" } else { "Saved assignment" })))
                    .child(message("Saving updates the assignment for future runs. An active run keeps its original instructions.")
                        .text_color(cx.theme().muted_foreground))
                    .when_some(node.save_error.clone(), |root, error| root.child(message(error).text_color(cx.theme().danger)))
                    .child(div().flex_1().min_h_0().child(Textarea::new(&node.draft)
                        .h(relative(1.)).aria_label("Assignment")))
            }
            Tab::Output => self.render_run_panel(key, root, cx),
            Tab::Changes => {
                let root = root.child(
                    div()
                        .flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("current-review")
                                .ghost()
                                .label("Current recorded revision")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.open_review(key, Target::Current, cx)
                                })),
                        )
                        .child(
                            Button::new("reload-review")
                                .ghost()
                                .label("Reload review")
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(node) = this.node_workspaces.get(&key) {
                                        this.open_review(key, node.target.clone(), cx);
                                    }
                                })),
                        ),
                );
                match &node.review {
                    Load::Idle | Load::Loading => root.child(message("Loading changes…")),
                    Load::Error(error) => {
                        root.child(message(error.clone()).text_color(cx.theme().danger))
                    }
                    Load::Ready(review) => {
                        let root = root.child(message(review.label.clone()));
                        let Some((_, diff)) = review.files.get(review.selected) else {
                            return root.child(message("No file changes in this revision."));
                        };
                        root.child(
                            div()
                                .id("review-files")
                                .flex()
                                .gap_1()
                                .overflow_x_scroll()
                                .children(review.files.iter().enumerate().map(
                                    |(index, (path, _))| {
                                        Button::new(path.clone())
                                            .ghost()
                                            .label(path.clone())
                                            .selected(review.selected == index)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(node) =
                                                    this.node_workspaces.get_mut(&key)
                                                    && let Load::Ready(review) = &mut node.review
                                                {
                                                    review.selected = index;
                                                    cx.notify();
                                                }
                                            }))
                                    },
                                )),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .child(Diff::new(diff).soft_wrap(true)),
                        )
                    }
                }
            }
            Tab::History => {
                let mut list = div()
                    .id("node-history")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&node.history_scroll)
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap_1();
                match &node.history {
                    Load::Idle | Load::Loading => list = list.child(message("Loading history…")),
                    Load::Error(error) => {
                        list = list.child(message(error.clone()).text_color(cx.theme().danger))
                    }
                    Load::Ready((runs, revisions)) => {
                        list = list.child(div().font_semibold().child("Runs"));
                        if runs.is_empty() {
                            list = list.child(message("No runs yet."));
                        }
                        for status in runs {
                            let id = status.run.id;
                            let label = format!(
                                "Run #{id} · {}",
                                node_runs::state_label(&status.run.state)
                            );
                            if !matches!(status.run.snapshot, SnapshotState::Saved { .. }) {
                                list = list.child(message(label));
                                continue;
                            }
                            list = list.child(
                                Button::new(SharedString::from(format!("run-{id}")))
                                    .ghost()
                                    .label(format!("{label} · Review snapshot"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.open_review(key, Target::Run(id), cx)
                                    })),
                            );
                        }
                        list = list.child(div().font_semibold().child("Native revisions"));
                        match revisions {
                            Ok(page) => {
                                for entry in &page.entries {
                                    let commit = entry.evolution.revision.commit_id.clone();
                                    list = list.child(
                                        Button::new(SharedString::from(commit.clone()))
                                            .ghost()
                                            .label(commit.chars().take(12).collect::<String>())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.open_review(
                                                    key,
                                                    Target::Revision(commit.clone()),
                                                    cx,
                                                )
                                            })),
                                    );
                                }
                            }
                            Err(error) => list = list.child(message(error.clone())),
                        }
                    }
                }
                root.child(
                    Button::new("reload-history")
                        .ghost()
                        .label("Refresh history")
                        .on_click(cx.listener(move |this, _, _, cx| this.load_history(key, cx))),
                )
                .child(list)
            }
        }
    }
}
