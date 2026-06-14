// 文字列 ABI 定数の正本。lib.rs（Rust 用）と build.rs（C の #define 生成用）の
// 両方から include! される。cbindgen は &str const を出力できないため、C 側へは
// build.rs がここから #define を生成して注入する。ここが唯一の定義箇所。

/// スイート ID: 型レジストリ。
pub const SYN_TYPE_REGISTRY_SUITE: &str = "synapse:type-registry";
/// スイート ID: URID。
pub const SYN_URID_SUITE: &str = "synapse:urid";
/// スイート ID: 宣言。
pub const SYN_DECL_SUITE: &str = "synapse:decl";
/// スイート ID: 評価。
pub const SYN_EVAL_SUITE: &str = "synapse:eval";
/// 拡張 ID: UI。
pub const SYN_EXT_UI: &str = "synapse:ext:ui";
/// モジュールがエクスポートすべきシンボル名。
pub const SYN_MODULE_ENTRY_SYMBOL: &str = "synapse_module";