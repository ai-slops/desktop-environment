use crate::{
    Error, ErrorCode, Id, Node, Result, evaluate, formula_dependencies, validate_formula_inputs,
    validate_property_formula_inputs,
};
use std::collections::{BTreeMap, BTreeSet};

const RESERVED: &[&str] = &[
    "available_width",
    "available_height",
    "count",
    "true",
    "false",
    "min",
    "max",
    "clamp",
    "floor",
    "ceil",
    "abs",
    "child_index",
    "preferred_width",
    "preferred_height",
];

/// Stable topological order, bounded declarations, and explicit inherited inputs.
pub fn parameter_order(
    definitions: &BTreeMap<String, String>,
    inherited: &BTreeSet<String>,
) -> Result<Vec<String>> {
    if definitions.len() > 32
        || inherited.len() + definitions.len() > 128
        || definitions.values().map(String::len).sum::<usize>() > 16_384
    {
        return Err(Error::new(
            ErrorCode::FormulaBudgetExceeded,
            "Parameter declaration budget exceeded",
            "parameters",
        ));
    }
    let mut allowed = inherited.clone();
    allowed.extend(definitions.keys().cloned());
    let mut pending = BTreeMap::new();
    for (name, expression) in definitions {
        if name.is_empty()
            || name.len() > 64
            || RESERVED.contains(&name.as_str())
            || !name.bytes().enumerate().all(|(index, byte)| {
                byte == b'_' || byte.is_ascii_alphabetic() || index > 0 && byte.is_ascii_digit()
            })
        {
            return Err(Error::new(
                ErrorCode::FormulaInvalid,
                "Invalid or reserved parameter name",
                name,
            ));
        }
        validate_formula_inputs(expression, &allowed)?;
        pending.insert(
            name.clone(),
            formula_dependencies(expression)?
                .intersection(&definitions.keys().cloned().collect())
                .cloned()
                .collect::<BTreeSet<_>>(),
        );
    }
    let mut order = Vec::new();
    while !pending.is_empty() {
        let next = pending
            .iter()
            .find(|(_, dependencies)| dependencies.is_empty())
            .map(|(name, _)| name.clone())
            .ok_or_else(|| {
                Error::new(
                    ErrorCode::FormulaInvalid,
                    format!(
                        "Cyclic parameter dependencies: {}",
                        pending.keys().cloned().collect::<Vec<_>>().join(", ")
                    ),
                    "parameters",
                )
            })?;
        pending.remove(&next);
        for dependencies in pending.values_mut() {
            dependencies.remove(&next);
        }
        order.push(next);
    }
    Ok(order)
}

pub fn resolve_parameters(
    definitions: &BTreeMap<String, String>,
    context: &BTreeMap<String, f64>,
) -> Result<BTreeMap<String, f64>> {
    let order = parameter_order(definitions, &context.keys().cloned().collect())?;
    let mut resolved = context.clone();
    for name in order {
        let value = evaluate(&definitions[&name], &resolved).map_err(|mut error| {
            error.objects = vec![name.clone()];
            error
        })?;
        if value.abs() > 1_000_000.0 {
            return Err(Error::new(
                ErrorCode::FormulaInvalid,
                "Parameter exceeds numeric bounds",
                name,
            ));
        }
        resolved.insert(name, value);
    }
    Ok(resolved)
}

