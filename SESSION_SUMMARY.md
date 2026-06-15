# Session Summary

## Current Goal
- Add an independent external DEX route for PumpSwap, Raydium AMM v4, and Raydium CPMM.
- Keep Pump.fun internal routing as the first priority.
- Support external dry-run logging first, then Jupiter small-position live buy/sell.
- Keep Cargo binary name `copy-trader`, GitHub artifact `copy-trader-linux`, and VPS runtime name `copy-trader-basice-salle`.

## Current Progress
- Added group-level external settings with serde defaults:
  - `external_mode`: `off`, `dry_run`, `live`
  - `external_buy_sol_amount`: default `0.002`
  - per-venue flags for PumpSwap, Raydium AMM, Raydium CPMM
- Added Telegram setting support:
  - `/set <group> external off|dry_run|live`
  - `/set <group> external_buy 0.002`
  - `/set <group> external_pumpswap on|off`
  - `/set <group> external_raydium_amm on|off`
  - `/set <group> external_raydium_cpmm on|off`
- Split transaction detection into internal and external parse scopes:
  - Pump.fun is parsed first.
  - External venues are parsed only after internal parsing fails.
- Main trade loop now routes external trades through a separate external branch and `continue`s before Pump.fun handling.
- External dry-run logs `EXTERNAL_DRY_RUN` with group, venue, side, mint, wallet, SOL amount, signature, and failed flag.
- External dry-run does not send transactions, does not create positions, and does not send Telegram buy-success messages.
- External live uses Jupiter SOL-to-token buy construction with group slippage and `external_buy_sol_amount`.
- External positions are marked as `ExternalJupiter` and use Jupiter token-to-SOL quote for autosell valuation.
- External sell path skips Pump.fun bonding curve snapshot requirements and routes via Jupiter.
- Bumped version from `1.8.1` to `1.8.2`.

## Files Changed
- `Cargo.toml`
- `Cargo.lock`
- `SESSION_SUMMARY.md`
- `src/groups.rs`
- `src/telegram.rs`
- `src/grpc/subscriber.rs`
- `src/main.rs`
- `src/tx/jupiter.rs`
- `src/autosell/position.rs`
- `src/autosell/mod.rs`
- `src/autosell/persistence.rs`
- `src/autosell/manager.rs`
- `src/tx/sell_executor.rs`
- Existing local diffs/formatting were preserved in:
  - `src/grpc/account_subscriber.rs`
  - `src/grpc/mod.rs`
  - `src/processor/raydium_cpmm.rs`
  - `src/utils/sol_price.rs`
  - `src/utils/token_info.rs`

## Validation
- `cargo fmt --check` passed.
- `cargo metadata --format-version 1 --no-deps` passed and reports:
  - package version `1.8.2`
  - binary target `copy-trader`
- `git diff --check` passed with only Windows line-ending warnings.
- `cargo check --bin copy-trader` on Windows failed before project source checks in `protobuf-src` because this local environment lacks `sh`.
- Production validation must come from GitHub Actions Ubuntu build.

## CI / GitHub Actions
- Workflow file unchanged: `.github/workflows/build-copy-trader-linux.yml`.
- Trigger remains push to `main` and `workflow_dispatch`.
- Build command remains `cargo build --release --bin copy-trader`.
- Artifact name remains `copy-trader-linux`.
- Artifact internal executable remains `copy-trader`.
- No automatic VPS deployment was added.
- Push/CI status still needs to be completed for this external route change.

## Artifact / VPS Naming
- Cargo binary: `copy-trader`
- GitHub Actions artifact: `copy-trader-linux`
- Binary inside artifact: `copy-trader`
- VPS runtime binary name: `copy-trader-basice-salle`
- VPS runtime directory: `/home/ubuntu/rust_project_basice-salle`
- VPS log file: `copy-trader-v1.8.log`

## Manual VPS Steps After CI Passes
```bash
cd /home/ubuntu/rust_project_basice-salle

mkdir -p /tmp/copy-trader-basice-salle
rm -f /tmp/copy-trader-basice-salle/copy-trader

gh run list --repo lorintancin-eng/rust-solana-cc-basic-salle --workflow "Build Copy Trader Linux" --branch main --limit 5
gh run download <RUN_ID> --repo lorintancin-eng/rust-solana-cc-basic-salle --name copy-trader-linux -D /tmp/copy-trader-basice-salle

chmod +x /tmp/copy-trader-basice-salle/copy-trader
cp /tmp/copy-trader-basice-salle/copy-trader /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle
chmod +x /home/ubuntu/rust_project_basice-salle/copy-trader-basice-salle

# dry-run example
./copy-trader-basice-salle >> copy-trader-v1.8.log 2>&1
tail -f copy-trader-v1.8.log | grep EXTERNAL_DRY_RUN
```

## Rollback
```bash
git revert <external-route-commit>
git push origin main
```

On the VPS, stop the current process, restore the previous `copy-trader-basice-salle` backup or download a previous successful artifact, then restart it.

Config-only rollback:
```text
/set <group> external off
```

## Remaining Work
1. Commit and push this external route change to `main`.
2. Wait for GitHub Actions Ubuntu build to pass.
3. Confirm artifact `copy-trader-linux` contains executable `copy-trader`.
4. Manually download and run it on the VPS.
5. Start with `external dry_run`, verify PumpSwap/Raydium side and mint logs, then switch selected groups to `external live`.

## Exact Next Step For Next Thread
- Read `AGENTS.md`.
- Read this `SESSION_SUMMARY.md`.
- Check `git status`, commit/push if not done, then monitor GitHub Actions.
