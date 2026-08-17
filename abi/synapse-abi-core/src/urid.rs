//! URID / 型 ID（基底層・スカラのみ）。
//!
//! URI と URID の相互変換そのものは `SynUridSuite`（suite 層）が担う。ここは
//! ワイヤに載る整数表現だけを定義する。

/// URI をセッション内で写像した安定整数。型 ID もこの空間。0 は無効。
pub type SynUrid = u32;
/// 型 ID（URID と同一空間）。
pub type SynTypeId = SynUrid;

/// 無効 URID。空値 sentinel（`type_id == 0`）でもある。
pub const SYN_URID_INVALID: SynUrid = 0;
/// 予約: 汎用ノードの出力/入力で「任意型」を表す（方式a）。
/// 接続判定で ANY は全許容。データは実体 type_id を運ぶ。
pub const SYN_TYPE_ANY: SynTypeId = 1;
