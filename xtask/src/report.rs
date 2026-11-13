//! Machine-readable reports and exit-code conventions.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Status {
    Pass,
    Fail,
    Error,
}

#[derive(Serialize)]
pub(crate) struct CheckResult {
    pub name: String,
    pub status: Status,
    pub duration_ms: u128,
    pub failure_summary: Option<String>,
    pub artifacts: Vec<String>,
}

impl CheckResult {
    pub fn error(name: &str, message: String) -> Self {
        Self {
            name: name.into(),
            status: Status::Error,
            duration_ms: 0,
            failure_summary: Some(message),
            artifacts: Vec::new(),
        }
    }
}

#[derive(Default, Serialize)]
pub(crate) struct Report {
    pub checks: Vec<CheckResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub performance: Option<serde_json::Value>,
}

impl Report {
    pub fn exit_code(&self) -> u8 {
        if self.checks.iter().any(|item| item.status == Status::Error) {
            2
        } else if self.checks.iter().any(|item| item.status == Status::Fail) {
            1
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environment_errors_take_precedence_over_check_failures() {
        let mut report = Report::default();
        assert_eq!(report.exit_code(), 0);
        let mut item = CheckResult::error("fmt", String::new());
        item.status = Status::Fail;
        report.checks.push(item);
        assert_eq!(report.exit_code(), 1);
        report
            .checks
            .push(CheckResult::error("doctor", String::new()));
        assert_eq!(report.exit_code(), 2);
    }
}
