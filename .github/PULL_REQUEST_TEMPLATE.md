## Summary

- What changed and why?

## Linked issues

- Closes #

## Validation

- [ ] `cargo test --workspace`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo fmt --all -- --check`
- [ ] `cd bindings && npm ci && npm run build`

## Governance checklist

- [ ] I reviewed `CONTRIBUTING.md` for workflow expectations
- [ ] I checked `CODEOWNERS` impact for touched paths
- [ ] I followed `SUPPORT.md` disclosure guidance for any security-sensitive change

## Snapshot policy

- [ ] If snapshot files under `contracts/test_snapshots/` changed, I reviewed the diff and confirmed every change is intentional
- [ ] If snapshot drift was reported in CI, I either regenerated snapshots or marked the drift as expected in the PR description
