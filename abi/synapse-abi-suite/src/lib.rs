//! Synapse プラグイン C ABI — **スイート層**（正本）。生成ヘッダ: `synapse_abi_suite.h`。
//!
//! # 互換性
//!
//! スイートは `SynHostStruct::fetch_suite(h, id)` で取得し、未知の id には NULL が返る。
//! したがってスイートの**追加は基底 ABI に触れない非破壊変更**で、旧ホスト × 新プラグインの
//! 組み合わせでも落ちない（ADR-003 / ADR-020）。
//!
//! ただし個々のスイート構造体自体に版数は無いため、**フィールド追加は基底 ABI と同じく
//! 破壊的**。柔らかいのは追加だけで、既存スイートの中身をいじるのは重い（Open-13）。
//!
//! # 依存方向
//!
//! この層は [`synapse_abi_core`] にのみ依存する。逆向きの依存は Cargo が循環として弾く。
//!
//! 生成ヘッダも同じ形で、`synapse_abi_suite.h` は冒頭で `synapse_abi_core.h` を include する
//! （cbindgen は依存クレートを parse しないので、core の型は名前参照だけが出る）。
//!
//! # 項目の追加
//!
//! 該当ファイルに宣言を書くだけでよい。Rust の公開面は下の glob 再エクスポートが拾い、
//! C ヘッダは `build.rs` が `src/` を再帰走査して拾う。登録表の類は無い。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![warn(missing_docs)]

pub(crate) mod decl;
pub(crate) mod eval;
pub(crate) mod type_registry;
pub(crate) mod urid;

pub use decl::*;
pub use eval::*;
pub use type_registry::*;
pub use urid::*;
