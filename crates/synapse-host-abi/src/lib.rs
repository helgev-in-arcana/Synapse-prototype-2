//! ホスト側 C-ABI 境界層。
//!
//! 責務は **C-ABI を安全な Rust に写すこと**だけ。扱う粒度は「個別モジュール」と
//! 「個別ノードインスタンス」まで。ノード同士の関係（リンク・評価順序・値の配線・
//! キャッシュ）は一切知らない——それは本体ホスト（FFI/unsafe 無しの上位クレート）の責務。
//!
//! 設計上の制約（ADR 参照）:
//!   - urid intern / 型レジストリ / ノード登録はプロセスグローバル（ADR-023, 1プロセス1
//!     セッション）。[`Session`] がその唯一の窓口。
//!   - [`OwnedValue`] は値渡し（ADR-022）。SVO の capture/present を内包する。
//!   - 同一インスタンスへの declare/negotiate/process は重ならない（ADR-019）。各メソッドが
//!     `&mut self` を取るため Rust の借用規則でコンパイル時に保証される。
//!
//! 制御の反転: 評価ループ（上流の充足）は本体ホストが回す。本層は negotiate で必要入力の
//! 一覧（[`Request`]）を返し、process には本体が用意した入力（[`InputBindings`]）を受け取る。
//!
//! # モジュール構成
//! 関心ごとに分割している。依存はおおむね上から下へ流れる:
//!   - [`ffi`]      … パニック遮断ガード・生ポインタ運搬・C 文字列変換（内部基盤）
//!   - [`error`]    … [`Error`] / [`Result`]（公開エラー型）
//!   - [`value`]    … [`OwnedValue`]（`SynValue` 値渡しの所有モデル）
//!   - [`decl`]     … [`NodeDecl`] と decl スイート（`declare` のバックエンド）
//!   - [`eval`]     … [`Request`] / [`InputBindings`] と eval スイート（`negotiate`/`process`）
//!   - [`session`]  … [`Session`]（プロセスグローバル: URID/型/ノード登録）
//!   - [`host`]     … `SynHostStruct` コールバック実装
//!   - [`module`]   … [`LoadedModule`] / [`NodeType`]（ロードと種別列挙）
//!   - [`node`]     … [`NodeInstance`]（インスタンス駆動の RAII）

#![warn(missing_docs)]

mod decl;
mod error;
mod eval;
mod ffi;
mod host;
mod module;
mod node;
mod session;
mod value;

pub use decl::{InputDecl, NodeDecl, OutputDecl};
pub use error::{Error, Result};
pub use eval::{InputBindings, Request};
pub use module::{LoadedModule, NodeType};
pub use node::NodeInstance;
pub use session::Session;
pub use value::OwnedValue;
