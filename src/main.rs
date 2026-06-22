mod app;
mod app_state;
mod feature_canvas;
mod model;
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
