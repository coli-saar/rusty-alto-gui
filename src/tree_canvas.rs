use crate::{model::TreeLayout, svg_view};
use iced::{
    Element, Length, Size,
    widget::{scrollable, svg},
};
use std::{fmt::Write, sync::Arc};

pub fn tree_view<Message: 'static>(
    layout: Arc<TreeLayout>,
    zoom: f32,
) -> Element<'static, Message> {
    let scale = zoom.max(0.35);
    let natural_width = layout.width + 24.0;
    let natural_height = layout.height + 36.0;
    let width = natural_width * scale;
    let height = natural_height * scale;
    let image = svg_view::natural_svg(
        svg::Handle::from_memory(tree_svg(&layout, natural_width, natural_height)),
        Size::new(width, height),
    );

    scrollable(image)
        .direction(scrollable::Direction::Both {
            vertical: scrollable::Scrollbar::default(),
            horizontal: scrollable::Scrollbar::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn tree_svg(layout: &TreeLayout, width: f32, height: f32) -> Vec<u8> {
    const OFFSET_X: f32 = 12.0;
    const OFFSET_Y: f32 = 18.0;
    let mut svg = String::new();
    write!(
        svg,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}">"#
    )
    .unwrap();
    svg.push_str(r##"<g fill="none" stroke="#647181" stroke-width="1.2" stroke-linecap="round">"##);
    for edge in &layout.edges {
        write!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}"/>"#,
            OFFSET_X + edge.parent_x,
            OFFSET_Y + edge.parent_y,
            OFFSET_X + edge.child_x,
            OFFSET_Y + edge.child_y,
        )
        .unwrap();
    }
    svg.push_str("</g>");
    svg.push_str(r##"<g fill="#ffffff">"##);
    for node in &layout.nodes {
        write!(
            svg,
            r#"<rect x="{}" y="{}" width="{}" height="30"/>"#,
            OFFSET_X + node.x - node.width / 2.0,
            OFFSET_Y + node.y,
            node.width,
        )
        .unwrap();
    }
    svg.push_str("</g>");
    svg.push_str(
        r##"<g fill="#202733" font-family="Inter, sans-serif" font-size="13" text-anchor="middle" dominant-baseline="middle">"##,
    );
    for node in &layout.nodes {
        write!(
            svg,
            r#"<text x="{}" y="{}">{}</text>"#,
            OFFSET_X + node.x,
            OFFSET_Y + node.y + 15.0,
            escape_xml(&node.label),
        )
        .unwrap();
    }
    svg.push_str("</g></svg>");
    svg.into_bytes()
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
    use crate::model::{TreeEdge, TreeNode};

    #[test]
    fn svg_contains_every_node_and_edge_and_escapes_labels() {
        let layout = TreeLayout {
            nodes: vec![
                TreeNode {
                    label: "a<&".into(),
                    x: 30.0,
                    y: 20.0,
                    width: 58.0,
                },
                TreeNode {
                    label: "b".into(),
                    x: 30.0,
                    y: 94.0,
                    width: 58.0,
                },
            ],
            edges: vec![TreeEdge {
                parent_x: 30.0,
                parent_y: 50.0,
                child_x: 30.0,
                child_y: 94.0,
            }],
            width: 60.0,
            height: 124.0,
        };
        let output = String::from_utf8(tree_svg(&layout, 84.0, 160.0)).unwrap();
        assert_eq!(output.matches("<text ").count(), 2);
        assert_eq!(output.matches("<line ").count(), 1);
        assert_eq!(output.matches("<rect ").count(), 2);
        assert!(output.contains("a&lt;&amp;"));
    }
}
