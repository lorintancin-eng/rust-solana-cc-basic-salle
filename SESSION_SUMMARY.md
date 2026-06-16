# Session Summary

## Current Goal
- Add an independent external DEX route for PumpSwap, Raydium AMM v4, and Raydium CPMM.
- Keep Pump.fun internal routing as the first priority.
- Support external dry-run logging first, then Jupiter small-position live buy/sell.
- Keep Cargo binary name `copy-trader`, GitHub artifact `copy-trader-linux`, and VPS runtime name `copy-trader-basice-salle`.

## Current Progress
- Added group-level external settings with serde defaults:
  - `external_mode`: `off`, `dry_run`, `live`; default is now `dry_run`
  - `external_buy_sol_amount`: default `0.002`
  - old per-venue fields are retained for `copy_groups.json` compatibility but no longer split external routing
- Added Telegram setting support:
  - `/set <group> external off|dry_run|live`
  - `/set <group> external_buy 0.002`
  - the Telegram setting menu exposes one external mode control; PumpSwap, Raydium AMM, and Raydium CPMM are enabled or disabled together by `external_mode`
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
- Follow-up fix after a real missed wallet trade:
  - The wallet `8zkgFGVZrDLieViwqiXFCydSX6WL5hsxmUu55yBdsNsZ` traded through Pump AMM program `pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA`.
  - Added that Pump AMM program as a PumpSwap external venue alias.
  - Added Pump AMM account layout support: token mint from instruction slots `3/4`, token program from slots `11/12`.
  - Added Pump AMM discriminator handling so sell instructions are not misclassified as buys.
  - Bumped version from `1.8.2` to `1.8.3`.
- Follow-up simplification after user requested one external switch:
  - New groups and missing `external_mode` fields default to `dry_run`.
  - `external_mode=off` disables all external venues.
  - `external_mode=dry_run` or `live` enables PumpSwap, Raydium AMM, and Raydium CPMM together.
  - Removed separate PumpSwap/Raydium AMM/Raydium CPMM buttons and shortcut setting keys from the Telegram menu/help.
  - Bumped version from `1.8.3` to `1.8.4`.

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
- Latest Pump AMM follow-up changed:
  - `Cargo.toml`
  - `Cargo.lock`
  - `src/grpc/subscriber.rs`
  - `SESSION_SUMMARY.md`
- Latest external switch simplification changed:
  - `Cargo.toml`
  - `Cargo.lock`
  - `src/groups.rs`
  - `src/telegram.rs`
  - `SESSION_SUMMARY.md`
- Existing local diffs/formatting were preserved in:
  - `src/grpc/account_subscriber.rs`
  - `src/grpc/mod.rs`
  - `src/processor/raydium_cpmm.rs`
  - `src/utils/sol_price.rs`
  - `src/utils/token_info.rs`

## Validation
- `cargo fmt --check` passed for version `1.8.4`.
- `cargo metadata --format-version 1 --no-deps` passed and reports:
  - package version `1.8.4`
  - binary target `copy-trader`
- `git diff --check` passed with only Windows line-ending warnings.
- `cargo test external --bin copy-trader` on Windows failed before project source checks in `protobuf-src` because this local environment lacks `sh`.
- `cargo check --bin copy-trader` on Windows failed before project source checks in `protobuf-src` because this local environment lacks `sh`.
- `cargo test pump_amm --bin copy-trader` on Windows hit the same `protobuf-src` / missing `sh` blocker before test compilation.
- Production validation must come from GitHub Actions Ubuntu build.

## CI / GitHub Actions
- Workflow file unchanged: `.github/workflows/build-copy-trader-linux.yml`.
- Trigger remains push to `main` and `workflow_dispatch`.
- Build command remains `cargo build --release --bin copy-trader`.
- Artifact name remains `copy-trader-linux`.
- Artifact internal executable remains `copy-trader`.
- No automatic VPS deployment was added.
- Code commit was pushed to GitHub `main` via GitHub Git Database API because local `git push` HTTPS timed out.
- Remote code commit: `05430f7c53443a9f829d9ba57082896d481e9fef`.
- GitHub Actions run `27560747785` completed successfully on Ubuntu.
- Artifact `copy-trader-linux` was created.
- Artifact was downloaded and verified to contain executable file `copy-trader`.

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
1. Manually download and run the latest `copy-trader-linux` artifact on the VPS.
2. Start with `external dry_run`, verify PumpSwap/Raydium side and mint logs, then switch selected groups to `external live`.
3. Do not run this project with the same wallet/config as another copy-trader instance.

## Exact Next Step For Next Thread
- Read `AGENTS.md`.
- Read this `SESSION_SUMMARY.md`.
- Check the latest GitHub Actions run on `main`.
- If the latest run is green, continue with manual VPS deployment using the commands above.
