# Bash `output_file`

The `bash` tool accepts an optional `output_file` path. When set, Quecto writes the full combined stdout/stderr stream to that path and returns only a concise inline summary with the saved path and byte/line counts.

Use `output_file` for output you will compute over or query selectively (for example, JSON snapshots that `python_lab`, `grep`, or a diff will inspect). Output the model must actually read and judge belongs inline.

Anti-pattern: redirecting large output to `output_file` and then reading the whole file back into the conversation. That is re-ingestion, not a saving.

When `output_file` is absent, bash output remains inline with the existing 2000-line/50KB tail cap. Overflow output is also saved under a stable temp directory and the path is reported. Timeout results include any captured output tail before the process was killed.
