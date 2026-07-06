//! PLAIN 型（`SynValue` ⇔ Rust 型）のマーカトレイトと値変換。
//!
//! # 意図
//! ノード作者が入出力に使う Rust の素値（`f32` / `i32` …）を、ABI の値渡し [`SynValue`] と
//! 相互変換するための土台。memcpy 可能な固定サイズ型を [`SynPlainType`] でマークし、SVO
//! （≤16byte はインライン、超えるならホスト確保バッファ経由）の差を変換関数に閉じ込める。
//!
//! # 安全性の要
//! [`SynPlainType::SIZE`] は `size_of::<Self>()` と一致しなければならない。`_SIZE_CHECK` で
//! コンパイル時（post-monomorphization）に強制し、不一致な型を使った瞬間にビルドを止める。
//! これにより 32-bit でも 8 バイト型を正しく扱える。

use core::ffi::CStr;

use synapse_abi::{SynTypeId, SynValue};

/// memcpy 可能な固定サイズ PLAIN 型。
///
/// ≤16byte（`SYN_VALUE_INLINE`）は SVO（payload へインライン）、超えるならホスト確保バッファ
/// 経由で値渡しする。
///
/// # Safety
/// `SIZE == size_of::<Self>()` かつ任意のビットパターンが妥当（`Copy`・パディング無しを推奨）で
/// なければならない。
pub unsafe trait SynPlainType: Copy + 'static {
    /// この型の URI（例 `c"synapse:float"`）。URID intern と型登録に使う。
    const URI: &'static CStr;
    /// バイトサイズ（`size_of::<Self>()`）。
    const SIZE: usize;
    /// 健全性チェック: `SIZE` は `size_of::<Self>()` と一致しなければならない。
    ///
    /// 値変換コード（[`svo_value`] / [`value_to_plain`]）が参照するので、不一致なら
    /// その型を使った瞬間にコンパイルエラーになる（サイズ ≤/> インライン幅は問わない）。
    #[doc(hidden)]
    const _SIZE_CHECK: () = assert!(
        Self::SIZE == core::mem::size_of::<Self>(),
        "SynPlainType::SIZE は size_of::<Self>() と一致させること"
    );
}

unsafe impl SynPlainType for f32 {
    const URI: &'static CStr = c"synapse:float";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for f64 {
    const URI: &'static CStr = c"synapse:double";
    const SIZE: usize = 8;
}
unsafe impl SynPlainType for i32 {
    const URI: &'static CStr = c"synapse:int";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for u32 {
    const URI: &'static CStr = c"synapse:uint";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for i64 {
    const URI: &'static CStr = c"synapse:long";
    const SIZE: usize = 8;
}
unsafe impl SynPlainType for u64 {
    const URI: &'static CStr = c"synapse:ulong";
    const SIZE: usize = 8;
}

/// SVO 経路の判定境界（16byte、ABI 定数 `SYN_VALUE_INLINE`）。
pub(crate) const INLINE_SIZE: usize = synapse_abi::SYN_VALUE_INLINE;

/// ≤インライン幅の値を `SynValue`（SVO: 値を payload にインライン格納）へ組み立てる。
///
/// 出力・既定値の「≤16byte 経路」で共用する。`v` は呼び出し側に生きている参照（コピーするだけ）。
///
/// # Safety
/// `T: SynPlainType` で `T::SIZE == size_of::<T>()`（`_SIZE_CHECK` で強制）。`T::SIZE` は
/// `INLINE_SIZE` 以下でなければならない（呼び出し側が分岐で保証する）。
pub(crate) unsafe fn svo_value<T: SynPlainType>(type_id: SynTypeId, v: &T) -> SynValue {
    let () = T::_SIZE_CHECK; // SIZE == size_of を強制（post-mono でチェック）
    let mut data = [0u8; INLINE_SIZE];
    core::ptr::copy_nonoverlapping(v as *const T as *const u8, data.as_mut_ptr(), T::SIZE);
    SynValue {
        type_id,
        payload: synapse_abi::SynValuePayload { data },
        size: T::SIZE,
    }
}

/// `SynValue` を Rust 値へ変換する。サイズ不一致は `None`（大型異型の範囲外読みを遮断）。
///
/// `T::SIZE <= INLINE_SIZE` か否かで SVO（payload インライン）/ 領域（`*ptr`）を出し分ける
/// （const 分岐＝常用型では分岐ごと消える）。型 ID の照合は呼び出し側
/// （[`crate::context::ProcessCtx`] がポートの type_id と突合）が行う。
///
/// # Safety
/// `v` は妥当な `SynValue`。大型（>16byte）のとき `v.payload.ptr` は `T::SIZE` バイト読める
/// 領域を指す。
pub(crate) unsafe fn value_to_plain<T: SynPlainType>(v: &SynValue) -> Option<T> {
    let () = T::_SIZE_CHECK; // SIZE == size_of を強制（post-mono でチェック）
    if v.size != T::SIZE {
        return None;
    }
    let mut out = core::mem::MaybeUninit::<T>::uninit();
    let src: *const u8 = if T::SIZE <= INLINE_SIZE {
        v.payload.data.as_ptr()
    } else {
        v.payload.ptr as *const u8
    };
    core::ptr::copy_nonoverlapping(src, out.as_mut_ptr() as *mut u8, T::SIZE);
    Some(out.assume_init())
}
