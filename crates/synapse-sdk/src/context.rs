//! 作者向けコンテキスト（`Declarer` / `ProcessCtx` / `NegotiateCtx`）。
//!
//! # 意図
//! [`Node`](crate::Node) の各フックに渡されるアクセサ。インデックス規約・既定値配送・SVO
//! 変換・negotiate の縮退形といった ABI の作法をここに隠し、作者は型安全なポートトークンと
//! 素の Rust 値だけを扱えばよいようにする。
//!
//! - [`Declarer`]     … `declare` 内でポートを宣言する（[`InPort`] 等を返す）。
//! - [`ProcessCtx`]   … `process` 内で入力を読み（`get` / `get_link`）出力を書く（`set`）。
//! - [`NegotiateCtx`] … `negotiate` 内で必要入力を要求する（既定 [`NegotiateCtx::request_all`]）。

use core::ffi::{c_void, CStr};
use core::marker::PhantomData;

use synapse_abi::{
    SynDeclBuilder, SynEvalCtx, SynRational, SynRequest, SynTypeId, SynValue, SYN_PORT_MULTI,
    SYN_URID_INVALID,
};

use crate::error::{Error, Result};
use crate::plain::{svo_value, value_to_plain, SynPlainType, PTR_SIZE};
use crate::port::{InPort, MultiInPort, OutPort};
use crate::suites::{decl_suite, eval_suite, urid_of};

/// `declare` 内でポートを宣言するビルダ。インデックス規約・既定値配送を隠す。
///
/// `input*` / `output` を呼んだ順がポート順（= インデックス）になる。
pub struct Declarer {
    b: *mut SynDeclBuilder,
    pub(crate) n_inputs: u32,
    n_outputs: u32,
}

impl Declarer {
    pub(crate) fn new(b: *mut SynDeclBuilder) -> Self {
        Self {
            b,
            n_inputs: 0,
            n_outputs: 0,
        }
    }

    /// 既定値つき単一入力。未接続時は既定値が配送される。
    pub fn input<T: SynPlainType>(&mut self, key: &CStr, label: &CStr, default: T) -> InPort<T> {
        let (idx, type_id) = self.declare_input::<T>(key, label, 0);
        unsafe {
            // 既定値の値渡し: ≤ptr は SVO、>ptr は呼び出し中のみ有効な借用（ホストが clone する）。
            // `default` はこのメソッドのフレームに生存し input_default 呼び出し中ずっと有効。
            let v = if T::SIZE <= PTR_SIZE {
                svo_value::<T>(type_id, &default)
            } else {
                SynValue {
                    type_id,
                    ptr: &default as *const T as *mut c_void,
                    size: T::SIZE,
                }
            };
            (decl_suite().input_default.unwrap())(self.b, key.as_ptr(), v);
        }
        InPort {
            index: idx,
            type_id,
            _t: PhantomData,
        }
    }

    /// 既定値なし単一入力。未接続時は空値（`get` が `None`）。
    pub fn input_opt<T: SynPlainType>(&mut self, key: &CStr, label: &CStr) -> InPort<T> {
        let (idx, type_id) = self.declare_input::<T>(key, label, 0);
        InPort {
            index: idx,
            type_id,
            _t: PhantomData,
        }
    }

    /// fan-in 入力（N リンク受理）。既定値は持たない。
    pub fn input_multi<T: SynPlainType>(&mut self, key: &CStr, label: &CStr) -> MultiInPort<T> {
        let (idx, type_id) = self.declare_input::<T>(key, label, SYN_PORT_MULTI);
        MultiInPort {
            index: idx,
            type_id,
            _t: PhantomData,
        }
    }

    /// 出力ポート。
    pub fn output<T: SynPlainType>(&mut self, key: &CStr, label: &CStr) -> OutPort<T> {
        let idx = self.n_outputs;
        self.n_outputs += 1;
        let type_id = urid_of(T::URI);
        unsafe {
            (decl_suite().output.unwrap())(self.b, key.as_ptr(), label.as_ptr(), type_id);
        }
        OutPort {
            index: idx,
            type_id,
            _t: PhantomData,
        }
    }

    fn declare_input<T: SynPlainType>(
        &mut self,
        key: &CStr,
        label: &CStr,
        flags: u32,
    ) -> (u32, SynTypeId) {
        let idx = self.n_inputs;
        self.n_inputs += 1;
        let type_id = urid_of(T::URI);
        unsafe {
            let types = [type_id];
            (decl_suite().input.unwrap())(
                self.b,
                key.as_ptr(),
                label.as_ptr(),
                types.as_ptr(),
                1,
                flags,
            );
        }
        (idx, type_id)
    }
}

