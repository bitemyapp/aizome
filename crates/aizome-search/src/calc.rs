//! Arithmetic typed into the palette ("15% of 80", "2^10/3", "sqrt(2)*pi"),
//! answered in place, as Spotlight does. Soroban, the calculator, uses the
//! same evaluator for its typed input.
//!
//! The grammar, loosest first:
//!
//! ```text
//! sum     = product (("+" | "-") product)*
//! product = unary (("*" | "×" | "/" | "÷" | "mod") unary | unary)*   (juxtaposition multiplies: 2pi, 3(4))
//! unary   = ("-" | "+" | "−") unary | power
//! power   = postfix ("^" | "**") unary
//! postfix = primary ("!" | "%")*                                      ("15%" is 0.15; "15% of 80" is 12)
//! primary = number | constant | function "(" sum ("," sum)* ")" | "(" sum ")"
//! ```

use std::fmt;

/// Why an expression has no value.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    Syntax(String),
    UnknownName(String),
    /// Division by zero, the square root of a negative, and the like.
    Domain(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax(s) => write!(f, "can't read {s}"),
            Error::UnknownName(n) => write!(f, "unknown name {n}"),
            Error::Domain(s) => f.write_str(s),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Num(f64),
    Name(String),
    Op(char),
    Open,
    Close,
    Comma,
}

fn tokenize(src: &str) -> Result<Vec<Token>, Error> {
    let mut out = Vec::new();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '0'..='9' | '.' => {
                let start = i;
                if c == '0' && matches!(chars.get(i + 1), Some('x' | 'X')) {
                    i += 2;
                    let hs = i;
                    while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                        i += 1;
                    }
                    let digits: String = chars[hs..i].iter().filter(|c| **c != '_').collect();
                    let v = u64::from_str_radix(&digits, 16).map_err(|_| Error::Syntax(digits.clone()))?;
                    out.push(Token::Num(v as f64));
                    continue;
                }
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_') {
                    i += 1;
                }
                // An exponent: 1e3, 2.5E-4 (but not "2e" followed by a name).
                if i < chars.len()
                    && matches!(chars[i], 'e' | 'E')
                    && (chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())
                        || (matches!(chars.get(i + 1), Some('+' | '-'))
                            && chars.get(i + 2).is_some_and(|c| c.is_ascii_digit())))
                {
                    i += 2;
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().filter(|c| **c != '_').collect();
                out.push(Token::Num(text.parse().map_err(|_| Error::Syntax(text.clone()))?));
            }
            '+' | '-' | '−' | '*' | '×' | '·' | '/' | '÷' | '^' | '!' | '%' => {
                if c == '*' && chars.get(i + 1) == Some(&'*') {
                    out.push(Token::Op('^'));
                    i += 2;
                    continue;
                }
                let op = match c {
                    '−' => '-',
                    '×' | '·' => '*',
                    '÷' => '/',
                    c => c,
                };
                out.push(Token::Op(op));
                i += 1;
            }
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            ',' => {
                out.push(Token::Comma);
                i += 1;
            }
            'π' => {
                out.push(Token::Name("pi".into()));
                i += 1;
            }
            'τ' => {
                out.push(Token::Name("tau".into()));
                i += 1;
            }
            c if c.is_alphabetic() => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                out.push(Token::Name(chars[start..i].iter().collect::<String>().to_lowercase()));
            }
            c => return Err(Error::Syntax(c.to_string())),
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.at).cloned();
        self.at += 1;
        t
    }

    fn sum(&mut self) -> Result<f64, Error> {
        let mut v = self.product()?;
        loop {
            match self.peek() {
                Some(Token::Op('+')) => {
                    self.at += 1;
                    v += self.product()?;
                }
                Some(Token::Op('-')) => {
                    self.at += 1;
                    v -= self.product()?;
                }
                _ => return Ok(v),
            }
        }
    }

    fn product(&mut self) -> Result<f64, Error> {
        let mut v = self.unary()?;
        loop {
            match self.peek() {
                Some(Token::Op('*')) => {
                    self.at += 1;
                    v *= self.unary()?;
                }
                Some(Token::Op('/')) => {
                    self.at += 1;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return Err(Error::Domain("division by zero"));
                    }
                    v /= d;
                }
                Some(Token::Name(n)) if n == "mod" => {
                    self.at += 1;
                    let d = self.unary()?;
                    if d == 0.0 {
                        return Err(Error::Domain("division by zero"));
                    }
                    v = v.rem_euclid(d);
                }
                // "15% of 80": the percent already divided by a hundred.
                Some(Token::Name(n)) if n == "of" => {
                    self.at += 1;
                    v *= self.unary()?;
                }
                // Juxtaposition: 2pi, 3(4), 2 sqrt(2).
                Some(Token::Num(_) | Token::Name(_) | Token::Open) => v *= self.power()?,
                _ => return Ok(v),
            }
        }
    }

    fn unary(&mut self) -> Result<f64, Error> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.at += 1;
                Ok(-self.unary()?)
            }
            Some(Token::Op('+')) => {
                self.at += 1;
                self.unary()
            }
            _ => self.power(),
        }
    }

    fn power(&mut self) -> Result<f64, Error> {
        let base = self.postfix()?;
        if self.peek() == Some(&Token::Op('^')) {
            self.at += 1;
            let exp = self.unary()?;
            return Ok(base.powf(exp));
        }
        Ok(base)
    }

    fn postfix(&mut self) -> Result<f64, Error> {
        let mut v = self.primary()?;
        loop {
            match self.peek() {
                Some(Token::Op('!')) => {
                    self.at += 1;
                    if v < 0.0 || v.fract() != 0.0 || v > 170.0 {
                        return Err(Error::Domain("factorial of a non-integer or too large"));
                    }
                    v = (1..=v as u64).map(|n| n as f64).product();
                }
                Some(Token::Op('%')) => {
                    self.at += 1;
                    v /= 100.0;
                }
                _ => return Ok(v),
            }
        }
    }

    fn primary(&mut self) -> Result<f64, Error> {
        match self.next() {
            Some(Token::Num(v)) => Ok(v),
            Some(Token::Open) => {
                let v = self.sum()?;
                match self.next() {
                    Some(Token::Close) | None => Ok(v), // a missing ")" at the end is forgiven while typing
                    Some(t) => Err(Error::Syntax(format!("{t:?}"))),
                }
            }
            Some(Token::Name(name)) => match name.as_str() {
                "pi" => Ok(std::f64::consts::PI),
                "tau" => Ok(std::f64::consts::TAU),
                "e" => Ok(std::f64::consts::E),
                _ => {
                    if self.peek() != Some(&Token::Open) {
                        // sqrt 2, sin pi: a function applied to the next factor.
                        if is_function(&name) {
                            let arg = self.power()?;
                            return apply(&name, &[arg]);
                        }
                        return Err(Error::UnknownName(name));
                    }
                    self.at += 1;
                    let mut args = vec![self.sum()?];
                    while self.peek() == Some(&Token::Comma) {
                        self.at += 1;
                        args.push(self.sum()?);
                    }
                    match self.next() {
                        Some(Token::Close) | None => {}
                        Some(t) => return Err(Error::Syntax(format!("{t:?}"))),
                    }
                    apply(&name, &args)
                }
            },
            Some(t) => Err(Error::Syntax(format!("{t:?}"))),
            None => Err(Error::Syntax("the end".into())),
        }
    }
}

