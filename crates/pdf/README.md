# DeckCraft PDF export

DeckCraft uses `craft-pdf` for writer construction, metadata, fallible font preparation and
final serialization. Slide rasterization, the selectable text overlay, font discovery and
shaping, hyperlinks, bookmarks, notes and handout layout remain app-owned. The existing
renderer also resolves and rasterizes WMF/EMF pictures before export.

Only the shared `write` feature is enabled, with default features disabled. Krilla retains
its raster-image and simple-text features, including notes/footer shaping. PDF embedding is
disabled: DeckCraft does not use it. The production exporter has no Hayro parser, interpreter,
image-decoder, fallback-font or CMap dependencies. Tests use `hayro-syntax` 0.7.2 directly,
with only `std` enabled, to inspect the generated PDFs.

The exporter runs inside a native unwind guard. Writer failures return `PdfError::Write`.
A font that cannot be prepared for embedding now produces an error instead of silently
omitting its selectable text or notes. The shared Krilla 0.8.2 patch avoids a second panic
in page/surface cleanup while unwinding and permanently poisons a document after cleanup
failure, so a retained writer cannot serialize it. Successful serialization paths are
unchanged. Native unwind guards cannot catch WASM aborts or process-wide allocation failures.

## Building and validation

Rust 1.95 or newer is required. The workspace pins `craft-pdf` 0.1.0 and its Krilla 0.8.2
patch to `https://github.com/storytold/craft-libs` revision
`39adbdf756569c8e03adb747d5e063f468ca98c1`. Both packages resolve from that immutable Git source;
a sibling checkout is unnecessary. Publish and review this shared-library prerequisite
before publishing the app migration, so fresh CI and release builds can fetch it.

Run `cargo test -p deckcraft-pdf` for export contracts and `cargo xtask ci` for all workspace
gates. PDF contract tests also run with `wasm-bindgen-test-runner` on
`wasm32-unknown-unknown`. The `pdf-deps` gate verifies the two writer source pins,
retained Krilla image/text features and the development-only parser boundary.

This migration does not add vector slide artwork or change text-shaping/ToUnicode policy.

## Rendering failures

PDF export uses the fallible slide renderer for slide artwork in slide, notes, and handout pages. A native rendering error propagates as `PdfError::Render(RenderError)` through the existing `Result<Vec<u8>>`; the caller receives no successful PDF bytes with replacement blank artwork. Genuinely blank slides still export normally. Existing `NoPages` and `Write` errors retain their contracts.

The new native enum variant is a matching change for downstream callers that exhaustively match `PdfError`. Its display message names slide rendering, and its error source retains the native `RenderError`. This does not promise that all renderer allocation failures or process aborts are recoverable, or change other renderer entry points. The renderer's process-global panic count is not used to decide whether PDF export succeeded.

The focused PDF regression reaches `Busy` through actual nested export inside an outer render callback, checks all three layouts, and verifies outer rendering and later export recovery. The renderer package separately tests caught callback panics; the PDF regression does not claim to inject a caught renderer panic through PDF export. Successful rendering, text, links, notes, and handout algorithms remain unchanged.

## Explicit line breaks and selectable text

The slide text layer and notes/handout master text use the model's logical paragraph
text when mapping layout character ranges. An explicit line-break run contributes
one character even though its stored text field is empty. Keeping that character
in the source projection prevents later searchable text from shifting or losing
characters; rendering geometry and the native shaping/layout algorithms are unchanged.
