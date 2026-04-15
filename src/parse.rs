//! Parser for bash conditional expressions.
//!
//! Grammar:
//!
//! ```text
//! expr      := or_expr
//! or_expr   := and_expr ( '||' and_expr )*
//! and_expr  := not_expr ( '&&' not_expr )*
//! not_expr  := '!' not_expr | atom
//! atom      := '[[' expr ']]' | primary
//! primary   := unary_op word
//!            | word binary_op word
//!            | word              -- equivalent to `-n word`
//! ```
//!
//! Operator-as-word tokens (e.g. `-f`, `-eq`) come through the lexer as
//! `Token::Word`; the parser inspects their literal text to classify them.
//! There is no `( … )` grouping; use `[[ … ]]` to be explicit.

use crate::ast::{BinaryOp, Expr, Primary, UnaryOp, Word};
use crate::error::ParseError;
use crate::lex::{Spanned, Token, lex};

pub fn parse(input: &str) -> Result<Expr, ParseError> {
    let tokens = lex(input)?;
    let mut p = Parser::new(tokens);
    let expr = p.parse_expr()?;
    if !p.at_end() {
        let t = p.peek().unwrap();
        return Err(ParseError::UnexpectedToken {
            token: token_display(&t.value),
            pos: t.start,
        });
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Spanned<Token>>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Spanned<Token>>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn at_end(&self) -> bool {
        self.pos >= self.tokens.len()
    }

    fn peek(&self) -> Option<&Spanned<Token>> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Spanned<Token>> {
        if self.pos < self.tokens.len() {
            let i = self.pos;
            self.pos += 1;
            Some(self.tokens[i].clone())
        } else {
            None
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, ParseError> {
        self.parse_or()
    }

    fn parse_or(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_and()?;
        while matches!(self.peek().map(|t| &t.value), Some(Token::OrOr)) {
            self.advance();
            let rhs = self.parse_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_not()?;
        while matches!(self.peek().map(|t| &t.value), Some(Token::AndAnd)) {
            self.advance();
            let rhs = self.parse_not()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_not(&mut self) -> Result<Expr, ParseError> {
        if matches!(self.peek().map(|t| &t.value), Some(Token::Bang)) {
            self.advance();
            let inner = self.parse_not()?;
            return Ok(Expr::Not(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        match self.peek().map(|t| t.value.clone()) {
            Some(Token::OpenBracket) => {
                let open = self.advance().unwrap();
                let inner = self.parse_expr()?;
                match self.peek().map(|t| &t.value) {
                    Some(Token::CloseBracket) => {
                        self.advance();
                        Ok(inner)
                    }
                    _ => Err(ParseError::UnterminatedDoubleBracket { pos: open.start }),
                }
            }
            Some(Token::OpenParen) | Some(Token::CloseParen) => {
                Err(ParseError::ParensNotSupported)
            }
            Some(Token::CloseBracket) => {
                let t = self.peek().unwrap();
                Err(ParseError::UnexpectedToken {
                    token: "]]".to_string(),
                    pos: t.start,
                })
            }
            None => Err(ParseError::UnexpectedEof),
            _ => self.parse_primary().map(Expr::Primary),
        }
    }

    fn parse_primary(&mut self) -> Result<Primary, ParseError> {
        // Need to peek the leading token. If it's a Word that names a unary
        // operator, consume it and read the operand.
        let first_word = self.peek_word_text();
        if let Some(text) = first_word {
            if text == "-a" || text == "-o" {
                // -a / -o are legal as the file/option-name unary primaries
                // when followed by a word that is the operand. But they are
                // ALSO the legacy combinators we banned. Disambiguate: in our
                // grammar -a means FileExists and -o means ShellOptSet; we
                // accept them as unary. We only reject them as combinators
                // (which would need to appear between primaries — not here).
            }
            if let Some(op) = UnaryOp::from_token(text) {
                self.advance();
                let arg = self.expect_word(op.token())?;
                return Ok(Primary::Unary { op, arg });
            }
        }

        // Otherwise: read the LHS word.
        let lhs = match self.advance() {
            Some(Spanned {
                value: Token::Word(w),
                ..
            }) => w,
            Some(other) => {
                return Err(ParseError::UnexpectedToken {
                    token: token_display(&other.value),
                    pos: other.start,
                });
            }
            None => return Err(ParseError::UnexpectedEof),
        };

        // Check for a binary operator next.
        let bop = self.peek_binary_op();
        if let Some(op) = bop {
            self.advance();
            if matches!(op, BinaryOp::RegexMatch) {
                let rhs = match self.advance() {
                    Some(Spanned {
                        value: Token::Word(w),
                        ..
                    }) => w,
                    _ => return Err(ParseError::RegexMissingRhs),
                };
                return Ok(Primary::Binary { op, lhs, rhs });
            }
            let rhs = self.expect_word(op.token())?;
            return Ok(Primary::Binary { op, lhs, rhs });
        }

        // Bare word.
        Ok(Primary::StringNonEmpty(lhs))
    }

    fn peek_word_text(&self) -> Option<&str> {
        match self.peek().map(|t| &t.value) {
            Some(Token::Word(w)) => single_literal(w),
            _ => None,
        }
    }

    fn peek_binary_op(&self) -> Option<BinaryOp> {
        match self.peek().map(|t| &t.value) {
            Some(Token::Eq) => Some(BinaryOp::GlobMatch),
            Some(Token::NotEq) => Some(BinaryOp::GlobNotMatch),
            Some(Token::RegexEq) => Some(BinaryOp::RegexMatch),
            Some(Token::Lt) => Some(BinaryOp::StrLt),
            Some(Token::Gt) => Some(BinaryOp::StrGt),
            Some(Token::Word(w)) => single_literal(w).and_then(BinaryOp::from_token),
            _ => None,
        }
    }

    fn expect_word(&mut self, op: &str) -> Result<Word, ParseError> {
        match self.advance() {
            Some(Spanned {
                value: Token::Word(w),
                ..
            }) => Ok(w),
            _ => Err(ParseError::ExpectedWord { op: op.to_string() }),
        }
    }
}

/// If the word is a single unquoted Literal, return its text — used to
/// recognize words that name operators.
fn single_literal(w: &Word) -> Option<&str> {
    if w.parts.len() != 1 {
        return None;
    }
    match &w.parts[0] {
        crate::ast::WordPart::Literal(s) => Some(s.as_str()),
        _ => None,
    }
}

fn token_display(t: &Token) -> String {
    match t {
        Token::OpenBracket => "[[".to_string(),
        Token::CloseBracket => "]]".to_string(),
        Token::AndAnd => "&&".to_string(),
        Token::OrOr => "||".to_string(),
        Token::Bang => "!".to_string(),
        Token::OpenParen => "(".to_string(),
        Token::CloseParen => ")".to_string(),
        Token::Eq => "==".to_string(),
        Token::NotEq => "!=".to_string(),
        Token::RegexEq => "=~".to_string(),
        Token::Lt => "<".to_string(),
        Token::Gt => ">".to_string(),
        Token::Word(w) => w.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Primary, UnaryOp, Word, WordPart};

    fn p(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("parse({s:?}) failed: {e}"))
    }

    #[test]
    fn implicit_unary() {
        let e = p("-f foo.txt");
        assert_eq!(
            e,
            Expr::Primary(Primary::Unary {
                op: UnaryOp::FileRegular,
                arg: Word::literal("foo.txt"),
            })
        );
    }

    #[test]
    fn explicit_brackets() {
        let e = p("[[ -f foo.txt ]]");
        assert_eq!(
            e,
            Expr::Primary(Primary::Unary {
                op: UnaryOp::FileRegular,
                arg: Word::literal("foo.txt"),
            })
        );
    }

    #[test]
    fn implicit_and_chain() {
        let e = p("-e foo && -f foo");
        match e {
            Expr::And(a, b) => {
                assert!(matches!(*a, Expr::Primary(Primary::Unary { op: UnaryOp::FileExists, .. })));
                assert!(matches!(*b, Expr::Primary(Primary::Unary { op: UnaryOp::FileRegular, .. })));
            }
            _ => panic!("expected And, got {e:?}"),
        }
    }

    #[test]
    fn mixed_implicit_explicit() {
        let e = p("[[ -e foo ]] && -f bar");
        assert!(matches!(e, Expr::And(_, _)));
    }

    #[test]
    fn precedence_or_lower_than_and() {
        // a && b || c && d  =>  (a && b) || (c && d)
        let e = p("-e a && -e b || -e c && -e d");
        match e {
            Expr::Or(l, r) => {
                assert!(matches!(*l, Expr::And(_, _)));
                assert!(matches!(*r, Expr::And(_, _)));
            }
            _ => panic!("expected Or"),
        }
    }

    #[test]
    fn not_prefix() {
        let e = p("! -f foo");
        match e {
            Expr::Not(inner) => assert!(matches!(*inner, Expr::Primary(_))),
            _ => panic!("expected Not"),
        }
    }

    #[test]
    fn binary_string_eq() {
        let e = p("$x == foo*");
        match e {
            Expr::Primary(Primary::Binary { op: BinaryOp::GlobMatch, lhs, rhs }) => {
                assert_eq!(lhs.parts, vec![WordPart::Var("x".into())]);
                assert_eq!(rhs.parts, vec![WordPart::Literal("foo*".into())]);
            }
            _ => panic!("expected Binary GlobMatch, got {e:?}"),
        }
    }

    #[test]
    fn binary_arith_lt() {
        let e = p("$x -lt 10");
        assert!(matches!(
            e,
            Expr::Primary(Primary::Binary {
                op: BinaryOp::ArithLt,
                ..
            })
        ));
    }

    #[test]
    fn regex_match() {
        let e = p("$line =~ ^foo");
        match e {
            Expr::Primary(Primary::Binary { op: BinaryOp::RegexMatch, .. }) => {}
            _ => panic!("expected RegexMatch, got {e:?}"),
        }
    }

    #[test]
    fn bare_word_means_nonempty() {
        let e = p("$x");
        match e {
            Expr::Primary(Primary::StringNonEmpty(w)) => {
                assert_eq!(w.parts, vec![WordPart::Var("x".into())]);
            }
            _ => panic!("expected StringNonEmpty, got {e:?}"),
        }
    }

    #[test]
    fn parens_are_not_supported() {
        assert!(matches!(parse("( -f x )"), Err(ParseError::ParensNotSupported)));
    }

    #[test]
    fn unterminated_brackets() {
        assert!(matches!(
            parse("[[ -f x"),
            Err(ParseError::UnterminatedDoubleBracket { .. })
        ));
    }

    #[test]
    fn nested_brackets() {
        // [[ ... && [[ ... ]] ]]
        let e = p("[[ -e a && [[ -f b || -d c ]] ]]");
        match e {
            Expr::And(_, r) => assert!(matches!(*r, Expr::Or(_, _))),
            _ => panic!("expected And"),
        }
    }
}
