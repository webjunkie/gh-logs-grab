# gh-logs-grab

Blazingly fast GitHub Actions log downloader and pytest error analyzer written in Rust.

## Features

- **Download**: Fetch all CI logs in parallel from GitHub Actions runs
- **Analyze**: Extract pytest errors with file, line, and error details
- **Timeline**: Track error regressions and fixes across multiple runs
- **Idempotent**: Safe to re-run, skips existing files
- **Organized**: Creates `{pr-number}/{run-id}/` folder structure

## Commands

### Download logs

```bash
# Download failed job logs (default)
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456

# Download all logs (including successful jobs)
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456 --all

# Custom output directory
gh-logs-grab download https://github.com/owner/repo/actions/runs/123456 -o my-logs
```

Creates:
- `logs/pr-{number}/{run-id}/metadata.json` - Run metadata with commit SHA, timestamps, job counts
- `logs/pr-{number}/{run-id}/*.log` - Individual job logs

### Analyze pytest errors

```bash
# Extract pytest errors from downloaded logs
gh-logs-grab analyze logs/pr-123/19344810699
```

Creates:
- `findings.json` - Unique pytest errors with occurrences across jobs

### Generate timeline

```bash
# Track errors across multiple runs
gh-logs-grab timeline logs/pr-123
```

Creates:
- `analysis.json` - Error timeline showing regressions, fixes, and persistent errors

## Full workflow example

```bash
# Download logs from multiple runs
gh-logs-grab download https://github.com/PostHog/posthog/actions/runs/19374816456
gh-logs-grab download https://github.com/PostHog/posthog/actions/runs/19378157593

# Analyze each run
gh-logs-grab analyze logs/pr-41513/19374816456
gh-logs-grab analyze logs/pr-41513/19378157593

# Generate timeline
gh-logs-grab timeline logs/pr-41513
```

Output:
```
✓ Timeline analysis:
  Regressed:    12 errors  ← New errors introduced
  Persistent:   8 errors   ← Present in all runs
  Intermittent: 5 errors   ← Flaky tests
  Fixed:        45 errors  ← Errors that disappeared
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

Claude will then pick up the `SKILL.md` and use `gh-logs-grab` when you ask about CI failures. You can also invoke it explicitly with `/gh-logs`.

## Token authentication

Auto-detects token from:
1. `--token` flag
2. `GITHUB_TOKEN` environment variable
3. `gh auth token` (GitHub CLI)
