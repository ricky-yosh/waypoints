//! Isolated Rune bootstrap fixture; no backend, project access, or preference writes.

use rune::tok::{self, type_};
use rune::{AppSpec, Cx, El, col, text};
use rune_kit::{Button, Kit};

const WINDOW_SIZE: (f32, f32) = (720.0, 480.0);

#[derive(Default)]
struct State {
    presses: u32,
}

fn view(state: &State, cx: &mut Cx<State>) -> El {
    let dark = cx.system_dark();
    let theme = tok::theme(dark);
    let kit = Kit::neutral(dark);

    col()
        .key("bootstrap")
        .w_full()
        .h_full()
        .center()
        .gap(16.0)
        .padding(24.0)
        .fill(theme.bg)
        .child(
            text("Waypoints · Rune")
                .key("title")
                .font(type_::TITLE)
                .color(theme.text),
        )
        .child(
            text("Frontend bootstrap fixture — no backend connected.")
                .color(theme.text)
                .wrap(),
        )
        .child(
            Button::new("Test interaction")
                .on_press(|state: &mut State| state.presses += 1)
                .build(&kit)
                .key("test-interaction"),
        )
        .child(
            text(format!("Interactions: {}", state.presses))
                .key("interaction-count")
                .color(theme.text),
        )
}

fn main() {
    rune::run(AppSpec::new(
        "Waypoints · Rune",
        WINDOW_SIZE,
        State::default,
        view,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use rune::{Env, Runtime};

    #[test]
    fn pressing_the_kit_button_updates_the_rendered_count() {
        let mut runtime = Runtime::new(State::default(), view, WINDOW_SIZE, Env::new(true, 1.0));
        runtime.frame(0.0);
        let (x, y) = runtime.find("test-interaction").expect("fixture button");
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(1.0 / 60.0);

        assert_eq!(runtime.state.presses, 1, "one press reaches app state");
        assert!(
            runtime.elements().iter().any(|element| {
                element.key.as_deref() == Some("interaction-count")
                    && element.text.as_deref() == Some("Interactions: 1")
            }),
            "the view renders the updated state"
        );
    }
}
