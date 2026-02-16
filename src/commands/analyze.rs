use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

use crate::models::{
    FailedStepOverview, Findings, FindingsSummary, FrameworkSummary, JobOverview, RunMetadata,
    TestError,
};
use crate::parsers::all_parsers;
use super::timeline_command;

fn compute_duration(started_at: &Option<String>, completed_at: &Option<String>) -> Option<i64> {
    let start = chrono::DateTime::parse_from_rfc3339(started_at.as_deref()?).ok()?;
    let end = chrono::DateTime::parse_from_rfc3339(completed_at.as_deref()?).ok()?;
    Some((end.timestamp() - start.timestamp()).abs())
}

fn conclusion_sort_key(conclusion: &str) -> u8 {
    match conclusion {
        "failure" => 0,
        "cancelled" => 1,
        "timed_out" => 2,
        "success" => 3,
        "skipped" => 4,
        _ => 5,
    }
}

fn build_jobs_overview(metadata: &RunMetadata) -> Vec<JobOverview> {
    let mut overview: Vec<JobOverview> = metadata
        .jobs
        .iter()
        .map(|job| {
            let conclusion = job.conclusion.as_deref().unwrap_or("unknown").to_string();
            let duration_secs = compute_duration(&job.started_at, &job.completed_at);

            let failed_steps = if conclusion != "success" && conclusion != "skipped" {
                job.steps
                    .iter()
                    .filter(|s| {
                        let sc = s.conclusion.as_deref().unwrap_or("unknown");
                        sc != "success" && sc != "skipped"
                    })
                    .map(|s| FailedStepOverview {
                        name: s.name.clone(),
                        conclusion: s.conclusion.as_deref().unwrap_or("unknown").to_string(),
                        number: s.number,
                        duration_secs: compute_duration(&s.started_at, &s.completed_at),
                    })
                    .collect()
            } else {
                Vec::new()
            };

            JobOverview {
                job_name: job.name.clone(),
                conclusion,
                duration_secs,
                failed_steps,
            }
        })
        .collect();

    overview.sort_by(|a, b| {
        conclusion_sort_key(&a.conclusion)
            .cmp(&conclusion_sort_key(&b.conclusion))
            .then_with(|| a.job_name.cmp(&b.job_name))
    });

    overview
}

fn print_jobs_overview(overview: &[JobOverview], total_jobs: usize) {
    let failed = overview
        .iter()
        .filter(|j| j.conclusion == "failure")
        .count();
    let cancelled = overview
        .iter()
        .filter(|j| j.conclusion == "cancelled")
        .count();
    let passed = overview
        .iter()
        .filter(|j| j.conclusion == "success")
        .count();

    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(format!("{} failed", failed));
    }
    if cancelled > 0 {
        parts.push(format!("{} cancelled", cancelled));
    }
    if passed > 0 {
        parts.push(format!("{} passed", passed));
    }
    let other = total_jobs - failed - cancelled - passed;
    if other > 0 {
        parts.push(format!("{} other", other));
    }

    println!("\nJobs overview ({} total, {}):", total_jobs, parts.join(", "));

    for job in overview {
        if job.conclusion == "success" || job.conclusion == "skipped" {
            continue;
        }
        let duration = job
            .duration_secs
            .map(|d| format!(", {}s", d))
            .unwrap_or_default();
        println!("  ✗ {} [{}{}]", job.job_name, job.conclusion, duration);
        for step in &job.failed_steps {
            let sdur = step
                .duration_secs
                .map(|d| format!(", {}s", d))
                .unwrap_or_default();
            println!(
                "    → Step {}: {} [{}{}]",
                step.number, step.name, step.conclusion, sdur
            );
        }
    }

    if passed > 0 {
        println!("  ✓ {} jobs passed", passed);
    }
}

