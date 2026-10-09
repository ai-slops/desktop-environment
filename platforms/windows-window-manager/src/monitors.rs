use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use window_manager_core::{Display, Error, ErrorCode, Id, Rect, Result, new_id};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonitorIdentity {
    pub device: String,
    pub hardware: Option<Id>,
    pub name: String,
    pub work_area: Rect,
    pub dpi: u32,
}

/// Explicit aliases expire on any observed topology change. Never persisted into layout data.
pub struct MonitorMappings {
    epoch: Id,
    inventory: Vec<MonitorIdentity>,
    aliases: BTreeMap<String, Id>,
}
impl Default for MonitorMappings {
    fn default() -> Self {
        Self { epoch: new_id("topology"), inventory: vec![], aliases: BTreeMap::new() }
    }
}

impl MonitorMappings {
    pub fn update(&mut self, mut inventory: Vec<MonitorIdentity>) -> Result<bool> {
        if inventory.len() > 128 {
            return Err(Error::new(
                ErrorCode::UnsupportedOperation,
                "Monitor inventory budget exceeded",
                "displays",
            ));
        }
        inventory.sort_by(|a, b| a.device.cmp(&b.device));
        for monitor in &inventory {
            monitor.work_area.validate()?;
            if monitor.device.is_empty()
                || !(48..=960).contains(&monitor.dpi)
                || inventory.iter().filter(|other| other.device == monitor.device).count() != 1
            {
                return Err(Error::new(
                    ErrorCode::AmbiguousBinding,
                    "Monitor device cannot be addressed uniquely",
                    "displays",
                ));
            }
        }
        if self.inventory == inventory {
            return Ok(false);
        }
        self.inventory = inventory;
        self.aliases.clear();
        self.epoch = new_id("topology");
        Ok(true)
    }
    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }
    #[must_use]
    pub fn inventory(&self) -> &[MonitorIdentity] {
        &self.inventory
    }

    pub fn resolve(&self, device: &str) -> Result<Id> {
        let monitor = self
            .inventory
            .iter()
            .find(|monitor| monitor.device == device)
            .ok_or_else(|| Error::new(ErrorCode::TargetMissing, "Monitor missing", device))?;
        if let Some(alias) = self.aliases.get(device) {
            return Ok(alias.clone());
        }
        if let Some(hardware) = &monitor.hardware
            && self
                .inventory
                .iter()
                .filter(|other| other.hardware.as_ref() == Some(hardware))
                .count()
                == 1
        {
            return Ok(hardware.clone());
        }
        Err(Error::new(
            ErrorCode::AmbiguousBinding,
            "Explicit current-topology monitor confirmation required",
            device,
        ))
    }

    pub fn confirm(&mut self, epoch: &str, device: &str, alias: Id) -> Result<()> {
        if epoch != self.epoch {
            return Err(Error::new(
                ErrorCode::StaleRevision,
                "Monitor topology changed; confirm the current inventory",
                "displays",
            ));
        }
        if alias.is_empty()
            || alias.len() > 200
            || !self.inventory.iter().any(|monitor| monitor.device == device)
        {
            return Err(Error::new(
                ErrorCode::InvalidConfiguration,
                "Invalid monitor confirmation",
                device,
            ));
        }
        if self.inventory.iter().any(|other| {
            other.device != device && self.resolve(&other.device).is_ok_and(|id| id == alias)
        }) {
            return Err(Error::new(
                ErrorCode::AmbiguousBinding,
                "Another current monitor already uses this identity",
                alias,
            ));
        }
        self.aliases.insert(device.into(), alias);
        Ok(())
    }

    #[must_use]
    pub fn displays(&self) -> BTreeMap<Id, Display> {
        self.inventory
            .iter()
            .filter_map(|monitor| {
                self.resolve(&monitor.device).ok().map(|id| {
                    (
                        id.clone(),
                        Display {
                            id,
                            name: monitor.name.clone(),
                            work_area: monitor.work_area,
                            dpi: monitor.dpi,
                        },
                    )
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_and_missing_hardware_need_confirmation_and_topology_change_expires_it()
    -> Result<()> {
        let one = MonitorIdentity {
            device: "one".into(),
            hardware: Some("duplicate".into()),
            name: "one".into(),
            work_area: Rect { x: 0, y: 0, width: 1000, height: 800 },
            dpi: 96,
        };
        let mut two = one.clone();
        two.device = "two".into();
        two.work_area.x = 1000;
        let mut mappings = MonitorMappings::default();
        mappings.update(vec![one.clone(), two.clone()])?;
        assert!(mappings.displays().is_empty());
        let epoch = mappings.epoch().to_owned();
        mappings.confirm(&epoch, "one", "original-one".into())?;
        assert!(mappings.confirm(&epoch, "two", "original-one".into()).is_err());
        mappings.confirm(&epoch, "two", "original-two".into())?;
        assert_eq!(mappings.displays().len(), 2);
        assert!(!mappings.update(vec![two.clone(), one.clone()])?);
        assert_eq!(mappings.displays().len(), 2);
        two.dpi = 144;
        mappings.update(vec![one, two])?;
        assert!(mappings.displays().is_empty());
        assert!(mappings.confirm(&epoch, "one", "original-one".into()).is_err());
        let mut missing = MonitorIdentity {
            device: "missing".into(),
            hardware: None,
            name: "missing".into(),
            work_area: Rect { x: 0, y: 0, width: 1000, height: 800 },
            dpi: 96,
        };
        mappings.update(vec![missing.clone()])?;
        assert!(mappings.resolve("missing").is_err());
        let epoch = mappings.epoch().to_owned();
        mappings.confirm(&epoch, "missing", "original".into())?;
        assert_eq!(mappings.resolve("missing")?, "original");
        missing.work_area.x = -1000;
        mappings.update(vec![missing])?;
        assert!(mappings.resolve("missing").is_err());
        Ok(())
    }
}
