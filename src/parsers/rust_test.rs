use regex::Regex;
use std::sync::LazyLock;

use crate::models::{ErrorOccurrence, TestError};
use super::{strip_ansi, strip_timestamp, TestErrorParser};

// ---- test_name stdout ----
static RUST_FAILURE_HEADER_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^---- (.+?) stdout ----$").unwrap());

// thread 'test_name' panicked at path/to/file.rs:42:10:
// thread 'test_name' (12345) panicked at path/to/file.rs:42:10:
static RUST_PANIC_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^thread '(.+?)' (?:\(\d+\) )?panicked at (.+?):(\d+):\d+:$").unwrap()
});

// Running tests/grpc_integration.rs (target/debug/deps/grpc_integration-abc123)
static RUST_RUNNING_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"Running\s+(.+?)\s+\(target/").unwrap()
});

pub struct RustTestParser;

impl TestErrorParser for RustTestParser {
    fn framework(&self) -> &'static str {
        "rust"
    }

    fn parse(&self, content: &str, job_name: &str, log_filename: &str) -> Vec<TestError> {
        let clean = strip_ansi(content);
        let lines: Vec<&str> = clean.lines().collect();
        let mut errors = Vec::new();

        // Track the current test binary/file from "Running ..." lines
        let mut current_test_file = String::new();

        let mut i = 0;
        while i < lines.len() {
            let stripped = strip_timestamp(lines[i]);

            // Track which test binary is running
            if let Some(caps) = RUST_RUNNING_RE.captures(stripped) {
                current_test_file = caps.get(1).unwrap().as_str().to_string();
                i += 1;
                continue;
            }

            // Look for failure detail headers: ---- test_name stdout ----
            let Some(header_caps) = RUST_FAILURE_HEADER_RE.captures(stripped) else {
                i += 1;
                continue;
            };

            let test_name = header_caps.get(1).unwrap().as_str().to_string();

            // Scan within this failure block for panic info and stack trace
            let mut test_file = current_test_file.clone();
            let mut error_type = String::new();
            let mut message = String::new();
            let mut line_number: Option<u32> = None;
            let mut stack_lines = Vec::new();
            let mut in_backtrace = false;

            let mut j = i + 1;
            while j < lines.len() {
                let inner = strip_timestamp(lines[j]).trim();

                // Stop at next failure header or the failures summary list
                if RUST_FAILURE_HEADER_RE.is_match(inner)
                    || inner == "failures:"
                    || inner.starts_with("test result:")
                {
                    break;
                }

                if let Some(panic_caps) = RUST_PANIC_RE.captures(inner) {
                    let panic_file = panic_caps.get(2).unwrap().as_str();
                    line_number = panic_caps.get(3).unwrap().as_str().parse().ok();
                    // Use panic location as test_file if it's a real source path
                    if panic_file.contains('/') {
                        test_file = panic_file.to_string();
                    }

                    // Next line is the error message
                    let mut k = j + 1;
                    while k < lines.len() {
                        let msg_line = strip_timestamp(lines[k]).trim();
                        if !msg_line.is_empty() {
                            message = msg_line.to_string();
                            break;
                        }
                        k += 1;
                    }

                    // Extract error type from message
                    error_type = extract_error_type(&message);
                } else if inner == "stack backtrace:" {
                    in_backtrace = true;
                } else if in_backtrace {
                    if inner.is_empty()
                        || inner.starts_with("note:")
                        || RUST_PANIC_RE.is_match(inner)
                    {
                        in_backtrace = false;
                    } else {
                        stack_lines.push(inner.to_string());
                    }
                }

                j += 1;
            }

            if error_type.is_empty() {
                error_type = "panic".to_string();
            }

            let traceback = if !stack_lines.is_empty() {
                // Merge frame pairs (function + location) and filter noise
                let mut merged = Vec::new();
                let mut k = 0;
                while k < stack_lines.len() {
                    let line = &stack_lines[k];
                    // Check if next line is an "at ..." location
                    let full = if k + 1 < stack_lines.len()
                        && stack_lines[k + 1].trim_start().starts_with("at ")
                    {
                        let pair = format!("{} {}", line, stack_lines[k + 1].trim());
                        k += 2;
                        pair
                    } else {
                        k += 1;
                        line.clone()
                    };
                    merged.push(full);
                }
                // Keep only application frames
                let app_frames: Vec<&String> = merged
                    .iter()
                    .filter(|l| {
                        !l.contains("/rustc/")
                            && !l.contains("/.cargo/registry/")
                            && !l.contains("core::ops::function")
                    })
                    .collect();
                let frames = if app_frames.is_empty() {
                    &merged
                } else {
                    &app_frames.into_iter().cloned().collect::<Vec<_>>()
                };
                Some(frames.join("\n"))
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

            i = j.max(i + 1);
        }

        errors
    }
}

fn extract_error_type(message: &str) -> String {
    // "called `Result::unwrap()` on an `Err` value: Store(Etcd(...))"
    if message.starts_with("called `Result::unwrap()`") {
        return "unwrap".to_string();
    }
    if message.starts_with("called `Option::unwrap()`") {
        return "unwrap".to_string();
    }
    // "assertion failed: ..." or "assertion `left == right` failed"
    if message.starts_with("assertion") {
        return "assertion".to_string();
    }
    // "index out of bounds"
    if message.starts_with("index out of bounds") {
        return "index_out_of_bounds".to_string();
    }
    // Generic: split on first colon
    if let Some(colon_pos) = message.find(':') {
        let prefix = message[..colon_pos].trim();
        if prefix.len() < 40 && !prefix.contains(' ') {
            return prefix.to_string();
        }
    }
    "panic".to_string()
}
