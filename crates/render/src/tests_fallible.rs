//! Per-call failures must not masquerade as successful blank slides.
use super::*;
use deckcraft_model::{Fill, Line, Xfrm, style::ColorRef};
use std::cell::{Cell, RefCell};

fn deck() -> Presentation {
    let mut presentation = Presentation::default();
    Arc::make_mut(&mut presentation.slides[0]).shapes = vec![Shape {
        id: ShapeId(901),
        xfrm: Some(Xfrm::new(-20.0, 30.0, 100.0, 80.0)),
        fill: Some(Fill::solid(ColorRef::rgb(Rgba::rgb(230, 30, 60)))),
        line: Some(Line::none()),
        ..Default::default()
    }];
    presentation
}

fn same(a: &(Image, Rect), b: &(Image, Rect)) {
    assert_eq!((a.0.width, a.0.height), (b.0.width, b.0.height));
    assert_eq!(a.0.pixels, b.0.pixels);
    assert_eq!(a.1, b.1);
}

#[test]
fn fallible_success_matches_legacy_pixels_extent_and_real_blank() {
    let presentation = deck();
    for scale in [0.25, 1.0, 2.0] {
        let options = RenderOpts { scale, offset: (12.0, 8.0), size: Some((320, 200)), ..Default::default() };
        let fallible = try_render_slide_drawn(&presentation, 0, &options).unwrap();
        same(&fallible, &render_slide_drawn(&presentation, 0, &options));
        assert!(fallible.1.x0 < 0.0, "the drawing extent includes the off-slide shape");
        assert!(fallible.0.pixels.chunks_exact(4).any(|pixel| pixel != [255, 255, 255, 255]));
        assert_eq!(try_render_slide(&presentation, 0, &options).unwrap().pixels, fallible.0.pixels);
    }
    let blank = Presentation::default();
    let options = RenderOpts { scale: 0.25, ..Default::default() };
    let image = try_render_slide(&blank, 0, &options).unwrap();
    assert_eq!((image.width, image.height), (240, 135));
    assert!(image.pixels.chunks_exact(4).all(|pixel| pixel == [255, 255, 255, 255]));
}

#[test]
fn fallible_invalid_index_preserves_legacy_empty_image_and_page_extent() {
    let presentation = deck();
    let options = RenderOpts { size: Some((17, 13)), ..Default::default() };
    for index in [presentation.slides.len(), usize::MAX] {
        assert!(matches!(try_render_slide(&presentation, index, &options), Err(RenderError::InvalidSlideIndex)));
        assert!(matches!(try_render_slide_drawn(&presentation, index, &options), Err(RenderError::InvalidSlideIndex)));
        let (image, extent) = render_slide_drawn(&presentation, index, &options);
        assert_eq!((image.width, image.height), (0, 0));
        assert!(image.pixels.is_empty());
        assert_eq!(extent, Rect::new(0.0, 0.0, presentation.slide_size.width, presentation.slide_size.height));
        assert!(render_slide(&presentation, index, &options).pixels.is_empty());
    }
}

#[test]
fn fallible_callback_panic_reports_failure_and_next_call_recovers() {
    let presentation = deck();
    let healthy = RenderOpts { size: Some((160, 100)), ..Default::default() };
    let baseline = try_render_slide_drawn(&presentation, 0, &healthy).unwrap();
    let calls = Cell::new(0);
    let panic_state = |_| {
        calls.set(calls.get() + 1);
        panic!("synthetic state callback failure");
    };
    let options = RenderOpts { state: Some(&panic_state), size: Some((160, 100)), ..Default::default() };
    assert!(matches!(try_render_slide_drawn(&presentation, 0, &options), Err(RenderError::Panicked)));
    assert_eq!(calls.get(), 1);
    same(&baseline, &try_render_slide_drawn(&presentation, 0, &healthy).unwrap());
    assert!(matches!(try_render_slide(&presentation, 0, &options), Err(RenderError::Panicked)));
    let (fallback, extent) = render_slide_drawn(&presentation, 0, &options);
    assert_eq!((fallback.width, fallback.height), (160, 100));
    assert!(fallback.pixels.iter().all(|value| *value == 255));
    assert_eq!(extent, Rect::new(0.0, 0.0, presentation.slide_size.width, presentation.slide_size.height));
    same(&baseline, &try_render_slide_drawn(&presentation, 0, &healthy).unwrap());
    // Results are per-call. The process-global diagnostic may also change on other test threads.
}

