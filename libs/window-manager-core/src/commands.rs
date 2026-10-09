use crate::{Configuration, Error, ErrorCode, Id, Request, Result, Target};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CommandTarget {
    View { target: Target },
    Composition { composition: Id },
    Workspace { workspace: Id, roots: BTreeMap<String, Id> },
}

/// Retains version-one fixed-View shortcut documents without guessing new scopes.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ShortcutTarget {
    FixedView(Target),
    Command(CommandTarget),
}
impl ShortcutTarget {
    pub fn resolve(&self, config: &Configuration) -> Result<Request> {
        match self {
            Self::FixedView(target) => {
                CommandTarget::View { target: target.clone() }.resolve(config)
            }
            Self::Command(target) => target.resolve(config),
        }
    }
    pub fn validate(&self, config: &Configuration) -> Result<()> {
        match self {
            Self::FixedView(target) => config.validate_target(target),
            Self::Command(target) => target.validate(config),
        }
    }
}
impl CommandTarget {
    pub fn validate(&self, config: &Configuration) -> Result<()> {
        match self {
            Self::View { target } => config.validate_target(target),
            Self::Composition { composition } => {
                if config.compositions.get(composition).is_none_or(|value| value.targets.is_empty())
                {
                    return Err(Error::new(
                        ErrorCode::TargetMissing,
                        "Composition missing or empty",
                        composition,
                    ));
                }
                Ok(())
            }
            Self::Workspace { workspace, roots } => {
                if !config.workspaces.contains_key(workspace) {
                    return Err(Error::new(
                        ErrorCode::TargetMissing,
                        "Workspace missing",
                        workspace,
                    ));
                }
                if roots.is_empty()
                    || roots.len() > 128
                    || roots.keys().any(|role| role.is_empty() || role.len() > 200)
                    || roots.values().any(|slot| !config.slots.contains_key(slot))
                    || roots.values().collect::<std::collections::BTreeSet<_>>().len()
                        != roots.len()
                {
                    return Err(Error::new(
                        ErrorCode::OutOfScope,
                        "Workspace command requires a unique fixed root/Slot mapping",
                        workspace,
                    ));
                }
                Ok(())
            }
        }
    }
    pub fn resolve(&self, config: &Configuration) -> Result<Request> {
        self.validate(config)?;
        let targets = match self {
            Self::View { target } => vec![target.clone()],
            Self::Composition { composition } => config.compositions[composition].targets.clone(),
            Self::Workspace { workspace, roots } => {
                let view =
                    config.workspaces[workspace].remembered_view.clone().ok_or_else(|| {
                        Error::new(
                            ErrorCode::TargetMissing,
                            "Workspace has no remembered View; select one explicitly",
                            workspace,
                        )
                    })?;
                let target = Target { view, roots: roots.clone() };
                config.validate_target(&target).map_err(|_| {
                    Error::new(
                        ErrorCode::TargetMissing,
                        "Remembered View does not provide the fixed command roles",
                        workspace,
                    )
                })?;
                vec![target]
            }
        };
        let mut request = Request::open(config, targets[0].clone());
        request.scope = targets.iter().flat_map(|target| target.roots.values().cloned()).collect();
        request.targets = targets;
        Ok(request)
    }
}
