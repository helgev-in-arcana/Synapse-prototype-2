//! ノード記述子（基底層）。
//!
//! `declare`/`negotiate`/`process` が受け取るのは不透明ハンドル（`SynDeclBuilder` /
//! `SynEvalCtx`）だけで、それを操作するスイート構造体は**型として参照しない**。
//! この型消去が core / suite の分割線になっている。

use core::ffi::{c_char, c_void};

use super::handle::{SynDeclBuilder, SynEvalCtx, SynNode};
use super::status::SynStatus;

/// インスタンス再入タイリング（宣言のみ・実装は後回し可）。
pub const SYN_CAP_REENTRANT_TILING: u32 = 1 << 0;
/// フレーム並列レンダ（宣言のみ）。
pub const SYN_CAP_PARALLEL_FRAMES: u32 = 1 << 1;
/// UI と process の同一インスタンス並行を許可。
pub const SYN_CAP_THREAD_SAFE_UI: u32 = 1 << 2;

/// 1 ノード種別の記述子。ロード時に host->register_node で登録する。
/// 必須: create/destroy/declare/negotiate/process。
#[repr(C)]
pub struct SynNodeDesc {
    /// SYN_CAP_*。
    pub caps: u32,
    /// ノード URI（"com.vendor.blur.gaussian"）。モジュール寿命まで存続(static 推奨)。
    pub node_uri: *const c_char,
    /// 表示名。
    pub display_name: *const c_char,

    /// 既定状態でインスタンスを生成。node は instance に保持し host->mark_dirty 等に使う。
    pub create: Option<unsafe extern "C" fn(node: *mut SynNode, out_instance: *mut *mut c_void) -> SynStatus>,
    /// インスタンス破棄。
    pub destroy: Option<unsafe extern "C" fn(instance: *mut c_void)>,

    /// 宣言フェーズ（データ無し）。状態からフル再宣言する冪等関数。
    pub declare: Option<unsafe extern "C" fn(instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus>,

    /// poll 列挙（データ無し・単発）。必要入力を request で積み SYN_OK を返す。
    /// 静的ノードは全入力を列挙するだけ。値依存の枝刈りは将来の二層 API で対応。
    pub negotiate: Option<unsafe extern "C" fn(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus>,

    /// 処理（1 回）。要求が全充足された後に呼ばれる。
    pub process: Option<unsafe extern "C" fn(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus>,

    /// ソケットに出ない内部パラメータの永続化（不透明 blob）。out=NULL/cap=0 で
    /// 必要サイズを written に返し、確保後に再呼び出しで書き込む。blob は自己記述
    /// （自前の version を内包）。ソケットデフォルトは host 専有でプラグインは値を
    /// 保持しない（get_input の借用のみ）ため、blob には構造的に内部パラメータだけが
    /// 入る。NULL 可。
    pub save_state: Option<unsafe extern "C" fn(instance: *mut c_void, out: *mut c_void, cap: usize, written: *mut usize) -> SynStatus>,
    /// 内部パラメータの復元。NULL 可。
    pub load_state: Option<unsafe extern "C" fn(instance: *mut c_void, input: *const c_void, len: usize) -> SynStatus>,

    /// 任意拡張（UI 等）。未対応は NULL を返す。
    pub get_extension: Option<unsafe extern "C" fn(instance: *mut c_void, ext_id: *const c_char) -> *const c_void>,
}
