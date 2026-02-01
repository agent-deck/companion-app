# Glyph Atlas Performance Optimization Research

## Executive Summary

This document summarizes research into implementing a glyph atlas for terminal rendering performance optimization, based on analysis of both wezterm's implementation and the current agentdeck codebase.

---

## Current Implementation Analysis

### How Glyphs Are Currently Rendered

**Location:** `src/window/glyph_cache.rs` + `src/window/terminal.rs`

1. **Per-glyph textures**: Each unique glyph gets its own egui `TextureHandle`
2. **HashMap cache**: `GlyphKey(text, style) → CachedGlyph` lookup
3. **On cache miss**: WezTerm's FreeType+HarfBuzz rasterizes glyph → creates individual GPU texture
4. **Render loop**: ~4800 `painter.image()` calls per frame for full terminal

### Identified Bottlenecks

| Bottleneck | Impact | Location |
|------------|--------|----------|
| Per-character texture lookup | 4800+ HashMap lookups/frame | terminal.rs:1643 |
| Individual GPU textures | Fragmented VRAM, no batching | glyph_cache.rs:308-372 |
| Per-glyph painter calls | 4800+ draw calls/frame | terminal.rs:1626-1727 |
| Rasterization on miss | 1-5ms per new glyph | glyph_cache.rs:254-305 |

---

## Wezterm's Glyph Atlas Approach

### Key Components

1. **Texture Atlas** (`window/src/bitmaps/atlas.rs`)
   - Uses `guillotiere` crate for 2D bin packing
   - Power-of-2 sizing (2048x2048, 4096x4096)
   - 1px padding between sprites to avoid filtering artifacts
   - Single GPU texture holds hundreds of glyphs

2. **Sprite-Based Caching** (`wezterm-gui/src/glyphcache.rs`)
   - `HashMap<GlyphKey, Rc<CachedGlyph>>` with borrowed key pattern for zero-copy lookups
   - Each `CachedGlyph` stores `Sprite` (atlas coords) + metrics
   - Separate caches for line glyphs, block glyphs, cursors, colors

3. **Quad-Based Rendering** (`wezterm-gui/src/quad.rs`)
   - 4 vertices per glyph (position + UV + color + metadata)
   - Batched into vertex buffers, single draw call per layer
   - 3 render layers: background/text, selection/underline, cursor

4. **Dynamic Atlas Growth**
   - On `OutOfTextureSpace`: recreate atlas at 2x size
   - Graceful degradation: scale down images if atlas stays full
   - All glyphs re-rasterized on atlas rebuild

5. **Shader-Based Rendering** (`glyph-frag.glsl`)
   - Dual samplers: nearest for text crispness, linear for images
   - HSV transforms in shader for theming
   - Monochrome glyphs use alpha mask + foreground tinting

---

## Proposed Implementation for Agentdeck

### Phase 1: Atlas Infrastructure

**New file:** `src/window/atlas.rs`

```rust
pub struct Atlas {
    texture: egui::TextureHandle,
    allocator: guillotiere::SimpleAtlasAllocator,
    size: usize,  // Power of 2 (512, 1024, 2048)
}

pub struct Sprite {
    pub atlas_id: usize,
    pub uv: egui::Rect,  // Normalized UV coordinates
}
```

**Changes to `glyph_cache.rs`:**
- Replace `TextureHandle` per glyph → `Sprite` with atlas coordinates
- Add atlas allocation on cache miss
- Handle atlas overflow (grow or create new atlas)

### Phase 2: Batch Rendering

**Changes to `terminal.rs`:**

Instead of:
```rust
for cell in cells_to_render {
    painter.image(glyph.texture.id(), rect, uv, color);
}
```

Use:
```rust
let mut mesh = egui::Mesh::with_texture(atlas.texture_id());
for cell in cells_to_render {
    let sprite = cache.get_sprite(...);
    mesh.add_rect_with_uv(rect, sprite.uv, color);
}
painter.add(Shape::mesh(mesh));
```

### Phase 3: Pre-Population

- Pre-rasterize ASCII 32-126 at startup
- Pre-rasterize common Unicode (box drawing, arrows)
- Background thread for speculative rasterization

---

## Expected Performance Gains

| Metric | Current | With Atlas |
|--------|---------|------------|
| GPU textures | 4800+ | 1-4 atlases |
| Draw calls/frame | 4800+ | ~10-50 |
| VRAM usage | Fragmented | Consolidated |
| Frame time | Baseline | -30-50% |

---

## Dependencies to Add

```toml
guillotiere = "0.6"  # 2D bin packing for atlas allocation
```

---

## Files to Modify

1. **`src/window/glyph_cache.rs`** - Major refactor for atlas-based sprites
2. **`src/window/terminal.rs`** - Batch rendering with mesh instead of per-glyph calls
3. **`src/window/mod.rs`** - Export new atlas module
4. **`Cargo.toml`** - Add guillotiere dependency

---

## Verification Plan

1. Visual correctness: All glyphs render at correct positions with proper colors
2. Performance: Measure frame time before/after with large terminal output
3. Memory: Monitor VRAM usage with GPU profiler
4. Edge cases: Color emoji, wide characters, ligatures, box drawing

---

## Complexity Assessment

**Estimated effort:** Medium-High (2-3 focused sessions)

**Risks:**
- UV coordinate precision issues at atlas boundaries
- Emoji/color glyph handling complexity
- Atlas rebuild latency during heavy use

**Alternative:** If full atlas is too complex, a simpler "texture page" approach (fixed grid of glyphs) could provide 50-70% of the benefit with less complexity.
