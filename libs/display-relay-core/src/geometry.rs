use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplayArea {
    pub left: i32,
    pub top: i32,
    pub width: u32,
    pub height: u32,
}

impl DisplayArea {
    #[must_use]
    pub const fn right(self) -> i32 {
        self.left.saturating_add_unsigned(self.width)
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.top.saturating_add_unsigned(self.height)
    }

    #[must_use]
    pub const fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left
            && (x as i64) < self.left as i64 + self.width as i64
            && y >= self.top
            && (y as i64) < self.top as i64 + self.height as i64
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointerSample {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct VirtualDesktop {
    pub bounds: DisplayArea,
}

impl VirtualDesktop {
    #[must_use]
    pub fn map_window_pointer(
        self,
        source_display: DisplayArea,
        window_size: (u32, u32),
        pointer: PointerSample,
    ) -> Option<(i32, i32)> {
        let (window_width, window_height) = window_size;
        if window_width == 0
            || window_height == 0
            || !pointer.x.is_finite()
            || !pointer.y.is_finite()
        {
            return None;
        }

        let clamped_x = f64::from(pointer.x).clamp(0.0, f64::from(window_width));
        let clamped_y = f64::from(pointer.y).clamp(0.0, f64::from(window_height));

        let x_ratio = clamped_x / f64::from(window_width);
        let y_ratio = clamped_y / f64::from(window_height);

        let display_x = rounded_coordinate(
            f64::from(source_display.left) + (x_ratio * f64::from(source_display.width)).round(),
        )?;
        let display_y = rounded_coordinate(
            f64::from(source_display.top) + (y_ratio * f64::from(source_display.height)).round(),
        )?;

        if source_display.contains(display_x, display_y) {
            Some((display_x, display_y))
        } else {
            None
        }
    }

    #[must_use]
    pub fn absolute_mouse(self, x: i32, y: i32) -> Option<(i32, i32)> {
        if !self.bounds.contains(x, y) {
            return None;
        }

        let width = f64::from(self.bounds.width.saturating_sub(1).max(1));
        let height = f64::from(self.bounds.height.saturating_sub(1).max(1));

        let x_offset = f64::from(x) - f64::from(self.bounds.left);
        let y_offset = f64::from(y) - f64::from(self.bounds.top);

        let absolute_x = rounded_coordinate((x_offset / width) * 65_535.0)?;
        let absolute_y = rounded_coordinate((y_offset / height) * 65_535.0)?;

        Some((absolute_x, absolute_y))
    }
}

#[allow(clippy::cast_possible_truncation)] // Rounded and explicitly checked against the full signed coordinate range.
fn rounded_coordinate(value: f64) -> Option<i32> {
    let rounded = value.round();
    (rounded.is_finite() && rounded >= f64::from(i32::MIN) && rounded <= f64::from(i32::MAX))
        .then_some(rounded as i32)
}

#[cfg(test)]
mod tests {
    use super::{DisplayArea, PointerSample, VirtualDesktop};

    #[test]
    fn maps_window_pointer_into_source_display() {
        let desktop = VirtualDesktop {
            bounds: DisplayArea { left: -1920, top: 0, width: 3840, height: 1080 },
        };

        let mapped = desktop.map_window_pointer(
            DisplayArea { left: 0, top: 0, width: 1920, height: 1080 },
            (960, 540),
            PointerSample { x: 480.0, y: 270.0 },
        );

        assert_eq!(mapped, Some((960, 540)));
    }

    #[test]
    fn normalizes_absolute_mouse_coordinates() {
        let desktop =
            VirtualDesktop { bounds: DisplayArea { left: 0, top: 0, width: 1920, height: 1080 } };

        let absolute = desktop.absolute_mouse(1919, 1079);

        assert_eq!(absolute, Some((65_535, 65_535)));
    }

    #[test]
    fn rejects_nonfinite_pointer_and_handles_large_signed_desktop_without_overflow() {
        let desktop = VirtualDesktop {
            bounds: DisplayArea { left: i32::MIN, top: 0, width: u32::MAX, height: 2 },
        };
        assert_eq!(desktop.absolute_mouse(i32::MAX - 1, 1), Some((65_535, 65_535)));
        assert_eq!(
            desktop.map_window_pointer(
                desktop.bounds,
                (100, 100),
                PointerSample { x: f32::NAN, y: 50.0 }
            ),
            None
        );
        assert_eq!(desktop.bounds.right(), i32::MAX);
    }
}
