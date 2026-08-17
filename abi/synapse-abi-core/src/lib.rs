//! Synapse プラグイン C ABI — **基底 ABI 層**（正本）。生成ヘッダ: `synapse_abi_core.h`。
//!
//! # 互換性
//!
//! ここに含まれる型のレイアウトを変えることは**破壊的変更**であり、[`SYN_ABI_VERSION`] の
//! 更新を要する（ホストは `!=` で拒否する。ADR-020）。この層が「版一致必須の凍結面」。
//!
//! # 不変条件: この層は上位層を参照しない
//!
//! `suite` / `ext` への依存を足してはならない。**クレート境界なので Cargo が循環依存として
//! 弾く**（レビュー任せではない）。成立している理由は、境界がすべて型消去されているため:
//!
//! | 境界 | シグネチャ |
//! |---|---|
//! | [`SynHostStruct::fetch_suite`] | `(h, suite_id) -> *const c_void` |
//! | [`SynNodeDesc::get_extension`] | `(instance, ext_id) -> *const c_void` |
//!
//! 両層をつなぐのは不透明ハンドル（[`SynDeclBuilder`] / [`SynEvalCtx`]）だけで、
//! それらを操作する関数群は上位層にいる。
//!
//! # 境界規約（両側が守る不変条件）
//!
//!   - **unwind は越えない**: コールバック境界を越える巻き戻し（Rust panic / C++ 例外）は
//!     未定義ではなく **abort** になる（関数ポインタは `extern "C"`）。実装側は境界内で必ず
//!     捕捉し、[`SynStatus`]（`SYN_ERR_*`）へ変換して返すこと。`extern "C-unwind"` は採らない
//!     ——「ABI は C・エラーは戻り値」という規律を優先する（ADR 参照）。
//!   - **ポインタは明記なき限り非 NULL**: 関数ポインタ引数のうち、doc で「NULL 可」と書いた
//!     ものだけが NULL を許す（例: サイズ問い合わせの `out=NULL`/`cap=0`、任意コールバック）。
//!     それ以外に NULL を渡すのは契約違反。信頼境界（プラグイン→ホスト）の実装は防御的に
//!     NULL を検査して `SYN_ERR_BAD_ARG` を返してよい。
//!   - **関数ポインタは `unsafe`**: 全コールバックは `Option<unsafe extern "C" fn ...>`。
//!     呼び出しは健全性前提（有効な ctx/ポインタ・寿命）に依存するため、呼ぶ側に `unsafe` を
//!     強制する。NULL 可能性（`Option`）と未検証ポインタ（`unsafe`）が型に表れる。
//!
//! # 生成される C を「行儀のよい C」に保つための方針
//!
//!   - 型は `#[repr(C)]`。ハンドルは Nomicon 推奨のゼロサイズ struct + PhantomData
//!     （空 enum は uninhabited で参照を作ると即 UB のため不採用）にして、C 側は
//!     不完全型 `typedef struct Foo Foo;`（`Foo *` として使用）。
//!   - コールバックは `Option<unsafe extern "C" fn ...>`（= NULL 可能な C 関数ポインタ）。
//!   - 定数は `pub const` とし、C へは `#define` に落とす（enum の基底型は処理系定義なので
//!     ABI では `#define` が安全、という方針を Rust 側でも踏襲）。
//!
//! # 項目の追加
//!
//! 該当ファイルに宣言を書くだけでよい。Rust の公開面は下の glob 再エクスポートが拾い、
//! C ヘッダは `build.rs`（[`synapse_abi_buildgen`]）が `src/` を再帰走査して拾う。
//! 登録表の類は無い。
//!
//! [`synapse_abi_buildgen`]: ../../synapse-abi-buildgen/src/lib.rs

#![allow(non_camel_case_types)]
#![allow(non_snake_case)]
#![allow(dead_code)]
#![warn(missing_docs)]

// サブモジュールは crate 内限定にし、項目だけをクレート直下へ glob 再エクスポートする。
// 層内のモジュール構成は自由でよい（層の分類キーはディレクトリではなくクレート）。
pub(crate) mod handle;
pub(crate) mod module;
pub(crate) mod node;
pub(crate) mod status;
pub(crate) mod urid;
pub(crate) mod value;

pub use handle::*;
pub use module::*;
pub use node::*;
pub use status::*;
pub use urid::*;
pub use value::*;
