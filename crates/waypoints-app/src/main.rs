mod persistence;
mod preferences;
mod settings;
mod workspace;

use gpui_kit::component::{TitleBar, WindowExt as _};
use gpui_kit::{
    App, AppContext as _, Bounds, KeyBinding, Menu, MenuItem, Pixels, WindowBounds, WindowOptions,
    actions, point, px, size,
};

use preferences::{WindowGeometry, WorkspacePreferences};

actions!(waypoints, [Quit, OpenSettings]);

const MIN_WINDOW_WIDTH: f32 = 720.0;
const MIN_WINDOW_HEIGHT: f32 = 480.0;

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            workspace::init(cx);
            let quit_key = if cfg!(target_os = "macos") {
                "cmd-q"
            } else {
                "ctrl-q"
            };
            let settings_key = if cfg!(target_os = "macos") {
                "cmd-,"
            } else {
                "ctrl-,"
            };
            cx.bind_keys([
                KeyBinding::new(quit_key, Quit, None),
                KeyBinding::new(settings_key, OpenSettings, None),
            ]);
            cx.set_menus([
                Menu::new("Waypoints").items([
                    MenuItem::action("Settings…", OpenSettings),
                    MenuItem::action("Quit Waypoints", Quit),
                ]),
                Menu::new("File").items([MenuItem::action("Add Project…", workspace::AddProject)]),
                Menu::new("View").items([
                    MenuItem::action("Show/Hide Projects", workspace::ToggleSidebar),
                    MenuItem::action("Refresh Selected Project", workspace::RefreshProject),
                ]),
            ]);
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let executor = cx.background_executor().clone();
            cx.spawn(async move |cx| {
                let loaded = executor
                    .spawn(async {
                        preferences::load_preferences().map(workspace::normalize_preferences)
                    })
                    .await;
                let (preferences, error) = match loaded {
                    Ok(preferences) => (preferences, None),
                    Err(error) => (WorkspacePreferences::default(), Some(format!("{error:#}"))),
                };
                cx.update(|cx| {
                    let options = window_options(preferences.window_bounds, cx);
                    let (window, view) = gpui_kit::open_window(options, cx, |window, cx| {
                        settings::apply_appearance(preferences.appearance, window, cx);
                        cx.new(|cx| {
                            let mut workspace = workspace::Workspace::new(window, cx);
                            workspace.restore_preferences(preferences, cx);
                            if let Some(error) = error {
                                workspace.show_preferences_error(error, cx);
                            }
                            workspace
                        })
                    })
                    .expect("opening the Waypoints window");
                    settings::register(view.downgrade(), cx);
                    let (writer, mut errors) = persistence::PreferencesWriter::new(cx);
                    let quit_writer = writer.clone();
                    cx.on_action(move |_: &Quit, cx| {
                        flush_then_quit(quit_writer.clone(), cx);
                    });
                    let mut previous = window
                        .update(cx, |_, window, cx| {
                            view.read(cx).snapshot_preferences(window)
                        })
                        .expect("new workspace window exists");
                    writer.save(previous.clone());
                    let observer_writer = writer.clone();
                    cx.observe(&view, move |view, cx| {
                        if let Ok(snapshot) = window.update(cx, |_, window, cx| {
                            view.read(cx).snapshot_preferences(window)
                        }) && previous != snapshot
                        {
                            previous = snapshot.clone();
                            observer_writer.save(snapshot);
                        }
                    })
                    .detach();
                    window
                        .update(cx, |_, window, cx| {
                            view.update(cx, |_, cx| {
                                cx.observe_window_bounds(window, |_, _, cx| cx.notify())
                                    .detach();
                            });
                            let appearance_view = view.downgrade();
                            window
                                .observe_window_appearance(move |window, cx| {
                                    if appearance_view.upgrade().is_some_and(|view| {
                                        view.read(cx).appearance == preferences::Appearance::System
                                    }) {
                                        settings::apply_appearance(
                                            preferences::Appearance::System,
                                            window,
                                            cx,
                                        );
                                    }
                                })
                                .detach();
                            let close_writer = writer.clone();
                            window.on_window_should_close(cx, move |_, cx| {
                                flush_then_quit(close_writer.clone(), cx);
                                false
                            });
                        })
                        .expect("new workspace window exists");
                    let weak = view.downgrade();
                    cx.spawn(async move |cx| {
                        while let Some(error) = errors.recv().await {
                            if weak
                                .update(cx, |view, cx| view.show_preferences_error(error, cx))
                                .is_err()
                            {
                                break;
                            }
                        }
                    })
                    .detach();
                    cx.on_app_quit(move |_| {
                        let writer = writer.clone();
                        async move {
                            let _ = writer.flush().await;
                        }
                    })
                    .detach();
                    cx.activate(true);
                });
            })
            .detach();
        });
}

