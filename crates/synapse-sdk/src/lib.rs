//! プラグイン側 SDK。
//!
//! 作者は [`Node`] を実装し [`synapse_module!`] で公開するだけ。SDK が以下を隠す:
//!   - スイートの fetch とモジュールグローバル保持（FINDINGS F-3）
//!   - `SynValue` ⇔ Rust 型の SVO 変換（値渡し: ADR-022）
//!   - declare のインデックス規約・既定値配送・negotiate の縮退形（ADR-011）
//!   - save/load の 2 段サイズ問い合わせプロトコル
//!   - FFI 境界を越えるパニックの遮断（`catch_unwind` → エラーステータス）
//!
//! 作者のコードに unsafe / グローバル / negotiate は現れない。

#![allow(clippy::missing_safety_doc)]
#![warn(missing_docs)]

use core::ffi::{c_void, CStr};
use core::marker::PhantomData;
use std::sync::OnceLock;

/// ABI 型を再エクスポート（マクロが `$crate::abi::...` で参照する）。
pub use synapse_abi as abi;

use synapse_abi::*;

/* ======================================================================= */
/*  PLAIN 型（SynValue ⇔ Rust 型）                                          */
/* ======================================================================= */

/// memcpy 可能な固定サイズ PLAIN 型。v1 では ≤8byte（SVO）のみ。
///
/// # Safety
/// `SIZE == size_of::<Self>()` かつ任意のビットパターンが妥当（Copy・no padding 推奨）。
pub unsafe trait SynPlainType: Copy + 'static {
    /// この型の URI（例 `c"synapse:float"`）。URID intern と型登録に使う。
    const URI: &'static CStr;
    /// バイトサイズ（`size_of::<Self>()`）。
    const SIZE: usize;
    /// 健全性チェック: `SIZE` は `size_of::<Self>()` と一致しなければならない。
    /// 値変換コード（[`svo_value`]/[`value_to_plain`]）が参照するので、不一致なら
    /// その型を使った瞬間にコンパイルエラーになる（サイズ ≤/> ポインタ幅は問わない＝
    /// 32-bit でも 8byte 型を正しく扱う）。
    #[doc(hidden)]
    const _SIZE_CHECK: () = assert!(
        Self::SIZE == core::mem::size_of::<Self>(),
        "SynPlainType::SIZE は size_of::<Self>() と一致させること"
    );
}

unsafe impl SynPlainType for f32 {
    const URI: &'static CStr = c"synapse:float";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for f64 {
    const URI: &'static CStr = c"synapse:double";
    const SIZE: usize = 8;
}
unsafe impl SynPlainType for i32 {
    const URI: &'static CStr = c"synapse:int";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for u32 {
    const URI: &'static CStr = c"synapse:uint";
    const SIZE: usize = 4;
}
unsafe impl SynPlainType for i64 {
    const URI: &'static CStr = c"synapse:long";
    const SIZE: usize = 8;
}
unsafe impl SynPlainType for u64 {
    const URI: &'static CStr = c"synapse:ulong";
    const SIZE: usize = 8;
}

const PTR_SIZE: usize = core::mem::size_of::<*mut c_void>();

/// ≤ptr の値を SynValue（SVO: 値を ptr フィールドにインライン格納）へ組み立てる。
/// 出力・既定値の「≤ptr 経路」で共用。`v` は呼び出し側に生きている参照（コピーするだけ）。
unsafe fn svo_value<T: SynPlainType>(type_id: SynTypeId, v: &T) -> SynValue {
    let () = T::_SIZE_CHECK; // SIZE == size_of を強制（post-mono でチェック）
    let mut bits: usize = 0;
    core::ptr::copy_nonoverlapping(
        v as *const T as *const u8,
        (&mut bits as *mut usize) as *mut u8,
        T::SIZE,
    );
    SynValue {
        type_id,
        ptr: bits as *mut c_void,
        size: T::SIZE,
    }
}

