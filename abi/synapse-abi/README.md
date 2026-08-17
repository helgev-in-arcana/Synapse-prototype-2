# synapse-abi

Synapse プラグイン C ABI。**正本(Single Source of Truth)は Rust 側**で、C ヘッダは cbindgen に
よる生成物。

## 層構造

ABI は互換性の粒度で 3 層に分かれ、**1 層 = 1 クレート = 1 ヘッダ**で対応する。

| 層 | クレート | 生成ヘッダ | 互換性 |
| --- | --- | --- | --- |
| 基底 ABI | `synapse-abi-core` | `include/synapse_abi_core.h` | `SYN_ABI_VERSION` 一致必須(ホストは `!=` で拒否) |
| スイート | `synapse-abi-suite` | `include/synapse_abi_suite.h` | `host->fetch_suite(id)` 引き。**追加は非破壊** |
| 拡張 | `synapse-abi-ext` | `include/synapse_abi_ext.h` | `desc->get_extension(ext_id)` 引き。**追加は非破壊** |
| — | `synapse-abi` (このクレート) | `include/synapse_abi.h` | アンブレラ。3 層の再エクスポート + ヘッダ取り込み |

**依存は `suite`/`ext` → `core` の一方向で、Cargo が強制する。** `core` から上位層を参照すると
`error: cyclic package dependency` で弾かれる(レビュー任せではない)。成立している理由は境界が
すべて型消去されているため — `fetch_suite` / `get_extension` / `get_api` の戻り値は
`const void *` で、両層をつなぐのは不透明ハンドル(`SynDeclBuilder` / `SynEvalCtx`)だけ。

**層をクレートにしているのは、Rust ではディレクトリとモジュール構成が一意対応しないから。**
クレートは Cargo が定める一意な単位なので、層の分類キーとして安定する。クレート内の
モジュール構成は自由でよい。

## 利用側

`synapse-abi` 1 つに依存すればよい。全項目がクレート直下へ glob 再エクスポートされているので、
層を意識せず `synapse_abi::SynEvalSuite` のようにフラットに参照する(`use synapse_abi::*;` も可)。
層クレートを直接依存に足す必要は無い。

C 側も `#include "synapse_abi.h"` 1 本で全体が入る。層ヘッダを個別に include してもよい
(各層ヘッダは自分で `synapse_abi_core.h` を引くので単体で完結する)。

## ファイル構成

| ファイル | 役割 |
| --- | --- |
| `abi/synapse-abi-core/src/` | 基底 ABI の正本。`status` / `urid` / `handle` / `value` / `node` / `module` |
| `abi/synapse-abi-suite/src/` | スイートの正本。`type_registry` / `urid` / `decl` / `eval` |
| `abi/synapse-abi-ext/src/` | 拡張の正本。`ui` |
| `abi/synapse-abi-*/build.rs` | 5 行。層固有の設定(ヘッダ名・ガード・バナー・取り込む下位層)を `buildgen` に渡すだけ |
| `abi/synapse-abi-buildgen/` | ビルド補助(配布物ではない)。自クレート `src/` の再帰走査 → cbindgen 実行 → `include/` へ書き出し。cbindgen の共通設定もここが正本 |
| `abi/synapse-abi/src/lib.rs` | アンブレラ。3 層の glob 再エクスポート 3 行 |
| `abi/synapse-abi/build.rs` | `synapse_abi.h`(取り込み口 + 設計概要 + C 側補助定義)を書き出す |
| `abi/synapse-abi/tests/` | フラット公開面の固定テスト |
| `include/` | 生成ヘッダ 4 本。コミット対象(プラグイン作者への配布物)。**手で編集しないこと** |
| `docs/synapse_adr.md` | 設計判断の記録(ADR)。決定・却下案・ロードマップ・未決事項 |

## 項目を追加するとき

**該当する層クレートの該当ファイルに宣言を書く。それだけ。**

- Rust の公開面 … 層クレートの `lib.rs` がサブモジュールを glob 再エクスポートし、
  `synapse-abi` が層を glob 再エクスポートするので自動で `synapse_abi::Foo` になる
- C ヘッダ … その層の `build.rs` が自クレートの `src/` を再帰的に `syn` で parse して
  公開項目名を集め、cbindgen の `export.include` に入れる。文字列定数(`pub const X: &str`)も
  同じ経路で拾われ、`#define` として同じヘッダへ注入される

