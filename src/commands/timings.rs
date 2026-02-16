use anyhow::Result;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tokio::fs;

use crate::models::RunMetadata;

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

pub async fn timings_command(pr_dir: PathBuf) -> Result<()> {
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

    let mut metadata_list: Vec<(PathBuf, RunMetadata)> = Vec::new();
    for dir in run_dirs {
        let metadata_content = fs::read_to_string(dir.join("metadata.json")).await?;
        let metadata: RunMetadata = serde_json::from_str(&metadata_content)?;
        metadata_list.push((dir, metadata));
    }
    metadata_list.sort_by_key(|(_, m)| m.run_number);

    let mut job_timings: HashMap<String, BTreeMap<String, JobTiming>> = HashMap::new();

    for (_, metadata) in &metadata_list {
        for job in &metadata.jobs {
            let duration = if let (Some(start), Some(end)) = (&job.started_at, &job.completed_at) {
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

    let mut jobs_analysis: Vec<JobTimingAnalysis> = job_timings
        .into_iter()
        .map(|(job_name, runs)| {
            let durations: Vec<i64> = runs.values().filter_map(|t| t.duration_secs).collect();

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

    let timings_path = pr_dir.join("timings.json");
    let report_json = serde_json::to_string_pretty(&report)?;
    fs::write(&timings_path, report_json).await?;

    println!("Top 5 slowest jobs (by avg, {} runs):", metadata_list.len());
    for (i, job) in report.jobs.iter().take(5).enumerate() {
        if let Some(avg) = job.avg_duration_secs {
            println!(
                "  {}. {} — avg {:.0}s (min {}s, max {}s)",
                i + 1,
                job.job_name,
                avg,
                job.min_duration_secs.unwrap_or(0),
                job.max_duration_secs.unwrap_or(0)
            );
        }
    }
    println!("Timings: {}", timings_path.display());

    Ok(())
}
