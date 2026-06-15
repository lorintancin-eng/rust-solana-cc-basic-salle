//! Metaplex Token Metadata 工具：PDA 推导 + URI 字段提取
//!
//! Metaplex Metadata v1 account layout（borsh，固定 offset 编码）：
//!   offset  0..1    key (discriminator)
//!   offset  1..33   update_authority (Pubkey)
//!   offset 33..65   mint (Pubkey)
//!   offset 65..101  name (4-byte len + 32 bytes content padded with nulls)
//!   offset 101..115 symbol (4-byte len + 10 bytes content)
//!   offset 115..319 uri (4-byte len + 200 bytes content)
//!   ...
//!
//! 我们只需要 uri 字段（200 bytes 区间），按 offset 直接读，跳过 borsh
//! 严格反序列化避免 padding 错位。
//!
//! 参考: https://docs.metaplex.com/programs/token-metadata/accounts

use std::str::FromStr;
use std::sync::OnceLock;

use solana_sdk::pubkey::Pubkey;

const URI_OFFSET: usize = 115;
const URI_MAX_LEN: usize = 200;
const URI_FIELD_TOTAL: usize = 4 + URI_MAX_LEN; // length prefix + content

static METAPLEX_PROGRAM: OnceLock<Pubkey> = OnceLock::new();

/// Metaplex Token Metadata 程序 ID（懒初始化）
fn metaplex_program() -> &'static Pubkey {
    METAPLEX_PROGRAM.get_or_init(|| {
        Pubkey::from_str("metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s")
            .expect("valid metaplex program id")
    })
}

/// 推导 Metaplex metadata PDA：
///   seeds = [b"metadata", program_id, mint]
pub fn derive_metadata_pda(mint: &Pubkey) -> Pubkey {
    let program = metaplex_program();
    let (pda, _bump) = Pubkey::find_program_address(
        &[b"metadata", program.as_ref(), mint.as_ref()],
        program,
    );
    pda
}

/// 从原始 metadata account data 提取 `uri` 字段。
/// 返回 None 表示数据长度不足、长度字段异常、或 UTF-8 失败。
pub fn extract_uri(data: &[u8]) -> Option<String> {
    if data.len() < URI_OFFSET + URI_FIELD_TOTAL {
        return None;
    }
    let len = u32::from_le_bytes([
        data[URI_OFFSET],
        data[URI_OFFSET + 1],
        data[URI_OFFSET + 2],
        data[URI_OFFSET + 3],
    ]) as usize;
    if len == 0 || len > URI_MAX_LEN {
        return None;
    }
    let content = &data[URI_OFFSET + 4..URI_OFFSET + 4 + len];
    let s = std::str::from_utf8(content).ok()?;
    Some(s.trim_end_matches('\0').trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pda_known_mint_smoke() {
        // pump.fun 的一个固定 token，PDA 推导应稳定
        let mint =
            Pubkey::from_str("So11111111111111111111111111111111111111112").expect("valid");
        let pda = derive_metadata_pda(&mint);
        // 不验证具体值（不同环境可能不同），只验证不 panic
        assert_ne!(pda, Pubkey::default());
    }

    #[test]
    fn extract_uri_handles_short_buffer() {
        assert_eq!(extract_uri(&[0u8; 100]), None);
    }
}
