//! UI 拡張（`SYN_EXT_UI`）。
//!
//! 拡張は `SynNodeDesc::get_extension(instance, ext_id)` で引く（CLAP 流）。基底 ABI にも
//! スイートにも触れずに機能を足せる穴なので、ここは独立した層として置く。

use core::ffi::{c_char, c_void};

use synapse_abi_core::SynStatus;

/// UI コンポーネント拡張。
#[repr(C)]
pub struct SynUiExt {
    /// UI を構築する（host_ui_handle はホスト側 UI コンテキスト）。
    pub build: Option<unsafe extern "C" fn(instance: *mut c_void, host_ui_handle: *mut c_void) -> SynStatus>,
    /// パラメータ変更の通知（param_key は変更されたソケット key）。
    pub on_change: Option<unsafe extern "C" fn(instance: *mut c_void, param_key: *const c_char) -> SynStatus>,
}

/// 拡張 ID: UI。
pub const SYN_EXT_UI: &str = "synapse:ext:ui";
