//! プラグイン側 SDK。
//!
//! 作者は [`Node`] を実装し [`synapse_module!`] で公開するだけ。SDK が以下を隠す:
//!   - スイートの fetch とモジュールグローバル保持（FINDINGS F-3）
//!   - `SynValue` ⇔ Rust 型の SVO 変換（値渡し: ADR-022）
//!   - declare のインデックス規約・既定値配送・negotiate の縮退形（ADR-011）
//!   - save/load の 2 段サイズ問い合わせプロトコル
//!   - FFI 境界を越えるパニックの遮断（`catch_unwind` → エラーステータス）
//!
//! 作者のコードに unsafe / グローバル / negotiate は現れない。
//!
//! # モジュール構成
//!   - [`plain`]   … [`SynPlainType`] と `SynValue` ⇔ Rust 値の変換（内部）
//!   - [`error`]   … [`Error`] / [`Result`]
//!   - [`port`]    … [`InPort`] / [`OutPort`] / [`MultiInPort`]（型安全なポートトークン）
//!   - [`suites`]  … モジュールグローバルなスイート保持（内部）
//!   - [`context`] … [`Declarer`] / [`ProcessCtx`] / [`NegotiateCtx`]（作者向けコンテキスト）
//!   - [`node`]    … [`Node`] トレイト（作者の実装責務）
//!   - [`tramp`]   … C-ABI トランポリンと静的記述子（内部 + マクロ用ヘルパ）
//!   - `macros`    … [`synapse_module!`] マクロ
//!
//! 通常は [`prelude`] をまとめて取り込めばよい。

#![allow(clippy::missing_safety_doc)]
#![warn(missing_docs)]

/// ABI 型を再エクスポート（マクロが `$crate::abi::...` で参照する）。
pub use synapse_abi as abi;

mod context;
mod error;
mod macros;
mod node;
mod plain;
mod port;
mod suites;
mod tramp;

pub use context::{Declarer, NegotiateCtx, ProcessCtx};
pub use error::{Error, Result};
pub use node::Node;
pub use plain::SynPlainType;
pub use port::{InPort, MultiInPort, OutPort};

// マクロ展開が `$crate::...` で参照する内部ヘルパ（`#[doc(hidden)]`）。直接利用は想定しない。
#[doc(hidden)]
pub use tramp::{__on_load_begin, __on_unload, __register_node, __register_type, SyncModule};

/// よく使う型・トレイト・マクロをまとめて取り込むための prelude。
pub mod prelude {
    pub use crate::synapse_module;
    pub use crate::{
        Declarer, Error, InPort, MultiInPort, NegotiateCtx, Node, OutPort, ProcessCtx, Result,
        SynPlainType,
    };
    pub use core::ffi::CStr;
}
