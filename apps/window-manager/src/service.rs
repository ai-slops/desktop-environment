use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};
use window_manager_core::{
    Binding, Configuration, Error, ErrorCode, Id, ManualEdit, ObservedWindow, Plan, Request,
    Result, Runtime, Snapshot, Status, TransitionResult, UndoRecord, WindowResult, atomic_write,
    plan,
};
use windows_window_manager::{Candidate, Journal, NativeEvent, RecoveryEntry};

pub fn journal_path(path: &Path) -> PathBuf {
    path.with_extension("recovery.json")
}

pub enum Command {
    Refresh(Configuration),
    Bind(Configuration, Id, Candidate),
    Preview(Configuration, Request),
    WindowAction(Configuration, window_manager_core::WindowAction),
    Apply(Configuration, Plan),
    Undo(Id),
    Promote(Id, Id, bool, bool),
    ProtectPresentation(Id, window_manager_core::Protection),
    PlaceControl(window_manager_core::Rect),
    AttentionTarget(Id, window_manager_core::Target),
    Pause(bool),
    Recover,
    Provider(ProviderCommand),
    Barrier(Id),
    Shutdown,
}

pub enum ProviderCommand {
    Register(Id, Id, std::collections::BTreeSet<Id>, bool, bool),
    Publish(window_manager_core::ProviderMessage),
    Disconnect(Id),
}

pub enum Event {
    Inventory(Vec<Candidate>),
    State(Snapshot, Runtime),
    Preview(Box<Plan>),
    Result(TransitionResult),
    Shortcut(u32),
    Error(Error),
    Notice(String),
    ManualEdit(ManualEdit),
    ControlReady(window_manager_core::Rect, bool),
    Barrier(Id),
}

pub struct Worker {
    pub sender: Sender<Command>,
    pub receiver: Receiver<Event>,
    finished: Receiver<()>,
}

impl Worker {
    pub fn start(path: PathBuf, config: Configuration) -> Self {
        Self::start_with_hotkeys(path, config, true, true)
    }

    pub fn start_session(path: PathBuf, config: Configuration, recover_on_exit: bool) -> Self {
        Self::start_with_hotkeys(path, config, false, recover_on_exit)
    }

    fn start_with_hotkeys(
        path: PathBuf,
        config: Configuration,
        hotkeys: bool,
        recover_on_exit: bool,
    ) -> Self {
        let (sender, commands) = mpsc::channel();
        let (events, receiver) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        std::thread::spawn(move || {
            run(&path, config, &commands, events, hotkeys, recover_on_exit);
            let _ = done.send(());
        });
        Self { sender, receiver, finished }
    }

    pub fn shutdown(&self) {
        let _ = self.sender.send(Command::Shutdown);
        // Recovery normally completes before GUI exit; the watchdog remains the crash fallback.
        let _ = self.finished.recv_timeout(Duration::from_secs(2));
    }
}

struct State {
    config: Configuration,
    bindings: BTreeMap<Id, Binding>,
    snapshot: Snapshot,
    runtime: Runtime,
    journal: Journal,
    path: PathBuf,
    events: Sender<Event>,
    undo: Vec<UndoRecord>,
    undoing: Option<Id>,
    gestures: BTreeMap<Id, ObservedWindow>,
}

impl State {
    fn refresh(&mut self) -> Result<()> {
        let displays = windows_window_manager::displays()?;
        if self.snapshot.displays != displays {
            self.snapshot.topology_revision += 1;
            self.snapshot.displays = displays;
        }
        self.snapshot.windows.clear();
        self.snapshot.now_ms = u64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(u64::MAX);
        let foreground = windows_window_manager::foreground_handle();
        self.snapshot.focused = self
            .bindings
            .iter()
            .find(|(_, binding)| binding.handle == foreground)
            .map(|(id, _)| id.clone());
        for (id, binding) in &self.bindings {
            if let Some(reference) = self.config.windows.get(id)
                && let Ok(observed) = windows_window_manager::observe(binding, reference.allow_hide)
            {
                self.snapshot.windows.insert(id.clone(), observed);
            }
        }
        Ok(())
    }

    fn publish(&self) {
        let _ = self.events.send(Event::State(self.snapshot.clone(), self.runtime.clone()));
    }

