use crate::{Candidate, Journal};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::mpsc::{self, Receiver};
use window_manager_core::{
    Binding, Display, Error, ErrorCode, Id, Mutation, ObservedWindow, Result,
};

fn unsupported<T>() -> Result<T> {
    Err(Error::new(
        ErrorCode::UnsupportedOperation,
        "Native window management is supported on Windows only",
        "platform",
    ))
}
#[must_use]
pub const fn foreground_handle() -> u64 {
    0
}
pub fn position_control(_: window_manager_core::Rect) -> Result<()> {
    unsupported()
}
#[must_use]
pub const fn control_fits(_: window_manager_core::Rect) -> bool {
    false
}
pub fn inventory() -> Result<Vec<Candidate>> {
    unsupported()
}
pub fn displays() -> Result<BTreeMap<Id, Display>> {
    unsupported()
}
pub fn monitor_inventory() -> Result<Vec<crate::MonitorIdentity>> {
    unsupported()
}
pub fn observe_mapped(_: &Binding, _: bool, _: &crate::MonitorMappings) -> Result<ObservedWindow> {
    unsupported()
}
pub fn bind(_: &Candidate, _: bool) -> Result<ObservedWindow> {
    unsupported()
}
pub fn bind_mapped(_: &Candidate, _: bool, _: &crate::MonitorMappings) -> Result<ObservedWindow> {
    unsupported()
}
pub fn observe(_: &Binding, _: bool) -> Result<ObservedWindow> {
    unsupported()
}
pub fn validate_binding(_: &Binding) -> Result<()> {
    unsupported()
}
pub fn submit(_: &Mutation) -> Result<()> {
    unsupported()
}
#[must_use]
pub fn submit_many(mutations: &[Mutation]) -> crate::SubmissionReport {
    crate::SubmissionReport {
        results: mutations
            .iter()
            .map(|mutation| (mutation.window.clone(), unsupported()))
            .collect(),
        batched_windows: 0,
        individual_windows: mutations.len(),
    }
}
pub fn focus(_: &Binding) -> Result<()> {
    unsupported()
}
pub fn recover(_: &Path) -> Result<Vec<String>> {
    unsupported()
}
pub fn recover_selected(
    _: &Path,
    _: Option<&std::collections::BTreeSet<Id>>,
) -> Result<Vec<String>> {
    unsupported()
}
#[must_use]
pub const fn parent_alive(_: u32, _: u64) -> bool {
    false
}
pub fn current_process_started() -> Result<u64> {
    unsupported()
}
pub fn process_started(_: u32) -> Result<u64> {
    unsupported()
}

#[derive(Clone, Debug)]
pub enum NativeEvent {
    Changed(u64),
    GestureStarted(u64),
    GestureEnded(u64),
    Shortcut(u32),
    RegistrationError(String),
}

pub fn event_stream(_: &[u32]) -> Receiver<NativeEvent> {
    let (sender, receiver) = mpsc::channel();
    let _ = sender.send(NativeEvent::RegistrationError("Global shortcuts require Windows".into()));
    receiver
}

// This type is used by the shared facade even without a native adapter.
const _: Option<Journal> = None;
