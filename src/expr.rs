//! A tiny, dependency-free expression parser/evaluator for a single-variable function `f(x)`.
//!
//! The agent submits a function (the move); we parse it ONCE into an AST ([`Expr`]) and then
//! evaluate it many times along the shot's x-range. Parsing is a standard recursive-descent
//! grammar with the usual precedence:
//!
//! ```text
//! expr   := term (('+' | '-') term)*
//! term   := unary (('*' | '/' | '%') unary)*
//! unary  := ('+' | '-') unary | power
//! power  := atom ('^' unary)?          // right-associative: 2^3^2 = 2^(3^2)
//! atom   := number | name | name '(' expr ')' | '(' expr ')'
//! ```
//!
//! Domain errors (e.g. `sqrt(-1)`, `ln(0)`, `1/0`) do NOT error — they evaluate to `NaN`/`inf`,
//! which the trajectory tracer reads as "the shot explodes here". So ANY *parseable* function is
//! a legal move; only a syntactically broken one is rejected. This mirrors Graphwar, where you can
//! absolutely fire a function that blows up in your face.

/// A parsed expression tree. Cheap to evaluate; built once per move.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Num(f64),
    Var,
    Neg(Box<Expr>),
    Bin(Op, Box<Expr>, Box<Expr>),
    Call(Func, Box<Expr>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Func {
    Sin,
    Cos,
    Tan,
    Asin,
    Acos,
    Atan,
    Sinh,
    Cosh,
    Tanh,
    Sqrt,
    Cbrt,
    Abs,
    Exp,
    Ln,
    Log,
    Log2,
    Floor,
    Ceil,
    Round,
    Sign,
}

impl Expr {
    /// Parse a function expression. Accepts an optional `y =` / `f(x) =` prefix so agents can
    /// write either `sin(x)` or `y = sin(x)`. Returns a human-readable error on a syntax problem.
    pub fn parse(src: &str) -> Result<Expr, String> {
        let src = strip_prefix(src);
        if src.trim().is_empty() {
            return Err("empty function".into());
        }
        let tokens = tokenize(src)?;
        let mut p = Parser { tokens, pos: 0 };
        let e = p.parse_expr()?;
        if p.pos != p.tokens.len() {
            return Err(format!("unexpected trailing input near token {}", p.pos));
        }
        Ok(e)
    }

    /// Evaluate at a given `x`. Domain errors propagate as `NaN`/`inf` (never panics).
    pub fn eval(&self, x: f64) -> f64 {
        match self {
            Expr::Num(n) => *n,
            Expr::Var => x,
            Expr::Neg(a) => -a.eval(x),
            Expr::Bin(op, a, b) => {
                let (a, b) = (a.eval(x), b.eval(x));
                match op {
                    Op::Add => a + b,
                    Op::Sub => a - b,
                    Op::Mul => a * b,
                    Op::Div => a / b,
                    Op::Rem => a % b,
                    Op::Pow => a.powf(b),
                }
            }
            Expr::Call(f, a) => {
                let a = a.eval(x);
                match f {
                    Func::Sin => a.sin(),
                    Func::Cos => a.cos(),
                    Func::Tan => a.tan(),
                    Func::Asin => a.asin(),
                    Func::Acos => a.acos(),
                    Func::Atan => a.atan(),
                    Func::Sinh => a.sinh(),
                    Func::Cosh => a.cosh(),
                    Func::Tanh => a.tanh(),
                    Func::Sqrt => a.sqrt(),
                    Func::Cbrt => a.cbrt(),
                    Func::Abs => a.abs(),
                    Func::Exp => a.exp(),
                    Func::Ln => a.ln(),
                    Func::Log => a.log10(),
                    Func::Log2 => a.log2(),
                    Func::Floor => a.floor(),
                    Func::Ceil => a.ceil(),
                    Func::Round => a.round(),
                    Func::Sign => a.signum(),
                }
            }
        }
    }
}

/// Drop a leading `y =` / `f(x) =` so both notations parse.
fn strip_prefix(s: &str) -> &str {
    let t = s.trim();
    for p in ["y=", "f(x)=", "y(x)="] {
        // Compare ignoring internal whitespace by checking a despaced prefix.
        let despaced: String = t.chars().filter(|c| !c.is_whitespace()).collect();
        if despaced.starts_with(p) {
            // Find the '=' in the original and return everything after it.
            if let Some(eq) = t.find('=') {
                return t[eq + 1..].trim();
            }
        }
    }
    t
}

// ---- tokenizer -------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    LParen,
    RParen,
}

