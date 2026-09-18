//! Bash-style scalar integer arithmetic.

use crate::{Env, EvalError};

const MAX_RECURSION: usize = 1024;

pub(crate) fn eval<E: Env>(input: &str, env: &mut E) -> Result<i64, EvalError> {
    eval_inner(input, env, 0, input)
}

fn eval_inner<E: Env>(
    input: &str,
    env: &mut E,
    depth: usize,
    recursion_name: &str,
) -> Result<i64, EvalError> {
    if depth >= MAX_RECURSION {
        return Err(EvalError::ArithmeticRecursion(recursion_name.to_owned()));
    }
    if input.trim().is_empty() {
        return Ok(0);
    }
    let tokens = tokenize(input)?;
    let mut parser = Parser::new(tokens);
    let expression = parser.parse_comma()?;
    if !matches!(parser.peek(), Token::End) {
        return Err(EvalError::InvalidArith(input.to_owned()));
    }
    evaluate(&expression, env, depth)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Number(String),
    Ident(String),
    Op(&'static str),
    LParen,
    RParen,
    Question,
    Colon,
    Comma,
    End,
}

fn tokenize(input: &str) -> Result<Vec<Token>, EvalError> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if bytes[i].is_ascii_digit() {
            let start = i;
            i += 1;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'#' | b'@' | b'_'))
            {
                i += 1;
            }
            tokens.push(Token::Number(input[start..i].to_owned()));
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let start = i;
            i += 1;
            while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            tokens.push(Token::Ident(input[start..i].to_owned()));
            continue;
        }
        let rest = &input[i..];
        let operator = [
            "<<=", ">>=", "++", "--", "**", "<=", ">=", "==", "!=", "&&", "||", "*=", "/=", "%=",
            "+=", "-=", "&=", "^=", "|=", "<<", ">>", "=", "+", "-", "*", "/", "%", "!", "~", "<",
            ">", "&", "^", "|",
        ]
        .into_iter()
        .find(|operator| rest.starts_with(operator));
        if let Some(operator) = operator {
            tokens.push(Token::Op(operator));
            i += operator.len();
            continue;
        }
        tokens.push(match bytes[i] {
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'?' => Token::Question,
            b':' => Token::Colon,
            b',' => Token::Comma,
            _ => return Err(EvalError::InvalidArith(input.to_owned())),
        });
        i += 1;
    }
    tokens.push(Token::End);
    Ok(tokens)
}

#[derive(Debug, Clone)]
enum Expr {
    Number(i64),
    Variable(String),
    Unary(UnaryOp, Box<Expr>),
    Binary(BinaryOp, Box<Expr>, Box<Expr>),
    Conditional(Box<Expr>, Box<Expr>, Box<Expr>),
    Group(Box<Expr>),
    Assign(String, AssignOp, Box<Expr>),
    PreIncrement(String, i64),
    PostIncrement(String, i64),
    Comma(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, Copy)]
enum UnaryOp {
    Plus,
    Minus,
    LogicalNot,
    BitwiseNot,
}

#[derive(Debug, Clone, Copy)]
enum BinaryOp {
    Power,
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
    LogicalAnd,
    LogicalOr,
}

#[derive(Debug, Clone, Copy)]
enum AssignOp {
    Set,
    Multiply,
    Divide,
    Remainder,
    Add,
    Subtract,
    ShiftLeft,
    ShiftRight,
    BitwiseAnd,
    BitwiseXor,
    BitwiseOr,
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn take(&mut self) -> Token {
        let token = self.tokens[self.pos].clone();
        self.pos += 1;
        token
    }

