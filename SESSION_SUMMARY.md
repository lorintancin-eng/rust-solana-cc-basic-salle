# Session Summary

## Current Goal
- 在现有架构上**纯增量**接入 2ev 反向跟单策略
  （目标钱包 sell → 我们反向买；5 条进场过滤；非迁移留仓；迁移盘 ATH 回撤部分卖；50x 部分锁利）
- 现有 SMART_BUY 跟单组**完全不受影响**（所有新字段默认值都让旧组保持原行为）

## Strategy 来源
- 策略文档：`C:\Users\123\OneDrive\Desktop\2ev卖出跟单策略.rtf`
- 目标钱包：`2ezv4U5HmPpkt2xLsKnw1FyyGmjFBeW7c166p99Hw2xB`
- 触发：2ev 第一笔 sell；进场价 ≈ 0.95× sell 价；同区块抢入

## Current Progress

### ✅ 已实现（本次提交）
1. **数据模型扩展**
   - [src/autosell/position.rs](src/autosell/position.rs)：
     - `SellReason::MigrationCompleted` 新枚举值
     - `SellSignal.sell_ratio: f64` 新字段（默认 1.0 = 全卖）
     - `SellSignal::full(...)` 工厂方法
   - [src/groups.rs](src/groups.rs)：`CopyGroup` 新增 10 个字段
     - `max_entry_mcap_usd: Option<f64>` 条件①
     - `require_social_link: bool` 条件②
     - `dev_max_open_count / dev_max_created_count / dev_max_twitter_bound: Option<u32>` 条件③④⑤
     - `disable_floor_sell: bool`（"非迁移永不卖"，禁用 SL/MaxLifetime）
     - `migration_exit_enabled: bool`
     - `trailing_partial_sell_ratio / take_profit_partial_ratio / migration_exit_partial_ratio: f64`
   - `PersistedGroup` 同步 + `#[serde(default)]` 兼容旧 `copy_groups.json`

2. **进场过滤模块（新建）**
   - [src/filter/mod.rs](src/filter/mod.rs)：`EntryFilters` + `FilterOutcome`
   - [src/filter/mcap.rs](src/filter/mcap.rs)：条件① 完整实现（同步、零 RPC，复用 `BondingCurveCache + SolUsdPrice`）
   - [src/filter/social.rs](src/filter/social.rs)：条件② **TODO 占位**（默认通过；接入 Metaplex metadata + IPFS 后切换实现）
   - [src/filter/dev_profile.rs](src/filter/dev_profile.rs)：条件③④⑤ **TODO 占位**（默认通过；待 D2 数据源决策）

3. **出场触发器扩展**
   - [src/autosell/manager.rs](src/autosell/manager.rs)：
     - `check_exit_conditions`：`disable_floor_sell` 时跳过 SL/MaxLifetime
     - 新增 `check_migration_exit`：bonding curve `complete=true` 触发 MigrationCompleted
     - trailing/TP 触发使用 `trailing_sell_ratio` / `take_profit_sell_ratio`（默认 1.0）
     - 接入两个调用点（`start_grpc_monitor` / `start_fallback_monitor`）

4. **部分卖执行**
   - [src/tx/sell_executor.rs](src/tx/sell_executor.rs:442)：`handle_sell_signal` 接收 `sell_ratio`，
     `ratio<1.0` 时按比例计算 `token_amount`，复用现有 `apply_partial_sell` 路径

5. **主路径接入**
   - [src/main.rs](src/main.rs)：`mod filter;` + 启动构造 `EntryFilters` + 在反向跟单分支调用过滤
   - 仅 `!trade.is_buy && group.buy_on_smart_sell()` 时启用过滤；SMART_BUY 路径完全不变

6. **Telegram 控制面**
   - [src/telegram.rs](src/telegram.rs)：`/set` 新增 10 个参数 key：
     - `max_mc / require_social / dev_open / dev_created / dev_tw`
     - `no_floor_sell / migration_exit / trailing_ratio / tp_ratio / migration_ratio`
   - 同步更新 `setting_label` 中文显示

7. **版本号**：`1.6.63 → 1.7.0`（minor 升级，新策略接入）

### ⚠️ 故意未做（标 TODO 等用户拍板）
- **D2 dev 画像数据源**：dev_profile.rs 当前默认通过；接入 GMGN/BullX/自建索引时只需替换 `dev_profile::check`
- **条件②社交媒体抓取**：social.rs 当前默认通过；接入 Metaplex+IPFS 时只需替换 `social::check`
- **USD 计价系统**：未做；当前 `buy_sol_amount` 仍按 SOL 单位（用户需手动按 SOL 价折算 $100）