fn tokenize(src: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = src.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                out.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                out.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                // accept `**` as power too
                if i + 1 < chars.len() && chars[i + 1] == '*' {
                    out.push(Tok::Caret);
                    i += 2;
                } else {
                    out.push(Tok::Star);
                    i += 1;
                }
            }
            '/' => {
                out.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                out.push(Tok::Percent);
                i += 1;
            }
            '^' => {
                out.push(Tok::Caret);
                i += 1;
            }
            '(' | '[' | '{' => {
                out.push(Tok::LParen);
                i += 1;
            }
            ')' | ']' | '}' => {
                out.push(Tok::RParen);
                i += 1;
            }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // optional scientific notation: 1e-3
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    let mut j = i + 1;
                    if j < chars.len() && (chars[j] == '+' || chars[j] == '-') {
                        j += 1;
                    }
                    if j < chars.len() && chars[j].is_ascii_digit() {
                        i = j;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                }
                let s: String = chars[start..i].iter().collect();
                let n: f64 = s.parse().map_err(|_| format!("bad number literal '{s}'"))?;
                out.push(Tok::Num(n));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let s: String = chars[start..i].iter().collect();
                out.push(Tok::Ident(s.to_ascii_lowercase()));
            }
            other => return Err(format!("unexpected character '{other}'")),
        }
    }
    Ok(out)
}

// ---- parser ----------------------------------------------------------------

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_term()?;
        while let Some(tok) = self.peek() {
            let op = match tok {
                Tok::Plus => Op::Add,
                Tok::Minus => Op::Sub,
                _ => break,
            };
            self.bump();
            let rhs = self.parse_term()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_term(&mut self) -> Result<Expr, String> {
        let mut lhs = self.parse_unary()?;
        loop {
            // implicit multiplication: `2x`, `2(x+1)`, `2sin(x)` — if the next token starts a
            // new atom with no operator, treat it as a `*`.
            let op = match self.peek() {
                Some(Tok::Star) => Some(Op::Mul),
                Some(Tok::Slash) => Some(Op::Div),
                Some(Tok::Percent) => Some(Op::Rem),
                Some(Tok::Num(_)) | Some(Tok::Ident(_)) | Some(Tok::LParen) => Some(Op::Mul),
                _ => None,
            };
            let Some(op) = op else { break };
            if matches!(op, Op::Mul | Op::Div | Op::Rem)
                && matches!(
                    self.peek(),
                    Some(Tok::Star) | Some(Tok::Slash) | Some(Tok::Percent)
                )
            {
                self.bump();
            }
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, String> {
        match self.peek() {
            Some(Tok::Minus) => {
                self.bump();
                Ok(Expr::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Tok::Plus) => {
                self.bump();
                self.parse_unary()
            }
            _ => self.parse_power(),
        }
    }

    fn parse_power(&mut self) -> Result<Expr, String> {
        let base = self.parse_atom()?;
        if matches!(self.peek(), Some(Tok::Caret)) {
            self.bump();
            // right-associative, and the exponent may itself be a unary (e.g. x^-2)
            let exp = self.parse_unary()?;
            Ok(Expr::Bin(Op::Pow, Box::new(base), Box::new(exp)))
        } else {
            Ok(base)
        }
    }

    fn parse_atom(&mut self) -> Result<Expr, String> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::LParen) => {
                let e = self.parse_expr()?;
                match self.bump() {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err("missing closing parenthesis".into()),
                }
            }
            Some(Tok::Ident(name)) => {
                if let Some(f) = func_of(&name) {
                    match self.bump() {
                        Some(Tok::LParen) => {}
                        _ => {
                            return Err(format!(
                                "function '{name}' needs parentheses, e.g. {name}(x)"
                            ))
                        }
                    }
                    let arg = self.parse_expr()?;
                    match self.bump() {
                        Some(Tok::RParen) => Ok(Expr::Call(f, Box::new(arg))),
                        _ => Err(format!("missing ')' after {name}(...)")),
                    }
                } else if let Some(c) = const_of(&name) {
                    Ok(Expr::Num(c))
                } else if name == "x" {
                    Ok(Expr::Var)
                } else {
                    Err(format!(
                        "unknown name '{name}' (only variable 'x', constants pi/e/tau, and the listed functions are allowed)"
                    ))
                }
            }
            other => Err(format!("expected a value, found {other:?}")),
        }
    }
}