const FUNCTIONS: &[&str] = &[
    "sqrt", "cbrt", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "ln", "log", "log2", "log10",
    "exp", "abs", "floor", "ceil", "round", "min", "max", "hypot", "atan2",
];

fn is_function(name: &str) -> bool {
    FUNCTIONS.contains(&name)
}

fn apply(name: &str, args: &[f64]) -> Result<f64, Error> {
    let one = |f: fn(f64) -> f64| -> Result<f64, Error> {
        match args {
            [x] => Ok(f(*x)),
            _ => Err(Error::Syntax(format!("{name} takes one value"))),
        }
    };
    let x = args.first().copied().unwrap_or(f64::NAN);
    match name {
        "sqrt" if x < 0.0 => Err(Error::Domain("square root of a negative number")),
        "sqrt" => one(f64::sqrt),
        "cbrt" => one(f64::cbrt),
        "sin" => one(f64::sin),
        "cos" => one(f64::cos),
        "tan" => one(f64::tan),
        "asin" => one(f64::asin),
        "acos" => one(f64::acos),
        "atan" => one(f64::atan),
        "sinh" => one(f64::sinh),
        "cosh" => one(f64::cosh),
        "tanh" => one(f64::tanh),
        "ln" | "log" | "log2" | "log10" if x <= 0.0 => Err(Error::Domain("logarithm of zero or a negative number")),
        "ln" => one(f64::ln),
        "log" | "log10" => match args {
            [x, base] => Ok(x.ln() / base.ln()),
            _ => one(f64::log10),
        },
        "log2" => one(f64::log2),
        "exp" => one(f64::exp),
        "abs" => one(f64::abs),
        "floor" => one(f64::floor),
        "ceil" => one(f64::ceil),
        "round" => one(f64::round),
        "min" => args.iter().copied().reduce(f64::min).ok_or(Error::Syntax("min of nothing".into())),
        "max" => args.iter().copied().reduce(f64::max).ok_or(Error::Syntax("max of nothing".into())),
        "hypot" | "atan2" => match args {
            [a, b] if name == "hypot" => Ok(a.hypot(*b)),
            [a, b] => Ok(a.atan2(*b)),
            _ => Err(Error::Syntax(format!("{name} takes two values"))),
        },
        _ => Err(Error::UnknownName(name.to_owned())),
    }
}

