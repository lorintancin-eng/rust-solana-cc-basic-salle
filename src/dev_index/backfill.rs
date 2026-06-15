//! 48 小时历史回扫：拉 pump.fun 程序最近 48h 的 create 指令，写入 DevIndex。
//!
//! 算法：
//!   1. RPC `getSignaturesForAddress(pump_fun, before=None, limit=1000)` 分页向前翻
//!   2. 用 `block_time` 判断是否仍在 48h 窗口内；超出则停止
//!   3. 对每个 signature 调用 `getTransaction`
//!   4. 在 instructions 里查 pump.fun 程序的 create 指令（discriminator 8 bytes 比对）
//!   5. 解析后 `dev_index.record_creation(creator, mint)`
//!   6. 异步触发 metadata 抓取 → twitter
//!
//! 调用方：main.rs 启动后异步 spawn 一次，不阻塞主流程。
//! 节流：每 RPC 调用 sleep `RPC_DELAY_MS` 避免触发免费 RPC 限速。
//! 标记 `_meta:backfill_done` 防止重启重复回扫。

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use solana_client::rpc_client::RpcClient;
use solana_client::rpc_client::GetConfirmedSignaturesForAddress2Config;
use solana_client::rpc_config::RpcTransactionConfig;
use solana_sdk::commitment_config::CommitmentConfig;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::signature::Signature;
use solana_transaction_status::{
    EncodedConfirmedTransactionWithStatusMeta, EncodedTransaction, UiMessage, UiTransactionEncoding,
};
use tracing::{debug, info, warn};

use super::indexer::spawn_metadata_fetch;
use super::parser::{self, PUMP_FUN_PROGRAM_ID_STR};
use super::DevIndex;

const BACKFILL_WINDOW_SECS: i64 = 48 * 3600; // 48h
const PAGE_LIMIT: usize = 1000;
const RPC_DELAY_MS: u64 = 50; // ~20 req/s — well under most free tier limits
const MAX_PAGES: usize = 200; // 200 * 1000 = 200K signatures hard cap

/// 异步入口。启动一个 task 跑回扫，完成后写 `_meta:backfill_done`。
/// 已完成过的回扫调用此函数会立即返回（幂等）。
pub fn spawn_backfill(dev_index: Arc<DevIndex>, rpc: Arc<RpcClient>) {
    if dev_index.is_backfill_done() {
        info!("dev_index backfill already done, skipping");
        return;
    }
    tokio::spawn(async move {
        info!("dev_index backfill starting (window=48h)");
        match run_backfill(dev_index.clone(), rpc).await {
            Ok(stats) => {
                info!(
                    "dev_index backfill complete: pages={} sigs={} creates={}",
                    stats.pages, stats.signatures, stats.creates
                );
                if let Err(e) = dev_index.mark_backfill_done() {
                    warn!("mark_backfill_done failed: {}", e);
                }
            }
            Err(e) => {
                warn!("dev_index backfill failed: {}", e);
            }
        }
    });
}

#[derive(Default, Debug)]
struct BackfillStats {
    pages: usize,
    signatures: usize,
    creates: usize,
}

