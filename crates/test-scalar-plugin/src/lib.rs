//! 最小プラグイン（raw / SDK ラッパー無し）。
//!
//! 3 ノードを提供する:
//!   - `synapse.test.const`  : 入力 0 / 出力 1(float)。内部パラメータ value(f32) を持ち
//!     save_state/load_state で永続化する。
//!   - `synapse.test.add`    : 入力 2(float, 既定値あり) / 出力 1(float)。out = a + b。
//!   - `synapse.test.subfold`: 入力 1(float, multi-input/fan-in) / 出力 1(float)。
//!     out = in[0] - in[1] - … - in[N-1]（順序依存の畳み込み減算）。
//!
//! ABI を意図的に「手で」叩くことで、SDK が将来吸収すべき痛点を洗い出す目的のコード。

#![allow(clippy::missing_safety_doc)]

use core::ffi::{c_char, c_int, c_void};
use std::sync::OnceLock;
use synapse_abi::*;

/* ----------------------------------------------------------------------- */
/*  モジュールグローバル（on_load で確定）                                  */
/* ----------------------------------------------------------------------- */

struct Globals {
    decl: usize,  // *const SynDeclSuite
    eval: usize,  // *const SynEvalSuite
    float_id: SynTypeId,
}

static G: OnceLock<Globals> = OnceLock::new();

fn g() -> &'static Globals {
    G.get().expect("globals not initialized (on_load 未実行)")
}

unsafe fn decl_suite() -> &'static SynDeclSuite {
    &*(g().decl as *const SynDeclSuite)
}
unsafe fn eval_suite() -> &'static SynEvalSuite {
    &*(g().eval as *const SynEvalSuite)
}

/* ----------------------------------------------------------------------- */
/*  SVO ヘルパ                                                              */
/* ----------------------------------------------------------------------- */

/// 入力読み出し: 型 ID とサイズを検証してから SVO（インライン）値を読む。
/// 異型・サイズ不一致・空は `None`（範囲外読みを防ぐ）。
/// これは手書きの定型ボイラープレートで、SDK 版ではジェネリクス（`SynPlainType`）で吸収される。
unsafe fn read_input_f32(v: &SynValue) -> Option<f32> {
    // 信頼境界の防御: ANY 宣言・上流バグで異型/サイズ違いが届いても安全側へ倒す。
    if v.type_id != g().float_id || v.size != 4 {
        return None;
    }
    // float は 4byte ≤ sizeof(void*) なので常に SVO（値は ptr フィールドにインライン）。
    let mut bytes = [0u8; 4];
    core::ptr::copy_nonoverlapping(
        (&v.ptr as *const *mut c_void) as *const u8,
        bytes.as_mut_ptr(),
        4,
    );
    Some(f32::from_ne_bytes(bytes))
}

/// float 値を SynValue として組み立てる(インライン SVO)。出力(set_output)・既定値の両方に使う。
/// 値渡しなので、SVO 値はこの構造体ごとホストへコピーされる(F-1 が解消した形)。
fn make_float_value(float_id: SynTypeId, f: f32) -> SynValue {
    let mut bits: usize = 0;
    let b = f.to_ne_bytes();
    unsafe {
        core::ptr::copy_nonoverlapping(b.as_ptr(), (&mut bits as *mut usize) as *mut u8, 4);
    }
    SynValue {
        type_id: float_id,
        ptr: bits as *mut c_void,
        size: 4,
    }
}

/* ----------------------------------------------------------------------- */
/*  float 型 vtable（PLAIN 4byte）                                          */
/* ----------------------------------------------------------------------- */

