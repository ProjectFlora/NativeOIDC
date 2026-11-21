# Performance Tracking for CI/CD

This document tracks build time improvements from cache usage.

## Issue #177: Rust Cache for Android CI/Release/Maven

### Goal

Reduce cold/warm execution times for Android CI, Release builds, and Maven publication by adding Rust cache to `ci.yml` and `release.yml`.

### Implementation (Nov 2026)

Added `Swatinem/rust-cache@v2` to:
- `ci.yml`: `android_adapters` job
- `release.yml`: `android`, `publish_maven` jobs

Cache configuration:
```yaml
uses: Swatinem/rust-cache@v2
with:
  cache-local-path: ~/.cargo/registry/cache/*
  cache-target-dir: target/debug
  restore-keys: |
    # Restore for each NDK/API/toolchain combination to avoid stale builds
    ${{ env.ANDROID_NDK_HOME }}-${{ env.ANDROID_API_LEVEL }}-*
```

### Cache Keys

- `~/.cargo/registry/cache/*`: Cargo registry cache
- `target/debug`: Rust build artifacts

Cache is separated by:
- NDK version (`27.2.12479018`)
- API level (`28` for CI, `35` for Android emulator tests)
- Toolchain target (implicitly handled by the runner/arch)

**Not cached**: Secrets (`GPG_PRIVATE_KEY`, `MAVEN_USER`, `MAVEN_PASSWORD`, etc.)

### Time Comparison

To be filled after first run:

| Run Type | Before | After | Delta |
|----------|--------|-------|-------|
| Cold (unit test) | 89s | TBD | TBD |
| Warm (unit test) | N/A | TBD | -TBD% |
| Cold (assembleRelease) | 100s | TBD | TBD |
| Warm (assembleRelease) | N/A | TBD | -TBD% |

Cache transfer time is estimated from `restore-keys` matches and local cache size.

### Verification Checklist

- [ ] CI runs successfully with cache enabled
- [ ] Both ABI outputs are present in AAR (`arm64-v8a`, `x86_64`)
- [ ] License notices are correct
- [ ] Consumer validation passes
- [ ] Cache invalidation works when NDK/API/toolchain changes

### See Also

- [Issue #177](https://github.com/ProjectFlora/NativeOIDC/issues/177)
- [Android のエミュレーターテストを、API 28 に加えて API 35 でも実行する #166](https://github.com/ProjectFlora/NativeOIDC/issues/166)
