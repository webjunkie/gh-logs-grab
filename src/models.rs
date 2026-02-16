use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// GitHub API types
// ---------------------------------------------------------------------------

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Step {
    pub name: String,
    pub conclusion: Option<String>,
    pub number: u64,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub name: String,
    pub conclusion: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Serialize, Deserialize)]
pub struct RunMetadata {
    pub run_id: String,
    pub run_number: u64,
    pub head_sha: String,
    pub head_branch: String,
    pub pr_number: Option<u64>,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    pub total_jobs: usize,
    pub failed_jobs: usize,
    pub downloaded_at: String,
    pub jobs: Vec<Job>,
}

// ---------------------------------------------------------------------------
// Error data model (framework-agnostic)
// ---------------------------------------------------------------------------

fn default_framework() -> String {
    "pytest".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct TestError {
    #[serde(default = "default_framework")]
    pub framework: String,
    pub test_file: String,
    pub test_name: String,
    pub error_type: String,
    pub message: String,
    pub line: Option<u32>,
    pub occurrences: Vec<ErrorOccurrence>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ErrorOccurrence {
    pub job: String,
    pub log_file: String,
    pub traceback: Option<String>,
}

#[derive(Serialize, Deserialize)]
pub struct Findings {
    pub analyzed_at: String,
    pub run_id: String,
    #[serde(default)]
    pub jobs_overview: Vec<JobOverview>,
    pub errors: Vec<TestError>,
    pub summary: FindingsSummary,
}

// ---------------------------------------------------------------------------
// Jobs/steps overview
// ---------------------------------------------------------------------------

#[derive(Serialize, Deserialize, Clone)]
pub struct JobOverview {
    pub job_name: String,
    pub conclusion: String,
    pub duration_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed_steps: Vec<FailedStepOverview>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FailedStepOverview {
    pub name: String,
    pub conclusion: String,
    pub number: u64,
    pub duration_secs: Option<i64>,
}

#[derive(Serialize, Deserialize)]
pub struct FindingsSummary {
    pub total_unique_errors: usize,
    pub total_error_occurrences: usize,
    pub jobs_analyzed: usize,
    #[serde(default)]
    pub by_framework: HashMap<String, FrameworkSummary>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FrameworkSummary {
    pub unique_errors: usize,
    pub total_occurrences: usize,
}