    fn take_op(&mut self, operators: &[&str]) -> Option<&'static str> {
        let Token::Op(operator) = self.peek() else {
            return None;
        };
        if !operators.contains(operator) {
            return None;
        }
        let operator = *operator;
        self.pos += 1;
        Some(operator)
    }

    fn parse_comma(&mut self) -> Result<Expr, EvalError> {
        let mut expression = self.parse_assignment()?;
        while matches!(self.peek(), Token::Comma) {
            self.take();
            expression = Expr::Comma(Box::new(expression), Box::new(self.parse_assignment()?));
        }
        Ok(expression)
    }

    fn parse_assignment(&mut self) -> Result<Expr, EvalError> {
        let left = self.parse_conditional()?;
        let Some(operator) = self.take_op(&[
            "=", "*=", "/=", "%=", "+=", "-=", "<<=", ">>=", "&=", "^=", "|=",
        ]) else {
            return Ok(left);
        };
        let Expr::Variable(name) = left else {
            return Err(EvalError::InvalidArith(
                "assignment requires a scalar variable".into(),
            ));
        };
        let op = match operator {
            "=" => AssignOp::Set,
            "*=" => AssignOp::Multiply,
            "/=" => AssignOp::Divide,
            "%=" => AssignOp::Remainder,
            "+=" => AssignOp::Add,
            "-=" => AssignOp::Subtract,
            "<<=" => AssignOp::ShiftLeft,
            ">>=" => AssignOp::ShiftRight,
            "&=" => AssignOp::BitwiseAnd,
            "^=" => AssignOp::BitwiseXor,
            "|=" => AssignOp::BitwiseOr,
            _ => unreachable!(),
        };
        Ok(Expr::Assign(name, op, Box::new(self.parse_assignment()?)))
    }

    fn parse_conditional(&mut self) -> Result<Expr, EvalError> {
        let condition = self.parse_logical_or()?;
        if !matches!(self.peek(), Token::Question) {
            return Ok(condition);
        }
        self.take();
        let when_true = self.parse_comma()?;
        if !matches!(self.take(), Token::Colon) {
            return Err(EvalError::InvalidArith("missing `:` in ternary".into()));
        }
        let when_false = self.parse_conditional()?;
        Ok(Expr::Conditional(
            Box::new(condition),
            Box::new(when_true),
            Box::new(when_false),
        ))
    }

    fn parse_logical_or(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_logical_and,
            &["||"],
            |operator| match operator {
                "||" => BinaryOp::LogicalOr,
                _ => unreachable!(),
            },
        )
    }

    fn parse_logical_and(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(Self::parse_bitwise_or, &["&&"], |operator| match operator {
            "&&" => BinaryOp::LogicalAnd,
            _ => unreachable!(),
        })
    }

    fn parse_bitwise_or(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(Self::parse_bitwise_xor, &["|"], |_| BinaryOp::BitwiseOr)
    }

    fn parse_bitwise_xor(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(Self::parse_bitwise_and, &["^"], |_| BinaryOp::BitwiseXor)
    }

    fn parse_bitwise_and(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(Self::parse_equality, &["&"], |_| BinaryOp::BitwiseAnd)
    }

    fn parse_equality(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_comparison,
            &["==", "!="],
            |operator| match operator {
                "==" => BinaryOp::Equal,
                "!=" => BinaryOp::NotEqual,
                _ => unreachable!(),
            },
        )
    }

    fn parse_comparison(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_shift,
            &["<", "<=", ">", ">="],
            |operator| match operator {
                "<" => BinaryOp::Less,
                "<=" => BinaryOp::LessEqual,
                ">" => BinaryOp::Greater,
                ">=" => BinaryOp::GreaterEqual,
                _ => unreachable!(),
            },
        )
    }

    fn parse_shift(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_additive,
            &["<<", ">>"],
            |operator| match operator {
                "<<" => BinaryOp::ShiftLeft,
                ">>" => BinaryOp::ShiftRight,
                _ => unreachable!(),
            },
        )
    }

    fn parse_additive(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_multiplicative,
            &["+", "-"],
            |operator| match operator {
                "+" => BinaryOp::Add,
                "-" => BinaryOp::Subtract,
                _ => unreachable!(),
            },
        )
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, EvalError> {
        self.parse_left_assoc(
            Self::parse_power,
            &["*", "/", "%"],
            |operator| match operator {
                "*" => BinaryOp::Multiply,
                "/" => BinaryOp::Divide,
                "%" => BinaryOp::Remainder,
                _ => unreachable!(),
            },
        )
    }

    fn parse_power(&mut self) -> Result<Expr, EvalError> {
        let left = self.parse_unary()?;
        if self.take_op(&["**"]).is_some() {
            return Ok(Expr::Binary(
                BinaryOp::Power,
                Box::new(left),
                Box::new(self.parse_power()?),
            ));
        }
        Ok(left)
    }

    fn parse_unary(&mut self) -> Result<Expr, EvalError> {
        if let Some(operator) = self.take_op(&["+", "-", "!", "~", "++", "--"]) {
            let expression = self.parse_unary()?;
            if matches!(operator, "++" | "--") {
                let Expr::Variable(name) = expression else {
                    return Err(EvalError::InvalidArith(
                        "increment requires a scalar variable".into(),
                    ));
                };
                return Ok(Expr::PreIncrement(
                    name,
                    if operator == "++" { 1 } else { -1 },
                ));
            }
            let op = match operator {
                "+" => UnaryOp::Plus,
                "-" => UnaryOp::Minus,
                "!" => UnaryOp::LogicalNot,
                "~" => UnaryOp::BitwiseNot,
                _ => unreachable!(),
            };
            return Ok(Expr::Unary(op, Box::new(expression)));
        }
        self.parse_postfix()
    }

    fn parse_postfix(&mut self) -> Result<Expr, EvalError> {
        let expression = self.parse_primary()?;
        let Some(operator) = self.take_op(&["++", "--"]) else {
            return Ok(expression);
        };
        let Expr::Variable(name) = expression else {
            return Err(EvalError::InvalidArith(
                "increment requires a scalar variable".into(),
            ));
        };
        Ok(Expr::PostIncrement(
            name,
            if operator == "++" { 1 } else { -1 },
        ))
    }

    fn parse_primary(&mut self) -> Result<Expr, EvalError> {
        match self.take() {
            Token::Number(number) => Ok(Expr::Number(parse_number(&number)?)),
            Token::Ident(name) => Ok(Expr::Variable(name)),
            Token::LParen => {
                let expression = self.parse_comma()?;
                if !matches!(self.take(), Token::RParen) {
                    return Err(EvalError::InvalidArith("missing `)`".into()));
                }
                Ok(Expr::Group(Box::new(expression)))
            }
            token => Err(EvalError::InvalidArith(format!(
                "expected arithmetic operand, found {token:?}"
            ))),
        }
    }

    fn parse_left_assoc(
        &mut self,
        next: fn(&mut Self) -> Result<Expr, EvalError>,
        operators: &[&str],
        map: impl Fn(&str) -> BinaryOp,
    ) -> Result<Expr, EvalError> {
        let mut expression = next(self)?;
        while let Some(operator) = self.take_op(operators) {
            let right = next(self)?;
            expression = Expr::Binary(map(operator), Box::new(expression), Box::new(right));
        }
        Ok(expression)
    }
}

