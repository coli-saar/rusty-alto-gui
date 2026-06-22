use crate::{model::FeatureStructureLayout, svg_view};
use iced::{
    Element, Length, Size,
    widget::{scrollable, svg},
};
use std::{fmt::Write, sync::Arc};

/// Render a feature structure as a scrollable SVG.
///
/// `on_export` builds the message emitted when the user presses the diagram to
/// drag it out as a PDF; it receives the diagram's SVG source and its on-screen
/// size. Drag-out only exists on macOS and Windows (see `app::start_pdf_drag`),
/// so on other platforms the press is not wired up.
pub fn feature_structure_view<Message: 'static + Clone>(
    layout: Arc<FeatureStructureLayout>,
    zoom: f32,
    on_export: impl FnOnce(Arc<String>, Size) -> Message,
) -> Element<'static, Message> {
    let scale = zoom.max(0.35);
    let natural_width = layout.width + 24.0;
    let natural_height = layout.height + 36.0;
    let width = natural_width * scale;
    let height = natural_height * scale;
    let svg = Arc::new(feature_structure_svg(&layout, natural_width, natural_height));
    let image = svg_view::natural_svg(
        svg::Handle::from_memory(svg.as_bytes().to_vec()),
        Size::new(width, height),
    );
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    let image = iced::widget::mouse_area(image).on_press(on_export(svg, Size::new(width, height)));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let _ = (on_export, svg);

    scrollable(image)
        .direction(scrollable::Direction::Both {
            vertical: scrollable::Scrollbar::default(),
            horizontal: scrollable::Scrollbar::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn feature_structure_svg(layout: &FeatureStructureLayout, width: f32, height: f32) -> String {
    const OFFSET_X: f32 = 12.0;
    const OFFSET_Y: f32 = 18.0;
    let mut svg = String::new();
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}">"#
    )
    .unwrap();
    svg.push_str(
        r##"<g fill="none" stroke="#202733" stroke-width="1.1" stroke-linecap="square">"##,
    );
    for line in &layout.lines {
        write!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
            OFFSET_X + line.from_x,
            OFFSET_Y + line.from_y,
            OFFSET_X + line.to_x,
            OFFSET_Y + line.to_y,
        )
        .unwrap();
    }
    for item in &layout.boxes {
        write!(
            svg,
            r#"<rect x="{}" y="{}" width="{}" height="{}"/>"#,
            OFFSET_X + item.x,
            OFFSET_Y + item.y,
            item.width,
            item.height,
        )
        .unwrap();
    }
    svg.push_str("</g>");
    svg.push_str(
        r##"<g fill="#202733" font-family="Inter, sans-serif" font-size="13" dominant-baseline="middle">"##,
    );
    for item in &layout.texts {
        write!(
            svg,
            r#"<text x="{}" y="{}" text-anchor="{}">{}</text>"#,
            OFFSET_X + item.x,
            OFFSET_Y + item.y,
            if item.centered { "middle" } else { "start" },
            escape_xml(&item.text),
        )
        .unwrap();
    }
    svg.push_str("</g></svg>");
    svg
}

fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{FeatureStructureBox, FeatureStructureLine, FeatureStructureText};

    #[test]
    fn svg_contains_every_feature_structure_primitive() {
        let layout = FeatureStructureLayout {
            texts: vec![FeatureStructureText {
                text: "case<&".into(),
                x: 10.0,
                y: 12.0,
                centered: false,
            }],
            lines: vec![FeatureStructureLine {
                from_x: 0.0,
                from_y: 0.0,
                to_x: 20.0,
                to_y: 0.0,
            }],
            boxes: vec![FeatureStructureBox {
                x: 2.0,
                y: 2.0,
                width: 10.0,
                height: 10.0,
            }],
            width: 30.0,
            height: 24.0,
        };
        let output = feature_structure_svg(&layout, 54.0, 60.0);
        assert_eq!(output.matches("<text ").count(), 1);
        assert_eq!(output.matches("<line ").count(), 1);
        assert_eq!(output.matches("<rect ").count(), 1);
        assert!(output.contains("case&lt;&amp;"));
    }
}
