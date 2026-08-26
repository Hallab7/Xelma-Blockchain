## WASM Size Budget

### Current baseline
Stored in `.wasm-size-baseline` at the repo root (plain integer, bytes).

### Budget policy
CI allows up to **+10%** growth above baseline before failing.

### How CI checks it
The `wasm-size-gate` job in `.github/workflows/ci.yml`:
1. Builds `xelma_contract.wasm` in release mode
2. Compares size against `.wasm-size-baseline` + 10%
3. Prints a size report on every build
4. Fails with an actionable error if the budget is exceeded

### Updating the baseline
When intentional size growth is merged (new feature, dependency bump):
1. Build locally: `cargo rustc --manifest-path=contracts/Cargo.toml --crate-type=cdylib --target=wasm32v1-none --release --locked`
2. Measure: `wc -c < target/wasm32v1-none/release/xelma_contract.wasm`
3. Update `.wasm-size-baseline` with the new byte count
4. Commit and push with message: `chore: update WASM size baseline to <N> bytes`

### Why this matters
Soroban contracts have deployment and execution constraints. Unbounded size growth
makes mainnet deployment riskier and more expensive. This gate ensures size changes
are intentional and reviewed.
