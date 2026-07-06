//! 評価モデル（`negotiate` / `process`）と eval スイート（プラグインが呼ぶ評価コールバック群）。
//!
//! # 意図
//! 評価ループ（上流の充足・評価順序）は**本体ホスト**が回す。本層は制御を反転して、
//! - `negotiate` で「どの入力リンクが要る」かの一覧（[`Request`]）を集め、
//! - `process` で本体が用意した入力（[`InputBindings`]）を読ませ、出力を受け取る。
//!
//! プラグインに渡す `SynEvalCtx*` の実体は内部の [`EvalScope`]。eval スイート
//! （[`EVAL_SUITE`]）の各コールバックはこのポインタを `EvalScope` として復元して働く。
//!
//! # 使い方
//! 本体ホストは [`InputBindings::new`] で束を作り、[`InputBindings::push_link`] で上流の
//! 評価結果をリンク順に詰め、[`crate::node::NodeInstance::process`] へ渡す。

use core::ffi::c_void;
use std::alloc::Layout;
use std::ptr::null_mut;

use synapse_abi::{
    SynEvalCtx, SynEvalSuite, SynRequest, SynStatus, SynTypeId, SynValue, SYN_ERR_BAD_ARG, SYN_OK,
    SYN_URID_INVALID,
};

use crate::ffi::{guard_or, guard_status};
use crate::session::Session;
use crate::value::OwnedValue;

/// `alloc` が返す型アラインメント準拠のホスト所有バッファ（process 終了まで生存）。
///
/// 型 vtable の `align` 属性（register_type で 2 の冪を検証済み, ADR-029）で確保する。
pub(crate) struct AlignedBuf {
    ptr: *mut u8,
    layout: Layout,
}

impl AlignedBuf {
    /// 確保して零初期化する。size==0・レイアウト不正・確保失敗は `None`。
    fn new(size: usize, align: usize) -> Option<Self> {
        let layout = Layout::from_size_align(size, align).ok()?;
        if layout.size() == 0 {
            return None;
        }
        // 安全性: layout.size() > 0 を確認済み。
        let ptr = unsafe { std::alloc::alloc_zeroed(layout) };
        if ptr.is_null() {
            return None;
        }
        Some(Self { ptr, layout })
    }
}

impl Drop for AlignedBuf {
    fn drop(&mut self) {
        // 安全性: ptr は同じ layout で alloc_zeroed した非 NULL ポインタ。
        unsafe { std::alloc::dealloc(self.ptr, self.layout) };
    }
}

/// `negotiate` が返す入力要求。`frame` は v1（単一フレーム）では無視する。
#[derive(Debug, Clone, Copy)]
pub struct Request {
    /// 要求する入力ポート（宣言順）。
    pub input_index: u32,
    /// multi-input 上のリンク番号（単一入力なら 0）。
    pub link_index: u32,
}

/// `process` に与える入力束。
///
/// 本体ホストが上流の評価結果をポートごと・リンク順に詰める。link_count は詰めたリンク数から
/// 導出される（multi-input 未接続は 0）。declare で既定値を持つ入力ポートは、接続なしのとき
/// その既定値が配送される。
pub struct InputBindings {
    links: Vec<Vec<OwnedValue>>,
    /// declare で既定値を持つ入力ポートの既定値（接続なしポートで配送）。
    defaults: Vec<Option<OwnedValue>>,
}

impl InputBindings {
    /// 入力ポート数を指定して空の束を作る。
    pub fn new(n_inputs: usize) -> Self {
        Self {
            links: (0..n_inputs).map(|_| Vec::new()).collect(),
            defaults: (0..n_inputs).map(|_| None).collect(),
        }
    }
    /// 入力ポートにリンク値を 1 本追加する（追加順がリンク順）。
    pub fn push_link(&mut self, input_index: usize, value: OwnedValue) {
        self.links[input_index].push(value);
    }

    /// 入力ポートの既定値を設定する（declare 結果から [`crate::node::NodeInstance`] が埋める）。
    pub(crate) fn set_default(&mut self, input_index: usize, value: OwnedValue) {
        if let Some(slot) = self.defaults.get_mut(input_index) {
            *slot = Some(value);
        }
    }

    pub(crate) fn link_count(&self, i: usize) -> u32 {
        self.links.get(i).map_or(0, |v| v.len() as u32)
    }
    fn get(&self, i: usize, l: usize) -> Option<&OwnedValue> {
        self.links.get(i).and_then(|v| v.get(l))
    }
    fn default(&self, i: usize) -> Option<&OwnedValue> {
        self.defaults.get(i).and_then(|o| o.as_ref())
    }
}

