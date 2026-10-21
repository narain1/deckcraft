use std::mem::size_of;

use super::{CopySlideError as Error, Limits, Result, ShapeInfo, Usage};
use crate::{
    style::*,
    table::{Row, TableBorder, TablePartStyle, TableStyleDef},
    text::*,
    theme::*,
    *,
};

pub(super) struct Plan {
    pub source_index: usize,
    pub high_water: u32,
    pub ids: Vec<ShapeInfo>,
    #[cfg(test)]
    pub usage: Usage,
}

pub(super) struct Inventory {
    limits: Limits,
    usage: Usage,
    copying: bool,
    high_water: u32,
    slides: Vec<u32>,
    layouts: Vec<u32>,
    media: Vec<u32>,
}

fn finite(values: &[f64]) -> Result<()> {
    if values.iter().all(|x| x.is_finite()) { Ok(()) } else { Err(Error::Invalid("nonfinite scalar")) }
}
fn option_finite<const N: usize>(values: [Option<f64>; N]) -> Result<()> {
    if values.into_iter().flatten().all(f64::is_finite) { Ok(()) } else { Err(Error::Invalid("nonfinite scalar")) }
}
fn unsupported(condition: bool, message: &'static str) -> Result<()> {
    if condition { Err(Error::Unsupported(message)) } else { Ok(()) }
}

