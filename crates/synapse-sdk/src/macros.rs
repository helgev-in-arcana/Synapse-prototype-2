//! 公開マクロ [`synapse_module!`]。
//!
//! モジュールエントリ（`synapse_module` シンボル）・型登録・ノード登録を一括生成する。作者は
//! このマクロを 1 度呼ぶだけで、ホストがロードできる完全なプラグインモジュールになる。

/// モジュールエントリ・型登録・ノード登録を一括生成する。
///
/// 展開されるもの:
///   - `#[no_mangle] extern "C" fn synapse_module()` … ホストが探すエントリシンボル
///   - `__synapse_register_types` … スイート取得
///     （[`__on_register_types_begin`](crate::__on_register_types_begin)）→ `types` の型登録
///   - `__synapse_register_nodes` … `nodes` のノード登録（全モジュールの型登録後に呼ばれる。
///     2フェーズロード＝ADR-027）
///   - モジュール記述子 `static MODULE` … モジュールイメージ内 static（dlclose で解放される）
///
/// ```ignore
/// synapse_module! {
///     uri: c"com.vendor.module",
///     version: c"0.1.0",
///     types: [f32],
///     nodes: [Const, Add],
/// }
/// ```
#[macro_export]
macro_rules! synapse_module {
    (
        uri: $uri:expr,
        version: $ver:expr,
        types: [$($ty:ty),* $(,)?],
        nodes: [$($node:ty),* $(,)?] $(,)?
    ) => {
        #[no_mangle]
        pub extern "C" fn synapse_module() -> *const $crate::abi::SynModule {
            // フェーズ1: スイート取得＋型登録（他モジュールの型に依存しない＝ADR-028）。
            extern "C" fn __synapse_register_types(
                h: $crate::abi::SynHost,
            ) -> $crate::abi::SynStatus {
                $crate::__on_register_types_begin(h);
                $( $crate::__register_type::<$ty>(h); )*
                $crate::abi::SYN_OK
            }
            // フェーズ2: ノード登録（全モジュールの型登録後にホストが呼ぶ）。
            extern "C" fn __synapse_register_nodes(
                h: $crate::abi::SynHost,
            ) -> $crate::abi::SynStatus {
                $( $crate::__register_node::<$node>(h); )*
                $crate::abi::SYN_OK
            }
            // モジュール記述子もモジュールイメージ内 static に置く（dlclose で解放される）。
            // 生ポインタを含むため Sync ラッパ経由。`&MODULE.0` は static への 'static 参照。
            static MODULE: $crate::SyncModule = $crate::SyncModule($crate::abi::SynModule {
                abi_version: $crate::abi::SYN_ABI_VERSION,
                module_uri: $uri.as_ptr(),
                module_version: $ver.as_ptr(),
                on_register_types: ::core::option::Option::Some(__synapse_register_types),
                on_register_nodes: ::core::option::Option::Some(__synapse_register_nodes),
                on_unload: ::core::option::Option::Some($crate::__on_unload as _),
            });
            &MODULE.0
        }
    };
}
