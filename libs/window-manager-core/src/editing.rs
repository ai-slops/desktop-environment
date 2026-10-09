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

impl crate::Configuration {
    /// An explicit choice to replace BOTH dimension rules; local manual edits never call this.
    pub fn replace_size_formulas(
        &mut self,
        view: &str,
        placement: &str,
        context: &str,
        size: [f64; 2],
    ) -> crate::Result<()> {
        let mut draft = self.clone();
        draft.save_properties(view, placement, context, None, Some(size))?;
        let node = draft
            .views
            .get_mut(view)
            .and_then(|view| view.roots.values_mut().find_map(|root| root.placement_mut(placement)))
            .ok_or_else(|| {
                crate::Error::new(crate::ErrorCode::TargetMissing, "Placement missing", placement)
            })?;
        let preference = node.preferences.get_mut(context).ok_or_else(|| {
            crate::Error::new(crate::ErrorCode::TargetMissing, "Context missing", placement)
        })?;
        preference.client_size = Some(size);
        preference.size_override = None;
        preference.width_formula = None;
        preference.height_formula = None;
        draft.validate()?;
        *self = draft;
        Ok(())
    }
    /// Copy only the authored size rule into the portable baseline. Geometry contexts,
    /// position and manual overrides remain independent.
    pub fn promote_size_rule(
        &mut self,
        view: &str,
        placement: &str,
        context: &str,
    ) -> crate::Result<()> {
        let mut draft = self.clone();
        let node = draft
            .views
            .get_mut(view)
            .and_then(|view| view.roots.values_mut().find_map(|root| root.placement_mut(placement)))
            .ok_or_else(|| {
                crate::Error::new(crate::ErrorCode::TargetMissing, "Placement missing", placement)
            })?;
        let rule = node.preferences.get(context).cloned().ok_or_else(|| {
            crate::Error::new(crate::ErrorCode::TargetMissing, "Context rule missing", placement)
        })?;
        node.default_preference.client_size = rule.client_size;
        node.default_preference.width_formula = rule.width_formula;
        node.default_preference.height_formula = rule.height_formula;
        draft.revision += 1;
        draft.validate()?;
        *self = draft;
        Ok(())
    }
}
