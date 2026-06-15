//! ホスト構造体（`SynHostStruct`）のコールバック実装。
//!
//! # 意図
//! プラグインの `on_load` には [`SynHostStruct`] を渡す。プラグインはこの構造体経由で、
//! - スイートの取得（[`h_fetch_suite`]：decl / eval / urid / type-registry を名前で引く）
//! - ノード記述子の登録（[`h_register_node`]）
//! - dirty 通知（[`h_mark_dirty`]：本層はキャッシュを持たないので受け口のみ）
//! - ログ出力（[`h_log`]）
//!
//! これらは [`crate::module::LoadedModule::load`] が組み立てる `SynHostStruct` に関数
//! ポインタとして差し込まれる。

use core::ffi::{c_char, c_int, c_void};

use synapse_abi::{
    SynHostStruct, SynNode, SynNodeDesc, SynStatus, SYN_DECL_SUITE, SYN_ERR_BAD_ARG,
    SYN_EVAL_SUITE, SYN_OK, SYN_TYPE_REGISTRY_SUITE, SYN_URID_SUITE,
};

use crate::decl::DECL_SUITE;
use crate::eval::EVAL_SUITE;
use crate::ffi::{cstr_to_string, guard_or, guard_status, guard_unit, SendPtr};
use crate::session::{lock_session, TYPE_SUITE, URID_SUITE};

/// スイート ID（文字列）からスイート構造体ポインタを引く。未知の ID は NULL。
pub(crate) extern "C" fn h_fetch_suite(
    _h: *mut SynHostStruct,
    id: *const c_char,
) -> *const c_void {
    guard_or(core::ptr::null(), || {
        if id.is_null() {
            return core::ptr::null();
        }
        let s = cstr_to_string(id);
        if s == SYN_DECL_SUITE {
            &DECL_SUITE as *const _ as *const c_void
        } else if s == SYN_EVAL_SUITE {
            &EVAL_SUITE as *const _ as *const c_void
        } else if s == SYN_URID_SUITE {
            &URID_SUITE as *const _ as *const c_void
        } else if s == SYN_TYPE_REGISTRY_SUITE {
            &TYPE_SUITE as *const _ as *const c_void
        } else {
            core::ptr::null()
        }
    })
}

/// ノード記述子をプロセスグローバルへ登録する。出所は現在ロード中のモジュール ID。
pub(crate) extern "C" fn h_register_node(
    _h: *mut SynHostStruct,
    desc: *const SynNodeDesc,
) -> SynStatus {
    guard_status(|| {
        if desc.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let mut st = lock_session();
        let mid = st.current_loading.unwrap_or(0);
        st.nodes.push((mid, SendPtr(desc)));
        SYN_OK
    })
}

/// dirty 通知の受け口。本層はキャッシュを持たない（dirty 伝播は本体ホストの責務）。
pub(crate) extern "C" fn h_mark_dirty(_h: *mut SynHostStruct, _node: *mut SynNode) {}

/// プラグインのログ出力。レベルとメッセージを stderr へ流す。
pub(crate) extern "C" fn h_log(_h: *mut SynHostStruct, level: c_int, msg: *const c_char) {
    guard_unit(|| {
        eprintln!("[plugin log L{level}] {}", cstr_to_string(msg));
    });
}
