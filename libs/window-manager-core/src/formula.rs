use crate::{Error, ErrorCode, Result};
use std::collections::BTreeMap;

const MAX_SOURCE: usize = 4096;
const MAX_TOKENS: usize = 512;
const MAX_DEPTH: usize = 32;

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Number(f64),
    Name(String),
    Op(String),
    End,
}

#[derive(Clone, Debug)]
enum Expr {
    Number(f64),
    Name(String),
    Unary(String, Box<Self>),
    Binary(String, Box<Self>, Box<Self>),
    If(Box<Self>, Box<Self>, Box<Self>),
    Call(String, Vec<Self>),
}

#[derive(Clone, Copy)]
enum Value {
    Number(f64),
    Boolean(bool),
}

impl Value {
    fn number(self) -> Result<f64> {
        match self {
            Self::Number(value) if value.is_finite() => Ok(value),
            _ => Err(invalid("Expected a finite number")),
        }
    }
    fn boolean(self) -> Result<bool> {
        match self {
            Self::Boolean(value) => Ok(value),
            Self::Number(_) => Err(invalid("Expected a Boolean expression")),
        }
    }
}

fn invalid(message: &str) -> Error {
    Error::new(ErrorCode::FormulaInvalid, message, "formula")
}
fn budget() -> Error {
    Error::new(
        ErrorCode::FormulaBudgetExceeded,
        "Formula exceeds source, token, or recursion budget",
        "formula",
    )
}

/// Pure numeric expression evaluation; unknown names and non-finite values are errors.
/// Comparisons/Boolean logic and lazy conditional expressions are available inside formulas.
pub fn evaluate(source: &str, context: &BTreeMap<String, f64>) -> Result<f64> {
    let tokens = tokenize(source)?;
    let mut parser = Parser { tokens, index: 0 };
    let expression = parser.parse(0, 0)?;
    if parser.peek() != &Token::End {
        return Err(invalid("Unexpected trailing input"));
    }
    evaluate_expr(&expression, context, 0)?.number()
}

/// Parse every branch before committing a draft. Evaluation still uses actual local inputs.
#[allow(clippy::items_after_statements)] // The AST validator belongs to this parse/validate boundary.
pub fn validate_formula(source: &str) -> Result<()> {
    let mut parser = Parser { tokens: tokenize(source)?, index: 0 };
    let expression = parser.parse(0, 0)?;
    if parser.peek() != &Token::End {
        return Err(invalid("Unexpected trailing input"));
    }
    fn validate(expr: &Expr, depth: usize) -> Result<()> {
        if depth > MAX_DEPTH {
            return Err(budget());
        }
        match expr {
            Expr::Number(_) => {}
            Expr::Name(name) => {
                if !["available_width", "available_height", "count", "true", "false"]
                    .contains(&name.as_str())
                {
                    return Err(invalid(&format!("Unknown input: {name}")));
                }
            }
            Expr::Unary(_, value) => validate(value, depth + 1)?,
            Expr::Binary(_, a, b) => {
                validate(a, depth + 1)?;
                validate(b, depth + 1)?;
            }
            Expr::If(a, b, c) => {
                validate(a, depth + 1)?;
                validate(b, depth + 1)?;
                validate(c, depth + 1)?;
            }
            Expr::Call(name, args) => {
                let arity = match name.as_str() {
                    "min" | "max" => 2,
                    "clamp" => 3,
                    "floor" | "ceil" | "abs" => 1,
                    "count" => 0,
                    _ => return Err(invalid("Unknown function")),
                };
                if args.len() != arity {
                    return Err(invalid("Invalid function argument count"));
                }
                for arg in args {
                    validate(arg, depth + 1)?;
                }
            }
        }
        Ok(())
    }
    validate(&expression, 0)
}

