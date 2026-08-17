//! URID スイート（`SYN_URID_SUITE`）。
//!
//! ハンドル引数を取らないため、背後の状態は構造的にプロセスグローバルになる
//! （1プロセス=1ホスト=1セッション、ADR-023）。

use core::ffi::c_char;

use synapse_abi_core::SynUrid;

/// URI と URID の相互変換。
#[repr(C)]
pub struct SynUridSuite {
    /// URI を URID に写像（intern、セッション不変）。
    pub map: Option<unsafe extern "C" fn(uri: *const c_char) -> SynUrid>,
    /// URID から URI を借用（セッション中のみ有効）。
    pub unmap: Option<unsafe extern "C" fn(id: SynUrid) -> *const c_char>,
}

/// スイート ID: URID。
pub const SYN_URID_SUITE: &str = "synapse:urid";
