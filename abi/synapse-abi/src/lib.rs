//! Synapse プラグイン C ABI — **アンブレラ**。
//!
//! ABI の正本は Rust で、互換性の粒度に合わせて **1 層 = 1 クレート = 1 ヘッダ**に分かれる。
//! このクレートは 3 層をまとめて再エクスポートし、`include/synapse_abi.h`（3 本を取り込む
//! アンブレラヘッダ）を生成する。利用者はこの 1 クレート / この 1 ヘッダだけ見ればよい。
//!
//! # 層構造（互換性の粒度がそのままクレート境界）
//!
//! | 層 | クレート | 生成ヘッダ | 互換性 |
//! |---|---|---|---|
//! | 基底 ABI | [`synapse_abi_core`] | `synapse_abi_core.h` | [`SYN_ABI_VERSION`] 一致必須（ホストは `!=` で拒否） |
//! | スイート | [`synapse_abi_suite`] | `synapse_abi_suite.h` | `fetch_suite(id)` 引き。**追加は非破壊**、構造体変更は破壊的 |
//! | 拡張 | [`synapse_abi_ext`] | `synapse_abi_ext.h` | `get_extension(ext_id)` 引き。**追加は完全に非破壊** |
//!
//! **依存は `suite`/`ext` → `core` の一方向で、クレート境界なので Cargo が強制する**
//! （`core` から上位層を参照すると循環依存で弾かれる）。成立している理由は、境界が
//! `fetch_suite` / `get_extension` / `get_api` の `*const c_void` で型消去され、不透明ハンドル
//! （`SynDeclBuilder` / `SynEvalCtx`）だけが両層をつないでいるため。
//!
//! # 公開面はフラット
//!
//! 全項目をクレート直下へ glob 再エクスポートしているので、`synapse_abi::SynEvalSuite` の
//! ように層を意識せず参照できる（`use synapse_abi::*;` も可）。層クレートを直接依存に足す
//! 必要は無い。
//!
//! # 項目の追加
//!
//! 該当する層クレートの該当ファイルに宣言を書くだけでよい。Rust の公開面は下の glob が
//! 拾い、C ヘッダは各層の `build.rs` が自クレートの `src/` を再帰走査して拾う。
//! **どの層に属するかはどのクレートに置いたかで決まる**（登録表の類は無い）。
//!
//! 詳細な決定根拠・却下案・未決事項は ADR（`docs/synapse_adr.md`）を参照。

#![warn(missing_docs)]

// 層クレートの全項目をここへ集約する。項目を足しても、この 3 行は変わらない。
pub use synapse_abi_core::*;
pub use synapse_abi_ext::*;
pub use synapse_abi_suite::*;
