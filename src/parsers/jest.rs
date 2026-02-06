use regex::Regex;
use std::sync::LazyLock;

use crate::models::{ErrorOccurrence, TestError};
use super::{strip_ansi, strip_timestamp, TestErrorParser};

static JEST_FAIL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"FAIL\s+(?:browser:\s*(\S+)\s+)?(\S+\.(?:tsx?|jsx?|mjs|cjs))").unwrap()
});

static JEST_ERROR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"●\s+(.+)$").unwrap());

static JEST_AT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s+at\s+").unwrap());

static JEST_LINE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[:/](\d+):\d+\)?$").unwrap());

pub struct JestParser;

impl TestErrorParser for JestParser {
    fn framework(&self) -> &'static str {
        "jest"
    }

    fn parse(&self, content: &str, job_name: &str, log_filename: &str) -> Vec<TestError> {
        let clean = strip_ansi(content);
        let lines: Vec<&str> = clean.lines().collect();
        let mut errors = Vec::new();

        let mut i = 0;
        while i < lines.len() {
            let stripped = strip_timestamp(lines[i]);

            let Some(fail_caps) = JEST_FAIL_RE.captures(stripped) else {
                i += 1;
                continue;
            };

            let browser = fail_caps.get(1).map(|m| m.as_str().to_string());
            let test_file = fail_caps.get(2).unwrap().as_str().to_string();

            // Scan for ● error lines within this FAIL block
            let mut j = i + 1;
            let mut found_error_in_block = false;

            while j < lines.len() {
                let inner = strip_timestamp(lines[j]);

                // Stop at next FAIL line, PASS line, or Test Suites summary
                if JEST_FAIL_RE.is_match(inner)
                    || inner.starts_with("PASS ")
                    || inner.starts_with("Test Suites:")
                {
                    break;
                }

                if let Some(err_caps) = JEST_ERROR_RE.captures(inner) {
                    found_error_in_block = true;
                    let raw_name = err_caps.get(1).unwrap().as_str().trim().to_string();

                    // Skip console noise — only real errors
                    if raw_name == "Console" {
                        j += 1;
                        continue;
                    }

                    // Look ahead for error message and stack trace
                    let mut message = String::new();
                    let mut stack_lines = Vec::new();
                    let mut line_number: Option<u32> = None;

                    let mut k = j + 1;
                    // Skip blank lines
                    while k < lines.len() {
                        let l = strip_timestamp(lines[k]).trim();
                        if !l.is_empty() {
                            break;
                        }
                        k += 1;
                    }

                    // Collect message and stack trace
                    while k < lines.len() {
                        let l = strip_timestamp(lines[k]);
                        let trimmed = l.trim();

                        // Stop conditions
                        if JEST_ERROR_RE.is_match(l)
                            || JEST_FAIL_RE.is_match(l)
                            || l.starts_with("PASS ")
                            || trimmed.starts_with("Test Suites:")
                        {
                            break;
                        }

                        if JEST_AT_RE.is_match(l) {
                            stack_lines.push(trimmed.to_string());
                            if line_number.is_none() {
                                if let Some(lm) = JEST_LINE_RE.captures(trimmed) {
                                    line_number =
                                        lm.get(1).unwrap().as_str().parse().ok();
                                }
                            }
                        } else if message.is_empty() && !trimmed.is_empty() {
                            message = trimmed.to_string();
                        }

                        k += 1;
                    }

                    // Extract error_type from message (split on first colon)
                    let (error_type, final_message) =
                        if let Some(colon_pos) = message.find(':') {
                            let et = message[..colon_pos].trim().to_string();
                            let msg = message[colon_pos + 1..].trim().to_string();
                            (et, msg)
                        } else {
                            (raw_name.clone(), message)
                        };

                    let test_name = if let Some(ref b) = browser {
                        format!("{} [{}]", raw_name, b)
                    } else {
                        raw_name
                    };

                    let traceback = if !stack_lines.is_empty() {
                        Some(stack_lines.join("\n"))
                    } else {
                        None
                    };

                    errors.push(TestError {
                        framework: self.framework().to_string(),
                        test_file: test_file.clone(),
                        test_name,
                        error_type,
                        message: final_message,
                        line: line_number,
                        occurrences: vec![ErrorOccurrence {
                            job: job_name.to_string(),
                            log_file: log_filename.to_string(),
                            traceback,
                        }],
                    });

                    j = k;
                    continue;
                }

                j += 1;
            }

            // If we saw a FAIL line but no ● lines, still record a generic error
            if !found_error_in_block {
                let test_name = if let Some(ref b) = browser {
                    format!("(suite) [{}]", b)
                } else {
                    "(suite)".to_string()
                };

                errors.push(TestError {
                    framework: self.framework().to_string(),
                    test_file: test_file.clone(),
                    test_name,
                    error_type: "SuiteFailed".to_string(),
                    message: String::new(),
                    line: None,
                    occurrences: vec![ErrorOccurrence {
                        job: job_name.to_string(),
                        log_file: log_filename.to_string(),
                        traceback: None,
                    }],
                });
            }

            i = j.max(i + 1);
        }

        errors
    }
}
