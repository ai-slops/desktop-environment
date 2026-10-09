use crate::{Desired, Id, ObservedWindow, Runtime, ShowState};

#[derive(Clone, Debug)]
pub struct ManualEdit {
    pub view: Id,
    pub placement: Id,
    pub window: Id,
    pub slot: Id,
    pub context: String,
    pub position: Option<[f64; 2]>,
    pub size: Option<[f64; 2]>,
}

impl ManualEdit {
    /// Called only for a paired native user move/size gesture, never a location-change notification alone.
    #[must_use]
    pub fn observed(
        view: Id,
        desired: &Desired,
        before: &ObservedWindow,
        after: &ObservedWindow,
    ) -> Option<Self> {
        if desired.carried
            || before.binding != after.binding
            || before.dpi != after.dpi
            || before.display != after.display
            || after.show_state != ShowState::Normal
        {
            return None;
        }
        let moved = (before.frame.x, before.frame.y) != (after.frame.x, after.frame.y);
        let resized = before.client != after.client;
        if !moved && !resized {
            return None;
        }
        let scale = f64::from(after.dpi) / 96.0;
        Some(Self {
            view,
            placement: desired.placement.clone(),
            window: desired.window.clone(),
            slot: desired.slot.clone(),
            context: desired.context.clone(),
            position: moved.then_some([
                f64::from(after.frame.x - desired.allocated.x) / scale,
                f64::from(after.frame.y - desired.allocated.y) / scale,
            ]),
            size: resized.then_some([
                f64::from(after.client[0]) / scale,
                f64::from(after.client[1]) / scale,
            ]),
        })
    }
}

impl Runtime {
    /// Saving a property promotes only that property's Visit exception. Other preservation remains.
    pub fn promote_properties(&mut self, slot: &str, window: &str, position: bool, size: bool) {
        if let Some(presentation) = self.presentations.get_mut(slot) {
            if let Some(exception) = presentation.overrides.get_mut(window) {
                if position {
                    exception.preserve_position = false;
                }
                if size {
                    exception.preserve_size = false;
                }
            }
            presentation.overrides.retain(|_, exception| {
                exception.preserve_position
                    || exception.preserve_size
                    || exception.mode == crate::TransitionMode::Bring
            });
        }
    }
}