#[test]
fn fallible_reentry_is_busy_without_poisoning_outer_pixels_or_extent() {
    let presentation = deck();
    let options = RenderOpts { scale: 0.5, ..Default::default() };
    let baseline = try_render_slide_drawn(&presentation, 0, &options).unwrap();
    let observed = RefCell::new(Vec::new());
    let state = |_| {
        observed.borrow_mut().push(try_render_slide_drawn(&presentation, 0, &options).unwrap_err());
        assert!(matches!(try_render_slide(&presentation, 0, &options), Err(RenderError::Busy)));
        let (legacy, extent) = render_slide_drawn(&presentation, 0, &options);
        assert_eq!((legacy.width, legacy.height), (480, 270));
        assert!(legacy.pixels.iter().all(|value| *value == 255));
        assert_eq!(extent, Rect::new(0.0, 0.0, presentation.slide_size.width, presentation.slide_size.height));
        ShapeState::default()
    };
    let reentrant = RenderOpts { state: Some(&state), scale: 0.5, ..Default::default() };
    let actual = try_render_slide_drawn(&presentation, 0, &reentrant).unwrap();
    assert!(!observed.borrow().is_empty());
    assert!(observed.borrow().iter().all(|error| *error == RenderError::Busy));
    same(&baseline, &actual);
    same(&baseline, &try_render_slide_drawn(&presentation, 0, &options).unwrap());
}

#[test]
fn fallible_concurrent_results_do_not_attribute_another_threads_panic() {
    let barrier = Arc::new(std::sync::Barrier::new(2));
    let fail_barrier = barrier.clone();
    let fail = std::thread::spawn(move || {
        let presentation = deck();
        let callback = |_| {
            fail_barrier.wait();
            panic!("synthetic concurrent state failure");
        };
        let options = RenderOpts { state: Some(&callback), scale: 0.25, ..Default::default() };
        let result = try_render_slide_drawn(&presentation, 0, &options);
        assert!(matches!(result, Err(RenderError::Panicked)));
        assert!(try_render_slide(&presentation, 0, &RenderOpts { scale: 0.25, ..Default::default() }).is_ok());
    });
    let healthy = std::thread::spawn(move || {
        let presentation = deck();
        let options = RenderOpts { scale: 0.25, ..Default::default() };
        let baseline = try_render_slide_drawn(&presentation, 0, &options).unwrap();
        let first = Cell::new(true);
        let callback = |_| {
            if first.replace(false) {
                barrier.wait();
            }
            ShapeState::default()
        };
        let concurrent = RenderOpts { state: Some(&callback), scale: 0.25, ..Default::default() };
        same(&baseline, &try_render_slide_drawn(&presentation, 0, &concurrent).unwrap());
        same(&baseline, &try_render_slide_drawn(&presentation, 0, &options).unwrap());
    });
    fail.join().unwrap();
    healthy.join().unwrap();
}

#[test]
fn fallible_slide_part_matches_legacy_pixels_and_reports_failures() {
    let presentation = deck();
    let options = RenderOpts { scale: 1.0, ..Default::default() };
    for part in [SlidePart::Backdrop, SlidePart::Content] {
        let fallible = try_render_slide_part(&presentation, 0, part, &options).unwrap();
        let legacy = render_slide_part(&presentation, 0, part, &options);
        assert_eq!((fallible.width, fallible.height), (legacy.width, legacy.height));
        assert_eq!(fallible.pixels, legacy.pixels);
        assert_eq!(try_render_slide_part(&presentation, 9, part, &options).unwrap_err(), RenderError::InvalidSlideIndex);
    }
}
