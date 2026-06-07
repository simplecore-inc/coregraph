use crate::types::{OutputConfig, OutputFormat, QueryResult};
use crate::OutputSerializer;

pub struct JsonSerializer;

impl OutputSerializer for JsonSerializer {
    fn format(&self) -> OutputFormat {
        OutputFormat::Json
    }

    fn serialize(&self, result: &QueryResult, _config: &OutputConfig) -> String {
        serde_json::to_string_pretty(result).unwrap_or_else(|e| format!("{{\"error\":\"{e}\"}}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OutputConfig, QueryResult};

    #[test]
    fn json_valid_for_empty() {
        let r = QueryResult::empty("Foo".to_string());
        let out = JsonSerializer.serialize(&r, &OutputConfig::default());
        let parsed: serde_json::Value = serde_json::from_str(&out).expect("valid JSON");
        assert_eq!(parsed["symbol_name"], "Foo");
    }

    #[test]
    fn json_format_returns_json() {
        assert_eq!(JsonSerializer.format(), OutputFormat::Json);
    }
}
