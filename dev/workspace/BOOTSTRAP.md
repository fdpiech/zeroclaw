# Bootstrap

Run these steps once on first startup to initialize the runtime state.
After each step succeeds, skip it on subsequent runs.

## 1. Inbox worker schedule

Set up the recurring inbox processing job (every 5 minutes):

```
cron_add(
  schedule = {kind: "every", every_ms: 300000},
  job_type  = "agent",
  prompt    = "Process all pending inbox files."
)
```

Verify it was created:

```
cron_list()
```

Expected: one job with prompt "Process all pending inbox files."

## 2. Inbox directory structure

The inbox_worker expects this layout under `workspace/inbox/`:

```
inbox/
  <source_type>/       ← drop files here (e.g. emails/, transcripts/)
  processing/          ← auto-managed; do not write here directly
  processed/           ← auto-managed
  failed/              ← auto-managed
```

Create source-type directories for the file types you will ingest.
The inbox_worker discovers them automatically at runtime.
