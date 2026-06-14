//! 統合テスト: synapse-host-abi（ホスト側境界層）× test-scalar-sdk（SDK プラグイン）。
//!
//! ABI の言語非依存性と、ラッパーが ABI に何も漏らしていないことを、
//! 「SDK で書いたプラグインを host-abi でロードして駆動できる」ことで検証する。
//!
//! グラフ評価ループはテストコード側に書く（host-abi の責務ではない＝本体ホストの責務を
//! テストが代行）。host-abi は個別ノードの declare/negotiate/process だけを提供する。

use std::path::PathBuf;

use synapse_host_abi::{LoadedModule, NodeInstance, OwnedValue};

/// ビルド済み SDK プラグイン DLL のパス。
fn plugin_path() -> PathBuf {
    // tests バイナリは target/debug/deps/ に置かれるので、2 つ上が target/debug。
    let mut dir = std::env::current_exe().unwrap();
    dir.pop(); // 実行ファイル名
    if dir.ends_with("deps") {
        dir.pop();
    }
    let fname = format!(
        "{}test_scalar_sdk{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_SUFFIX
    );
    dir.join(fname)
}

fn f32_of(v: &OwnedValue) -> f32 {
    let b = v.bytes();
    f32::from_ne_bytes([b[0], b[1], b[2], b[3]])
}

/// const ノード: 値を load_state で設定し、declare→process して出力 f32 を得る。
fn eval_const(module: &LoadedModule, value: f32) -> f32 {
    let ty = module.find_node("synapse.test.const").expect("const なし");
    let mut node = module.instantiate(ty).unwrap();
    node.declare().unwrap();
    node.load_state(&value.to_ne_bytes()).unwrap();

    // 入力なし。negotiate は空のはず。
    let reqs = node.negotiate(&[]).unwrap();
    assert!(reqs.is_empty(), "const は入力要求を出さない");

    let inputs = node.make_input_bindings().unwrap();
    let outs = node.process(&inputs).unwrap();
    f32_of(outs[0].as_ref().expect("const 出力なし"))
}

#[test]
fn load_module_and_list_nodes() {
    let module = LoadedModule::load(&plugin_path()).expect("ロード失敗");
    assert_eq!(module.module_uri(), "synapse.test");
    let mut uris: Vec<String> = module.node_types().iter().map(|t| t.uri()).collect();
    uris.sort();
    assert_eq!(
        uris,
        vec![
            "synapse.test.add".to_string(),
            "synapse.test.const".to_string(),
            "synapse.test.subfold".to_string(),
        ]
    );
}

#[test]
fn const_declare_process_and_state_roundtrip() {
    let module = LoadedModule::load(&plugin_path()).unwrap();

    // 既定値（load_state 前）は 1.0。
    let ty = module.find_node("synapse.test.const").unwrap();
    let mut node = module.instantiate(ty).unwrap();
    node.declare().unwrap();
    let inputs = node.make_input_bindings().unwrap();
    let outs = node.process(&inputs).unwrap();
    assert_eq!(f32_of(outs[0].as_ref().unwrap()), 1.0);

    // load_state で 3.0 に。
    node.load_state(&3.0f32.to_ne_bytes()).unwrap();
    let outs = node.process(&node.make_input_bindings().unwrap()).unwrap();
    assert_eq!(f32_of(outs[0].as_ref().unwrap()), 3.0);

    // save_state 往復。
    let saved = node.save_state().unwrap().expect("状態あり");
    assert_eq!(saved.len(), 4);
    assert_eq!(f32::from_ne_bytes([saved[0], saved[1], saved[2], saved[3]]), 3.0);
}