/// SynValue を Rust 値へ。サイズ不一致は `None`（>ptr 異型の範囲外読みを遮断）。
/// `T::SIZE <= ptr` か否かで SVO（ptr フィールド）/領域（*ptr）を出し分ける（32-bit でも正しい）。
/// 型 ID の照合は呼び出し側（[`ProcessCtx::get`] 等が `InPort` の type_id と突合）が行う。
unsafe fn value_to_plain<T: SynPlainType>(v: &SynValue) -> Option<T> {
    let () = T::_SIZE_CHECK; // SIZE == size_of を強制（post-mono でチェック）
    if v.size != T::SIZE {
        return None;
    }
    let mut out = core::mem::MaybeUninit::<T>::uninit();
    let src: *const u8 = if T::SIZE <= PTR_SIZE {
        (&v.ptr as *const *mut c_void) as *const u8
    } else {
        v.ptr as *const u8
    };
    core::ptr::copy_nonoverlapping(src, out.as_mut_ptr() as *mut u8, T::SIZE);
    Some(out.assume_init())
}

/* ======================================================================= */
/*  エラー                                                                 */
/* ======================================================================= */

/// ノード処理が返しうるエラー。`SynStatus` へ変換されて ABI 境界を越える。
#[derive(Debug)]
pub enum Error {
    /// 入力値の型が期待と異なる。
    TypeMismatch,
    /// 内部状態が不正（load_state の入力長不足など）。
    BadState,
    /// 任意のステータスコードを直接返す。
    Status(SynStatus),
}

impl Error {
    fn to_status(&self) -> SynStatus {
        match self {
            Error::TypeMismatch => SYN_ERR_TYPE_MISMATCH,
            Error::BadState => SYN_ERR_BAD_ARG,
            Error::Status(s) => *s,
        }
    }
}

/// SDK 共通の結果型。
pub type Result<T> = core::result::Result<T, Error>;

/* ======================================================================= */
/*  ポートトークン（型安全・Copy）                                         */
/* ======================================================================= */

macro_rules! define_port {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        pub struct $name<T> {
            index: u32,
            /// declare 時に解決した実体型の URID。process 時の型照合に使う。
            type_id: SynTypeId,
            _t: PhantomData<T>,
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

/* ======================================================================= */
/*  モジュールグローバル（スイート保持: F-3 の隠蔽）                       */
/* ======================================================================= */

/// `Send`/`Sync` を付けたスイートポインタ運搬用 newtype（`usize` より provenance と意図が明確）。
///
/// # Safety
/// 指す先はホスト所有のスイート構造体。1モジュール=1ホスト（on_load ハンドシェイク, ADR-023）で
/// モジュール寿命中不変、読み取り専用に使う。
struct SuitePtr(*const c_void);
unsafe impl Send for SuitePtr {}
unsafe impl Sync for SuitePtr {}

struct Suites {
    decl: SuitePtr,
    eval: SuitePtr,
    urid: SuitePtr,
    treg: SuitePtr,
}

static SUITES: OnceLock<Suites> = OnceLock::new();

fn suites() -> &'static Suites {
    SUITES.get().expect("on_load が未実行（synapse_module! が必要）")
}
unsafe fn decl_suite() -> &'static SynDeclSuite {
    &*(suites().decl.0 as *const SynDeclSuite)
}
unsafe fn eval_suite() -> &'static SynEvalSuite {
    &*(suites().eval.0 as *const SynEvalSuite)
}
unsafe fn urid_suite() -> &'static SynUridSuite {
    &*(suites().urid.0 as *const SynUridSuite)
}

fn urid_of(uri: &CStr) -> SynUrid {
    unsafe { (urid_suite().map.unwrap())(uri.as_ptr()) }
}

/* ======================================================================= */
/*  declare / process / negotiate コンテキスト                             */
/* ======================================================================= */

/// `declare` 内でポートを宣言するビルダ。インデックス規約・既定値配送を隠す。
pub struct Declarer {
    b: *mut SynDeclBuilder,
    n_inputs: u32,
    n_outputs: u32,
}