impl Inventory {
    pub(super) fn inspect(doc: &Presentation, source: SlideId, limits: Limits) -> Result<Plan> {
        let Presentation {
            slide_size,
            notes_size,
            slides,
            masters,
            notes_master,
            handout_master,
            sections,
            custom_shows,
            show,
            header_footer: _,
            props: _,
            default_text_style,
            media,
            first_slide_number: _,
            next_id,
            embedded_fonts,
            table_styles,
            tags,
            kept_parts,
            guides,
            raw_ext,
        } = doc;
        let mut scan =
            Self { limits, usage: Usage::default(), copying: false, high_water: (*next_id).max(1), slides: vec![], layouts: vec![], media: vec![] };
        scan.collection(slides)?;
        if slides.len() >= limits.slides {
            return Err(Error::ResourceLimit("slide count"));
        }
        let source_index = slides.iter().position(|s| s.id == source).ok_or(Error::NoSlide)?;
        finite(&[slide_size.width, slide_size.height, notes_size.width, notes_size.height])?;
        if [slide_size.width, slide_size.height, notes_size.width, notes_size.height].iter().any(|n| *n <= 0.0) {
            return Err(Error::Invalid("nonpositive page size"));
        }
        unsupported(!sections.is_empty() || !custom_shows.is_empty(), "sections or custom shows")?;
        let ShowSettings {
            show_type,
            loop_until_esc,
            without_narration,
            without_animation,
            use_timings,
            range,
            custom_show,
            pen_color,
            laser_color,
            show_media_controls,
        } = show;
        scan.string(show_type)?;
        if let Some(name) = custom_show {
            scan.string(name)?;
        }
        unsupported(
            show_type != "speaker"
                || *loop_until_esc
                || *without_narration
                || *without_animation
                || !*use_timings
                || range.is_some()
                || custom_show.is_some()
                || *pen_color != Rgba::rgb(255, 0, 0)
                || *laser_color != Rgba::rgb(255, 0, 0)
                || !*show_media_controls,
            "nondefault show settings",
        )?;
        unsupported(!tags.is_empty() || !kept_parts.is_empty() || raw_ext.is_some(), "presentation opaque data")?;
        scan.collection(guides)?;
        for DrawGuide { vertical: _, pos } in guides {
            finite(&[*pos])?;
        }
        let mut slide_ids = scan.temporary::<u32>(slides.len())?;
        for slide in slides {
            scan.id(&mut slide_ids, slide.id.0);
        }
        scan.unique(&mut slide_ids)?;
        scan.slides = slide_ids;
        scan.collection(media)?;
        let mut media_ids = scan.temporary::<u32>(media.len())?;
        for MediaItem { id, name: _, content_type: _, data: _, link } in media {
            unsupported(link.is_some(), "externally linked media")?;
            scan.id(&mut media_ids, id.0);
        }
        scan.unique(&mut media_ids)?;
        scan.media = media_ids;
        scan.collection(masters)?;
        let extra = usize::from(notes_master.is_some()) + usize::from(handout_master.is_some());
        let mut master_ids = scan.temporary::<u32>(masters.len().checked_add(extra).ok_or(Error::ResourceLimit("masters"))?)?;
        let mut layout_count = 0usize;
        for master in masters.iter().map(AsRef::as_ref).chain([notes_master.as_deref(), handout_master.as_deref()].into_iter().flatten()) {
            scan.id(&mut master_ids, master.id.0);
            scan.collection(&master.layouts)?;
            layout_count = layout_count.checked_add(master.layouts.len()).ok_or(Error::ResourceLimit("layouts"))?;
        }
        scan.unique(&mut master_ids)?;
        let mut layout_ids = scan.temporary::<u32>(layout_count)?;
        for master in masters.iter().map(AsRef::as_ref).chain([notes_master.as_deref(), handout_master.as_deref()].into_iter().flatten()) {
            for layout in &master.layouts {
                scan.id(&mut layout_ids, layout.id.0);
            }
        }
        scan.unique(&mut layout_ids)?;
        scan.layouts = layout_ids;
        // A slide layout must belong to an ordinary master: native Presentation::layout does not
        // search notes/handout masters. Their IDs are nevertheless globally reserved above.
        for slide in slides {
            scan.usage.work(32, limits)?;
            scan.lookup(&scan.layouts, slide.layout.0)?;
            let mut found = false;
            for master in masters {
                scan.usage.work(master.layouts.len(), limits)?;
                if master.layouts.iter().any(|l| l.id == slide.layout) {
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(Error::Invalid("slide layout belongs to no slide master"));
            }
        }
        scan.list(default_text_style)?;
        scan.collection(embedded_fonts)?;
        for EmbeddedFont { typeface: _, panose: _, pitch_family: _, charset: _, regular, bold, italic, bold_italic } in embedded_fonts {
            for id in [regular, bold, italic, bold_italic].into_iter().flatten() {
                scan.media_ref(*id)?;
            }
        }
        scan.collection(table_styles)?;
        for TableStyleDef { id: _, name: _, parts, background, raw } in table_styles {
            unsupported(!raw.is_empty(), "raw table style XML")?;
            scan.collection(parts)?;
            for (_, style) in parts {
                scan.table_style(style)?;
            }
            if let Some(style) = background {
                scan.table_style(style)?;
            }
        }
        for master in masters.iter().map(AsRef::as_ref).chain([notes_master.as_deref(), handout_master.as_deref()].into_iter().flatten()) {
            scan.master(master)?;
        }
        let mut selected = None;
        for (index, slide) in slides.iter().enumerate() {
            scan.copying = index == source_index;
            if scan.copying {
                scan.usage.copied(size_of::<Slide>(), limits)?;
            }
            let ids = scan.slide(slide)?;
            if scan.copying {
                selected = Some(ids);
            }
        }
        let selected = selected.ok_or(Error::NoSlide)?;
        Usage::add(&mut scan.usage.shapes, selected.len(), limits.shapes, "global shape count")?;
        scan.usage.temporary::<std::sync::Arc<Slide>>(slides.len() + 1, limits)?;
        Ok(Plan {
            source_index,
            high_water: scan.high_water,
            ids: selected,
            #[cfg(test)]
            usage: scan.usage,
        })
    }

    fn collection<T>(&mut self, values: &[T]) -> Result<()> {
        self.usage.work(values.len(), self.limits)?;
        if self.copying {
            self.usage.copied(values.len().checked_mul(size_of::<T>()).ok_or(Error::ResourceLimit("copied payload"))?, self.limits)?;
        }
        Ok(())
    }
    fn string(&mut self, value: &str) -> Result<()> {
        self.usage.work(value.len(), self.limits)?;
        if self.copying {
            self.usage.copied(value.len(), self.limits)?;
        }
        Ok(())
    }
    fn strings(&mut self, values: &[Option<&String>]) -> Result<()> {
        for value in values.iter().flatten() {
            self.string(value)?;
        }
        Ok(())
    }
    fn temporary<T>(&mut self, n: usize) -> Result<Vec<T>> {
        self.usage.temporary::<T>(n, self.limits)?;
        let mut out = Vec::new();
        out.try_reserve_exact(n).map_err(|_| Error::Allocation)?;
        Ok(out)
    }
    fn id(&mut self, ids: &mut Vec<u32>, id: u32) {
        self.high_water = self.high_water.max(id);
        ids.push(id);
    }
    fn sort_work(&mut self, count: usize) -> Result<()> {
        let log = usize::BITS - count.leading_zeros();
        self.usage.work(count.checked_mul(log as usize).ok_or(Error::ResourceLimit("sorting work"))?, self.limits)
    }
    fn unique(&mut self, ids: &mut [u32]) -> Result<()> {
        self.sort_work(ids.len())?;
        ids.sort_unstable();
        if ids.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(Error::Invalid("duplicate object ID"));
        }
        Ok(())
    }
    fn lookup(&self, ids: &[u32], id: u32) -> Result<()> {
        // Numeric binary searches have at most32 comparisons; callers precharge each reference.
        if ids.binary_search(&id).is_ok() { Ok(()) } else { Err(Error::Invalid("missing object reference")) }
    }
    fn media_ref(&mut self, id: MediaId) -> Result<()> {
        self.usage.work(32, self.limits)?;
        self.lookup(&self.media, id.0)
    }
    fn local(&mut self, ids: &[ShapeInfo], id: ShapeId) -> Result<Option<usize>> {
        self.usage.work(if self.copying { 64 } else { 32 }, self.limits)?;
        ids.binary_search_by_key(&id.0, |s| s.id)
            .ok()
            .and_then(|i| ids.get(i))
            .map(|s| s.paragraphs)
            .ok_or(Error::Invalid("missing local shape reference"))
    }
    fn color_map(&mut self, values: &[(String, String)]) -> Result<()> {
        self.collection(values)?;
        for (a, b) in values {
            self.string(a)?;
            self.string(b)?;
        }
        Ok(())
    }
    fn count_shapes(&mut self, shapes: &[Shape], depth: usize, count: &mut usize) -> Result<()> {
        if depth > self.limits.depth {
            return Err(Error::ResourceLimit("group depth"));
        }
        self.usage.work(shapes.len(), self.limits)?;
        Usage::add(&mut self.usage.shapes, shapes.len(), self.limits.shapes, "global shape count")?;
        Usage::add(count, shapes.len(), self.limits.tree_shapes, "shape tree count")?;
        for shape in shapes {
            if let ShapeKind::Group { children, child: _ } = &shape.kind {
                self.count_shapes(children, depth + 1, count)?;
            }
        }
        Ok(())
    }
    fn collect_shapes(&mut self, shapes: &[Shape], ids: &mut Vec<ShapeInfo>) {
        // Count/depth and the exact single allocation were checked before this second traversal.
        for shape in shapes {
            self.high_water = self.high_water.max(shape.id.0);
            ids.push(ShapeInfo { id: shape.id.0, new: 0, paragraphs: shape.text.as_ref().map(|t| t.paragraphs.len()) });
            if let ShapeKind::Group { children, child: _ } = &shape.kind {
                self.collect_shapes(children, ids);
            }
        }
    }
    fn tree(&mut self, shapes: &[Shape]) -> Result<Vec<ShapeInfo>> {
        let mut count = 0;
        self.count_shapes(shapes, 0, &mut count)?;
        // Charge ID collection and the later selected-tree remap before allocating/copying.
        self.usage.work(count.checked_mul(if self.copying { 34 } else { 1 }).ok_or(Error::ResourceLimit("shape work"))?, self.limits)?;
        let mut ids = self.temporary::<ShapeInfo>(count)?;
        self.collect_shapes(shapes, &mut ids);
        self.sort_work(ids.len())?;
        ids.sort_unstable_by_key(|s| s.id);
        if ids.windows(2).any(|p| p[0].id == p[1].id) {
            return Err(Error::Invalid("duplicate shape ID in tree"));
        }
        self.shapes(shapes, &ids)?;
        Ok(ids)
    }
    fn slide(&mut self, slide: &Slide) -> Result<Vec<ShapeInfo>> {
        let Slide {
            id: _,
            layout: _,
            name,
            shapes,
            background,
            hidden: _,
            show_master_shapes: _,
            transition,
            animations,
            notes,
            comments,
            color_map,
            tags,
            raw_ext,
        } = slide;
        unsupported(transition.is_some(), "transitions")?;
        unsupported(!comments.is_empty() || !tags.is_empty() || raw_ext.is_some(), "slide comments or opaque data")?;
        self.string(name)?;
        if let Some(bg) = background {
            self.background(bg)?;
        }
        if let Some(map) = color_map {
            self.color_map(map)?;
        }
        self.text(notes)?;
        let ids = self.tree(shapes)?;
        self.collection(animations)?;
        for a in animations {
            self.animation(a, &ids)?;
        }
        Ok(ids)
    }
    fn animation(&mut self, a: &Animation, ids: &[ShapeInfo]) -> Result<()> {
        let Animation {
            shape,
            class: _,
            effect,
            option,
            start: _,
            duration_ms: _,
            delay_ms: _,
            repeat: _,
            rewind: _,
            text_build: _,
            paragraph,
            path,
            trigger,
            amount,
            color,
            preset_id: _,
            preset_subtype: _,
            after,
            sound,
            smooth_start,
            smooth_end,
            bounce_end,
            auto_reverse: _,
        } = a;
        unsupported(sound.is_some(), "animation sounds")?;
        self.string(effect)?;
        self.string(option)?;
        self.strings(&[path.as_ref(), after.as_ref()])?;
        let paragraphs = self.local(ids, *shape)?;
        if paragraph.is_some_and(|p| paragraphs.is_none_or(|n| p as usize >= n)) {
            return Err(Error::Invalid("animation paragraph"));
        }
        if let Some(id) = trigger {
            self.local(ids, *id)?;
        }
        option_finite([*amount])?;
        finite(&[*smooth_start, *smooth_end, *bounce_end])?;
        if let Some(c) = color {
            self.color(c)?;
        }
        Ok(())
    }
    fn master(&mut self, master: &Master) -> Result<()> {
        let Master { id: _, name: _, theme, shapes, background, title_style, body_style, other_style, layouts, color_map, preserve: _, raw_ext } =
            master;
        unsupported(raw_ext.is_some(), "master raw XML")?;
        self.tree(shapes)?;
        self.theme(theme)?;
        if let Some(bg) = background {
            self.background(bg)?;
        }
        self.list(title_style)?;
        self.list(body_style)?;
        self.list(other_style)?;
        self.color_map(color_map)?;
        for Layout { id: _, name: _, kind: _, shapes, background, show_master_shapes: _, preserve: _, color_map, raw_ext } in layouts {
            unsupported(raw_ext.is_some(), "layout raw XML")?;
            self.tree(shapes)?;
            if let Some(bg) = background {
                self.background(bg)?;
            }
            if let Some(map) = color_map {
                self.color_map(map)?;
            }
        }
        Ok(())
    }
    fn theme(&mut self, theme: &Theme) -> Result<()> {
        let Theme { name: _, colors: _, fonts: _, format, raw_extra } = theme;
        unsupported(raw_extra.is_some(), "theme raw XML")?;
        let FormatScheme { name: _, fills, lines, effects, bg_fills } = format;
        for values in [fills, bg_fills] {
            self.collection(values)?;
            for fill in values {
                self.fill(fill)?;
            }
        }
        self.collection(lines)?;
        for line in lines {
            self.line(line)?;
        }
        self.collection(effects)?;
        for effect in effects {
            self.effects(effect)?;
        }
        Ok(())
    }
    fn table_style(&mut self, value: &TablePartStyle) -> Result<()> {
        let TablePartStyle { fill, fill_ref, borders, text_color, bold: _, italic: _ } = value;
        if let Some(f) = fill {
            self.fill(f)?;
        }
        if let Some((_, c)) = fill_ref {
            self.color(c)?;
        }
        if let Some(c) = text_color {
            self.color(c)?;
        }
        for border in borders.iter().flatten() {
            match border {
                TableBorder::Line { line } => self.line(line)?,
                TableBorder::Ref { idx: _, color } => self.color(color)?,
            }
        }
        Ok(())
    }
    fn shapes(&mut self, shapes: &[Shape], ids: &[ShapeInfo]) -> Result<()> {
        self.collection(shapes)?;
        for shape in shapes {
            let Shape {
                id: _,
                name,
                xfrm,
                kind,
                geom,
                ph,
                fill,
                line,
                effects,
                style,
                text,
                hidden: _,
                locked: _,
                text_box: _,
                descr,
                title,
                decorative: _,
                click,
                hover,
                tags,
                raw_ext,
                diagram,
            } = shape;
            unsupported(!tags.is_empty() || raw_ext.is_some() || diagram.is_some(), "shape opaque data or SmartArt")?;
            self.string(name)?;
            self.string(descr)?;
            self.string(title)?;
            if let Some(x) = xfrm {
                Self::xfrm(x)?;
            }
            self.geom(geom)?;
            if let Some(Placeholder { kind: _, idx: _, vertical: _, size, has_custom_prompt: _ }) = ph {
                self.strings(&[size.as_ref()])?;
            }
            if let Some(f) = fill {
                self.fill(f)?;
            }
            if let Some(l) = line {
                self.line(l)?;
            }
            if let Some(e) = effects {
                self.effects(e)?;
            }
            if let Some(ShapeStyle { line_ref, fill_ref, effect_ref, font_ref }) = style {
                for (_, c) in [line_ref, fill_ref, effect_ref] {
                    self.color(c)?;
                }
                self.string(&font_ref.0)?;
                if let Some(c) = &font_ref.1 {
                    self.color(c)?;
                }
            }
            if let Some(t) = text {
                self.text(t)?;
            }
            for link in [click, hover].into_iter().flatten() {
                self.link(link)?;
            }
            match kind {
                ShapeKind::Shape => {}
                ShapeKind::Picture { fill } => self.picture(fill)?,
                ShapeKind::Group { children, child } => {
                    Self::xfrm(child)?;
                    self.shapes(children, ids)?;
                }
                ShapeKind::Connector { start, end } => {
                    for (id, _) in [start, end].into_iter().flatten() {
                        self.local(ids, *id)?;
                    }
                }
                ShapeKind::Table(table) => self.table(table)?,
                ShapeKind::Chart(_)
                | ShapeKind::Media(_)
                | ShapeKind::Ink { strokes: _ }
                | ShapeKind::Model3d { media: _, poster: _ }
                | ShapeKind::Opaque { xml: _, preview: _, label: _, parts: _ } => {
                    return Err(Error::Unsupported("shape kind"));
                }
            }
        }
        Ok(())
    }
    fn xfrm(value: &Xfrm) -> Result<()> {
        let Xfrm { x, y, w, h, rot, flip_h: _, flip_v: _ } = value;
        finite(&[*x, *y, *w, *h, *rot])
    }
    fn geom(&mut self, geom: &Geom) -> Result<()> {
        match geom {
            Geom::Preset { name, adj } => {
                self.string(name)?;
                self.collection(adj)?;
                finite(adj)?;
            }
            Geom::Custom { paths } => {
                self.collection(paths)?;
                for CustomPath { w, h, d, fill: _, stroke: _ } in paths {
                    finite(&[*w, *h])?;
                    self.string(d)?;
                }
            }
        }
        Ok(())
    }
    fn table(&mut self, table: &Table) -> Result<()> {
        let Table { cols, rows, style, first_row: _, first_col: _, last_row: _, last_col: _, band_row: _, band_col: _, background } = table;
        self.string(style)?;
        self.collection(cols)?;
        finite(cols)?;
        if let Some(f) = background {
            self.fill(f)?;
        }
        self.collection(rows)?;
        for Row { height, cells } in rows {
            finite(&[*height])?;
            self.collection(cells)?;
            for Cell {
                text,
                fill,
                borders,
                diag_down,
                diag_up,
                margins,
                anchor: _,
                grid_span: _,
                row_span: _,
                h_merge: _,
                v_merge: _,
                vertical: _,
            } in cells
            {
                self.text(text)?;
                if let Some(f) = fill {
                    self.fill(f)?;
                }
                for line in borders.iter().chain([diag_down, diag_up]).flatten() {
                    self.line(line)?;
                }
                if let Some(m) = margins {
                    finite(m)?;
                }
            }
        }
        Ok(())
    }
    fn text(&mut self, text: &TextBody) -> Result<()> {
        let TextBody { body: props, list_style, paragraphs } = text;
        let BodyProps {
            inset_l,
            inset_t,
            inset_r,
            inset_b,
            anchor: _,
            anchor_ctr: _,
            wrap: _,
            autofit,
            vert: _,
            rot,
            columns: _,
            col_spacing,
            warp,
            warp_adj,
            rtl_col: _,
            upright: _,
            space_first_last: _,
        } = props;
        option_finite([*inset_l, *inset_t, *inset_r, *inset_b, *rot, *col_spacing])?;
        if let Some(fit) = autofit {
            match fit {
                AutoFit::None | AutoFit::Shape => {}
                AutoFit::Shrink { font_scale, line_reduction } => {
                    option_finite([*font_scale])?;
                    finite(&[*line_reduction])?;
                }
            }
        }
        self.strings(&[warp.as_ref()])?;
        self.collection(warp_adj)?;
        finite(warp_adj)?;
        self.list(list_style)?;
        self.collection(paragraphs)?;
        for Paragraph { level: _, props, runs, def_props, end_props } in paragraphs {
            self.para(props)?;
            self.run_props(def_props)?;
            self.run_props(end_props)?;
            self.collection(runs)?;
            for Run { text, props, kind } in runs {
                self.string(text)?;
                self.run_props(props)?;
                match kind {
                    RunKind::Text | RunKind::Break => {}
                    RunKind::Field { field } => self.string(field)?,
                    RunKind::Math { omml: _ } => return Err(Error::Unsupported("math XML")),
                }
            }
        }
        Ok(())
    }
    fn list(&mut self, list: &ListStyle) -> Result<()> {
        let ListStyle { levels } = list;
        self.collection(levels)?;
        unsupported(levels.len() > 9, "more than9 text list levels")?;
        for LevelStyle { para, run } in levels.iter().flatten() {
            self.para(para)?;
            self.run_props(run)?;
        }
        Ok(())
    }
    fn para(&mut self, props: &ParaProps) -> Result<()> {
        let ParaProps {
            align: _,
            margin_left,
            margin_right,
            indent,
            line_spacing,
            space_before,
            space_after,
            bullet,
            bullet_font,
            bullet_color,
            bullet_size,
            tabs,
            default_tab,
            rtl: _,
            font_align,
            east_asian_line_break: _,
            latin_line_break: _,
            hanging_punct: _,
        } = props;
        option_finite([*margin_left, *margin_right, *indent, *bullet_size, *default_tab])?;
        for spacing in [line_spacing, space_before, space_after].into_iter().flatten() {
            match spacing {
                Spacing::Pct(n) | Spacing::Pts(n) => finite(&[*n])?,
            }
        }
        self.strings(&[bullet_font.as_ref(), font_align.as_ref()])?;
        if let Some(c) = bullet_color {
            self.color(c)?;
        }
        if let Some(bullet) = bullet {
            match bullet {
                Bullet::None => {}
                Bullet::Char { char } => self.string(char)?,
                Bullet::AutoNum { scheme, start_at: _ } => self.string(scheme)?,
                Bullet::Picture { media } => self.media_ref(*media)?,
            }
        }
        if let Some(tabs) = tabs {
            self.collection(tabs)?;
            unsupported(tabs.len() > 32, "more than32 paragraph tab stops")?;
            for TabStop { pos, align } in tabs {
                finite(&[*pos])?;
                self.string(align)?;
            }
        }
        Ok(())
    }
    fn run_props(&mut self, props: &RunProps) -> Result<()> {
        let RunProps {
            font,
            font_ea,
            font_cs,
            font_sym,
            size,
            bold: _,
            italic: _,
            underline,
            underline_color,
            strike: _,
            baseline,
            caps: _,
            spacing,
            kern,
            fill,
            outline,
            highlight,
            link,
            hover,
            lang,
            shadow,
            inner_shadow,
            glow,
            reflection,
            soft_edge,
            no_proof: _,
        } = props;
        self.strings(&[font.as_ref(), font_ea.as_ref(), font_cs.as_ref(), font_sym.as_ref(), underline.as_ref(), lang.as_ref()])?;
        option_finite([*size, *baseline, *spacing, *kern, *soft_edge])?;
        for c in [underline_color, highlight].into_iter().flatten() {
            self.color(c)?;
        }
        if let Some(f) = fill {
            self.fill(f)?;
        }
        if let Some(l) = outline {
            self.line(l)?;
        }
        for link in [link, hover].into_iter().flatten() {
            self.link(link)?;
        }
        for shadow in [shadow, inner_shadow].into_iter().flatten() {
            self.shadow(shadow)?;
        }
        if let Some(g) = glow {
            self.glow(g)?;
        }
        if let Some(r) = reflection {
            Self::reflection(r)?;
        }
        Ok(())
    }
    fn link(&mut self, link: &Hyperlink) -> Result<()> {
        let Hyperlink { action, tooltip, highlight_click: _, sound, stop_sound, zoom_return } = link;
        unsupported(sound.is_some() || *stop_sound || zoom_return.is_some(), "sound or zoom actions")?;
        self.string(tooltip)?;
        match action {
            Action::Url { url } => self.string(url)?,
            Action::Slide { slide } => {
                self.usage.work(32, self.limits)?;
                self.lookup(&self.slides, slide.0)?;
            }
            Action::NextSlide
            | Action::PreviousSlide
            | Action::FirstSlide
            | Action::LastSlide
            | Action::EndShow
            | Action::LastViewed
            | Action::CustomShow { name: _, returns: _ }
            | Action::Program { path: _ }
            | Action::PlayMedia => {
                return Err(Error::Unsupported("nonfixed hyperlink action"));
            }
        }
        Ok(())
    }
    fn background(&mut self, bg: &Background) -> Result<()> {
        match bg {
            Background::Fill { fill } => self.fill(fill),
            Background::Ref { idx: _, color } => self.color(color),
        }
    }
    fn color(&mut self, c: &ColorRef) -> Result<()> {
        let ColorRef { base, mods } = c;
        fn fixed<T: Copy>(_: &[T]) {}
        fixed(mods); // A future heap-owning color transform requires a new inventory case.
        self.collection(mods)?;
        match base {
            ColorBase::Rgb { rgb: _ } | ColorBase::Scheme { slot: _ } => {}
            ColorBase::Preset { name } | ColorBase::System { name, last: _ } => self.string(name)?,
        }
        Ok(())
    }
    fn fill(&mut self, f: &Fill) -> Result<()> {
        match f {
            Fill::None | Fill::Group | Fill::Background => {}
            Fill::Solid { color } => self.color(color)?,
            Fill::Gradient(Gradient { stops, shape, rotate_with_shape: _, tile }) => {
                finite(tile)?;
                self.collection(stops)?;
                for GradientStop { pos, color } in stops {
                    finite(&[*pos])?;
                    self.color(color)?;
                }
                match shape {
                    GradientShape::Linear { angle, scaled: _ } => finite(&[*angle])?,
                    GradientShape::Path { path, focus } => {
                        self.string(path)?;
                        finite(focus)?;
                    }
                }
            }
            Fill::Picture(p) => self.picture(p)?,
            Fill::Pattern(PatternFill { preset, fg, bg }) => {
                self.string(preset)?;
                self.color(fg)?;
                self.color(bg)?;
            }
        }
        Ok(())
    }
    fn picture(&mut self, p: &PictureFill) -> Result<()> {
        let PictureFill { media, crop, mode, alpha, adjust, upright: _, artistic } = p;
        unsupported(artistic.is_some(), "artistic picture effects")?;
        self.media_ref(*media)?;
        finite(crop)?;
        option_finite([*alpha])?;
        match mode {
            PictureMode::Stretch { fill_rect } => finite(fill_rect)?,
            PictureMode::Tile { tx, ty, sx, sy, flip, align } => {
                finite(&[*tx, *ty, *sx, *sy])?;
                self.string(flip)?;
                self.string(align)?;
            }
        }
        let PictureAdjust { brightness, contrast, saturation, grayscale: _, duotone, clear_color: _, sharpen, bilevel, alpha_inv: _ } = adjust;
        finite(&[*brightness, *contrast, *sharpen])?;
        option_finite([*saturation, *bilevel])?;
        if let Some((a, b)) = duotone {
            self.color(a)?;
            self.color(b)?;
        }
        Ok(())
    }
    fn line(&mut self, line: &Line) -> Result<()> {
        let Line { fill, width, dash, cap: _, join: _, compound: _, head, tail } = line;
        option_finite([*width])?;
        if let Some(f) = fill {
            self.fill(f)?;
        }
        if let Some(dash) = dash {
            match dash {
                Dash::Preset { name } => self.string(name)?,
                Dash::Custom { pattern } => {
                    self.collection(pattern)?;
                    for (a, b) in pattern {
                        finite(&[*a, *b])?;
                    }
                }
            }
        }
        for LineEnd { kind, w, len } in [head, tail].into_iter().flatten() {
            self.string(kind)?;
            self.string(w)?;
            self.string(len)?;
        }
        Ok(())
    }
    fn shadow(&mut self, shadow: &Shadow) -> Result<()> {
        let Shadow { color, blur, dist, dir, inner: _, sx, sy, kx, ky, align, rotate_with_shape: _ } = shadow;
        self.color(color)?;
        self.string(align)?;
        finite(&[*blur, *dist, *dir, *sx, *sy, *kx, *ky])
    }
    fn glow(&mut self, glow: &Glow) -> Result<()> {
        let Glow { color, radius } = glow;
        self.color(color)?;
        finite(&[*radius])
    }
    fn reflection(r: &Reflection) -> Result<()> {
        let Reflection { blur, start_alpha, end_alpha, end_pos, dist } = r;
        finite(&[*blur, *start_alpha, *end_alpha, *end_pos, *dist])
    }
    fn effects(&mut self, effect: &Effects) -> Result<()> {
        let Effects { outer_shadow, inner_shadow, glow, soft_edge, reflection, bevel, raw3d, fill_overlay } = effect;
        unsupported(raw3d.is_some(), "raw3d effects")?;
        option_finite([*soft_edge])?;
        self.strings(&[bevel.as_ref()])?;
        for shadow in [outer_shadow, inner_shadow].into_iter().flatten() {
            self.shadow(shadow)?;
        }
        if let Some(g) = glow {
            self.glow(g)?;
        }
        if let Some(r) = reflection {
            Self::reflection(r)?;
        }
        if let Some(FillOverlay { blend, fill }) = fill_overlay {
            self.string(blend)?;
            self.fill(fill)?;
        }
        Ok(())
    }
}
