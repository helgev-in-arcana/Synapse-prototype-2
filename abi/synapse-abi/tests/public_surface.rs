//! クレート直下のフラット再エクスポート面を固定する。
//!
//! `src/lib.rs` の再エクスポートは glob（`pub use synapse_abi_core::*;` 等）なので、
//! 層クレートへの追加漏れは起きない代わりに、**glob 同士の名前衝突で項目が黙って
//! 消える**という失敗モードがある（Rust の曖昧 glob はその名前を使うまでエラーに
//! ならない）。ここで全項目に一度ずつ触れて、消失を検出する。
//!
//! buildgen の `verify`（C ヘッダ側の出力漏れ検査）と対になる Rust 側の網。
//! ABI に項目を足したらここにも 1 行足すこと。

#![allow(unused_imports)]

use synapse_abi::*;

/// 型がクレート直下から参照できることを確認する（存在しなければコンパイルエラー）。
macro_rules! assert_exported {
    ($($t:ty),* $(,)?) => { $(const _: Option<&$t> = None;)* };
}

// ---- core: handle ----
assert_exported!(SynNode, SynDeclBuilder, SynEvalCtx);

// ---- core: status / urid / value ----
assert_exported!(SynStatus, SynUrid, SynTypeId, SynValue, SynValuePayload);

// ---- core: node / module ----
assert_exported!(SynNodeDesc, SynHostStruct, SynHost, SynModule, SynModuleEntryFn);

// ---- suite ----
assert_exported!(
    SynTypeVTable,
    SynTypeRegistrySuite,
    SynUridSuite,
    SynDeclSuite,
    SynRational,
    SynRequest,
    SynEvalSuite,
);

// ---- ext ----
assert_exported!(SynUiExt);

/// 定数がクレート直下から参照できること。型も同時に固定する。
#[test]
fn constants_are_exported_with_expected_values() {
    // status
    assert_eq!(SYN_OK, 0 as SynStatus);
    assert_eq!(SYN_ERR_UNKNOWN, -1);
    assert_eq!(SYN_ERR_UNSUPPORTED, -2);
    assert_eq!(SYN_ERR_BAD_ARG, -3);
    assert_eq!(SYN_ERR_NO_MEMORY, -4);
    assert_eq!(SYN_ERR_TYPE_MISMATCH, -5);
    assert_eq!((SYN_LOG_ERROR, SYN_LOG_WARN, SYN_LOG_INFO, SYN_LOG_DEBUG), (0, 1, 2, 3));

    // urid
    assert_eq!(SYN_URID_INVALID, 0 as SynUrid);
    assert_eq!(SYN_TYPE_ANY, 1 as SynTypeId);

    // value（v3: SVO インライン幅は 16。payload の実サイズと一致していること）
    assert_eq!(SYN_VALUE_INLINE, 16);
    assert_eq!(core::mem::size_of::<SynValuePayload>(), SYN_VALUE_INLINE);

    // node caps / port / type flags
    assert_eq!(SYN_CAP_REENTRANT_TILING, 1 << 0);
    assert_eq!(SYN_CAP_PARALLEL_FRAMES, 1 << 1);
    assert_eq!(SYN_CAP_THREAD_SAFE_UI, 1 << 2);
    assert_eq!(SYN_PORT_MULTI, 1 << 0);
    assert_eq!(SYN_TYPE_PLAIN_BYTES, 1 << 0);
    assert_eq!(SYN_TYPE_OPAQUE, 1 << 1);
    assert_eq!(SYN_TYPE_SHARED, 1 << 2);

    // module
    assert_eq!(SYN_ABI_VERSION, 3);

    // 文字列 ABI 定数（C 側 #define と同じ正本から出ている）
    assert_eq!(SYN_TYPE_REGISTRY_SUITE, "synapse:type-registry");
    assert_eq!(SYN_URID_SUITE, "synapse:urid");
    assert_eq!(SYN_DECL_SUITE, "synapse:decl");
    assert_eq!(SYN_EVAL_SUITE, "synapse:eval");
    assert_eq!(SYN_EXT_UI, "synapse:ext:ui");
    assert_eq!(SYN_MODULE_ENTRY_SYMBOL, "synapse_module");
}

/// ワイヤ表現のレイアウト不変条件（ADR-005 / ADR-006）。
#[test]
fn wire_layout_invariants() {
    // type_id 空間を安定させるため 32-bit（C ヘッダ側の static_assert と対）。
    assert_eq!(core::mem::size_of::<SynUrid>(), 4);
    // SVO の実体はポインタ幅ではなく定数 16。32/64-bit で挙動を揃えるための不変条件。
    assert_ne!(SYN_VALUE_INLINE, core::mem::size_of::<*const ()>());
}
