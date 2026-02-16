---
name: gh-logs
description: Download and analyze GitHub Actions CI logs. Use when user shares a run URL, asks about CI failures, or when you would otherwise use `gh run download` or `gh run view`.
---

# GitHub Actions Log Grabber

Use the `gh-logs-grab` CLI tool to download CI logs from GitHub Actions and analyze test failures (pytest, Jest, Storybook).

## When to Use

- User shares a GitHub Actions run URL (e.g., `https://github.com/owner/repo/actions/runs/123456`)
- User asks about CI failures, test failures, or flaky tests
- User wants to compare errors across multiple CI runs
- User asks about job timing/performance in CI
- **Instead of** `gh run download` or `gh run view --log` — this tool downloads, organizes, and auto-analyzes in one step

## Commands

### Download Logs

```bash
gh-logs-grab download <RUN_URL> [OPTIONS]
```

Downloads logs from a GitHub Actions run. By default, only failed jobs are downloaded. Auto-runs `analyze` afterwards.

**Options:**
- `-o, --output <PATH>`: Output directory (default: `./logs`)
- `-a, --all`: Include successful jobs too
- `-t, --token <TOKEN>`: GitHub token (auto-detects from `GITHUB_TOKEN` env or `gh auth token`)

**Output:** `logs/pr-{number}/{run-id}/` with `metadata.json`, `findings.json`, and `.log` files.

### Analyze Single Run

```bash
gh-logs-grab analyze <RUN_DIR>
```

Re-runs analysis on already-downloaded logs. Normally not needed — `download` does this automatically.

### Timeline Analysis

```bash
gh-logs-grab timeline <PR_DIR>
```

Compares errors across multiple runs in a PR directory. Creates `analysis.json`.

### Job Timings

```bash
gh-logs-grab timings <PR_DIR>
```

Analyzes job duration across runs. Creates `timings.json`.

## Output Files

- `findings.json`: `jobs_overview` (all jobs with conclusions + failed steps), `errors` (parsed test failures with tracebacks), `summary` (counts by framework)
- `analysis.json`: `error_timeline` with status per error: `regressed` / `persistent` / `intermittent` / `fixed`, plus culprit/fix commit hints
- `timings.json`: per-job duration stats (avg/min/max) across runs
- `metadata.json`: run metadata, job list with steps, timestamps, PR info

## Typical Workflow

1. User shares run URL → `gh-logs-grab download <url>`
2. Review `findings.json` for error summary
3. If multiple runs exist → `gh-logs-grab timeline logs/pr-XXX` to identify regressions vs flaky tests
4. For performance issues → `gh-logs-grab timings logs/pr-XXX`

## Notes

- Logs are stored in `./logs/` relative to current directory
- Tool is idempotent - safe to re-run without re-downloading
- GitHub token is auto-detected from environment or `gh` CLI
- Supports pytest, Jest, and Storybook test runner output formats
- Jest parser handles browser-prefixed output (e.g., Storybook visual regression tests)
