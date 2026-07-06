# Synapse プラグイン側 仕様・ADR 状態索引（派生ビュー）

> **正本は [`synapse_adr.md`](./synapse_adr.md)**。本書はそこから状態を一覧化した**派生ビュー**で、
> 詳細（文脈/決定/却下案/帰結）は各 ADR を参照する。矛盾したら ADR 本体が正。
> ホスト本体実装の決定は [`host_adr.md`](./host_adr.md)。

スコープ: **プラグインインターフェース（C ABI） / ホストラッパー / プラグイン SDK** の3レイヤ。
ホスト本体実装は「→ホスト」とだけ示す。

## 分類軸

| | 実装あり | 実装なし |
|---|---|---|
| **仕様確定** | 確定 | 延期 |
| **仕様未定** | 暫定 | 未定 |

- **★** … 不可逆（差し替え不能・ロック対象）。
- **破棄** … 却下。番号と却下理由を残す。
- **[契約IN]/[→ホスト]** … 1つの意思決定が境界をまたぐもの。契約面はプラグイン側、機構はホスト側 ADR へ（1エントリ＋リンク1本）。

## 語彙（確定）

- **declare** … インスタンス生成時／再宣言要求時のソケット宣言。**評価ループ外**。
- **negotiate** … ソケット接続の瞬間の接続可否＝型互換判断。評価ループ外。ホスト主・プラグインは簡便チェック。
- **request** … pollループ内で上流へデータ要求。ホストが受けて上流ノードを駆動。
- **process** … 処理本体。Ready を返すまで request↔process を再周回。

---

# 1. プラグインインターフェース（C ABI）

## 確定（仕様確定 × 実装済）

- ★ 純粋 C ABI 境界・単一エントリ・不透明ハンドル・スイート方式 (ADR-003)
- ★ ワイヤ `{URID:u32, ptr, size}`・vtable は lookup／init 時キャッシュ (ADR-005)
- ★ 出力型1固定／入力多態／negotiate＝接続時のみ (ADR-007) [契約IN]
- ★ process 再入可能性（同一出力に process 複数回・借用はイテレーション全体で生存） (ADR-011)
- ★ declare / negotiate / request↔process の四層分離（declare は評価ループ外） (ADR-002)
- ★ FFI 跨ぎメモリはホスト確保／`alloc(ctx,size)` (ADR-012)
- ★ FFI 再帰なし契約 (ADR-001) [契約IN／評価器→ホスト]
- ★ 1プロセス=1ホスト=1セッション／スイート関数プロセスグローバル (ADR-023)
- ★ SVO＝16byte 以下を payload（union `{ptr, data[16]}`）にインライン格納（ABI v3） (ADR-006)
- multi-input＝1ポート N 受理 (ADR-008)
- 冪等 declare＋安定 key で reconcile（方式として） (ADR-010)
- 可変出力は `List<T>`＋抽出ノード（方式として。実装は延期） (ADR-009)
- 型＋API 二層エスケープハッチ（OPAQUE `get_api`） (ADR-016)
- 空値＝`type_id==0`／null 処理はプラグイン (ADR-018)
- `clone/drop=refcount` の vtable 契約 (ADR-015) [契約IN／寿命管理→ホスト]
- `mark_dirty` の ABI 表面と「内部状態変更時のみ」契約 (ADR-014) [契約IN／無効化機構→ホスト]
- 型の `default/serialize`・`save_state/load_state` の ABI (ADR-017) [ABI IN／オートメーション→ホスト]
- caps 宣言（`SYN_CAP_*`）と直列化契約 (ADR-019/021) [宣言IN／スケジューラ→ホスト]
- unwind は abort・エラーは戻り値（`extern "C-unwind"` 不採用） (ADR-024)
- 評価境界の値受け渡しと寿命（出力=ホスト確保／入力=借用、32-bit でも const 分岐） (ADR-026)
- アラインメントは型属性（`SynTypeVTable.align`・登録時 2 冪検証）＋ `alloc(ctx,size,t)` で型を受領 (ADR-029)
- ★ 型-型依存の禁止（型登録フェーズ中の他モジュール型 lookup 拒否まで実装） (ADR-028)
- 2フェーズロード：`on_register_types` / `on_register_nodes`＋`load_many`（ABI v2） (ADR-027)
- declare の key＝文字列 (ADR-010, Open-11 解決)

## 延期（仕様確定 × 実装なし）

