//! 过滤条件①：入场市值上限
//!
//! 算法：
//!   mcap_sol = price_sol(bonding_curve) * (token_total_supply / 10^6)
//!   mcap_usd = mcap_sol * sol_usd_price
//!   pass if mcap_usd < group.max_entry_mcap_usd
//!
//! 数据全部从内存缓存读取（BondingCurveCache + SolUsdPrice），零 RPC，纳秒级。
//! 缓存缺失时默认通过 —— 反向跟单要保同区块抢入，宁可漏过滤也不阻塞。

use solana_sdk::pubkey::Pubkey;

use super::FilterOutcome;
use crate::groups::CopyGroup;
use crate::grpc::BondingCurveCache;
use crate::utils::sol_price::SolUsdPrice;

const PUMPFUN_TOKEN_DECIMALS: f64 = 1_000_000.0; // 6 decimals

pub fn check(
    group: &CopyGroup,
    mint: &Pubkey,
    bc_cache: &BondingCurveCache,
    sol_usd: &SolUsdPrice,
) -> FilterOutcome {
    let limit = match group.max_entry_mcap_usd {
        Some(v) if v > 0.0 => v,
        _ => return FilterOutcome::Pass,
    };

    let bc_state = match bc_cache.get(mint) {
        Some(state) => state,
        None => return FilterOutcome::Pass, // 缓存未到位 → 默认通过
    };

    let price_sol = bc_state.price_sol();
    if price_sol <= 0.0 {
        return FilterOutcome::Pass;
    }

    let sol_usd_price = sol_usd.get();
    if sol_usd_price <= 0.0 {
        return FilterOutcome::Pass;
    }

    let supply_display = bc_state.token_total_supply as f64 / PUMPFUN_TOKEN_DECIMALS;
    let mcap_sol = price_sol * supply_display;
    let mcap_usd = mcap_sol * sol_usd_price;

    if !group.passes_mcap_filter(mcap_usd) {
        return FilterOutcome::Reject(format!(
            "mcap=${:.0} >= limit=${:.0}",
            mcap_usd, limit
        ));
    }

    FilterOutcome::Pass
}
