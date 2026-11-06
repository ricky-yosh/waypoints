use std::collections::{BTreeMap, VecDeque};

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::base::{Disableable as _, Popup, StyledExt as _, TestSupportExt as _};
use gpui_kit::component::{
    ActiveTheme as _, Icon, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    scroll::ScrollableElement as _,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Anchor, App, Bounds, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    PathBuilder, Pixels, ScrollHandle, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, point, px, rems, size,
};
use waypoints_core::run::{Run, RunState, SnapshotState};
use waypoints_core::storage::{Node, WorkflowGraph};

use super::{
    DismissGraphPreview, InspectGraphNode, Workspace,
    node_workspace::{run_state_visual, semantic_status},
};

const CARD_WIDTH: f32 = 13.;
const COLUMN_GAP: f32 = 2.5;
const MIN_COMPACT_CARD_WIDTH: f32 = 9.75;
const PADDING: f32 = 1.0;
const ZOOM_LEVELS: [u8; 5] = [60, 80, 100, 120, 140];
const DEFAULT_ZOOM: usize = 2;
const PREVIEW_OPEN_DELAY: Duration = Duration::from_millis(350);
const PREVIEW_CLOSE_DELAY: Duration = Duration::from_millis(220);

struct GraphNodeStyle {
    selected: bool,
    show_detail: bool,
    preview_max_height: Pixels,
}

pub(super) struct GraphCamera {
    scroll: ScrollHandle,
    zoom: usize,
}

impl Default for GraphCamera {
    fn default() -> Self {
        Self {
            scroll: ScrollHandle::new(),
            zoom: DEFAULT_ZOOM,
        }
    }
}

#[derive(Default)]
pub(super) struct GraphActivity<'a> {
    pub(super) run: Option<&'a Run>,
    pub(super) permission: Option<String>,
    pub(super) assistant: Option<&'a str>,
    pub(super) tool: Option<String>,
    pub(super) failed_tool: Option<String>,
    pub(super) checked: bool,
}

impl GraphActivity<'_> {
    fn phase(&self) -> &'static str {
        match self.run.map(|run| &run.state) {
            Some(RunState::Starting) => "Starting",
            Some(RunState::Running) => "Running",
            Some(RunState::AwaitingPermission) => "Needs permission",
            Some(RunState::Cancelling) => "Cancelling",
            Some(RunState::Finalizing) => "Finalizing",
            Some(RunState::Completed) => "Run completed",
            Some(RunState::Cancelled) => "Cancelled",
            Some(RunState::Failed { .. }) => "Run failed",
            Some(RunState::Interrupted { .. }) => "Interrupted",
            Some(RunState::Recovered { .. }) => "Recovered",
            None if self.checked => "Not run",
            _ => "Unknown",
        }
    }

    pub(super) fn exception(&self) -> Option<String> {
        if self
            .run
            .is_some_and(|run| matches!(run.snapshot, SnapshotState::Failed { .. }))
        {
            return Some("Revision recording failed".into());
        }
        self.failed_tool
            .as_ref()
            .map(|tool| format!("Tool failed · {tool}"))
    }
}

fn positions(graph: &WorkflowGraph, column_step: f32, row_step: f32) -> BTreeMap<i64, (f32, f32)> {
    let mut incoming: BTreeMap<_, _> = graph.nodes.iter().map(|node| (node.id, 0)).collect();
    let mut outgoing = BTreeMap::<_, Vec<_>>::new();
    for edge in &graph.connections {
        if let Some(count) = incoming.get_mut(&edge.target_id) {
            *count += 1;
            outgoing
                .entry(edge.source_id)
                .or_default()
                .push(edge.target_id);
        }
    }
    let mut ready: VecDeque<_> = incoming
        .iter()
        .filter_map(|(&id, &count)| (count == 0).then_some(id))
        .collect();
    let mut depths = BTreeMap::<_, usize>::new();
    while let Some(source) = ready.pop_front() {
        let next_depth = depths.get(&source).copied().unwrap_or_default() + 1;
        for target in outgoing.get(&source).into_iter().flatten() {
            let depth = depths.entry(*target).or_default();
            *depth = (*depth).max(next_depth);
            if let Some(count) = incoming.get_mut(target) {
                *count -= 1;
                if *count == 0 {
                    ready.push_back(*target);
                }
            }
        }
    }
    let mut counts = BTreeMap::<_, usize>::new();
    for node in &graph.nodes {
        *counts
            .entry(depths.get(&node.id).copied().unwrap_or_default())
            .or_default() += 1;
    }
    let max_rows = counts.values().copied().max().unwrap_or_default();
    let mut rows = BTreeMap::<_, usize>::new();
    graph
        .nodes
        .iter()
        .map(|node| {
            let depth = depths.get(&node.id).copied().unwrap_or_default();
            let row = rows.entry(depth).or_default();
            let position = (
                PADDING + depth as f32 * column_step,
                PADDING + (*row as f32 + (max_rows - counts[&depth]) as f32 * 0.5) * row_step,
            );
            *row += 1;
            (node.id, position)
        })
        .collect()
}