/// Invalid drafts cannot replace a last-good rule; dependencies never read layout outputs.
pub fn validate_rules(node: &Node, inherited: &BTreeSet<String>) -> Result<()> {
    let result = (|| match node {
        Node::Placement(placement) => {
            for preference in
                std::iter::once(&placement.default_preference).chain(placement.preferences.values())
            {
                for expression in
                    [&preference.width_formula, &preference.height_formula].into_iter().flatten()
                {
                    validate_property_formula_inputs(expression, 1.0, 65_536.0, false, inherited)?;
                }
            }
            Ok(())
        }
        Node::Group(group) => {
            parameter_order(&group.parameters, inherited)?;
            let mut inputs = inherited.clone();
            inputs.extend(group.parameters.keys().cloned());
            for expression in group.parameters.values() {
                validate_property_formula_inputs(
                    expression,
                    -1_000_000.0,
                    1_000_000.0,
                    false,
                    &inputs,
                )?;
            }
            validate_property_formula_inputs(&group.gap, 0.0, 4096.0, false, &inputs)?;
            validate_property_formula_inputs(&group.columns, 1.0, 256.0, true, &inputs)?;
            for variant in &group.variants {
                if let Some(expression) = &variant.condition {
                    validate_formula_inputs(expression, &inputs)?;
                    match crate::evaluate_condition(expression, &BTreeMap::new()) {
                        Ok(_) => {}
                        Err(error)
                            if error.message.starts_with("Unknown input:")
                                || error.message == "Missing candidate count" => {}
                        Err(error) => return Err(error),
                    }
                }
            }
            if let Some(expression) = &group.sort_formula {
                let mut sort_inputs = inputs.clone();
                sort_inputs.extend(
                    ["child_index", "preferred_width", "preferred_height"].map(str::to_owned),
                );
                validate_property_formula_inputs(
                    expression,
                    -1_000_000.0,
                    1_000_000.0,
                    false,
                    &sort_inputs,
                )?;
            }
            for child in &group.children {
                validate_rules(child, &inputs)?;
            }
            if let Some(membership) = &group.membership {
                for placement in membership.retired.values() {
                    validate_rules(&Node::Placement(placement.clone()), &inputs)?;
                }
            }
            Ok(())
        }
    })();
    result.map_err(|mut error: Error| {
        error.objects.insert(0, node.id().to_owned());
        error
    })
}

#[must_use]
pub fn declared_parameters(roots: &BTreeMap<String, Node>) -> BTreeMap<String, String> {
    fn collect(node: &Node, result: &mut BTreeMap<Id, String>) {
        if let Node::Group(group) = node {
            for (name, expression) in &group.parameters {
                result.insert(format!("{}:{name}", group.id), expression.clone());
            }
            for child in &group.children {
                collect(child, result);
            }
        }
    }
    let mut result = BTreeMap::new();
    for root in roots.values() {
        collect(root, &mut result);
    }
    result
}

pub fn ancestor_parameters(
    root: &Node,
    target: &str,
    area: crate::Rect,
    dpi: u32,
    bounds: Option<&BTreeMap<Id, crate::Rect>>,
) -> Result<BTreeMap<String, f64>> {
    fn visit(
        node: &Node,
        target: &str,
        area: crate::Rect,
        dpi: u32,
        bounds: Option<&BTreeMap<Id, crate::Rect>>,
        mut inputs: BTreeMap<String, f64>,
    ) -> Result<BTreeMap<String, f64>> {
        if node.id() == target {
            return Ok(inputs);
        }
        let Node::Group(group) = node else {
            return Err(Error::new(ErrorCode::OutOfScope, "Expanded target path missing", target));
        };
        let scale = f64::from(dpi) / 96.0;
        let context_area = bounds.and_then(|bounds| bounds.get(&group.id)).copied().unwrap_or(area);
        inputs.extend([
            ("available_width".into(), f64::from(context_area.width) / scale),
            ("available_height".into(), f64::from(context_area.height) / scale),
            ("count".into(), f64::from(u32::try_from(group.children.len()).unwrap_or_default())),
        ]);
        let inputs = resolve_parameters(&group.parameters, &inputs)?;
        let child =
            group.children.iter().find(|child| child.find(target).is_some()).ok_or_else(|| {
                Error::new(ErrorCode::OutOfScope, "Expanded target path missing", target)
            })?;
        visit(child, target, area, dpi, bounds, inputs)
    }
    visit(root, target, area, dpi, bounds, BTreeMap::new())
}
