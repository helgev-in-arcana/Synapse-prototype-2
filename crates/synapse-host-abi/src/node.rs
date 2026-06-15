//! ノードインスタンス（RAII で `create` ⇄ `destroy` を対にする）。
//!
//! # 意図
//! 1 つのノードインスタンスに対する declare / negotiate / process / save / load の呼び出しを
//! 安全な Rust メソッドとして提供する。各メソッドは `&mut self` を取るため、同一インスタンス
//! への呼び出しの非重複（ADR-019）が借用規則でコンパイル時に保証される。declare で得た
//! 既定値は内部に保持し、[`NodeInstance::make_input_bindings`] が process 用の入力束へ埋める。
//!
//! # 典型的な使い方
//! ```ignore
//! let mut node = module.instantiate(ty)?;
//! let decl = node.declare()?;            // ポート構成を得る
//! let reqs = node.negotiate(&link_counts)?; // 必要な入力リンクを問い合わせ
//! let mut inputs = node.make_input_bindings()?;
//! // inputs.push_link(...) で上流の評価結果を詰める
//! let outputs = node.process(&inputs)?;  // 出力スロットを得る
//! ```

use core::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::null_mut;

use synapse_abi::SynNodeDesc;

use crate::decl::{DeclScope, NodeDecl};
use crate::error::{check, Error, Result};
use crate::eval::{EvalScope, InputBindings, Request, MODE_NEGOTIATE, MODE_PROCESS};
use crate::module::LoadedModule;
use crate::value::OwnedValue;

/// 1 ノードインスタンス。Drop で `destroy` を呼ぶ。
///
/// declare / negotiate / process が `&mut self` を取るため、同一インスタンスへの呼び出しの
/// 非重複（ADR-019）が借用規則で保証される。`'m` でモジュールより長生きしないことを強制。
pub struct NodeInstance<'m> {
    desc: *const SynNodeDesc,
    instance: *mut c_void,
    decl: Option<NodeDecl>,
    /// declare で得た入力ポートごとの既定値（make_input_bindings で配る）。
    declared_defaults: Vec<Option<OwnedValue>>,
    _m: PhantomData<&'m LoadedModule>,
}

impl<'m> NodeInstance<'m> {
    /// 生成済みインスタンスポインタからラッパを組み立てる（[`LoadedModule::instantiate`] 専用）。
    pub(crate) fn new(desc: *const SynNodeDesc, instance: *mut c_void) -> Self {
        Self {
            desc,
            instance,
            decl: None,
            declared_defaults: Vec::new(),
            _m: PhantomData,
        }
    }
}