fn parse_number(raw: &str) -> Result<i64, EvalError> {
    let (base, digits) = if let Some(hash) = raw.find('#') {
        let base = raw[..hash]
            .parse::<u32>()
            .map_err(|_| EvalError::InvalidArith(raw.into()))?;
        if !(2..=64).contains(&base) || hash + 1 == raw.len() {
            return Err(EvalError::InvalidArith(raw.into()));
        }
        (base, &raw[hash + 1..])
    } else if raw.starts_with("0x") || raw.starts_with("0X") {
        (16, &raw[2..])
    } else if raw.len() > 1 && raw.starts_with('0') {
        (8, &raw[1..])
    } else {
        (10, raw)
    };
    if digits.is_empty() {
        return Err(EvalError::InvalidArith(raw.into()));
    }
    let mut value = 0i64;
    for ch in digits.chars() {
        let digit =
            arithmetic_digit(ch, base).ok_or_else(|| EvalError::InvalidArith(raw.into()))?;
        if digit >= base {
            return Err(EvalError::InvalidArith(raw.into()));
        }
        value = value.wrapping_mul(base as i64).wrapping_add(digit as i64);
    }
    Ok(value)
}

fn arithmetic_digit(ch: char, base: u32) -> Option<u32> {
    match ch {
        '0'..='9' => Some(ch as u32 - '0' as u32),
        'a'..='z' => Some(ch as u32 - 'a' as u32 + 10),
        'A'..='Z' if base <= 36 => Some(ch as u32 - 'A' as u32 + 10),
        'A'..='Z' => Some(ch as u32 - 'A' as u32 + 36),
        '@' => Some(62),
        '_' => Some(63),
        _ => None,
    }
}