/// negotiate 呼び出しを表すモード値（`EvalScope::mode`）。
pub(crate) const MODE_NEGOTIATE: u8 = 0;
/// process 呼び出しを表すモード値（`EvalScope::mode`）。
pub(crate) const MODE_PROCESS: u8 = 1;

/// `SynEvalCtx*` の実体。`negotiate` / `process` のバックエンド状態。
///
/// `inputs` は process 中のみ非 NULL（negotiate では入力値は読めない）。`scratch` は
/// プラグインが alloc で確保した大型出力バッファを process 終了まで生かす受け皿。
pub(crate) struct EvalScope {
    #[allow(dead_code)]
    pub(crate) mode: u8,
    pub(crate) link_counts: Vec<u32>,
    pub(crate) requests: Vec<Request>,
    pub(crate) inputs: *const InputBindings, // process 中のみ非 NULL
    pub(crate) outputs: Vec<Option<OwnedValue>>,
    pub(crate) scratch: Vec<AlignedBuf>,
}

extern "C" fn ev_request(ctx: *mut SynEvalCtx, req: *const SynRequest) -> SynStatus {
    guard_status(|| {
        if ctx.is_null() || req.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        let r = unsafe { &*req };
        s.requests.push(Request {
            input_index: r.input_index,
            link_index: r.link_index,
        });
        SYN_OK
    })
}

extern "C" fn ev_link_count(ctx: *mut SynEvalCtx, input_index: u32) -> u32 {
    guard_or(0, || {
        if ctx.is_null() {
            return 0;
        }
        let s = unsafe { &*(ctx as *mut EvalScope) };
        s.link_counts.get(input_index as usize).copied().unwrap_or(0)
    })
}

extern "C" fn ev_get_input(ctx: *mut SynEvalCtx, input_index: u32, link_index: u32) -> SynValue {
    // 空値 sentinel（ADR-018: type_id==0）。SynValue は非 Copy なので都度組み立てる。
    fn empty() -> SynValue {
        SynValue {
            type_id: SYN_URID_INVALID,
            ptr: null_mut(),
            size: 0,
        }
    }
    guard_or(empty(), || {
        if ctx.is_null() {
            return empty();
        }
        let s = unsafe { &*(ctx as *mut EvalScope) };
        let ii = input_index as usize;
        if !s.inputs.is_null() {
            let inp = unsafe { &*s.inputs };
            if let Some(v) = inp.get(ii, link_index as usize) {
                return unsafe { v.to_value() };
            }
            // 接続なし: 既定値があれば配送、なければ空。
            if let Some(d) = inp.default(ii) {
                return unsafe { d.to_value() };
            }
        }
        empty()
    })
}

extern "C" fn ev_alloc(ctx: *mut SynEvalCtx, size: usize, t: SynTypeId) -> *mut c_void {
    guard_or(null_mut(), || {
        if ctx.is_null() {
            return null_mut();
        }
        // アラインメントは型の静的属性から引く（ADR-029）。未登録型は確保しない。
        let align = match Session::type_vtable(t) {
            Some(vt) => unsafe { (*vt).align },
            None => return null_mut(),
        };
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        match AlignedBuf::new(size, align) {
            Some(b) => {
                let p = b.ptr as *mut c_void;
                s.scratch.push(b);
                p
            }
            None => null_mut(),
        }
    })
}

extern "C" fn ev_set_output(ctx: *mut SynEvalCtx, output_index: u32, value: SynValue) -> SynStatus {
    guard_status(|| {
        if ctx.is_null() {
            return SYN_ERR_BAD_ARG;
        }
        let s = unsafe { &mut *(ctx as *mut EvalScope) };
        let oi = output_index as usize;
        if oi < s.outputs.len() {
            s.outputs[oi] = Some(unsafe { OwnedValue::from_value(&value) });
            SYN_OK
        } else {
            SYN_ERR_BAD_ARG
        }
    })
}

extern "C" fn ev_passthrough(
    ctx: *mut SynEvalCtx,
    output_index: u32,
    input_value: SynValue,
) -> SynStatus {
    ev_set_output(ctx, output_index, input_value)
}

/// プラグインの `negotiate` / `process` に渡す eval スイート（C-ABI コールバックの集合）。
pub(crate) static EVAL_SUITE: SynEvalSuite = SynEvalSuite {
    request: Some(ev_request),
    link_count: Some(ev_link_count),
    get_input: Some(ev_get_input),
    alloc: Some(ev_alloc),
    set_output: Some(ev_set_output),
    passthrough: Some(ev_passthrough),
};
