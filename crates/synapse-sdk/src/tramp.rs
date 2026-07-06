//! C-ABI トランポリンと記述子（マクロが組み立てる土台）。
//!
//! # 意図
//! 作者の [`Node`] / [`SynPlainType`] を、ホストが呼ぶ `extern "C"` 関数ポインタ群へ橋渡し
//! する。境界を越えるパニックは [`guard`] で遮断し、エラーステータスへ落とす。型 / ノードの
//! 記述子（vtable / node desc）は**モジュールイメージ内の static**（const 経由）に置き、
//! `dlclose` で正しく解放されるようにする（ヒープ Box::leak だと残留する）。
//!
//! # 公開ヘルパ
//! `__on_register_types_begin` / `__register_type` / `__register_node` / `__on_unload` / [`SyncModule`] は
//! [`synapse_module!`](crate::synapse_module) マクロが `$crate::...` で参照する内部 API。
//! `#[doc(hidden)]` だが、マクロ展開先から見えるよう `pub`。直接呼ぶことは想定しない。

use core::ffi::c_void;

use synapse_abi::{
    SynDeclBuilder, SynEvalCtx, SynHost, SynModule, SynNode, SynNodeDesc, SynStatus, SynTypeId,
    SynTypeRegistrySuite, SynTypeVTable, SYN_ERR_BAD_ARG, SYN_ERR_UNKNOWN, SYN_OK,
    SYN_TYPE_PLAIN_BYTES,
};

use crate::context::{Declarer, NegotiateCtx, ProcessCtx};
use crate::node::Node;
use crate::plain::SynPlainType;
use crate::suites::{set_suites, suites, SuitePtr, Suites};

/* ----------------------------------------------------------------------- */
/*  ノードインスタンスのトランポリン                                        */
/* ----------------------------------------------------------------------- */

/// インスタンスの実体。declare で数えた入力ポート数を negotiate 用に保持する。
struct NodeWrapper<N> {
    node: N,
    n_inputs: u32,
}

/// FFI 境界を越えるパニックを遮断する（panic 時は `SYN_ERR_UNKNOWN`）。
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

/* ----------------------------------------------------------------------- */
/*  PLAIN 型 vtable のトランポリン                                          */
/* ----------------------------------------------------------------------- */

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

/* ----------------------------------------------------------------------- */
/*  静的記述子（モジュールイメージ内 static）                               */
/* ----------------------------------------------------------------------- */

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

/* ----------------------------------------------------------------------- */
/*  マクロが呼ぶ公開ヘルパ（マクロ本体を小さく保つ）                        */
/* ----------------------------------------------------------------------- */

/// `on_register_types` 冒頭でスイートを fetch してモジュールグローバルへ格納する。
#[doc(hidden)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // host は生成された登録フェーズ関数から妥当な値で渡る
pub fn __on_register_types_begin(h: SynHost) {
    unsafe {
        let host = &*h;
        let f = host.fetch_suite.expect("fetch_suite");
        let decl = SuitePtr(f(h, c"synapse:decl".as_ptr()));
        let eval = SuitePtr(f(h, c"synapse:eval".as_ptr()));
        let urid = SuitePtr(f(h, c"synapse:urid".as_ptr()));
        let treg = SuitePtr(f(h, c"synapse:type-registry".as_ptr()));
        set_suites(Suites {
            decl,
            eval,
            urid,
            treg,
        });
    }
}

/// PLAIN 型 `T` の vtable をホストの型レジストリへ登録する。
#[doc(hidden)]
pub fn __register_type<T: SynPlainType>(_h: SynHost) {
    unsafe {
        let treg = &*(suites().treg.0 as *const SynTypeRegistrySuite);
        // &<T>::VTABLE は単相化ごとの rodata（モジュールイメージ内）への 'static 参照に昇格する。
        (treg.register_type.unwrap())(T::URI.as_ptr(), &<T as PlainVtable>::VTABLE);
    }
}

/// ノード `N` の記述子をホストへ登録する（`on_register_nodes` から呼ぶ）。
#[doc(hidden)]
#[allow(clippy::not_unsafe_ptr_arg_deref)] // host は生成された登録フェーズ関数から妥当な値で渡る
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
///
/// `static` はモジュールイメージ内に確保され、dlclose で正しく解放される。
///
/// # Safety
/// 中身は読み取り専用（エントリ関数が `&MODULE.0` を返すだけ）で、可変共有しないため Sync。
#[doc(hidden)]
pub struct SyncModule(pub SynModule);
unsafe impl Sync for SyncModule {}
