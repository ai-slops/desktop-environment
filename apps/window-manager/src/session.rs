//! Local JSON Lines transport over inherited pipes. Capabilities come only from startup flags.
use crate::service::{Command, Event, ProviderCommand, Worker};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{BufRead, Read, Write};
use std::path::Path;
use std::time::Duration;
use window_manager_core::{
    Configuration, DomainEvent, Error, ErrorCode, EventCursor, EventLog, Id, Plan, ProviderMessage,
    PublicSnapshot, Request, Result, Runtime, Snapshot, WindowAction, new_id,
};
use windows_window_manager::Candidate;

const MAX_LINE: u64 = 65_536;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    id: Id,
    command: SessionCommand,
}

#[derive(Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum SessionCommand {
    Snapshot {},
    Configuration {},
    Simulate {
        input: window_manager_core::Simulation,
    },
    Inventory {},
    MonitorInventory {},
    ConfirmMonitor {
        epoch: Id,
        device: String,
        alias: Id,
    },
    Events {
        cursor: EventCursor,
    },
    Bind {
        expected_revision: u64,
        window: Id,
        candidate: Id,
    },
    Preview {
        request: Request,
    },
    Collapse {
        slot: Id,
    },
    WindowAction {
        action: WindowAction,
    },
    Apply {
        plan: Id,
    },
    Undo {
        id: Id,
    },
    Pause {
        paused: bool,
    },
    Recover {},
    Structure {
        edit: window_manager_core::StructureEdit,
    },
    ConfigurationUndo {
        expected_revision: u64,
    },
    ProviderRegister {
        provider: Id,
        session: Id,
        resources: BTreeSet<Id>,
        output: bool,
        attention: bool,
    },
    ProviderPublish {
        message: ProviderMessage,
    },
    ProviderDisconnect {
        provider: Id,
    },
}

impl SessionCommand {
    const fn cacheable(&self) -> bool {
        !matches!(
            self,
            Self::Snapshot {}
                | Self::Configuration {}
                | Self::Inventory {}
                | Self::MonitorInventory {}
                | Self::Events { .. }
                | Self::Simulate { .. }
        )
    }
    fn authorize(&self, control: bool, providers: bool) -> Result<()> {
        let permitted = match self {
            Self::Snapshot {}
            | Self::Simulate { .. }
            | Self::Configuration {}
            | Self::Inventory {}
            | Self::MonitorInventory {}
            | Self::Events { .. } => true,
            Self::ProviderRegister { .. }
            | Self::ProviderPublish { .. }
            | Self::ProviderDisconnect { .. } => providers,
            _ => control,
        };
        if permitted {
            Ok(())
        } else {
            Err(Error::new(
                ErrorCode::PermissionDenied,
                "This session lacks the required startup capability grant",
                "session",
            ))
        }
    }
}

struct Session {
    worker: Worker,
    config: Configuration,
    snapshot: Snapshot,
    runtime: Runtime,
    log: EventLog,
    plans: BTreeMap<Id, Plan>,
    candidates: BTreeMap<Id, Candidate>,
    control: bool,
    providers: bool,
    path: std::path::PathBuf,
    history: Vec<Configuration>,
    monitor_epoch: Id,
    monitors: Vec<windows_window_manager::MonitorIdentity>,
    replay: crate::replay::ReplayCache,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.worker.shutdown();
    }
}

