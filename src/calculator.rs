//! A small arithmetic evaluator: `+ - * / % ^`, parentheses, unary minus,
//! common functions and the constants `pi`, `e` and `tau`.
//! `x`, `×` and `÷` are accepted as operators, so `3x4` works.

pub struct Calculator;

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Num(f64),
    Ident(String),
    Op(char),
    Of,
    LParen,
    RParen,
    Comma,
}

fn parse_radix_prefix(chars: &[char], start: usize) -> Option<(f64, usize)> {
    if chars.get(start) != Some(&'0') {
        return None;
    }
    let (radix, valid): (u32, fn(char) -> bool) = match chars.get(start + 1)? {
        'x' | 'X' => (16, |c| c.is_ascii_hexdigit()),
        'b' | 'B' => (2, |c| matches!(c, '0' | '1')),
        'o' | 'O' => (8, |c| matches!(c, '0'..='7')),
        _ => return None,
    };
    let mut i = start + 2;
    let mut digits = String::new();
    while i < chars.len() && (valid(chars[i]) || chars[i] == '_') {
        if chars[i] != '_' {
            digits.push(chars[i]);
        }
        i += 1;
    }
    if digits.is_empty() {
        return None;
    }
    let value = u64::from_str_radix(&digits, radix).ok()? as f64;
    Some((value, i))
}

