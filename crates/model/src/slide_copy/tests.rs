#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use super::*;
use crate::{style::*, text::*, *};

fn link(id: SlideId) -> Hyperlink {
    Hyperlink {
        action: Action::Slide { slide: id },
        tooltip: "destination".into(),
        highlight_click: true,
        sound: None,
        stop_sound: false,
        zoom_return: None,
    }
}

fn fixture() -> Presentation {
    let mut p = Presentation::default();
    let source = p.slides[0].id;
    let layout = defaults::layout_of_kind(&p, LayoutType::Blank).unwrap();
    let other = defaults::new_slide(&mut p, layout).unwrap();
    p.slides.push(Arc::new(other));
    p.media.push(MediaItem {
        id: MediaId(9_000),
        name: "shared.png".into(),
        content_type: "image/png".into(),
        data: Arc::new(vec![1, 2, 3]),
        link: None,
    });
    let mut text = TextBody::from_text("Grouped label\nSecond paragraph");
    text.paragraphs[0].runs[0].props.link = Some(link(source));
    text.paragraphs[0].def_props.hover = Some(link(p.slides[1].id));
    text.list_style.set(0, LevelStyle { run: RunProps { link: Some(link(source)), ..Default::default() }, ..Default::default() });
    let group = Shape {
        id: ShapeId(1_000),
        name: "group".into(),
        kind: ShapeKind::Group {
            child: Xfrm::new(0., 0., 500., 300.),
            children: vec![
                Shape { id: ShapeId(1_001), name: "child".into(), text: Some(text), click: Some(link(source)), ..Default::default() },
                Shape {
                    id: ShapeId(1_002),
                    name: "table".into(),
                    kind: ShapeKind::Table(Table {
                        cols: vec![80.],
                        rows: vec![crate::table::Row { height: 30., cells: vec![Cell::new("Table cell")] }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
        },
        ..Default::default()
    };
    let picture_fill = PictureFill { media: MediaId(9_000), ..Default::default() };
    let source_slide = Arc::make_mut(&mut p.slides[0]);
    source_slide.name = "Source".into();
    source_slide.shapes = vec![
        group,
        Shape {
            id: ShapeId(1_003),
            kind: ShapeKind::Connector { start: Some((ShapeId(1_001), 0)), end: Some((ShapeId(1_002), 1)) },
            ..Default::default()
        },
        Shape { id: ShapeId(1_004), kind: ShapeKind::Picture { fill: picture_fill.clone() }, ..Default::default() },
    ];
    source_slide.background = Some(Background::Fill { fill: Fill::Picture(picture_fill) });
    source_slide.notes = TextBody::from_text("Speaker notes");
    source_slide.notes.paragraphs[0].runs[0].props.link = Some(link(source));
    source_slide.animations = vec![Animation { shape: ShapeId(1_001), trigger: Some(ShapeId(1_002)), paragraph: Some(1), ..Default::default() }];
    Arc::make_mut(&mut p.masters[0]).layouts[0].shapes[0].click = Some(link(source));
    p
}

fn ids(slide: &Slide) -> Vec<u32> {
    let mut out = vec![];
    walk(&slide.shapes, &mut |s, _| out.push(s.id.0));
    out
}

#[test]
fn slide_copy_fresh_ids_references_sharing_and_value_independence() {
    let mut p = fixture();
    let original = p.clone();
    let source = p.slides[0].id;
    let new = p.copy_slide(source).unwrap();
    assert_eq!(&p.slides[..2], original.slides.as_slice());
    assert_eq!(p.media, original.media);
    assert!(Arc::ptr_eq(&p.media[0].data, &original.media[0].data));
    assert!(Arc::ptr_eq(&p.masters[0], &original.masters[0]));
    let copy = p.slide(new).unwrap();
    assert_eq!(copy.layout, original.slides[0].layout);
    assert_eq!(copy.notes, original.slides[0].notes);
    assert_eq!(copy.background, original.slides[0].background);
    let before_ids = ids(&original.slides[0]);
    let after_ids = ids(copy);
    assert_eq!(before_ids.len(), after_ids.len());
    assert!(new.0 > 9_000 && after_ids.iter().all(|id| *id > new.0));
    assert!(after_ids.iter().all(|id| !before_ids.contains(id)));
    assert_eq!(p.next_id, *after_ids.iter().max().unwrap());
    let child = copy.shape(ShapeId(after_ids[1])).unwrap();
    assert_eq!(child.click, original.slides[0].shape(ShapeId(1_001)).unwrap().click);
    assert_eq!(child.text, original.slides[0].shape(ShapeId(1_001)).unwrap().text);
    assert_eq!(copy.animations[0].shape, ShapeId(after_ids[1]));
    assert_eq!(copy.animations[0].trigger, Some(ShapeId(after_ids[2])));
    assert!(matches!(copy.shapes[1].kind, ShapeKind::Connector { start: Some((id, 0)), end: Some((end, 1)) }
        if id == ShapeId(after_ids[1]) && end == ShapeId(after_ids[2])));
    let ShapeKind::Table(table) = &mut p.slide_mut(new).unwrap().shape_mut(ShapeId(after_ids[2])).unwrap().kind else {
        panic!("table");
    };
    table.cell_mut(0, 0).unwrap().text = TextBody::from_text("Changed copy");
    assert_eq!(p.slides[0], original.slides[0]);
    p.slides.retain(|s| s.id != new);
    assert_eq!(p.media, original.media);
    assert_eq!(p.slides, original.slides);
}

fn atomic(mut p: Presentation, error: Error) {
    let source = p.slides[0].id;
    let before = p.clone();
    assert_eq!(p.copy_slide(source), Err(error));
    assert_eq!(p, before);
}
use super::CopySlideError as Error;

#[test]
fn slide_copy_rejects_each_opaque_organization_and_action_category_atomically() {
    type Edit = fn(&mut Presentation);
    let cases: &[(&str, Edit, Error)] = &[
        ("section", |p| p.sections.push(Section { name: "S".into(), slides: vec![p.slides[0].id] }), Error::Unsupported("sections or custom shows")),
        ("show", |p| p.show.range = Some((1, 1)), Error::Unsupported("nondefault show settings")),
        ("presentation raw", |p| p.raw_ext = Some("<x/>".into()), Error::Unsupported("presentation opaque data")),
        ("presentation tags", |p| p.tags.push(("x".into(), "y".into())), Error::Unsupported("presentation opaque data")),
        ("transition", |p| Arc::make_mut(&mut p.slides[1]).transition = Some(Transition::default()), Error::Unsupported("transitions")),
        ("comments", |p| Arc::make_mut(&mut p.slides[0]).comments.push(Comment::default()), Error::Unsupported("slide comments or opaque data")),
        ("slide raw", |p| Arc::make_mut(&mut p.slides[0]).raw_ext = Some("".into()), Error::Unsupported("slide comments or opaque data")),
        (
            "shape tags",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].tags.push(("x".into(), "y".into())),
            Error::Unsupported("shape opaque data or SmartArt"),
        ),
        (
            "diagram",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].diagram = Some(Diagram::default()),
            Error::Unsupported("shape opaque data or SmartArt"),
        ),
        (
            "opaque",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].kind = ShapeKind::Opaque { xml: "".into(), preview: None, label: "".into(), parts: vec![] },
            Error::Unsupported("shape kind"),
        ),
        ("ink", |p| Arc::make_mut(&mut p.slides[0]).shapes[0].kind = ShapeKind::Ink { strokes: vec![] }, Error::Unsupported("shape kind")),
        (
            "3d",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].kind = ShapeKind::Model3d { media: MediaId(9_000), poster: None },
            Error::Unsupported("shape kind"),
        ),
        ("animation sound", |p| Arc::make_mut(&mut p.slides[0]).animations[0].sound = Some(MediaId(9_000)), Error::Unsupported("animation sounds")),
        ("external media", |p| p.media[0].link = Some("https://example.com/a".into()), Error::Unsupported("externally linked media")),
        ("master raw", |p| Arc::make_mut(&mut p.masters[0]).raw_ext = Some("".into()), Error::Unsupported("master raw XML")),
        ("layout raw", |p| Arc::make_mut(&mut p.masters[0]).layouts[0].raw_ext = Some("".into()), Error::Unsupported("layout raw XML")),
        ("theme raw", |p| Arc::make_mut(&mut p.masters[0]).theme.raw_extra = Some("".into()), Error::Unsupported("theme raw XML")),
        (
            "table style raw",
            |p| p.table_styles.push(crate::table::TableStyleDef { raw: "<x/>".into(), ..Default::default() }),
            Error::Unsupported("raw table style XML"),
        ),
        (
            "raw3d",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].effects = Some(Effects { raw3d: Some("".into()), ..Default::default() }),
            Error::Unsupported("raw3d effects"),
        ),
        (
            "math",
            |p| Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].runs[0].kind = RunKind::Math { omml: "<x/>".into() },
            Error::Unsupported("math XML"),
        ),
        (
            "relative action",
            |p| p.default_text_style.levels[0].as_mut().unwrap().run.link = Some(Hyperlink { action: Action::LastSlide, ..link(p.slides[0].id) }),
            Error::Unsupported("nonfixed hyperlink action"),
        ),
        (
            "zoom",
            |p| Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].runs[0].props.link.as_mut().unwrap().zoom_return = Some(p.slides[1].id),
            Error::Unsupported("sound or zoom actions"),
        ),
        (
            "link sound",
            |p| Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].runs[0].props.link.as_mut().unwrap().sound = Some(MediaId(9_000)),
            Error::Unsupported("sound or zoom actions"),
        ),
        (
            "artistic",
            |p| {
                let ShapeKind::Picture { fill } = &mut Arc::make_mut(&mut p.slides[0]).shapes[2].kind else {
                    panic!();
                };
                fill.artistic = Some(Artistic { effect: "blur".into(), original: MediaId(9_000) });
            },
            Error::Unsupported("artistic picture effects"),
        ),
    ];
    let more: &[(&str, Edit, Error)] = &[
        ("custom show", |p| p.custom_shows.push(CustomShow::default()), Error::Unsupported("sections or custom shows")),
        ("kept part", |p| p.kept_parts.push(KeptPart::default()), Error::Unsupported("presentation opaque data")),
        ("slide tags", |p| Arc::make_mut(&mut p.slides[1]).tags.push(("x".into(), "y".into())), Error::Unsupported("slide comments or opaque data")),
        ("shape raw", |p| Arc::make_mut(&mut p.slides[0]).shapes[0].raw_ext = Some("".into()), Error::Unsupported("shape opaque data or SmartArt")),
        ("chart", |p| Arc::make_mut(&mut p.slides[0]).shapes[0].kind = ShapeKind::Chart(Box::default()), Error::Unsupported("shape kind")),
        (
            "media player",
            |p| Arc::make_mut(&mut p.slides[0]).shapes[0].kind = ShapeKind::Media(MediaClip::default()),
            Error::Unsupported("shape kind"),
        ),
        (
            "stop sound",
            |p| Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].runs[0].props.link.as_mut().unwrap().stop_sound = true,
            Error::Unsupported("sound or zoom actions"),
        ),
    ];
    for (label, edit, error) in cases.iter().chain(more) {
        let mut p = fixture();
        edit(&mut p);
        eprintln!("refusal category: {label}");
        atomic(p, *error);
    }
}

