# GUARDRAILS.md — Multi-Directory

**Rust Guardrails — Vibe Engineering Standard**

## Non-Negotiable
- No `unwrap()` or `expect()` in production code.
- `validate_domain_safe()` guards all handlers — never bypass.
- `Command::new()` safe by design — shell injection not exploitable.
- JWT middleware on all 402+ handlers with public path bypass — never add unchecked routes.
- All 17 domains must return 200.
- `cargo clippy -- -D warnings` must pass.
- Build through `/usr/local/bin/swift-build.sh multidirectory-rust`.
