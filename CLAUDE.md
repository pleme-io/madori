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
| `render.rs` | `RenderCallback` trait — `frame_demand` answers `FrameDemand::{Idle, Now, At(instant), Continuous}` before any acquire, and its default maps `needs_frame` (`true` → `Now`), so a consumer that overrides neither draws every frame as before — and `RenderContext` (gpu, text, surface_view, elapsed, dt). `text` is `&mut garasu::TextLayerStack` (was `TextRenderer`): the framework owns the stack, the app mints its own per-surface layers — so multi-pass text apps can't clobber one layer's buffer with another's. Single-pass apps use the back-compat `text.prepare`/`text.render` unchanged. |
| `error.rs` | `MadoriError` — event loop and GPU init failures |
| `doorbell.rs` | `Doorbell` — one per loop, owned by `AppBuilder` from `new()`, so a waker exists only with the loop it rings; `AppBuilder::waker` hands out coalescing `std::task::Waker`s over the loop's one `EventLoopProxy`. A ring sends only when its flag goes false → true. `Turnstile::redraw` is the redraw turn — the pacer's `redrawing`, then the flag lowered, then the consumer's dispatch, which is where it drains — and the only code that lowers the flag; the loop and the pacer tests both run it, and so do `Turnstile::shift` and `Turnstile::wait`, which run a drain turn with no frame the moment a window becomes Hidden (an occlusion, a minimize, a Wayland redraw inferred withheld), because the ring that raised the flag may never be answered by a redraw. Loom-modelled (`doorbell::loom_model`) |
| `pacer/` | `Pacer` — the loop's scheduling decisions and the frames it owes (`FrameDebt`, settled only by a present), testable without a window. Under `Reactive` it is Parked, Hot or Hidden: a ring or an event while Parked draws at once; while Hot a frame is drawn at most once per tick, the tick the slower of the pacing's rate and the display's (read from the window's monitor, 60 Hz when unreadable); two ticks that draw nothing park it; an `At` demand parks it until that instant; an occluded or minimized window — on Wayland, one whose requested redraw the compositor has withheld for 4 refreshes (≥50 ms), its frame callbacks having stopped — is Hidden, acquires nothing, drains its rings and owes `FrameDebt::Revealed` from the moment it hides. `Visible` is the token `Surface::acquire` takes, minted only by the pacer's non-Hidden branch (`tests/trybuild.rs` pins E0451 and E0061), and `tests/structural.rs` refuses a `get_current_texture` call anywhere else in `src/` or `examples/`; `Visibility` (`AppBuilder::visibility`) reports hidden/hides/reveals. `Capped` and `Continuous` keep their schedules and ignore occlusion. `pacer/matrix.rs` is the state × event matrix over a fake surface, every pacing a row |

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