pub(super) fn node_title(id: i64, prompt: &str) -> String {
    prompt
        .lines()
        .find(|line| !line.trim().is_empty())
        .map_or_else(|| format!("Node #{id}"), |line| line.trim().to_owned())
}

pub(super) fn assignment_summary(prompt: &str) -> Option<&str> {
    prompt
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .nth(1)
}

impl Workspace {
    pub(super) fn graph_zoom_controls(
        &self,
        key: (u64, i64),
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let zoom = self.graph_cameras[&key].zoom;
        div()
            .flex()
            .items_center()
            .gap_1()
            .flex_none()
            .child(
                Button::new("graph-zoom-out")
                    .ghost()
                    .small()
                    .icon(IconName::Minus)
                    .accessibility_label("Zoom out")
                    .tooltip("Zoom out")
                    .disabled(zoom == 0)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_graph_zoom(key, zoom.saturating_sub(1), cx)
                    })),
            )
            .child(
                Button::new("graph-zoom-reset")
                    .ghost()
                    .small()
                    .label(format!("Reset zoom · {}%", ZOOM_LEVELS[zoom]))
                    .accessibility_label(format!(
                        "Graph zoom {}%. Reset to 100%",
                        ZOOM_LEVELS[zoom]
                    ))
                    .tooltip("Reset graph zoom to 100%.")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_graph_zoom(key, DEFAULT_ZOOM, cx)
                    })),
            )
            .child(
                Button::new("graph-zoom-in")
                    .ghost()
                    .small()
                    .icon(IconName::Plus)
                    .accessibility_label("Zoom in")
                    .tooltip("Zoom in")
                    .disabled(zoom == ZOOM_LEVELS.len() - 1)
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.set_graph_zoom(key, zoom + 1, cx)),
                    ),
            )
    }

    pub(super) fn set_graph_zoom(&mut self, key: (u64, i64), zoom: usize, cx: &mut Context<Self>) {
        let Some(camera) = self.graph_cameras.get_mut(&key) else {
            return;
        };
        let ratio = f32::from(ZOOM_LEVELS[zoom]) / f32::from(ZOOM_LEVELS[camera.zoom]);
        camera.scroll.set_offset(camera.scroll.offset() * ratio);
        camera.zoom = zoom;
        self.graph_preview = None;
        self.graph_preview_keyboard = false;
        self.graph_preview_task = None;
        cx.notify();
    }

    fn schedule_graph_preview(
        &mut self,
        node: i64,
        open: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        if self.graph_preview_keyboard {
            return;
        }
        if open && self.graph_preview == Some(node) {
            self.graph_preview_task = None;
            return;
        }
        self.graph_preview_task = Some(cx.spawn_in(window, async move |workspace, cx| {
            let delay = if open {
                PREVIEW_OPEN_DELAY
            } else {
                PREVIEW_CLOSE_DELAY
            };
            cx.background_executor().timer(delay).await;
            let _ = cx.update(|window, cx| {
                let _ = workspace.update(cx, |this, cx| {
                    if open && this.prepare_node(node, window, cx).is_some() {
                        this.graph_preview = Some(node);
                    } else if this.graph_preview == Some(node) {
                        this.graph_preview = None;
                    }
                    cx.notify();
                });
            });
        }));
    }

    fn dismiss_graph_preview(&mut self, node: i64, window: &mut Window, cx: &mut Context<Self>) {
        if self.graph_preview == Some(node) {
            self.graph_preview = None;
            self.graph_preview_task = None;
            if self.graph_preview_keyboard
                && let Some((project, workflow)) = self.state.selected
                && let Some(handle) = self.graph_node_focus.get(&(project, workflow, node))
            {
                handle.focus(window, cx);
            }
            self.graph_preview_keyboard = false;
            cx.notify();
        }
    }

    pub(super) fn render_graph(
        &self,
        graph: &WorkflowGraph,
        key: (u64, i64),
        selected: Option<i64>,
        window: &Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let camera = &self.graph_cameras[&key];
        let zoom = ZOOM_LEVELS[camera.zoom];
        let full = zoom >= 100;
        let scale = f32::from(zoom) / 100.;
        let card_height = if full { 4.25 } else { 5. };
        let card_width = if full {
            CARD_WIDTH
        } else {
            CARD_WIDTH.max(MIN_COMPACT_CARD_WIDTH / scale)
        };
        let positions = positions(graph, card_width + COLUMN_GAP, card_height + 0.5);
        let preview_max_height = window.viewport_size().height * 0.68;
        let edges: Vec<_> = graph
            .connections
            .iter()
            .filter_map(|edge| {
                Some((
                    *positions.get(&edge.source_id)?,
                    *positions.get(&edge.target_id)?,
                ))
            })
            .collect();
        let width = positions
            .values()
            .map(|(x, _)| x + card_width + PADDING)
            .fold(0., f32::max);
        let height = positions
            .values()
            .map(|(_, y)| y + card_height + PADDING)
            .fold(0., f32::max);
        let foreground = cx.theme().muted_foreground;
        let grid = cx.theme().border;
        let length = |value| rems(value * scale);

        div()
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(
                canvas(
                    |_, _, _: &mut App| (),
                    move |bounds, (), window, _| paint_grid(bounds, grid, window),
                )
                .absolute()
                .size_full(),
            )
            .child(
                div()
                    .id("workflow-graph")
                    .test_support()
                    .size_full()
                    .overflow_scroll()
                    .track_scroll(&camera.scroll)
                    .vertical_scrollbar(&camera.scroll)
                    .horizontal_scrollbar(&camera.scroll)
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .justify_center()
                            .flex_none()
                            .min_w_full()
                            .w(length(width))
                            .when(!full, |world| world.min_h_full().items_center())
                            .child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .w(length(width))
                                    .h(length(height))
                                    .child(
                                        canvas(
                                            |_, _, _: &mut App| (),
                                            move |bounds, (), window, _| {
                                                let unit = window.rem_size();
                                                let resolve = |value| rems(value * scale) * unit;
                                                for ((sx, sy), (tx, ty)) in &edges {
                                                    let start = bounds.origin
                                                        + point(
                                                            resolve(sx + card_width),
                                                            resolve(sy + card_height / 2.),
                                                        );
                                                    let end = bounds.origin
                                                        + point(
                                                            resolve(*tx),
                                                            resolve(ty + card_height / 2.),
                                                        );
                                                    let middle = start.x + (end.x - start.x) * 0.5;
                                                    let mut path = PathBuilder::stroke(px(1.));
                                                    path.move_to(start);
                                                    path.cubic_bezier_to(
                                                        end,
                                                        point(middle, start.y),
                                                        point(middle, end.y),
                                                    );
                                                    path.move_to(
                                                        end + point(resolve(-0.35), resolve(-0.2)),
                                                    );
                                                    path.line_to(end);
                                                    path.line_to(
                                                        end + point(resolve(-0.35), resolve(0.2)),
                                                    );
                                                    if let Ok(path) = path.build() {
                                                        window.paint_path(path, foreground);
                                                    }
                                                }
                                            },
                                        )
                                        .absolute()
                                        .size_full(),
                                    )
                                    .children(graph.nodes.iter().map(|node| {
                                        let (x, y) = positions[&node.id];
                                        self.render_graph_node(
                                            graph,
                                            node,
                                            key,
                                            GraphNodeStyle {
                                                selected: selected == Some(node.id),
                                                show_detail: full,
                                                preview_max_height,
                                            },
                                            cx,
                                        )
                                        .absolute()
                                        .left(length(x))
                                        .top(length(y))
                                        .w(length(card_width))
                                        .h(length(card_height))
                                    })),
                            ),
                    )
                    // A bottom gutter lets users scroll any row above the floating
                    // workbench while preserving the same camera across open/close.
                    .child(div().h_full().flex_none()),
            )
    }

    fn render_graph_node(
        &self,
        graph: &WorkflowGraph,
        node: &Node,
        key: (u64, i64),
        style: GraphNodeStyle,
        cx: &Context<Self>,
    ) -> gpui_kit::Stateful<gpui_kit::Div> {
        let id = node.id;
        let foreground = cx.theme().muted_foreground;
        let title = node_title(id, &node.prompt);
        let activity = self.graph_activity((key.0, key.1, id));
        let phase = activity.phase();
        let parents: Vec<_> = graph
            .connections
            .iter()
            .filter(|edge| edge.target_id == id)
            .map(|edge| format!("#{}", edge.source_id))
            .collect();
        let label = if parents.is_empty() {
            format!("{}, {phase}", title.trim_end_matches('.'))
        } else {
            format!(
                "{}, {phase}. Depends on {}",
                title.trim_end_matches('.'),
                parents.join(", ")
            )
        };
        let (icon, color) = activity.run.map_or((IconName::Circle, foreground), |run| {
            run_state_visual(&run.state, cx.theme())
        });
        let exception = activity.exception();
        let (recent, recent_color) = if style.show_detail {
            if let Some(tool) = activity.permission.as_ref() {
                (Some(format!("Permission · {tool}")), cx.theme().warning)
            } else if exception.is_some() {
                (exception, cx.theme().danger)
            } else {
                (
                    activity
                        .tool
                        .clone()
                        .or_else(|| activity.assistant.map(|text| first_line(text).to_owned())),
                    foreground,
                )
            }
        } else {
            let exception = if activity
                .run
                .is_some_and(|run| matches!(run.snapshot, SnapshotState::Failed { .. }))
            {
                Some("Revision save failed")
            } else {
                activity.failed_tool.as_ref().map(|_| "Tool failed")
            };
            (exception.map(str::to_owned), cx.theme().danger)
        };
        let detail = style
            .show_detail
            .then(|| assignment_summary(&node.prompt))
            .flatten();
        let content = div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .items_start()
            .when(!style.show_detail, |card| card.line_height(rems(0.875)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .w_full()
                    .child(Icon::new(icon).size_3().text_color(color).flex_none())
                    .child(
                        div()
                            .min_w_0()
                            .when(style.show_detail, |title| title.text_sm())
                            .when(!style.show_detail, |title| title.text_xs())
                            .font_semibold()
                            .text_ellipsis()
                            .child(title.clone()),
                    ),
            )
            .child(
                // Keep the semantic hue in the dot and tint while the label
                // stays on the normal foreground for readable contrast.
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .when(style.show_detail, |badge| badge.ml_4())
                    .rounded_full()
                    .px_1p5()
                    .py_0p5()
                    .text_xs()
                    .text_color(cx.theme().foreground)
                    .bg(color.opacity(0.12))
                    .child(div().w_1p5().h_1p5().rounded_full().flex_none().bg(color))
                    .child(phase),
            )
            .when_some(detail, |card, text| {
                card.child(
                    div()
                        .w_full()
                        .text_xs()
                        .text_color(foreground)
                        .text_ellipsis()
                        .child(text.to_owned()),
                )
            })
            .when_some(recent, |card, text| {
                if recent_color == foreground {
                    card.child(
                        div()
                            .w_full()
                            .text_xs()
                            .text_color(foreground)
                            .text_ellipsis()
                            .child(text),
                    )
                } else {
                    card.child(div().w_full().text_xs().child(semantic_status(
                        recent_color,
                        div().min_w_0().text_ellipsis().child(text),
                        cx,
                    )))
                }
            });

        let button = Button::new(SharedString::from(format!("graph-node-{id}")))
            .ghost()
            .track_focus(&self.graph_node_focus[&(key.0, key.1, id)])
            .selected(style.selected)
            .size_full()
            .accessibility_label(label)
            .child(content)
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.schedule_graph_preview(id, *hovered, window, cx);
            }))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.open_node(id, window, cx);
            }));
        div()
            .id(("graph-node-keys", id as usize))
            .key_context("WaypointsGraphNode")
            .rounded_md()
            .bg(cx.theme().popover.blend(cx.theme().muted.opacity(0.32)))
            .border_1()
            .border_color(if style.selected {
                // Selection emphasis uses the primary role; info is reserved
                // for informational status.
                cx.theme().primary
            } else {
                cx.theme().background.opacity(0.)
            })
            .on_action(cx.listener(move |this, _: &InspectGraphNode, window, cx| {
                if this.prepare_node(id, window, cx).is_none() {
                    return;
                }
                this.graph_preview_task = None;
                this.graph_preview = Some(id);
                this.graph_preview_keyboard = true;
                this.graph_preview_focus.focus(window, cx);
                cx.notify();
            }))
            .child(button)
            .child(
                // Anchor from the card's top-right corner so a tall preview
                // opens beside it without extending into the workflow header.
                Popup::new(("node-anchor", id as usize), div())
                    .anchor(Anchor::TopLeft)
                    .absolute()
                    .left_full()
                    .top_0()
                    .size_1()
                    .when(self.graph_preview == Some(id), |popup| {
                        popup.content(self.render_graph_preview(
                            id,
                            title,
                            &node.prompt,
                            &activity,
                            style.preview_max_height,
                            cx,
                        ))
                    }),
            )
    }

    fn render_graph_preview(
        &self,
        id: i64,
        title: String,
        assignment: &str,
        activity: &GraphActivity<'_>,
        max_height: Pixels,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let (snapshot, snapshot_color) = match activity.run.map(|run| &run.snapshot) {
            Some(SnapshotState::Saved { revision }) => (
                Some(format!(
                    "Recorded revision · {}",
                    super::node_workspace::short_id(&revision.commit_id)
                )),
                muted,
            ),
            Some(SnapshotState::Failed { detail }) => (
                Some(format!("Revision recording failed · {detail}")),
                cx.theme().danger,
            ),
            Some(SnapshotState::NotRecorded) => {
                (Some("No revision recorded for this run".into()), muted)
            }
            _ => (None, muted),
        };
        let assignment = assignment.trim();
        let body = assignment
            .split_once('\n')
            .map(|(_, rest)| rest.trim())
            .filter(|rest| !rest.is_empty())
            .unwrap_or(assignment);
        div()
            .id("graph-preview")
            .test_support()
            .key_context("WaypointsGraphPreview")
            .occlude()
            .w_64()
            .max_h(max_height)
            .flex()
            .flex_col()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().popover)
            .shadow_lg()
            .on_action(
                cx.listener(move |this, _: &DismissGraphPreview, window, cx| {
                    this.dismiss_graph_preview(id, window, cx);
                }),
            )
            .on_hover(cx.listener(move |this, hovered, window, cx| {
                this.schedule_graph_preview(id, *hovered, window, cx);
            }))
            .on_mouse_down_out(cx.listener(move |this, _, window, cx| {
                this.dismiss_graph_preview(id, window, cx);
            }))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .font_semibold()
                            .line_clamp(2)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted)
                            .rounded_full()
                            .px_2()
                            .bg(cx.theme().muted.opacity(0.6))
                            .child(activity.phase()),
                    )
                    .child(
                        Button::new(("graph-preview-close", id as usize))
                            .ghost()
                            .xsmall()
                            .icon(IconName::X)
                            .accessibility_label("Close preview")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.dismiss_graph_preview(id, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .id("graph-preview-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        div()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt_2()
                            .child(div().text_sm().font_semibold().child("Assignment"))
                            .child(div().text_xs().line_clamp(4).child(if body.is_empty() {
                                "No assignment text".into()
                            } else {
                                body.to_owned()
                            })),
                    )
                    .child(
                        div()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt_2()
                            .child(div().text_sm().font_semibold().child("Recent output"))
                            .child(
                                div().text_xs().text_color(muted).line_clamp(4).child(
                                    activity
                                        .assistant
                                        .unwrap_or("No assistant output available")
                                        .trim()
                                        .to_owned(),
                                ),
                            )
                            .when_some(activity.tool.as_ref(), |section, title| {
                                section.child(
                                    div()
                                        .text_xs()
                                        .line_clamp(2)
                                        .child(format!("Tool · {title}")),
                                )
                            })
                            .when_some(activity.failed_tool.as_ref(), |section, title| {
                                section.child(semantic_status(
                                    cx.theme().danger,
                                    div()
                                        .text_xs()
                                        .line_clamp(2)
                                        .child(format!("Tool failed · {title}")),
                                    cx,
                                ))
                            })
                            .when_some(activity.permission.as_ref(), |section, title| {
                                section.child(semantic_status(
                                    cx.theme().warning,
                                    div()
                                        .text_xs()
                                        .line_clamp(2)
                                        .child(format!("Permission requested · {title}")),
                                    cx,
                                ))
                            }),
                    )
                    .when_some(snapshot, |preview, text| {
                        preview.child(
                            div()
                                .text_xs()
                                .text_color(snapshot_color)
                                .border_t_1()
                                .border_color(cx.theme().border)
                                .pt_2()
                                .line_clamp(2)
                                .child(text),
                        )
                    }),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Shift+Enter opens this preview."),
            )
            .child(
                div().flex().justify_end().child(
                    Button::new("open-preview-workbench")
                        .outline()
                        .small()
                        .label("Open in workbench")
                        .track_focus(&self.graph_preview_focus)
                        .on_click(
                            cx.listener(move |this, _, window, cx| this.open_node(id, window, cx)),
                        ),
                ),
            )
    }
}

