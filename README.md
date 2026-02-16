# gh-logs-grab

Fast GitHub Actions log downloader and test error analyzer. Supports pytest, Jest, and Storybook output formats.

## Features

- **Download**: Fetch failed job logs in parallel from GitHub Actions runs
- **Analyze**: Extract test errors (pytest, Jest/Storybook) with file, line, and error details
- **Jobs overview**: See which jobs and steps failed at a glance
- **Timeline**: Track error regressions and fixes across multiple runs
- **Timings**: Identify slowest jobs across runs
- **Idempotent**: Safe to re-run, skips existing files

## Commands

### Download logs

```bash
# Download failed job logs (default) — auto-analyzes afterwards
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456

# Download all logs (including successful jobs)
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456 --all

# Custom output directory
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456 -o my-logs
```

Output:
```
Downloaded 10 logs for run 22057860677 (PostHog/posthog repo)

Jobs overview (65 total, 8 failed, 55 passed, 2 other):
  ✗ Django tests – Core (22/38) [failure, 521s]
    → Step 22: Run Core tests [failure, 429s]
  ✗ Discover product tests [failure, 45s]
    → Step 5: Install pnpm dependencies [failure, 2s]
  ✓ 55 jobs passed

Test errors: 27 unique (28 occurrences) [pytest: 27 unique, 28 total]
Findings: logs/pr-47485/22057860677/findings.json
```

Creates `logs/pr-{number}/{run-id}/` with:
- `metadata.json` — run metadata, job list with steps
- `findings.json` — jobs overview, parsed test errors, per-framework summary
- `*.log` — individual job logs

### Analyze

```bash
gh-logs-grab analyze logs/pr-123/19344810699
```

Re-runs analysis on already-downloaded logs. Normally not needed — `download` does this automatically.

### Timeline

```bash
gh-logs-grab timeline logs/pr-123
```

Compares errors across multiple runs. Creates `analysis.json` classifying each error as regressed, persistent, intermittent, or fixed.

### Timings

```bash
gh-logs-grab timings logs/pr-123
```

Analyzes job duration across runs. Creates `timings.json` with avg/min/max per job.

## Workflow

```bash
# Download multiple runs (analyze + timeline run automatically)
gh-logs-grab download https://github.com/PostHog/posthog/actions/runs/19374816456
gh-logs-grab download https://github.com/PostHog/posthog/actions/runs/19378157593

# Check timings separately if needed
gh-logs-grab timings logs/pr-41513
```

## Installation

```bash
cargo install --path .
```

### Claude Code skill

This repo includes a [Claude Code](https://docs.anthropic.com/en/docs/claude-code) skill that lets Claude automatically download and analyze CI logs when you share a GitHub Actions URL.

To install, symlink this repo into your Claude skills directory:

```bash
ln -s /path/to/gh-logs-grab ~/.claude/skills/gh-logs
```

Claude will then use `gh-logs-grab` when you ask about CI failures. You can also invoke it explicitly with `/gh-logs`.

## Token authentication

Auto-detects token from:
1. `--token` flag
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` (GitHub CLI)