extern "C" fn float_init(dst: *mut c_void, _t: SynTypeId) -> SynStatus {
    unsafe { *(dst as *mut f32) = 0.0 };
    SYN_OK
}
extern "C" fn float_clone(dst: *mut c_void, src: *const c_void, _t: SynTypeId) -> SynStatus {
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, dst as *mut u8, 4) };
    SYN_OK
}
extern "C" fn float_serialize(
    obj: *const c_void,
    _t: SynTypeId,
    out: *mut c_void,
    cap: usize,
    written: *mut usize,
) -> SynStatus {
    unsafe { *written = 4 };
    if !out.is_null() && cap >= 4 {
        unsafe { core::ptr::copy_nonoverlapping(obj as *const u8, out as *mut u8, 4) };
    }
    SYN_OK
}
extern "C" fn float_deserialize(
    dst: *mut c_void,
    _t: SynTypeId,
    input: *const c_void,
    len: usize,
) -> SynStatus {
    if len >= 4 {
        unsafe { core::ptr::copy_nonoverlapping(input as *const u8, dst as *mut u8, 4) };
        SYN_OK
    } else {
        SYN_ERR_BAD_ARG
    }
}

static FLOAT_VTABLE: SynTypeVTable = SynTypeVTable {
    flags: SYN_TYPE_PLAIN_BYTES,
    size: 4,
    align: 4,
    init: Some(float_init),
    clone: Some(float_clone),
    drop: None,
    serialize: Some(float_serialize),
    deserialize: Some(float_deserialize),
    get_api: None,
};

/* ----------------------------------------------------------------------- */
/*  const ノード                                                           */
/* ----------------------------------------------------------------------- */

