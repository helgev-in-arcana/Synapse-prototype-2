//! ロード済みプラグインモジュールとノード種別ハンドル。
//!
//! # 意図
//! DLL のロード・ABI 検査・2フェーズ登録（`on_register_types` → `on_register_nodes`,
//! ADR-027）・`on_unload`（Drop）までのライフサイクルを [`LoadedModule`] に閉じ込める。
//! 複数モジュールは [`LoadedModule::load_many`] が「全モジュールの型登録 → 全モジュールの
//! ノード登録」の順で処理し、ノード登録時に参照しうる型が出揃っていることを保証する。
//! モジュールが登録したノード種別は [`NodeType`] で列挙でき、[`LoadedModule::instantiate`]
//! でノードインスタンス（[`crate::node::NodeInstance`]）を生成する。ノードインスタンスは
//! このモジュールより長生きできない（ライフタイムで強制）。
//!
//! # 信頼境界・グローバル登録
//! `dlopen` の時点で任意コードが走り得る（防げない＝信頼できる供給元のみロードする前提）。
//! ノード登録はプロセスグローバル（ADR-023）に積まれるため、各ロードに [`ModuleId`] を
//! 採番し、on_load 中の登録に出所を付与して「このモジュール分」を ID で正確に切り出す。
//! アンロード時は dlclose の前に [`purge_module`] でグローバル登録を除去し、dangling を防ぐ。

use core::ffi::c_void;
use std::marker::PhantomData;
use std::path::Path;
use std::ptr::null_mut;
use std::sync::Mutex;

use libloading::{Library, Symbol};
use synapse_abi::{SynHostStruct, SynModule, SynNodeDesc, SYN_ABI_VERSION};

use crate::error::{check, Error, Result};
use crate::ffi::cstr_to_string;
use crate::host::{h_fetch_suite, h_log, h_mark_dirty, h_register_node};
use crate::node::NodeInstance;
use crate::session::{lock_session, purge_module, ModuleId};

/// ロード済みプラグインモジュール。Drop で `on_unload` を呼び、グローバル登録を purge する。
pub struct LoadedModule {
    _lib: Library,
    _host: Box<SynHostStruct>,
    module: *const SynModule,
    /// このモジュールの ID（登録の出所追跡・アンロード時 purge に使う）。
    id: ModuleId,
    /// このモジュールが on_register_nodes で登録した node desc（モジュールイメージ内 static を指す）。
    descs: Vec<*const SynNodeDesc>,
}

/// ノード種別ハンドル（モジュールに紐づく）。
///
/// `'m` はモジュールの生存に縛られ、記述子の dangling を型で防ぐ。
#[derive(Clone, Copy)]
pub struct NodeType<'m> {
    desc: *const SynNodeDesc,
    _m: PhantomData<&'m LoadedModule>,
}

impl<'m> NodeType<'m> {
    /// ノード URI。
    pub fn uri(&self) -> String {
        cstr_to_string(unsafe { &*self.desc }.node_uri)
    }
    /// 表示名。
    pub fn display_name(&self) -> String {
        cstr_to_string(unsafe { &*self.desc }.display_name)
    }
}

/// フェーズ実行前のモジュール（DLL ロード・ABI 検査・ID 採番まで済み）。
struct PendingModule {
    lib: Library,
    host: Box<SynHostStruct>,
    module: *const SynModule,
    id: ModuleId,
}

impl LoadedModule {
    /// 単一モジュールをロードする（[`LoadedModule::load_many`] の縮退形）。
    pub fn load(path: &Path) -> Result<Self> {
        let mut v = Self::load_many(&[path])?;
        Ok(v.pop().expect("load_many は paths と同数を返す"))
    }

