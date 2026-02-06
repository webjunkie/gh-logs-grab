use regex::Regex;
use std::sync::LazyLock;

use crate::models::{ErrorOccurrence, TestError};
use super::TestErrorParser;

static PYTEST_FAILED_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(FAILED|ERROR)\s+([^\s]+)\s+-\s+(.*)$").unwrap());

static LINE_NUMBER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r":(\d+):").unwrap());

pub struct PytestParser;

impl TestErrorParser for PytestParser {
    fn framework(&self) -> &'static str {
        "pytest"
    }

    fn parse(&self, content: &str, job_name: &str, log_filename: &str) -> Vec<TestError> {
        let mut errors = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        let mut i = 0;
        while i < lines.len() {
            let line = lines[i];

            if let Some(captures) = PYTEST_FAILED_RE.captures(line) {
                let test_path = captures.get(2).unwrap().as_str();
                let error_info = captures.get(3).unwrap().as_str();

                let (error_type, initial_message) =
                    if let Some(colon_pos) = error_info.find(':') {
                        let et = error_info[..colon_pos].trim().to_string();
                        let msg = error_info[colon_pos + 1..].trim();
                        (et, msg)
                    } else {
                        (error_info.trim().to_string(), "")
                    };

                let parts: Vec<&str> = test_path.split("::").collect();
                let test_file = parts[0].to_string();
                let test_name = parts[1..].join("::");

                let mut message = initial_message.to_string();
                let mut line_number = None;
                let mut traceback_lines = Vec::new();

                let mut j = i + 1;
                while j < lines.len() && j < i + 30 {
                    let next_line = lines[j];

                    if next_line.starts_with("FAILED") || next_line.starts_with("=====") {
                        break;
                    }

                    if next_line.trim().starts_with("E   ") {
                        let err_line = next_line.trim()[4..].trim();
                        if !message.is_empty() && !err_line.is_empty() {
                            traceback_lines.push(err_line.to_string());
                        }
                    }

                    if next_line.contains(".py:") {
                        if let Some(line_match) = LINE_NUMBER_RE.captures(next_line) {
                            line_number =
                                line_match.get(1).unwrap().as_str().parse().ok();
                        }
                    }

                    j += 1;
                }

                if !traceback_lines.is_empty() && message.is_empty() {
                    message = traceback_lines[0].clone();
                }

                let traceback = if !traceback_lines.is_empty() {
                    Some(traceback_lines.join("\n"))
                } else {
                    None
                };

                errors.push(TestError {
                    framework: self.framework().to_string(),
                    test_file,
                    test_name,
                    error_type,
                    message,
                    line: line_number,
                    occurrences: vec![ErrorOccurrence {
                        job: job_name.to_string(),
                        log_file: log_filename.to_string(),
                        traceback,
                    }],
                });
            }

            i += 1;
        }

        errors
    }
}
