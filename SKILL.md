---
name: gh-logs
description: Download and analyze GitHub Actions CI logs. Use when user shares a run URL or asks about CI failures.
---

# GitHub Actions Log Grabber

Use the `gh-logs-grab` CLI tool to download CI logs from GitHub Actions and analyze pytest failures.

## When to Use

- User shares a GitHub Actions run URL (e.g., `https://github.com/owner/repo/actions/runs/123456`)
- User asks about CI failures, test failures, or flaky tests
- User wants to compare errors across multiple CI runs
- User asks about job timing/performance in CI

## Commands

### Download Logs

```bash
gh-logs-grab download <RUN_URL> [OPTIONS]
```

Downloads logs from a GitHub Actions run. By default, only failed jobs are downloaded.

**Options:**
- `-o, --output <PATH>`: Output directory (default: `./logs`)
- `-a, --all`: Include successful jobs too
- `-t, --token <TOKEN>`: GitHub token (auto-detects from `GITHUB_TOKEN` env or `gh auth token`)

**Output structure:**
```
logs/pr-{number}/{run-id}/
├── metadata.json
├── findings.json          # Auto-generated pytest analysis
├── job-name-failure.log
└── ...
```

### Analyze Single Run

```bash
gh-logs-grab analyze <RUN_DIR>
```

Extracts pytest errors from logs. Creates `findings.json` with deduplicated errors.

Note: `download` runs this automatically.

### Timeline Analysis

```bash
gh-logs-grab timeline <PR_DIR>
```

Compares errors across multiple runs in a PR directory. Creates `analysis.json` with:
- **Regressed**: New errors introduced
- **Persistent**: Present in all runs
- **Intermittent**: Flaky tests
- **Fixed**: Errors that disappeared

### Job Timings

```bash
gh-logs-grab timings <PR_DIR>
```

Analyzes job duration across runs. Creates `timings.json` and shows top 5 slowest jobs.

## Typical Workflow

1. User shares run URL → `gh-logs-grab download <url>`
2. Review `findings.json` for error summary
3. If multiple runs exist → `gh-logs-grab timeline logs/pr-XXX` to identify regressions vs flaky tests
4. For performance issues → `gh-logs-grab timings logs/pr-XXX`

## Notes

- Logs are stored in `./logs/` relative to current directory
- Tool is idempotent - safe to re-run without re-downloading
- GitHub token is auto-detected from environment or `gh` CLI
