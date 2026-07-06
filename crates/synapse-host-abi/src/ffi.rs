//! FFI 境界の下支え（パニック遮断・生ポインタ運搬・C 文字列変換）。
//!
//! # 意図
//! ホスト側コールバックは `extern "C"` であり、その境界をまたいで Rust の巻き戻し
//! （unwind）を伝播させると ABI 契約違反で**プロセス全体が abort** する。プラグインから
//! の不正引数や内部バグで発火した panic をプロセスごと巻き込まないよう、本モジュールの
//! ガード関数で `catch_unwind` し、安全なデフォルト値（`SYN_ERR_*` や空値）へ落として返す。
//!
//! # 使い方
//! 各 `extern "C"` コールバックの本体を [`guard_status`] / [`guard_or`] / [`guard_unit`] の
//! いずれかで包む。返り値の種類（ステータス / 任意値 / なし）に応じて使い分ける。
//!
//! ここに置く道具は本クレート内部専用（すべて `pub(crate)`）で、公開 API には現れない。

use core::ffi::c_char;
use std::ffi::CStr;
use std::panic::{catch_unwind, AssertUnwindSafe};

use synapse_abi::{SynStatus, SYN_ERR_UNKNOWN};

/// SVO（Small Value Optimization, ADR-006/022）の判定境界（16byte、ABI 定数）。
///
/// 値サイズがこの幅以下なら `SynValue` の payload（`data`）へ値そのものをインライン格納し、
/// 超える場合のみ別領域（`ptr`）を指す。ポインタ幅非依存なので 32-bit/64-bit で挙動が揃う。
pub(crate) const INLINE_SIZE: usize = synapse_abi::SYN_VALUE_INLINE;

/// `SynStatus` を返すコールバックのパニックガード。panic 時は `SYN_ERR_UNKNOWN` を返す。
pub(crate) fn guard_status(f: impl FnOnce() -> SynStatus) -> SynStatus {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(SYN_ERR_UNKNOWN)
}

/// 任意の値 `R` を返すコールバックのパニックガード。panic 時は `default` を返す。
pub(crate) fn guard_or<R>(default: R, f: impl FnOnce() -> R) -> R {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

/// 返り値なしコールバックのパニックガード。panic は握り潰す（境界外へ漏らさない）。
pub(crate) fn guard_unit(f: impl FnOnce()) {
    let _ = catch_unwind(AssertUnwindSafe(f));
}

/// `Send` / `Sync` を付与した生ポインタ運搬用 newtype。
///
/// `usize` で運ぶより provenance（来歴）と意図が型に表れる。プロセスグローバルなセッション
/// 状態（型 vtable / ノード記述子）の登録に使う。
///
/// # Safety
/// 指す先はホスト所有、またはモジュールイメージ内の静的データ。1 プロセス = 1 セッション
/// （ADR-023）で、登録 / 解決 / 除去はセッションの `Mutex` でシリアライズされるためデータ
/// レースしない。生存は登録元モジュールに従い、アンロード時に当該エントリが除去される。
pub(crate) struct SendPtr<T>(pub(crate) *const T);

// `derive(Clone, Copy)` は `T: Copy` 境界を付けてしまう（T = vtable / desc は非 Copy）ため
// 手書きする。ポインタ自体は常に Copy なので、SendPtr<T> は T によらず Copy。
impl<T> Clone for SendPtr<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for SendPtr<T> {}
unsafe impl<T> Send for SendPtr<T> {}
unsafe impl<T> Sync for SendPtr<T> {}

/// C 文字列ポインタを Rust `String` へコピーする。NULL は空文字列に倒す（堅牢性方針）。
pub(crate) fn cstr_to_string(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}