pub async fn analyze_command(run_dir: PathBuf) -> Result<()> {
    let findings_path = run_dir.join("findings.json");
    let metadata_path = run_dir.join("metadata.json");

    if !metadata_path.exists() {
        anyhow::bail!(
            "No metadata.json found in {}. Run download first.",
            run_dir.display()
        );
    }

    let metadata_content = fs::read_to_string(&metadata_path).await?;
    let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;

    // Check if findings are fresh
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
            return Ok(());
        }
    }

    let mut log_files = Vec::new();
    let mut entries = fs::read_dir(&run_dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("log") {
            log_files.push(path);
        }
    }

    let jobs_count = log_files.len();
    let parsers = all_parsers();

    // Process files in parallel using Rayon
    let all_file_errors: Vec<(String, Vec<TestError>)> = log_files
        .par_iter()
        .map(|path| {
            let filename = path.file_name().unwrap().to_string_lossy().to_string();

            let job_name = filename
                .strip_suffix("-failure.log")
                .or_else(|| filename.strip_suffix("-success.log"))
                .or_else(|| filename.strip_suffix("-cancelled.log"))
                .unwrap_or(&filename)
                .replace('_', " ");

            let content = std::fs::read_to_string(path).expect("Failed to read log file");

            // Run all parsers, collect results
            let mut file_errors = Vec::new();
            for parser in &parsers {
                file_errors.extend(parser.parse(&content, &job_name, &filename));
            }

            (filename, file_errors)
        })
        .collect();

    // Merge errors by signature (framework + file + test + error_type)
    let mut all_errors: HashMap<String, TestError> = HashMap::new();
    for (_, errors) in all_file_errors {
        for error in errors {
            let key = format!(
                "{}::{}::{}::{}",
                error.framework, error.test_file, error.test_name, error.error_type
            );

            all_errors
                .entry(key)
                .and_modify(|e| {
                    e.occurrences.extend(error.occurrences.clone());
                })
                .or_insert(error);
        }
    }

    let errors: Vec<TestError> = all_errors.into_values().collect();
    let total_occurrences: usize = errors.iter().map(|e| e.occurrences.len()).sum();

    // Build per-framework summary
    let mut by_framework: HashMap<String, FrameworkSummary> = HashMap::new();
    for error in &errors {
        let entry = by_framework
            .entry(error.framework.clone())
            .or_insert(FrameworkSummary {
                unique_errors: 0,
                total_occurrences: 0,
            });
        entry.unique_errors += 1;
        entry.total_occurrences += error.occurrences.len();
    }

    // Build jobs/steps overview from metadata
    let jobs_overview = build_jobs_overview(&metadata);
    let total_jobs = metadata.total_jobs;
    let pr_number = metadata.pr_number;

    let findings = Findings {
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        run_id: metadata.run_id,
        jobs_overview: jobs_overview.clone(),
        errors: errors.clone(),
        summary: FindingsSummary {
            total_unique_errors: errors.len(),
            total_error_occurrences: total_occurrences,
            jobs_analyzed: jobs_count,
            by_framework: by_framework.clone(),
        },
    };

    let findings_json = serde_json::to_string_pretty(&findings)?;
    fs::write(&findings_path, findings_json).await?;

    // Print jobs overview
    print_jobs_overview(&jobs_overview, total_jobs);

    // Print per-framework error summary
    if errors.is_empty() {
        println!("\nNo parsed test errors found.");
    } else {
        let fw_parts: Vec<String> = by_framework
            .iter()
            .map(|(fw, s)| format!("{}: {} unique, {} total", fw, s.unique_errors, s.total_occurrences))
            .collect();
        println!(
            "\nTest errors: {} unique ({} occurrences) [{}]",
            errors.len(),
            total_occurrences,
            fw_parts.join("; ")
        );
    }

    println!("Findings: {}", findings_path.display());

    // Auto-generate timeline for the PR
    if pr_number.is_some() {
        let pr_dir = run_dir.parent().unwrap();
        println!();
        if let Err(e) = timeline_command(pr_dir.to_path_buf()).await {
            eprintln!("Warning: Failed to generate timeline: {}", e);
        }
    }

    Ok(())
}
