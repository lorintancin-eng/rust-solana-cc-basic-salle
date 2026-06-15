//! pump.fun `create` 指令解析。
//!
//! Anchor 指令格式：
//!   [0..8]   discriminator (sighash of "global:create")
//!   [8..]    args: name (string), symbol (string), uri (string), creator (Pubkey)
//!
//! `create` 指令的 anchor discriminator = first 8 bytes of sha256("global:create")
//! 实际值: [24, 30, 200, 40, 5, 28, 7, 119]
//!
//! create 指令的账户列表（slot 顺序）：
//!   0  mint                (writable, signer)
//!   1  mint_authority      (writable)
//!   2  bonding_curve       (writable)
//!   3  associated_bonding  (writable)
//!   4  global              (read)
//!   5  mpl_token_metadata  (read)
//!   6  metadata            (writable)
//!   7  user (creator/dev)  (writable, signer)
//!   8  system_program      (read)
//!   9  token_program       (read)
//!   ...
//!
//! 我们只需要 mint (slot 0) + user/creator (slot 7)。
//! `creator` 字段在 args 里也有，但实际付钱者（signer）= slot 7 才是真正的"dev wallet"。

use solana_sdk::pubkey::Pubkey;

/// pump.fun program ID
pub const PUMP_FUN_PROGRAM_ID_STR: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

/// `create` 指令 anchor discriminator (sha256("global:create")[..8])
pub const CREATE_DISCRIMINATOR: [u8; 8] = [24, 30, 200, 40, 5, 28, 7, 119];

const MIN_CREATE_ACCOUNTS: usize = 8;
const MINT_SLOT: usize = 0;
const USER_SLOT: usize = 7;

#[derive(Debug, Clone)]
pub struct CreateEvent {
    pub mint: Pubkey,
    pub creator: Pubkey,
    /// metadata URI（可选，可后续异步抓取得到 twitter）
    pub uri: Option<String>,
    pub name: Option<String>,
}

/// 尝试解析一个 instruction（data + accounts）为 CreateEvent。
/// 不匹配（非 create 指令）时返回 None。
pub fn try_parse_create(data: &[u8], accounts: &[Pubkey]) -> Option<CreateEvent> {
    if data.len() < 8 {
        return None;
    }
    if data[..8] != CREATE_DISCRIMINATOR {
        return None;
    }
    if accounts.len() < MIN_CREATE_ACCOUNTS {
        return None;
    }

    let mint = accounts[MINT_SLOT];
    let creator = accounts[USER_SLOT];

    // 解析 args：name / symbol / uri 各是 borsh String (4-byte LE len + UTF-8 bytes)
    let mut cursor = &data[8..];
    let name = read_borsh_string(&mut cursor);
    let _symbol = read_borsh_string(&mut cursor);
    let uri = read_borsh_string(&mut cursor);

    Some(CreateEvent {
        mint,
        creator,
        uri,
        name,
    })
}

fn read_borsh_string(cursor: &mut &[u8]) -> Option<String> {
    if cursor.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes([cursor[0], cursor[1], cursor[2], cursor[3]]) as usize;
    if cursor.len() < 4 + len {
        return None;
    }
    let s = std::str::from_utf8(&cursor[4..4 + len]).ok()?.to_string();
    *cursor = &cursor[4 + len..];
    Some(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminator_mismatch_returns_none() {
        let data = vec![0u8; 16];
        let accounts = vec![Pubkey::default(); 10];
        assert!(try_parse_create(&data, &accounts).is_none());
    }

    #[test]
    fn short_data_returns_none() {
        assert!(try_parse_create(&[1, 2, 3], &[]).is_none());
    }

    #[test]
    fn missing_accounts_returns_none() {
        let mut data = CREATE_DISCRIMINATOR.to_vec();
        data.extend(&[0u8; 100]); // dummy args
        assert!(try_parse_create(&data, &[Pubkey::default()]).is_none());
    }

    #[test]
    fn parses_minimal_create() {
        let mut data = CREATE_DISCRIMINATOR.to_vec();
        // name=""
        data.extend_from_slice(&0u32.to_le_bytes());
        // symbol=""
        data.extend_from_slice(&0u32.to_le_bytes());
        // uri="https://x.io"
        let uri = "https://x.io";
        data.extend_from_slice(&(uri.len() as u32).to_le_bytes());
        data.extend_from_slice(uri.as_bytes());

        let accounts = vec![Pubkey::new_unique(); 10];
        let evt = try_parse_create(&data, &accounts).expect("parse ok");
        assert_eq!(evt.uri.as_deref(), Some("https://x.io"));
    }
}
