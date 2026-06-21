use crate::{model::FeatureStructureLayout, theme};
use iced::{
    Element, Font, Length, Pixels, Point, Rectangle, Renderer, Size, Theme, alignment, mouse,
    widget::{canvas, scrollable, text},
};
use std::sync::Arc;

pub fn feature_structure_view<Message: 'static>(
    layout: Arc<FeatureStructureLayout>,
    zoom: f32,
) -> Element<'static, Message> {
    let scale = zoom.max(0.35);
    let width = layout.width * scale + 24.0;
    let height = layout.height * scale + 36.0;
    scrollable(
        canvas(FeatureStructureScene { layout, zoom })
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

struct FeatureStructureScene {
    layout: Arc<FeatureStructureLayout>,
    zoom: f32,
}

impl<Message> canvas::Program<Message, Theme, Renderer> for FeatureStructureScene {
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

        for line in &self.layout.lines {
            let path = canvas::Path::line(
                Point::new(
                    offset_x + line.from_x * scale,
                    offset_y + line.from_y * scale,
                ),
                Point::new(
                    offset_x + line.to_x * scale,
                    offset_y + line.to_y * scale,
                ),
            );
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(theme::TEXT)
                    .with_width((1.2 * scale).max(0.8)),
            );
        }

        for item in &self.layout.boxes {
            let path = canvas::Path::rectangle(
                Point::new(offset_x + item.x * scale, offset_y + item.y * scale),
                Size::new(item.width * scale, item.height * scale),
            );
            frame.stroke(
                &path,
                canvas::Stroke::default()
                    .with_color(theme::TEXT)
                    .with_width((1.0 * scale).max(0.8)),
            );
        }

        for item in &self.layout.texts {
            frame.fill_text(canvas::Text {
                content: item.text.clone(),
                position: Point::new(
                    offset_x + item.x * scale,
                    offset_y + item.y * scale,
                ),
                color: theme::TEXT,
                size: Pixels::from((13.0 * scale).clamp(9.0, 22.0)),
                line_height: text::LineHeight::default(),
                font: Font::with_name("Inter"),
                horizontal_alignment: if item.centered {
                    alignment::Horizontal::Center
                } else {
                    alignment::Horizontal::Left
                },
                vertical_alignment: alignment::Vertical::Center,
                shaping: text::Shaping::Advanced,
            });
        }

        vec![frame.into_geometry()]
    }
}
