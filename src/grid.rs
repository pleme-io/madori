use ishou_tokens::refined::{Bounds, Refined};
use std::num::NonZeroU16;

pub struct CellPxBounds;

impl Bounds<f32> for CellPxBounds {
    fn min() -> f32 {
        1.0
    }
    fn max() -> f32 {
        4096.0
    }
    fn default() -> f32 {
        1.0
    }
}

pub type CellPx = Refined<f32, CellPxBounds>;

pub struct ExtentPxBounds;

impl Bounds<f32> for ExtentPxBounds {
    fn min() -> f32 {
        0.0
    }
    fn max() -> f32 {
        1.0e7
    }
    fn default() -> f32 {
        0.0
    }
}

pub type ExtentPx = Refined<f32, ExtentPxBounds>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    pub width: CellPx,
    pub height: CellPx,
}

pub const MONO_ADVANCE_ESTIMATE: f32 = 0.6;

impl CellMetrics {
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: CellPx::new(width),
            height: CellPx::new(height),
        }
    }

    #[must_use]
    pub fn estimate(font_size: f32, line_height: f32) -> Self {
        Self::new(font_size * MONO_ADVANCE_ESTIMATE, font_size * line_height)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Insets {
    pub top: ExtentPx,
    pub right: ExtentPx,
    pub bottom: ExtentPx,
    pub left: ExtentPx,
}

impl Insets {
    #[must_use]
    pub fn uniform(px: f32) -> Self {
        let px = ExtentPx::new(px);
        Self {
            top: px,
            right: px,
            bottom: px,
            left: px,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SurfaceSize {
    pub width: ExtentPx,
    pub height: ExtentPx,
}

impl SurfaceSize {
    #[must_use]
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: ExtentPx::new(width),
            height: ExtentPx::new(height),
        }
    }

    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn physical(width: u32, height: u32) -> Self {
        Self::new(width as f32, height as f32)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Grid {
    cols: NonZeroU16,
    rows: NonZeroU16,
}

impl Grid {
    #[must_use]
    pub fn new(cols: u16, rows: u16) -> Option<Self> {
        Some(Self {
            cols: NonZeroU16::new(cols)?,
            rows: NonZeroU16::new(rows)?,
        })
    }

    #[must_use]
    pub fn fit(surface: SurfaceSize, cell: CellMetrics, insets: Insets) -> Self {
        let inner_w = surface.width.get() - insets.left.get() - insets.right.get();
        let inner_h = surface.height.get() - insets.top.get() - insets.bottom.get();
        Self {
            cols: cells_along(inner_w, cell.width.get()),
            rows: cells_along(inner_h, cell.height.get()),
        }
    }

    #[must_use]
    pub fn cols(self) -> u16 {
        self.cols.get()
    }

    #[must_use]
    pub fn rows(self) -> u16 {
        self.rows.get()
    }

    #[must_use]
    pub fn without_rows(self, reserved: u16) -> Self {
        Self {
            cols: self.cols,
            rows: NonZeroU16::new(self.rows.get().saturating_sub(reserved))
                .unwrap_or(NonZeroU16::MIN),
        }
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn cells_along(inner: f32, cell: f32) -> NonZeroU16 {
    let n = (inner.max(0.0) / cell)
        .floor()
        .clamp(1.0, f32::from(u16::MAX));
    NonZeroU16::new(n as u16).unwrap_or(NonZeroU16::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn surface() -> impl Strategy<Value = (f32, f32)> {
        (0.0f32..8000.0, 0.0f32..8000.0)
    }

    fn cell() -> impl Strategy<Value = (f32, f32)> {
        (1.0f32..64.0, 1.0f32..128.0)
    }

    proptest! {
        #[test]
        fn the_grid_is_the_largest_that_fits_inside_the_insets(
            (w, h) in surface(), (cw, ch) in cell(), pad in 0.0f32..64.0,
        ) {
            let g = Grid::fit(SurfaceSize::new(w, h), CellMetrics::new(cw, ch), Insets::uniform(pad));
            let (iw, ih) = ((w - 2.0 * pad).max(0.0), (h - 2.0 * pad).max(0.0));
            if g.cols() > 1 {
                prop_assert!(f32::from(g.cols()) * cw <= iw + 1e-3);
            }
            if g.rows() > 1 {
                prop_assert!(f32::from(g.rows()) * ch <= ih + 1e-3);
            }
            prop_assert!(f32::from(g.cols() + 1) * cw > iw - 1e-3);
            prop_assert!(f32::from(g.rows() + 1) * ch > ih - 1e-3);
        }

        #[test]
        fn a_larger_surface_never_fits_fewer_cells(
            (w, h) in surface(), (dw, dh) in (0.0f32..500.0, 0.0f32..500.0), (cw, ch) in cell(),
        ) {
            let m = CellMetrics::new(cw, ch);
            let a = Grid::fit(SurfaceSize::new(w, h), m, Insets::uniform(0.0));
            let b = Grid::fit(SurfaceSize::new(w + dw, h + dh), m, Insets::uniform(0.0));
            prop_assert!(b.cols() >= a.cols() && b.rows() >= a.rows());
        }

        #[test]
        fn a_change_smaller_than_the_slack_keeps_the_grid(
            (w, h) in surface(), (cw, ch) in cell(), frac in 0.0f32..1.0,
        ) {
            let m = CellMetrics::new(cw, ch);
            let a = Grid::fit(SurfaceSize::new(w, h), m, Insets::uniform(0.0));
            let slack = (w - f32::from(a.cols()) * cw).max(0.0);
            let b = Grid::fit(SurfaceSize::new(w - slack * (1.0 - frac) * 0.99, h), m, Insets::uniform(0.0));
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn an_estimate_is_the_monospace_advance_by_the_line_height() {
        let m = CellMetrics::estimate(10.0, 1.5);
        assert_eq!((m.width.get(), m.height.get()), (6.0, 15.0));
    }

    #[test]
    fn degenerate_inputs_still_yield_a_one_by_one_grid() {
        let m = CellMetrics::new(f32::NAN, -3.0);
        let g = Grid::fit(
            SurfaceSize::new(f32::NAN, -10.0),
            m,
            Insets::uniform(f32::INFINITY),
        );
        assert_eq!((g.cols(), g.rows()), (1, 1));
        assert_eq!(Grid::new(0, 3), None);
        assert_eq!(Grid::new(3, 4).unwrap().without_rows(9).rows(), 1);
    }
}
