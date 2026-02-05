use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures::future::join_all;
use rayon::prelude::*;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

#[derive(Parser)]
#[command(name = "gh-logs-grab")]
#[command(about = "Download and analyze GitHub Actions logs blazingly fast")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Download logs from a GitHub Actions run
    Download {
        /// GitHub Actions run URL (e.g., https://github.com/owner/repo/actions/runs/123456)
        /// Can also be a job URL - will extract the run ID
        run_url: String,

        /// Output directory for logs (defaults to ./logs)
        #[arg(short, long, default_value = "logs")]
        output: PathBuf,

        /// GitHub token (reads from GITHUB_TOKEN env var if not provided)
        #[arg(short, long, env = "GITHUB_TOKEN")]
        token: Option<String>,

        /// Download all logs (by default only failed logs are downloaded)
        #[arg(short, long)]
        all: bool,
    },
    /// Analyze logs in a run directory to extract pytest errors
    Analyze {
        /// Path to run directory (e.g., logs/pr-123/19374816456)
        run_dir: PathBuf,
    },
    /// Generate timeline analysis across multiple runs
    Timeline {
        /// Path to PR directory (e.g., logs/pr-123)
        pr_dir: PathBuf,
    },
    /// Generate timing analysis across multiple runs
    Timings {
        /// Path to PR directory (e.g., logs/pr-123)
        pr_dir: PathBuf,
    },
}

#[derive(Deserialize, Debug)]
struct JobsResponse {
    jobs: Vec<Job>,
}

#[derive(Deserialize, Debug, Clone, Serialize)]
struct Job {
    id: u64,
    name: String,
    conclusion: Option<String>,
    started_at: Option<String>,
    completed_at: Option<String>,
}

#[derive(Deserialize, Debug)]
struct WorkflowRun {
    head_branch: String,
    event: String,
    head_sha: String,
    run_number: u64,
    created_at: String,
    updated_at: String,
    html_url: String,
    #[serde(default)]
    pull_requests: Vec<PullRequest>,
}

#[derive(Deserialize, Debug)]
struct PullRequest {
    number: u64,
}

#[derive(Serialize, Deserialize)]
struct RunMetadata {
    run_id: String,
    run_number: u64,
    head_sha: String,
    head_branch: String,
    pr_number: Option<u64>,
    html_url: String,
    created_at: String,
    updated_at: String,
    total_jobs: usize,
    failed_jobs: usize,
    downloaded_at: String,
    jobs: Vec<Job>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct PytestError {
    test_file: String,
    test_name: String,
    error_type: String,
    message: String,
    line: Option<u32>,
    occurrences: Vec<ErrorOccurrence>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct ErrorOccurrence {
    job: String,
    log_file: String,
    traceback: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Findings {
    analyzed_at: String,
    run_id: String,
    errors: Vec<PytestError>,
    summary: FindingsSummary,
}

#[derive(Serialize, Deserialize)]
struct FindingsSummary {
    total_unique_errors: usize,
    total_error_occurrences: usize,
    jobs_analyzed: usize,
}

async fn get_github_token() -> Result<String> {
    // Try gh CLI first
    let output = tokio::process::Command::new("gh")
        .args(["auth", "token"])
        .output()
        .await;

    if let Ok(output) = output {
        if output.status.success() {
            return Ok(String::from_utf8(output.stdout)?.trim().to_string());
        }
    }

    anyhow::bail!("No GitHub token found. Set GITHUB_TOKEN env var or use --token")
}

fn parse_run_url(url: &str) -> Result<(String, String, String)> {
    // Parse: https://github.com/owner/repo/actions/runs/123456
    // Or: https://github.com/owner/repo/actions/runs/123456/job/456789?pr=123
    let parts: Vec<&str> = url.split('/').collect();
    if parts.len() < 7 || parts[5] != "actions" || parts[6] != "runs" {
        anyhow::bail!("Invalid GitHub Actions run URL format");
    }

    let owner = parts[3].to_string();
    let repo = parts[4].to_string();

    // Strip query params from run_id
    let run_id = parts[7].split('?').next().unwrap().to_string();

    Ok((owner, repo, run_id))
}

fn build_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", token)).unwrap(),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(USER_AGENT, HeaderValue::from_static("gh-logs-grab"));
    headers.insert("X-GitHub-Api-Version", HeaderValue::from_static("2022-11-28"));
    headers
}

async fn fetch_run_info(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> Result<WorkflowRun> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs/{}",
        owner, repo, run_id
    );

    let resp = client
        .get(&url)
        .send()
        .await
        .context("Failed to fetch run info")?;

    if !resp.status().is_success() {
        anyhow::bail!("API request failed: {}", resp.status());
    }

    Ok(resp.json().await?)
}

async fn fetch_jobs(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> Result<Vec<Job>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs/{}/jobs?per_page=100",
        owner, repo, run_id
    );

