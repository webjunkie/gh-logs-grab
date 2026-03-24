use anyhow::Result;
use futures::future::join_all;
use std::path::PathBuf;
use tokio::fs;

use crate::github;
use crate::models::RunMetadata;
use super::analyze_command;

pub async fn download_command(
    run_url: String,
    output: PathBuf,
    token: Option<String>,
    all: bool,
) -> Result<()> {
    let token = match token {
        Some(t) => t,
        None => github::get_github_token().await?,
    };

    let (owner, repo, run_id) = github::parse_run_url(&run_url)?;

    let headers = github::build_headers(&token);
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()?;

    let run_info = github::fetch_run_info(&client, &owner, &repo, &run_id).await?;

    let pr_number = run_info.pull_requests.first().map(|pr| pr.number);
    let base_dir = if let Some(pr_num) = pr_number {
        output.join(format!("pr-{}", pr_num))
    } else {
        let safe_branch = run_info.head_branch.replace('/', "_");
        output.join(safe_branch)
    };

    let run_output_dir = base_dir.join(&run_id);
    fs::create_dir_all(&run_output_dir).await?;

    let jobs = github::fetch_jobs(&client, &owner, &repo, &run_id).await?;
    let total_jobs = jobs.len();

    let mut jobs_to_download = jobs.clone();
    if !all {
        jobs_to_download.retain(|job| {
            job.conclusion.as_deref() != Some("success") && job.conclusion.is_some()
        });
    }

    let failed_jobs_count = jobs_to_download.len();

    if !jobs_to_download.is_empty() {
        let download_tasks: Vec<_> = jobs_to_download
            .iter()
            .map(|job| github::download_job_logs(&client, &owner, &repo, job, &run_output_dir))
            .collect();

        let results = join_all(download_tasks).await;
        let failed = results.iter().filter(|r| r.is_err()).count();

        println!(
            "Downloaded {} logs for run {} ({}/{} repo){}\n",
            jobs_to_download.len() - failed,
            run_id,
            owner,
            repo,
            if failed > 0 {
                format!(" ({} failed)", failed)
            } else {
                String::new()
            }
        );
    } else {
        println!("No failed jobs for run {} ({}/{} repo)\n", run_id, owner, repo);
    }

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

    if let Err(e) = analyze_command(run_output_dir).await {
        eprintln!("Warning: Failed to analyze logs: {}", e);
    }

    Ok(())
}
