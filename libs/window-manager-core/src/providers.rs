use crate::{Configuration, Error, ErrorCode, Id, Request, Result, Runtime, Target, tab_path};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputState {
    VerifiedPrivate,
    OutputLinked,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Working,
    ApprovalNeeded,
    InputNeeded,
    Error,
    Finished,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderMessage {
    pub provider: Id,
    pub session: Id,
    pub sequence: u64,
    pub window: Id,
    pub observed_ms: u64,
    pub ttl_ms: u64,
    pub output: Option<OutputState>,
    pub attention: Option<AttentionKind>,
}

#[derive(Clone, Debug)]
struct Provider {
    session: Id,
    sequence: u64,
    resources: BTreeSet<Id>,
    output: bool,
    attention: bool,
    connected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ProviderRegistry {
    providers: BTreeMap<Id, Provider>,
    evidence: BTreeMap<(Id, Id), ProviderMessage>,
}

impl ProviderRegistry {
    /// Adapter registration is a local trusted capability grant, never granted by layout packages.
    pub fn register(
        &mut self,
        id: Id,
        session: Id,
        resources: BTreeSet<Id>,
        output: bool,
        attention: bool,
    ) -> Result<()> {
        if id.is_empty()
            || session.is_empty()
            || id.len() > 200
            || session.len() > 200
            || resources.len() > 256
            || self.providers.len() >= 32 && !self.providers.contains_key(&id)
        {
            return Err(Error::new(
                ErrorCode::InvalidConfiguration,
                "Provider budget or identity invalid",
                id,
            ));
        }
        self.evidence.retain(|(provider, _), _| provider != &id);
        self.providers.insert(
            id,
            Provider { session, sequence: 0, resources, output, attention, connected: true },
        );
        Ok(())
    }

    pub fn accept(&mut self, message: ProviderMessage, now_ms: u64) -> Result<()> {
        let provider = self.providers.get_mut(&message.provider).ok_or_else(|| {
            Error::new(ErrorCode::PermissionDenied, "Provider is not registered", &message.provider)
        })?;
        if !provider.connected
            || provider.session != message.session
            || message.sequence <= provider.sequence
            || !provider.resources.contains(&message.window)
            || message.output.is_some() && !provider.output
            || message.attention.is_some() && !provider.attention
            || !(1..=60_000).contains(&message.ttl_ms)
            || message.observed_ms > now_ms
            || now_ms.saturating_sub(message.observed_ms) >= message.ttl_ms
        {
            return Err(Error::new(
                ErrorCode::PermissionDenied,
                "Stale, out-of-scope, or unsupported provider evidence",
                &message.provider,
            ));
        }
        provider.sequence = message.sequence;
        self.evidence.insert((message.provider.clone(), message.window.clone()), message);
        Ok(())
    }

    pub fn disconnect(&mut self, id: &str) {
        if let Some(provider) = self.providers.get_mut(id) {
            provider.connected = false;
        }
    }

    fn fresh(&self, message: &ProviderMessage, now_ms: u64) -> bool {
        self.providers
            .get(&message.provider)
            .is_some_and(|provider| provider.connected && provider.session == message.session)
            && now_ms >= message.observed_ms
            && now_ms - message.observed_ms < message.ttl_ms
    }

    #[must_use]
    pub fn output(&self, window: &str, now_ms: u64) -> OutputState {
        let states: Vec<_> = self
            .evidence
            .values()
            .filter(|message| message.window == window && message.output.is_some())
            .map(|message| {
                if self.fresh(message, now_ms) {
                    message.output.unwrap_or(OutputState::Unknown)
                } else {
                    OutputState::Unknown
                }
            })
            .collect();
        if states.contains(&OutputState::OutputLinked) {
            OutputState::OutputLinked
        } else if !states.is_empty()
            && states.iter().all(|state| *state == OutputState::VerifiedPrivate)
        {
            OutputState::VerifiedPrivate
        } else {
            OutputState::Unknown
        }
    }

    #[must_use]
    pub fn attention(&self, now_ms: u64) -> Vec<&ProviderMessage> {
        self.evidence
            .values()
            .filter(|message| message.attention.is_some() && self.fresh(message, now_ms))
            .collect()
    }
}

pub fn attention_request(
    config: &Configuration,
    runtime: &Runtime,
    window: &str,
) -> Result<Request> {
    if let Some(target) = runtime.attention_targets.get(window) {
        config.validate_target(target)?;
        for (root, slot) in &target.roots {
            let mut choices = BTreeMap::new();
            if tab_path(&config.views[&target.view].roots[root], window, &mut choices) {
                let mut request = Request::open(
                    config,
                    Target {
                        view: target.view.clone(),
                        roots: BTreeMap::from([(root.clone(), slot.clone())]),
                    },
                );
                request.selected_tabs = choices;
                return Ok(request);
            }
        }
        return Err(Error::new(
            ErrorCode::TargetMissing,
            "Attention target does not contain the resource",
            window,
        ));
    }
    let claim = runtime.claims.get(window).ok_or_else(|| {
        Error::new(
            ErrorCode::TargetMissing,
            "Attention resource has no designated active slot",
            window,
        )
    })?;
    let presentation = runtime.presentations.get(&claim.slot).ok_or_else(|| {
        Error::new(ErrorCode::TargetMissing, "Attention Presentation missing", window)
    })?;
    let target = Target {
        view: presentation.view.clone(),
        roots: BTreeMap::from([(presentation.root.clone(), claim.slot.clone())]),
    };
    let mut request = Request::open(config, target);
    if !tab_path(
        &config.views[&presentation.view].roots[&presentation.root],
        window,
        &mut request.selected_tabs,
    ) {
        return Err(Error::new(
            ErrorCode::TargetMissing,
            "Attention resource is not in the designated View",
            window,
        ));
    }
    Ok(request)
}
