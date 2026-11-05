//! Headless screenshots of the real workspace views using deterministic fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, px, size,
};

use crate::workspace::{self, ScreenshotSurface, Workspace};

pub fn run() {
    if let Err(error) = capture_all() {
        eprintln!("screenshot capture failed: {error:#}");
        std::process::exit(1);
    }
}

fn capture_all() -> anyhow::Result<()> {
    let directory = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("screenshots"));
    std::fs::create_dir_all(&directory)?;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for surface in [
            ScreenshotSurface::Assignment,
            ScreenshotSurface::Output,
            ScreenshotSurface::Changes,
            ScreenshotSurface::History,
        ] {
            let path = directory.join(format!("{}-{}.png", surface_name(surface), mode_name(mode)));
            capture(mode, surface, &path)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn capture(mode: ThemeMode, surface: ScreenshotSurface, path: &Path) -> anyhow::Result<()> {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(gpui_kit::assets::AllAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| {
        gpui_kit::init(cx);
        Theme::change(mode, None, cx);
        workspace::init(cx);
    });
    let (handle, _view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(1200.), px(860.)),
                ))),
                focus: false,
                show: false,
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut workspace = Workspace::new(window, cx);
                    workspace::fixtures::install(&mut workspace, surface, window, cx);
                    workspace
                })
            },
        )
    })?;
    cx.update_window(handle, |_, window, cx| window.render_frame(cx))?;
    let image = cx.capture_screenshot(handle)?;
    image.save(path)?;
    Ok(())
}

fn surface_name(surface: ScreenshotSurface) -> &'static str {
    match surface {
        ScreenshotSurface::Assignment => "assignment-preview",
        ScreenshotSurface::Output => "output",
        ScreenshotSurface::Changes => "changes",
        ScreenshotSurface::History => "history",
    }
}

fn mode_name(mode: ThemeMode) -> &'static str {
    match mode {
        ThemeMode::Light => "light",
        ThemeMode::Dark => "dark",
    }
}
