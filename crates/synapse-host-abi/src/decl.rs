//! 宣言（`declare`）の結果モデルと decl スイート（プラグインが呼ぶ宣言コールバック群）。
//!
//! # 意図
//! ノードは `declare` で自分の入出力ポート（key / 受理型 / フラグ / 既定値）を申告する。
//! 本モジュールは、その申告を受け取る C-ABI コールバック（[`DECL_SUITE`]）と、申告結果を
//! 安全な Rust 構造体（[`NodeDecl`] とその要素）として提供する。本体ホストはこの結果の
//! key / 型 / フラグを見て接続を解決する。
//!
//! # 仕組み
//! プラグインに渡す `SynDeclBuilder*` の実体は内部の [`DeclScope`]。decl スイートの各
//! コールバックはこのポインタを `DeclScope` として復元し、宣言を積む。`declare` 終了後、
//! [`crate::node::NodeInstance::declare`] が `DeclScope` を [`NodeDecl`] へ確定する。

use core::ffi::c_char;

use synapse_abi::{
    SynDeclBuilder, SynDeclSuite, SynStatus, SynTypeId, SynValue, SYN_ERR_BAD_ARG, SYN_OK,
    SYN_PORT_MULTI, SYN_TYPE_ANY,
};

use crate::ffi::{cstr_to_string, guard_status};
use crate::value::OwnedValue;

/// 入力ポートの宣言。
#[derive(Debug)]
pub struct InputDecl {
    /// 論理同一性を担う安定 key（再 declare を跨いで安定）。
    pub key: String,
    /// 受理する型集合（多態）。
    pub types: Vec<SynTypeId>,
    /// `SYN_PORT_*` フラグ（multi-input 等）。
    pub flags: u32,
    /// 既定値を持つか。
    pub has_default: bool,
}

/// 出力ポートの宣言。
#[derive(Debug)]
pub struct OutputDecl {
    /// 安定 key。
    pub key: String,
    /// 出力型（ちょうど 1 つ）。
    pub ty: SynTypeId,
}

/// `declare` の結果。本体ホストはこの key / 型 / flags を見て接続を解決する。
#[derive(Debug)]
pub struct NodeDecl {
    /// 入力ポート（宣言順）。
    pub inputs: Vec<InputDecl>,
    /// 出力ポート（宣言順）。
    pub outputs: Vec<OutputDecl>,
}

impl NodeDecl {
    /// key から入力インデックスを引く。
    pub fn input_index(&self, key: &str) -> Option<u32> {
        self.inputs.iter().position(|p| p.key == key).map(|i| i as u32)
    }
    /// key から出力インデックスを引く。
    pub fn output_index(&self, key: &str) -> Option<u32> {
        self.outputs.iter().position(|p| p.key == key).map(|i| i as u32)
    }
    /// 指定入力ポートが multi-input（fan-in）か。
    pub fn is_multi(&self, input_index: u32) -> bool {
        self.inputs
            .get(input_index as usize)
            .is_some_and(|p| p.flags & SYN_PORT_MULTI != 0)
    }
}

/// `declare` 中にビルダへ積まれる内部状態。
///
/// `SynDeclBuilder*` の実体としてプラグインへ渡される。既定値は実体（[`OwnedValue`]）を
/// 入力インデックスとともに保持し、後段の [`crate::node::NodeInstance`] が process 時の
/// 既定値配送に使う。
pub(crate) struct DeclScope {
    pub(crate) inputs: Vec<InputDecl>,
    pub(crate) outputs: Vec<OutputDecl>,
    pub(crate) defaults: Vec<(usize, OwnedValue)>, // (input_index, default)
}

extern "C" fn decl_output(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    ty: SynTypeId,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        s.outputs.push(OutputDecl {
            key: cstr_to_string(key),
            ty,
        });
        SYN_OK
    })
}

extern "C" fn decl_input(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    _label: *const c_char,
    types: *const SynTypeId,
    n_types: usize,
    flags: u32,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        let ts = if types.is_null() || n_types == 0 {
            Vec::new()
        } else {
            unsafe { core::slice::from_raw_parts(types, n_types) }.to_vec()
        };
        s.inputs.push(InputDecl {
            key: cstr_to_string(key),
            types: ts,
            flags,
            has_default: false,
        });
        SYN_OK
    })
}

extern "C" fn decl_connected_type(
    _b: *mut SynDeclBuilder,
    _input_key: *const c_char,
    _link_index: u32,
) -> SynTypeId {
    SYN_TYPE_ANY // 方式a: v1 は常に ANY
}

extern "C" fn decl_input_default(
    b: *mut SynDeclBuilder,
    key: *const c_char,
    value: SynValue,
) -> SynStatus {
    guard_status(|| {
        if b.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(b as *mut DeclScope) };
        let k = cstr_to_string(key);
        if let Some(idx) = s.inputs.iter().position(|p| p.key == k) {
            s.inputs[idx].has_default = true;
            s.defaults.push((idx, unsafe { OwnedValue::from_value(&value) }));
            SYN_OK
        } else {
            SYN_ERR_BAD_ARG
        }
    })
}

/// プラグインの `declare` に渡す decl スイート（C-ABI コールバックの集合）。
pub(crate) static DECL_SUITE: SynDeclSuite = SynDeclSuite {
    output: Some(decl_output),
    input: Some(decl_input),
    connected_type: Some(decl_connected_type),
    input_default: Some(decl_input_default),
};
