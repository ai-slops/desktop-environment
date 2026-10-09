use crate::{
    Configuration, Error, ErrorCode, Id, OutputState, Rect, Result, Runtime, ShowState, Snapshot,
    TransitionResult, new_id,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EventCursor {
    pub epoch: Id,
    pub sequence: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DomainEvent {
    StateChanged { revision: u64, slots: BTreeSet<Id> },
    TopologyChanged { revision: u64 },
    Transition { result: TransitionResult },
    OperationFailed { error: Error },
    ProviderChanged { provider: Id },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventEnvelope {
    pub cursor: EventCursor,
    pub event: DomainEvent,
}

/// Bounded domain history. Native callbacks and handles never form this protocol.
pub struct EventLog {
    epoch: Id,
    sequence: u64,
    events: VecDeque<EventEnvelope>,
}
impl Default for EventLog {
    fn default() -> Self {
        Self { epoch: new_id("events"), sequence: 0, events: VecDeque::new() }
    }
}

impl EventLog {
    #[must_use]
    pub fn cursor(&self) -> EventCursor {
        EventCursor { epoch: self.epoch.clone(), sequence: self.sequence }
    }
    pub fn push(&mut self, event: DomainEvent) {
        if self.sequence == u64::MAX {
            self.epoch = new_id("events");
            self.sequence = 0;
            self.events.clear();
        }
        self.sequence += 1;
        self.events.push_back(EventEnvelope { cursor: self.cursor(), event });
        if self.events.len() > 256 {
            self.events.pop_front();
        }
    }
    pub fn since(&self, cursor: &EventCursor) -> Result<Vec<EventEnvelope>> {
        if cursor.epoch != self.epoch
            || cursor.sequence > self.sequence
            || self
                .events
                .front()
                .is_some_and(|first| cursor.sequence.saturating_add(1) < first.cursor.sequence)
        {
            return Err(Error::new(
                ErrorCode::StaleRevision,
                "Event interval unavailable; request a fresh snapshot and use its cursor",
                "events",
            ));
        }
        Ok(self
            .events
            .iter()
            .filter(|event| event.cursor.sequence > cursor.sequence)
            .cloned()
            .collect())
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicWindow {
    pub available: bool,
    pub frame: Option<Rect>,
    pub client: Option<[i32; 2]>,
    pub display: Option<Id>,
    pub dpi: Option<u32>,
    pub visible: Option<bool>,
    pub show_state: Option<ShowState>,
    pub slot: Option<Id>,
    pub placement: Option<Id>,
    pub capabilities: PublicCapabilities,
    pub owned_dialog: bool,
    pub output: OutputState,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicCapabilities {
    pub move_supported: bool,
    pub resize_supported: bool,
    pub hide_supported: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicPresentation {
    pub view: Id,
    pub root: String,
    pub visit: Id,
    pub display: Id,
    pub selected_tabs: BTreeMap<Id, Id>,
    pub variants: BTreeMap<Id, Id>,
    pub filter: Option<crate::Query>,
    pub expansion: Option<crate::Expansion>,
    pub area: crate::Rect,
}

#[derive(Clone, Debug, Serialize)]
pub struct PublicSnapshot {
    pub revision: u64,
    pub topology_revision: u64,
    pub displays: BTreeMap<Id, crate::Display>,
    pub cursor: EventCursor,
    pub paused: bool,
    pub suspended_slots: BTreeSet<Id>,
    pub windows: BTreeMap<Id, PublicWindow>,
    pub presentations: BTreeMap<Id, PublicPresentation>,
}

impl PublicSnapshot {
    #[must_use]
    pub fn capture(
        config: &Configuration,
        runtime: &Runtime,
        snapshot: &Snapshot,
        cursor: EventCursor,
    ) -> Self {
        Self {
            revision: config.revision,
            topology_revision: snapshot.topology_revision,
            displays: snapshot.displays.clone(),
            cursor,
            paused: runtime.paused,
            suspended_slots: runtime.suspended.clone(),
            windows: config
                .windows
                .keys()
                .map(|id| {
                    let window = snapshot.windows.get(id);
                    let claim = runtime.claims.get(id);
                    (
                        id.clone(),
                        PublicWindow {
                            available: window.is_some(),
                            frame: window.map(|window| window.frame),
                            client: window.map(|window| window.client),
                            display: window.map(|window| window.display.clone()),
                            dpi: window.map(|window| window.dpi),
                            visible: window.map(|window| window.visible),
                            show_state: window.map(|window| window.show_state),
                            slot: claim.map(|claim| claim.slot.clone()),
                            placement: claim.map(|claim| claim.placement.clone()),
                            capabilities: PublicCapabilities {
                                move_supported: window.is_some_and(|window| window.can_move),
                                resize_supported: window.is_some_and(|window| window.can_resize),
                                hide_supported: window.is_some_and(|window| window.can_hide),
                            },
                            owned_dialog: window.is_some_and(|window| window.has_owned_dialog),
                            output: runtime.providers.output(id, snapshot.now_ms),
                        },
                    )
                })
                .collect(),
            presentations: runtime
                .presentations
                .iter()
                .map(|(slot, presentation)| {
                    (
                        slot.clone(),
                        PublicPresentation {
                            view: presentation.view.clone(),
                            root: presentation.root.clone(),
                            visit: presentation.visit.clone(),
                            display: presentation.context_display.clone(),
                            selected_tabs: presentation.selected_tabs.clone(),
                            variants: presentation.variants.clone(),
                            filter: presentation.filter.clone(),
                            expansion: presentation.expansion.clone(),
                            area: presentation.context_area,
                        },
                    )
                })
                .collect(),
        }
    }
}

#[must_use]
pub fn unix_millis() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_gap_restart_and_future_cursor_require_snapshot_resynchronization() {
        let mut log = EventLog::default();
        let initial = log.cursor();
        for revision in 0..257 {
            log.push(DomainEvent::TopologyChanged { revision });
        }
        assert!(log.since(&initial).is_err());
        let cursor = EventCursor { epoch: initial.epoch.clone(), sequence: 1 };
        assert!(log.since(&cursor).is_ok_and(|events| events.len() == 256));
        assert!(log.since(&EventCursor { epoch: initial.epoch, sequence: 258 }).is_err());
        assert!(EventLog::default().since(&log.cursor()).is_err());
        assert!(log.since(&log.cursor()).is_ok_and(|events| events.is_empty()));
    }

    #[test]
    fn public_snapshot_contains_no_native_identity() {
        let config = Configuration::default();
        let snapshot = PublicSnapshot::capture(
            &config,
            &Runtime::default(),
            &Snapshot::default(),
            EventLog::default().cursor(),
        );
        let value = serde_json::to_string(&snapshot).unwrap_or_default();
        for forbidden in ["handle", "binding", "process_started", "token_property", "title"] {
            assert!(!value.contains(forbidden));
        }
        assert!(value.contains("topology_revision"));
    }
}
