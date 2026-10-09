use std::collections::BTreeMap;
use window_manager_core::Id;

#[derive(Default)]
pub struct ReflowScheduler {
    pending: BTreeMap<Id, (u64, u64)>,
    finished: BTreeMap<Id, u64>,
}
impl ReflowScheduler {
    pub fn retain(&mut self, slots: &std::collections::BTreeSet<Id>) {
        self.pending.retain(|slot, _| slots.contains(slot));
        self.finished.retain(|slot, _| slots.contains(slot));
    }
    pub fn completed(&self, slot: &str, fingerprint: u64) -> bool {
        self.finished.get(slot) == Some(&fingerprint)
    }
    pub fn ready(&mut self, slot: &str, fingerprint: u64, now_ms: u64) -> bool {
        if self.finished.get(slot) == Some(&fingerprint) {
            return false;
        }
        let pending = self.pending.entry(slot.into()).or_insert((fingerprint, now_ms));
        if pending.0 != fingerprint || pending.1 > now_ms {
            *pending = (fingerprint, now_ms);
        }
        if now_ms.saturating_sub(pending.1) < 250 {
            return false;
        }
        self.finished.insert(slot.into(), fingerprint);
        self.pending.remove(slot);
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stable_inputs_debounce_once_and_unrelated_slots_do_not_restart_each_other() {
        let mut scheduler = ReflowScheduler::default();
        assert!(!scheduler.ready("a", 1, 0));
        assert!(!scheduler.ready("a", 2, 200));
        assert!(!scheduler.ready("b", 7, 200));
        assert!(!scheduler.ready("a", 2, 449));
        assert!(scheduler.ready("a", 2, 450));
        assert!(scheduler.ready("b", 7, 450));
        assert!(!scheduler.ready("a", 2, 1000));
        assert!(!scheduler.ready("a", 3, 1000));
        assert!(scheduler.ready("a", 3, 1250));
    }
}
