use std::collections::{BTreeMap, VecDeque};

use gpui_kit::component::{ActiveTheme as _, Selectable as _, button::Button};
use gpui_kit::{
    App, Context, InteractiveElement as _, ParentElement as _, PathBuilder, SharedString,
    StatefulInteractiveElement as _, Styled as _, canvas, div, point, px, rems,
};
use waypoints_core::storage::WorkflowGraph;

use super::Workspace;

// Graph coordinates use the same rem scale as the labels inside the cards.
const CARD_WIDTH: f32 = 11.0;
const CARD_HEIGHT: f32 = 3.125;
const COLUMN_STEP: f32 = 14.5;
const ROW_STEP: f32 = 4.125;
const PADDING: f32 = 1.0;

fn positions(graph: &WorkflowGraph) -> BTreeMap<i64, (f32, f32)> {
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
    let mut rows = BTreeMap::<_, usize>::new();
    graph
        .nodes
        .iter()
        .map(|node| {
            let depth = depths.get(&node.id).copied().unwrap_or_default();
            let row = rows.entry(depth).or_default();
            let position = (
                PADDING + depth as f32 * COLUMN_STEP,
                PADDING + *row as f32 * ROW_STEP,
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

pub(super) fn render_graph(
    graph: &WorkflowGraph,
    selected: Option<i64>,
    run_status: impl Fn(i64) -> Option<&'static str>,
    cx: &Context<Workspace>,
) -> impl gpui_kit::IntoElement {
    let positions = positions(graph);
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
        .map(|(x, _)| x + CARD_WIDTH + PADDING)
        .fold(0., f32::max);
    let height = positions
        .values()
        .map(|(_, y)| y + CARD_HEIGHT + PADDING)
        .fold(0., f32::max);
    let color = cx.theme().muted_foreground;
    let selected_border = cx.theme().ring;
    div()
        .id("workflow-graph")
        .size_full()
        .overflow_scroll()
        .child(
            div()
                .relative()
                .w(rems(width))
                .h(rems(height))
                .child(
                    canvas(
                        |_, _, _: &mut App| (),
                        move |bounds, (), window, _| {
                            let unit = window.rem_size();
                            let resolve = |value| rems(value) * unit;
                            for ((sx, sy), (tx, ty)) in &edges {
                                let start = bounds.origin
                                    + point(
                                        resolve(sx + CARD_WIDTH),
                                        resolve(sy + CARD_HEIGHT / 2.),
                                    );
                                let end = bounds.origin
                                    + point(resolve(*tx), resolve(ty + CARD_HEIGHT / 2.));
                                let turn = end.x - resolve((COLUMN_STEP - CARD_WIDTH) / 2.);
                                let mut path = PathBuilder::stroke(px(1.25));
                                path.move_to(start);
                                path.line_to(point(turn, start.y));
                                path.line_to(point(turn, end.y));
                                path.line_to(end);
                                path.move_to(end + point(resolve(-0.375), resolve(-0.25)));
                                path.line_to(end);
                                path.line_to(end + point(resolve(-0.375), resolve(0.25)));
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, color);
                                }
                            }
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .children(graph.nodes.iter().map(|node| {
                    let id = node.id;
                    let (x, y) = positions[&id];
                    let title = node_title(id, &node.prompt);
                    let parents: Vec<_> = graph
                        .connections
                        .iter()
                        .filter(|edge| edge.target_id == id)
                        .map(|edge| format!("#{}", edge.source_id))
                        .collect();
                    let label = if parents.is_empty() {
                        title.clone()
                    } else {
                        format!("{title}. Depends on {}", parents.join(", "))
                    };
                    let mut content = div()
                        .w_full()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .items_start()
                        .child(div().w_full().text_ellipsis().child(title.clone()));
                    let label = if let Some(status) = run_status(id) {
                        content = content.child(div().text_xs().text_color(color).child(status));
                        format!("{label}. {status}")
                    } else {
                        label
                    };
                    div()
                        .absolute()
                        .left(rems(x))
                        .top(rems(y))
                        .w(rems(CARD_WIDTH))
                        .h(rems(CARD_HEIGHT))
                        .rounded_md()
                        .border_1()
                        .border_color(if selected == Some(id) {
                            selected_border
                        } else {
                            cx.theme().background
                        })
                        .child(
                            Button::new(SharedString::from(format!("graph-node-{id}")))
                                .outline()
                                .selected(selected == Some(id))
                                .size_full()
                                .accessibility_label(label)
                                .tooltip(title.clone())
                                .child(content)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_node(id, window, cx);
                                })),
                        )
                })),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
        let positions = positions(&graph(&[1, 2, 3], &[(1, 2), (2, 3)]));

        assert_eq!(positions[&1], (PADDING, PADDING));
        assert_eq!(positions[&2], (PADDING + COLUMN_STEP, PADDING));
        assert_eq!(positions[&3], (PADDING + 2. * COLUMN_STEP, PADDING));
    }

    #[test]
    fn positions_pack_siblings_into_rows_by_node_order() {
        let positions = positions(&graph(&[1, 2, 3, 4], &[(1, 2), (1, 3), (2, 4), (3, 4)]));

        assert_eq!(positions[&2], (PADDING + COLUMN_STEP, PADDING));
        assert_eq!(positions[&3], (PADDING + COLUMN_STEP, PADDING + ROW_STEP));
        assert_eq!(positions[&4], (PADDING + 2. * COLUMN_STEP, PADDING));
    }

    #[test]
    fn positions_keep_disconnected_and_cyclic_nodes_on_the_first_column() {
        let disconnected = positions(&graph(&[1, 2, 3], &[]));
        assert_eq!(disconnected[&1], (PADDING, PADDING));
        assert_eq!(disconnected[&2], (PADDING, PADDING + ROW_STEP));
        assert_eq!(disconnected[&3], (PADDING, PADDING + 2. * ROW_STEP));

        let cyclic = positions(&graph(&[1, 2], &[(1, 2), (2, 1)]));
        assert_eq!(cyclic[&1], (PADDING, PADDING));
        assert_eq!(cyclic[&2], (PADDING, PADDING + ROW_STEP));
    }

    #[test]
    fn positions_ignore_edges_to_unknown_nodes() {
        let positions = positions(&graph(&[1, 2], &[(1, 99), (99, 2)]));

        assert_eq!(positions[&1], (PADDING, PADDING));
        assert_eq!(positions[&2], (PADDING, PADDING + ROW_STEP));
    }
}