- `List<T>`＋抽出ノードによる可変出力の実装 (ADR-009)（canonical 型集合＝Open-10 依存）

## 暫定（仕様未定 × 実装済）

- データ受け渡し API：`get_input→SynValue` / `set_output(value)` 値渡し (ADR-022) [IF・ラッパー・SDK 跨り]
- イテレーション具体形：request / process 分離＋非 Ready で再周回・返り値プロトコル (ADR-011)
- エントリポイント具体＝`synapse_module` 固定シンボル（凍結せず） (ADR-003)
- バージョニング＝`SYN_ABI_VERSION` 一個 (ADR-020)
- 汎用ノードの ANY 出力解決＝方式a (ADR-013) [`connected_type` は確定IN／伝播パス→ホスト]
- アンロード時グローバル purge の v1 運用 (ADR-025) [契約IN／本体側運用→ホスト]

## 未定（仕様未定 × 実装なし）

- ROI／領域の ABI 表現 (Open-7) [表現IN／伝播→ホスト]
- 必須ソケット強制 `required` フラグ (Open-17)
- multi-input グループ型統一の強制 (Open-1)
- プラグイン側の簡便型チェック（fan-in で上流型が揃う保証がないため） (Open-21)
- `List<T>` の評価責任・借用窓 (Open-2)
- 可変長サイズ問い合わせ規約の統一 (Open-14)

## 破棄

- vtable 同梱 fat-pointer 案 — 却下理由: アンロード時 vtable ダングリング／永続化でゴミ化／IPC で無意味 → lookup が正 (ADR-005)
- `Vec<{type_id, pointer}>` 2フィールド版 — `{type_id, ptr, size}` が正 (ADR-005)
- `alloc` size=0 vtable フォールバック (FINDINGS F-4)
- マルチホスト前提 — 1プロセス=1ホストに確定 (ADR-023)
- 世代付きハンドル — 却下理由: 照合先アリーナ不在では UAF 後の確率的検知にしかならず、借用寿命は四層分離＋process 再入契約が構造的に保証済み。必要なら OPAQUE 型が 16byte ハンドル内に型単位で後付け可 (Open-20)

---

# 2. ホストラッパー（synapse-host-abi）

## 確定（仕様確定 × 実装済）

- C-ABI → 安全 Rust 写像（個別モジュール／個別インスタンス粒度のみ。リンク・評価順・配線・キャッシュは持たない＝→ホスト本体）
- declare 結果モデル `NodeDecl` / `InputDecl` / `OutputDecl`（`src/decl.rs`）
- 同一インスタンスの非重複呼び出しを `&mut self` でコンパイル時保証 (ADR-019 写像)
- 全コールバックの `catch_unwind` 包み＝unwind 越境防止 (ADR-024 のラッパー側)

## 延期（仕様確定 × 実装なし）

- `OwnedValue` の SVO ゼロアロケ化 (Open-19)（方向は決定・現状 `Vec<u8>` でヒープ確保）

## 暫定（仕様未定 × 実装済）

- `OwnedValue`（SynValue 値渡しの capture/present） (ADR-022 従属)

## 未定（仕様未定 × 実装なし）

- 独自項目なし（IF 側の未定に従属）

## 破棄

- なし

---

# 3. プラグイン SDK

実装はほぼ未着手。「設計はあるが実装が薄い層」。
（synapse-sdk は `Node` トレイト＋`synapse_module!` マクロで F-3・値変換・縮退 negotiate・save/load・unwind 遮断を隠蔽済み＝そこは確定×実装済。以下は未踏部分。）

## 確定（仕様確定 × 実装済）

- `Node` トレイト＋`synapse_module!` マクロによるボイラープレート隠蔽（スイート fetch＋モジュールグローバル保持, FINDINGS F-3）
- SynValue⇔Rust 型変換・縮退 process・save/load 2段・FFI 越え panic 遮断 (ADR-024/026 の SDK 側)

## 延期（仕様確定 × 実装なし）

- （特になし。隠蔽すべきボイラープレートは順次拡張）

## 暫定（仕様未定 × 実装済）

- 単発／2パス／任意パスの UX 吸収（同一 ABI へコンパイル）（イテレーション具体形に従属, ADR-011）

## 未定（仕様未定 × 実装なし）

- 二層 SDK API 詳細・値依存枝刈り (Open-8)
- 静的／動的ノードの2トレイト → 同一 ABI シンボル（構想のみ）
- Forge／Parser 安全ヘルパ（後回し）

## 破棄

- なし
