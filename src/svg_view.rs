use iced::advanced::svg::Renderer as _;
use iced::{
    Element, Length, Point, Radians, Rectangle, Renderer, Size, Theme,
    advanced::{Layout, Widget, layout, mouse, renderer, svg, widget::Tree},
    widget::svg::Handle,
};

/// An SVG widget that keeps the document at its requested natural size.
///
/// A scrollable forces its child to be at least as large as the viewport.
/// Iced's stock SVG widget scales the document to those enlarged bounds,
/// turning 100% zoom into "fit to panel". This widget instead centers the
/// natural-size document in the enlarged bounds and only scales when callers
/// explicitly change `natural_size`.
pub fn natural_svg<Message: 'static>(
    handle: Handle,
    natural_size: Size,
) -> Element<'static, Message> {
    Element::new(NaturalSvg {
        handle,
        natural_size,
    })
}

struct NaturalSvg {
    handle: Handle,
    natural_size: Size,
}

impl<Message> Widget<Message, Theme, Renderer> for NaturalSvg {
    fn size(&self) -> Size<Length> {
        Size::new(
            Length::Fixed(self.natural_size.width),
            Length::Fixed(self.natural_size.height),
        )
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(
            limits,
            Length::Fixed(self.natural_size.width),
            Length::Fixed(self.natural_size.height),
        )
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let bounds = layout.bounds();
        let drawing_bounds = Rectangle::new(
            Point::new(
                bounds.center_x() - self.natural_size.width / 2.0,
                bounds.center_y() - self.natural_size.height / 2.0,
            ),
            self.natural_size,
        );
        renderer.draw_svg(
            svg::Svg {
                handle: self.handle.clone(),
                color: None,
                rotation: Radians(0.0),
                opacity: 1.0,
            },
            drawing_bounds,
            bounds,
        );
    }
}
