use crate::{
    Configuration, Error, ErrorCode, Group, Id, Node, Placement, Preference, Result, ShowState,
    Snapshot, Strategy, View, context_key, new_id, resolve_slot, slot_bounds,
};
use std::collections::{BTreeMap, BTreeSet};

impl Configuration {
    /// Saves only authored geometry from explicit live selections; creates no executable plan.
    pub fn save_observed_view(
        &mut self,
        workspace: &str,
        slot: &str,
        windows: &BTreeSet<Id>,
        name: String,
        snapshot: &Snapshot,
    ) -> Result<Id> {
        self.validate()?;
        if !self.workspaces.contains_key(workspace) || windows.is_empty() || windows.len() > 256 {
            return Err(Error::new(
                ErrorCode::InvalidConfiguration,
                "Capture needs a Workspace and 1–256 selected references",
                "capture",
            ));
        }
        let slot = self
            .slots
            .get(slot)
            .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "Capture Slot missing", slot))?;
        let slot = resolve_slot(self, slot, snapshot)?;
        let area = slot_bounds(&slot, snapshot)?;
        let display = &snapshot.displays[&slot.display];
        let scale = f64::from(display.dpi) / 96.0;
        let mut group = Group::new("현재 창 배치".into());
        group.strategy = Strategy::Free;
        for window in windows {
            let reference = self.windows.get(window).ok_or_else(|| {
                Error::new(ErrorCode::TargetMissing, "Capture reference missing", window)
            })?;
            let observed = snapshot.windows.get(window).ok_or_else(|| {
                Error::new(ErrorCode::StaleBinding, "Capture requires a live binding", window)
            })?;
            if !observed.visible
                || observed.show_state != ShowState::Normal
                || observed.has_owned_dialog
            {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Capture requires a visible normal window without an owned modal",
                    window,
                ));
            }
            if observed.dpi != display.dpi
                || observed.display != slot.display
                || !area.contains(observed.frame)
            {
                return Err(Error::new(
                    ErrorCode::UnsatisfiableConstraints,
                    "Selected window is outside the chosen compatible-DPI Slot",
                    window,
                ));
            }
            observed.frame.validate()?;
            if observed.client.iter().any(|size| *size <= 0) {
                return Err(Error::new(
                    ErrorCode::UnsupportedOperation,
                    "Capture client geometry unavailable",
                    window,
                ));
            }
            let mut placement = Placement::new(window.clone(), reference.alias.clone());
            placement.preferences.insert(
                context_key(&slot, "base"),
                Preference {
                    position: Some([
                        f64::from(observed.frame.x - area.x) / scale,
                        f64::from(observed.frame.y - area.y) / scale,
                    ]),
                    client_size: Some(observed.client.map(|size| f64::from(size) / scale)),
                    ..Preference::default()
                },
            );
            group.children.push(Node::Placement(placement));
        }
        let view = View {
            id: new_id("view"),
            workspace: workspace.into(),
            name,
            roots: BTreeMap::from([("main".into(), Node::Group(group))]),
        };
        let id = view.id.clone();
        let mut draft = self.clone();
        draft.views.insert(id.clone(), view);
        draft.revision += 1;
        draft.validate()?;
        *self = draft;
        Ok(id)
    }
}