impl Session {
    fn receive(&mut self, envelope: Envelope) -> Value {
        let cacheable = envelope.command.cacheable();
        let fingerprint = match serde_json::to_vec(&envelope.command) {
            Ok(fingerprint) => fingerprint,
            Err(error) => {
                return json!({"id":envelope.id,"error":Error::new(ErrorCode::InvalidConfiguration,error.to_string(),"session"),"cursor":self.log.cursor()});
            }
        };
        if cacheable {
            match self.replay.lookup(&envelope.id, &fingerprint) {
                Ok(Some(response)) => return response,
                Err(error) => {
                    return json!({"id":envelope.id,"error":error,"cursor":self.log.cursor()});
                }
                Ok(None) => {}
            }
        }
        let response = match self.execute(envelope.command) {
            Ok(result) => json!({"id":envelope.id,"result":result,"cursor":self.log.cursor()}),
            Err(error) => json!({"id":envelope.id,"error":error,"cursor":self.log.cursor()}),
        };
        if cacheable {
            self.replay.insert(envelope.id, fingerprint, &response);
        }
        response
    }
    fn exchange(&mut self, command: Command) -> Result<Vec<Event>> {
        let barrier = new_id("barrier");
        self.worker.sender.send(command).map_err(|_| unavailable())?;
        self.worker.sender.send(Command::Barrier(barrier.clone())).map_err(|_| unavailable())?;
        let mut replies = Vec::new();
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        let mut failure = None;
        loop {
            let event = self
                .worker
                .receiver
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .map_err(|_| unavailable())?;
            match &event {
                Event::Barrier(id) if id == &barrier => break,
                Event::State(snapshot, runtime) => {
                    for window in self.snapshot.windows.keys().chain(
                        snapshot
                            .windows
                            .keys()
                            .filter(|window| !self.snapshot.windows.contains_key(*window)),
                    ) {
                        if self.snapshot.windows.get(window).map(|window| &window.binding)
                            != snapshot.windows.get(window).map(|window| &window.binding)
                        {
                            self.log.push(DomainEvent::BindingChanged {
                                window: window.clone(),
                                available: snapshot.windows.contains_key(window),
                            });
                        }
                    }
                    if self.snapshot.topology_revision != snapshot.topology_revision {
                        self.log.push(DomainEvent::TopologyChanged {
                            revision: snapshot.topology_revision,
                        });
                    }
                    self.snapshot = snapshot.clone();
                    self.runtime = runtime.clone();
                    self.log.push(DomainEvent::StateChanged {
                        revision: self.config.revision,
                        slots: self.runtime.presentations.keys().cloned().collect(),
                    });
                }
                Event::Result(result) => {
                    self.log.push(DomainEvent::Transition { result: result.clone() });
                }
                Event::Monitors(epoch, monitors) => {
                    self.monitor_epoch.clone_from(epoch);
                    self.monitors.clone_from(monitors);
                }
                Event::Error(error) => {
                    self.log.push(DomainEvent::OperationFailed { error: error.clone() });
                    failure = Some(error.clone());
                }
                _ => {}
            }
            replies.push(event);
        }
        failure.map_or(Ok(replies), Err)
    }

    fn public_snapshot(&mut self) -> Value {
        self.snapshot.now_ms = window_manager_core::unix_millis();
        json!(PublicSnapshot::capture(
            &self.config,
            &self.runtime,
            &self.snapshot,
            self.log.cursor()
        ))
    }

