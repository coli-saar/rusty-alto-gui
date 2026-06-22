use crate::{
    model::{ConflictHighlight, TreeLayout},
    svg_view,
};
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
    for edge in &layout.edges {
        write!(
            svg,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" fill="none" stroke="{}" stroke-width="1.2" stroke-linecap="round"/>"#,
            OFFSET_X + edge.parent_x,
            OFFSET_Y + edge.parent_y,
            OFFSET_X + edge.child_x,
            OFFSET_Y + edge.child_y,
            if edge.muted { "#b8c0ca" } else { "#647181" },
        )
        .unwrap();
    }
    for node in &layout.nodes {
        let has_half_sources = node.top_source != ConflictHighlight::None
            || node.bottom_source != ConflictHighlight::None;
        let (fill, stroke) = match (has_half_sources, node.conflict) {
            (true, _) => ("#ffffff", "#647181"),
            (false, ConflictHighlight::Left) => ("#e7f6fa", "#147d92"),
            (false, ConflictHighlight::Right) => ("#fff1df", "#b35c00"),
            (false, ConflictHighlight::Both) => ("#ffffff", "#647181"),
            (false, ConflictHighlight::None) if node.muted => ("#f5f6f8", "#cbd1d8"),
            (false, ConflictHighlight::None) => ("#ffffff", "#d4d9df"),
        };
        write!(
            svg,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}" stroke="{}" stroke-width="1"/>"#,
            OFFSET_X + node.x - node.width / 2.0,
            OFFSET_Y + node.y,
            node.width,
            node.height,
            fill,
            stroke,
        )
        .unwrap();
        if has_half_sources {
            let x = OFFSET_X + node.x - node.width / 2.0;
            let color = |side| match side {
                ConflictHighlight::Left => "#e7f6fa",
                ConflictHighlight::Right => "#fff1df",
                _ => "#ffffff",
            };
            for (source, y) in [
                (node.top_source, OFFSET_Y + node.y),
                (node.bottom_source, OFFSET_Y + node.y + node.height / 2.0),
            ] {
                if source != ConflictHighlight::None {
                    write!(
                        svg,
                        r#"<path d="M {x} {y} h {} v {} h -{} z" fill="{}"/>"#,
                        node.width,
                        node.height / 2.0,
                        node.width,
                        color(source),
                    )
                    .unwrap();
                }
            }
            write!(
                svg,
                r##"<rect x="{x}" y="{}" width="{}" height="{}" rx="3" fill="none" stroke="#647181" stroke-width="1"/>"##,
                OFFSET_Y + node.y,
                node.width,
                node.height,
            )
            .unwrap();
        }
    }
    for node in &layout.nodes {
        let color = if node.muted && node.conflict == ConflictHighlight::None {
            "#8a949f"
        } else {
            "#202733"
        };
        let mut line_y = OFFSET_Y + node.y + 15.0;
        if let Some(top) = &node.top {
            let (top_color, top_weight) = if node.top_conflict {
                ("#c62828", "700")
            } else {
                (color, "400")
            };
            write!(
                svg,
                r#"<text x="{}" y="{}" fill="{}" font-family="Inter, sans-serif" font-size="11" font-weight="{}" text-anchor="middle" dominant-baseline="middle">top: {}</text>"#,
                OFFSET_X + node.x,
                line_y,
                top_color,
                top_weight,
                escape_xml(top),
            )
            .unwrap();
            line_y += 20.0;
        }
        write!(
            svg,
            r#"<text x="{}" y="{}" fill="{}" font-family="Inter, sans-serif" font-size="13" font-weight="600" text-anchor="middle" dominant-baseline="middle">{}</text>"#,
            OFFSET_X + node.x,
            line_y,
            color,
            escape_xml(&node.label),
        )
        .unwrap();
        line_y += 20.0;
        if let Some(bottom) = &node.bottom {
            let (bottom_color, bottom_weight) = if node.bottom_conflict {
                ("#c62828", "700")
            } else {
                (color, "400")
            };
            write!(
                svg,
                r#"<text x="{}" y="{}" fill="{}" font-family="Inter, sans-serif" font-size="11" font-weight="{}" text-anchor="middle" dominant-baseline="middle">bot: {}</text>"#,
                OFFSET_X + node.x,
                line_y,
                bottom_color,
                bottom_weight,
                escape_xml(bottom),
            )
            .unwrap();
        }
    }
    svg.push_str("</svg>");
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
                    top: None,
                    bottom: None,
                    muted: false,
                    conflict: ConflictHighlight::None,
                    top_source: ConflictHighlight::None,
                    bottom_source: ConflictHighlight::None,
                    top_conflict: false,
                    bottom_conflict: false,
                    x: 30.0,
                    y: 20.0,
                    width: 58.0,
                    height: 30.0,
                },
                TreeNode {
                    label: "b".into(),
                    top: None,
                    bottom: None,
                    muted: false,
                    conflict: ConflictHighlight::None,
                    top_source: ConflictHighlight::None,
                    bottom_source: ConflictHighlight::None,
                    top_conflict: false,
                    bottom_conflict: false,
                    x: 30.0,
                    y: 94.0,
                    width: 58.0,
                    height: 30.0,
                },
            ],
            edges: vec![TreeEdge {
                parent_x: 30.0,
                parent_y: 50.0,
                child_x: 30.0,
                child_y: 94.0,
                muted: false,
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

    #[test]
    fn svg_uses_distinct_conflict_colors_and_embeds_both_values() {
        let layout = TreeLayout {
            nodes: vec![TreeNode {
                label: "NP".into(),
                top: Some("[case: nom]".into()),
                bottom: Some("[case: acc]".into()),
                muted: false,
                conflict: ConflictHighlight::Both,
                top_source: ConflictHighlight::Left,
                bottom_source: ConflictHighlight::Right,
                top_conflict: true,
                bottom_conflict: true,
                x: 100.0,
                y: 20.0,
                width: 180.0,
                height: 70.0,
            }],
            edges: Vec::new(),
            width: 200.0,
            height: 110.0,
        };
        let output = String::from_utf8(tree_svg(&layout, 224.0, 146.0)).unwrap();
        assert!(output.contains("#e7f6fa"));
        assert!(output.contains("#fff1df"));
        assert!(!output.contains("<path") || !output.contains("<path stroke="));
        assert!(output.contains("top: [case: nom]"));
        assert!(output.contains("bot: [case: acc]"));
        assert_eq!(output.matches("fill=\"#c62828\"").count(), 2);
        assert_eq!(output.matches("font-weight=\"700\"").count(), 2);
    }
}
