# Madori (間取り) — GPU App Framework

> **★★★ CSE / Knowable Construction.** This repo operates under **Constructive Substrate Engineering** — canonical specification at [`pleme-io/theory/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md`](https://github.com/pleme-io/theory/blob/main/CONSTRUCTIVE-SUBSTRATE-ENGINEERING.md). The Compounding Directive (operational rules: solve once, load-bearing fixes only, idiom-first, models stay current, direction beats velocity) is in the org-level pleme-io/CLAUDE.md ★★★ section. Read both before non-trivial changes.


## Build & Test

```bash
cargo build
cargo test --lib
```

## Architecture

Application shell that wraps garasu + winit into a ready-to-use event loop, render loop,
and input dispatch system. Eliminates ~200 lines of identical boilerplate per GPU app.

### Modules

| Module | Purpose |
|--------|---------|
| `app.rs` | `App`, `AppBuilder`, `AppConfig` — fluent builder, window creation, event loop |
| `event.rs` | `AppEvent`, `KeyEvent`, `MouseEvent`, `KeyCode`, `Modifiers` — platform-independent input |
| `render.rs` | `RenderCallback` trait, `RenderContext` (gpu, text, surface_view, elapsed, dt) |
| `error.rs` | `MadoriError` — event loop and GPU init failures |

### Layer Position

```
Application code (mado, hibiki, kagi, ...)
       ↓
madori (event loop, render loop, input dispatch)
       ↓
garasu (GpuContext, TextRenderer)
       ↓
wgpu + winit + glyphon
```

### Consumers

Used by: mado, hibiki, kagi, kekkai, fumi, nami

## Design Decisions

- **Builder pattern**: `App::builder(renderer).title("...").size(w,h).on_event(handler).run()`
- **RenderCallback trait**: apps implement `render()`, `resize()`, `init()` — madori owns the loop
- **Platform-independent input**: `KeyCode::from_winit()` maps winit keys to abstract codes
- **ClearRenderer**: built-in no-op renderer for testing (clears to Nord background)
- **Does NOT own GPU internals** — delegates to garasu for context, text, shaders