    #[allow(clippy::too_many_lines)] // Journal, submission, and settlement are one execution transaction.
    fn apply(&mut self, plan: &Plan) -> Result<()> {
        if !self.gestures.is_empty() {
            return Err(Error::new(
                ErrorCode::PermissionDenied,
                "A user move/size gesture is in progress; retry after it ends",
                &plan.id,
            ));
        }
        self.refresh()?;
        plan.revalidate(&self.config, &self.runtime, &self.snapshot)?;
        let prior_runtime = self.runtime.clone();
        if plan.idempotent {
            let _ = self.events.send(Event::Result(TransitionResult {
                request: plan.id.clone(),
                status: Status::Settled,
                windows: Vec::new(),
                rendering_readiness: "unknown".into(),
            }));
            return Ok(());
        }
        // Persist every prior visible state BEFORE submitting the first hide.
        let mut journal = self.journal.clone();
        for mutation in &plan.mutations {
            if mutation.visible == Some(false) {
                let prior = &plan.expected[&mutation.window];
                if !journal.entries.iter().any(|entry| entry.prior.binding == prior.binding) {
                    journal.entries.push(RecoveryEntry {
                        window: mutation.window.clone(),
                        prior: prior.clone(),
                        request: plan.id.clone(),
                    });
                }
            }
        }
        journal.save(&self.path)?;
        self.journal = journal;
        let submission = windows_window_manager::submit_many(&plan.mutations);
        let mut outcomes = Vec::new();
        for mutation in &plan.mutations {
            let error = submission
                .results
                .get(&mutation.window)
                .and_then(|result| result.as_ref().err())
                .cloned();
            outcomes.push(WindowResult {
                window: mutation.window.clone(),
                submitted: error.is_none(),
                settled: false,
                error,
            });
        }
        let deadline = Instant::now() + Duration::from_millis(900);
        loop {
            self.refresh()?;
            for (mutation, outcome) in plan.mutations.iter().zip(&mut outcomes) {
                if !outcome.submitted {
                    continue;
                }
                outcome.settled =
                    self.snapshot.windows.get(&mutation.window).is_some_and(|observed| {
                        observed.binding == mutation.binding
                            && mutation.geometry.is_none_or(|frame| observed.frame == frame)
                            && mutation.visible.is_none_or(|visible| observed.visible == visible)
                            && mutation.show_state.is_none_or(|state| observed.show_state == state)
                            && (!plan
                                .desired
                                .get(&mutation.window)
                                .is_some_and(|desired| desired.strict_size)
                                || observed.client == plan.expected[&mutation.window].client)
                    });
            }
            if outcomes.iter().all(|outcome| outcome.settled || !outcome.submitted)
                || Instant::now() >= deadline
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let settled = outcomes.iter().all(|outcome| outcome.settled);
        for outcome in &mut outcomes {
            if outcome.submitted && !outcome.settled {
                outcome.error = Some(Error::new(
                    ErrorCode::ApplicationTimeout,
                    "Native operation did not settle within 900ms; enforcement paused",
                    &outcome.window,
                ));
            }
        }
        let mut failed_windows = std::collections::BTreeSet::new();
        for slot in &plan.scope {
            let component = plan.component(slot, &prior_runtime);
            let component_settled = component.mutations.iter().all(|mutation| {
                outcomes.iter().any(|outcome| outcome.window == mutation.window && outcome.settled)
            });
            if component_settled {
                component.commit(&mut self.runtime);
                if self.undoing.as_ref() != Some(&plan.id) {
                    self.undo.push(UndoRecord::capture(
                        &component,
                        &prior_runtime,
                        &self.snapshot,
                        &self.runtime,
                    ));
                    if self.undo.len() > 50 {
                        self.undo.remove(0);
                    }
                }
            } else {
                self.runtime.suspended.insert(slot.clone());
                self.runtime.claims.retain(|_, claim| &claim.slot != slot);
                self.runtime.presentations.remove(slot);
                self.runtime.geometry.retain(|_, desired| &desired.slot != slot);
                *self.runtime.generations.entry(slot.clone()).or_default() += 1;
                failed_windows
                    .extend(component.mutations.iter().map(|mutation| mutation.window.clone()));
            }
        }
        if settled {
            if self.undoing.as_ref() == Some(&plan.id) {
                self.undo.pop();
                self.undoing = None;
                if let Some(record) = self.undo.last_mut() {
                    record.rebase_after_undo(&self.runtime, &self.snapshot);
                }
            }
            self.journal.entries.retain(|entry| {
                self.snapshot.windows.get(&entry.window).is_none_or(|observed| !observed.visible)
            });
            self.journal.save(&self.path)?;
        } else {
            // Failed slots are suspended independently; successful slots retain their claims.
            let _ = windows_window_manager::recover_selected(&self.path, Some(&failed_windows));
            self.journal = Journal::load(&self.path)?;
        }
        // Focus succeeds independently of unrelated failed components.
        for (mutation, outcome) in plan.mutations.iter().zip(&mut outcomes) {
            if mutation.focus && outcome.settled {
                outcome.error = windows_window_manager::focus(&mutation.binding).err();
            }
        }
        let result = TransitionResult {
            request: plan.id.clone(),
            status: if settled {
                Status::Settled
            } else if outcomes.iter().any(|outcome| outcome.settled) {
                Status::PartiallyApplied
            } else {
                Status::Failed
            },
            windows: outcomes,
            rendering_readiness: "unknown".into(),
        };
        // Redacted diagnostics: opaque IDs, operation counts, scope, revisions; no titles/screenshots.
        let record = serde_json::json!({"request": plan.id, "scope": plan.scope, "configuration_revision": plan.config_revision, "topology_revision": plan.topology_revision, "impact": plan.impact, "submission": {"batched_windows": submission.batched_windows, "individual_windows": submission.individual_windows}, "result": result});
        let log_path = self.path.with_extension("last-transition.json");
        atomic_write(
            &log_path,
            &serde_json::to_vec_pretty(&record)
                .map_err(|error| Error::new(ErrorCode::StorageFailure, error.to_string(), "log"))?,
        )?;
        let _ = self.events.send(Event::Result(result));
        self.publish();
        Ok(())
    }
}

#[allow(clippy::too_many_lines)] // This dispatch loop owns the complete worker lifecycle.
fn run(
    path: &Path,
    config: Configuration,
    commands: &Receiver<Command>,
    events: Sender<Event>,
    hotkeys: bool,
    recover_on_exit: bool,
) {
    let journal_path = journal_path(path);
    let journal = match Journal::load(&journal_path) {
        Ok(journal) => journal,
        Err(error) => {
            let _ = events.send(Event::Error(error));
            return;
        }
    };
    // Resume only lifetime-token matches from our own recovery journal; other references require explicit binding.
    let bindings = journal
        .entries
        .iter()
        .filter(|entry| config.windows.contains_key(&entry.window))
        .map(|entry| (entry.window.clone(), entry.prior.binding.clone()))
        .collect();
    let shortcuts: Vec<_> =
        config.shortcuts.iter().filter(|_| hotkeys).map(|shortcut| shortcut.number).collect();
    let native_events = windows_window_manager::event_stream(&shortcuts);
    let mut state = State {
        config,
        bindings,
        snapshot: Snapshot::default(),
        runtime: Runtime::default(),
        journal,
        path: journal_path,
        events,
        undo: Vec::new(),
        undoing: None,
        gestures: BTreeMap::new(),
    };
    if let Err(error) = state.refresh() {
        let _ = state.events.send(Event::Error(error));
    }
    if let Ok(inventory) = windows_window_manager::inventory() {
        let _ = state.events.send(Event::Inventory(inventory));
    }
    state.publish();
    let mut dirty = false;
    let mut refreshed = Instant::now();
    loop {
        while let Ok(event) = native_events.try_recv() {
            match event {
                NativeEvent::GestureStarted(handle) => {
                    if let Some((id, observed)) = state
                        .snapshot
                        .windows
                        .iter()
                        .find(|(_, observed)| observed.binding.handle == handle)
                    {
                        state.gestures.insert(id.clone(), observed.clone());
                    }
                }
                NativeEvent::GestureEnded(handle) => {
                    let id = state
                        .bindings
                        .iter()
                        .find(|(_, binding)| binding.handle == handle)
                        .map(|(id, _)| id.clone());
                    if let Some(id) = id
                        && let Some(before) = state.gestures.remove(&id)
                    {
                        let _ = state.refresh();
                        if let Some(desired) = state.runtime.geometry.get(&id)
                            && let Some(presentation) =
                                state.runtime.presentations.get(&desired.slot)
                            && let Some(after) = state.snapshot.windows.get(&id)
                            && let Some(edit) = ManualEdit::observed(
                                presentation.view.clone(),
                                desired,
                                &before,
                                after,
                            )
                        {
                            let _ = state.events.send(Event::ManualEdit(edit));
                        }
                        state.publish();
                    }
                }
                NativeEvent::Changed(handle) => {
                    if state.bindings.values().any(|binding| binding.handle == handle) {
                        dirty = true;
                    }
                }
                NativeEvent::Shortcut(number) => {
                    let _ = state.events.send(Event::Shortcut(number));
                }
                NativeEvent::RegistrationError(message) => {
                    let _ = state.events.send(Event::Notice(message));
                }
            }
        }
        if dirty && refreshed.elapsed() >= Duration::from_millis(150) {
            let _ = state.refresh();
            state.publish();
            refreshed = Instant::now();
            dirty = false;
        }
        let command = match commands.recv_timeout(Duration::from_millis(50)) {
            Ok(command) => command,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let result = match command {
            Command::Refresh(config) => {
                state.config = config;
                let result = state.refresh();
                if let Ok(inventory) = windows_window_manager::inventory() {
                    let _ = state.events.send(Event::Inventory(inventory));
                }
                state.publish();
                result
            }
            Command::Bind(config, id, candidate) => {
                state.config = config;
                match state.config.windows.get(&id) {
                    Some(_)
                        if state.bindings.iter().any(|(other, binding)| {
                            other != &id
                                && binding.handle == candidate.handle
                                && windows_window_manager::validate_binding(binding).is_ok()
                        }) =>
                    {
                        Err(Error::new(
                            ErrorCode::AmbiguousBinding,
                            "This real window is already registered; reuse its existing Window reference",
                            id,
                        ))
                    }
                    Some(reference) => {
                        windows_window_manager::bind(&candidate, reference.allow_hide).and_then(
                            |observed| {
                                state.bindings.insert(id, observed.binding);
                                state.refresh()?;
                                state.publish();
                                Ok(())
                            },
                        )
                    }
                    None => {
                        Err(Error::new(ErrorCode::TargetMissing, "Window reference missing", id))
                    }
                }
            }
            Command::WindowAction(config, action) => {
                state.config = config;
                state.refresh().and_then(|()| {
                    let plan = window_manager_core::plan_window_action(
                        &state.config,
                        &state.runtime,
                        &state.snapshot,
                        &action,
                    )?;
                    let _ = state.events.send(Event::Preview(Box::new(plan)));
                    Ok(())
                })
            }
            Command::Preview(config, request) => {
                state.config = config;
                state.refresh().and_then(|()| {
                    let plan = plan(&state.config, &state.runtime, &state.snapshot, &request)?;
                    let _ = state.events.send(Event::Preview(Box::new(plan)));
                    Ok(())
                })
            }
            Command::Apply(config, plan) => {
                state.config = config;
                state.apply(&plan)
            }
            Command::Undo(id) => state.refresh().and_then(|()| {
                let record = state.undo.last().ok_or_else(|| {
                    Error::new(ErrorCode::TargetMissing, "No native transition to undo", &id)
                })?;
                let mut reversed =
                    record.reverse(&state.config, &state.runtime, &state.snapshot)?;
                reversed.id.clone_from(&id);
                state.undoing = Some(id);
                let _ = state.events.send(Event::Preview(Box::new(reversed)));
                Ok(())
            }),
            Command::Promote(slot, window, position, size) => {
                state.runtime.promote_properties(&slot, &window, position, size);
                state.publish();
                Ok(())
            }
            Command::ProtectPresentation(slot, protection) => {
                if let Some(presentation) = state.runtime.presentations.get_mut(&slot) {
                    presentation.protection = protection;
                    *state.runtime.generations.entry(slot).or_default() += 1;
                    state.publish();
                    Ok(())
                } else {
                    Err(Error::new(ErrorCode::TargetMissing, "Presentation is not active", slot))
                }
            }
            Command::PlaceControl(bounds) => {
                let result = windows_window_manager::position_control(bounds);
                let _ = state.events.send(Event::ControlReady(bounds, result.is_ok()));
                result
            }
            Command::AttentionTarget(window, target) => {
                state.config.validate_target(&target).map(|()| {
                    state.runtime.attention_targets.insert(window, target);
                    state.publish();
                })
            }
            Command::Pause(paused) => {
                state.runtime.paused = paused;
                if !paused {
                    state.runtime.suspended.clear();
                }
                state.publish();
                Ok(())
            }
            Command::Recover => {
                windows_window_manager::recover(&state.path).and_then(|diagnostics| {
                    let _ = state.events.send(Event::Notice(diagnostics.join("\n")));
                    state.journal = Journal::load(&state.path)?;
                    state.runtime.claims.clear();
                    for slot in state.runtime.presentations.keys() {
                        *state.runtime.generations.entry(slot.clone()).or_default() += 1;
                    }
                    state.runtime.presentations.clear();
                    state.runtime.geometry.clear();
                    state.refresh()?;
                    state.publish();
                    Ok(())
                })
            }
            Command::Provider(command) => {
                let result = match command {
                    ProviderCommand::Register(id, session, resources, output, attention) => {
                        if resources.iter().any(|id| !state.config.windows.contains_key(id)) {
                            Err(Error::new(
                                ErrorCode::TargetMissing,
                                "Provider resource missing",
                                id,
                            ))
                        } else {
                            state
                                .runtime
                                .providers
                                .register(id, session, resources, output, attention)
                        }
                    }
                    ProviderCommand::Publish(message) => {
                        state.runtime.providers.accept(message, window_manager_core::unix_millis())
                    }
                    ProviderCommand::Disconnect(id) => {
                        state.runtime.providers.disconnect(&id);
                        Ok(())
                    }
                };
                state.publish();
                result
            }
            Command::Barrier(id) => {
                let _ = state.events.send(Event::Barrier(id));
                Ok(())
            }
            Command::Shutdown => break,
        };
        if let Err(error) = result {
            let _ = state.events.send(Event::Error(error));
        }
    }
    if recover_on_exit && let Err(error) = windows_window_manager::recover(&state.path) {
        let _ = state.events.send(Event::Error(error));
    }
}
