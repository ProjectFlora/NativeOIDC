<!-- agent-workflow -->

## PR for #179

### Goal
Fold the "Swift package bindings match" check into the `Tests` job, removing the standalone `spm_bindings` job that duplicated binding generation.

### Changes
- `.github/workflows/ci.yml`: Added `Swift package bindings match` step to the `test` job after `Generate native bindings`, removed the standalone `spm_bindings` job and its references
- `scripts/select_ci_jobs.py`: Removed `spm_bindings` from `JOBS` and `JOB_GROUPS`; updated five selection rules
- `scripts/test_select_ci_jobs.py`: Updated expected sets for `native_oidc.swift` and `generate-bindings.sh`
- `docs/ci-job-selection.md`: Removed `spm_bindings` row from job-groups table; updated rule tables
- `CHANGELOG.md`: Added Changed entry under `[Unreleased]`

### Selection rules (per plan)
The following paths now select `test` instead of `spm_bindings`:
1. `src/**`: Rust, packages (was: Rust, packages, spm_bindings)
2. `uniffi-bindgen.rs`: Rust, apple, android (was: Rust, apple, android, spm_bindings)
3. `Sources/NativeOIDC/native_oidc.swift`: apple, test (was: apple, spm_bindings)
4. `scripts/generate-bindings.sh`: test, apple, android (was: test, apple, android, spm_bindings)
5. `scripts/check-spm-bindings.sh`: test (was: test, spm_bindings)

### Branch protection verification
Verified that `Swift package bindings match` is not a required check via the API; no branch-protection change needed.

### Time comparison (baseline from run 44632551243)
| Phase | Before | After |
| --- | --- | --- |
| Tests job | ~61s (includes generation + Kotlin build) | ~61s |
| Swift package bindings match | ~28s (standalone job) | Folded into Tests |
| **Total wall time** | ~90s (sequential on Linux) | ~61s (parallel within Tests) |

The PR's own run will fill in the actual timings.

### Out of scope
- The `spm_manifest` job: unchanged (checks Package.swift without generation)
- Apple/Android binding generation: fingerprint cache already dedupes
- Release workflows: never referenced the SPM bindings check