#[test]
fn slide_copy_references_namespaces_and_reserved_low_ids() {
    let mut low = fixture();
    let source = low.slides[0].id;
    Arc::make_mut(&mut low.slides[1]).shapes = vec![Shape { id: ShapeId(0), ..Default::default() }, Shape { id: ShapeId(1), ..Default::default() }];
    low.next_id = 0;
    assert!(low.copy_slide(source).unwrap().0 > 9_000);
    let mut low_source = Presentation::default();
    Arc::make_mut(&mut low_source.slides[0]).id = SlideId(0);
    Arc::make_mut(&mut low_source.slides[0]).shapes[0].id = ShapeId(0);
    Arc::make_mut(&mut low_source.slides[0]).shapes[1].id = ShapeId(1);
    assert!(low_source.copy_slide(SlideId(0)).unwrap().0 > 1);
    // Benign cross-slide shape-ID reuse remains accepted, but new IDs do not reuse any of it.
    let mut repeated = fixture();
    Arc::make_mut(&mut repeated.slides[1]).shapes = repeated.slides[0].shapes.clone();
    let id = repeated.slides[0].id;
    repeated.copy_slide(id).unwrap();
    let mut p = fixture();
    Arc::make_mut(&mut p.slides[0]).animations[0].paragraph = Some(2);
    atomic(p, Error::Invalid("animation paragraph"));
    let mut p = fixture();
    Arc::make_mut(&mut p.slides[0]).animations[0].trigger = Some(ShapeId(888_888));
    atomic(p, Error::Invalid("missing local shape reference"));
    let mut p = fixture();
    let ShapeKind::Connector { end, .. } = &mut Arc::make_mut(&mut p.slides[0]).shapes[1].kind else {
        panic!();
    };
    *end = Some((ShapeId(999_999), 0));
    atomic(p, Error::Invalid("missing local shape reference"));
    let mut p = fixture();
    p.media.clear();
    atomic(p, Error::Invalid("missing object reference"));
    let mut p = fixture();
    let missing = link(SlideId(999_999));
    Arc::make_mut(&mut p.masters[0]).layouts[0].shapes[0].hover = Some(missing);
    atomic(p, Error::Invalid("missing object reference"));
    let mut p = fixture();
    p.media.push(p.media[0].clone());
    atomic(p, Error::Invalid("duplicate object ID"));
    let mut p = fixture();
    Arc::make_mut(&mut p.slides[0]).shapes[2].id = ShapeId(1_003);
    atomic(p, Error::Invalid("duplicate shape ID in tree"));
    let mut p = fixture();
    Arc::make_mut(&mut p.slides[0]).notes.body.rot = Some(f64::INFINITY);
    atomic(p, Error::Invalid("nonfinite scalar"));
    let mut p = fixture();
    let mut master = p.masters[0].as_ref().clone();
    master.id = MasterId(80_000);
    master.shapes.clear();
    master.layouts = vec![Layout { id: LayoutId(90_000), shapes: vec![Shape { id: ShapeId(99_000), ..Default::default() }], ..Default::default() }];
    p.notes_master = Some(Arc::new(master));
    let id = p.slides[0].id;
    assert!(p.copy_slide(id).unwrap().0 > 99_000);
    let mut exhausted = fixture();
    exhausted.next_id = u32::MAX;
    atomic(exhausted, Error::IdExhausted);
    let mut exact = fixture();
    let n = ids(&exact.slides[0]).len() as u32;
    exact.next_id = u32::MAX - n - 1;
    let source = exact.slides[0].id;
    exact.copy_slide(source).unwrap();
    assert_eq!(exact.next_id, u32::MAX);
    atomic(exact, Error::IdExhausted);
}

