//! プロセスグローバルなセッション状態（ADR-023: 1 プロセス = 1 ホスト = 1 セッション）。
//!
//! # 意図
//! URID intern（URI ⇄ 安定 ID）、型 vtable レジストリ、ノード記述子の登録は、設計上
//! プロセスグローバル（ADR-023）。これらの唯一の窓口が [`Session`] と内部の [`SESSION`]。
//! プラグインが呼ぶ URID スイート（[`URID_SUITE`]）と型レジストリスイート（[`TYPE_SUITE`]）も
//! ここに置き、すべて同一の `Mutex` でシリアライズする。
//!
//! # モジュール出所追跡
//! 登録（vtable / node desc）はどのモジュール由来かを [`ModuleId`] で記録する。モジュールの
//! アンロード時に [`purge_module`] が同 ID の登録をまとめて除去し、dlclose 後の dangling
//! ポインタ（モジュールイメージ内 static を指す）を残さない。

use core::ffi::c_char;
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::sync::{Mutex, MutexGuard, OnceLock};

use synapse_abi::{
    SynNodeDesc, SynStatus, SynTypeId, SynTypeRegistrySuite, SynTypeVTable, SynUrid, SynUridSuite,
    SYN_ERR_BAD_ARG, SYN_OK, SYN_URID_INVALID,
};

use crate::ffi::{cstr_to_string, guard_or, guard_status, SendPtr};

/// モジュール識別子。
///
/// ロードごとに採番し、登録（vtable / node desc）の出所を追跡する。アンロード時に同 ID の
/// 登録をグローバルから [`purge_module`] で除去して dangling を防ぐ。`0` は「ロード文脈外の
/// 登録」用に予約（purge 対象外）。
pub(crate) type ModuleId = u64;

/// プロセスグローバル状態の実体。すべて [`SESSION`] の `Mutex` 下で操作する。
pub(crate) struct SessionInner {
    uri_to_id: HashMap<String, u32>,
    id_to_uri: HashMap<u32, CString>,
    next_id: u32,
    pub(crate) next_module_id: ModuleId,
    /// 現在 on_load 実行中のモジュール ID（reg_type / register_node が出所を付与するのに使う）。
    /// ロードは [`crate::module::LoadedModule::load`] が LOAD_LOCK で直列化するので競合しない。
    pub(crate) current_loading: Option<ModuleId>,
    /// 型 ID → (登録元モジュール, vtable ポインタ)。ポインタはモジュールイメージ内 static。
    pub(crate) vtables: HashMap<u32, (ModuleId, SendPtr<SynTypeVTable>)>,
    /// (登録元モジュール, node desc ポインタ)。desc はモジュールイメージ内 static。
    pub(crate) nodes: Vec<(ModuleId, SendPtr<SynNodeDesc>)>,
}

static SESSION: OnceLock<Mutex<SessionInner>> = OnceLock::new();

fn session_inner() -> &'static Mutex<SessionInner> {
    SESSION.get_or_init(|| {
        Mutex::new(SessionInner {
            uri_to_id: HashMap::new(),
            id_to_uri: HashMap::new(),
            next_id: 2,        // 0=invalid, 1=ANY
            next_module_id: 1, // 0 は「ロード文脈外の登録」用に予約（purge 対象外）
            current_loading: None,
            vtables: HashMap::new(),
            nodes: Vec::new(),
        })
    })
}

/// セッションロックを取得する。
///
/// poison は回収して続行する（intern マップ中心の状態で、一部のコールバックが panic しても
/// 半端な状態が壊滅的にならないため、可用性を優先）。
pub(crate) fn lock_session() -> MutexGuard<'static, SessionInner> {
    session_inner().lock().unwrap_or_else(|e| e.into_inner())
}

/// ロード元モジュールの全登録（node desc / vtable）をグローバルから除去する。
///
/// dlclose で desc / vtable のアドレス（モジュールイメージ内 static）が無効化されるため、
/// アンロード時や on_load 失敗時にこれを呼んで stale ポインタを残さない。
pub(crate) fn purge_module(id: ModuleId) {
    let mut st = lock_session();
    st.nodes.retain(|&(mid, _)| mid != id);
    st.vtables.retain(|_, &mut (mid, _)| mid != id);
}

/// プロセスグローバル状態（URID intern / 型 vtable / ノード登録）への安全な窓口。
///
/// 1 プロセス = 1 ホスト = 1 セッション（ADR-023）なので状態はグローバルが正。
pub struct Session;

impl Session {
    /// URI をセッション安定な URID に intern する。
    pub fn urid(uri: &CStr) -> SynUrid {
        urid_map(uri.as_ptr())
    }

    /// 型 ID から登録済み vtable を引く（未登録 / アンロード済みなら `None`）。
    ///
    /// 返るポインタは**登録元モジュールの生存中のみ有効**（モジュールイメージ内 static を指す）。
    /// 当該モジュールがアンロードされると、対応エントリは [`purge_module`] が除去するため
    /// 以後 `None` を返す（dangling ポインタは返さない）。
    pub fn type_vtable(id: SynTypeId) -> Option<*const SynTypeVTable> {
        lock_session().vtables.get(&id).map(|&(_, p)| p.0)
    }
}

extern "C" fn urid_map(uri: *const c_char) -> SynUrid {
    guard_or(SYN_URID_INVALID, || {
        if uri.is_null() {
            return SYN_URID_INVALID;
        }
        let s = cstr_to_string(uri);
        let mut st = lock_session();
        if let Some(&id) = st.uri_to_id.get(&s) {
            return id;
        }
        let id = st.next_id;
        st.next_id += 1;
        st.uri_to_id.insert(s.clone(), id);
        // s は cstr 由来で内部 NUL を含まないため CString 化は失敗しない。
        if let Ok(c) = CString::new(s) {
            st.id_to_uri.insert(id, c);
        }
        id
    })
}

extern "C" fn urid_unmap(id: SynUrid) -> *const c_char {
    guard_or(core::ptr::null(), || {
        lock_session()
            .id_to_uri
            .get(&id)
            .map_or(core::ptr::null(), |c| c.as_ptr())
    })
}

/// プラグインへ供給する URID スイート。
pub(crate) static URID_SUITE: SynUridSuite = SynUridSuite {
    map: Some(urid_map),
    unmap: Some(urid_unmap),
};

extern "C" fn reg_type(uri: *const c_char, vt: *const SynTypeVTable) -> SynStatus {
    guard_status(|| {
        if uri.is_null() || vt.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let id = urid_map(uri); // 先に完全 return するので Mutex 二重ロックにならない
        let mut st = lock_session();
        let mid = st.current_loading.unwrap_or(0);
        st.vtables.insert(id, (mid, SendPtr(vt)));
        SYN_OK
    })
}

extern "C" fn reg_lookup(t: SynTypeId) -> *const SynTypeVTable {
    guard_or(core::ptr::null(), || {
        lock_session()
            .vtables
            .get(&t)
            .map_or(core::ptr::null(), |&(_, p)| p.0)
    })
}

extern "C" fn reg_type_of(uri: *const c_char) -> SynTypeId {
    urid_map(uri)
}

/// プラグインへ供給する型レジストリスイート。
pub(crate) static TYPE_SUITE: SynTypeRegistrySuite = SynTypeRegistrySuite {
    register_type: Some(reg_type),
    lookup: Some(reg_lookup),
    type_of: Some(reg_type_of),
};
