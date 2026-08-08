mod input;
mod mouse_routing;
mod render;
mod screen_layout;

pub use input::{MouseAction, MouseInput, key_from_event, mouse_from_event};
pub use mouse_routing::resolve as resolve_mouse;
pub use render::{
    render, render_background_colours, render_foreground_colours, render_to_text,
    render_with_layout,
};
pub use screen_layout::{ScreenLayout, WorkspacePanes};
