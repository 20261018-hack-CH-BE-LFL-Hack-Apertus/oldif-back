pub fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 200
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.:@".contains(c))
}
pub fn valid_locale(value: &str) -> bool {
    matches!(value, "en" | "es" | "de" | "de-CH" | "fr" | "it" | "rm")
}
pub fn valid_question(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 8000
}
pub fn tabular_cell(value: &str) -> String {
    let flat = value.replace(['\t', '\n', '\r'], " ");
    if flat.trim_start().starts_with(['=', '+', '-', '@']) {
        format!("'{flat}")
    } else {
        flat
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_inputs() {
        assert!(!valid_id("../source"));
        assert!(!valid_locale("xx"));
        assert!(!valid_question(&"x".repeat(8001)));
        assert!(valid_locale("de-CH"));
    }
    #[test]
    fn exports_cannot_run_spreadsheet_formulas() {
        assert_eq!(tabular_cell("=1+1"), "'=1+1");
        assert_eq!(tabular_cell("  @SUM(A1)"), "'  @SUM(A1)");
        assert_eq!(tabular_cell("1799\tBern"), "1799 Bern");
    }
}