#[test]
fn slide_copy_each_action_and_inherited_reference_is_checked() {
    for action in [
        Action::NextSlide,
        Action::PreviousSlide,
        Action::FirstSlide,
        Action::LastSlide,
        Action::EndShow,
        Action::LastViewed,
        Action::CustomShow { name: "show".into(), returns: false },
        Action::Program { path: "app".into() },
        Action::PlayMedia,
    ] {
        let mut p = fixture();
        Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].end_props.link = Some(Hyperlink { action, ..link(p.slides[0].id) });
        atomic(p, Error::Unsupported("nonfixed hyperlink action"));
    }
    for scope in 0..6 {
        let mut p = fixture();
        let missing = Some(link(SlideId(888_888)));
        match scope {
            0 => p.default_text_style.levels[0].as_mut().unwrap().run.hover = missing,
            1 => Arc::make_mut(&mut p.masters[0]).title_style.levels[0].as_mut().unwrap().run.link = missing,
            2 => Arc::make_mut(&mut p.slides[0]).notes.paragraphs[0].end_props.hover = missing,
            3 => {
                let ShapeKind::Table(t) = &mut Arc::make_mut(&mut p.slides[0]).shape_mut(ShapeId(1_002)).unwrap().kind else {
                    panic!();
                };
                t.rows[0].cells[0].text.paragraphs[0].def_props.link = missing;
            }
            4 | 5 => {
                let mut m = p.masters[0].as_ref().clone();
                m.id = MasterId(77_777);
                m.layouts.clear();
                m.shapes.clear();
                m.body_style.levels[0].as_mut().unwrap().run.link = missing;
                if scope == 4 {
                    p.notes_master = Some(Arc::new(m));
                } else {
                    p.handout_master = Some(Arc::new(m));
                }
            }
            _ => panic!("fixture scope"),
        }
        atomic(p, Error::Invalid("missing object reference"));
    }
    let mut p = fixture();
    p.embedded_fonts.push(EmbeddedFont { regular: Some(MediaId(888_888)), ..Default::default() });
    atomic(p, Error::Invalid("missing object reference"));
    let mut p = fixture();
    Arc::make_mut(&mut p.masters[0]).theme.format.fills.push(Fill::Picture(PictureFill { media: MediaId(888_888), ..Default::default() }));
    atomic(p, Error::Invalid("missing object reference"));
}

