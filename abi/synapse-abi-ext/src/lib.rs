//! Synapse プラグイン C ABI — **拡張層**（正本）。生成ヘッダ: `synapse_abi_ext.h`。
//!
//! # 互換性
//!
//! 拡張は `SynNodeDesc::get_extension(instance, ext_id)` で引く（CLAP 流）。未対応なら NULL が
//! 返るだけなので、**拡張の追加は完全に非破壊**。基底 ABI にもスイートにも触れずに機能を
//! 足せる穴で、ノード記述子を変えずに済ませるための層。
//!
//! # 依存方向
//!
//! この層は [`synapse_abi_core`] にのみ依存する。スイート層とは互いに独立。

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![warn(missing_docs)]

pub(crate) mod ui;

pub use ui::*;
