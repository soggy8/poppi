pub struct Calculator;

impl Calculator {
    pub fn evaluate(expression: &str) -> Result<f64, String> {
        // Remove common calculator symbols and replace with proper operators
        let cleaned = expression
            .replace('×', "*")
            .replace('÷', "/")
            .replace('x', "*")
            .replace('X', "*");

        // Try to evaluate the expression using meval
        match meval::eval_str(&cleaned) {
            Ok(result) => Ok(result),
            Err(e) => Err(format!("Invalid expression: {}", e)),
        }
    }

    pub fn is_calculation(query: &str) -> bool {
        // Simple heuristic: if it contains numbers and math operators, it's likely a calculation
        let has_number = query.chars().any(|c| c.is_ascii_digit());
        let has_operator = query.chars().any(|c| "+-*/×÷xX^()".contains(c));
        
        has_number && (has_operator || query.trim().parse::<f64>().is_ok())
    }

    pub fn format_result(result: f64) -> String {
        // Format as integer if it's a whole number, otherwise keep decimals
        if result.fract() == 0.0 {
            format!("{}", result as i64)
        } else {
            // Round to reasonable precision
            format!("{:.10}", result)
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_calculation() {
        assert_eq!(Calculator::evaluate("2+2"), Ok(4.0));
        assert_eq!(Calculator::evaluate("10*5"), Ok(50.0));
    }
}
