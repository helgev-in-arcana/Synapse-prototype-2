//! 型レジストリスイート（`SYN_TYPE_REGISTRY_SUITE`）。
//!
//! `SynTypeVTable` は `SynValue::payload` の所有権規約（誰が clone/drop するか、確保時の
//! アラインメント保証 ADR-029）を定めるが、`SynValue` 側はこれを型として参照しない。
//! 結合はドキュメント上のみで、依存方向は suite → core の一方向。

use core::ffi::{c_char, c_void};

use synapse_abi_core::{SynStatus, SynTypeId};

/// memcpy/シリアライズ可能な素のバイト列。
pub const SYN_TYPE_PLAIN_BYTES: u32 = 1 << 0;
/// ptr は不透明ハンドル。get_api で操作する。
pub const SYN_TYPE_OPAQUE: u32 = 1 << 1;
/// clone は参照カウント増加（浅いコピー）。
pub const SYN_TYPE_SHARED: u32 = 1 << 2;

/// 型ごとの操作テーブル。メモリ確保/解放はホスト。型は構築/破棄/複製のみ知る。
#[repr(C)]
pub struct SynTypeVTable {
    /// SYN_TYPE_* フラグ。
    pub flags: u32,
    /// 固定サイズ。可変なら 0。
    pub size: usize,
    /// アラインメント要件（2の冪。register_type が検証する）。可変サイズ型では
    /// ホスト確保バッファ先頭のアラインメントとして解釈する（ADR-029）。
    pub align: usize,

    /// 既定値を dst に構築する。
    pub init: Option<unsafe extern "C" fn(dst: *mut c_void, t: SynTypeId) -> SynStatus>,
    /// 複製。PLAIN/可変型はディープコピー。SHARED/OPAQUE は refcount++ でよい
    /// （passthrough で大きな/GPU リソースを複製しないために重要）。
    pub clone: Option<unsafe extern "C" fn(dst: *mut c_void, src: *const c_void, t: SynTypeId) -> SynStatus>,
    /// 破棄のみ（free はホスト）。SHARED は refcount--、0 で実リソース解放。
    pub drop: Option<unsafe extern "C" fn(obj: *mut c_void, t: SynTypeId)>,

    /// 永続化（PLAIN のみ）。out=NULL/cap=0 で必要サイズを written に返す。OPAQUE は NULL 可。
    pub serialize:
        Option<unsafe extern "C" fn(obj: *const c_void, t: SynTypeId, out: *mut c_void, cap: usize, written: *mut usize) -> SynStatus>,
    /// 復元（PLAIN のみ）。OPAQUE は NULL 可。
    pub deserialize:
        Option<unsafe extern "C" fn(dst: *mut c_void, t: SynTypeId, input: *const c_void, len: usize) -> SynStatus>,

    /// 型+API 二層: opaque 型が自前の API テーブルを公開する（例 "synapse:gpu:texture"）。
    /// PLAIN 型は NULL。
    pub get_api: Option<unsafe extern "C" fn(t: SynTypeId, api_id: *const c_char) -> *const c_void>,
}

/// 型の登録・解決。
#[repr(C)]
pub struct SynTypeRegistrySuite {
    /// 型を URI と vtable で登録する。vt->align が 2 の冪でなければ SYN_ERR_BAD_ARG（ADR-029）。
    pub register_type: Option<unsafe extern "C" fn(uri: *const c_char, vt: *const SynTypeVTable) -> SynStatus>,
    /// 型 ID から vtable を解決する（結果はセッション中キャッシュ可）。
    pub lookup: Option<unsafe extern "C" fn(t: SynTypeId) -> *const SynTypeVTable>,
    /// URI から型 ID を得る。
    pub type_of: Option<unsafe extern "C" fn(uri: *const c_char) -> SynTypeId>,
}

/// スイート ID: 型レジストリ。
pub const SYN_TYPE_REGISTRY_SUITE: &str = "synapse:type-registry";
