mod workspace;

use gpui_kit::{AppContext as _, WindowOptions};

fn main() {
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| workspace::Workspace::new(window, cx))
            })
            .expect("opening the Waypoints window");
        });
}
