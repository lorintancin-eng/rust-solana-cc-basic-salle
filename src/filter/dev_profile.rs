//! 过滤条件③④⑤：dev 画像
//!
//!   ③ dev_max_open_count    —— dev 历史毕业 (migrated) token 数 ≤ N
//!   ④ dev_max_created_count —— dev 总创建 token 数 ≤ N
//!   ⑤ dev_max_twitter_bound —— dev Twitter 绑定 token 数 ≤ N
//!
//! 架构：`DevProvider` 是数据源抽象，三种实现：
//!   - `Stub`        永远返回"无数据"（filter Pass）
//!   - `LocalIndex`  本地 sled 索引（mint→creator + dev stats）
//!   - `Gmgn`        GMGN OpenAPI（推荐 —— 实时、数据全）
//!
//! main.rs 启动时：env `GMGN_API_KEY` 存在 → 优先 Gmgn；否则退回 LocalIndex；
//! 都不可用退回 Stub。
//!
//! 数据缺失（Stub 或网络失败）时整体 Pass —— 抢入路径优先，宁错过过滤也不阻塞买入。

use std::sync::Arc;

use solana_sdk::pubkey::Pubkey;

use super::dev_profile_gmgn::GmgnProvider;
use super::FilterOutcome;
use crate::dev_index::{DevIndex, DevStats as IndexDevStats};
use crate::groups::CopyGroup;

/// dev 钱包画像统计（filter 层使用，与 dev_index::DevStats 等价）
#[derive(Debug, Clone, Copy, Default)]
pub struct DevStats {
    /// 已毕业的 token 数（migrated to PumpSwap/Raydium）
    pub open_count: u32,
    /// dev 总共创建过的 token 数
    pub created_count: u32,
    /// dev 推特账号绑定过的 token 数
    pub twitter_bound: u32,
}

impl From<IndexDevStats> for DevStats {
    fn from(s: IndexDevStats) -> Self {
        Self {
            open_count: s.open_count,
            created_count: s.created_count,
            twitter_bound: s.twitter_bound,
        }
    }
}

/// dev 数据源抽象。
/// 内部用 enum 而不是 trait object，避免动态分发开销。
#[derive(Clone)]
pub enum DevProvider {
    /// 无数据源 - `lookup_by_mint` 永远返回 None，filter 永远 Pass
    Stub,
    /// 本地 sled 索引（pump.fun create 实时 + 48h 历史回扫）
    /// 索引按 `mint → creator → DevStats` 链路查询。
    LocalIndex(Arc<DevIndex>),
    /// GMGN OpenAPI（推荐 —— 数据全面、无需自建索引）
    Gmgn(Arc<GmgnProvider>),
}

impl DevProvider {
    /// 占位实现：永远返回"无数据"
    pub fn stub() -> Self {
        Self::Stub
    }

    /// 本地索引数据源
    pub fn local_index(dev_index: Arc<DevIndex>) -> Self {
        Self::LocalIndex(dev_index)
    }

    /// GMGN 数据源
    pub fn gmgn(provider: Arc<GmgnProvider>) -> Self {
        Self::Gmgn(provider)
    }

    /// 按 mint 查 dev 画像（mint 上的 dev = token creator）。
    /// 返回 None 表示数据不可用（filter 默认 Pass，保留抢入路径）。
    pub fn lookup_by_mint(&self, mint: &Pubkey) -> Option<DevStats> {
        match self {
            Self::Stub => None,
            Self::LocalIndex(idx) => {
                // sled: mint_key → creator bytes → dev stats
                let creator_bytes = idx.lookup_creator_for_mint(mint)?;
                let creator = Pubkey::try_from(creator_bytes.as_slice()).ok()?;
                idx.lookup(&creator).map(DevStats::from)
            }
            Self::Gmgn(gmgn) => gmgn.lookup_by_mint(mint),
        }
    }

    /// 按 mint 查是否有社交链接（条件 ②）。
    /// 仅 GMGN provider 支持（沿用同一次 token/info 调用的 link 段）。
    /// 其它 provider 返回 None → 调用方继续走 RPC 兜底路径。
    pub fn lookup_social_by_mint(&self, mint: &Pubkey) -> Option<bool> {
        match self {
            Self::Stub | Self::LocalIndex(_) => None,
            Self::Gmgn(gmgn) => gmgn.lookup_social_by_mint(mint),
        }
    }
}

/// 主入口：按 `mint` 评估 dev 过滤条件 ③④⑤。
/// 注意：`source_wallet` 不一定是 dev（2ev 反向跟单时 source_wallet = 2ev 本人，
/// 而 dev = mint 的 creator）。所以这里**只用 mint** 查 dev 画像。
pub fn check(group: &CopyGroup, mint: &Pubkey, provider: &DevProvider) -> FilterOutcome {
    let Some(stats) = provider.lookup_by_mint(mint) else {
        // 数据不可用 → 默认 Pass（避免误锁；等接入真实数据源）
        return FilterOutcome::Pass;
    };

    if let Some(limit) = group.dev_max_open_count {
        if stats.open_count > limit {
            return FilterOutcome::Reject(format!(
                "dev_open={} > limit={}",
                stats.open_count, limit
            ));
        }
    }
    if let Some(limit) = group.dev_max_created_count {
        if stats.created_count > limit {
            return FilterOutcome::Reject(format!(
                "dev_created={} > limit={}",
                stats.created_count, limit
            ));
        }
    }
    if let Some(limit) = group.dev_max_twitter_bound {
        if stats.twitter_bound > limit {
            return FilterOutcome::Reject(format!(
                "dev_tw={} > limit={}",
                stats.twitter_bound, limit
            ));
        }
    }

    FilterOutcome::Pass
}
