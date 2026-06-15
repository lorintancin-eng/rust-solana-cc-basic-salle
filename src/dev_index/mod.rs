//! Dev 画像本地索引（2ev 反向跟单策略条件 ③④⑤ 数据源）
//!
//! 三个数据维度：
//!   ③ open_count    —— dev 历史毕业（bonding curve complete=true）token 数
//!   ④ created_count —— dev 在 pump.fun 上总共创建 token 数
//!   ⑤ twitter_bound —— dev 推特账号绑定过的 token 数
//!
//! 索引来源：
//!   1. 实时增量：YellowStone gRPC 订阅 pump.fun 程序 tx stream，解析 create 指令
//!   2. 毕业事件：复用现有 AccountSubscriber 的 BondingCurve 更新流，detect complete 翻转
//!   3. Twitter：复用 filter/social.rs 的 Metaplex metadata + IPFS 抓取
//!   4. 48 小时历史回扫：启动时一次性，getSignaturesForAddress 分页 + getTransaction
//!
//! 数据持久化：sled，路径 `./dev_index/`
//! Schema（key 前缀分桶）：
//!   "dev:{pubkey}"          → DevStats (json)        — 主索引
//!   "mint:{pubkey}"         → creator pubkey (bytes) — mint 反查 creator
//!   "dev_tw:{pubkey}"       → twitter handle (utf-8) — dev 关联推特
//!   "tw_mints:{handle}"     → mint set (json)        — 推特 → 代币集合
//!   "_meta:cursor"          → last_processed_slot u64 (LE)
//!   "_meta:backfill_done"   → "1" if 48h backfill 完成

pub mod backfill;
pub mod indexer;
pub mod parser;

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sled::Db;
use solana_sdk::pubkey::Pubkey;
use tracing::{debug, info, warn};

/// dev 画像统计
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct DevStats {
    pub created_count: u32,
    pub open_count: u32, // 历史毕业（migrated）数
    pub twitter_bound: u32,
}

/// dev 索引（持久化到 sled，线程安全）
#[derive(Clone)]
pub struct DevIndex {
    db: Arc<Db>,
}

