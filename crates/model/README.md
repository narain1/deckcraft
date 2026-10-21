# Presentation model

`Presentation::copy_slide(source)` appends a bounded copy inside the same presentation and returns its fresh `SlideId`. It copies the selected slide's modeled values, gives every copied shape a fresh identity, and remaps local connector endpoints and animation targets/triggers. Existing slide and shape identities do not change. Media bytes, masters and layouts remain shared.

Fixed slide hyperlinks keep their targets. In particular, a self-link on the source continues to point to the source after copying. The same policy applies to notes and inherited layout/master text. Numeric model IDs can normalize during PPTX serialization; relationships identify the reopened target objects. Cross-presentation copying and editor selection/history are separate operations.

## Supported model profile

This is an ordinary modeled-presentation operation, not a general opaque-content copier. Shape, picture, group, connector and table values, notes, backgrounds and modeled formatting are admitted. Existing rendering, text shaping and serialization limitations remain; copying does not introduce a new layout algorithm or guarantee that content fits.

Before copying, the operation rejects these profiles anywhere in the presentation, including shared masters, layouts and notes styles:

- Sections, custom shows, nondefault show settings, transitions and comments.
- Raw XML/extensions/tags, kept package parts, SmartArt, charts, media players, ink, 3-D models, math XML, artistic picture effects and raw table-style definitions.
- Relative/custom-show/program/media actions, sound actions, zoom-return actions and externally linked media.
- Missing or ambiguous IDs/references, nonfinite inspected geometry/formatting values, more than nine list levels or more than 32 paragraph tab stops.

URLs and fixed slide links are retained. Existing per-slide shape-ID reuse in different shape trees is allowed; newly allocated identities exceed all existing slide, shape, master, layout and media IDs, including notes and handout masters. Zero and one are reserved for new allocation but do not automatically invalidate an existing native namespace. Connector and animation shape references must be local to their owning tree. No invalid reference is silently detached or removed.

## Limits and failures

The resulting presentation has at most 10,000 slides and 100,000 shapes, with at most 20,000 shapes per tree and group depth 32 (root depth zero). Preflight allows at most two million inspected-item/string-byte work units, 64 MiB of selected-slide clone payload, and 16 MiB of temporary collection payload. The latter counts bounded reservations cumulatively, including the destination slide list; it is a conservative policy rather than a measured peak-heap claim. Inline descriptors are charged once, with dynamic strings/vectors charged separately. Shared media bytes are not cloned or decoded.

All checked errors leave presentation values, ordering and the live ID allocator unchanged. The operation inventories the source and inherited references, reserves IDs locally, clones only the selected slide, and stages all remapping before committing. A failed reservation can change a vector's spare capacity; semantic state remains unchanged. Process-wide allocation aborts are not recoverable through `Result`.

The lower-level implementation follows the native editor's re-ID/remapping algorithm but refuses unsupported depth and references instead of silently truncating or dropping them. Existing editor commands are unchanged; this does not claim compatibility with their multi-selection, undo or organization policies.
