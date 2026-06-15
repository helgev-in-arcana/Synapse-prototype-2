//! モジュールグローバルなスイート保持（FINDINGS F-3 の隠蔽）。
//!
//! # 意図
//! ホストは `on_load` のハンドシェイクでスイート（decl / eval / urid / type-registry の
//! 関数テーブル）を渡す。これを毎回引き回すのは煩雑なので、SDK がモジュールグローバルに
//! 保持し、`Declarer` / `ProcessCtx` 等から透過的に参照できるようにする。作者のコードには
//! スイートもグローバルも現れない。
//!
//! 1 モジュール = 1 ホスト（on_load ハンドシェイク, ADR-023）でモジュール寿命中は不変・
//! 読み取り専用に使う。初期化は [`set_suites`]（`on_load` から一度だけ）。

use core::ffi::{c_void, CStr};
use std::sync::OnceLock;

use synapse_abi::{SynDeclSuite, SynEvalSuite, SynUrid, SynUridSuite};

/// `Send` / `Sync` を付けたスイートポインタ運搬用 newtype（`usize` より provenance と意図が明確）。
///
/// # Safety
/// 指す先はホスト所有のスイート構造体。モジュール寿命中不変、読み取り専用に使う。
pub(crate) struct SuitePtr(pub(crate) *const c_void);
unsafe impl Send for SuitePtr {}
unsafe impl Sync for SuitePtr {}

/// `on_load` で取得した 4 スイートのポインタ束。
pub(crate) struct Suites {
    pub(crate) decl: SuitePtr,
    pub(crate) eval: SuitePtr,
    pub(crate) urid: SuitePtr,
    pub(crate) treg: SuitePtr,
}

static SUITES: OnceLock<Suites> = OnceLock::new();

/// スイート束をモジュールグローバルへ一度だけ格納する（`on_load` から呼ぶ）。
pub(crate) fn set_suites(s: Suites) {
    let _ = SUITES.set(s);
}

/// 格納済みスイート束を取得する。`on_load` 未実行なら panic（`synapse_module!` が必要）。
pub(crate) fn suites() -> &'static Suites {
    SUITES.get().expect("on_load が未実行（synapse_module! が必要）")
}

/// decl スイート参照。
///
/// # Safety
/// 格納済みポインタが妥当な `SynDeclSuite` を指していること（`on_load` 後は常に成立）。
pub(crate) unsafe fn decl_suite() -> &'static SynDeclSuite {
    &*(suites().decl.0 as *const SynDeclSuite)
}

/// eval スイート参照。
///
/// # Safety
/// 格納済みポインタが妥当な `SynEvalSuite` を指していること。
pub(crate) unsafe fn eval_suite() -> &'static SynEvalSuite {
    &*(suites().eval.0 as *const SynEvalSuite)
}

/// urid スイート参照。
///
/// # Safety
/// 格納済みポインタが妥当な `SynUridSuite` を指していること。
pub(crate) unsafe fn urid_suite() -> &'static SynUridSuite {
    &*(suites().urid.0 as *const SynUridSuite)
}

/// URI をホストの URID へ intern する。
pub(crate) fn urid_of(uri: &CStr) -> SynUrid {
    unsafe { (urid_suite().map.unwrap())(uri.as_ptr()) }
}
