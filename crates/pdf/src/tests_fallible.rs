//! Failures must reach the PDF caller rather than become embedded white artwork.
use super::*;
use deckcraft_model::{Fill as ShapeFill, Line, Rgba, Shape, ShapeId, Xfrm, style::ColorRef};
use deckcraft_render::{RenderError, ShapeState, try_render_slide_drawn};
use std::{cell::Cell, error::Error, sync::Arc};

fn layouts() -> [PageLayout; 3] {
    [PageLayout::Slides, PageLayout::Notes, PageLayout::Handouts { per_page: 3 }]
}

fn deck() -> Presentation {
    let mut p = Presentation::default();
    Arc::make_mut(&mut p.slides[0]).shapes = vec![Shape {
        id: ShapeId(114),
        xfrm: Some(Xfrm::new(-20.0, 30.0, 100.0, 80.0)),
        fill: Some(ShapeFill::solid(ColorRef::rgb(Rgba::rgb(230, 30, 60)))),
        line: Some(Line::none()),
        ..Default::default()
    }];
    p
}

#[test]
fn fallible_pdf_busy_propagates_through_all_layouts_without_poisoning_outer_render() {
    let p = deck();
    let render_opts = RenderOpts { scale: 0.25, ..Default::default() };
    let baseline = try_render_slide_drawn(&p, 0, &render_opts).unwrap();
    assert!(baseline.1.x0 < 0.0);
    for layout in layouts() {
        let options = PdfOptions { layout, dpi: 48.0, ..Default::default() };
        let observed = Cell::new(0);
        let state = |_| {
            let error = export(&p, &options).unwrap_err();
            assert!(matches!(error, PdfError::Render(RenderError::Busy)), "{layout:?}: {error}");
            assert_eq!(error.to_string(), "slide rendering: this thread's slide renderer is already in use");
            assert_eq!(error.source().unwrap().downcast_ref::<RenderError>(), Some(&RenderError::Busy));
            observed.set(observed.get() + 1);
            ShapeState::default()
        };
        let actual = try_render_slide_drawn(&p, 0, &RenderOpts { state: Some(&state), scale: 0.25, ..Default::default() }).unwrap();
        assert!(observed.get() > 0, "export must run inside the actual renderer callback");
        assert_eq!((actual.0.width, actual.0.height, actual.1), (baseline.0.width, baseline.0.height, baseline.1));
        assert_eq!(actual.0.pixels, baseline.0.pixels);
        let recovered = try_render_slide_drawn(&p, 0, &render_opts).unwrap();
        assert_eq!(recovered.0.pixels, baseline.0.pixels);
        assert_eq!(recovered.1, baseline.1);
        let bytes = export(&p, &options).unwrap();
        let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
        assert_eq!(pdf.pages().len(), 1);
    }
}

#[test]
fn fallible_pdf_genuinely_blank_slides_still_export_and_reopen() {
    let mut p = Presentation::default();
    Arc::make_mut(&mut p.slides[0]).shapes.clear();
    let blank = try_render_slide_drawn(&p, 0, &RenderOpts { scale: 0.25, ..Default::default() }).unwrap();
    assert!(blank.0.pixels.chunks_exact(4).all(|pixel| pixel == [255, 255, 255, 255]));
    for layout in layouts() {
        let options = PdfOptions { layout, dpi: 48.0, ..Default::default() };
        let bytes = export(&p, &options).unwrap();
        let pdf = hayro_syntax::Pdf::new(bytes).unwrap();
        assert_eq!(pdf.pages().len(), 1);
        let size = if layout == PageLayout::Slides { (p.slide_size.width, p.slide_size.height) } else { options.paper };
        assert_eq!(pdf.pages()[0].render_dimensions(), (size.0 as f32, size.1 as f32));
    }
}
