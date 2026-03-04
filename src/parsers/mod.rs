mod jest;
mod pytest;
mod rust_test;

use regex::Regex;
use std::sync::LazyLock;

use crate::models::TestError;

pub use jest::JestParser;
pub use pytest::PytestParser;
pub use rust_test::RustTestParser;

// ---------------------------------------------------------------------------
// Shared regexes
// ---------------------------------------------------------------------------

pub static ANSI_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\x1b\[[0-9;]*m").unwrap());

pub static TIMESTAMP_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d+Z\s?").unwrap());

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub fn strip_ansi(text: &str) -> String {
    ANSI_RE.replace_all(text, "").into_owned()
}

pub fn strip_timestamp(line: &str) -> &str {
    TIMESTAMP_RE.find(line).map_or(line, |m| &line[m.end()..])
}

// ---------------------------------------------------------------------------
// Parser trait
// ---------------------------------------------------------------------------

pub trait TestErrorParser: Send + Sync {
    fn framework(&self) -> &'static str;
    fn parse(&self, content: &str, job_name: &str, log_filename: &str) -> Vec<TestError>;
}

pub fn all_parsers() -> Vec<Box<dyn TestErrorParser>> {
    vec![
        Box::new(PytestParser),
        Box::new(JestParser),
        Box::new(RustTestParser),
    ]
}