fn evaluate<E: Env>(expression: &Expr, env: &mut E, depth: usize) -> Result<i64, EvalError> {
    match expression {
        Expr::Number(value) => Ok(*value),
        Expr::Variable(name) => variable_value(name, env, depth),
        Expr::Unary(op, inner) => {
            let value = evaluate(inner, env, depth)?;
            Ok(match op {
                UnaryOp::Plus => value,
                UnaryOp::Minus => value.wrapping_neg(),
                UnaryOp::LogicalNot => i64::from(value == 0),
                UnaryOp::BitwiseNot => !value,
            })
        }
        Expr::Binary(BinaryOp::LogicalAnd, left, right) => {
            let left = evaluate(left, env, depth)?;
            Ok(i64::from(left != 0 && evaluate(right, env, depth)? != 0))
        }
        Expr::Binary(BinaryOp::LogicalOr, left, right) => {
            let left = evaluate(left, env, depth)?;
            Ok(i64::from(left != 0 || evaluate(right, env, depth)? != 0))
        }
        Expr::Binary(op, left, right) => {
            let left = evaluate(left, env, depth)?;
            let right = evaluate(right, env, depth)?;
            apply_binary(*op, left, right)
        }
        Expr::Conditional(condition, when_true, when_false) => {
            if evaluate(condition, env, depth)? != 0 {
                evaluate(when_true, env, depth)
            } else {
                evaluate(when_false, env, depth)
            }
        }
        Expr::Group(inner) => evaluate(inner, env, depth),
        Expr::Assign(name, op, right) => {
            let right = evaluate(right, env, depth)?;
            let value = if matches!(op, AssignOp::Set) {
                right
            } else {
                let left = variable_value(name, env, depth)?;
                apply_assignment(*op, left, right)?
            };
            assign(name, value, env)?;
            Ok(value)
        }
        Expr::PreIncrement(name, delta) => {
            let value = variable_value(name, env, depth)?.wrapping_add(*delta);
            assign(name, value, env)?;
            Ok(value)
        }
        Expr::PostIncrement(name, delta) => {
            let value = variable_value(name, env, depth)?;
            assign(name, value.wrapping_add(*delta), env)?;
            Ok(value)
        }
        Expr::Comma(left, right) => {
            evaluate(left, env, depth)?;
            evaluate(right, env, depth)
        }
    }
}

fn variable_value<E: Env>(name: &str, env: &mut E, depth: usize) -> Result<i64, EvalError> {
    let value = env.var(name).unwrap_or("").to_owned();
    eval_inner(&value, env, depth + 1, name)
}

fn assign<E: Env>(name: &str, value: i64, env: &mut E) -> Result<(), EvalError> {
    if env.set_var(name, value.to_string()) {
        Ok(())
    } else {
        Err(EvalError::ArithmeticAssignmentUnsupported(name.to_owned()))
    }
}

fn apply_assignment(op: AssignOp, left: i64, right: i64) -> Result<i64, EvalError> {
    apply_binary(
        match op {
            AssignOp::Set => unreachable!(),
            AssignOp::Multiply => BinaryOp::Multiply,
            AssignOp::Divide => BinaryOp::Divide,
            AssignOp::Remainder => BinaryOp::Remainder,
            AssignOp::Add => BinaryOp::Add,
            AssignOp::Subtract => BinaryOp::Subtract,
            AssignOp::ShiftLeft => BinaryOp::ShiftLeft,
            AssignOp::ShiftRight => BinaryOp::ShiftRight,
            AssignOp::BitwiseAnd => BinaryOp::BitwiseAnd,
            AssignOp::BitwiseXor => BinaryOp::BitwiseXor,
            AssignOp::BitwiseOr => BinaryOp::BitwiseOr,
        },
        left,
        right,
    )
}