    /// 複数モジュールを 2 フェーズでロードする（ADR-027）。
    ///
    /// 全モジュールの `on_register_types` を先に呼び、その後に全モジュールの
    /// `on_register_nodes` を呼ぶ。これにより「ノード登録時には参照しうる型がすべて
    /// 出揃っている」を保証する。型登録フェーズ中は他モジュール型の lookup を拒否する
    /// （型-型依存の禁止＝ADR-028 の違反検知。[`crate::session`] 参照）。
    ///
    /// # 信頼境界
    /// `Library::new` の時点で DLL のグローバルコンストラクタ等、任意コードが走り得る。これは
    /// dlopen の本質で防げない——プラグインは信頼できる供給元のみロードする前提（コード署名等は
    /// 上位の責務）。
    ///
    /// # グローバル登録の出所付与
    /// 各モジュールに [`ModuleId`] を採番し `current_loading` にセットすることで、登録フェーズ中の
    /// register_node / reg_type 登録に出所を付与し、「このモジュール分」を ID で正確に切り出す。
    /// ロード全体を LOAD_LOCK で直列化する（実運用では起動時に逐次ロードするので制約にならない）。
    /// いずれかのフェーズが失敗したら、バッチ全体の部分登録を purge してから返す。
    pub fn load_many(paths: &[&Path]) -> Result<Vec<Self>> {
        static LOAD_LOCK: Mutex<()> = Mutex::new(());
        let _load_guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

        // --- フェーズ0: 全 DLL をロードし ABI を検査、ID を採番 ---
        let mut pending: Vec<PendingModule> = Vec::with_capacity(paths.len());
        let result = (|| -> Result<Vec<Self>> {
            for path in paths {
                let lib = unsafe { Library::new(path) }.map_err(|e| Error::Load(e.to_string()))?;
                let entry: Symbol<unsafe extern "C" fn() -> *const SynModule> =
                    unsafe { lib.get(b"synapse_module\0") }.map_err(|_| Error::MissingEntry)?;
                let module = unsafe { entry() };
                if module.is_null() {
                    return Err(Error::MissingEntry);
                }
                let m = unsafe { &*module };
                if m.abi_version != SYN_ABI_VERSION {
                    return Err(Error::AbiVersion {
                        found: m.abi_version,
                        expected: SYN_ABI_VERSION,
                    });
                }
                let host = Box::new(SynHostStruct {
                    host_ctx: null_mut(),
                    fetch_suite: Some(h_fetch_suite),
                    register_node: Some(h_register_node),
                    mark_dirty: Some(h_mark_dirty),
                    log: Some(h_log),
                });
                let id = {
                    let mut st = lock_session();
                    let id = st.next_module_id;
                    st.next_module_id += 1;
                    id
                };
                pending.push(PendingModule {
                    lib,
                    host,
                    module,
                    id,
                });
            }

            // --- フェーズ1: 全モジュールの型登録 ---
            for p in pending.iter_mut() {
                let m = unsafe { &*p.module };
                if let Some(f) = m.on_register_types {
                    {
                        let mut st = lock_session();
                        st.current_loading = Some(p.id);
                        st.in_type_phase = true;
                    }
                    let status = unsafe { f(p.host.as_mut() as *mut SynHostStruct) };
                    {
                        let mut st = lock_session();
                        st.current_loading = None;
                        st.in_type_phase = false;
                    }
                    check(status)?;
                }
            }

            // --- フェーズ2: 全モジュールのノード登録 ---
            for p in pending.iter_mut() {
                let m = unsafe { &*p.module };
                if let Some(f) = m.on_register_nodes {
                    lock_session().current_loading = Some(p.id);
                    let status = unsafe { f(p.host.as_mut() as *mut SynHostStruct) };
                    lock_session().current_loading = None;
                    check(status)?;
                }
            }

            // --- 完了: 各モジュールの登録ノードを回収して LoadedModule 化 ---
            let st = lock_session();
            Ok(pending
                .drain(..)
                .map(|p| {
                    let descs: Vec<*const SynNodeDesc> = st
                        .nodes
                        .iter()
                        .filter(|&&(mid, _)| mid == p.id)
                        .map(|&(_, ptr)| ptr.0)
                        .collect();
                    LoadedModule {
                        _lib: p.lib,
                        _host: p.host,
                        module: p.module,
                        id: p.id,
                        descs,
                    }
                })
                .collect())
        })();

        // 失敗時: バッチ全体の部分登録を purge してから返す（dlclose は pending の drop で起こる）。
        if result.is_err() {
            {
                let mut st = lock_session();
                st.current_loading = None;
                st.in_type_phase = false;
            }
            for p in &pending {
                purge_module(p.id);
            }
        }
        result
    }

    /// モジュール URI（名前空間）。
    pub fn module_uri(&self) -> String {
        cstr_to_string(unsafe { &*self.module }.module_uri)
    }

    /// このモジュールが登録した全ノード種別。
    pub fn node_types(&self) -> Vec<NodeType<'_>> {
        self.descs
            .iter()
            .map(|&desc| NodeType {
                desc,
                _m: PhantomData,
            })
            .collect()
    }

    /// URI でノード種別を引く。
    pub fn find_node(&self, uri: &str) -> Option<NodeType<'_>> {
        self.node_types().into_iter().find(|t| t.uri() == uri)
    }

    /// ノードインスタンスを生成する（`create`）。
    pub fn instantiate<'m>(&'m self, ty: NodeType<'m>) -> Result<NodeInstance<'m>> {
        let desc = unsafe { &*ty.desc };
        let create = desc.create.ok_or(Error::NullCallback("create"))?;
        let mut instance: *mut c_void = null_mut();
        check(unsafe { create(null_mut(), &mut instance) })?;
        Ok(NodeInstance::new(ty.desc, instance))
    }
}

impl Drop for LoadedModule {
    fn drop(&mut self) {
        let m = unsafe { &*self.module };
        if let Some(on_unload) = m.on_unload {
            unsafe { on_unload(self._host.as_mut() as *mut SynHostStruct) };
        }
        // dlclose（_lib の drop）より前に、このモジュール由来のグローバル登録を除去する。
        // これで以後の Session::type_vtable / 別モジュールロードが stale ポインタを掴まない。
        purge_module(self.id);
    }
}
