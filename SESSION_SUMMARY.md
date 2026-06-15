# Session Summary

## Current Goal
- Remove all 2ev-specific strategy code from the current Rust project.
- Keep the generic copy-trader behavior, Cargo binary name, and GitHub Actions artifact naming unchanged.

## Current Progress
- Removed 2ev entry filter wiring from `main.rs`.
- Removed the `filter` module and files used by the 2ev market-cap/social/dev-profile checks.
- Removed the `dev_index` module and files used by the 2ev dev-profile data source.
- Removed 2ev-specific `CopyGroup` fields and persistence mappings:
  - market-cap/social/dev filters
  - floor-sell disabling
  - migration exit
  - automatic TP/trailing/migration partial-sell ratios
  - USD buy override
- Removed automatic migration-exit sell behavior from `AutoSellManager`.
- Removed automatic `SellSignal` ratio-based partial sells.
- Kept Telegram manual partial sell buttons and execution path.
- Removed Telegram `/set` keys and group-detail display for deleted strategy fields.
- Removed `sled` and `uuid` dependencies from `Cargo.toml`; pruned the root lockfile dependency and `sled` package entry.
- Bumped version from `1.8.0` to `1.8.1`.

## Files Changed
- `Cargo.toml`
- `Cargo.lock`
- `SESSION_SUMMARY.md`
- `src/main.rs`
- `src/groups.rs`
- `src/autosell/manager.rs`
- `src/autosell/position.rs`
- `src/tx/sell_executor.rs`
- `src/telegram.rs`
- `src/processor/pumpfun.rs`
- deleted `src/filter/*`
- deleted `src/dev_index/*`

## Validation
- `cargo fmt` passed.
- `cargo metadata --format-version 1 --no-deps` passed and reports:
  - package version `1.8.1`
  - binary target `copy-trader`
  - no `sled` or `uuid` direct dependencies
- Static scan found no 2ev/dev-index/filter/sell-ratio strategy keywords in `src`, `Cargo.toml`, `Cargo.lock`, or `.github/workflows`.
- `cargo check --bin copy-trader` did not reach project source checks on Windows. It failed in `protobuf-src` because the Windows environment lacks `sh` for the dependency build script. Production validation remains GitHub Actions Ubuntu build.

## CI / GitHub Actions
- Workflow file unchanged: `.github/workflows/build-copy-trader-linux.yml`.
- Trigger: push to `main` and `workflow_dispatch`.
- Build command remains `cargo build --release --bin copy-trader`.
- Artifact name remains `copy-trader-linux`.
- Artifact internal executable remains `copy-trader`.
- No automatic VPS deployment was added.

## Artifact / VPS Naming
- Cargo binary: `copy-trader`
- GitHub Actions artifact: `copy-trader-linux`
- Binary inside artifact: `copy-trader`
- VPS runtime binary name: `copy-trader-basice-salle`
- VPS runtime directory: `/home/ubuntu/rust_project_basice-salle`
- VPS log file: `copy-trader-v1.8.log`

## Manual VPS Steps After CI Passes
```bash
mkdir -p /tmp/copy-trader-basice-salle
rm -f /tmp/copy-trader-basice-salle/copy-trader

gh run list --repo lorintancin-eng/rust-solana-cc-basic-salle --workflow "Build Copy Trader Linux" --branch main --limit 5
gh run download <RUN_ID> --repo lorintancin-eng/rust-solana-cc-basic-salle --name copy-trader-linux -D /tmp/copy-trader-basice-salle

chmod +x /tmp/copy-trader-basice-salle/copy-trader
mkdir -p /home/ubuntu/rust_project_basice-salle
cp /tmp/copy-trader-basice-salle/copy-trader /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle
chmod +x /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle

pkill -f /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle || true
cd /home/ubuntu/rust_project_basice-salle
nohup ./copy-trader-basice-salle > copy-trader-v1.8.log 2>&1 &
tail -f copy-trader-v1.8.log
```

## Rollback
```bash
cd /home/ubuntu/rust_project_basice-salle
pkill -f /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle || true
cp ./copy-trader-basice-salle.bak ./copy-trader-basice-salle
chmod +x ./copy-trader-basice-salle
nohup ./copy-trader-basice-salle > copy-trader-v1.8.log 2>&1 &
```

## Remaining Work
1. Commit local changes.
2. Push to GitHub `main`.
3. Watch GitHub Actions Ubuntu build for the final production validation.
4. If CI passes, manually download and run the `copy-trader-linux` artifact on the VPS.

## Exact Next Step For Next Thread
- Read `AGENTS.md`.
- Read this `SESSION_SUMMARY.md`.
- Check whether local git metadata is available; this workspace currently reported `not a git repository`.
- If git metadata is restored or initialized safely, commit and push to `main`.
- Then watch GitHub Actions for `copy-trader-linux`.
