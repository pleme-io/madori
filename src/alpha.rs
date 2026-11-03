use wgpu::CompositeAlphaMode;

#[must_use]
pub fn select_alpha_mode(
    available: &[CompositeAlphaMode],
    transparent: bool,
) -> CompositeAlphaMode {
    if transparent {
        let carrying = [
            CompositeAlphaMode::PreMultiplied,
            CompositeAlphaMode::PostMultiplied,
            CompositeAlphaMode::Inherit,
        ];
        if let Some(mode) = carrying.into_iter().find(|m| available.contains(m)) {
            return mode;
        }
    }
    opaque_choice(available)
}

fn opaque_choice(available: &[CompositeAlphaMode]) -> CompositeAlphaMode {
    // Prefer Opaque alpha_mode when the surface
    // advertises it. On macOS the default
    // (caps.alpha_modes[0]) is sometimes
    // PostMultiplied or Inherit, which causes
    // the FIRST presented frame to flash a
    // garbage-colour (often purple/magenta) for
    // a few ms while the OS compositor decides
    // how to blend an uninitialized surface
    // against whatever's behind the window.
    // Opaque tells the compositor "draw my
    // pixels verbatim, no alpha" — same shape
    // ghostty / kitty / alacritty use on macOS.
    if available.contains(&CompositeAlphaMode::Opaque) {
        CompositeAlphaMode::Opaque
    } else {
        // Same empty-vector hazard as `formats`, reached by
        // the same mismatch. The guard above has already
        // returned in that case, so `Auto` here is only for
        // an adapter that advertises formats but no alpha
        // modes — let wgpu decide rather than panic.
        available
            .first()
            .copied()
            .unwrap_or(CompositeAlphaMode::Auto)
    }
}

#[must_use]
pub fn carries_alpha(mode: CompositeAlphaMode) -> bool {
    matches!(
        mode,
        CompositeAlphaMode::PreMultiplied
            | CompositeAlphaMode::PostMultiplied
            | CompositeAlphaMode::Inherit
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SurfaceAlpha {
    pub mode: CompositeAlphaMode,
    pub transparent: bool,
}

impl SurfaceAlpha {
    #[must_use]
    pub fn translucent_mode(self) -> Option<CompositeAlphaMode> {
        (self.transparent && carries_alpha(self.mode)).then_some(self.mode)
    }

    #[must_use]
    pub fn alpha_unavailable(self) -> bool {
        self.transparent && !carries_alpha(self.mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CompositeAlphaMode::{Auto, Inherit, Opaque, PostMultiplied, PreMultiplied};

    #[test]
    fn transparent_walks_the_preference_order() {
        let cases: &[(&[CompositeAlphaMode], CompositeAlphaMode)] = &[
            (&[Opaque, PreMultiplied], PreMultiplied),
            (&[PreMultiplied, Opaque], PreMultiplied),
            (&[Opaque, PostMultiplied, PreMultiplied], PreMultiplied),
            (&[Inherit, PostMultiplied, PreMultiplied], PreMultiplied),
            (&[Opaque, PostMultiplied], PostMultiplied),
            (&[Inherit, PostMultiplied], PostMultiplied),
            (&[Opaque, Inherit], Inherit),
        ];
        for (available, want) in cases {
            assert_eq!(select_alpha_mode(available, true), *want, "{available:?}");
        }
    }

    #[test]
    fn omoyas_offer_is_not_answered_by_its_first_entry() {
        let offered = [Opaque, PreMultiplied];
        assert_eq!(offered[0], Opaque);
        assert_eq!(select_alpha_mode(&offered, true), PreMultiplied);
    }

    #[test]
    fn transparent_with_no_alpha_on_offer_falls_back_to_the_opaque_choice() {
        for available in [&[Opaque][..], &[Auto][..], &[Auto, Opaque][..], &[][..]] {
            assert_eq!(
                select_alpha_mode(available, true),
                select_alpha_mode(available, false),
                "{available:?}"
            );
        }
        assert_eq!(select_alpha_mode(&[Opaque], true), Opaque);
    }

    #[test]
    fn not_transparent_keeps_the_choice_of_before() {
        let before = |available: &[CompositeAlphaMode]| {
            if available.contains(&Opaque) {
                Opaque
            } else {
                available.first().copied().unwrap_or(Auto)
            }
        };
        let all = [Auto, Opaque, PreMultiplied, PostMultiplied, Inherit];
        let mut sets: Vec<Vec<CompositeAlphaMode>> = vec![Vec::new()];
        for bits in 1u32..(1 << all.len()) {
            for rotation in 0..all.len() {
                let set: Vec<_> = (0..all.len())
                    .filter(|i| bits & (1 << i) != 0)
                    .map(|i| all[(i + rotation) % all.len()])
                    .collect();
                sets.push(set);
            }
        }
        for set in &sets {
            assert_eq!(select_alpha_mode(set, false), before(set), "{set:?}");
        }
        assert_eq!(select_alpha_mode(&[Opaque, PreMultiplied], false), Opaque);
        assert_eq!(
            select_alpha_mode(&[PostMultiplied, Inherit], false),
            PostMultiplied
        );
    }

    #[test]
    fn only_the_three_composited_modes_carry_alpha() {
        for mode in [PreMultiplied, PostMultiplied, Inherit] {
            assert!(carries_alpha(mode), "{mode:?}");
        }
        for mode in [Opaque, Auto] {
            assert!(!carries_alpha(mode), "{mode:?}");
        }
    }

    #[test]
    fn a_window_that_did_not_ask_is_never_translucent_whatever_the_surface_gave() {
        let asked = SurfaceAlpha {
            mode: PreMultiplied,
            transparent: true,
        };
        assert_eq!(asked.translucent_mode(), Some(PreMultiplied));
        assert!(!asked.alpha_unavailable());

        let unasked = SurfaceAlpha {
            mode: PreMultiplied,
            transparent: false,
        };
        assert_eq!(unasked.translucent_mode(), None);
        assert!(!unasked.alpha_unavailable());

        let starved = SurfaceAlpha {
            mode: Opaque,
            transparent: true,
        };
        assert_eq!(starved.translucent_mode(), None);
        assert!(starved.alpha_unavailable());
    }
}
