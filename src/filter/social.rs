//! 过滤条件②：要求 token 至少有一个社交链接（Twitter / Telegram / Website）
//!
//! 流程：
//!   1. `derive_metadata_pda(mint)` 推导 Metaplex metadata account
//!   2. RPC `get_account_data` 拉取（spawn_blocking 包裹同步 client）
//!   3. `extract_uri` 解析出 URI（一般是 IPFS / Arweave）
//!   4. HTTP GET URI（IPFS gateway 处理），解析 JSON 中 twitter/telegram/website/x 字段
//!   5. 任意一个非空字符串 → has_social=true
//!   6. 结果写入 `SocialCache`（TTL 24h）
//!
//! 调度策略：
//!   - filter::evaluate 同步调用 `check(...)`，读缓存
//!     - 命中 true → Pass
//!     - 命中 false → Reject
//!     - **未命中 → Pass（默认放行）** + 触发 `spawn_prefetch`
//!   - spawn_prefetch 是异步、不阻塞抢入路径
//!
//! 局限性：第一次见到的 mint 会"放行后再抓取"，等同区块抢入后才知道结果。
//! 反向跟单场景下同一 mint 极少出现第二次，所以缓存对实测帮助有限。
//! 后续可演进为"先买后过滤主动卖出"。

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use tracing::debug;

use super::metadata;
use super::FilterOutcome;
use crate::groups::CopyGroup;

const CACHE_TTL: Duration = Duration::from_secs(24 * 3600);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const IPFS_GATEWAY: &str = "https://ipfs.io/ipfs/";
const ARWEAVE_GATEWAY: &str = "https://arweave.net/";

#[derive(Clone, Debug)]
struct SocialEntry {
    has_social: bool,
    cached_at: Instant,
}

/// 社交链接抓取结果缓存
#[derive(Clone, Default)]
pub struct SocialCache {
    inner: Arc<DashMap<Pubkey, SocialEntry>>,
}

impl SocialCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, mint: &Pubkey) -> Option<bool> {
        let entry = self.inner.get(mint)?;
        if entry.cached_at.elapsed() < CACHE_TTL {
            Some(entry.has_social)
        } else {
            None
        }
    }

    fn put(&self, mint: Pubkey, has_social: bool) {
        self.inner.insert(
            mint,
            SocialEntry {
                has_social,
                cached_at: Instant::now(),
            },
        );
    }
}

/// 同步过滤检查：仅读缓存，不阻塞主路径
pub fn check(group: &CopyGroup, mint: &Pubkey, cache: &SocialCache) -> FilterOutcome {
    if !group.require_social_link {
        return FilterOutcome::Pass;
    }
    match cache.get(mint) {
        Some(true) => FilterOutcome::Pass,
        Some(false) => FilterOutcome::Reject("no social link in metadata".to_string()),
        None => FilterOutcome::Pass, // 缓存未到位 → 放行（等异步预热）
    }
}

/// 异步预热：spawn 后台任务抓取 metadata 并写入缓存
pub fn spawn_prefetch(mint: Pubkey, rpc: Arc<RpcClient>, cache: SocialCache) {
    if cache.get(&mint).is_some() {
        return;
    }
    tokio::spawn(async move {
        match fetch_social_status(mint, rpc).await {
            Ok(has_social) => {
                debug!(
                    "Social prefetch [{}]: has_social={}",
                    &mint.to_string()[..12],
                    has_social
                );
                cache.put(mint, has_social);
            }
            Err(err) => {
                debug!(
                    "Social prefetch failed [{}]: {}",
                    &mint.to_string()[..12],
                    err
                );
                // 失败不缓存，下次再试（避免临时故障误锁）
            }
        }
    });
}

async fn fetch_social_status(mint: Pubkey, rpc: Arc<RpcClient>) -> anyhow::Result<bool> {
    let pda = metadata::derive_metadata_pda(&mint);

    // 同步 RPC client → spawn_blocking
    let account_data = tokio::task::spawn_blocking(move || rpc.get_account_data(&pda))
        .await
        .map_err(|e| anyhow::anyhow!("spawn_blocking join: {}", e))?
        .map_err(|e| anyhow::anyhow!("rpc get_account_data: {}", e))?;

    let uri = metadata::extract_uri(&account_data)
        .ok_or_else(|| anyhow::anyhow!("metadata uri not found"))?;

    let json_url = resolve_uri(&uri);
    let http = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| anyhow::anyhow!("http client build: {}", e))?;

    let resp = http
        .get(&json_url)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("http get: {}", e))?;
    if !resp.status().is_success() {
        anyhow::bail!("http status {}", resp.status());
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| anyhow::anyhow!("json parse: {}", e))?;

    let has_social = has_nonempty_str(&json, "twitter")
        || has_nonempty_str(&json, "telegram")
        || has_nonempty_str(&json, "website")
        || has_nonempty_str(&json, "x")
        || has_nonempty_str(&json, "discord")
        // pump.fun 元数据常用 "metadata.extensions.twitter" 等嵌套，但常规 token 直接放顶层
        || nested_has_nonempty(&json, &["extensions", "twitter"])
        || nested_has_nonempty(&json, &["extensions", "telegram"])
        || nested_has_nonempty(&json, &["extensions", "website"]);

    Ok(has_social)
}

fn has_nonempty_str(json: &serde_json::Value, key: &str) -> bool {
    json.get(key)
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn nested_has_nonempty(json: &serde_json::Value, path: &[&str]) -> bool {
    let mut current = json;
    for key in path {
        match current.get(*key) {
            Some(v) => current = v,
            None => return false,
        }
    }
    current
        .as_str()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
}

fn resolve_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        return format!("{}{}", IPFS_GATEWAY, rest);
    }
    if let Some(rest) = trimmed.strip_prefix("ar://") {
        return format!("{}{}", ARWEAVE_GATEWAY, rest);
    }
    trimmed.to_string()
}