extern "C" fn const_create(_node: *mut SynNode, out_instance: *mut *mut c_void) -> SynStatus {
    let b = Box::new(1.0f32); // 既定値 1.0
    unsafe { *out_instance = Box::into_raw(b) as *mut c_void };
    SYN_OK
}
extern "C" fn const_destroy(instance: *mut c_void) {
    unsafe { drop(Box::from_raw(instance as *mut f32)) };
}
extern "C" fn const_declare(_instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus {
    unsafe {
        let d = decl_suite();
        (d.output.unwrap())(b, c"out".as_ptr(), c"Out".as_ptr(), g().float_id);
    }
    SYN_OK
}
extern "C" fn const_negotiate(_instance: *mut c_void, _ctx: *mut SynEvalCtx) -> SynStatus {
    SYN_OK // 入力なし: 要求なし
}
extern "C" fn const_process(instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    unsafe {
        let e = eval_suite();
        let value = *(instance as *const f32);
        let out = make_float_value(g().float_id, value);
        (e.set_output.unwrap())(ctx, 0, out);
    }
    SYN_OK
}
extern "C" fn const_save(
    instance: *mut c_void,
    out: *mut c_void,
    cap: usize,
    written: *mut usize,
) -> SynStatus {
    unsafe { *written = 4 };
    if !out.is_null() && cap >= 4 {
        let v = unsafe { *(instance as *const f32) };
        unsafe { core::ptr::copy_nonoverlapping(v.to_ne_bytes().as_ptr(), out as *mut u8, 4) };
    }
    SYN_OK
}
extern "C" fn const_load(instance: *mut c_void, input: *const c_void, len: usize) -> SynStatus {
    if len < 4 {
        return SYN_ERR_BAD_ARG;
    }
    let mut bytes = [0u8; 4];
    unsafe { core::ptr::copy_nonoverlapping(input as *const u8, bytes.as_mut_ptr(), 4) };
    unsafe { *(instance as *mut f32) = f32::from_ne_bytes(bytes) };
    SYN_OK
}

/* ----------------------------------------------------------------------- */
/*  add ノード                                                             */
/* ----------------------------------------------------------------------- */

extern "C" fn add_create(_node: *mut SynNode, out_instance: *mut *mut c_void) -> SynStatus {
    // 内部状態なし。非 NULL のダミーを置く。
    let b = Box::new(0u8);
    unsafe { *out_instance = Box::into_raw(b) as *mut c_void };
    SYN_OK
}
extern "C" fn add_destroy(instance: *mut c_void) {
    unsafe { drop(Box::from_raw(instance as *mut u8)) };
}
extern "C" fn add_declare(_instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus {
    let float = g().float_id;
    let types = [float];
    unsafe {
        let d = decl_suite();
        (d.input.unwrap())(b, c"a".as_ptr(), c"A".as_ptr(), types.as_ptr(), 1, 0);
        let da = make_float_value(float, 0.0);
        (d.input_default.unwrap())(b, c"a".as_ptr(), da);
        (d.input.unwrap())(b, c"b".as_ptr(), c"B".as_ptr(), types.as_ptr(), 1, 0);
        let db = make_float_value(float, 4.0); // 未接続なら 4.0
        (d.input_default.unwrap())(b, c"b".as_ptr(), db);
        (d.output.unwrap())(b, c"out".as_ptr(), c"Out".as_ptr(), float);
    }
    SYN_OK
}
extern "C" fn add_negotiate(_instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    unsafe {
        let e = eval_suite();
        // 宣言した 2 入力それぞれ、接続リンク数だけ要求を積む。
        for i in 0..2u32 {
            let n = (e.link_count.unwrap())(ctx, i);
            for l in 0..n {
                let req = SynRequest {
                    input_index: i,
                    link_index: l,
                    frame: SynRational { num: 0, den: 1 },
                };
                (e.request.unwrap())(ctx, &req);
            }
        }
    }
    SYN_OK
}
extern "C" fn add_process(_instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    unsafe {
        let e = eval_suite();
        let va = (e.get_input.unwrap())(ctx, 0, 0);
        let vb = (e.get_input.unwrap())(ctx, 1, 0);
        let a = read_input_f32(&va).unwrap_or(0.0);
        let b = read_input_f32(&vb).unwrap_or(0.0);
        let out = make_float_value(g().float_id, a + b);
        (e.set_output.unwrap())(ctx, 0, out);
    }
    SYN_OK
}

/* ----------------------------------------------------------------------- */
/*  subfold ノード（fan-in / multi-input 検証用）                          */
/*  単一の multi-input ポートに N リンクを受け、out = in[0]-in[1]-...-in[N-1]。 */
/*  順序依存の畳み込みなので、link_count>1・同一ポートへの繰り返し get_input・ */
/*  リンク順序の安定性(ADR-008) を 1 ノードで同時に検証できる。                */
/* ----------------------------------------------------------------------- */

extern "C" fn subfold_declare(_instance: *mut c_void, b: *mut SynDeclBuilder) -> SynStatus {
    let float = g().float_id;
    let types = [float];
    unsafe {
        let d = decl_suite();
        // SYN_PORT_MULTI: 1 宣言ポートで N リンクを受理（fan-in）。既定値は持たせない。
        (d.input.unwrap())(b, c"in".as_ptr(), c"In".as_ptr(), types.as_ptr(), 1, SYN_PORT_MULTI);
        (d.output.unwrap())(b, c"out".as_ptr(), c"Out".as_ptr(), float);
    }
    SYN_OK
}

extern "C" fn subfold_negotiate(_instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    unsafe {
        let e = eval_suite();
        let n = (e.link_count.unwrap())(ctx, 0);
        for l in 0..n {
            let req = SynRequest {
                input_index: 0,
                link_index: l,
                frame: SynRational { num: 0, den: 1 },
            };
            (e.request.unwrap())(ctx, &req);
        }
    }
    SYN_OK
}

extern "C" fn subfold_process(_instance: *mut c_void, ctx: *mut SynEvalCtx) -> SynStatus {
    unsafe {
        let e = eval_suite();
        let n = (e.link_count.unwrap())(ctx, 0);
        let mut acc = 0.0f32;
        if n > 0 {
            let v0 = (e.get_input.unwrap())(ctx, 0, 0);
            acc = read_input_f32(&v0).unwrap_or(0.0);
            for l in 1..n {
                let v = (e.get_input.unwrap())(ctx, 0, l);
                acc -= read_input_f32(&v).unwrap_or(0.0);
            }
        }
        let out = make_float_value(g().float_id, acc);
        (e.set_output.unwrap())(ctx, 0, out);
    }
    SYN_OK
}

/* ----------------------------------------------------------------------- */
/*  記述子・エントリポイント                                               */
/* ----------------------------------------------------------------------- */

fn make_const_desc() -> SynNodeDesc {
    SynNodeDesc {
        caps: 0,
        node_uri: c"synapse.test.const".as_ptr(),
        display_name: c"Const Float".as_ptr(),
        create: Some(const_create),
        destroy: Some(const_destroy),
        declare: Some(const_declare),
        negotiate: Some(const_negotiate),
        process: Some(const_process),
        save_state: Some(const_save),
        load_state: Some(const_load),
        get_extension: None,
    }
}

fn make_add_desc() -> SynNodeDesc {
    SynNodeDesc {
        caps: 0,
        node_uri: c"synapse.test.add".as_ptr(),
        display_name: c"Add".as_ptr(),
        create: Some(add_create),
        destroy: Some(add_destroy),
        declare: Some(add_declare),
        negotiate: Some(add_negotiate),
        process: Some(add_process),
        save_state: None,
        load_state: None,
        get_extension: None,
    }
}

fn make_subfold_desc() -> SynNodeDesc {
    SynNodeDesc {
        caps: 0,
        node_uri: c"synapse.test.subfold".as_ptr(),
        display_name: c"Subtract Fold (fan-in)".as_ptr(),
        // 内部状態なしなので add のダミー create/destroy を流用。
        create: Some(add_create),
        destroy: Some(add_destroy),
        declare: Some(subfold_declare),
        negotiate: Some(subfold_negotiate),
        process: Some(subfold_process),
        save_state: None,
        load_state: None,
        get_extension: None,
    }
}

/// フェーズ1（2フェーズロード, ADR-027）: スイート fetch と型登録。
/// 自己完結の float 型のみ登録する（他モジュールの型に依存しない＝ADR-028）。
extern "C" fn on_register_types(h: SynHost) -> SynStatus {
    unsafe {
        let host = &*h;
        let fetch = host.fetch_suite.expect("fetch_suite");
        let urid = fetch(h, c"synapse:urid".as_ptr()) as *const SynUridSuite;
        let treg = fetch(h, c"synapse:type-registry".as_ptr()) as *const SynTypeRegistrySuite;
        let decl = fetch(h, c"synapse:decl".as_ptr()) as *const SynDeclSuite;
        let eval = fetch(h, c"synapse:eval".as_ptr()) as *const SynEvalSuite;
        if urid.is_null() || treg.is_null() || decl.is_null() || eval.is_null() {
            return SYN_ERR_UNSUPPORTED;
        }
        let float_id = ((*urid).map.unwrap())(c"synapse:float".as_ptr());
        let st = ((*treg).register_type.unwrap())(c"synapse:float".as_ptr(), &FLOAT_VTABLE);
        if st != SYN_OK {
            return st;
        }

        G.set(Globals {
            decl: decl as usize,
            eval: eval as usize,
            float_id,
        })
        .ok();
    }
    SYN_OK
}

/// フェーズ2: ノード登録（全モジュールの型登録後にホストが呼ぶ）。
extern "C" fn on_register_nodes(h: SynHost) -> SynStatus {
    unsafe {
        let host = &*h;
        let cdesc = Box::leak(Box::new(make_const_desc()));
        let adesc = Box::leak(Box::new(make_add_desc()));
        let sdesc = Box::leak(Box::new(make_subfold_desc()));
        let reg = host.register_node.expect("register_node");
        reg(h, cdesc as *const SynNodeDesc);
        reg(h, adesc as *const SynNodeDesc);
        reg(h, sdesc as *const SynNodeDesc);
    }
    SYN_OK
}

extern "C" fn on_unload(_h: SynHost) {
    // 最小実装では leak したままにする（プロセス終了で回収）。
}

/// モジュールエントリ。各 .dll/.so がただ 1 つエクスポートするシンボル。
#[no_mangle]
pub extern "C" fn synapse_module() -> *const SynModule {
    static M: OnceLock<usize> = OnceLock::new();
    *M.get_or_init(|| {
        let m = Box::leak(Box::new(SynModule {
            abi_version: SYN_ABI_VERSION,
            module_uri: c"synapse.test".as_ptr(),
            module_version: c"0.1.0".as_ptr(),
            on_register_types: Some(on_register_types),
            on_register_nodes: Some(on_register_nodes),
            on_unload: Some(on_unload),
        }));
        m as *const SynModule as usize
    }) as *const SynModule
}

// c_int 未使用警告抑制（将来 log level 等で使う）。
#[allow(dead_code)]
fn _unused(_x: c_int, _y: *const c_char) {}
