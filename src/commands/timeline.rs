use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tokio::fs;

use crate::models::{Findings, RunMetadata};

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
    status: String,
    occurrences_by_run: BTreeMap<String, usize>,
    likely_culprit_commit: Option<String>,
    likely_fix_commit: Option<String>,
}

pub async fn timeline_command(pr_dir: PathBuf) -> Result<()> {
    println!("Generating timeline for {}...", pr_dir.display());

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

    let mut runs_with_metadata = Vec::new();
    for run_dir in run_dirs {
        let metadata_path = run_dir.join("metadata.json");
        let metadata_content = fs::read_to_string(&metadata_path).await?;
        let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;
        runs_with_metadata.push((run_dir, metadata));
    }
    runs_with_metadata.sort_by_key(|(_, meta)| meta.run_number);

    println!("Found {} runs", runs_with_metadata.len());

    let mut error_tracker: HashMap<String, (ErrorTimeline, String)> = HashMap::new();
    let mut run_summaries = Vec::new();

    for (run_dir, metadata) in &runs_with_metadata {
        let findings_path = run_dir.join("findings.json");

        if !findings_path.exists() {
            println!(
                "⚠ No findings.json for run {} - run `analyze` first",
                metadata.run_id
            );
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

        for error in &findings.errors {
            let signature = format!(
                "{}::{}::{}::{}",
                error.framework, error.test_file, error.test_name, error.error_type
            );

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
                        status: String::new(),
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

    // Determine status for each error
    for (timeline, _) in error_tracker.values_mut() {
        let total_runs = runs_with_metadata.len();
        let runs_with_error = timeline.occurrences_by_run.len();
        let first_run_id = &runs_with_metadata[0].1.run_id;
        let last_run_id = &runs_with_metadata[total_runs - 1].1.run_id;

        timeline.status = if timeline.first_seen_run != *first_run_id {
            timeline.likely_culprit_commit = Some(timeline.first_seen_sha.clone());
            "regressed".to_string()
        } else if timeline.last_seen_run != *last_run_id {
            let last_seen_idx = runs_with_metadata
                .iter()
                .position(|(_, m)| m.run_id == timeline.last_seen_run)
                .unwrap();
            if last_seen_idx + 1 < total_runs {
                timeline.likely_fix_commit =
                    Some(runs_with_metadata[last_seen_idx + 1].1.head_sha.clone());
            }
            "fixed".to_string()
        } else if runs_with_error == total_runs {
            if total_runs == 1 {
                timeline.likely_culprit_commit = Some(timeline.first_seen_sha.clone());
            }
            "persistent".to_string()
        } else {
            "intermittent".to_string()
        };
    }

    let mut error_timeline: Vec<ErrorTimeline> = error_tracker
        .into_iter()
        .map(|(_, (timeline, _))| timeline)
        .collect();
    error_timeline.sort_by(|a, b| {
        let status_order = |s: &str| match s {
            "regressed" => 0,
            "persistent" => 1,
            "intermittent" => 2,
            "fixed" => 3,
            _ => 4,
        };
        status_order(&a.status)
            .cmp(&status_order(&b.status))
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

    println!("\n✓ Timeline analysis:");
    let regressed = timeline
        .error_timeline
        .iter()
        .filter(|e| e.status == "regressed")
        .count();
    let persistent = timeline
        .error_timeline
        .iter()
        .filter(|e| e.status == "persistent")
        .count();
    let fixed = timeline
        .error_timeline
        .iter()
        .filter(|e| e.status == "fixed")
        .count();
    let intermittent = timeline
        .error_timeline
        .iter()
        .filter(|e| e.status == "intermittent")
        .count();

    println!("  Regressed:    {} errors", regressed);
    println!("  Persistent:   {} errors", persistent);
    println!("  Intermittent: {} errors", intermittent);
    println!("  Fixed:        {} errors", fixed);
    println!("\n✓ Wrote timeline to {}", timeline_path.display());

    Ok(())
}
