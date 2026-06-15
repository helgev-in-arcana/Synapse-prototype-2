//! 型安全なポートトークン（`InPort` / `OutPort` / `MultiInPort`）。
//!
//! # 意図
//! `declare` で確定したポートを、値型 `T` を型パラメータに持つ軽量トークンとして表す。作者は
//! このトークンを `process` のアクセサ（`ctx.get` / `ctx.set` / `ctx.get_link`）へ渡すだけで、
//! インデックスや URID を意識せずに済む。トークンは `Copy` なのでフィールドに保持して使い回せる。
//!
//! 各トークンは declare 時に解決した実体型の URID（`type_id`）を内包し、process 時の型照合に
//! 使う（ANY 宣言や上流バグで異型が届いても範囲外読みを起こさない）。

use core::marker::PhantomData;

use synapse_abi::SynTypeId;

/// `InPort` / `OutPort` / `MultiInPort` を同型に生成するための内部マクロ。
macro_rules! define_port {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $name<T> {
            pub(crate) index: u32,
            /// declare 時に解決した実体型の URID。process 時の型照合に使う。
            pub(crate) type_id: SynTypeId,
            pub(crate) _t: PhantomData<T>,
        }
        impl<T> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T> Copy for $name<T> {}
        impl<T> Default for $name<T> {
            fn default() -> Self {
                Self {
                    index: 0,
                    type_id: $crate::abi::SYN_URID_INVALID,
                    _t: PhantomData,
                }
            }
        }
    };
}

define_port!(InPort, "単一入力ポート。`declare` で確定し `process` で `ctx.get` に渡す。");
define_port!(OutPort, "出力ポート。`process` で `ctx.set` に渡す。");
define_port!(MultiInPort, "fan-in 入力ポート（N リンク）。`ctx.link_count` / `ctx.get_link`。");