fn tokenize(input: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if let Some((value, next)) = parse_radix_prefix(&chars, i) {
            tokens.push(Token::Num(value));
            i = next;
        } else if c.is_ascii_digit() || c == '.' {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit() || chars[i] == '.' || chars[i] == '_')
            {
                i += 1;
            }
            let raw: String = chars[start..i].iter().collect();
            let clean: String = raw.chars().filter(|&ch| ch != '_').collect();
            let value = clean
                .parse::<f64>()
                .map_err(|_| format!("Invalid number: {}", raw))?;
            tokens.push(Token::Num(value));
        } else if c.is_alphabetic() {
            let start = i;
            while i < chars.len() && (chars[i].is_alphabetic() || chars[i].is_ascii_digit()) {
                i += 1;
            }
            // A digit only belongs to a name when it follows letters (log2, log10),
            // so "2x3" still reads as 2 * 3.
            let mut word: String = chars[start..i].iter().collect::<String>().to_lowercase();
            if word.starts_with('x') && word[1..].chars().all(|d| d.is_ascii_digit()) {
                i = start + 1;
                word = "x".to_string();
            }
            if word == "x" {
                tokens.push(Token::Op('*'));
            } else if word == "of" {
                tokens.push(Token::Of);
            } else {
                tokens.push(Token::Ident(word));
            }
        } else {
            let token = match c {
                '+' | '-' | '/' | '%' | '^' => Token::Op(c),
                '*' => {
                    if chars.get(i + 1) == Some(&'*') {
                        i += 1;
                        Token::Op('^')
                    } else {
                        Token::Op('*')
                    }
                }
                '×' => Token::Op('*'),
                '÷' => Token::Op('/'),
                '(' => Token::LParen,
                ')' => Token::RParen,
                ',' => Token::Comma,
                other => return Err(format!("Unexpected character: {}", other)),
            };
            tokens.push(token);
            i += 1;
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn next(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.pos).cloned();
        self.pos += 1;
        token
    }

    fn peek_at(&self, offset: usize) -> Option<&Token> {
        self.tokens.get(self.pos + offset)
    }

    fn expr(&mut self) -> Result<f64, String> {
        let (mut value, mut lhs_percent) = self.term_with_percent()?;
        while let Some(Token::Op(op @ ('+' | '-'))) = self.peek().cloned() {
            self.pos += 1;
            let (rhs, rhs_percent) = self.term_with_percent()?;
            value = match (op, rhs_percent && !lhs_percent) {
                ('+', true) => value * (1.0 + rhs),
                (_, true) => value * (1.0 - rhs),
                ('+', false) => value + rhs,
                (_, false) => value - rhs,
            };
            lhs_percent = false;
        }
        Ok(value)
    }

    /// Returns `(evaluated_term, was_single_trailing_percent)`.
    fn term_with_percent(&mut self) -> Result<(f64, bool), String> {
        let (mut value, mut single_percent) = self.unary_with_percent()?;
        loop {
            match self.peek().cloned() {
                Some(Token::Op(op @ ('*' | '/' | '%'))) => {
                    self.pos += 1;
                    let (rhs, _) = self.unary_with_percent()?;
                    value = match op {
                        '*' => value * rhs,
                        '/' => value / rhs,
                        _ => value % rhs,
                    };
                    single_percent = false;
                }
                Some(Token::Of) => {
                    self.pos += 1;
                    let (rhs, _) = self.unary_with_percent()?;
                    value *= rhs;
                    single_percent = false;
                }
                _ => break,
            }
        }
        Ok((value, single_percent))
    }

    fn unary_with_percent(&mut self) -> Result<(f64, bool), String> {
        match self.peek() {
            Some(Token::Op('-')) => {
                self.pos += 1;
                let (v, pct) = self.unary_with_percent()?;
                Ok((-v, pct))
            }
            Some(Token::Op('+')) => {
                self.pos += 1;
                self.unary_with_percent()
            }
            _ => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<(f64, bool), String> {
        let base = self.power()?;
        // `%` is postfix percentage unless followed by a modulo right-hand operand.
        let is_postfix_pct = self.peek() == Some(&Token::Op('%'))
            && !matches!(
                self.peek_at(1),
                Some(Token::Num(_) | Token::Ident(_) | Token::LParen)
            );
        if is_postfix_pct {
            self.pos += 1;
            return Ok((base / 100.0, true));
        }
        Ok((base, false))
    }

    fn power(&mut self) -> Result<f64, String> {
        let base = self.primary()?;
        if let Some(Token::Op('^')) = self.peek() {
            self.pos += 1;
            // Right associative, and binds tighter than unary minus on the left.
            let (exponent, _) = self.unary_with_percent()?;
            return Ok(base.powf(exponent));
        }
        Ok(base)
    }

    fn primary(&mut self) -> Result<f64, String> {
        match self.next() {
            Some(Token::Num(n)) => Ok(n),
            Some(Token::LParen) => {
                let value = self.expr()?;
                self.expect_rparen()?;
                Ok(value)
            }
            Some(Token::Ident(name)) => {
                if let Some(Token::LParen) = self.peek() {
                    self.pos += 1;
                    let mut args = vec![self.expr()?];
                    while let Some(Token::Comma) = self.peek() {
                        self.pos += 1;
                        args.push(self.expr()?);
                    }
                    self.expect_rparen()?;
                    apply_function(&name, &args)
                } else {
                    match name.as_str() {
                        "pi" => Ok(std::f64::consts::PI),
                        "e" => Ok(std::f64::consts::E),
                        "tau" => Ok(std::f64::consts::TAU),
                        _ => Err(format!("Unknown name: {}", name)),
                    }
                }
            }
            Some(token) => Err(format!("Unexpected token: {:?}", token)),
            None => Err("Unexpected end of expression".to_string()),
        }
    }

    fn expect_rparen(&mut self) -> Result<(), String> {
        match self.next() {
            Some(Token::RParen) => Ok(()),
            _ => Err("Missing closing parenthesis".to_string()),
        }
    }
}

fn apply_function(name: &str, args: &[f64]) -> Result<f64, String> {
    let unary = |f: fn(f64) -> f64| {
        if args.len() == 1 {
            Ok(f(args[0]))
        } else {
            Err(format!("{} takes one argument", name))
        }
    };
    match name {
        "sqrt" => unary(f64::sqrt),
        "abs" => unary(f64::abs),
        "sin" => unary(f64::sin),
        "cos" => unary(f64::cos),
        "tan" => unary(f64::tan),
        "asin" => unary(f64::asin),
        "acos" => unary(f64::acos),
        "atan" => unary(f64::atan),
        "ln" => unary(f64::ln),
        "log" | "log10" => unary(f64::log10),
        "log2" => unary(f64::log2),
        "exp" => unary(f64::exp),
        "floor" => unary(f64::floor),
        "ceil" => unary(f64::ceil),
        "round" => unary(f64::round),
        "min" => Ok(args.iter().copied().fold(f64::INFINITY, f64::min)),
        "max" => Ok(args.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
        _ => Err(format!("Unknown function: {}", name)),
    }
}

impl Calculator {
    pub fn evaluate(expression: &str) -> Result<f64, String> {
        let tokens = tokenize(expression)?;
        if tokens.is_empty() {
            return Err("Empty expression".to_string());
        }
        let mut parser = Parser { tokens, pos: 0 };
        let value = parser.expr()?;
        if parser.pos < parser.tokens.len() {
            return Err("Unexpected trailing input".to_string());
        }
        if !value.is_finite() {
            return Err("Result is not a finite number".to_string());
        }
        Ok(value)
    }

    /// Cheap pre-check: a digit plus something that makes it more than a bare
    /// number. A bare number such as "2048" stays an app search.
    pub fn is_calculation(query: &str) -> bool {
        let has_number = query.chars().any(|c| c.is_ascii_digit());
        let has_operator = query.chars().any(|c| "+-*/×÷xX^%()".contains(c));
        has_number && has_operator
    }

    pub fn format_result(result: f64) -> String {
        if result.fract() == 0.0 && result.abs() < 1e15 {
            return format!("{}", result as i64);
        }
        let text = format!("{:.10}", result);
        let trimmed = text.trim_end_matches('0').trim_end_matches('.');
        if trimmed == "0" || trimmed == "-0" || result.abs() >= 1e15 {
            // Too small or too large for fixed notation.
            format!("{:e}", result)
        } else {
            trimmed.to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval(s: &str) -> f64 {
        Calculator::evaluate(s).unwrap()
    }

    #[test]
    fn basic_arithmetic() {
        assert_eq!(eval("2+2"), 4.0);
        assert_eq!(eval("10*5-3"), 47.0);
        assert_eq!(eval("(5+3)*2"), 16.0);
        assert_eq!(eval("7 % 4"), 3.0);
        assert_eq!(eval("1.5 + .5"), 2.0);
    }

    #[test]
    fn precedence_and_associativity() {
        assert_eq!(eval("2+3*4"), 14.0);
        assert_eq!(eval("2^3^2"), 512.0);
        assert_eq!(eval("-2^2"), -4.0);
        assert_eq!(eval("2^-1"), 0.5);
        assert_eq!(eval("2**3"), 8.0);
        assert_eq!(eval("--3"), 3.0);
    }

    #[test]
    fn x_is_multiplication_but_not_inside_names() {
        assert_eq!(eval("3x4"), 12.0);
        assert_eq!(eval("3 X 4"), 12.0);
        assert_eq!(eval("3×4"), 12.0);
        assert_eq!(eval("8÷2"), 4.0);
        assert_eq!(eval("max(1, 7, 3)"), 7.0);
        assert!((eval("exp(1)") - std::f64::consts::E).abs() < 1e-12);
    }

    #[test]
    fn functions_and_constants() {
        assert_eq!(eval("sqrt(16)"), 4.0);
        assert_eq!(eval("min(4, 2)"), 2.0);
        assert_eq!(eval("log2(8)"), 3.0);
        assert_eq!(eval("log10(1000)"), 3.0);
        assert!((eval("2*pi") - std::f64::consts::TAU).abs() < 1e-12);
    }

    #[test]
    fn errors() {
        assert!(Calculator::evaluate("1/0").is_err());
        assert!(Calculator::evaluate("2+").is_err());
        assert!(Calculator::evaluate("(2+3").is_err());
        assert!(Calculator::evaluate("gtk4-demo").is_err());
        assert!(Calculator::evaluate("2 3").is_err());
        assert!(Calculator::evaluate("").is_err());
    }

    #[test]
    fn detection() {
        assert!(Calculator::is_calculation("2+2"));
        assert!(Calculator::is_calculation("sqrt(16)"));
        assert!(!Calculator::is_calculation("2048"));
        assert!(!Calculator::is_calculation("firefox"));
    }

    #[test]
    fn formatting() {
        assert_eq!(Calculator::format_result(4.0), "4");
        assert_eq!(Calculator::format_result(0.5), "0.5");
        assert_eq!(Calculator::format_result(1.0 / 3.0), "0.3333333333");
        assert_eq!(Calculator::format_result(-12.0), "-12");
        assert_eq!(Calculator::format_result(1e-12), "1e-12");
    }

    #[test]
    fn percentages_and_radix_literals() {
        assert!((eval("15% of 240") - 36.0).abs() < 1e-9);
        assert!((eval("80 + 19%") - 95.2).abs() < 1e-9);
        assert!((eval("100 - 20%") - 80.0).abs() < 1e-9);
        assert!((eval("200 * 15%") - 30.0).abs() < 1e-9);
        assert!((eval("50% + 10%") - 0.6).abs() < 1e-9);
        // Binary modulo still works when followed by an operand.
        assert_eq!(eval("7 % 4"), 3.0);
        assert_eq!(eval("80 + 7 % 4"), 83.0);

        assert_eq!(eval("0xff + 1"), 256.0);
        assert_eq!(eval("0b1010 * 3"), 30.0);
        assert_eq!(eval("0o755 - 0o700"), 45.0);
        assert_eq!(eval("1_000_000 / 1_000"), 1000.0);
    }
}
