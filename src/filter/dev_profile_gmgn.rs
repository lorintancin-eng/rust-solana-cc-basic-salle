//! GMGN OpenAPI dev profile provider.
//!
//! Strategy condition ③④⑤ data source, queried by **mint** (we don't have
//! `mint → creator` reverse map locally; GMGN gives it).
//!
//! ## Non-blocking design (抢入路径优先)
//!
//! `lookup_by_mint` is **sync** and **never blocks**:
//!   - cache hit  → return cached stats (or cached None)
//!   - cache miss → spawn background fetch task, return `None` immediately
//!
//! "Return None on miss" means filter falls through to Pass for that trade.
//! Subsequent trades of the same mint (or anything created by same dev) hit
//! the populated cache and get filtered properly. Trading off one "leaked"
//! first trade per cold mint for zero impact on the same-block抢入 latency.
//!
//! ## API flow (cache miss)
//!
//!   1. GET /v1/token/info?address={mint}        → dev.creator_address + ③⑤
//!   2. GET /v1/user/created_tokens?wallet=dev   → ④
//!
//! ## Auth
//!
//! X-APIKEY header + `timestamp` (unix s) + UUID `client_id` query params
//! (server validates replay window ~7s). No request signing — read endpoints
//! only need the API key.
//!
//! ## Rate limit
//!
//! GMGN free tier: 1 req/s globally. We serialize via `Semaphore(1)` plus
//! a min interval of 1500ms between calls. 1h positive cache + 60s negative
//! cache keep the call rate well below budget under normal trade flow.

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use dashmap::DashMap;
use reqwest::Client;
use serde_json::Value;
use solana_sdk::pubkey::Pubkey;
use std::str::FromStr;
use tokio::sync::Semaphore;
use tracing::{debug, warn};

use super::dev_profile::DevStats;

const HOST: &str = "https://openapi.gmgn.ai";
const ENDPOINT_TOKEN_INFO: &str = "/v1/token/info";
const ENDPOINT_CREATED_TOKENS: &str = "/v1/user/created_tokens";

/// Positive cache TTL — dev stats change slowly
const CACHE_TTL: Duration = Duration::from_secs(3600);
/// Negative cache TTL — short, retry transient failures sooner
const NEG_TTL: Duration = Duration::from_secs(60);
/// HTTP timeout per call
const HTTP_TIMEOUT: Duration = Duration::from_secs(5);
/// Min interval between GMGN calls (rate limit safety; default GMGN cap = 1 req/s)
const MIN_INTERVAL_MS: u64 = 1500;

#[derive(Debug, Clone, Copy)]
struct Cached {
    stats: Option<DevStats>,
    /// 同一次 token/info 调用顺带得到的 social link 状态（条件 ②）。
    /// None 表示这次抓取没拿到任何 social 数据（也作为「无社交」处理 —— 与
    /// `has_social=false` 区分仅为日志诊断）。
    has_social: Option<bool>,
    inserted_at: Instant,
}

impl Cached {
    fn expired(&self) -> bool {
        // 任一字段有数据就算正向缓存（TTL 长），全 None 才走短 TTL
        let any_data = self.stats.is_some() || self.has_social.is_some();
        let ttl = if any_data { CACHE_TTL } else { NEG_TTL };
        self.inserted_at.elapsed() >= ttl
    }
}

#[derive(Clone)]
pub struct GmgnProvider {
    api_key: String,
    http: Client,
    /// mint → DevStats cache (avoid re-fetching for repeated trades of same mint)
    mint_cache: Arc<DashMap<Pubkey, Cached>>,
    /// dev → DevStats cache (shared when multiple mints same creator)
    dev_cache: Arc<DashMap<Pubkey, Cached>>,
    /// Coalesce concurrent fetches for the same mint
    in_flight: Arc<DashMap<Pubkey, ()>>,
    /// Serialize HTTP calls so we never exceed GMGN's 1 req/s
    sem: Arc<Semaphore>,
}