fn apply_binary(op: BinaryOp, left: i64, right: i64) -> Result<i64, EvalError> {
    Ok(match op {
        BinaryOp::Power => {
            if right < 0 {
                return Err(EvalError::NegativeExponent);
            }
            wrapping_pow(left, right as u64)
        }
        BinaryOp::Multiply => left.wrapping_mul(right),
        BinaryOp::Divide => {
            if right == 0 {
                return Err(EvalError::DivisionByZero);
            }
            left.checked_div(right).unwrap_or(i64::MIN)
        }
        BinaryOp::Remainder => {
            if right == 0 {
                return Err(EvalError::DivisionByZero);
            }
            left.checked_rem(right).unwrap_or(0)
        }
        BinaryOp::Add => left.wrapping_add(right),
        BinaryOp::Subtract => left.wrapping_sub(right),
        BinaryOp::ShiftLeft => left.wrapping_shl(right as u32),
        BinaryOp::ShiftRight => left.wrapping_shr(right as u32),
        BinaryOp::Less => i64::from(left < right),
        BinaryOp::LessEqual => i64::from(left <= right),
        BinaryOp::Greater => i64::from(left > right),
        BinaryOp::GreaterEqual => i64::from(left >= right),
        BinaryOp::Equal => i64::from(left == right),
        BinaryOp::NotEqual => i64::from(left != right),
        BinaryOp::BitwiseAnd => left & right,
        BinaryOp::BitwiseXor => left ^ right,
        BinaryOp::BitwiseOr => left | right,
        BinaryOp::LogicalAnd | BinaryOp::LogicalOr => unreachable!(),
    })
}

fn wrapping_pow(mut base: i64, mut exponent: u64) -> i64 {
    let mut result = 1i64;
    while exponent != 0 {
        if exponent & 1 != 0 {
            result = result.wrapping_mul(base);
        }
        exponent >>= 1;
        if exponent != 0 {
            base = base.wrapping_mul(base);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MapEnv;

    fn value(input: &str, env: &mut MapEnv) -> i64 {
        eval(input, env).unwrap()
    }

    #[test]
    fn precedence_and_bases() {
        let mut env = MapEnv::new();
        assert_eq!(value("2 + 3 * 4", &mut env), 14);
        assert_eq!(value("2**3**2", &mut env), 512);
        assert_eq!(value("-2**2", &mut env), 4);
        assert_eq!(value("010 + 0x10 + 2#10 + 64#_", &mut env), 89);
    }

    #[test]
    fn variables_recurse_and_mutate() {
        let mut env = MapEnv::new().with_var("x", "y+1").with_var("y", "2");
        assert_eq!(value("x", &mut env), 3);
        assert_eq!(value("x=7, x++", &mut env), 7);
        assert_eq!(env.var("x"), Some("8"));
    }

    #[test]
    fn lazy_operators_skip_errors_and_assignments() {
        let mut env = MapEnv::new();
        assert_eq!(value("1 || 1/0", &mut env), 1);
        assert_eq!(value("0 ? x=1 : 4", &mut env), 4);
        assert_eq!(env.var("x"), None);
    }

    #[test]
    fn wrapping_and_errors() {
        let mut env = MapEnv::new();
        assert_eq!(value("9223372036854775807+1", &mut env), i64::MIN);
        assert!(matches!(
            eval("1/0", &mut env),
            Err(EvalError::DivisionByZero)
        ));
        assert!(matches!(
            eval("2**-1", &mut env),
            Err(EvalError::NegativeExponent)
        ));
        assert!(matches!(
            eval("0x", &mut env),
            Err(EvalError::InvalidArith(_))
        ));
        assert!(matches!(
            eval("(x)=2", &mut env),
            Err(EvalError::InvalidArith(_))
        ));
    }
}
