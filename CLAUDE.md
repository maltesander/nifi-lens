# Project Rules

Read and follow @AGENTS.md — it contains architecture, patterns, and procedures.

## Rules

- Always ask to update @AGENTS.md when architecture and patterns change, or new knowledge is acquired.
- Always add unit, wiremock or integration tests for new functionality.
- Always update each README.md for any changes or new features.
- Always refer to <https://docs.rs/crate/nifi-rust-client/0.5.0> documentation.

## Scope

- Never modify files outside the scope of the current task.
- Never add features, refactoring, or "improvements" beyond what was asked.
- Always ask if unsure whether something is in scope.

## Data Retrieval

Never read entire files by default. Survey, locate, then extract.

1. **Survey first** — check file size before reading (`stat -c%s file`). Files >50 KB must be sliced, not read whole.
2. **Navigate definitions with ctags** — run `ctags -R .` once to build a tags index, then `grep "^SymbolName" tags` to find the exact file and line of any function, struct, or trait — no file reading needed.
3. **Locate with Grep** — find patterns, keywords, or usages before reading. Use `-C` for context lines.
4. **Extract with Read (offset + limit)** — once you know the line range, read only that slice.
5. **Structured data** — use `jq` for JSON, `yq` for YAML; never read raw markup whole.
6. **Filesystem survey** — use `tree -L 2 -I '.git|target|node_modules'` instead of recursive `ls`.
7. **Verify edits with diff** — after editing, `diff -u` to confirm changes instead of re-reading.