    println!("Fetching jobs for run {}...", run_id);

    let mut all_jobs = Vec::new();
    let mut page = 1;

    loop {
        let page_url = format!("{}&page={}", url, page);
        let resp = client
            .get(&page_url)
            .send()
            .await
            .context("Failed to fetch jobs")?;

        if !resp.status().is_success() {
            anyhow::bail!("API request failed: {}", resp.status());
        }

        let jobs_response: JobsResponse = resp.json().await?;

        if jobs_response.jobs.is_empty() {
            break;
        }

        all_jobs.extend(jobs_response.jobs);
        page += 1;
    }

    println!("Found {} jobs", all_jobs.len());
    Ok(all_jobs)
}

async fn download_job_logs(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    job: &Job,
    job_output_dir: &PathBuf,
) -> Result<()> {
    let conclusion = job.conclusion.as_deref().unwrap_or("unknown");
    let sanitized_name = job
        .name
        .replace('/', "_")
        .replace('\\', "_")
        .replace(':', "_")
        .replace(' ', "_");

    let filename = format!("{}-{}.log", sanitized_name, conclusion);
    let filepath = job_output_dir.join(filename);

    // Check if file already exists
    if filepath.exists() {
        let metadata = fs::metadata(&filepath).await?;
        println!("Downloading: {} ... SKIP (exists, {} bytes)", job.name, metadata.len());
        return Ok(());
    }

    print!("Downloading: {} ... ", job.name);

    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/jobs/{}/logs",
        owner, repo, job.id
    );

    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        println!("SKIP (no logs)");
        return Ok(());
    }

    let content = resp.bytes().await?;
    let size = content.len();

    // Create parent directory if it doesn't exist
    if let Some(parent) = filepath.parent() {
        fs::create_dir_all(parent).await?;
    }

    fs::write(&filepath, content).await?;

    println!("OK ({} bytes)", size);

    Ok(())
}