fn flush_then_quit(writer: persistence::PreferencesWriter, cx: &mut App) {
    // GPUI limits quit observers to a short grace period. Normal user-initiated
    // quits must finish the write queue before starting that shutdown period.
    let window = cx
        .active_window()
        .or_else(|| cx.windows().into_iter().next());
    cx.spawn(async move |cx| {
        let result = writer.flush().await;
        cx.update(|cx| match result {
            Ok(()) => cx.quit(),
            Err(error) => {
                if let Some(handle) = window {
                    let _ = handle.update(cx, |_, window, cx| {
                        if !window.has_active_dialog(cx) {
                            window.open_alert_dialog(cx, move |dialog, _, _| {
                                dialog
                                    .title("Quit without saving workspace?")
                                    .description(format!("{error}\nProject files and saved workflows are unaffected. Keep Waypoints open to fix the problem, then quit again to retry."))
                                    .confirm()
                                    .ok_text("Quit Without Saving")
                                    .cancel_text("Keep Open")
                                    .on_ok(|_, _, cx| { cx.quit(); true })
                            });
                        }
                    });
                }
            }
        });
    })
    .detach();
}

fn window_options(geometry: Option<WindowGeometry>, cx: &App) -> WindowOptions {
    let displays: Vec<_> = cx
        .displays()
        .iter()
        .map(|display| display.bounds())
        .collect();
    // A disconnected display must not leave the restored window unreachable.
    let bounds = geometry
        .and_then(|geometry| restored_window_bounds(geometry, &displays))
        .map(WindowBounds::Windowed)
        .unwrap_or_else(|| WindowBounds::centered(size(px(1100.0), px(760.0)), cx));
    WindowOptions {
        window_bounds: Some(bounds),
        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
        ..if cfg!(target_os = "macos") {
            TitleBar::window_options()
        } else {
            WindowOptions::default()
        }
    }
}

fn restored_window_bounds(
    geometry: WindowGeometry,
    displays: &[Bounds<Pixels>],
) -> Option<Bounds<Pixels>> {
    let desired = Bounds::new(
        point(px(geometry.x), px(geometry.y)),
        size(
            px(geometry.width.max(MIN_WINDOW_WIDTH)),
            px(geometry.height.max(MIN_WINDOW_HEIGHT)),
        ),
    );
    let display = displays
        .iter()
        .find(|display| display.intersects(&desired))?;
    let width = geometry
        .width
        .max(MIN_WINDOW_WIDTH)
        .min(f32::from(display.size.width).max(MIN_WINDOW_WIDTH));
    let height = geometry
        .height
        .max(MIN_WINDOW_HEIGHT)
        .min(f32::from(display.size.height).max(MIN_WINDOW_HEIGHT));
    let left = f32::from(display.origin.x);
    let top = f32::from(display.origin.y);
    Some(Bounds::new(
        point(
            px(geometry
                .x
                .clamp(left, (f32::from(display.right()) - width).max(left))),
            px(geometry
                .y
                .clamp(top, (f32::from(display.bottom()) - height).max(top))),
        ),
        size(px(width), px(height)),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_restoration_keeps_chrome_on_a_connected_display() {
        let display = Bounds::new(point(px(0.0), px(0.0)), size(px(1440.0), px(900.0)));
        let geometry = WindowGeometry {
            x: 1400.0,
            y: -700.0,
            width: 1100.0,
            height: 800.0,
        };
        let bounds =
            restored_window_bounds(geometry, &[display]).expect("partially visible window");
        assert_eq!(bounds.origin, point(px(340.0), px(0.0)));
        assert_eq!(bounds.size, size(px(1100.0), px(800.0)));
        assert!(
            restored_window_bounds(
                WindowGeometry {
                    x: 3000.0,
                    ..geometry
                },
                &[display]
            )
            .is_none()
        );
    }
}