    #[allow(clippy::too_many_lines)] // Exhaustive command dispatch keeps capability checks in one place.
    fn execute(&mut self, command: SessionCommand) -> Result<Value> {
        command.authorize(self.control, self.providers)?;
        match command {
            SessionCommand::Configuration {} => Ok(json!(self.config)),
            SessionCommand::MonitorInventory {} => {
                self.exchange(Command::Refresh(self.config.clone()))?;
                Ok(json!({"epoch":self.monitor_epoch,"monitors":self.monitors}))
            }
            SessionCommand::ConfirmMonitor { epoch, device, alias } => {
                self.exchange(Command::ConfirmMonitor { epoch, device, alias })?;
                self.plans.clear();
                Ok(self.public_snapshot())
            }
            SessionCommand::Snapshot {} => {
                self.exchange(Command::Refresh(self.config.clone()))?;
                Ok(self.public_snapshot())
            }
            SessionCommand::Simulate { input } => {
                Ok(json!(window_manager_core::simulate(&self.config, &input)?))
            }
            SessionCommand::Events { cursor } => {
                self.exchange(Command::Barrier(new_id("poll")))?;
                Ok(json!(self.log.since(&cursor)?))
            }
            SessionCommand::Inventory {} => {
                let replies = self.exchange(Command::Refresh(self.config.clone()))?;
                self.candidates.clear();
                let mut inventory = Vec::new();
                for event in replies {
                    if let Event::Inventory(candidates) = event {
                        for candidate in candidates.into_iter().take(256) {
                            let id = new_id("candidate");
                            inventory.push(json!({"id":id,"title":candidate.title,"class":candidate.class,"process":candidate.process,"frame":candidate.frame}));
                            self.candidates.insert(id, candidate);
                        }
                    }
                }
                Ok(json!(inventory))
            }
            SessionCommand::Bind { expected_revision, window, candidate } => {
                if expected_revision != self.config.revision {
                    return Err(Error::new(
                        ErrorCode::StaleRevision,
                        "Configuration changed",
                        window,
                    ));
                }
                let candidate = self.candidates.remove(&candidate).ok_or_else(|| {
                    Error::new(
                        ErrorCode::StaleBinding,
                        "Candidate token expired; refresh inventory",
                        candidate,
                    )
                })?;
                self.exchange(Command::Bind(self.config.clone(), window, candidate))?;
                self.plans.clear();
                Ok(self.public_snapshot())
            }
            SessionCommand::Preview { request } => {
                self.preview(Command::Preview(self.config.clone(), request))
            }
            SessionCommand::WindowAction { action } => {
                self.preview(Command::WindowAction(self.config.clone(), action))
            }
            SessionCommand::Undo { id } => self.preview(Command::Undo(id)),
            SessionCommand::Collapse { slot } => {
                self.exchange(Command::Refresh(self.config.clone()))?;
                let request = self.runtime.collapse_request(&self.config, &slot)?;
                self.preview(Command::Preview(self.config.clone(), request))
            }
            SessionCommand::Apply { plan } => {
                let plan = self.plans.remove(&plan).ok_or_else(|| {
                    Error::new(ErrorCode::TargetMissing, "Preview token missing or consumed", plan)
                })?;
                let replies = self.exchange(Command::Apply(self.config.clone(), plan))?;
                replies
                    .into_iter()
                    .find_map(|event| {
                        if let Event::Result(result) = event { Some(json!(result)) } else { None }
                    })
                    .ok_or_else(unavailable)
            }
            SessionCommand::Pause { paused } => {
                self.exchange(Command::Pause(paused))?;
                Ok(self.public_snapshot())
            }
            SessionCommand::Recover {} => {
                let windows = self.runtime.claims.keys().cloned().collect();
                self.exchange(Command::Recover)?;
                self.log.push(DomainEvent::Recovery { windows });
                self.plans.clear();
                Ok(self.public_snapshot())
            }
            SessionCommand::ProviderRegister {
                provider,
                session,
                resources,
                output,
                attention,
            } => {
                self.exchange(Command::Provider(ProviderCommand::Register(
                    provider.clone(),
                    session,
                    resources,
                    output,
                    attention,
                )))?;
                self.log.push(DomainEvent::ProviderChanged { provider });
                Ok(self.public_snapshot())
            }
            SessionCommand::Structure { edit } => {
                let draft = self.config.edit_structure(&edit)?;
                self.save_configuration(draft)?;
                self.log.push(DomainEvent::MembershipChanged {
                    groups: self
                        .config
                        .views
                        .values()
                        .flat_map(|view| {
                            view.roots
                                .iter()
                                .filter(|(role, node)| {
                                    self.history
                                        .last()
                                        .and_then(|before| before.views.get(&view.id))
                                        .and_then(|before| before.roots.get(*role))
                                        .is_none_or(|before| {
                                            serde_json::to_value(before).ok()
                                                != serde_json::to_value(node).ok()
                                        })
                                })
                                .map(|(_, node)| node.id().to_owned())
                        })
                        .collect(),
                });
                Ok(self.public_snapshot())
            }
            SessionCommand::ConfigurationUndo { expected_revision } => {
                if expected_revision != self.config.revision {
                    return Err(Error::new(
                        ErrorCode::StaleRevision,
                        "Configuration revision changed",
                        "configuration",
                    ));
                }
                let mut draft = self.history.last().cloned().ok_or_else(|| {
                    Error::new(
                        ErrorCode::TargetMissing,
                        "No configuration edit to undo",
                        "configuration",
                    )
                })?;
                draft.revision = self.config.revision + 1;
                draft.save(&self.path)?;
                self.config = draft;
                self.history.pop();
                self.plans.clear();
                self.exchange(Command::Refresh(self.config.clone()))?;
                Ok(self.public_snapshot())
            }
            SessionCommand::ProviderPublish { message } => {
                let attention = message.attention.map(|_| message.window.clone());
                let provider = message.provider.clone();
                self.exchange(Command::Provider(ProviderCommand::Publish(message)))?;
                self.log.push(DomainEvent::ProviderChanged { provider: provider.clone() });
                if let Some(window) = attention {
                    self.log.push(DomainEvent::AttentionChanged { provider, window });
                }
                Ok(self.public_snapshot())
            }
            SessionCommand::ProviderDisconnect { provider } => {
                self.exchange(Command::Provider(ProviderCommand::Disconnect(provider.clone())))?;
                self.log.push(DomainEvent::ProviderChanged { provider });
                Ok(self.public_snapshot())
            }
        }
    }

