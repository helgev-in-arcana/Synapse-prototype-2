# 最小ホスト/プラグイン実装で判明した ABI の痛点

raw（SDK ラッパー無し）で `synapse-host-mini` と `test-scalar-plugin` を書き、
`declare →（接続時 negotiate）→ request↔process` を一周させて得られた知見。SDK 層・ABI 改訂の
（語彙は `synapse_adr.md` 確定語彙に従う: negotiate=接続可否 / request=poll 内データ要求）
判断材料として記録する。実装はこれらを**現状 ABI のまま回避**できているが、
プラグイン作者に押し付けると事故るので SDK が吸収すべき、というのが要点。

## F-1【解決済み】: SynValue を値渡しにして対称化

当初の `alloc_output(ctx, idx, ty, size, out_value: *mut SynValue)` は out パラメータが
プラグインのローカルで、SVO 値を書き戻してもホストから見えない非対称があった。

**解決**: SynValue を境界で**値渡し**にした（設計者の元々の意図）。
- 読み: `get_input(ctx, in, link) -> SynValue`（値返し）
- 書き: `set_output(ctx, out, value: SynValue)`（値渡し）。SVO はこれだけで完結。
- 大型(>8byte): `alloc(ctx, size) -> *mut c_void` でホスト所有バッファを得て書き、その
  ptr を SynValue.ptr に入れて set_output（確保はホスト＝ADR-012 維持）。

値渡しなら SynValue 構造体ごとコピーされ、SVO のビットは `ptr` フィールドに入ったまま
ホストへ渡るため、プラグインローカル不可視の問題が原理的に消える。読み書きが対称になり、
`write_output` ヘルパは不要になった（`make_float_value` で SynValue を組むだけ）。
→ ADR-022 として記録。本リポジトリの ABI v0.5 はこの形。

## F-2【決定により解決・ABI 対応不要 → ADR-023】: ctx 無しスイートとプロセスグローバル

`SynUridSuite::map/unmap`、`SynTypeRegistrySuite::register_type/lookup/type_of` は
ホストハンドルもコンテキストも引数に取らない。C の関数ポインタはデータを閉じ込め
られないため、これらの背後の状態は**構造的に**プロセスグローバル（または thread_local）
にしか置けない（本実装の `static HOST: OnceLock<Mutex<…>>`）。`Box::leak` 等で
ホストをヒープに置く手法は、`h` を受け取るコールバック群には有効だが、ハンドル無しの
これらスイート関数には適用できない点に注意。

**決定**: マルチホストは想定しない（1プロセス=1ホスト=1セッション、ADR-023）。
この前提ではプロセスグローバル＝セッションの実体であり ADR-005 と整合。ABI 変更不要。
将来マルチホストが必要になったら `h` 追加の ABI 変更が要る——その判断は ADR-023 参照。

## F-3【決定により解決・ABI 対応不要 → ADR-023】: プラグイン側のスイート保持

`declare(instance, builder)` は builder しか渡さない。プラグインは decl/eval スイートを
**on_load で fetch して module グローバルに保持**する必要がある。「1モジュール=1ホスト」
（ハンドシェイク=on_load で確定）の前提で正当な定石（CLAP 同様）であり、ABI は変えず
このボイラープレートは SDK が隠蔽する。

## F-4: `alloc` への size=0 vtable フォールバックは採用しない

旧 `alloc_output` 設計（ADR-022 改訂前）では「固定サイズ型は size=0 で vtable.size を
使わせてよい」というオプションが想定されていた。ADR-022（SynValue 値渡し化）で
`alloc_output` を廃止し `alloc(ctx, size) -> *mut c_void` に整理した際、このフォールバック
は現行 ABI に含めない方針とした。プラグイン（および SDK の `set`）は常に実サイズを渡す。

## F-5: `create` の `node: *mut SynNode` の用途が test では空

`mark_dirty` 用にインスタンスへ保持する想定だが、本テストのノードは内部状態の
動的変更をしないため未使用。dirty 伝播を検証するテスト（UI からの mark_dirty →
下流キャッシュ無効化）は次段で必要。

## 検証済みの ABI 経路（回帰の土台）

- register_type / register_node / fetch_suite の取り回し
- create → declare（output / input / input_default）→（接続時 negotiate）→ request↔process
- 非再帰 pull 評価（FFI 再帰なし。Rust 側再帰でも各プラグイン呼び出しは戻ってから次へ）
- get_input: 上流出力値の配送 / 未接続ソケットへの既定値配送
- alloc / set_output（値渡し）と SVO 読み出し（≤ptr幅 インライン）
- save_state / load_state 往復（サイズ問い合わせ → 書き込みの2段）
- 空値表現（type_id==0）の経路（未接続・既定値なしソケット）※本グラフでは未発火
- fan-in（multi-input）: SYN_PORT_MULTI ポートへの N リンク、link_count>1、同一ポートへの
  繰り返し get_input、リンク順序の安定性（subfold = 順序依存の畳み込み減算で検証）

## ラッパー層（実装済み）

- **synapse-sdk**: プラグイン側。`Node` トレイト + `synapse_module!` マクロで、F-3
  （スイート fetch + モジュールグローバル保持）・SynValue⇔Rust 型変換・request↔process の縮退形（初回 process で即 Ready）・
  save/load の2段プロトコル・FFI 越えパニック遮断を隠蔽。作者コードに unsafe/グローバルなし。
- **synapse-host-abi**: ホスト側。C-ABI 境界のみを安全な Rust に写す（個別モジュール/ノード単位）。
  グラフ・評価器・キャッシュは持たず本体ホストの責務。`&mut self` メソッドで ADR-019 の
  直列化契約がコンパイル時に保証される。

**クロス検証（ABI 漏れゼロの証明）**:
- SDK プラグイン × host-abi: `cargo test -p synapse-host-abi`（6 ケース）
- SDK プラグイン × raw mini ホスト: `cargo run -p synapse-host-mini -- test_scalar_sdk`
- raw プラグイン × raw mini ホスト: `cargo run -p synapse-host-mini`
いずれも同じアサーション（const→add / fan-in subfold / save-load）を通過。

## 次段の候補

- 本体ホストクレート（FFI/unsafe 無し）: グラフ管理・非再帰評価器・dirty 伝播キャッシュ・
  上流チェーンの pull。host-abi に依存し、ABI を直接触らない。
- 動的ノード（値依存の枝刈り negotiate override）、画像/GPU 型（OPAQUE + get_api）、
  multi-output（List<T>）。
- dirty 伝播（F-5）と passthrough（未知型素通し）の検証。