#[test]
fn slide_copy_clone_payload_counts_dynamic_data_without_media_copy() {
    let p = fixture();
    let id = p.slides[0].id;
    let before = Inventory::inspect(&p, id, LIMITS).unwrap().usage;
    let mut strings = p.clone();
    Arc::make_mut(&mut strings.slides[0]).name.push_str("abc");
    let after = Inventory::inspect(&strings, id, LIMITS).unwrap().usage;
    assert_eq!(after.copied - before.copied, 3);
    assert_eq!(after.work - before.work, 3);
    let mut tabs = p.clone();
    Arc::make_mut(&mut tabs.slides[0]).notes.paragraphs[0].props.tabs = Some(vec![TabStop { pos: 72., align: "left".into() }]);
    let after = Inventory::inspect(&tabs, id, LIMITS).unwrap().usage;
    assert_eq!(after.copied - before.copied, size_of::<TabStop>() + 4);
    let mut media = p.clone();
    media.media[0].data = Arc::new(vec![0; 1024 * 1024]);
    let after = Inventory::inspect(&media, id, LIMITS).unwrap().usage;
    assert_eq!(after.copied, before.copied);
    assert_eq!(after.temporary, before.temporary);
    let shared = media.media[0].data.clone();
    media.copy_slide(id).unwrap();
    assert!(Arc::ptr_eq(&shared, &media.media[0].data));
    let mut strings = p.clone();
    Arc::make_mut(&mut strings.slides[0]).notes.paragraphs[0].runs[0].text = "x".repeat(2_000_001);
    atomic(strings, Error::ResourceLimit("source work"));
}

