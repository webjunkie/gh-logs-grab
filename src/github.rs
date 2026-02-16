use anyhow::{Context, Result};
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION, USER_AGENT};
use serde::Deserialize;
use std::path::PathBuf;
use tokio::fs;

use crate::models::Job;

#[derive(Deserialize, Debug)]
pub struct JobsResponse {
    pub jobs: Vec<Job>,
}

#[derive(Deserialize, Debug)]
pub struct WorkflowRun {
    pub head_branch: String,
    #[allow(dead_code)]
    pub event: String,
    pub head_sha: String,
    pub run_number: u64,
    pub created_at: String,
    pub updated_at: String,
    pub html_url: String,
    #[serde(default)]
    pub pull_requests: Vec<PullRequest>,
}

#[derive(Deserialize, Debug)]
pub struct PullRequest {
    pub number: u64,
}

pub async fn get_github_token() -> Result<String> {
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

pub fn parse_run_url(url: &str) -> Result<(String, String, String)> {
    let parts: Vec<&str> = url.split('/').collect();
    if parts.len() < 7 || parts[5] != "actions" || parts[6] != "runs" {
        anyhow::bail!("Invalid GitHub Actions run URL format");
    }

    let owner = parts[3].to_string();
    let repo = parts[4].to_string();
    let run_id = parts[7].split('?').next().unwrap().to_string();

    Ok((owner, repo, run_id))
}

pub fn build_headers(token: &str) -> HeaderMap {
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
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );
    headers
}

pub async fn fetch_run_info(
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

pub async fn fetch_jobs(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    run_id: &str,
) -> Result<Vec<Job>> {
    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs/{}/jobs?per_page=100",
        owner, repo, run_id
    );

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

    Ok(all_jobs)
}

pub async fn download_job_logs(
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

    if filepath.exists() {
        return Ok(());
    }

    let url = format!(
        "https://api.github.com/repos/{}/{}/actions/jobs/{}/logs",
        owner, repo, job.id
    );

    let resp = client.get(&url).send().await?;

    if !resp.status().is_success() {
        return Ok(());
    }

    let content = resp.bytes().await?;

    if let Some(parent) = filepath.parent() {
        fs::create_dir_all(parent).await?;
    }

    fs::write(&filepath, content).await?;

    Ok(())
}