impl GmgnProvider {
    pub fn new(api_key: String) -> Result<Self> {
        let http = Client::builder()
            .timeout(HTTP_TIMEOUT)
            .user_agent("solana-copy-trader/1.8")
            .build()?;
        Ok(Self {
            api_key,
            http,
            mint_cache: Arc::new(DashMap::new()),
            dev_cache: Arc::new(DashMap::new()),
            in_flight: Arc::new(DashMap::new()),
            sem: Arc::new(Semaphore::new(1)),
        })
    }

    /// **Sync, non-blocking** lookup for dev profile (conditions ③④⑤).
    /// Returns cached stats if available; otherwise schedules background fetch
    /// and returns `None` (filter falls through to Pass for this trade).
    pub fn lookup_by_mint(&self, mint: &Pubkey) -> Option<DevStats> {
        if let Some(entry) = self.mint_cache.get(mint) {
            if !entry.expired() {
                return entry.stats;
            }
        }

        // Cache miss / expired → schedule async fetch, return None now.
        self.spawn_fetch(*mint);
        None
    }

    /// **Sync, non-blocking** lookup for social link presence (condition ②).
    /// Same cache as dev profile — one token/info call serves both filters.
    /// Returns None on cache miss (filter Pass; fetch scheduled).
    pub fn lookup_social_by_mint(&self, mint: &Pubkey) -> Option<bool> {
        if let Some(entry) = self.mint_cache.get(mint) {
            if !entry.expired() {
                return entry.has_social;
            }
        }
        self.spawn_fetch(*mint);
        None
    }

