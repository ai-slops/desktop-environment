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
        self.left + self.width as i32
    }

    #[must_use]
    pub const fn bottom(self) -> i32 {
        self.top + self.height as i32
    }

    #[must_use]
    pub fn contains(self, x: i32, y: i32) -> bool {
        x >= self.left && x < self.right() && y >= self.top && y < self.bottom()
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
        if window_width == 0 || window_height == 0 {
            return None;
        }

        let clamped_x = pointer.x.clamp(0.0, window_width as f32);
        let clamped_y = pointer.y.clamp(0.0, window_height as f32);

        let x_ratio = clamped_x / window_width as f32;
        let y_ratio = clamped_y / window_height as f32;

        let display_x =
            source_display.left + (x_ratio * source_display.width as f32).round() as i32;
        let display_y =
            source_display.top + (y_ratio * source_display.height as f32).round() as i32;

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

        let width = self.bounds.width.saturating_sub(1).max(1) as f32;
        let height = self.bounds.height.saturating_sub(1).max(1) as f32;

        let x_offset = (x - self.bounds.left) as f32;
        let y_offset = (y - self.bounds.top) as f32;

        let absolute_x = ((x_offset / width) * 65_535.0).round() as i32;
        let absolute_y = ((y_offset / height) * 65_535.0).round() as i32;

        Some((absolute_x, absolute_y))
    }
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
}
