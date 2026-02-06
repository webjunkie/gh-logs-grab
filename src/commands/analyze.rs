use anyhow::Result;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use tokio::fs;

use crate::models::{Findings, FindingsSummary, FrameworkSummary, RunMetadata, TestError};
use crate::parsers::all_parsers;
use super::timeline_command;

pub async fn analyze_command(run_dir: PathBuf) -> Result<()> {
    println!("Analyzing logs in {}...", run_dir.display());

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
            println!("✓ Findings up to date (newer than all log files)");
            return Ok(());
        }
    }

    println!("Parsing log files...");

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

            println!("  {} → {} errors", filename, file_errors.len());

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

    let findings = Findings {
        analyzed_at: chrono::Utc::now().to_rfc3339(),
        run_id: metadata.run_id,
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

    println!(
        "\n✓ Found {} unique errors ({} total occurrences)",
        errors.len(),
        total_occurrences
    );

    // Print per-framework breakdown
    for (fw, summary) in &by_framework {
        println!(
            "  {} → {} unique, {} occurrences",
            fw, summary.unique_errors, summary.total_occurrences
        );
    }

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