impl Declarer {
    fn new(b: *mut SynDeclBuilder) -> Self {
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
            // 既定値の値渡し: ≤ptr は SVO、>ptr は呼び出し中のみ有効な借用（ホストが clone する。
            // `default` はこのメソッドのフレームに生存し input_default 呼び出し中ずっと有効）。
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
    fn new(ctx: *mut SynEvalCtx) -> Self {
        Self { ctx }
    }

    /// 入力リンクを読む共通処理。空値・型不一致・サイズ不一致はすべて `None` に倒す。
    /// 型 ID は declare 時に解決した `port_type_id` と照合する（ANY 宣言・上流バグで異型が
    /// 届いても範囲外読みしない）。
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
    /// 空値→`Ok(None)`、型不一致→`Err(TypeMismatch)`、一致→`Ok(Some(v))`。
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
                let buf = (e.alloc.unwrap())(self.ctx, T::SIZE);
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

    /// fan-in ポートの l 番目のリンク値。空値・型不一致は `None`。
    pub fn get_link<T: SynPlainType>(&self, port: MultiInPort<T>, link: u32) -> Option<T> {
        unsafe { self.read::<T>(port.index, link, port.type_id) }
    }
}

/// `negotiate` 内のコンテキスト。既定の `request_all` で全入力を列挙する。
pub struct NegotiateCtx {
    ctx: *mut SynEvalCtx,
    n_inputs: u32,
}

impl NegotiateCtx {
    fn new(ctx: *mut SynEvalCtx, n_inputs: u32) -> Self {
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

/* ======================================================================= */
/*  Node トレイト                                                          */
/* ======================================================================= */

/// 処理ノード。作者はこれを実装する。
pub trait Node: Default + 'static {
    /// ノード URI（例 `c"com.vendor.blur.gaussian"`）。
    const URI: &'static CStr;
    /// エディタ等に表示する名前。
    const DISPLAY_NAME: &'static CStr;

    /// ポートを宣言する（状態からフル再宣言・冪等）。
    fn declare(&mut self, d: &mut Declarer);

    /// 処理本体。`ctx.get`/`ctx.set` で入出力する。
    fn process(&mut self, ctx: &mut ProcessCtx) -> Result<()>;

    /// 必要入力の列挙。既定は全入力（静的ノード）。値依存の枝刈りが要る時だけ override。
    fn negotiate(&mut self, ctx: &mut NegotiateCtx) {
        ctx.request_all();
    }

    /// ソケットに出ない内部パラメータの保存。既定は無し。
    fn save_state(&self) -> Option<Vec<u8>> {
        None
    }

    /// 内部パラメータの復元。
    fn load_state(&mut self, _bytes: &[u8]) -> Result<()> {
        Ok(())
    }
}

/* ======================================================================= */
/*  トランポリン（C-ABI 境界）                                             */
/* ======================================================================= */

/// インスタンスの実体。declare で数えた入力ポート数を negotiate 用に保持する。
struct NodeWrapper<N> {
    node: N,
    n_inputs: u32,
}

/// FFI 境界を越えるパニックを遮断する。
fn guard<F: FnOnce() -> SynStatus>(f: F) -> SynStatus {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(s) => s,
        Err(_) => SYN_ERR_UNKNOWN,
    }
}

extern "C" fn create_tramp<N: Node>(_node: *mut SynNode, out: *mut *mut c_void) -> SynStatus {
    guard(|| {
        let w = Box::new(NodeWrapper {
            node: N::default(),
            n_inputs: 0,
        });
        unsafe { *out = Box::into_raw(w) as *mut c_void };
        SYN_OK
    })
}

extern "C" fn destroy_tramp<N: Node>(instance: *mut c_void) {
    let _ = guard(|| {
        unsafe { drop(Box::from_raw(instance as *mut NodeWrapper<N>)) };
        SYN_OK
    });
}

extern "C" fn declare_tramp<N: Node>(instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus {
    guard(|| {
        let w = unsafe { &mut *(instance as *mut NodeWrapper<N>) };
        let mut d = Declarer::new(b);
        w.node.declare(&mut d);
        w.n_inputs = d.n_inputs;
        SYN_OK
    })
}

extern "C" fn negotiate_tramp<N: Node>(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    guard(|| {
        let w = unsafe { &mut *(instance as *mut NodeWrapper<N>) };
        let mut nc = NegotiateCtx::new(ctx, w.n_inputs);
        w.node.negotiate(&mut nc);
        SYN_OK
    })
}

extern "C" fn process_tramp<N: Node>(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    guard(|| {
        let w = unsafe { &mut *(instance as *mut NodeWrapper<N>) };
        let mut pc = ProcessCtx::new(ctx);
        match w.node.process(&mut pc) {
            Ok(()) => SYN_OK,
            Err(e) => e.to_status(),
        }
    })
}

extern "C" fn save_tramp<N: Node>(
    instance: *mut c_void,
    out: *mut c_void,
    cap: usize,
    written: *mut usize,
) -> SynStatus {
    guard(|| {
        let w = unsafe { &*(instance as *const NodeWrapper<N>) };
        match w.node.save_state() {
            None => {
                unsafe { *written = 0 };
                SYN_OK
            }
            Some(bytes) => {
                unsafe { *written = bytes.len() };
                if !out.is_null() && cap >= bytes.len() {
                    unsafe {
                        core::ptr::copy_nonoverlapping(bytes.as_ptr(), out as *mut u8, bytes.len())
                    };
                }
                SYN_OK
            }
        }
    })
}

extern "C" fn load_tramp<N: Node>(
    instance: *mut c_void,
    input: *const c_void,
    len: usize,
) -> SynStatus {
    guard(|| {
        let w = unsafe { &mut *(instance as *mut NodeWrapper<N>) };
        let slice = if input.is_null() || len == 0 {
            &[][..]
        } else {
            unsafe { core::slice::from_raw_parts(input as *const u8, len) }
        };
        match w.node.load_state(slice) {
            Ok(()) => SYN_OK,
            Err(e) => e.to_status(),
        }
    })
}

/* ======================================================================= */
/*  PLAIN 型 vtable トランポリン                                           */
/* ======================================================================= */

extern "C" fn t_init<T: SynPlainType>(dst: *mut c_void, _t: SynTypeId) -> SynStatus {
    unsafe { core::ptr::write_bytes(dst as *mut u8, 0, T::SIZE) };
    SYN_OK
}
extern "C" fn t_clone<T: SynPlainType>(
    dst: *mut c_void,
    src: *const c_void,
    _t: SynTypeId,
) -> SynStatus {
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, T::SIZE) };
    SYN_OK
}
extern "C" fn t_serialize<T: SynPlainType>(
    obj: *const c_void,
    _t: SynTypeId,
    out: *mut c_void,
    cap: usize,
    written: *mut usize,
) -> SynStatus {
    unsafe { *written = T::SIZE };
    if !out.is_null() && cap >= T::SIZE {
        unsafe { core::ptr::copy_nonoverlapping(obj as *const u8, out as *mut u8, T::SIZE) };
    }
    SYN_OK
}
extern "C" fn t_deserialize<T: SynPlainType>(
    dst: *mut c_void,
    _t: SynTypeId,
    input: *const c_void,
    len: usize,
) -> SynStatus {
    if len >= T::SIZE {
        unsafe { core::ptr::copy_nonoverlapping(input as *const u8, dst as *mut u8, T::SIZE) };
        SYN_OK
    } else {
        SYN_ERR_BAD_ARG
    }
}

