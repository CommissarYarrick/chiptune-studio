# Agent workflow

- Read the score with `chip inspect` before editing. Use its `score_sha256` to reject stale revisions.
- Preserve the user's original recording. Save edits as new revisions.
- Render a short preview before extending an arrangement. Do not claim to have listened when only technical metrics were inspected.
- A manifest's hashes and levels are technical evidence, not artistic approval.
- Keep takes, projects, temporary files, and renders under the configured data directory; never commit private recordings or credentials.
- Run targeted tests while iterating; run cargo test, cargo clippy --all-targets -- -D warnings, and cargo fmt --all --check before publishing. All checks run locally; do not add GitHub CI.
- Keep public documentation, comments, and interface strings in English.

- Keep the project MIT. Only permissive dependencies with MIT-compatible terms are allowed. Preserve all upstream notices and refresh the dependency license inventory when Cargo.lock changes.
