# Resize — one model for every madori grid app

Tier: DESIGN (2026-10-24). Phase 0 and the layer anchor are code; the rest is the plan.

## The problem class

A window resize in a grid app (mado, escriba, suzuri) goes through five
independent pieces of hand-written code. No type joins them:

| Step | Where it is today | Defect it caused |
|---|---|---|
| pixels → cells | 6 hand-rolled copies: mado `render.rs` `cells_for_window_phys`, `main.rs`, `single_pane.rs`, `gui_tear_attach.rs`; escriba `gpu.rs`; suzuri `renderer/mod.rs` | the copies disagree on padding, floor and scale |
| when to commit a new grid | mado `InputEngine` `grid_sync_sig` (pixels) + `pushed_cells` (cells) | a PTY resize on every pixel of a drag (fixed 48ffd84, as a patch) |
| what the pane does with it | tear `pane_resize_absolute`: PTY ioctl + grid lock + registry lock, synchronous | main-thread work inside the macOS modal resize loop |
| reflow | mado `Terminal::resize` rewraps all of scrollback per column change | O(history) per column, on the frame path |
| what the frame shows meanwhile | madori reconfigures; mado `render` could return after acquire; the Metal layer stretched | blank, stale and stretched frames |

Each defect was found and patched in place. The class stays open, because
nothing makes the bad states unrepresentable.

## Destination

### 1. `madori::grid` — the one pixels → cells function

```rust
pub struct CellMetrics { w: PositiveF32, h: PositiveF32 }   // constructor refuses <= 0, NaN
pub struct Insets { top, right, bottom, left: f32 }          // physical px
pub struct Grid { cols: NonZeroU16, rows: NonZeroU16 }
impl Grid { pub fn fit(surface: SurfaceSize, cell: CellMetrics, insets: Insets) -> Grid }
```

`Grid` has no other constructor in app code. A zero grid and a negative cell
cannot be built. All 6 copies become calls to `Grid::fit`. A proptest holds the
law: `fit` is monotone in each surface axis, and a surface change smaller than
one cell keeps the grid.

### 2. `madori::resize::GridReconciler` — when a grid is committed

```rust
pub enum Commit { Live, Coalesce { quiet: Duration }, OnSettle }
pub trait GridSink { fn commit(&mut self, grid: Grid); }
pub struct GridReconciler<S: GridSink> { committed: Option<Grid>, pending: Option<(Grid, Instant)>, policy: Commit, sink: S }
```

Input once per frame: `(SurfaceSize, CellMetrics, Insets)`. Output: `commit`
only on a grid that differs from the committed one, and only at the time the
policy allows. madori owns the timer, so the sink is never called from inside
the frame. mado's `grid_sync_sig` and `pushed_cells` and gui_tear_attach's own
resize code go away. The mirror Terminal and the pane are ONE `GridSink`, so
they cannot disagree.

Proptest: for any drag sequence, `Live` commits exactly the distinct
consecutive grids, `Coalesce` commits the last grid once after `quiet`, and
after settle the committed grid always equals `Grid::fit` of the last surface.

### 3. Presentation contract — an acquired frame is always painted

* `RenderCallback::render` returns a `Painted` token. Only the paint path of
  `RenderContext` constructs it. madori presents only with a token, so "present
  an unpainted drawable" does not compile. Today it is a runtime rule in mado
  (48ffd84 repaints the retained snapshot).
* `SurfaceAnchor { TopLeft, Stretch }` in `AppConfig` (default `TopLeft`). On
  macOS it sets the CAMetalLayer `contentsGravity`, derived from
  `contentsAreFlipped` (code: `src/metal_layer.rs`, 5ae97fa). The layer
  background takes the app clear colour, so the area the old frame does not
  cover is the theme background, not the desktop.
* `presentsWithTransaction` during a live resize: needs wgpu-hal to expose the
  flag (it is `pub` on the hal Surface, but wgpu 25 `as_hal` gives `&`). Upstream
  patch or a wgpu bump.

### 4. Reflow off the frame path

Destination: the Terminal stores logical lines and wraps on demand for the rows
it shows (the wezterm model), so a width change costs O(viewport), not
O(history). Interim, in the same config surface: `reflow = live | on-settle`;
`on-settle` truncates the display during a drag and rewraps once on commit.

### 5. tear — a pane resize never blocks the caller

`pane_resize_absolute` becomes a latest-value-wins cell read by the pane's own
task. The caller (mado's frame path) only stores the value.

## Configuration

One typed block, per app, through shikumi:

```yaml
window:
  resize:
    commit: coalesce      # live | coalesce | on-settle
    quiet_ms: 40
    anchor: top-left      # top-left | stretch
    reflow: on-settle     # live | on-settle
```

Each knob turns behaviour on or off; none deletes a path.

## Order

| Phase | Change | Repos | Gate |
|---|---|---|---|
| 0 | done: dedupe per cell (48ffd84), paint held frames (48ffd84), layer anchor (5ae97fa) | mado, madori | red-run tests, pixel probe |
| 1 | `madori::grid` + replace the 6 copies | madori, mado, escriba, suzuri | proptest on `Grid::fit` |
| 2 | `GridReconciler` + `Commit` policy + config | madori, mado | drag proptest |
| 3 | `Painted` token, anchor in `AppConfig`, layer background | madori + every consumer | does not compile without a token |
| 4 | tear latest-value-wins pane resize | tear | caller never takes the pty lock |
| 5 | lazy reflow | mado | per-column cost independent of scrollback |