/* ======================================================================= */
/*  マクロが呼ぶ公開ヘルパ（マクロ本体を小さく保つ）                       */
/* ======================================================================= */

#[doc(hidden)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // host は生成された on_load から妥当な値で渡る
pub fn __on_load_begin(h: SynHost) {
    unsafe {
        let host = &*h;
        let f = host.fetch_suite.expect("fetch_suite");
        let decl = SuitePtr(f(h, c"synapse:decl".as_ptr()));
        let eval = SuitePtr(f(h, c"synapse:eval".as_ptr()));
        let urid = SuitePtr(f(h, c"synapse:urid".as_ptr()));
        let treg = SuitePtr(f(h, c"synapse:type-registry".as_ptr()));
        let _ = SUITES.set(Suites {
            decl,
            eval,
            urid,
            treg,
        });
    }
}

// 型ごと・ノードごとの記述子は、ヒープ(Box::leak)ではなく**モジュールイメージ内の static**に置く。
// 関連 const にすると各単相化の rodata に置かれ、`&` で 'static 参照に昇格する。これにより
//   (1) リークしない（Box::leak はヒープに残り dlclose しても解放されない）
//   (2) dlclose でモジュールイメージごと正しく解放される（将来のアンロード対応の前提）
// を同時に満たす。ABI が要求する「記述子はモジュール寿命まで存続」とも整合する。

