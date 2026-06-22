mod app;
mod app_state;
mod feature_canvas;
mod model;
// PDF drag-out exists only on macOS and Windows (see pdf_export / app).
#[cfg(any(target_os = "macos", target_os = "windows"))]
mod pdf_export;
#[cfg(target_os = "macos")]
mod platform_menu;
mod svg_view;
mod tag_folder;
mod theme;
mod tree_canvas;
mod workers;

fn main() -> iced::Result {
    app::run()
}
