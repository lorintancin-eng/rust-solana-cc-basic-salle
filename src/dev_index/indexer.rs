//! 实时增量 dev 索引器。
//!
//! 启动一个独立 task：
//!   1. 从 trade_rx 派生的"全量 pump.fun trade 信号"中抽取 create 指令
//!   2. 调用 DevIndex.record_creation(creator, mint)
//!   3. 异步触发 metadata 抓取 → DevIndex.link_twitter
//!
//! 当前实现复用 main.rs 现有的 trade_rx（已订阅 pump.fun program）。
//! 注意：现有 subscriber 只发送匹配 target_wallets 的 trade —— 对 create 来说
//! 这意味着我们**只能索引被跟单组监听的 dev 钱包创建的 token**。
//!
//! TODO: 想索引全网所有 pump.fun create，需要新增一个独立 gRPC 订阅 by program
//! 不带 wallet filter。当前 Phase 1 先用现有流；上线后看实际数据覆盖率再决定。

use std::sync::Arc;
use std::time::Duration;

use solana_client::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use tracing::{debug, info, warn};

use super::parser;
use super::DevIndex;
use crate::filter::metadata as filter_metadata;

/// 启动 metadata 抓取任务：拉 mint 的 Metaplex metadata → 解析 JSON → 提取 twitter
/// 失败不阻塞、不重试。
pub fn spawn_metadata_fetch(
    dev_index: Arc<DevIndex>,
    rpc: Arc<RpcClient>,
    mint: Pubkey,
    creator: Pubkey,
    uri_hint: Option<String>,
) {
    tokio::spawn(async move {
        // 优先用 create 指令里提供的 uri；缺失则从 metadata account 反查
        let uri = match uri_hint {
            Some(u) if !u.trim().is_empty() => u,
            _ => {
                let pda = filter_metadata::derive_metadata_pda(&mint);
                let rpc_for_blocking = rpc.clone();
                match tokio::task::spawn_blocking(move || rpc_for_blocking.get_account_data(&pda))
                    .await
                {
                    Ok(Ok(data)) => match filter_metadata::extract_uri(&data) {
                        Some(u) => u,
                        None => {
                            debug!(
                                "metadata uri missing for {}",
                                &mint.to_string()[..12]
                            );
                            return;
                        }
                    },
                    _ => {
                        debug!("metadata fetch failed for {}", &mint.to_string()[..12]);
                        return;
                    }
                }
            }
        };

        let url = resolve_uri(&uri);
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let json: serde_json::Value = match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(j) => j,
                Err(e) => {
                    debug!("metadata json parse {}: {}", &mint.to_string()[..12], e);
                    return;
                }
            },
            Ok(r) => {
                debug!(
                    "metadata http {} for {}",
                    r.status(),
                    &mint.to_string()[..12]
                );
                return;
            }
            Err(e) => {
                debug!("metadata fetch http {}: {}", &mint.to_string()[..12], e);
                return;
            }
        };

        let twitter = extract_twitter(&json);
        if let Some(handle) = twitter {
            if let Err(e) = dev_index.link_twitter(creator, mint, handle) {
                warn!("link_twitter failed: {}", e);
            }
        }
    });
}

fn extract_twitter(json: &serde_json::Value) -> Option<String> {
    let candidates = [
        json.get("twitter"),
        json.get("x"),
        json.get("extensions")
            .and_then(|e| e.get("twitter")),
        json.get("extensions").and_then(|e| e.get("x")),
    ];
    for c in candidates {
        if let Some(s) = c.and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return Some(t.to_string());
            }
        }
    }
    None
}

fn resolve_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("ipfs://") {
        return format!("https://ipfs.io/ipfs/{}", rest);
    }
    if let Some(rest) = trimmed.strip_prefix("ar://") {
        return format!("https://arweave.net/{}", rest);
    }
    trimmed.to_string()
}

/// 处理一个解析出的 create event：写入 DevIndex + 触发异步 metadata 抓取
pub fn handle_create(
    dev_index: &Arc<DevIndex>,
    rpc: &Arc<RpcClient>,
    event: parser::CreateEvent,
) {
    if let Err(e) = dev_index.record_creation(event.creator, event.mint) {
        warn!("record_creation failed: {}", e);
        return;
    }
    info!(
        "DEV create indexed: dev={} mint={} name={:?}",
        &event.creator.to_string()[..12],
        &event.mint.to_string()[..12],
        event.name
    );
    spawn_metadata_fetch(
        dev_index.clone(),
        rpc.clone(),
        event.mint,
        event.creator,
        event.uri,
    );
}
