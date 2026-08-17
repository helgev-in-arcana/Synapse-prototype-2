//! 宣言スイート（`SYN_DECL_SUITE`）。
//!
//! Blender 流: 状態からフル再宣言し、ホストが key で接続を整合させる。

use core::ffi::c_char;

use synapse_abi_core::{SynDeclBuilder, SynStatus, SynTypeId, SynValue};

/// fan-in: N 本のリンクを受け、順序付き配列で配送する入力ポート。
pub const SYN_PORT_MULTI: u32 = 1 << 0;

/// declare 内で呼ぶソケット宣言関数群。プラグインは確保せずこれらを呼ぶだけ。
/// key は配列インデックスではなく論理的同一性から導くこと（接続が壊れる）。
#[repr(C)]
pub struct SynDeclSuite {
    /// 出力ポート: 型はちょうど 1 つ。汎用ノードは SYN_TYPE_ANY を渡してよい（方式a）。
    pub output: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, key: *const c_char, label: *const c_char, ty: SynTypeId) -> SynStatus>,
    /// 入力ポート: types のいずれかを受理（多態）。SYN_TYPE_ANY で全許容。
    /// flags は SYN_PORT_*。
    pub input: Option<
        unsafe extern "C" fn(
            b: *mut SynDeclBuilder,
            key: *const c_char,
            label: *const c_char,
            types: *const SynTypeId,
            n_types: usize,
            flags: u32,
        ) -> SynStatus,
    >,
    /// 方式b（型伝播）: 接続中の入力の実体型を返す。declare 内で出力型の導出に使う。
    /// 未接続/不定なら SYN_TYPE_ANY。v1 は使わず ANY で書いてよい。
    pub connected_type: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, input_key: *const c_char, link_index: u32) -> SynTypeId>,
    /// 入力ソケットの初期デフォルト値（未接続時に get_input が返す値=パラメータ）。
    /// value は値渡し（SynValue 自体はコピー。大型データは value.ptr 経由の借用で、
    /// 呼び出し中のみ有効）。ホストが型 vtable で複製して保持する。
    /// 再 declare では key 一致ソケットの既存値を保持し、初期値は新規 key にのみ適用。
    pub input_default: Option<unsafe extern "C" fn(b: *mut SynDeclBuilder, key: *const c_char, value: SynValue) -> SynStatus>,
}

/// スイート ID: 宣言。
pub const SYN_DECL_SUITE: &str = "synapse:decl";