/// PLAIN 型ごとの vtable（const）。`t_*` トランポリンは fn アイテム→fn ポインタの const coercion。
trait PlainVtable: SynPlainType {
    const VTABLE: SynTypeVTable = SynTypeVTable {
        flags: SYN_TYPE_PLAIN_BYTES,
        size: <Self as SynPlainType>::SIZE,
        align: core::mem::align_of::<Self>(),
        init: Some(t_init::<Self>),
        clone: Some(t_clone::<Self>),
        drop: None,
        serialize: Some(t_serialize::<Self>),
        deserialize: Some(t_deserialize::<Self>),
        get_api: None,
    };
}
impl<T: SynPlainType> PlainVtable for T {}

/// ノードごとの記述子（const）。
trait NodeDescStatic: Node {
    const DESC: SynNodeDesc = SynNodeDesc {
        caps: 0,
        node_uri: <Self as Node>::URI.as_ptr(),
        display_name: <Self as Node>::DISPLAY_NAME.as_ptr(),
        create: Some(create_tramp::<Self>),
        destroy: Some(destroy_tramp::<Self>),
        declare: Some(declare_tramp::<Self>),
        negotiate: Some(negotiate_tramp::<Self>),
        process: Some(process_tramp::<Self>),
        save_state: Some(save_tramp::<Self>),
        load_state: Some(load_tramp::<Self>),
        get_extension: None,
    };
}
impl<N: Node> NodeDescStatic for N {}

#[doc(hidden)]
pub fn __register_type<T: SynPlainType>(_h: SynHost) {
    unsafe {
        let treg = &*(suites().treg.0 as *const SynTypeRegistrySuite);
        // &<T>::VTABLE は単相化ごとの rodata（モジュールイメージ内）への 'static 参照に昇格する。
        (treg.register_type.unwrap())(T::URI.as_ptr(), &<T as PlainVtable>::VTABLE);
    }
}

#[doc(hidden)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // host は生成された on_load から妥当な値で渡る
pub fn __register_node<N: Node>(h: SynHost) {
    unsafe {
        let host = &*h;
        (host.register_node.unwrap())(h, &<N as NodeDescStatic>::DESC);
    }
}

/// モジュールの on_unload。静的記述子はモジュールイメージと共に解放されるので何もしない。
#[doc(hidden)]
pub extern "C" fn __on_unload(_h: SynHost) {}

/// `SynModule` を `static` に置くための Sync ラッパ（生ポインタを含むため）。
/// `static` はモジュールイメージ内に確保され、dlclose で正しく解放される。
///
/// # Safety
/// 中身は読み取り専用（エントリ関数が `&MODULE.0` を返すだけ）で、可変共有しないため Sync。
#[doc(hidden)]
pub struct SyncModule(pub SynModule);
unsafe impl Sync for SyncModule {}

/* ======================================================================= */
/*  公開マクロ                                                             */
/* ======================================================================= */

/// モジュールエントリ・型登録・ノード登録を一括生成する。
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

/* ======================================================================= */
/*  prelude                                                                */
/* ======================================================================= */

/// よく使う型・トレイト・マクロをまとめて取り込むための prelude。
pub mod prelude {
    pub use crate::{
        Declarer, Error, InPort, MultiInPort, NegotiateCtx, Node, OutPort, ProcessCtx, Result,
        SynPlainType,
    };
    pub use crate::synapse_module;
    pub use core::ffi::CStr;
}
