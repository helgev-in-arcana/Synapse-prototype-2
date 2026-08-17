//! ホスト & モジュールエントリポイント（基底層）。
//!
//! `fetch_suite` の戻り値が `*const c_void` であることが core / suite 分割の要。
//! ホスト構造体はスイート構造体を型として知らないため、スイートの追加は基底 ABI に
//! 一切触れずに行える（ADR-020 の「スイートは id で取得」）。

use core::ffi::{c_char, c_int, c_void};

use super::handle::SynNode;
use super::node::SynNodeDesc;
use super::status::SynStatus;

/// ホストが提供する操作群。on_load で受け取りモジュール側に保持する（1 モジュール=1 ホスト）。
#[repr(C)]
pub struct SynHostStruct {
    /// ホスト側の不透明ポインタ。
    pub host_ctx: *mut c_void,
    /// スイートを id で取得（未提供なら NULL）。
    pub fetch_suite: Option<unsafe extern "C" fn(h: *mut SynHostStruct, suite_id: *const c_char) -> *const c_void>,
    /// ノード記述子を登録（on_load 中に呼ぶ）。
    pub register_node: Option<unsafe extern "C" fn(h: *mut SynHostStruct, desc: *const SynNodeDesc) -> SynStatus>,
    /// 状態変更通知。当該ノード+下流サブツリーのキャッシュを無効化する。
    pub mark_dirty: Option<unsafe extern "C" fn(h: *mut SynHostStruct, node: *mut SynNode)>,
    /// ログ出力（level は SYN_LOG_*）。
    pub log: Option<unsafe extern "C" fn(h: *mut SynHostStruct, level: c_int, msg: *const c_char)>,
}

/// ホストハンドル（SynHostStruct へのポインタ）。
pub type SynHost = *mut SynHostStruct;

/// モジュール記述子。各 .so/.dll は `synapse_module` を 1 つだけエクスポートする。
///
/// ロードは 2 フェーズ（ADR-027）: ホストは**全モジュール**の `on_register_types` を先に呼び、
/// その後**全モジュール**の `on_register_nodes` を呼ぶ。これにより「ノード登録時には参照しうる
/// 型がすべて出揃っている」を保証する。型登録フェーズ内で他モジュールの型に依存してはならない
/// （型-型依存の禁止、ADR-028 ★）。
#[repr(C)]
pub struct SynModule {
    /// ビルド時の SYN_ABI_VERSION。
    pub abi_version: u32,
    /// 名前空間（"com.vendor.blur"）。
    pub module_uri: *const c_char,
    /// semver 文字列。
    pub module_version: *const c_char,
    /// フェーズ1: 型をここで登録する（スイート fetch もここで行う）。型が無ければ NULL 可。
    /// 他モジュールの型を lookup してはならない（ADR-028。ホストは違反を拒否してよい）。
    pub on_register_types: Option<unsafe extern "C" fn(h: SynHost) -> SynStatus>,
    /// フェーズ2: ノードをここで登録する。全モジュールの型登録後に呼ばれる。
    /// ノードが無ければ NULL 可。
    pub on_register_nodes: Option<unsafe extern "C" fn(h: SynHost) -> SynStatus>,
    /// アンロード前に 1 回。
    pub on_unload: Option<unsafe extern "C" fn(h: SynHost)>,
}

/// ABI バージョン（当面はこの 1 個で足りる）。
///
/// 基底 ABI（本モジュールと `SynNodeDesc` / `SynValue`）は版内で固定で、ホストは
/// `!=` で拒否する（ADR-020）。スイートは id 引きで版に載らない。
/// v2: 2フェーズロード（ADR-027）＋ alloc の type_id 引数（ADR-029）。
/// v3: SVO インライン幅を 16byte へ拡張、payload を union 化（ADR-006/Open-20）。
pub const SYN_ABI_VERSION: u32 = 3;

/// モジュールエントリのシグネチャ: `const SynModule *synapse_module(void);`
pub type SynModuleEntryFn = Option<unsafe extern "C" fn() -> *const SynModule>;

/// モジュールがエクスポートすべきシンボル名。
///
/// 文字列定数は cbindgen が出力できないため、build.rs がソースを parse して
/// `#define` を生成する（層ヘッダへの振り分けもファイル位置から決まる）。
pub const SYN_MODULE_ENTRY_SYMBOL: &str = "synapse_module";