/// `process` 内の入出力アクセス。
pub struct ProcessCtx {
    ctx: *mut SynEvalCtx,
}

impl ProcessCtx {
    pub(crate) fn new(ctx: *mut SynEvalCtx) -> Self {
        Self { ctx }
    }

    /// 入力リンクを読む共通処理。空値・型不一致・サイズ不一致はすべて `None` に倒す。
    ///
    /// 型 ID は declare 時に解決した `port_type_id` と照合する（ANY 宣言・上流バグで異型が
    /// 届いても範囲外読みしない）。
    ///
    /// # Safety
    /// `self.ctx` は process 中の妥当な `SynEvalCtx`。
    unsafe fn read<T: SynPlainType>(
        &self,
        index: u32,
        link: u32,
        port_type_id: SynTypeId,
    ) -> Option<T> {
        let v = (eval_suite().get_input.unwrap())(self.ctx, index, link);
        if v.type_id == SYN_URID_INVALID || v.type_id != port_type_id {
            return None;
        }
        value_to_plain::<T>(&v)
    }

    /// 単一入力を読む。空値（未接続かつ既定値なし）・型不一致は `None`。
    pub fn get<T: SynPlainType>(&self, port: InPort<T>) -> Option<T> {
        unsafe { self.read::<T>(port.index, 0, port.type_id) }
    }

    /// 単一入力を読む（型不一致を区別したい場合）。
    ///
    /// 空値 → `Ok(None)`、型不一致 → `Err(TypeMismatch)`、一致 → `Ok(Some(v))`。
    pub fn get_checked<T: SynPlainType>(&self, port: InPort<T>) -> Result<Option<T>> {
        unsafe {
            let v = (eval_suite().get_input.unwrap())(self.ctx, port.index, 0);
            if v.type_id == SYN_URID_INVALID {
                Ok(None)
            } else if v.type_id != port.type_id {
                Err(Error::TypeMismatch)
            } else {
                Ok(value_to_plain::<T>(&v))
            }
        }
    }

    /// 出力を書く（値渡し）。
    pub fn set<T: SynPlainType>(&mut self, port: OutPort<T>, value: T) {
        unsafe {
            let e = eval_suite();
            // ≤ptr は SVO（ptr フィールドにインライン）。>ptr は出力が下流に保持されるため、
            // ホスト確保バッファ（ADR-012）に書いてその ptr を渡す。`T::SIZE <= PTR_SIZE` は
            // const なので 64-bit の常用型では分岐ごと消える（ゼロコスト）。
            let v = if T::SIZE <= PTR_SIZE {
                svo_value::<T>(port.type_id, &value)
            } else {
                let buf = (e.alloc.unwrap())(self.ctx, T::SIZE, port.type_id);
                if buf.is_null() {
                    return; // 確保失敗: 出力を書かない（堅牢性: パニックしない）。
                }
                core::ptr::copy_nonoverlapping(
                    &value as *const T as *const u8,
                    buf as *mut u8,
                    T::SIZE,
                );
                SynValue {
                    type_id: port.type_id,
                    ptr: buf,
                    size: T::SIZE,
                }
            };
            (e.set_output.unwrap())(self.ctx, port.index, v);
        }
    }

    /// fan-in ポートのリンク数。
    pub fn link_count<T>(&self, port: MultiInPort<T>) -> u32 {
        unsafe { (eval_suite().link_count.unwrap())(self.ctx, port.index) }
    }

    /// fan-in ポートの `link` 番目のリンク値。空値・型不一致は `None`。
    pub fn get_link<T: SynPlainType>(&self, port: MultiInPort<T>, link: u32) -> Option<T> {
        unsafe { self.read::<T>(port.index, link, port.type_id) }
    }
}

/// `negotiate` 内のコンテキスト。既定の [`NegotiateCtx::request_all`] で全入力を列挙する。
pub struct NegotiateCtx {
    ctx: *mut SynEvalCtx,
    n_inputs: u32,
}

impl NegotiateCtx {
    pub(crate) fn new(ctx: *mut SynEvalCtx, n_inputs: u32) -> Self {
        Self { ctx, n_inputs }
    }

    /// 宣言済み全入力ポートの全リンクを要求する（静的ノードの縮退 negotiate）。
    pub fn request_all(&mut self) {
        unsafe {
            let e = eval_suite();
            for i in 0..self.n_inputs {
                let n = (e.link_count.unwrap())(self.ctx, i);
                for l in 0..n {
                    let req = SynRequest {
                        input_index: i,
                        link_index: l,
                        frame: SynRational { num: 0, den: 1 },
                    };
                    (e.request.unwrap())(self.ctx, &req);
                }
            }
        }
    }
}