/// Reject invalid constants before persistence; context-dependent values are checked at planning.
pub fn validate_property_formula(
    source: &str,
    minimum: f64,
    maximum: f64,
    integer: bool,
) -> Result<()> {
    validate_formula(source)?;
    match evaluate(source, &BTreeMap::new()) {
        Ok(value) if value < minimum || value > maximum || (integer && value.fract() != 0.0) => {
            Err(invalid("Constant result is outside this property's bounds"))
        }
        Ok(_) => Ok(()),
        Err(error)
            if error.message.starts_with("Unknown input:")
                || error.message == "Missing candidate count" =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn tokenize(source: &str) -> Result<Vec<Token>> {
    if source.len() > MAX_SOURCE {
        return Err(budget());
    }
    let chars: Vec<_> = source.chars().collect();
    let mut index = 0;
    let mut tokens = Vec::new();
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            index += 1;
            continue;
        }
        if c.is_ascii_digit() || c == '.' {
            let start = index;
            while index < chars.len() && (chars[index].is_ascii_digit() || chars[index] == '.') {
                index += 1;
            }
            let text: String = chars[start..index].iter().collect();
            let value = text.parse::<f64>().map_err(|_| invalid("Invalid number"))?;
            if !value.is_finite() {
                return Err(invalid("Number must be finite"));
            }
            tokens.push(Token::Number(value));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let start = index;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            tokens.push(Token::Name(chars[start..index].iter().collect()));
        } else {
            let pair: String = chars[index..(index + 2).min(chars.len())].iter().collect();
            if ["<=", ">=", "==", "!=", "&&", "||"].contains(&pair.as_str()) {
                tokens.push(Token::Op(pair));
                index += 2;
            } else if "+-*/%^<>()!,?:".contains(c) {
                tokens.push(Token::Op(c.to_string()));
                index += 1;
            } else {
                return Err(invalid("Unsupported character; formulas cannot access host APIs"));
            }
        }
        if tokens.len() > MAX_TOKENS {
            return Err(budget());
        }
    }
    tokens.push(Token::End);
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    index: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        self.tokens.get(self.index).unwrap_or(&Token::End)
    }
    fn take(&mut self) -> Token {
        let token = self.peek().clone();
        self.index += 1;
        token
    }
    fn consume(&mut self, text: &str) -> bool {
        if self.peek() == &Token::Op(text.into()) {
            self.index += 1;
            true
        } else {
            false
        }
    }
    fn require(&mut self, text: &str) -> Result<()> {
        if self.consume(text) { Ok(()) } else { Err(invalid("Missing punctuation")) }
    }
    fn parse(&mut self, minimum: u8, depth: usize) -> Result<Expr> {
        if depth > MAX_DEPTH {
            return Err(budget());
        }
        let mut left = match self.take() {
            Token::Number(value) => Expr::Number(value),
            Token::Name(name) => {
                if self.consume("(") {
                    let mut args = Vec::new();
                    if !self.consume(")") {
                        loop {
                            args.push(self.parse(0, depth + 1)?);
                            if self.consume(")") {
                                break;
                            }
                            self.require(",")?;
                        }
                    }
                    Expr::Call(name, args)
                } else {
                    Expr::Name(name)
                }
            }
            Token::Op(op) if op == "(" => {
                let result = self.parse(0, depth + 1)?;
                self.require(")")?;
                result
            }
            Token::Op(op) if ["-", "+", "!"].contains(&op.as_str()) => {
                Expr::Unary(op, Box::new(self.parse(8, depth + 1)?))
            }
            _ => return Err(invalid("Expected a value")),
        };
        while let Token::Op(op) = self.peek().clone() {
            if op == "?" && minimum == 0 {
                self.take();
                let yes = self.parse(0, depth + 1)?;
                self.require(":")?;
                let no = self.parse(0, depth + 1)?;
                left = Expr::If(Box::new(left), Box::new(yes), Box::new(no));
                continue;
            }
            let priority = match op.as_str() {
                "||" => 1,
                "&&" => 2,
                "==" | "!=" => 3,
                "<" | ">" | "<=" | ">=" => 4,
                "+" | "-" => 5,
                "*" | "/" | "%" => 6,
                "^" => 7,
                _ => break,
            };
            if priority < minimum {
                break;
            }
            self.take();
            let right = self.parse(priority + 1, depth + 1)?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }
}

// Equality in the expression language intentionally has exact IEEE numeric semantics.
#[allow(clippy::float_cmp)]
fn evaluate_expr(expr: &Expr, context: &BTreeMap<String, f64>, depth: usize) -> Result<Value> {
    if depth > MAX_DEPTH {
        return Err(budget());
    }
    let child = |expr: &Expr| evaluate_expr(expr, context, depth + 1);
    match expr {
        Expr::Number(value) => Ok(Value::Number(*value)),
        Expr::Name(name) => match name.as_str() {
            "true" => Ok(Value::Boolean(true)),
            "false" => Ok(Value::Boolean(false)),
            _ => context
                .get(name)
                .copied()
                .map(Value::Number)
                .ok_or_else(|| invalid(&format!("Unknown input: {name}"))),
        },
        Expr::Unary(op, expr) => {
            let value = child(expr)?;
            Ok(match op.as_str() {
                "!" => Value::Boolean(!value.boolean()?),
                "-" => Value::Number(-value.number()?),
                _ => Value::Number(value.number()?),
            })
        }
        Expr::If(condition, yes, no) => child(if child(condition)?.boolean()? { yes } else { no }),
        Expr::Binary(op, left, right) => {
            let left = child(left)?;
            if op == "&&" {
                return Ok(Value::Boolean(left.boolean()? && child(right)?.boolean()?));
            }
            if op == "||" {
                return Ok(Value::Boolean(left.boolean()? || child(right)?.boolean()?));
            }
            let a = left.number()?;
            let b = child(right)?.number()?;
            let result = match op.as_str() {
                "+" => Value::Number(a + b),
                "-" => Value::Number(a - b),
                "*" => Value::Number(a * b),
                "/" | "%" if b == 0.0 => return Err(invalid("Division by zero")),
                "/" => Value::Number(a / b),
                "%" => Value::Number(a % b),
                "^" => Value::Number(a.powf(b)),
                "<" => Value::Boolean(a < b),
                ">" => Value::Boolean(a > b),
                "<=" => Value::Boolean(a <= b),
                ">=" => Value::Boolean(a >= b),
                "==" => Value::Boolean(a == b),
                "!=" => Value::Boolean(a != b),
                _ => return Err(invalid("Unknown operator")),
            };
            if let Value::Number(value) = result {
                value.is_finite().then_some(result).ok_or_else(|| invalid("Non-finite result"))
            } else {
                Ok(result)
            }
        }
        Expr::Call(name, args) => {
            let values: Vec<f64> =
                args.iter().map(|arg| child(arg)?.number()).collect::<Result<_>>()?;
            let value = match (name.as_str(), values.as_slice()) {
                ("min", [a, b]) => a.min(*b),
                ("max", [a, b]) => a.max(*b),
                ("clamp", [value, min, max]) if min <= max => value.clamp(*min, *max),
                ("floor", [value]) => value.floor(),
                ("ceil", [value]) => value.ceil(),
                ("abs", [value]) => value.abs(),
                ("count", []) => {
                    *context.get("count").ok_or_else(|| invalid("Missing candidate count"))?
                }
                _ => return Err(invalid("Unknown function or invalid arguments")),
            };
            if !value.is_finite() {
                return Err(invalid("Non-finite function result"));
            }
            Ok(Value::Number(value))
        }
    }
}