    fn preview(&mut self, command: Command) -> Result<Value> {
        if self.plans.len() >= 32 {
            return Err(Error::new(
                ErrorCode::PermissionDenied,
                "Preview budget reached; apply a preview or start a new session",
                "session",
            ));
        }
        let replies = self.exchange(command)?;
        let plan = replies
            .into_iter()
            .find_map(|event| if let Event::Preview(plan) = event { Some(*plan) } else { None })
            .ok_or_else(unavailable)?;
        let token = new_id("preview");
        // Serialize authored domain data only. Plan.expected and mutations contain native bindings.
        let response = json!({"plan":token,"request":plan.id,"revision":plan.config_revision,"topology_revision":plan.topology_revision,"scope":plan.scope,"domains":plan.domains,"blocked":plan.blocked,"desired":plan.desired,"diagnostics":plan.diagnostics,"impact":plan.impact,"idempotent":plan.idempotent});
        self.plans.insert(token, plan);
        Ok(response)
    }

    fn save_configuration(&mut self, draft: Configuration) -> Result<()> {
        draft.save(&self.path)?;
        self.history.push(self.config.clone());
        if self.history.len() > 50 {
            self.history.remove(0);
        }
        self.config = draft;
        self.log.push(DomainEvent::ConfigurationChanged { revision: self.config.revision });
        self.plans.clear();
        self.exchange(Command::Refresh(self.config.clone()))?;
        Ok(())
    }
}

fn unavailable() -> Error {
    Error::new(ErrorCode::ApplicationTimeout, "Worker did not complete the command", "session")
}

pub fn run(path: &Path, control: bool, providers: bool) -> anyhow::Result<()> {
    let _lock = crate::cli::lock_configuration(path)?;
    let config = Configuration::load(path)?;
    let _watchdog = if control { Some(crate::ui::start_watchdog(path)?) } else { None };
    let mut session = Session {
        worker: Worker::start_session(path.to_owned(), config.clone(), control),
        config,
        snapshot: Snapshot::default(),
        runtime: Runtime::default(),
        log: EventLog::default(),
        plans: BTreeMap::new(),
        candidates: BTreeMap::new(),
        control,
        providers,
        path: path.to_owned(),
        history: Vec::new(),
        monitor_epoch: String::new(),
        monitors: Vec::new(),
        replay: crate::replay::ReplayCache::default(),
    };
    session.exchange(Command::Refresh(session.config.clone()))?;
    let input = std::io::stdin();
    let mut input = input.lock();
    let output = std::io::stdout();
    let mut output = output.lock();
    loop {
        let mut line = Vec::new();
        let count = input.by_ref().take(MAX_LINE + 1).read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if u64::try_from(count).unwrap_or(u64::MAX) > MAX_LINE {
            anyhow::bail!("Session line exceeds 64 KiB");
        }
        let response = match serde_json::from_slice::<Envelope>(&line) {
            Ok(envelope) if !envelope.id.is_empty() && envelope.id.len() <= 200 => {
                session.receive(envelope)
            }
            _ => {
                json!({"id":null,"error":Error::new(ErrorCode::InvalidConfiguration,"Malformed or unsupported session command","session"),"cursor":session.log.cursor()})
            }
        };
        serde_json::to_writer(&mut output, &response)?;
        output.write_all(b"\n")?;
        output.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_grants_and_strict_commands_cannot_be_widened_by_payloads() {
        assert!(SessionCommand::Snapshot {}.authorize(false, false).is_ok());
        assert!(SessionCommand::Recover {}.authorize(false, true).is_err());
        assert!(
            SessionCommand::ProviderDisconnect { provider: "p".into() }
                .authorize(true, false)
                .is_err()
        );
        for input in [
            r#"{"id":"x","command":{"kind":"snapshot","allow_control":true}}"#,
            r#"{"id":"x","command":{"kind":"recover"},"allow_control":true}"#,
        ] {
            assert!(serde_json::from_str::<Envelope>(input).is_err());
        }
    }
}
