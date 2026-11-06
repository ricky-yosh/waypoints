//! Screenshots and native inspection of the workspace using deterministic fixtures.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{
    AppContext as _, Bounds, HeadlessAppContext, WindowBounds, WindowOptions, px, size,
};

use crate::workspace::{self, ScreenshotSurface, Workspace};

const SURFACES: [ScreenshotSurface; 6] = [
    ScreenshotSurface::Overview,
    ScreenshotSurface::CompactOverview,
    ScreenshotSurface::Assignment,
    ScreenshotSurface::Output,
    ScreenshotSurface::Changes,
    ScreenshotSurface::History,
];

pub fn run() {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    let result = if first.as_deref() == Some("--inspect") {
        let surface = args.next();
        let mode = if args.next().as_deref() == Some("light") {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        SURFACES
            .into_iter()
            .find(|candidate| Some(surface_name(*candidate)) == surface.as_deref())
            .ok_or_else(|| anyhow::anyhow!("select a fixture surface to inspect"))
            .map(|surface| inspect_native(surface, mode))
    } else {
        capture_all(
            first
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("screenshots")),
        )
    };
    if let Err(error) = result {
        eprintln!("screenshot capture failed: {error:#}");
        std::process::exit(1);
    }
}

fn capture_all(directory: PathBuf) -> anyhow::Result<()> {
    std::fs::create_dir_all(&directory)?;
    for mode in [ThemeMode::Light, ThemeMode::Dark] {
        for surface in SURFACES {
            let path = directory.join(format!("{}-{}.png", surface_name(surface), mode_name(mode)));
            capture(mode, surface, &path)?;
            println!("wrote {}", path.display());
        }
    }
    Ok(())
}

fn inspect_native(surface: ScreenshotSurface, mode: ThemeMode) {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::AllAssets)
        .run(move |cx| {
            gpui_kit::init(cx);
            Theme::change(mode, None, cx);
            workspace::init(cx);
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            let options = crate::window_options(None, cx);
            gpui_kit::open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::centered(capture_size(surface), cx)),
                    ..options
                },
                cx,
                |window, cx| {
                    cx.new(|cx| {
                        let mut view = Workspace::new(window, cx);
                        workspace::fixtures::install(&mut view, surface, window, cx);
                        view
                    })
                },
            )
            .expect("open Waypoints fixture inspection window");
            cx.activate(true);
        });
}

fn capture_size(surface: ScreenshotSurface) -> gpui_kit::Size<gpui_kit::Pixels> {
    match surface {
        ScreenshotSurface::Overview | ScreenshotSurface::CompactOverview => {
            size(px(1440.), px(510.))
        }
        _ => size(px(1280.), px(720.)),
    }
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
                    capture_size(surface),
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
        ScreenshotSurface::Overview => "overview-preview",
        ScreenshotSurface::CompactOverview => "overview-compact",
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