async fn run_backfill(dev_index: Arc<DevIndex>, rpc: Arc<RpcClient>) -> Result<BackfillStats> {
    let program_id = Pubkey::from_str(PUMP_FUN_PROGRAM_ID_STR).context("pump.fun program id")?;
    let cutoff_unix = chrono::Utc::now().timestamp() - BACKFILL_WINDOW_SECS;

    let mut stats = BackfillStats::default();
    let mut before: Option<Signature> = None;

    for page_idx in 0..MAX_PAGES {
        let sig_infos = {
            let rpc_c = rpc.clone();
            let program_id_c = program_id;
            let before_c = before;
            tokio::task::spawn_blocking(move || {
                let config = GetConfirmedSignaturesForAddress2Config {
                    before: before_c,
                    until: None,
                    limit: Some(PAGE_LIMIT),
                    commitment: Some(CommitmentConfig::confirmed()),
                };
                rpc_c.get_signatures_for_address_with_config(&program_id_c, config)
            })
            .await??
        };

        if sig_infos.is_empty() {
            debug!("backfill page {} empty, stopping", page_idx);
            break;
        }

        stats.pages += 1;

        // 时间 cutoff：检查最后一个 sig 是否仍在窗口内
        let last_block_time = sig_infos.last().and_then(|s| s.block_time);
        let mut hit_cutoff = false;

        for sig_info in &sig_infos {
            stats.signatures += 1;
            if let Some(t) = sig_info.block_time {
                if t < cutoff_unix {
                    hit_cutoff = true;
                    break;
                }
            }

            let sig = match Signature::from_str(&sig_info.signature) {
                Ok(s) => s,
                Err(_) => continue,
            };
            if sig_info.err.is_some() {
                continue;
            }

            // 节流
            tokio::time::sleep(Duration::from_millis(RPC_DELAY_MS)).await;

            let tx_result = {
                let rpc_c = rpc.clone();
                tokio::task::spawn_blocking(move || {
                    rpc_c.get_transaction_with_config(
                        &sig,
                        RpcTransactionConfig {
                            encoding: Some(UiTransactionEncoding::Json),
                            commitment: Some(CommitmentConfig::confirmed()),
                            max_supported_transaction_version: Some(0),
                        },
                    )
                })
                .await
            };

            let tx = match tx_result {
                Ok(Ok(tx)) => tx,
                Ok(Err(e)) => {
                    debug!("get_transaction {}: {}", &sig_info.signature[..8], e);
                    continue;
                }
                Err(e) => {
                    warn!("spawn_blocking join error: {}", e);
                    continue;
                }
            };

            if let Some(create_count) = process_tx(&dev_index, &rpc, &program_id, tx) {
                stats.creates += create_count;
            }
        }

        if hit_cutoff {
            info!("backfill reached 48h cutoff at page {}", page_idx);
            break;
        }

        // 设置下一页 before 游标
        before = sig_infos
            .last()
            .and_then(|s| Signature::from_str(&s.signature).ok());

        if last_block_time.is_none() {
            // 无 block_time 时保险措施：MAX_PAGES 已经限制了总数
            debug!("page {} has no block_time, continuing", page_idx);
        }
    }

    Ok(stats)
}

fn process_tx(
    dev_index: &Arc<DevIndex>,
    rpc: &Arc<RpcClient>,
    program_id: &Pubkey,
    tx: EncodedConfirmedTransactionWithStatusMeta,
) -> Option<usize> {
    let transaction = match tx.transaction.transaction {
        EncodedTransaction::Json(ui_tx) => ui_tx,
        _ => return None,
    };
    let message = transaction.message;
    let (account_keys_strs, instructions) = match message {
        UiMessage::Raw(raw) => (raw.account_keys, raw.instructions),
        UiMessage::Parsed(_) => return None, // skip parsed encoding
    };

    let account_keys: Vec<Pubkey> = account_keys_strs
        .iter()
        .filter_map(|s| Pubkey::from_str(s).ok())
        .collect();
    if account_keys.is_empty() {
        return None;
    }

    let program_idx_target = account_keys.iter().position(|k| k == program_id)?;

    let mut count = 0;
    for ix in &instructions {
        if ix.program_id_index as usize != program_idx_target {
            continue;
        }
        // ix.data is base58 encoded by default in Json raw encoding
        let raw_data = match bs58::decode(&ix.data).into_vec() {
            Ok(d) => d,
            Err(_) => continue,
        };
        let accs: Vec<Pubkey> = ix
            .accounts
            .iter()
            .filter_map(|i| account_keys.get(*i as usize).copied())
            .collect();

        if let Some(event) = parser::try_parse_create(&raw_data, &accs) {
            if let Err(e) = dev_index.record_creation(event.creator, event.mint) {
                debug!("backfill record_creation: {}", e);
                continue;
            }
            spawn_metadata_fetch(
                dev_index.clone(),
                rpc.clone(),
                event.mint,
                event.creator,
                event.uri,
            );
            count += 1;
        }
    }
    Some(count)
}