    /// Spawn a background fetch unless one is already in flight for this mint.
    /// Must be called from inside a Tokio runtime (the trade loop is async).
    fn spawn_fetch(&self, mint: Pubkey) {
        // Coalesce: skip if another fetch is already in flight for this mint
        if self.in_flight.insert(mint, ()).is_some() {
            return;
        }

        let this = self.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let (stats, has_social) = this.fetch(&mint).await;
                    this.mint_cache.insert(
                        mint,
                        Cached {
                            stats,
                            has_social,
                            inserted_at: Instant::now(),
                        },
                    );
                    this.in_flight.remove(&mint);
                });
            }
            Err(_) => {
                // Not in a runtime — clear in_flight slot so future attempts can retry
                self.in_flight.remove(&mint);
                warn!("GmgnProvider::spawn_fetch called outside tokio runtime");
            }
        }
    }

    /// Returns (dev_stats, has_social) — one token/info call powers both.
    /// dev_stats may need a follow-up created_tokens call for condition ④.
    async fn fetch(&self, mint: &Pubkey) -> (Option<DevStats>, Option<bool>) {
        // Step 1: token/info → creator_address + dev hints + link section
        let token_info = match self.call_token_info(mint).await {
            Some(v) => v,
            None => return (None, None),
        };
        let data = match token_info.get("data") {
            Some(d) => d,
            None => return (None, None),
        };

        // 解析 link 段计算 has_social（条件 ②）
        let has_social = compute_has_social(data.get("link"));

        let dev_obj = match data.get("dev") {
            Some(d) => d,
            None => return (None, has_social),
        };
        let creator_str = match dev_obj.get("creator_address").and_then(|v| v.as_str()) {
            Some(s) if !s.is_empty() => s,
            _ => return (None, has_social),
        };
        let creator = match Pubkey::from_str(creator_str) {
            Ok(p) => p,
            Err(_) => return (None, has_social),
        };

        let creator_open_hint = dev_obj
            .get("creator_open_count")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32);
        let twitter_create_count = dev_obj
            .get("twitter_create_token_count")
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
            .unwrap_or(0);

        // Step 2: check dev_cache first to skip the second call when possible
        if let Some(entry) = self.dev_cache.get(&creator) {
            if !entry.expired() {
                if let Some(mut stats) = entry.stats {
                    stats.twitter_bound = twitter_create_count;
                    return (Some(stats), has_social);
                }
            }
        }

        // Step 3: created_tokens for the dev
        let created = self.call_created_tokens(&creator).await;
        let stats = match created.as_ref().and_then(|j| j.get("data")) {
            Some(data) => {
                let inner = data.get("inner_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let open = data.get("open_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let open_effective = open.max(creator_open_hint.unwrap_or(0));
                DevStats {
                    open_count: open_effective,
                    created_count: inner + open,
                    twitter_bound: twitter_create_count,
                }
            }
            None => DevStats {
                open_count: creator_open_hint.unwrap_or(0),
                created_count: creator_open_hint.unwrap_or(0),
                twitter_bound: twitter_create_count,
            },
        };

        self.dev_cache.insert(
            creator,
            Cached {
                stats: Some(stats),
                has_social: None, // dev_cache 不存 social；按 mint 维度查
                inserted_at: Instant::now(),
            },
        );

        debug!(
            "GMGN dev profile: mint={} creator={} open={} created={} tw={} has_social={:?}",
            &mint.to_string()[..12],
            &creator.to_string()[..12],
            stats.open_count,
            stats.created_count,
            stats.twitter_bound,
            has_social,
        );
        (Some(stats), has_social)
    }

    async fn call_token_info(&self, mint: &Pubkey) -> Option<Value> {
        let url = format!(
            "{}{}?chain=sol&address={}&timestamp={}&client_id={}",
            HOST,
            ENDPOINT_TOKEN_INFO,
            mint,
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4()
        );
        self.do_get(&url, "token/info", mint).await
    }

    async fn call_created_tokens(&self, dev: &Pubkey) -> Option<Value> {
        let url = format!(
            "{}{}?chain=sol&wallet_address={}&timestamp={}&client_id={}",
            HOST,
            ENDPOINT_CREATED_TOKENS,
            dev,
            chrono::Utc::now().timestamp(),
            uuid::Uuid::new_v4()
        );
        self.do_get(&url, "created_tokens", dev).await
    }

    /// Rate-limited HTTP GET, returns parsed JSON Value when `code == 0`.
    async fn do_get(&self, url: &str, endpoint_name: &str, ref_key: &Pubkey) -> Option<Value> {
        let _permit = self.sem.acquire().await.ok()?;
        // Throttle even on cache miss bursts
        tokio::time::sleep(Duration::from_millis(MIN_INTERVAL_MS)).await;

        let resp = match self
            .http
            .get(url)
            .header("X-APIKEY", &self.api_key)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    "GMGN {} HTTP error for {}: {}",
                    endpoint_name,
                    &ref_key.to_string()[..12],
                    e
                );
                return None;
            }
        };

        let status = resp.status();
        if !status.is_success() {
            warn!(
                "GMGN {} HTTP {} for {}",
                endpoint_name,
                status,
                &ref_key.to_string()[..12]
            );
            return None;
        }

        let json: Value = match resp.json().await {
            Ok(j) => j,
            Err(e) => {
                warn!(
                    "GMGN {} JSON parse for {}: {}",
                    endpoint_name,
                    &ref_key.to_string()[..12],
                    e
                );
                return None;
            }
        };

        let code = json.get("code").and_then(|v| v.as_i64()).unwrap_or(-1);
        if code != 0 {
            let msg = json
                .get("message")
                .or_else(|| json.get("error"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            warn!(
                "GMGN {} code={} for {} msg={}",
                endpoint_name,
                code,
                &ref_key.to_string()[..12],
                msg
            );
            return None;
        }

        Some(json)
    }
}

/// 从 GMGN `/v1/token/info` 响应的 `link` 字段判断是否至少有一个非空社交链接。
/// 字段命名按 GMGN OpenAPI（`twitter_username`、`website`、`telegram`、`discord`）。
/// 注：`twitter_username` 偶尔会是 tweet URL（如 "user/status/123"）—— 仍视为
/// 有效社交（用户挂了引用推文也是宣传痕迹）。返回：
///   - Some(true)  至少一个非空
///   - Some(false) link 对象存在但全部空
///   - None        link 字段缺失（不算"无社交"，留给上层默认 Pass）
fn compute_has_social(link: Option<&Value>) -> Option<bool> {
    let link = link?;
    let keys = [
        "twitter_username",
        "website",
        "telegram",
        "discord",
    ];
    let any = keys.iter().any(|k| {
        link.get(*k)
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
    });
    Some(any)
}