## Files Changed
- `Cargo.toml` (版本号)
- `SESSION_SUMMARY.md`
- `src/autosell/manager.rs`
- `src/autosell/position.rs`
- `src/filter/mod.rs` *(new)*
- `src/filter/mcap.rs` *(new)*
- `src/filter/social.rs` *(new TODO 占位)*
- `src/filter/dev_profile.rs` *(new TODO 占位)*
- `src/groups.rs`
- `src/main.rs`
- `src/telegram.rs`
- `src/tx/sell_executor.rs`

## CI / GitHub Actions Status
- Workflow: `.github/workflows/build-copy-trader-linux.yml`
- 触发：push to `main`
- 验证 source of truth：**GitHub Actions Linux build 唯一标准**
- Windows 本地 `cargo check` 因 `openssl-sys` Perl 模块问题失败（与本次修改无关，环境问题）

## Artifact / Package Naming
- Executable: `copy-trader`
- Artifact: `copy-trader-linux`

## Backup
- Tag: `backup/pre-2ev-strategy-v1.6.63` (origin)
- Branch: `backup/pre-2ev-strategy` (origin)
- 回滚命令：
  ```bash
  git checkout main && git reset --hard backup/pre-2ev-strategy-v1.6.63 && git push --force-with-lease origin main
  ```

## 配置 2ev 跟单组（部署后在 Telegram 执行）
```
/groupadd 2ev_reverse 2ezv4U5HmPpkt2xLsKnw1FyyGmjFBeW7c166p99Hw2xB
/usegroup 2ev_reverse
/set 2ev_reverse entry sell                  # 反向跟单（卖时买入）
/set 2ev_reverse mode tp_sl                  # 不跟卖
/set 2ev_reverse consensus 1
/set 2ev_reverse buy 0.4                     # 按 SOL=$250 折算 $100；自行调整
/set 2ev_reverse min_buy 0
/set 2ev_reverse slippage 2800
/set 2ev_reverse sell_slippage 2800
/set 2ev_reverse tip_buy 20000000            # Jito 0.02 SOL
/set 2ev_reverse tip_sell 20000000
/set 2ev_reverse zero_slot_tip 5000000       # 0slot 0.005 SOL
/set 2ev_reverse hold 0                      # 永不到期
/set 2ev_reverse max_mc 3000                 # 条件①：MC < $3000
/set 2ev_reverse require_social on           # 条件②（当前 TODO 默认通过）
/set 2ev_reverse dev_open 2                  # 条件③（当前 TODO 默认通过）
/set 2ev_reverse dev_created 10              # 条件④（当前 TODO 默认通过）
/set 2ev_reverse dev_tw 20                   # 条件⑤（当前 TODO 默认通过）
/set 2ev_reverse no_floor_sell on            # 非迁移永不卖
/set 2ev_reverse migration_exit on
/set 2ev_reverse trailing 30                 # ATH 回撤 30% 触发
/set 2ev_reverse trailing_ratio 0.66         # 卖 ATH 的 2/3
/set 2ev_reverse tp 5000                     # 50x 触发
/set 2ev_reverse tp_ratio 0.5                # 卖一半锁利
/set 2ev_reverse migration_ratio 1.0
/set 2ev_reverse enabled on
```

## Manual VPS Run Steps（GitHub Actions 通过后）
```bash
rm -rf /tmp/build && mkdir -p /tmp/build
gh run list --repo lorintancin-eng/rust-solana-cc --workflow "Build Copy Trader Linux" --branch main --limit 5
gh run download <RUN_ID> --repo lorintancin-eng/rust-solana-cc --name copy-trader-linux -D /tmp/build
chmod +x /tmp/build/copy-trader
cp /tmp/build/copy-trader /home/ubuntu/rust_project/copy-trader
pkill -f copy-trader || true
nohup /home/ubuntu/rust_project/copy-trader > /home/ubuntu/rust_project/copy-trader.log 2>&1 &
```

## Remaining Work
1. 用户审查本次改动（diff）
2. commit + push 到 `main`
3. 等 GitHub Actions Linux build 绿
4. 部署 `copy-trader-linux` artifact 到 VPS
5. TG 配置 2ev_reverse 组（按上面命令清单，仓位先压到 $20/单试运行 4-8 小时）
6. 观察日志：
   - "Entry filter rejected" 出现频率（看条件①过滤效率）
   - 实际进场价对比 2ev sell 价（验证 0.95× 假设）
   - migration 触发后是否走部分卖（看 "Partial sell" 日志）
7. 等用户拍板 **D2 dev 画像数据源**（当前 ③④⑤ 默认通过）
8. 视情况补 social / dev_profile 真实实现

## Exact Next Step For Next Thread
- 读 `AGENTS.md`
- 读 `SESSION_SUMMARY.md`
- 检查最新 `main` commit 与 GitHub Actions Linux 构建结果
- 若 CI 绿 → 部署 + TG 配置 2ev_reverse 组
- 若 CI 红 → 修复编译错误（最可能：未识别的 import / 未到位的 pub 导出）
