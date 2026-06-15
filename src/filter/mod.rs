//! 进场过滤模块（2ev 反向跟单策略）
//!
//! 仅在 `entry_mode == ENTRY_MODE_SMART_SELL`（反向跟单）路径上生效，
//! 不影响传统 SMART_BUY 跟单组的现有行为。
//!
//! 5 条过滤条件：
//!   ① max_entry_mcap_usd —— 入场市值上限（USD）。同步实现，零 RPC。
//!   ② require_social_link —— 至少一个社交链接。【TODO 占位，默认通过】
//!   ③ dev_max_open_count —— dev 历史毕业数上限。【TODO 占位，默认通过】
//!   ④ dev_max_created_count —— dev 总创建数上限。【TODO 占位，默认通过】
//!   ⑤ dev_max_twitter_bound —— dev 推特绑币数上限。【TODO 占位，默认通过】
//!
//! 配置任意一项才会启用对应过滤；全 None/false 时整体相当于关闭，无开销。

pub mod dev_profile;
pub mod dev_profile_gmgn;
mod mcap;
pub(crate) mod metadata;
mod social;

use std::sync::Arc;

use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;

use crate::groups::CopyGroup;
use crate::grpc::BondingCurveCache;
use crate::utils::sol_price::SolUsdPrice;

pub use social::SocialCache;

/// 单个 trade 是否通过 5 条进场过滤
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOutcome {
    /// 全部启用的过滤项均通过（或未启用任何过滤）
    Pass,
    /// 至少一项拒绝，附原因（用于日志）
    Reject(String),
}

impl FilterOutcome {
    pub fn is_pass(&self) -> bool {
        matches!(self, FilterOutcome::Pass)
    }
}

/// 进场过滤器集合
#[derive(Clone)]
pub struct EntryFilters {
    bc_cache: BondingCurveCache,
    sol_usd: SolUsdPrice,
    rpc_client: Arc<RpcClient>,
    social_cache: SocialCache,
    dev_provider: Arc<dev_profile::DevProvider>,
}

impl EntryFilters {
    pub fn new(
        bc_cache: BondingCurveCache,
        sol_usd: SolUsdPrice,
        rpc_client: Arc<RpcClient>,
    ) -> Self {
        Self {
            bc_cache,
            sol_usd,
            rpc_client,
            social_cache: SocialCache::new(),
            // 默认 stub 实现（永远 Pass）；通过 with_dev_provider 替换为真实数据源
            dev_provider: Arc::new(dev_profile::DevProvider::stub()),
        }
    }

    /// 注入真实的 dev 数据源（GMGN / BullX / 自建索引等）。
    /// 替换后 dev_profile::check 会用注入的 provider 评估。
    pub fn with_dev_provider(mut self, provider: dev_profile::DevProvider) -> Self {
        self.dev_provider = Arc::new(provider);
        self
    }

    /// 评估单个 trade 是否通过 group 配置的全部过滤项。
    /// 任何一项拒绝即整体拒绝。
    /// 数据缺失时该项默认通过 —— 抢入路径优先。
    ///
    /// `source_wallet` 当前仅给 social/mcap 用；dev 过滤改用 `mint` 内部反查
    /// creator（修复了之前误把 source_wallet 当 dev 的 bug）。
    pub fn evaluate(
        &self,
        group: &CopyGroup,
        mint: &Pubkey,
        _source_wallet: &Pubkey,
    ) -> FilterOutcome {
        if group.has_mcap_filter() {
            if let FilterOutcome::Reject(reason) =
                mcap::check(group, mint, &self.bc_cache, &self.sol_usd)
            {
                return FilterOutcome::Reject(reason);
            }
        }

        if group.require_social_link {
            // 优先 GMGN（pre-exec / 新 BC mint 上 Metaplex 还没落地的场景下唯一可用源）
            match self.dev_provider.lookup_social_by_mint(mint) {
                Some(true) => {
                    // GMGN 确认有 social link → 通过
                }
                Some(false) => {
                    return FilterOutcome::Reject("no social link (GMGN)".to_string());
                }
                None => {
                    // GMGN 未命中（cache miss / 非 GMGN provider）→ 退到 RPC 路径
                    // RPC 路径在抢入场景常常拿不到（mint 未落地），只作长期 fallback
                    social::spawn_prefetch(
                        *mint,
                        self.rpc_client.clone(),
                        self.social_cache.clone(),
                    );
                    if let FilterOutcome::Reject(reason) =
                        social::check(group, mint, &self.social_cache)
                    {
                        return FilterOutcome::Reject(reason);
                    }
                }
            }
        }

        if group.has_dev_filter() {
            if let FilterOutcome::Reject(reason) =
                dev_profile::check(group, mint, &self.dev_provider)
            {
                return FilterOutcome::Reject(reason);
            }
        }

        FilterOutcome::Pass
    }
}