impl DevIndex {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = sled::Config::new()
            .path(path)
            .cache_capacity(64 * 1024 * 1024) // 64 MB
            .flush_every_ms(Some(2000))
            .open()
            .context("open sled db")?;
        info!("DevIndex opened: {} keys", db.len());
        Ok(Self { db: Arc::new(db) })
    }

    /// 查询 dev 画像。返回 None 表示从未见过该 dev。
    pub fn lookup(&self, dev: &Pubkey) -> Option<DevStats> {
        let key = dev_key(dev);
        match self.db.get(&key).ok()? {
            Some(bytes) => serde_json::from_slice(&bytes).ok(),
            None => None,
        }
    }

    /// `mint` 反查 creator 的原始 32 字节。返回 None 表示未见过该 mint 的 create。
    /// 上层调用方负责 `Pubkey::try_from(bytes.as_slice())`。
    pub fn lookup_creator_for_mint(&self, mint: &Pubkey) -> Option<Vec<u8>> {
        let key = mint_key(mint);
        match self.db.get(&key).ok()? {
            Some(bytes) => Some(bytes.to_vec()),
            None => None,
        }
    }

    /// 记录 dev 创建一个 token；同时建立 mint → creator 反查。
    /// `twitter` 可选，由 metadata 抓取异步填回（见 `link_twitter`）。
    pub fn record_creation(&self, dev: Pubkey, mint: Pubkey) -> Result<()> {
        // 1. mint → creator 反查
        self.db
            .insert(mint_key(&mint), dev.to_bytes().to_vec())
            .context("insert mint key")?;

        // 2. dev created_count++ （只在 mint 之前未记录时增加，避免重复）
        let mint_key_bytes = mint_key(&mint);
        let first_time = !self.db.contains_key(&mint_key_bytes).unwrap_or(false);
        if first_time {
            self.bump_dev(&dev, |s| s.created_count = s.created_count.saturating_add(1))?;
        } else {
            // 已经记录过的 mint，不重复累计 — record_creation 幂等
            debug!("record_creation idempotent: {}", &mint.to_string()[..12]);
        }

        Ok(())
    }

    /// 记录 mint 完成毕业（bonding curve complete=true）。
    /// 通过 mint 反查 creator，给 dev 加 open_count。
    /// 用 `migrated:{mint}` 标记防止重复加。
    pub fn record_migration(&self, mint: Pubkey) -> Result<()> {
        let migrated_marker = format!("migrated:{}", mint).into_bytes();
        if self.db.contains_key(&migrated_marker).unwrap_or(false) {
            return Ok(()); // 已记录
        }

        let creator_bytes = match self.db.get(mint_key(&mint)).ok().flatten() {
            Some(b) => b,
            None => {
                debug!(
                    "record_migration: creator unknown for {}, skipping (may be backfilled later)",
                    &mint.to_string()[..12]
                );
                return Ok(());
            }
        };
        let creator = Pubkey::try_from(creator_bytes.as_ref()).ok();
        let Some(creator) = creator else {
            return Ok(());
        };

        self.bump_dev(&creator, |s| {
            s.open_count = s.open_count.saturating_add(1)
        })?;
        self.db
            .insert(migrated_marker, b"1".as_slice())
            .context("mark migrated")?;

        info!(
            "DEV migration recorded: dev={} mint={}",
            &creator.to_string()[..12],
            &mint.to_string()[..12]
        );
        Ok(())
    }

    /// 为 dev 关联 twitter handle（来自 metadata JSON 抓取）。
    /// 同一 dev 多次调用以最后一次为准；多 token 共享同一 twitter 在 `tw_mints` 累计。
    pub fn link_twitter(&self, dev: Pubkey, mint: Pubkey, twitter: String) -> Result<()> {
        let twitter = normalize_twitter(&twitter);
        if twitter.is_empty() {
            return Ok(());
        }

        // 1. dev → twitter（覆盖）
        self.db
            .insert(dev_tw_key(&dev), twitter.as_bytes())
            .context("insert dev_tw")?;

        // 2. twitter → set of mints（去重累加）
        let tw_key = tw_mints_key(&twitter);
        let mut mints: Vec<String> = match self.db.get(&tw_key).ok().flatten() {
            Some(b) => serde_json::from_slice(&b).unwrap_or_default(),
            None => Vec::new(),
        };
        let mint_str = mint.to_string();
        let was_new = !mints.contains(&mint_str);
        if was_new {
            mints.push(mint_str);
            self.db
                .insert(&tw_key, serde_json::to_vec(&mints)?)
                .context("insert tw_mints")?;
        }

        // 3. dev.twitter_bound = count of mints for dev's twitter
        let bound_count = mints.len() as u32;
        self.bump_dev(&dev, |s| s.twitter_bound = bound_count)?;

        Ok(())
    }

    fn bump_dev<F>(&self, dev: &Pubkey, mutator: F) -> Result<()>
    where
        F: Fn(&mut DevStats),
    {
        let key = dev_key(dev);
        // sled `update_and_fetch` closure is FnMut and may be called multiple times
        // under contention, so `mutator` must be Fn (callable repeatedly).
        let updated = self.db.update_and_fetch(&key, |old| {
            let mut stats: DevStats = old
                .and_then(|b| serde_json::from_slice(b).ok())
                .unwrap_or_default();
            mutator(&mut stats);
            serde_json::to_vec(&stats).ok()
        })?;
        if updated.is_none() {
            warn!("bump_dev: write returned None for {}", dev);
        }
        Ok(())
    }

    pub fn is_backfill_done(&self) -> bool {
        self.db
            .get(b"_meta:backfill_done")
            .ok()
            .flatten()
            .is_some()
    }

    pub fn mark_backfill_done(&self) -> Result<()> {
        self.db
            .insert(b"_meta:backfill_done", b"1".as_slice())
            .context("mark backfill done")?;
        self.db.flush().context("flush")?;
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush().context("flush")?;
        Ok(())
    }

    pub fn total_devs(&self) -> usize {
        self.db
            .iter()
            .keys()
            .filter_map(|k| k.ok())
            .filter(|k| k.starts_with(b"dev:"))
            .count()
    }
}

fn dev_key(dev: &Pubkey) -> Vec<u8> {
    let mut k = b"dev:".to_vec();
    k.extend_from_slice(dev.as_ref());
    k
}

fn mint_key(mint: &Pubkey) -> Vec<u8> {
    let mut k = b"mint:".to_vec();
    k.extend_from_slice(mint.as_ref());
    k
}

fn dev_tw_key(dev: &Pubkey) -> Vec<u8> {
    let mut k = b"dev_tw:".to_vec();
    k.extend_from_slice(dev.as_ref());
    k
}

fn tw_mints_key(twitter: &str) -> Vec<u8> {
    let mut k = b"tw_mints:".to_vec();
    k.extend_from_slice(twitter.as_bytes());
    k
}

fn normalize_twitter(raw: &str) -> String {
    let s = raw.trim();
    // 去掉 @ 前缀和 https://twitter.com/ 等
    let s = s.strip_prefix('@').unwrap_or(s);
    let s = s
        .strip_prefix("https://twitter.com/")
        .or_else(|| s.strip_prefix("http://twitter.com/"))
        .or_else(|| s.strip_prefix("https://x.com/"))
        .or_else(|| s.strip_prefix("http://x.com/"))
        .unwrap_or(s);
    s.split('/').next().unwrap_or(s).to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_twitter_handles_prefixes() {
        assert_eq!(normalize_twitter("@FooBar"), "foobar");
        assert_eq!(
            normalize_twitter("https://twitter.com/FooBar/status/123"),
            "foobar"
        );
        assert_eq!(normalize_twitter("https://x.com/foobar"), "foobar");
        assert_eq!(normalize_twitter("  foobar  "), "foobar");
    }
}
