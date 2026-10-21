//! Bounded same-presentation copy. The private editor re-ID algorithm is reused here with
//! strict preflight instead of silent deep-tree truncation, detached connectors or lost effects.

use std::{mem::size_of, sync::Arc};

use crate::{Presentation, Shape, ShapeId, ShapeKind, SlideId};

mod inventory;
use inventory::Inventory;

/// An explicit refusal from [`Presentation::copy_slide`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CopySlideError {
    #[error("slide to copy does not exist")]
    NoSlide,
    #[error("slide copy does not support {0}")]
    Unsupported(&'static str),
    #[error("invalid slide copy source: {0}")]
    Invalid(&'static str),
    #[error("slide copy resource limit: {0}")]
    ResourceLimit(&'static str),
    #[error("slide copy object IDs exhausted")]
    IdExhausted,
    #[error("slide copy allocation reservation failed")]
    Allocation,
}

type Result<T> = std::result::Result<T, CopySlideError>;

#[derive(Clone, Copy)]
struct Limits {
    slides: usize,
    tree_shapes: usize,
    shapes: usize,
    depth: usize,
    work: usize,
    copied: usize,
    temporary: usize,
}

const LIMITS: Limits = Limits {
    slides: 10_000,
    tree_shapes: 20_000,
    shapes: 100_000,
    depth: 32,
    work: 2_000_000,
    copied: 64 * 1024 * 1024,
    temporary: 16 * 1024 * 1024,
};

#[derive(Clone, Copy, Debug, Default)]
struct Usage {
    work: usize,
    copied: usize,
    temporary: usize,
    shapes: usize,
}

impl Usage {
    fn add(value: &mut usize, amount: usize, limit: usize, name: &'static str) -> Result<()> {
        *value = value.checked_add(amount).filter(|n| *n <= limit).ok_or(CopySlideError::ResourceLimit(name))?;
        Ok(())
    }
    fn work(&mut self, n: usize, limits: Limits) -> Result<()> {
        Self::add(&mut self.work, n, limits.work, "source work")
    }
    fn copied(&mut self, n: usize, limits: Limits) -> Result<()> {
        Self::add(&mut self.copied, n, limits.copied, "copied payload")
    }
    fn temporary<T>(&mut self, n: usize, limits: Limits) -> Result<()> {
        let bytes = n.checked_mul(size_of::<T>()).ok_or(CopySlideError::ResourceLimit("temporary payload"))?;
        Self::add(&mut self.temporary, bytes, limits.temporary, "temporary payload")
    }
}

impl Presentation {
    /// Append a bounded copy of a slide and return its fresh identity.
    ///
    /// Every copied shape receives a fresh ID; local connectors and animation targets/triggers
    /// follow those IDs. Fixed slide hyperlinks keep their original targets, including self-links
    /// to the source. Media, masters and layouts stay shared. Existing IDs and values do not change.
    ///
    /// This accepts ordinary modeled presentations only: it refuses sections/custom shows,
    /// nondefault show settings, transitions, comments, opaque/XML/SmartArt/chart/media-player/ink/
    /// 3-D content, sound/zoom/relative/program actions, externally linked media and math XML.
    /// Refusals apply throughout the presentation, including inherited layout/master/notes styles.
    /// It is not a cross-deck or arbitrary imported-content copier. Numeric IDs may normalize when
    /// PPTX is serialized; native serialization and rendering retain their existing limitations.
    ///
    /// The final slide count is at most10,000; each shape tree has at most20,000 shapes and depth32
    /// (root depth0), with100,000 total shapes,2M inspected-item/string-byte work,64MiB cloned payload
    /// and16MiB temporary collection payload. All explicit errors preserve model equality and the
    /// allocator. Process-wide allocation aborts are not recoverable through this Result.
    pub fn copy_slide(&mut self, source: SlideId) -> Result<SlideId> {
        self.copy_slide_with_limits(source, LIMITS)
    }

    fn copy_slide_with_limits(&mut self, source: SlideId, limits: Limits) -> Result<SlideId> {
        let plan = Inventory::inspect(self, source, limits)?;
        let count = u32::try_from(plan.ids.len()).map_err(|_| CopySlideError::IdExhausted)?;
        let new_slide = plan.high_water.checked_add(1).ok_or(CopySlideError::IdExhausted)?;
        let last = new_slide.checked_add(count).ok_or(CopySlideError::IdExhausted)?;
        // Inventory owns the charged sorted ID map. Computing replacements is fallible before
        // cloning; no live allocator update occurs. Its entries preserve benign cross-tree reuse.
        let mut map = plan.ids;
        for (index, entry) in map.iter_mut().enumerate() {
            entry.new = new_slide
                .checked_add(u32::try_from(index).map_err(|_| CopySlideError::IdExhausted)?.checked_add(1).ok_or(CopySlideError::IdExhausted)?)
                .ok_or(CopySlideError::IdExhausted)?;
        }
        self.slides.try_reserve_exact(1).map_err(|_| CopySlideError::Allocation)?;
        let original = self.slides.get(plan.source_index).ok_or(CopySlideError::NoSlide)?;
        let mut pending = original.as_ref().clone();
        pending.id = SlideId(new_slide);
        remap(&mut pending.shapes, &map)?;
        for effect in &mut pending.animations {
            effect.shape = ShapeId(mapped(&map, effect.shape.0)?);
            if let Some(trigger) = &mut effect.trigger {
                *trigger = ShapeId(mapped(&map, trigger.0)?);
            }
        }
        self.slides.push(Arc::new(pending));
        self.next_id = last;
        Ok(SlideId(new_slide))
    }
}

#[derive(Clone, Copy)]
struct ShapeInfo {
    id: u32,
    new: u32,
    paragraphs: Option<usize>,
}

fn mapped(map: &[ShapeInfo], id: u32) -> Result<u32> {
    map.binary_search_by_key(&id, |entry| entry.id)
        .ok()
        .and_then(|i| map.get(i))
        .map(|entry| entry.new)
        .ok_or(CopySlideError::Invalid("missing local shape reference"))
}

fn remap(shapes: &mut [Shape], map: &[ShapeInfo]) -> Result<()> {
    // Exact tree depth/count and every endpoint were proven before the selected slide was cloned.
    for shape in shapes {
        shape.id = ShapeId(mapped(map, shape.id.0)?);
        match &mut shape.kind {
            ShapeKind::Group { children, child: _ } => remap(children, map)?,
            ShapeKind::Connector { start, end } => {
                for (id, _) in [start, end].into_iter().flatten() {
                    *id = ShapeId(mapped(map, id.0)?);
                }
            }
            ShapeKind::Shape | ShapeKind::Picture { fill: _ } | ShapeKind::Table(_) => {}
            ShapeKind::Chart(_)
            | ShapeKind::Media(_)
            | ShapeKind::Ink { strokes: _ }
            | ShapeKind::Model3d { media: _, poster: _ }
            | ShapeKind::Opaque { xml: _, preview: _, label: _, parts: _ } => {
                return Err(CopySlideError::Unsupported("shape kind"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
