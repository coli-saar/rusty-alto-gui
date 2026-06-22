//! Converting the in-app SVG views (trees, feature structures) to PDF.
//!
//! Every visual is already produced as SVG source (see [`crate::tree_canvas`]
//! and [`crate::feature_canvas`]), so exporting is a matter of re-parsing that
//! source and re-emitting it as PDF. The on-screen rendering uses the bundled
//! Inter font, so we load the same font data into usvg's database to keep the
//! PDF visually identical and independent of whatever fonts the host has.
//!
//! The actual drag-out gesture is platform-specific and lives in
//! [`crate::app`]; this module only provides the pure conversions plus a helper
//! to stage the PDF in a temporary file.

use std::path::PathBuf;

// The same font faces registered with iced in `app::run`.
const INTER_REGULAR: &[u8] = include_bytes!("../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../assets/fonts/Inter-SemiBold.ttf");

/// Upper bound, in pixels, on the drag preview's longest side. The preview is
/// normally drawn at the view's on-screen size; this just keeps a hugely
/// zoomed-in tree from producing an enormous bitmap.
const THUMBNAIL_MAX_DIM: f32 = 1024.0;

fn parse_tree(svg: &str) -> Result<usvg::Tree, String> {
    let mut options = usvg::Options::default();
    {
        let fontdb = options.fontdb_mut();
        fontdb.load_font_data(INTER_REGULAR.to_vec());
        fontdb.load_font_data(INTER_MEDIUM.to_vec());
        fontdb.load_font_data(INTER_SEMIBOLD.to_vec());
        // Fall back to host fonts for any glyphs Inter is missing, then make
        // the SVG's "sans-serif" request resolve to Inter like it does on screen.
        fontdb.load_system_fonts();
        fontdb.set_sans_serif_family("Inter");
    }
    usvg::Tree::from_str(svg, &options).map_err(|error| format!("Could not parse SVG: {error}"))
}

/// Convert SVG source into a standalone PDF document.
pub fn svg_to_pdf(svg: &str) -> Result<Vec<u8>, String> {
    let tree = parse_tree(svg)?;
    svg2pdf::to_pdf(
        &tree,
        svg2pdf::ConversionOptions::default(),
        svg2pdf::PageOptions::default(),
    )
    .map_err(|error| format!("Could not render PDF: {error}"))
}

/// Rasterize SVG source to PNG bytes for use as a drag-cursor preview, scaled
/// so its longest side is `target_longest_side` pixels (clamped to
/// [`THUMBNAIL_MAX_DIM`]).
///
/// Pass the view's on-screen size so the preview matches what the user sees: a
/// correctly sized preview centered on the pointer reads as "I'm dragging this
/// exact thing", whereas an arbitrarily scaled one looks detached from the
/// cursor.
///
/// Returns `None` on any failure: the preview is cosmetic, so a missing
/// thumbnail must never block the export itself.
pub fn svg_to_png_thumbnail(svg: &str, target_longest_side: f32) -> Option<Vec<u8>> {
    let tree = parse_tree(svg).ok()?;
    let size = tree.size();
    let intrinsic_longest = size.width().max(size.height()).max(1.0);
    let target = target_longest_side.clamp(1.0, THUMBNAIL_MAX_DIM);
    let scale = target / intrinsic_longest;
    let width = ((size.width() * scale).ceil() as u32).max(1);
    let height = ((size.height() * scale).ceil() as u32).max(1);
    let mut pixmap = tiny_skia::Pixmap::new(width, height)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().ok()
}

/// A drag-cursor preview for `svg`, sized to `target_longest_side` pixels, that
/// always yields valid PNG bytes.
///
/// macOS builds an `NSImage` from these bytes and panics on invalid data, so
/// this never returns the empty/`None` case: it falls back to a 1x1 pixel when
/// rasterization fails for any reason.
pub fn drag_thumbnail(svg: &str, target_longest_side: f32) -> Vec<u8> {
    svg_to_png_thumbnail(svg, target_longest_side).unwrap_or_else(|| {
        tiny_skia::Pixmap::new(1, 1)
            .and_then(|pixmap| pixmap.encode_png().ok())
            .unwrap_or_default()
    })
}

/// Write a PDF rendering of `svg` to a uniquely named temporary file and return
/// its path.
///
/// The file is intentionally left on disk: a native drag-out completes
/// asynchronously, so the drop target reads the file after this call returns.
/// Files land in a dedicated temp subdirectory that the OS reclaims on its own.
pub fn write_temp_pdf(svg: &str, name: &str) -> Result<PathBuf, String> {
    let pdf = svg_to_pdf(svg)?;
    let dir = std::env::temp_dir().join("rusty-alto-export");
    std::fs::create_dir_all(&dir).map_err(|error| format!("Could not create temp dir: {error}"))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = dir.join(format!("{name}-{stamp}.pdf"));
    std::fs::write(&path, pdf).map_err(|error| format!("Could not write PDF: {error}"))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A minimal version of the SVG the tree/feature views emit: a rect plus a
    // text node in the bundled font.
    const SAMPLE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 80 40"><rect x="2" y="2" width="76" height="36" rx="3" fill="#ffffff" stroke="#647181"/><text x="40" y="20" font-family="Inter, sans-serif" font-size="13" text-anchor="middle" dominant-baseline="middle">NP</text></svg>"##;

    #[test]
    fn produces_a_pdf_document() {
        let pdf = svg_to_pdf(SAMPLE_SVG).expect("conversion should succeed");
        // Every PDF starts with the "%PDF-" magic header.
        assert!(pdf.starts_with(b"%PDF-"), "output should be a PDF");
    }

    #[test]
    fn invalid_svg_is_an_error() {
        assert!(svg_to_pdf("not an svg at all").is_err());
    }

    #[test]
    fn produces_a_png_thumbnail() {
        let png = svg_to_png_thumbnail(SAMPLE_SVG, 160.0).expect("thumbnail should render");
        // PNG signature.
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    }
}
