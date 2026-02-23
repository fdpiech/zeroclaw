# Bootstrap

One-time operator setup. These steps are not executed automatically —
ask your agent to perform them interactively before first use.

The agent does not self-bootstrap on startup. Run each step once,
verify it succeeded, and you are done.

## 1. Inbox worker schedule

Set up the recurring inbox processing job (every 5 minutes).
Ask your agent:

> "Create a cron job named 'inbox-worker' that runs every 5 minutes as an
> agent job with prompt: Process all pending inbox files."

Or invoke directly:

```json
{
  "name": "inbox-worker",
  "schedule": {"kind": "every", "every_ms": 300000},
  "job_type": "agent",
  "prompt": "Process all pending inbox files."
}
```

Verify it was created by asking: "List cron jobs."
Expected: one job named "inbox-worker" with a next_run timestamp.

## 2. Inbox directory structure

The inbox_worker expects this layout under `workspace/inbox/`:

```
inbox/
  <source_type>/       ← drop files here (e.g. emails/, transcripts/)
  processing/          ← auto-managed; do not write here directly
  processed/           ← auto-managed
  failed/              ← auto-managed
```

Create the source-type directories for the file types you will ingest.
The inbox_worker discovers them automatically at runtime.

## Known limitation

Work teams (delegates) always run with native tool calling regardless
of the `tool_dispatcher` setting in `[agent]`. On llama3.2 this means
the inbox_worker's multi-step loop depends on the model's native
function-calling quality. If processing is unreliable, consider using a
more capable model for `[agents.inbox_worker]`.
