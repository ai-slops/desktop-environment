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

    fn monitor(device: &str, hardware: Option<&str>, dpi: u32) -> MonitorIdentity {
        MonitorIdentity {
            device: device.into(),
            hardware: hardware.map(str::to_owned),
            name: device.into(),
            work_area: Rect { x: 0, y: 0, width: 2560, height: 1440 },
            dpi,
        }
    }

    #[test]
    fn adding_and_removing_mixed_scale_monitors_preserves_unique_hardware_identity() -> Result<()> {
        for dpi in [96, 120, 144, 168, 192] {
            let one = monitor("one", Some("hardware-one"), 96);
            let mut added = monitor("added", Some("hardware-added"), dpi);
            added.work_area = Rect { x: -2160, y: -300, width: 2160, height: 3840 };
            let mut mappings = MonitorMappings::default();
            mappings.update(vec![one.clone()])?;
            let epoch = mappings.epoch().to_owned();
            assert!(mappings.update(vec![added.clone(), one.clone()])?);
            assert_ne!(mappings.epoch(), epoch);
            assert_eq!(mappings.resolve("one")?, "hardware-one");
            assert_eq!(mappings.displays()["hardware-added"].dpi, dpi);
            assert_eq!(mappings.displays()["hardware-added"].work_area, added.work_area);
            let epoch = mappings.epoch().to_owned();
            assert!(!mappings.update(vec![one.clone(), added])?);
            assert_eq!(mappings.epoch(), epoch);
            mappings.update(vec![one])?;
            assert_eq!(mappings.resolve("one")?, "hardware-one");
            assert_eq!(
                mappings.resolve("added").err().map(|e| e.code),
                Some(ErrorCode::TargetMissing)
            );
        }
        Ok(())
    }

    #[test]
    fn scale_work_area_orientation_and_hotplug_expire_manual_confirmation() -> Result<()> {
        let original = monitor("one", None, 96);
        let mut scale = original.clone();
        scale.dpi = 120;
        let mut taskbar = original.clone();
        taskbar.work_area.height -= 48;
        let mut portrait = original.clone();
        portrait.work_area = Rect { x: -1440, y: -200, width: 1440, height: 2560 };
        for inventory in [
            vec![scale],
            vec![taskbar],
            vec![portrait],
            vec![original.clone(), monitor("added", Some("second"), 192)],
            vec![],
        ] {
            let mut mappings = MonitorMappings::default();
            mappings.update(vec![original.clone()])?;
            let epoch = mappings.epoch().to_owned();
            mappings.confirm(&epoch, "one", "saved-slot".into())?;
            mappings.update(inventory)?;
            assert_eq!(
                mappings.confirm(&epoch, "one", "saved-slot".into()).err().map(|e| e.code),
                Some(ErrorCode::StaleRevision)
            );
            assert!(!mappings.displays().contains_key("saved-slot"));
            // Reconnection at the original scale still requires a fresh confirmation.
            mappings.update(vec![original.clone()])?;
            assert_eq!(
                mappings.resolve("one").err().map(|e| e.code),
                Some(ErrorCode::AmbiguousBinding)
            );
            let current = mappings.epoch().to_owned();
            mappings.confirm(&current, "one", "saved-slot".into())?;
            assert_eq!(mappings.displays()["saved-slot"].dpi, 96);
        }
        Ok(())
    }

    #[test]
    fn identical_hardware_with_different_scales_is_not_disambiguated_by_geometry() -> Result<()> {
        let one = monitor("one", Some("same-hardware"), 96);
        let mut two = monitor("two", Some("same-hardware"), 192);
        two.work_area.x = -2560;
        let mut mappings = MonitorMappings::default();
        mappings.update(vec![one, two])?;
        assert!(mappings.displays().is_empty());
        let epoch = mappings.epoch().to_owned();
        mappings.confirm(&epoch, "one", "left".into())?;
        mappings.confirm(&epoch, "two", "right".into())?;
        assert_eq!(mappings.displays()["left"].dpi, 96);
        assert_eq!(mappings.displays()["right"].dpi, 192);
        Ok(())
    }

    #[test]
    fn invalid_inventory_does_not_discard_a_confirmed_mapping() -> Result<()> {
        let original = monitor("one", None, 96);
        let mut mappings = MonitorMappings::default();
        mappings.update(vec![original.clone()])?;
        let epoch = mappings.epoch().to_owned();
        mappings.confirm(&epoch, "one", "saved-slot".into())?;
        for dpi in [0, 47, 961, u32::MAX] {
            let mut invalid = original.clone();
            invalid.dpi = dpi;
            assert!(mappings.update(vec![invalid]).is_err());
            assert_eq!(mappings.epoch(), epoch);
            assert_eq!(mappings.resolve("one")?, "saved-slot");
        }
        assert!(mappings.update(vec![original.clone(), original]).is_err());
        assert_eq!(mappings.epoch(), epoch);
        assert_eq!(mappings.resolve("one")?, "saved-slot");
        Ok(())
    }

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