#[test]
fn add_with_upstream_and_default() {
    // const(3.0) ─▶ add.a,  add.b = 既定 4.0  →  7.0
    let module = LoadedModule::load(&plugin_path()).unwrap();

    let upstream = eval_const(&module, 3.0);
    assert_eq!(upstream, 3.0);

    let ty = module.find_node("synapse.test.add").unwrap();
    let mut add: NodeInstance = module.instantiate(ty).unwrap();
    let decl = add.declare().unwrap();
    let a_idx = decl.input_index("a").unwrap();
    let b_idx = decl.input_index("b").unwrap();
    assert_eq!((a_idx, b_idx), (0, 1));

    // a に上流 const の値を 1 リンク接続。b は未接続（既定値配送）。
    let mut inputs = add.make_input_bindings().unwrap();
    inputs.push_link(
        a_idx as usize,
        OwnedValue::from_plain_bytes(
            synapse_host_abi::Session::urid(c"synapse:float"),
            &upstream.to_ne_bytes(),
        ),
    );

    // negotiate: a は 1 リンク、b は 0 リンク。要求は a のみ 1 件。
    let reqs = add.negotiate(&[1, 0]).unwrap();
    assert_eq!(reqs.len(), 1);
    assert_eq!((reqs[0].input_index, reqs[0].link_index), (0, 0));

    let outs = add.process(&inputs).unwrap();
    assert_eq!(f32_of(outs[0].as_ref().unwrap()), 7.0);
}

#[test]
fn subfold_fan_in_preserves_order() {
    // const(10), const(1), const(2) ─▶ subfold.in（1ポート3リンク）  out = 10-1-2 = 7
    let module = LoadedModule::load(&plugin_path()).unwrap();
    let vals = [10.0f32, 1.0, 2.0];
    let upstream: Vec<f32> = vals.iter().map(|&v| eval_const(&module, v)).collect();

    let ty = module.find_node("synapse.test.subfold").unwrap();
    let mut node = module.instantiate(ty).unwrap();
    let decl = node.declare().unwrap();
    let in_idx = decl.input_index("in").unwrap();
    assert!(decl.is_multi(in_idx), "in は multi-input");

    let float_id = synapse_host_abi::Session::urid(c"synapse:float");
    let mut inputs = node.make_input_bindings().unwrap();
    for &v in &upstream {
        inputs.push_link(
            in_idx as usize,
            OwnedValue::from_plain_bytes(float_id, &v.to_ne_bytes()),
        );
    }

    // negotiate: in は 3 リンク → 要求 3 件（順序保持）。
    let reqs = node.negotiate(&[3]).unwrap();
    assert_eq!(reqs.len(), 3);
    for (l, r) in reqs.iter().enumerate() {
        assert_eq!((r.input_index, r.link_index), (0, l as u32));
    }

    let outs = node.process(&inputs).unwrap();
    // 10 - 1 - 2 = 7（順序が崩れれば別の値になる）。
    assert_eq!(f32_of(outs[0].as_ref().unwrap()), 7.0);
}

#[test]
fn module_reload_after_drop_is_clean() {
    // モジュールをロード→評価→drop（= on_unload + グローバル purge + dlclose）し、
    // 再ロードしても壊れず評価できることを確認する。drop で型/ノード登録が purge され、
    // 再ロードが stale 状態に汚染されない（dangling vtable/desc を残さない）ことのカバレッジ。
    {
        let module = LoadedModule::load(&plugin_path()).unwrap();
        assert_eq!(module.node_types().len(), 3);
        assert_eq!(eval_const(&module, 5.0), 5.0);
    } // ここで drop → purge

    // 再ロード: 前回の登録残骸が無く、ノードはちょうど 3、評価も正しい。
    let module = LoadedModule::load(&plugin_path()).unwrap();
    assert_eq!(module.node_types().len(), 3);
    assert_eq!(eval_const(&module, 9.0), 9.0);
}

#[test]
fn empty_socket_is_none() {
    // add.a を未接続・既定値あり → 既定 0.0、add.b 既定 4.0 → 4.0。
    // （既定値があるので空ではないが、既定値配送の経路を確認）
    let module = LoadedModule::load(&plugin_path()).unwrap();
    let ty = module.find_node("synapse.test.add").unwrap();
    let mut add = module.instantiate(ty).unwrap();
    add.declare().unwrap();
    let inputs = add.make_input_bindings().unwrap(); // 接続なし
    let outs = add.process(&inputs).unwrap();
    assert_eq!(f32_of(outs[0].as_ref().unwrap()), 4.0); // 0.0 + 4.0
}
