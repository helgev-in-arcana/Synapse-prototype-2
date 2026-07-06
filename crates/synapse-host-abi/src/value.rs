//! 値の所有モデル（`SynValue` の値渡し ⇄ ホスト所有バイト列）。
//!
//! # 意図
//! ABI 上の値は [`SynValue`] による**値渡し**（ADR-022）で、≤16byte（`SYN_VALUE_INLINE`）の
//! 小さな値は payload へインライン格納する SVO（Small Value Optimization）、それを超える
//! 大型値は別領域を指す。本モジュールの [`OwnedValue`] は、この二様式を**ホスト所有の連続
//! バイト列**へ正規化して隠す。プラグインへ渡すとき／受け取るときの境界変換をここに閉じ込める。
//!
//! # 使い方
//! - 本体ホストは [`OwnedValue::from_plain_bytes`] / [`OwnedValue::empty`] で値を組み立て、
//!   [`OwnedValue::bytes`] / [`OwnedValue::type_id`] で中身を読む。
//! - ABI 境界変換（[`OwnedValue::from_value`] / [`OwnedValue::to_value`]）はクレート内部専用。

use core::ffi::c_void;

use synapse_abi::{SynTypeId, SynValue, SynValuePayload, SYN_URID_INVALID};

use crate::ffi::INLINE_SIZE;

/// ホスト所有の値。
///
/// SVO（≤16byte）でも大型でも、内部表現は統一して「先頭から `size` バイトに実体を持つ
/// バイト列」とする。`bytes` の確保長は `size.max(INLINE_SIZE)`（SVO 経路で `to_value` が
/// インライン幅ぶん読み出せるようにするためのパディング）。
#[derive(Clone)]
pub struct OwnedValue {
    pub(crate) type_id: SynTypeId,
    pub(crate) size: usize,
    pub(crate) bytes: Vec<u8>, // 長さは size.max(INLINE_SIZE)
}

impl OwnedValue {
    /// 空値（`type_id == 0`）。未接続かつ既定値なしのソケットを表す。
    pub fn empty() -> Self {
        Self {
            type_id: SYN_URID_INVALID,
            size: 0,
            bytes: vec![0u8; INLINE_SIZE],
        }
    }

    /// 生バイト列から PLAIN 値を作る。`src` を実体としてコピーする。
    pub fn from_plain_bytes(type_id: SynTypeId, src: &[u8]) -> Self {
        let mut bytes = vec![0u8; src.len().max(INLINE_SIZE)];
        bytes[..src.len()].copy_from_slice(src);
        Self {
            type_id,
            size: src.len(),
            bytes,
        }
    }

    /// 空値（`type_id == 0`）か。
    pub fn is_empty(&self) -> bool {
        self.type_id == SYN_URID_INVALID
    }

    /// 値の型 ID。
    pub fn type_id(&self) -> SynTypeId {
        self.type_id
    }

    /// 実体バイト列（長さ `size`。パディングは含まない）。
    pub fn bytes(&self) -> &[u8] {
        &self.bytes[..self.size]
    }

    /// プラグインが値渡しした `SynValue` をホスト所有へコピーする。
    ///
    /// # Safety
    /// `v` は妥当な `SynValue`。大型（>16byte）の場合 `v.payload.ptr` は `v.size` バイト
    /// 読める領域を指していなければならない。
    pub(crate) unsafe fn from_value(v: &SynValue) -> Self {
        let sz = v.size;
        let mut bytes = vec![0u8; sz.max(INLINE_SIZE)];
        if sz <= INLINE_SIZE {
            // SVO: 値は payload にインライン。
            bytes[..sz].copy_from_slice(&v.payload.data[..sz]);
        } else {
            core::ptr::copy_nonoverlapping(v.payload.ptr as *const u8, bytes.as_mut_ptr(), sz);
        }
        Self {
            type_id: v.type_id,
            size: sz,
            bytes,
        }
    }

    /// プラグインへ値渡しする `SynValue` を組み立てる。
    ///
    /// ≤16byte は SVO（payload へインライン）、大型は `self.bytes` を指す。後者は
    /// `self` が生存する間のみ有効。
    ///
    /// # Safety
    /// 返り値（大型のとき）は `self` の借用を含むため、`self` より長く使ってはならない。
    pub(crate) unsafe fn to_value(&self) -> SynValue {
        if self.size <= INLINE_SIZE {
            let mut data = [0u8; INLINE_SIZE];
            data[..self.size].copy_from_slice(&self.bytes[..self.size]);
            SynValue {
                type_id: self.type_id,
                payload: SynValuePayload { data },
                size: self.size,
            }
        } else {
            SynValue {
                type_id: self.type_id,
                payload: SynValuePayload {
                    ptr: self.bytes.as_ptr() as *mut c_void,
                },
                size: self.size,
            }
        }
    }
}
