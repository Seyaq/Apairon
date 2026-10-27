# Agent Instructions

## Scope

These instructions apply to coding agents working in this repository. Follow the user's latest explicit instructions and preserve the existing project goals.

## Before editing

- Inspect the relevant source, project structure, Cargo configuration, tests, and documentation.
- Verify the installed Rust toolchain when possible; do not assume version numbers or API availability.
- Read `CLAUDE.md` for project-specific rules when present.
- For Rust/game-engine work, consult `.claude/skills/rust-game-engine-engineering/SKILL.md` when available.
- Do not guess the contents of files you have not inspected.

## Engineering rules

- Prefer stable, idiomatic Rust and Rust 2024 for new code unless the project specifies otherwise.
- Prioritize correctness, explicit ownership, clear errors, lifecycle safety, modularity, and maintainability.
- Prefer safe Rust; justify and localize any `unsafe` code.
- Keep dependencies, allocations, abstractions, and refactors minimal and justified.
- Measure before making performance claims or optimizations.
- Never hard-code a runtime delta time in place of measured elapsed time.
- Do not introduce major engine subsystems without a concrete requirement.
- Preserve unrelated behavior and do not silently change project requirements.

## Verification

Use relevant checks for the actual workspace, typically formatting, `cargo check`, tests, and Clippy. Run them when possible, fix root causes, and report failures or unrun checks honestly. Never claim to have tested or benchmarked code without actually doing so.

## Delivery

Summarize:
1. What changed and why.
2. Which files were affected.
3. Which checks actually ran and their results.
4. Remaining risks or next steps.

Be direct and precise. Do not fabricate repository state, command output, benchmarks, or successful verification.
