//! ロード済みプラグインモジュールとノード種別ハンドル。
//!
//! # 意図
//! DLL のロード・ABI 検査・`on_load` 実行・`on_unload`（Drop）までのライフサイクルを
//! [`LoadedModule`] に閉じ込める。モジュールが登録したノード種別は [`NodeType`] で列挙でき、
//! [`LoadedModule::instantiate`] でノードインスタンス（[`crate::node::NodeInstance`]）を生成
//! する。ノードインスタンスはこのモジュールより長生きできない（ライフタイムで強制）。
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
    /// このモジュールが on_load で登録した node desc（モジュールイメージ内 static を指す）。
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

impl LoadedModule {
    /// DLL をロードし、`synapse_module` を取得、ABI を検査し、`on_load` まで実行する。
    ///
    /// # 信頼境界
    /// `Library::new` の時点で DLL のグローバルコンストラクタ等、任意コードが走り得る。これは
    /// dlopen の本質で防げない——プラグインは信頼できる供給元のみロードする前提（コード署名等は
    /// 上位の責務）。
    ///
    /// # グローバル登録の出所付与
    /// このモジュールに [`ModuleId`] を採番し `current_loading` にセットすることで、on_load 中の
    /// register_node / reg_type 登録に出所を付与し、「このモジュール分」を ID で正確に切り出す
    /// （before/after 差分は不要）。ロード全体を LOAD_LOCK で直列化する（実運用では起動時に逐次
    /// ロードするので制約にならない）。on_load 失敗時は部分登録を purge してから返す。
    pub fn load(path: &Path) -> Result<Self> {
        static LOAD_LOCK: Mutex<()> = Mutex::new(());
        let _load_guard = LOAD_LOCK.lock().unwrap_or_else(|e| e.into_inner());

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

        let mut host = Box::new(SynHostStruct {
            host_ctx: null_mut(),
            fetch_suite: Some(h_fetch_suite),
            register_node: Some(h_register_node),
            mark_dirty: Some(h_mark_dirty),
            log: Some(h_log),
        });

        // ID を採番し、on_load 中の登録に出所を付与する。
        let id = {
            let mut st = lock_session();
            let id = st.next_module_id;
            st.next_module_id += 1;
            st.current_loading = Some(id);
            id
        };
        let on_load = m.on_load.ok_or(Error::NullCallback("on_load"))?;
        let status = unsafe { on_load(host.as_mut() as *mut SynHostStruct) };
        // 成否によらず current_loading を必ずクリアし、この ID の登録ノードを回収する。
        let descs: Vec<*const SynNodeDesc> = {
            let mut st = lock_session();
            st.current_loading = None;
            st.nodes
                .iter()
                .filter(|&&(mid, _)| mid == id)
                .map(|&(_, p)| p.0)
                .collect()
        };
        // on_load 失敗時は部分登録の残骸を purge してから返す（dlclose は lib drop で起こる）。
        if let Err(e) = check(status) {
            purge_module(id);
            return Err(e);
        }

        Ok(LoadedModule {
            _lib: lib,
            _host: host,
            module,
            id,
            descs,
        })
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