#[test]
fn slide_copy_exact_resource_boundaries_and_preflight_rollback() {
    let p = fixture();
    let source = p.slides[0].id;
    let report = Inventory::inspect(&p, source, LIMITS).unwrap();
    let exact = Limits {
        slides: p.slides.len() + 1,
        work: report.usage.work,
        copied: report.usage.copied,
        temporary: report.usage.temporary,
        shapes: report.usage.shapes,
        ..LIMITS
    };
    p.clone().copy_slide_with_limits(source, exact).unwrap();
    for (name, limits) in [
        ("slide count", Limits { slides: exact.slides - 1, ..exact }),
        ("source work", Limits { work: exact.work - 1, ..exact }),
        ("copied payload", Limits { copied: exact.copied - 1, ..exact }),
        ("temporary payload", Limits { temporary: exact.temporary - 1, ..exact }),
        ("global shape count", Limits { shapes: exact.shapes - 1, ..exact }),
    ] {
        let mut candidate = p.clone();
        assert_eq!(candidate.copy_slide_with_limits(source, limits), Err(Error::ResourceLimit(name)));
        assert_eq!(candidate, p);
    }
    let mut deep = fixture();
    let mut shape = Shape { id: ShapeId(100_000), ..Default::default() };
    for depth in 0..33 {
        shape =
            Shape { id: ShapeId(100_001 + depth), kind: ShapeKind::Group { children: vec![shape], child: Xfrm::default() }, ..Default::default() };
    }
    Arc::make_mut(&mut deep.slides[0]).shapes = vec![shape];
    Arc::make_mut(&mut deep.slides[0]).animations.clear();
    atomic(deep, Error::ResourceLimit("group depth"));
    let mut depth_exact = fixture();
    let mut shape = Shape { id: ShapeId(200_000), ..Default::default() };
    for depth in 0..32 {
        shape =
            Shape { id: ShapeId(200_001 + depth), kind: ShapeKind::Group { children: vec![shape], child: Xfrm::default() }, ..Default::default() };
    }
    Arc::make_mut(&mut depth_exact.slides[0]).shapes = vec![shape];
    Arc::make_mut(&mut depth_exact.slides[0]).animations.clear();
    let source = depth_exact.slides[0].id;
    depth_exact.copy_slide(source).unwrap();
    for count in [128, 129] {
        let mut p = fixture();
        let source = p.slides[0].id;
        Arc::make_mut(&mut p.slides[0]).shapes = (0..count).map(|n| Shape { id: ShapeId(300_000 + n), ..Default::default() }).collect();
        Arc::make_mut(&mut p.slides[0]).animations.clear();
        let before = p.clone();
        let result = p.copy_slide_with_limits(source, Limits { tree_shapes: 128, ..LIMITS });
        if count == 128 {
            result.unwrap();
        } else {
            assert_eq!(result, Err(Error::ResourceLimit("shape tree count")));
            assert_eq!(p, before);
        }
    }
    let mut levels = fixture();
    levels.default_text_style.levels.resize(10, None);
    atomic(levels, Error::Unsupported("more than9 text list levels"));
    let mut tabs = fixture();
    Arc::make_mut(&mut tabs.slides[0]).notes.paragraphs[0].props.tabs = Some(vec![TabStop { pos: 0., align: "l".into() }; 33]);
    atomic(tabs, Error::Unsupported("more than32 paragraph tab stops"));
}
