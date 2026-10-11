use std::time::{Duration, Instant};
use winit::dpi::PhysicalPosition;

/// Recognizes two completed left clicks; a drag or leaving the client cancels it.
#[derive(Default)]
pub(crate) struct ClickTracker {
    pressed: Option<PhysicalPosition<f64>>,
    previous: Option<(Instant, PhysicalPosition<f64>)>,
}
impl ClickTracker {
    pub(crate) fn cancel(&mut self) {
        *self = Self::default();
    }
    pub(crate) fn press(&mut self, position: PhysicalPosition<f64>) {
        self.pressed = Some(position);
    }
    pub(crate) fn motion(&mut self, position: PhysicalPosition<f64>, tolerance: (f64, f64)) {
        if self.pressed.is_some_and(|pressed| !near(pressed, position, tolerance)) {
            self.cancel();
        }
    }
    pub(crate) fn release(
        &mut self,
        position: PhysicalPosition<f64>,
        now: Instant,
        interval: Duration,
        tolerance: (f64, f64),
    ) -> bool {
        let Some(pressed) = self.pressed.take() else { return false };
        if !near(pressed, position, tolerance) {
            self.previous = None;
            return false;
        }
        if self.previous.is_some_and(|(time, point)| {
            now.saturating_duration_since(time) <= interval && near(point, position, tolerance)
        }) {
            self.previous = None;
            true
        } else {
            self.previous = Some((now, position));
            false
        }
    }
}
fn near(a: PhysicalPosition<f64>, b: PhysicalPosition<f64>, tolerance: (f64, f64)) -> bool {
    a.x.is_finite()
        && a.y.is_finite()
        && b.x.is_finite()
        && b.y.is_finite()
        && (a.x - b.x).abs() <= tolerance.0
        && (a.y - b.y).abs() <= tolerance.1
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn double_click_requires_nearby_completed_clicks_and_cancels_drag_leave_and_triples() {
        let mut clicks = ClickTracker::default();
        let p = PhysicalPosition::new(50.0, 60.0);
        let now = Instant::now();
        let interval = Duration::from_millis(500);
        let tolerance = (4.0, 4.0);
        clicks.press(p);
        assert!(!clicks.release(p, now, interval, tolerance));
        clicks.press(p);
        assert!(clicks.release(p, now + Duration::from_millis(100), interval, tolerance));
        clicks.press(p);
        assert!(!clicks.release(p, now + Duration::from_millis(200), interval, tolerance));
        clicks.cancel();
        clicks.press(p);
        assert!(!clicks.release(PhysicalPosition::new(80.0, 60.0), now, interval, tolerance));
        clicks.press(p);
        assert!(!clicks.release(p, now, interval, tolerance));
        clicks.press(p);
        assert!(!clicks.release(p, now + Duration::from_secs(1), interval, tolerance));
        clicks.cancel();
        assert!(!clicks.release(p, now, interval, tolerance));
        clicks.press(p);
        clicks.motion(PhysicalPosition::new(100.0, 100.0), tolerance);
        clicks.motion(p, tolerance);
        assert!(!clicks.release(p, now, interval, tolerance));
    }
}