**どの層に属するかはどのクレートに置いたかで決まる。** 登録表の類は無い。新しいファイルを
足すときだけ、そのクレートの `lib.rs` に `pub(crate) mod x;` と `pub use x::*;` の 2 行を書く。

### なぜ `export.include` が要るのか

cbindgen の出力集合は `到達可能集合(functions ∪ globals ∪ constants ∪ export.include)` で、
ABI クレートは宣言専用で `extern "C"` 関数を 1 つも持たない。関数が無いと struct/union が
誰からも参照されず落ちるため(定数と型エイリアスだけが出る)、全型を明示的に include する
必要がある。だから `buildgen` がソースを走査してリストを導出している。

一方 `export.exclude` は**不要**。cbindgen は `parse_deps = false`(既定)で依存クレートを
parse しないので、上位層のヘッダに core の型が重複出力されることはない(名前参照だけが出る)。
クレート境界がそのまま分離になっている。

### 検査

`buildgen` の `verify` が「ソースに在るのにヘッダへ出なかった」項目を検出してビルドを落とす
(項目種別が `export.item_types` から漏れている等)。

Rust 側は `abi/synapse-abi/tests/public_surface.rs` が対になる。glob 再エクスポートは
名前衝突時に項目が黙って消えるため(Rust の曖昧 glob は使うまでエラーにならない)、全項目に
一度ずつ触れて消失を検出する。**ここは意図的に残した唯一の手書きリスト** — 定義ではなく
ガードなので、項目を足したら 1 行足す。

不透明ハンドル(`SynNode` 等)は `opaque_handle!` マクロ生成なので syn からは `Item::Macro` に
見え、項目リストには入らない。C の前方 typedef は `synapse-abi-core/build.rs` の `OPAQUE_FWD`
が注入する — これは意図した除外。

## 再生成と鮮度確認

```sh
cargo build                       # include/ のヘッダ 4 本を再生成
git diff --exit-code include/     # コミット済みヘッダとの一致確認
```

生成は必ず `cargo build` 経由で行う。`cbindgen` CLI の直叩きでは、export リストの導出も
文字列 ABI 定数の注入も行われない。

## 生成ヘッダの受け入れテスト

clang (VS 同梱の LLVM など) で:

```sh
cd include
# アンブレラ + 各層ヘッダ単体(層ヘッダが自分で core を引けているかの確認)
for h in synapse_abi.h synapse_abi_core.h synapse_abi_suite.h synapse_abi_ext.h; do
  printf '#include "%s"\nint main(void){return 0;}\n' "$h" > t.c
  clang -std=c11 -Wall -Wextra -Wpedantic -I. -fsyntax-only t.c || echo "FAIL $h"
done
# C++ 互換
printf '#include "synapse_abi.h"\nint main(){return 0;}\n' > t.cpp
clang++ -std=c++17 -Wall -Wextra -I. -fsyntax-only t.cpp
# 二重 include ガード確認(アンブレラと層ヘッダの混在も含む)
printf '#include "synapse_abi.h"\n#include "synapse_abi.h"\n#include "synapse_abi_core.h"\nint main(void){return 0;}\n' > t.c
clang -std=c11 -Wall -Wextra -Wpedantic -I. -fsyntax-only t.c
```

(ヘッダ単体コンパイル時の `-Wunused-function` 警告は static inline 補助関数によるもので無害。)

## 設計メモ

- 不透明ハンドル(`SynNode` 等)は Rust 側で Nomicon 流のゼロサイズ struct +
  `PhantomData<(*mut u8, PhantomPinned)>` として宣言する(uninhabited な空 enum への参照が
  UB になるのを避け、`!Send`/`!Sync`/`!Unpin` も表現する)。cbindgen はこの形では型本体を
  出力しないため、C の前方宣言 `typedef struct X X;`(不完全型)を注入している。
- cbindgen は毎回クレート全体をパースして依存順に並べ替えるため、Rust のファイル配置は
  生成ヘッダ内の宣言順に影響しない。
- 定数を Rust enum ではなく `pub const` にしているのは、C の enum 基底型が処理系定義で
  ABI 幅を固定できないため(`#define` + 固定幅 typedef に落とす)。
- 設計意図・アーキテクチャの柱・ロードマップは `include/synapse_abi.h` 冒頭のコメントに、
  決定の経緯と却下案は [../../docs/synapse_adr.md](../../docs/synapse_adr.md) に記録している。
