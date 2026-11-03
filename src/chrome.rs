use crate::event::PointerIcon;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowState {
    pub maximized: bool,
    pub fullscreen: bool,
    pub decorated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResizeEdge {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}

impl ResizeEdge {
    pub const ALL: [Self; 8] = [
        Self::North,
        Self::NorthEast,
        Self::East,
        Self::SouthEast,
        Self::South,
        Self::SouthWest,
        Self::West,
        Self::NorthWest,
    ];

    #[must_use]
    pub fn pointer_icon(self) -> PointerIcon {
        match self {
            Self::North | Self::South => PointerIcon::NsResize,
            Self::East | Self::West => PointerIcon::EwResize,
            Self::NorthEast | Self::SouthWest => PointerIcon::NeswResize,
            Self::NorthWest | Self::SouthEast => PointerIcon::NwseResize,
        }
    }

    #[must_use]
    pub(crate) fn to_winit(self) -> winit::window::ResizeDirection {
        use winit::window::ResizeDirection as D;
        match self {
            Self::North => D::North,
            Self::NorthEast => D::NorthEast,
            Self::East => D::East,
            Self::SouthEast => D::SouthEast,
            Self::South => D::South,
            Self::SouthWest => D::SouthWest,
            Self::West => D::West,
            Self::NorthWest => D::NorthWest,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChromeButton {
    Minimize,
    Maximize,
    Close,
}

impl ChromeButton {
    pub const ALL: [Self; 3] = [Self::Minimize, Self::Maximize, Self::Close];

    const fn slots_from_right(self) -> f32 {
        match self {
            Self::Close => 1.0,
            Self::Maximize => 2.0,
            Self::Minimize => 3.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowOp {
    Drag,
    Resize(ResizeEdge),
    ToggleMaximize,
    Minimize,
    Close,
    ShowMenu { x: i32, y: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WindowRegion {
    Interior,
    Band,
    Button(ChromeButton),
    Edge(ResizeEdge),
}

impl WindowRegion {
    #[must_use]
    pub fn pointer_icon(self) -> PointerIcon {
        match self {
            Self::Edge(edge) => edge.pointer_icon(),
            Self::Interior | Self::Band | Self::Button(_) => PointerIcon::Default,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromeMetrics {
    pub band_height: f32,
    pub edge_width: f32,
    pub corner_extent: f32,
    pub button_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ButtonRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl ButtonRect {
    fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ChromeLayout {
    width: f32,
    height: f32,
    band_height: f32,
    edge_width: f32,
    corner_extent: f32,
    button_width: f32,
}

impl ChromeLayout {
    #[must_use]
    pub fn for_window(
        width: f32,
        height: f32,
        metrics: ChromeMetrics,
        state: WindowState,
    ) -> Option<Self> {
        if state.decorated || state.fullscreen {
            return None;
        }
        let edge_width = if state.maximized {
            0.0
        } else {
            metrics.edge_width.max(0.0)
        };
        Some(Self {
            width,
            height,
            band_height: metrics.band_height.max(0.0),
            edge_width,
            corner_extent: metrics.corner_extent.max(edge_width),
            button_width: metrics.button_width.max(0.0),
        })
    }

    #[must_use]
    pub fn band_height(&self) -> f32 {
        self.band_height
    }

    #[must_use]
    pub fn button_rect(&self, button: ChromeButton) -> ButtonRect {
        ButtonRect {
            x: self.width - self.button_width * button.slots_from_right(),
            y: 0.0,
            width: self.button_width,
            height: self.band_height,
        }
    }

    fn edge_at(&self, x: f32, y: f32) -> Option<ResizeEdge> {
        if self.edge_width <= 0.0 {
            return None;
        }
        let edge = self.edge_width;
        let corner = self.corner_extent;
        let (left, right) = (x < edge, x >= self.width - edge);
        let (top, bottom) = (y < edge, y >= self.height - edge);
        let (near_left, near_right) = (x < corner, x >= self.width - corner);
        let (near_top, near_bottom) = (y < corner, y >= self.height - corner);
        if (top && near_left) || (left && near_top) {
            Some(ResizeEdge::NorthWest)
        } else if (top && near_right) || (right && near_top) {
            Some(ResizeEdge::NorthEast)
        } else if (bottom && near_left) || (left && near_bottom) {
            Some(ResizeEdge::SouthWest)
        } else if (bottom && near_right) || (right && near_bottom) {
            Some(ResizeEdge::SouthEast)
        } else if top {
            Some(ResizeEdge::North)
        } else if bottom {
            Some(ResizeEdge::South)
        } else if left {
            Some(ResizeEdge::West)
        } else if right {
            Some(ResizeEdge::East)
        } else {
            None
        }
    }

    #[must_use]
    pub fn hit_test(&self, x: f32, y: f32) -> WindowRegion {
        if let Some(edge) = self.edge_at(x, y) {
            return WindowRegion::Edge(edge);
        }
        if y >= self.band_height {
            return WindowRegion::Interior;
        }
        ChromeButton::ALL
            .into_iter()
            .find(|b| self.button_rect(*b).contains(x, y))
            .map_or(WindowRegion::Band, WindowRegion::Button)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const METRICS: ChromeMetrics = ChromeMetrics {
        band_height: 30.0,
        edge_width: 6.0,
        corner_extent: 16.0,
        button_width: 40.0,
    };

    fn restored() -> ChromeLayout {
        ChromeLayout::for_window(800.0, 600.0, METRICS, WindowState::default()).unwrap()
    }

    #[test]
    fn every_side_is_its_own_edge() {
        let l = restored();
        assert_eq!(
            l.hit_test(400.0, 0.0),
            WindowRegion::Edge(ResizeEdge::North)
        );
        assert_eq!(
            l.hit_test(400.0, 5.9),
            WindowRegion::Edge(ResizeEdge::North)
        );
        assert_eq!(
            l.hit_test(400.0, 599.0),
            WindowRegion::Edge(ResizeEdge::South)
        );
        assert_eq!(l.hit_test(0.0, 300.0), WindowRegion::Edge(ResizeEdge::West));
        assert_eq!(
            l.hit_test(799.0, 300.0),
            WindowRegion::Edge(ResizeEdge::East)
        );
    }

    #[test]
    fn every_corner_is_its_own_edge_and_reaches_past_the_edge_width() {
        let l = restored();
        let cases = [
            ((0.0, 0.0), ResizeEdge::NorthWest),
            ((799.0, 0.0), ResizeEdge::NorthEast),
            ((0.0, 599.0), ResizeEdge::SouthWest),
            ((799.0, 599.0), ResizeEdge::SouthEast),
            ((12.0, 1.0), ResizeEdge::NorthWest),
            ((1.0, 12.0), ResizeEdge::NorthWest),
            ((788.0, 1.0), ResizeEdge::NorthEast),
            ((798.0, 12.0), ResizeEdge::NorthEast),
            ((12.0, 598.0), ResizeEdge::SouthWest),
            ((1.0, 588.0), ResizeEdge::SouthWest),
            ((788.0, 598.0), ResizeEdge::SouthEast),
            ((798.0, 588.0), ResizeEdge::SouthEast),
        ];
        for ((x, y), edge) in cases {
            assert_eq!(l.hit_test(x, y), WindowRegion::Edge(edge), "{x},{y}");
        }
    }

    #[test]
    fn the_band_answers_between_the_edges_and_above_the_terminal() {
        let l = restored();
        assert_eq!(l.hit_test(100.0, 6.0), WindowRegion::Band);
        assert_eq!(l.hit_test(100.0, 29.9), WindowRegion::Band);
        assert_eq!(l.hit_test(679.9, 15.0), WindowRegion::Band);
    }

    #[test]
    fn each_button_owns_its_slot_right_to_left() {
        let l = restored();
        assert_eq!(
            l.hit_test(700.0, 15.0),
            WindowRegion::Button(ChromeButton::Minimize)
        );
        assert_eq!(
            l.hit_test(740.0, 15.0),
            WindowRegion::Button(ChromeButton::Maximize)
        );
        assert_eq!(
            l.hit_test(780.0, 15.0),
            WindowRegion::Button(ChromeButton::Close)
        );
        for button in ChromeButton::ALL {
            let r = l.button_rect(button);
            assert_eq!(
                l.hit_test(r.x + r.width / 2.0, r.y + r.height / 2.0),
                WindowRegion::Button(button)
            );
        }
    }

    #[test]
    fn a_click_inside_the_terminal_area_stays_with_the_terminal() {
        let l = restored();
        for (x, y) in [(400.0, 300.0), (6.0, 30.0), (793.9, 593.9), (100.0, 30.0)] {
            assert_eq!(l.hit_test(x, y), WindowRegion::Interior, "{x},{y}");
        }
    }

    #[test]
    fn maximized_has_no_edges_but_keeps_the_band_and_its_buttons() {
        let state = WindowState {
            maximized: true,
            ..WindowState::default()
        };
        let l = ChromeLayout::for_window(800.0, 600.0, METRICS, state).unwrap();
        assert_eq!(l.hit_test(0.0, 300.0), WindowRegion::Interior);
        assert_eq!(l.hit_test(799.0, 599.0), WindowRegion::Interior);
        assert_eq!(l.hit_test(400.0, 0.0), WindowRegion::Band);
        assert_eq!(l.hit_test(0.0, 0.0), WindowRegion::Band);
        assert_eq!(
            l.hit_test(799.0, 0.0),
            WindowRegion::Button(ChromeButton::Close)
        );
    }

    #[test]
    fn fullscreen_and_a_compositors_own_bar_leave_no_chrome_at_all() {
        for state in [
            WindowState {
                fullscreen: true,
                ..WindowState::default()
            },
            WindowState {
                decorated: true,
                ..WindowState::default()
            },
            WindowState {
                decorated: true,
                maximized: true,
                ..WindowState::default()
            },
        ] {
            assert_eq!(
                ChromeLayout::for_window(800.0, 600.0, METRICS, state),
                None,
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_zero_edge_width_turns_resizing_off_and_nothing_else() {
        let metrics = ChromeMetrics {
            edge_width: 0.0,
            corner_extent: 0.0,
            ..METRICS
        };
        let l = ChromeLayout::for_window(800.0, 600.0, metrics, WindowState::default()).unwrap();
        assert_eq!(l.hit_test(400.0, 0.0), WindowRegion::Band);
        assert_eq!(l.hit_test(0.0, 300.0), WindowRegion::Interior);
    }

    #[test]
    fn every_edge_shows_a_resize_pointer_on_its_own_axis() {
        for edge in ResizeEdge::ALL {
            let want = match edge {
                ResizeEdge::North | ResizeEdge::South => PointerIcon::NsResize,
                ResizeEdge::East | ResizeEdge::West => PointerIcon::EwResize,
                ResizeEdge::NorthEast | ResizeEdge::SouthWest => PointerIcon::NeswResize,
                ResizeEdge::NorthWest | ResizeEdge::SouthEast => PointerIcon::NwseResize,
            };
            assert_eq!(WindowRegion::Edge(edge).pointer_icon(), want, "{edge:?}");
        }
        assert_eq!(WindowRegion::Band.pointer_icon(), PointerIcon::Default);
    }
}