fn func_of(name: &str) -> Option<Func> {
    Some(match name {
        "sin" => Func::Sin,
        "cos" => Func::Cos,
        "tan" => Func::Tan,
        "asin" | "arcsin" => Func::Asin,
        "acos" | "arccos" => Func::Acos,
        "atan" | "arctan" => Func::Atan,
        "sinh" => Func::Sinh,
        "cosh" => Func::Cosh,
        "tanh" => Func::Tanh,
        "sqrt" => Func::Sqrt,
        "cbrt" => Func::Cbrt,
        "abs" => Func::Abs,
        "exp" => Func::Exp,
        "ln" => Func::Ln,
        "log" | "log10" => Func::Log,
        "log2" => Func::Log2,
        "floor" => Func::Floor,
        "ceil" => Func::Ceil,
        "round" => Func::Round,
        "sign" | "sgn" | "signum" => Func::Sign,
        _ => return None,
    })
}

fn const_of(name: &str) -> Option<f64> {
    Some(match name {
        "pi" => std::f64::consts::PI,
        "tau" => std::f64::consts::TAU,
        "e" => std::f64::consts::E,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str, x: f64) -> f64 {
        Expr::parse(s).unwrap().eval(x)
    }

    #[test]
    fn arithmetic_and_precedence() {
        assert_eq!(ev("2 + 3 * 4", 0.0), 14.0);
        assert_eq!(ev("(2 + 3) * 4", 0.0), 20.0);
        assert_eq!(ev("2 ^ 3 ^ 2", 0.0), 512.0); // right-assoc
        assert_eq!(ev("-x", 5.0), -5.0);
        assert_eq!(ev("x^2", 4.0), 16.0);
    }

    #[test]
    fn variable_and_constants() {
        assert_eq!(ev("x", 7.5), 7.5);
        assert!((ev("pi", 0.0) - std::f64::consts::PI).abs() < 1e-12);
        assert!((ev("2*x + 3", 1.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn functions() {
        assert!((ev("sin(0)", 0.0)).abs() < 1e-12);
        assert!((ev("cos(0)", 0.0) - 1.0).abs() < 1e-12);
        assert!((ev("sqrt(x)", 9.0) - 3.0).abs() < 1e-12);
        assert!((ev("abs(x)", -4.0) - 4.0).abs() < 1e-12);
    }

    #[test]
    fn prefix_and_implicit_mul() {
        assert_eq!(ev("y = 2*x", 3.0), 6.0);
        assert_eq!(ev("2x", 3.0), 6.0); // implicit multiplication
        assert_eq!(ev("3sin(0) + 4", 0.0), 4.0);
        assert_eq!(ev("x**2", 5.0), 25.0); // python-style power
    }

    #[test]
    fn domain_errors_are_nan_not_panic() {
        assert!(ev("sqrt(x)", -1.0).is_nan());
        assert!(ev("ln(0)", 0.0).is_infinite());
        assert!(ev("1/0", 0.0).is_infinite());
    }

    #[test]
    fn syntax_errors_rejected() {
        assert!(Expr::parse("2 +").is_err());
        assert!(Expr::parse("sin x").is_err()); // needs parens
        assert!(Expr::parse("foo(x)").is_err()); // unknown function
        assert!(Expr::parse("(2+3").is_err()); // unbalanced
        assert!(Expr::parse("").is_err());
    }
}