async fn download_command(
    run_url: String,
    output: PathBuf,
    token: Option<String>,
    all: bool,
) -> Result<()> {
    // Get token
    let token = match token {
        Some(t) => t,
        None => get_github_token().await?,
    };

    // Parse URL
    let (owner, repo, run_id) = parse_run_url(&run_url)?;
    println!("Owner: {}, Repo: {}, Run ID: {}", owner, repo, run_id);

    // Build HTTP client
    let headers = build_headers(&token);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    // Fetch run info to get PR number
    let run_info = fetch_run_info(&client, &owner, &repo, &run_id).await?;

    // Determine folder structure: {pr_num}/{run_id}/ or {branch}/{run_id}/
    let pr_number = run_info.pull_requests.first().map(|pr| pr.number);
    let base_dir = if let Some(pr_num) = pr_number {
        output.join(format!("pr-{}", pr_num))
    } else {
        // Fallback to branch name if not a PR
        let safe_branch = run_info.head_branch.replace('/', "_");
        output.join(safe_branch)
    };

    let run_output_dir = base_dir.join(&run_id);
    fs::create_dir_all(&run_output_dir).await?;
    println!("Output directory: {}", run_output_dir.display());

    // Fetch all jobs
    let jobs = fetch_jobs(&client, &owner, &repo, &run_id).await?;
    let total_jobs = jobs.len();

    // Filter to failed jobs unless --all flag is set
    let mut jobs_to_download = jobs.clone();
    if !all {
        jobs_to_download.retain(|job| {
            job.conclusion.as_deref() != Some("success") && job.conclusion.is_some()
        });
        println!("Filtered to {} failed jobs", jobs_to_download.len());
    }

    let failed_jobs_count = jobs_to_download.len();

    if jobs_to_download.is_empty() {
        println!("No jobs to download!");
        return Ok(());
    }

    // Download all logs in parallel
    println!("\nDownloading logs in parallel...\n");

    let download_tasks: Vec<_> = jobs_to_download
        .iter()
        .map(|job| download_job_logs(&client, &owner, &repo, job, &run_output_dir))
        .collect();

    let results = join_all(download_tasks).await;

    // Check for errors
    let mut failed = 0;
    for result in results {
        if result.is_err() {
            failed += 1;
        }
    }

    println!("\n✓ Downloaded {} logs", jobs_to_download.len() - failed);
    if failed > 0 {
        println!("✗ Failed: {}", failed);
    }

    // Write metadata file
    let metadata = RunMetadata {
        run_id: run_id.clone(),
        run_number: run_info.run_number,
        head_sha: run_info.head_sha,
        head_branch: run_info.head_branch,
        pr_number,
        html_url: run_info.html_url,
        created_at: run_info.created_at,
        updated_at: run_info.updated_at,
        total_jobs,
        failed_jobs: failed_jobs_count,
        downloaded_at: chrono::Utc::now().to_rfc3339(),
        jobs: jobs.clone(),
    };

    let metadata_path = run_output_dir.join("metadata.json");
    let metadata_json = serde_json::to_string_pretty(&metadata)?;
    fs::write(&metadata_path, metadata_json).await?;
    println!("✓ Wrote metadata to {}", metadata_path.display());

    // Auto-analyze after download
    println!("\n→ Analyzing downloaded logs...");
    if let Err(e) = analyze_command(run_output_dir).await {
        eprintln!("Warning: Failed to analyze logs: {}", e);
    }

    Ok(())
}

use std::collections::HashMap;
use regex::Regex;