impl NodeInstance<'_> {
    fn desc(&self) -> &SynNodeDesc {
        unsafe { &*self.desc }
    }

    /// `declare` を実行し、宣言結果（key / 型 / flags / 既定値の有無）を返す。
    ///
    /// 結果は内部にも保持する。既定値の実体は process 時に配送するため、入力ポートごとに保持し
    /// [`NodeInstance::make_input_bindings`] が既定値を埋めた入力束を返す。
    pub fn declare(&mut self) -> Result<&NodeDecl> {
        let declare = self.desc().declare.ok_or(Error::NullCallback("declare"))?;
        let mut scope = DeclScope {
            inputs: Vec::new(),
            outputs: Vec::new(),
            defaults: Vec::new(),
        };
        check(unsafe { declare(self.instance, (&mut scope as *mut DeclScope).cast()) })?;
        // 既定値を input ごとに格納（process の InputBindings 構築に使う）。
        let mut defaults: Vec<Option<OwnedValue>> = (0..scope.inputs.len()).map(|_| None).collect();
        for (idx, val) in scope.defaults {
            defaults[idx] = Some(val);
        }
        self.decl = Some(NodeDecl {
            inputs: scope.inputs,
            outputs: scope.outputs,
        });
        self.declared_defaults = defaults;
        Ok(self.decl.as_ref().unwrap())
    }

    /// 直近の declare 結果。
    pub fn decl(&self) -> Option<&NodeDecl> {
        self.decl.as_ref()
    }

    /// 宣言済み既定値を埋めた入力束を作る。
    ///
    /// 本体ホストはここへ接続リンクを push（[`InputBindings::push_link`]）して process に渡す。
    /// declare 前に呼ぶと [`Error::NotDeclared`]。
    pub fn make_input_bindings(&self) -> Result<InputBindings> {
        let decl = self.decl.as_ref().ok_or(Error::NotDeclared)?;
        let mut b = InputBindings::new(decl.inputs.len());
        for (i, d) in self.declared_defaults.iter().enumerate() {
            if let Some(v) = d {
                b.set_default(i, v.clone());
            }
        }
        Ok(b)
    }

    /// `negotiate` を実行し、必要入力の一覧を返す。
    ///
    /// `link_counts` は接続トポロジ（本体ホストが知る各入力ポートのリンク数）。declare 前に
    /// 呼ぶと [`Error::NotDeclared`]。
    pub fn negotiate(&mut self, link_counts: &[u32]) -> Result<Vec<Request>> {
        if self.decl.is_none() {
            return Err(Error::NotDeclared);
        }
        let negotiate = self
            .desc()
            .negotiate
            .ok_or(Error::NullCallback("negotiate"))?;
        let mut scope = EvalScope {
            mode: MODE_NEGOTIATE,
            link_counts: link_counts.to_vec(),
            requests: Vec::new(),
            inputs: core::ptr::null(),
            outputs: Vec::new(),
            scratch: Vec::new(),
        };
        check(unsafe { negotiate(self.instance, (&mut scope as *mut EvalScope).cast()) })?;
        Ok(scope.requests)
    }

    /// `process` を実行する。
    ///
    /// 入力束 `inputs` の各ポートのリンク数が link_count としてプラグインへ渡る。出力スロット
    /// （宣言した出力数。未書き込みは `None`）を返す。declare 前に呼ぶと [`Error::NotDeclared`]。
    pub fn process(&mut self, inputs: &InputBindings) -> Result<Vec<Option<OwnedValue>>> {
        let decl = self.decl.as_ref().ok_or(Error::NotDeclared)?;
        let n_out = decl.outputs.len();
        let link_counts: Vec<u32> = (0..decl.inputs.len())
            .map(|i| inputs.link_count(i))
            .collect();
        let process = unsafe { &*self.desc }
            .process
            .ok_or(Error::NullCallback("process"))?;
        let mut scope = EvalScope {
            mode: MODE_PROCESS,
            link_counts,
            requests: Vec::new(),
            inputs: inputs as *const InputBindings,
            outputs: (0..n_out).map(|_| None).collect(),
            scratch: Vec::new(),
        };
        check(unsafe { process(self.instance, (&mut scope as *mut EvalScope).cast()) })?;
        Ok(scope.outputs)
    }

    /// 内部状態を保存する（2 段サイズ問い合わせを内包）。状態が無ければ `None`。
    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        let save = match self.desc().save_state {
            Some(f) => f,
            None => return Ok(None),
        };
        let mut written: usize = 0;
        check(unsafe { save(self.instance, null_mut(), 0, &mut written) })?;
        if written == 0 {
            return Ok(None);
        }
        let mut buf = vec![0u8; written];
        check(unsafe {
            save(
                self.instance,
                buf.as_mut_ptr() as *mut c_void,
                written,
                &mut written,
            )
        })?;
        buf.truncate(written);
        Ok(Some(buf))
    }

    /// 内部状態を復元する。
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<()> {
        let load = match self.desc().load_state {
            Some(f) => f,
            None => return Ok(()),
        };
        check(unsafe { load(self.instance, bytes.as_ptr() as *const c_void, bytes.len()) })
    }
}

impl Drop for NodeInstance<'_> {
    fn drop(&mut self) {
        if let Some(destroy) = self.desc().destroy {
            unsafe { destroy(self.instance) };
        }
    }
}
