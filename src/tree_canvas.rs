use crate::{model::TreeLayout, theme};
use iced::{
    Element, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, alignment, mouse,
    widget::{canvas, scrollable, text},
};
use std::sync::Arc;

pub fn tree_view<Message: 'static>(
    layout: Arc<TreeLayout>,
    zoom: f32,
) -> Element<'static, Message> {
    let scale = zoom.max(0.35);
    // Size the canvas to the tree's natural extent (plus a little padding) so
    // the scrollable can pan a tree that's wider or taller than the viewport.
    let width = layout.width * scale + 24.0;
    let height = layout.height * scale + 36.0;
    scrollable(
        canvas(TreeScene { layout, zoom })
            .width(Length::Fixed(width))
            .height(Length::Fixed(height)),
    )
    .direction(scrollable::Direction::Both {
        vertical: scrollable::Scrollbar::default(),
        horizontal: scrollable::Scrollbar::default(),
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

struct TreeScene {
    layout: Arc<TreeLayout>,
    zoom: f32,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for TreeScene {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let scale = self.zoom.max(0.35);
        let offset_x = ((bounds.width - self.layout.width * scale) / 2.0).max(12.0);
        let offset_y = 18.0;

        for edge in &self.layout.edges {
            let path = canvas::Path::line(
                Point::new(
                    offset_x + edge.parent_x * scale,
                    offset_y + edge.parent_y * scale,
                ),
                Point::new(
                    offset_x + edge.child_x * scale,
                    offset_y + edge.child_y * scale,
                ),
            );
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(theme::MUTED)
                    .with_width(1.2),
            );
        }

        for node in &self.layout.nodes {
            let width = node.width * scale;
            let height = 30.0 * scale;
            let top_left = Point::new(
                offset_x + (node.x - node.width / 2.0) * scale,
                offset_y + node.y * scale,
            );
            // No box: just mask the label area with the panel colour so the
            // edges stop short of the text instead of running into it.
            frame.fill_rectangle(top_left, Size::new(width, height), theme::SURFACE);
            frame.fill_text(canvas::Text {
                content: node.label.clone(),
                position: Point::new(top_left.x + width / 2.0, top_left.y + height / 2.0),
                color: theme::TEXT,
                size: Pixels::from((13.0 * scale).clamp(9.0, 20.0)),
                line_height: text::LineHeight::default(),
                font: Font::with_name("Inter"),
                max_width: f32::INFINITY,
                align_x: alignment::Horizontal::Center.into(),
                align_y: alignment::Vertical::Center,
                shaping: text::Shaping::Basic,
            });
        }
        vec![frame.into_geometry()]
    }
}