/// Evaluates an expression.
pub fn eval(src: &str) -> Result<f64, Error> {
    let tokens = tokenize(src)?;
    if tokens.is_empty() {
        return Err(Error::Syntax("nothing".into()));
    }
    let mut p = Parser { tokens, at: 0 };
    let v = p.sum()?;
    if p.at < p.tokens.len() {
        return Err(Error::Syntax(format!("{:?}", p.tokens[p.at])));
    }
    if v.is_nan() {
        return Err(Error::Domain("not a number"));
    }
    Ok(v)
}

/// Whether text typed into the palette looks like arithmetic rather than a
/// search: it has a digit or a constant, and an operator or a function. "e"
/// and "log" alone are searches.
pub fn looks_like_math(src: &str) -> bool {
    let Ok(tokens) = tokenize(src) else { return false };
    let has_value = tokens.iter().any(|t| matches!(t, Token::Num(_)));
    let has_op = tokens.iter().any(|t| matches!(t, Token::Op(_)))
        || tokens.iter().any(|t| matches!(t, Token::Name(n) if is_function(n) || n == "of" || n == "mod"));
    has_value && (has_op || tokens.len() > 1) && tokens.iter().all(|t| !matches!(t, Token::Name(n) if !is_known(n)))
}

fn is_known(name: &str) -> bool {
    is_function(name) || matches!(name, "pi" | "tau" | "e" | "of" | "mod")
}

/// Formats a value for display: up to 12 significant digits, no trailing
/// zeros, thousands grouped with thin spaces, scientific notation beyond
/// that range.
pub fn format(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    if !v.is_finite() {
        return if v > 0.0 { "∞".into() } else { "−∞".into() };
    }
    let mag = v.abs().log10().floor() as i32;
    if !(-6..15).contains(&mag) {
        let s = format!("{v:.10e}");
        let (m, e) = s.split_once('e').unwrap_or((&s, "0"));
        let m = m.trim_end_matches('0').trim_end_matches('.');
        return format!("{m} × 10^{e}").replace('-', "−");
    }
    let decimals = (11 - mag).clamp(0, 15) as usize;
    let s = format!("{:.*}", decimals, v.abs());
    let s = if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s };
    let (int, frac) = s.split_once('.').map_or((s.as_str(), None), |(a, b)| (a, Some(b)));
    let mut grouped = String::new();
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push('\u{2009}');
        }
        grouped.push(c);
    }
    let sign = if v < 0.0 { "−" } else { "" };
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn evaluates() {
        assert_eq!(eval("1 + 2 * 3").unwrap(), 7.0);
        assert_eq!(eval("(1 + 2) * 3").unwrap(), 9.0);
        assert_eq!(eval("2^10").unwrap(), 1024.0);
        assert_eq!(eval("2^3^2").unwrap(), 512.0);
        assert_eq!(eval("-2^2").unwrap(), -4.0);
        assert_eq!(eval("15% of 80").unwrap(), 12.0);
        assert_eq!(eval("50%").unwrap(), 0.5);
        assert_eq!(eval("5!").unwrap(), 120.0);
        assert_eq!(eval("10 mod 3").unwrap(), 1.0);
        assert_eq!(eval("0x1F").unwrap(), 31.0);
        assert_eq!(eval("1_000 × 3 ÷ 4").unwrap(), 750.0);
        assert!(close(eval("2pi").unwrap(), std::f64::consts::TAU));
        assert!(close(eval("sqrt(2)*sqrt 2").unwrap(), 2.0));
        assert!(close(eval("log(8, 2)").unwrap(), 3.0));
        assert_eq!(eval("max(1, 5, 3)").unwrap(), 5.0);
        assert_eq!(eval("(1+2").unwrap(), 3.0);
        assert_eq!(eval("1e3 + 2.5E-1").unwrap(), 1000.25);
    }

    #[test]
    fn errors() {
        assert_eq!(eval("1/0"), Err(Error::Domain("division by zero")));
        assert!(matches!(eval("foo(2)"), Err(Error::UnknownName(_))));
        assert!(matches!(eval("1 +"), Err(Error::Syntax(_))));
        assert!(eval("sqrt(-1)").is_err());
    }

    #[test]
    fn tells_math_from_search() {
        assert!(looks_like_math("2+2"));
        assert!(looks_like_math("15% of 80"));
        assert!(looks_like_math("sqrt 2"));
        assert!(!looks_like_math("e"));
        assert!(!looks_like_math("log"));
        assert!(!looks_like_math("firefox"));
        assert!(!looks_like_math("find rep"));
        assert!(!looks_like_math("42"));
        assert!(!looks_like_math("2 apples"));
    }

    #[test]
    fn formats() {
        assert_eq!(format(1234567.0), "1\u{2009}234\u{2009}567");
        assert_eq!(format(0.1 + 0.2), "0.3");
        assert_eq!(format(-2.5), "−2.5");
        assert_eq!(format(1.0 / 3.0), "0.333333333333");
        assert_eq!(format(6.02e23), "6.02 × 10^23");
    }
}
