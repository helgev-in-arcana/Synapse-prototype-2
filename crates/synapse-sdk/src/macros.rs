//! 公開マクロ [`synapse_module!`]。
//!
//! モジュールエントリ（`synapse_module` シンボル）・型登録・ノード登録を一括生成する。作者は
//! このマクロを 1 度呼ぶだけで、ホストがロードできる完全なプラグインモジュールになる。

/// モジュールエントリ・型登録・ノード登録を一括生成する。
///
/// 展開されるもの:
///   - `#[no_mangle] extern "C" fn synapse_module()` … ホストが探すエントリシンボル
///   - `__synapse_on_load` … スイート取得（[`__on_load_begin`](crate::__on_load_begin)）→
///     `types` の型登録 → `nodes` のノード登録
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
            extern "C" fn __synapse_on_load(h: $crate::abi::SynHost) -> $crate::abi::SynStatus {
                $crate::__on_load_begin(h);
                $( $crate::__register_type::<$ty>(h); )*
                $( $crate::__register_node::<$node>(h); )*
                $crate::abi::SYN_OK
            }
            // モジュール記述子もモジュールイメージ内 static に置く（dlclose で解放される）。
            // 生ポインタを含むため Sync ラッパ経由。`&MODULE.0` は static への 'static 参照。
            static MODULE: $crate::SyncModule = $crate::SyncModule($crate::abi::SynModule {
                abi_version: $crate::abi::SYN_ABI_VERSION,
                module_uri: $uri.as_ptr(),
                module_version: $ver.as_ptr(),
                on_load: ::core::option::Option::Some(__synapse_on_load),
                on_unload: ::core::option::Option::Some($crate::__on_unload as _),
            });
            &MODULE.0
        }
    };
}