fn parse_pytest_errors(log_content: &str, job_name: &str, log_filename: &str) -> Vec<PytestError> {
    let mut errors = Vec::new();
    let lines: Vec<&str> = log_content.lines().collect();

    // Pattern: FAILED posthog/api/test_capture.py::TestCapture::test_alias - AssertionError: ...
    //       or ERROR posthog/api/test_capture.py::TestCapture::test_alias - django.db.utils.IntegrityError: ...
    //       or FAILED posthog/api/test.py::test_foo - assert False
    // May have timestamp prefix like: 2025-11-14T22:35:35.6475486Z FAILED ...
    let failed_pattern = Regex::new(r"(FAILED|ERROR)\s+([^\s]+)\s+-\s+(.*)$").unwrap();

    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];

        if let Some(captures) = failed_pattern.captures(line) {
            let test_path = captures.get(2).unwrap().as_str();
            let error_info = captures.get(3).unwrap().as_str();

            // Split error_info into error_type and message
            // Format can be "ErrorType: message" or just "assert False"
            let (error_type, initial_message) = if let Some(colon_pos) = error_info.find(':') {
                let et = error_info[..colon_pos].trim().to_string();
                let msg = error_info[colon_pos + 1..].trim();
                (et, msg)
            } else {
                // No colon, treat whole thing as error type
                (error_info.trim().to_string(), "")
            };

            // Split test path into file and test name
            let parts: Vec<&str> = test_path.split("::").collect();
            let test_file = parts[0].to_string();
            let test_name = parts[1..].join("::");

            // Try to extract more context from following lines
            let mut message = initial_message.to_string();
            let mut line_number = None;
            let mut traceback_lines = Vec::new();

            // Look ahead for error details
            let mut j = i + 1;
            while j < lines.len() && j < i + 30 {
                let next_line = lines[j];

                // Stop at next FAILED or blank line
                if next_line.starts_with("FAILED") || next_line.starts_with("=====") {
                    break;
                }

                // Collect traceback lines starting with E
                if next_line.trim().starts_with("E   ") {
                    let err_line = next_line.trim()[4..].trim();
                    if !message.is_empty() && !err_line.is_empty() {
                        traceback_lines.push(err_line.to_string());
                    }
                }

                // Extract line number from traceback
                if next_line.contains(".py:") {
                    if let Some(line_match) = Regex::new(r":(\d+):")
                        .unwrap()
                        .captures(next_line)
                    {
                        line_number = line_match.get(1).unwrap().as_str().parse().ok();
                    }
                }

                j += 1;
            }

            // Use first traceback line as main message if more detailed
            if !traceback_lines.is_empty() && message.is_empty() {
                message = traceback_lines[0].clone();
            }

            let traceback = if !traceback_lines.is_empty() {
                Some(traceback_lines.join("\n"))
            } else {
                None
            };

            errors.push(PytestError {
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

async fn analyze_command(run_dir: PathBuf) -> Result<()> {
    println!("Analyzing logs in {}...", run_dir.display());

    // Check if findings already exist and are fresh
    let findings_path = run_dir.join("findings.json");
    let metadata_path = run_dir.join("metadata.json");

    if !metadata_path.exists() {
        anyhow::bail!("No metadata.json found in {}. Run download first.", run_dir.display());
    }

    // Read metadata
    let metadata_content = fs::read_to_string(&metadata_path).await?;
    let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;

    // Check if findings are fresh (newer than all log files)
    if findings_path.exists() {
        let findings_mtime = fs::metadata(&findings_path).await?.modified()?;
        let mut all_logs_older = true;

        let mut entries = fs::read_dir(&run_dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            if entry.path().extension().and_then(|s| s.to_str()) == Some("log") {
                if let Ok(log_meta) = entry.metadata().await {
                    if let Ok(log_mtime) = log_meta.modified() {
                        if log_mtime > findings_mtime {
                            all_logs_older = false;
                            break;
                        }
                    }
                }
            }
        }

        if all_logs_older {
            println!("✓ Findings up to date (newer than all log files)");
            return Ok(());
        }
    }

    // Parse all log files in parallel
    println!("Parsing log files...");

    // Collect all log file paths first
    let mut log_files = Vec::new();
    let mut entries = fs::read_dir(&run_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("log") {
            log_files.push(path);
        }
    }

    let jobs_count = log_files.len();

    // Process files in parallel using Rayon
    let all_file_errors: Vec<(String, Vec<PytestError>)> = log_files
        .par_iter()
        .map(|path| {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            // Extract job name from filename
            let job_name = filename
                .strip_suffix("-failure.log")
                .or_else(|| filename.strip_suffix("-success.log"))
                .unwrap_or(&filename)
                .replace('_', " ");

            // Read file synchronously (we're already in a parallel context)
            let content = std::fs::read_to_string(path).expect("Failed to read log file");
            let errors = parse_pytest_errors(&content, &job_name, &filename);

            println!("  {} → {} errors", filename, errors.len());

            (filename, errors)
        })
        .collect();

    // Merge errors by signature (file + test + error_type)
    let mut all_errors: HashMap<String, PytestError> = HashMap::new();
    for (_, errors) in all_file_errors {
        for error in errors {
            let key = format!("{}::{}::{}", error.test_file, error.test_name, error.error_type);

            all_errors
                .entry(key)
                .and_modify(|e| {
                    e.occurrences.extend(error.occurrences.clone());
                })
                .or_insert(error);
        }
    }

    let errors: Vec<PytestError> = all_errors.into_values().collect();
    let total_occurrences: usize = errors.iter().map(|e| e.occurrences.len()).sum();

    let findings = Findings {
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        run_id: metadata.run_id,
        errors: errors.clone(),
        summary: FindingsSummary {
            total_unique_errors: errors.len(),
            total_error_occurrences: total_occurrences,
            jobs_analyzed: jobs_count,
        },
    };

    let findings_json = serde_json::to_string_pretty(&findings)?;
    fs::write(&findings_path, findings_json).await?;

    println!("\n✓ Found {} unique errors ({} total occurrences)",
             errors.len(), total_occurrences);
    println!("✓ Wrote findings to {}", findings_path.display());

    // Auto-generate timeline for the PR
    if let Some(pr_num) = metadata.pr_number {
        let pr_dir = run_dir.parent().unwrap();
        println!("\n→ Generating timeline for PR {}...", pr_num);
        if let Err(e) = timeline_command(pr_dir.to_path_buf()).await {
            eprintln!("Warning: Failed to generate timeline: {}", e);
        }
    }

    Ok(())
}

use std::collections::BTreeMap;

#[derive(Serialize, Deserialize)]
struct Timeline {
    analyzed_at: String,
    pr_number: Option<u64>,
    runs_analyzed: Vec<RunSummary>,
    error_timeline: Vec<ErrorTimeline>,
}

#[derive(Serialize, Deserialize)]
struct RunSummary {
    run_id: String,
    run_number: u64,
    head_sha: String,
    failed_jobs: usize,
    unique_errors: usize,
}

#[derive(Serialize, Deserialize)]
struct ErrorTimeline {
    signature: String,
    test_file: String,
    test_name: String,
    error_type: String,
    message: String,
    first_seen_run: String,
    first_seen_sha: String,
    last_seen_run: String,
    last_seen_sha: String,
    status: String,  // "persistent", "fixed", "regressed"
    occurrences_by_run: BTreeMap<String, usize>,
    likely_culprit_commit: Option<String>,  // SHA where regression was introduced
    likely_fix_commit: Option<String>,      // SHA where error was fixed
}

async fn timeline_command(pr_dir: PathBuf) -> Result<()> {
    println!("Generating timeline for {}...", pr_dir.display());

    // Find all run directories
    let mut run_dirs = Vec::new();
    let mut entries = fs::read_dir(&pr_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.is_dir() && path.join("metadata.json").exists() {
            run_dirs.push(path);
        }
    }

    if run_dirs.is_empty() {
        anyhow::bail!("No run directories found in {}", pr_dir.display());
    }

    // Sort runs by run number
    let mut runs_with_metadata = Vec::new();
    for run_dir in run_dirs {
        let metadata_path = run_dir.join("metadata.json");
        let metadata_content = fs::read_to_string(&metadata_path).await?;
        let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;
        runs_with_metadata.push((run_dir, metadata));
    }
    runs_with_metadata.sort_by_key(|(_, meta)| meta.run_number);

    println!("Found {} runs", runs_with_metadata.len());

    // Load findings for each run
    let mut error_tracker: HashMap<String, (ErrorTimeline, String)> = HashMap::new();  // (timeline, first message)
    let mut run_summaries = Vec::new();

    for (run_dir, metadata) in &runs_with_metadata {
        let findings_path = run_dir.join("findings.json");

        if !findings_path.exists() {
            println!("⚠ No findings.json for run {} - run `analyze` first", metadata.run_id);
            continue;
        }

        let findings_content = fs::read_to_string(&findings_path).await?;
        let findings: Findings = serde_json::from_str(&findings_content)?;

        run_summaries.push(RunSummary {
            run_id: metadata.run_id.clone(),
            run_number: metadata.run_number,
            head_sha: metadata.head_sha.clone(),
            failed_jobs: metadata.failed_jobs,
            unique_errors: findings.summary.total_unique_errors,
        });

        // Track each error
        for error in &findings.errors {
            let signature = format!("{}::{}::{}", error.test_file, error.test_name, error.error_type);

            error_tracker
                .entry(signature.clone())
                .and_modify(|(timeline, _)| {
                    timeline.last_seen_run = metadata.run_id.clone();
                    timeline.last_seen_sha = metadata.head_sha.clone();
                    timeline
                        .occurrences_by_run
                        .insert(metadata.run_id.clone(), error.occurrences.len());
                })
                .or_insert_with(|| {
                    let timeline = ErrorTimeline {
                        signature: signature.clone(),
                        test_file: error.test_file.clone(),
                        test_name: error.test_name.clone(),
                        error_type: error.error_type.clone(),
                        message: error.message.clone(),
                        first_seen_run: metadata.run_id.clone(),
                        first_seen_sha: metadata.head_sha.clone(),
                        last_seen_run: metadata.run_id.clone(),
                        last_seen_sha: metadata.head_sha.clone(),
                        status: String::new(),  // Will be set later
                        occurrences_by_run: {
                            let mut map = BTreeMap::new();
                            map.insert(metadata.run_id.clone(), error.occurrences.len());
                            map
                        },
                        likely_culprit_commit: None,
                        likely_fix_commit: None,
                    };
                    (timeline, error.message.clone())
                });
        }
    }

    // Determine status for each error and identify culprit/fix commits
    for (timeline, _) in error_tracker.values_mut() {
        let total_runs = runs_with_metadata.len();
        let runs_with_error = timeline.occurrences_by_run.len();
        let first_run_id = &runs_with_metadata[0].1.run_id;
        let last_run_id = &runs_with_metadata[total_runs - 1].1.run_id;

        timeline.status = if timeline.first_seen_run != *first_run_id {
            // Regression - set culprit commit
            timeline.likely_culprit_commit = Some(timeline.first_seen_sha.clone());
            "regressed".to_string()
        } else if timeline.last_seen_run != *last_run_id {
            // Fixed - find the commit after last_seen
            let last_seen_idx = runs_with_metadata.iter()
                .position(|(_, m)| m.run_id == timeline.last_seen_run)
                .unwrap();
            if last_seen_idx + 1 < total_runs {
                timeline.likely_fix_commit = Some(runs_with_metadata[last_seen_idx + 1].1.head_sha.clone());
            }
            "fixed".to_string()
        } else if runs_with_error == total_runs {
            // Persistent errors - if only one run, treat it as the culprit
            if total_runs == 1 {
                timeline.likely_culprit_commit = Some(timeline.first_seen_sha.clone());
            }
            "persistent".to_string()
        } else {
            "intermittent".to_string()
        };
    }

    let mut error_timeline: Vec<ErrorTimeline> = error_tracker.into_iter()
        .map(|(_, (timeline, _))| timeline)
        .collect();
    error_timeline.sort_by(|a, b| {
        // Sort by status (regressed, persistent, intermittent, fixed) then by signature
        let status_order = |s: &str| match s {
            "regressed" => 0,
            "persistent" => 1,
            "intermittent" => 2,
            "fixed" => 3,
            _ => 4,
        };
        status_order(&a.status).cmp(&status_order(&b.status))
            .then_with(|| a.signature.cmp(&b.signature))
    });

    let pr_number = runs_with_metadata.first().and_then(|(_, m)| m.pr_number);

    let timeline = Timeline {
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        pr_number,
        runs_analyzed: run_summaries,
        error_timeline,
    };

    let timeline_path = pr_dir.join("analysis.json");
    let timeline_json = serde_json::to_string_pretty(&timeline)?;
    fs::write(&timeline_path, timeline_json).await?;

    // Print summary
    println!("\n✓ Timeline analysis:");
    let regressed = timeline.error_timeline.iter().filter(|e| e.status == "regressed").count();
    let persistent = timeline.error_timeline.iter().filter(|e| e.status == "persistent").count();
    let fixed = timeline.error_timeline.iter().filter(|e| e.status == "fixed").count();
    let intermittent = timeline.error_timeline.iter().filter(|e| e.status == "intermittent").count();

    println!("  Regressed:    {} errors", regressed);
    println!("  Persistent:   {} errors", persistent);
    println!("  Intermittent: {} errors", intermittent);
    println!("  Fixed:        {} errors", fixed);
    println!("\n✓ Wrote timeline to {}", timeline_path.display());

    Ok(())
}

#[derive(Serialize)]
struct JobTiming {
    started_at: Option<String>,
    completed_at: Option<String>,
    duration_secs: Option<i64>,
    conclusion: Option<String>,
}

#[derive(Serialize)]
struct JobTimingAnalysis {
    job_name: String,
    runs: BTreeMap<String, JobTiming>,
    avg_duration_secs: Option<f64>,
    min_duration_secs: Option<i64>,
    max_duration_secs: Option<i64>,
}

#[derive(Serialize)]
struct TimingsReport {
    analyzed_at: String,
    pr_number: Option<u64>,
    runs_analyzed: Vec<String>,
    jobs: Vec<JobTimingAnalysis>,
}

async fn timings_command(pr_dir: PathBuf) -> Result<()> {
    println!("Generating timings for {}...", pr_dir.display());

    // Find all run directories with metadata.json
    let mut run_dirs = Vec::new();
    let mut entries = fs::read_dir(&pr_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let metadata_path = entry.path().join("metadata.json");
        if metadata_path.exists() {
            run_dirs.push(entry.path());
        }
    }

    if run_dirs.is_empty() {
        anyhow::bail!("No run directories found in {}", pr_dir.display());
    }

    // Sort by run number
    let mut metadata_list: Vec<(PathBuf, RunMetadata)> = Vec::new();
    for dir in run_dirs {
        let metadata_content = fs::read_to_string(dir.join("metadata.json")).await?;
        let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;
        metadata_list.push((dir, metadata));
    }
    metadata_list.sort_by_key(|(_, m)| m.run_number);

    println!("Found {} runs\n", metadata_list.len());

    // Build job timing map
    let mut job_timings: HashMap<String, BTreeMap<String, JobTiming>> = HashMap::new();

    for (_, metadata) in &metadata_list {
        for job in &metadata.jobs {
            let duration = if let (Some(start), Some(end)) = (&job.started_at, &job.completed_at) {
                // Parse timestamps and calculate duration
                let start_time = chrono::DateTime::parse_from_rfc3339(start).ok();
                let end_time = chrono::DateTime::parse_from_rfc3339(end).ok();

                if let (Some(s), Some(e)) = (start_time, end_time) {
                    Some((e.timestamp() - s.timestamp()).abs())
                } else {
                    None
                }
            } else {
                None
            };

            let timing = JobTiming {
                started_at: job.started_at.clone(),
                completed_at: job.completed_at.clone(),
                duration_secs: duration,
                conclusion: job.conclusion.clone(),
            };

            job_timings
                .entry(job.name.clone())
                .or_default()
                .insert(metadata.run_id.clone(), timing);
        }
    }

    // Calculate statistics and build analysis
    let mut jobs_analysis: Vec<JobTimingAnalysis> = job_timings
        .into_iter()
        .map(|(job_name, runs)| {
            let durations: Vec<i64> = runs.values()
                .filter_map(|t| t.duration_secs)
                .collect();

            let avg = if !durations.is_empty() {
                Some(durations.iter().sum::<i64>() as f64 / durations.len() as f64)
            } else {
                None
            };

            let min = durations.iter().min().copied();
            let max = durations.iter().max().copied();

            JobTimingAnalysis {
                job_name,
                runs,
                avg_duration_secs: avg,
                min_duration_secs: min,
                max_duration_secs: max,
            }
        })
        .collect();

    // Sort by average duration (slowest first)
    jobs_analysis.sort_by(|a, b| {
        b.avg_duration_secs
            .partial_cmp(&a.avg_duration_secs)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let pr_number = metadata_list.first().and_then(|(_, m)| m.pr_number);
    let run_ids: Vec<String> = metadata_list.iter().map(|(_, m)| m.run_id.clone()).collect();

    let report = TimingsReport {
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        pr_number,
        runs_analyzed: run_ids,
        jobs: jobs_analysis,
    };

    // Write report
    let timings_path = pr_dir.join("timings.json");
    let report_json = serde_json::to_string_pretty(&report)?;
    fs::write(&timings_path, report_json).await?;

    println!("✓ Wrote timings to {}", timings_path.display());
    println!("\nTop 5 slowest jobs (by average):");
    for (i, job) in report.jobs.iter().take(5).enumerate() {
        if let Some(avg) = job.avg_duration_secs {
            println!("  {}. {} - avg: {:.0}s (min: {}s, max: {}s)",
                i + 1,
                job.job_name,
                avg,
                job.min_duration_secs.unwrap_or(0),
                job.max_duration_secs.unwrap_or(0)
            );
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Download {
            run_url,
            output,
            token,
            all,
        } => download_command(run_url, output, token, all).await,
        Commands::Analyze { run_dir } => analyze_command(run_dir).await,
        Commands::Timeline { pr_dir } => timeline_command(pr_dir).await,
        Commands::Timings { pr_dir } => timings_command(pr_dir).await,
    }
}