fn first_line(text: &str) -> &str {
    text.trim().lines().next().unwrap_or_default()
}

fn paint_grid(bounds: Bounds<Pixels>, color: Hsla, window: &mut Window) {
    let step = window.rem_size();
    let dot = size(px(1.), px(1.));
    let mut y = bounds.top() + step * 0.5;
    while y < bounds.bottom() {
        let mut x = bounds.left() + step * 0.5;
        while x < bounds.right() {
            window.paint_quad(gpui_kit::fill(Bounds::new(point(x, y), dot), color));
            x += step;
        }
        y += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ROW_STEP: f32 = 6.25;
    const COLUMN_STEP: f32 = CARD_WIDTH + COLUMN_GAP;

    fn graph(nodes: &[i64], edges: &[(i64, i64)]) -> WorkflowGraph {
        serde_json::from_value(json!({
            "workflow": {
                "id": 1,
                "project_path": "/tmp/project",
                "name": "Workflow",
            },
            "nodes": nodes
                .iter()
                .map(|id| json!({"id": id, "workflow_id": 1, "prompt": format!("Node {id}")}))
                .collect::<Vec<_>>(),
            "connections": edges
                .iter()
                .map(|(source, target)| json!({
                    "workflow_id": 1,
                    "source_id": source,
                    "target_id": target,
                }))
                .collect::<Vec<_>>(),
        }))
        .expect("graph fixture matches the storage schema")
    }

    #[test]
    fn positions_follow_the_longest_dependency_path() {
        let positions = positions(&graph(&[1, 2, 3], &[(1, 2), (2, 3)]), COLUMN_STEP, ROW_STEP);

        assert_eq!(positions[&1], (PADDING, PADDING));
        assert_eq!(positions[&2], (PADDING + COLUMN_STEP, PADDING));
        assert_eq!(positions[&3], (PADDING + 2. * COLUMN_STEP, PADDING));
    }

    #[test]
    fn positions_pack_siblings_into_rows_by_node_order() {
        let positions = positions(
            &graph(&[1, 2, 3, 4], &[(1, 2), (1, 3), (2, 4), (3, 4)]),
            COLUMN_STEP,
            ROW_STEP,
        );

        assert_eq!(positions[&2], (PADDING + COLUMN_STEP, PADDING));
        assert_eq!(positions[&3], (PADDING + COLUMN_STEP, PADDING + ROW_STEP));
        assert_eq!(
            positions[&4],
            (PADDING + 2. * COLUMN_STEP, PADDING + ROW_STEP / 2.)
        );
    }

    #[test]
    fn positions_keep_disconnected_and_cyclic_nodes_on_the_first_column() {
        let disconnected = positions(&graph(&[1, 2, 3], &[]), COLUMN_STEP, ROW_STEP);
        assert_eq!(disconnected[&1], (PADDING, PADDING));
        assert_eq!(disconnected[&2], (PADDING, PADDING + ROW_STEP));
        assert_eq!(disconnected[&3], (PADDING, PADDING + 2. * ROW_STEP));

        let cyclic = positions(&graph(&[1, 2], &[(1, 2), (2, 1)]), COLUMN_STEP, ROW_STEP);
        assert_eq!(cyclic[&1], (PADDING, PADDING));
        assert_eq!(cyclic[&2], (PADDING, PADDING + ROW_STEP));
    }

    #[test]
    fn positions_ignore_edges_to_unknown_nodes() {
        let positions = positions(&graph(&[1, 2], &[(1, 99), (99, 2)]), COLUMN_STEP, ROW_STEP);

        assert_eq!(positions[&1], (PADDING, PADDING));
        assert_eq!(positions[&2], (PADDING, PADDING + ROW_STEP));
    }
}
