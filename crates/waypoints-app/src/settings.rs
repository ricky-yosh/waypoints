use gpui_kit::base::{StyledExt as _, TestSupportExt as _};
use std::cell::Cell;

use gpui_kit::component::{
    ActiveTheme as _, Theme, ThemeMode,
    radio::{Radio, RadioGroup},
    scroll::ScrollableElement as _,
};
use gpui_kit::{
    AnyWindowHandle, App, AppContext as _, Context, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Role, StatefulInteractiveElement as _, Styled as _,
    TitlebarOptions, WeakEntity, Window, WindowBounds, WindowOptions, div, px, size,
};

use crate::{OpenSettings, preferences::Appearance, workspace::Workspace};

const APPEARANCES: [(Appearance, &str); 3] = [
    (Appearance::System, "System"),
    (Appearance::Light, "Light"),
    (Appearance::Dark, "Dark"),
];

pub(crate) fn apply_appearance(appearance: Appearance, window: &mut Window, cx: &mut App) {
    match appearance {
        Appearance::System => Theme::sync_system_appearance(Some(window), cx),
        Appearance::Light => Theme::change(ThemeMode::Light, Some(window), cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, Some(window), cx),
    }
}

pub(crate) fn register(workspace: WeakEntity<Workspace>, cx: &mut App) {
    let settings = Cell::new(None::<AnyWindowHandle>);
    cx.on_action(move |_: &OpenSettings, cx| {
        if let Some(handle) = settings.get()
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let options = WindowOptions {
            titlebar: Some(TitlebarOptions {
                title: Some("Waypoints Settings".into()),
                ..Default::default()
            }),
            window_bounds: Some(WindowBounds::centered(size(px(480.), px(320.)), cx)),
            window_min_size: Some(size(px(360.), px(240.))),
            ..Default::default()
        };
        let workspace = workspace.clone();
        match gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Settings { workspace })) {
            Ok((handle, _)) => settings.set(Some(handle)),
            Err(error) => eprintln!("Could not open Waypoints Settings: {error:#}"),
        }
    });
}

struct Settings {
    workspace: WeakEntity<Workspace>,
}

impl Render for Settings {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let appearance = self
            .workspace
            .upgrade()
            .map(|view| view.read(cx).appearance);
        div()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("settings-content")
                    .size_full()
                    .overflow_y_scrollbar()
                    .child(
                        div()
                            .p_6()
                            .flex()
                            .flex_col()
                            .gap_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .id("appearance-heading")
                                            .test_support()
                                            .role(Role::Heading)
                                            .aria_level(2)
                                            .text_lg()
                                            .font_semibold()
                                            .child("Appearance"),
                                    )
                                    .child(
                                        div()
                                            .text_sm()
                                            .text_color(cx.theme().muted_foreground)
                                            .child("Choose how Waypoints looks. System follows your device appearance."),
                                    ),
                            )
                            .child(
                                RadioGroup::new("appearance")
                                    .children(APPEARANCES.map(|(_, label)| {
                                        Radio::new(label)
                                            .label(label)
                                            .accessibility_label(format!("Appearance: {label}"))
                                    }))
                                    .selected_index(
                                        APPEARANCES
                                            .iter()
                                            .position(|(mode, _)| Some(*mode) == appearance),
                                    )
                                    .on_change(cx.listener(|this, &index, window, cx| {
                                        let (appearance, _) = APPEARANCES[index];
                                        if this
                                            .workspace
                                            .update(cx, |workspace, cx| {
                                                workspace.appearance = appearance;
                                                apply_appearance(appearance, window, cx);
                                                cx.notify();
                                            })
                                            .is_ok()
                                        {
                                            cx.notify();
                                        }
                                    })),
                            ),
                    ),
            )
    }
}
