//! データ単位（ワイヤフォーマット）。
//!
//! `SynValue` は評価境界を**値渡し**で越える唯一のデータ表現。payload の所有権規約
//! （clone/drop の担い手・確保時のアラインメント保証）は型 vtable が定めるが、
//! それは suite 層（`SynTypeVTable`）の責務で、ここは型として参照しない。

use core::ffi::c_void;

use super::urid::SynTypeId;

/// SVO インライン幅（バイト）。`size <= SYN_VALUE_INLINE` の値は `SynValue` の payload に
/// 直接格納する。ポインタ幅ではなく定数 16（color RGBA f32・vec2 f64・time rational 等の
/// 最頻パラメータ型が収まる幅。32-bit/64-bit で挙動が揃う）。
pub const SYN_VALUE_INLINE: usize = 16;

/// `SynValue` の payload 領域（インライン格納と領域ポインタの重ね合わせ）。
///
/// どちらのフィールドが有効かは `SynValue::size` で決まる（`size <= SYN_VALUE_INLINE` なら
/// `data`、超えるなら `ptr`）。`data` の読み書きは型 pun ではなく memcpy 経由で行うこと。
#[repr(C)]
#[derive(Clone, Copy)]
pub union SynValuePayload {
    /// size > SYN_VALUE_INLINE: ホスト所有領域へのポインタ / OPAQUE 型: 不透明ハンドル。
    pub ptr: *mut c_void,
    /// size <= SYN_VALUE_INLINE: 値そのもの（SVO インライン。memcpy で出し入れする）。
    pub data: [u8; 16],
}

/// エッジを流れるデータ単位。
///
/// `size <= SYN_VALUE_INLINE`（16byte）のとき payload は `data` に直接格納する
/// (small-value optimization)。読み書きは型 pun ではなく memcpy 経由で行うこと。
/// 不変条件: PLAIN 型の payload は位置独立な素のバイト列で、生ポインタを含まない。
/// 空（未接続かつデフォルト無し）の表現は `type_id == 0`。`ptr == NULL` は使わない
/// （SVO では inline の零値と区別できないため）。
#[repr(C)]
pub struct SynValue {
    /// 実体型の URID。0 は空。
    pub type_id: SynTypeId,
    /// size>16: 領域ポインタ（`ptr`） / size<=16: 値そのもの（`data`, SVO） /
    /// opaque型: 不透明ハンドル（`ptr`）。
    pub payload: SynValuePayload,
    /// 意味的なバイト数。
    pub size: usize,
}
