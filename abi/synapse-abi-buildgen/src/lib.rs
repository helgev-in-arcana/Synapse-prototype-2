//! ABI 層クレートの `build.rs` から呼ぶ C ヘッダ生成ヘルパ。
//!
//! # なぜこれが要るのか
//!
//! ABI は互換性の粒度で 3 層（core / suite / ext）に分かれ、**1 層 = 1 クレート = 1 ヘッダ**で
//! 対応させている。層の分類キーがクレートなので、クレート内のモジュール構成は自由で構わない
//! （ディレクトリとモジュールツリーが一意対応しない Rust の性質と衝突しない）。
//!
//! cbindgen は `parse_deps = false`（既定）なら依存クレートを parse しないので、
//! 上位層のヘッダに下位層の型が**重複出力されることはない**（名前参照だけが出る）。
//! したがって層間の `export.exclude` は不要で、各クレートは自分のことだけ知っていればよい。
//!
//! ただし `export.include` は必要になる。cbindgen の出力集合は
//! `到達可能集合(functions ∪ globals ∪ constants ∪ export.include)` であり、ABI クレートは
//! 宣言専用で `extern "C"` 関数を 1 つも持たないため、struct/union が誰からも参照されず
//! 落ちてしまう。そこで [`generate`] は**自クレートの `src/` を再帰走査**して公開項目名を集め、
//! それを `export.include` に入れる。手書きのリストは持たない。
//!
//! 文字列定数（`pub const X: &str`）は cbindgen が出力できないので、同じ走査で拾って
//! `#define` を自前生成する。
//!
//! # 使い方
//!
//! ```ignore
//! fn main() {
//!     synapse_abi_buildgen::generate(synapse_abi_buildgen::Layer {
//!         header: "synapse_abi_core.h",
//!         guard: "SYNAPSE_ABI_CORE_H",
//!         banner: BANNER,
//!         includes: &[],
//!         sys_includes: true,
//!         after_includes: OPAQUE_FWD,
//!     });
//! }
//! ```

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 1 層（= 1 クレート）分の生成設定。
pub struct Layer {
    /// 出力するヘッダのファイル名（`include/` 直下に置かれる）。
    pub header: &'static str,
    /// インクルードガードのマクロ名。
    pub guard: &'static str,
    /// ヘッダ冒頭に置く説明コメント。
    pub banner: &'static str,
    /// この層が取り込む下位層のヘッダ（例 `&["synapse_abi_core.h"]`）。
    pub includes: &'static [&'static str],
    /// `stdint.h` 等の標準ヘッダを出すか。最下層（core）だけ true。
    pub sys_includes: bool,
    /// インクルード群の直後に差し込む生テキスト（不透明ハンドルの前方宣言など）。
    pub after_includes: &'static str,
}

/// 層から抽出した内容。
struct Scanned {
    /// cbindgen に出力させる項目名（型・数値定数）。
    items: BTreeSet<String>,
    /// 文字列定数 `(名前, 値)`。cbindgen が出力できないので `#define` を自前生成する。
    strings: Vec<(String, String)>,
}

/// 生成ヘッダの置き場所（ワークスペース直下の `include/`）を返す。
///
/// ルートは `[workspace]` を含む `Cargo.toml` を上へ辿って探す。「N 個上」という深さ依存に
/// しないのは、クレートの置き場所（`crates/` か `abi/` か）を動かしたときに黙って
/// 間違ったディレクトリへ書き出すのを避けるため。
pub fn include_dir() -> PathBuf {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let root = workspace_root(&manifest).unwrap_or_else(|| {
        panic!(
            "[workspace] を含む Cargo.toml が {} から上に見つからなかった",
            manifest.display()
        )
    });
    let dir = root.join("include");
    std::fs::create_dir_all(&dir).expect("create include/");
    dir
}

/// `[workspace]` を宣言する `Cargo.toml` を持つ最も近い祖先を返す。
fn workspace_root(from: &Path) -> Option<PathBuf> {
    from.ancestors().find(|dir| {
        std::fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|t| t.lines().any(|l| l.trim_start().starts_with("[workspace]")))
    })
    .map(Path::to_path_buf)
}

/// cbindgen の共通設定。層ごとの差分は [`Layer`] だけに閉じる。
///
/// 生成される C を「行儀のよい C」に保つための方針:
///   - 定数は `#define`（C の enum 基底型は処理系定義で ABI 幅を固定できないため）
///   - `usize` は `size_t`、`c_int` は `int`、`*const c_char` は `const char *`
///   - doc コメント `///` がそのまま C コメントになる
fn base_config() -> cbindgen::Config {
    let mut c = cbindgen::Config {
        language: cbindgen::Language::C,
        pragma_once: false,
        cpp_compat: true,
        documentation: true,
        no_includes: true,
        usize_is_size_t: true,
        ..Default::default()
    };
    c.style = cbindgen::Style::Both;
    c.documentation_style = cbindgen::DocumentationStyle::C;
    c.export.item_types = vec![
        cbindgen::ItemType::Constants,
        cbindgen::ItemType::Enums,
        cbindgen::ItemType::Structs,
        cbindgen::ItemType::Unions,
        cbindgen::ItemType::Typedefs,
        cbindgen::ItemType::OpaqueItems,
        cbindgen::ItemType::Functions,
    ];
    c.constant.allow_static_const = false;
    c.constant.allow_constexpr = false;
    c
}

/// この層のヘッダを生成して `include/` に書き出す。
///
/// 出力対象は**呼び出し元クレートの `src/` を再帰走査**して導出する。項目を追加するときに
/// 書くのは宣言だけで、登録作業は要らない。
pub fn generate(layer: Layer) {
    let crate_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let src = PathBuf::from(&crate_dir).join("src");
    let scanned = scan(&src);

    assert!(
        !scanned.items.is_empty(),
        "{}/src から出力対象の項目が 1 つも見つからなかった",
        crate_dir,
    );

    let mut config = base_config();
    config.include_guard = Some(layer.guard.to_string());
    config.header = Some(layer.banner.to_string());
    config.export.include = scanned.items.iter().cloned().collect();
    if layer.sys_includes {
        config.sys_includes = vec![
            "stdint.h".into(),
            "stddef.h".into(),
            "stdbool.h".into(),
            "assert.h".into(),
        ];
    }

    // 依存クレートは parse しない（parse_deps = false が既定）ので、下位層の型は
    // 名前参照だけが出る。ここで下位層ヘッダを取り込めば解決する。
    let mut after = String::new();
    for header in layer.includes {
        after.push_str(&format!("\n#include \"{header}\"\n"));
    }
    after.push_str(layer.after_includes);
    if !scanned.strings.is_empty() {
        let width = scanned.strings.iter().map(|(n, _)| n.len()).max().unwrap_or(0);
        after.push_str("\n/* ---- 文字列 ABI 定数 (正本: このクレートの Rust ソース) ---- */\n");
        for (name, value) in &scanned.strings {
            after.push_str(&format!("#define {name:<width$} \"{value}\"\n"));
        }
    }
    config.after_includes = Some(after);

    let out = include_dir().join(layer.header);
    cbindgen::Builder::new()
        .with_crate(&crate_dir)
        .with_config(config)
        .generate()
        .unwrap_or_else(|e| panic!("cbindgen failed for {}: {e}", layer.header))
        .write_to_file(&out);

    verify(&out, &scanned, layer.header);

    println!("cargo:rerun-if-changed=src");
}

/// 生成後の検査。走査で拾った名前が実際にヘッダへ出たかを確かめる。
///
/// リスト自体はソースから導出するので登録漏れは起きないが、cbindgen 側の都合
/// （`item_types` から漏れた項目種別など）で黙って落ちるケースは残る。ここで落とす。
fn verify(path: &Path, scanned: &Scanned, header: &str) {
    let text = std::fs::read_to_string(path).expect("read generated header");
    let missing: Vec<_> = scanned
        .items
        .iter()
        .chain(scanned.strings.iter().map(|(n, _)| n))
        .filter(|name| !contains_word(&text, name))
        .collect();

    assert!(
        missing.is_empty(),
        "{header} に出力されなかった項目がある: {missing:?}\n\
         （ソースには在るが cbindgen が出力しなかった。項目種別が export.item_types に \
         含まれているか確認すること）",
    );
}

/// 識別子境界つきの部分文字列検索。単純な `contains` だと `SynValue` が
/// `SynValuePayload` に、`SynUrid` が `SynUridSuite` に誤ヒットして検査をすり抜ける。
fn contains_word(text: &str, name: &str) -> bool {
    let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
    text.match_indices(name).any(|(i, _)| {
        let before_ok = text[..i].chars().next_back().is_none_or(|c| !ident(c));
        let after_ok = text[i + name.len()..].chars().next().is_none_or(|c| !ident(c));
        before_ok && after_ok
    })
}

/// クレートの `src/` を再帰走査して公開項目名と文字列定数を集める。
///
/// **走査は「分類」ではなく「自クレート内の列挙」**なので、モジュール構成が
/// どうであれ（`#[path]`・インライン `mod`・入れ子ディレクトリ）取りこぼさない。
fn scan(src: &Path) -> Scanned {
    let mut out = Scanned { items: BTreeSet::new(), strings: Vec::new() };
    scan_dir(src, &mut out);
    out
}

fn scan_dir(dir: &Path, out: &mut Scanned) {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()))
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            scan_dir(&path, out);
        } else if path.extension().is_some_and(|x| x == "rs") {
            let text = std::fs::read_to_string(&path).expect("read source");
            let file =
                syn::parse_file(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()));
            collect_items(&file.items, out);
        }
    }
}

/// 項目リストを走査する。`mod { .. }` ブロックがあれば中へ降りる。
///
/// 非公開 `mod` の中でも降りる: `pub(crate) mod` の中の `pub` 項目は lib.rs の
/// glob 再エクスポートで公開面に出るため、ヘッダにも出す必要がある。
/// マクロ生成の項目（不透明ハンドル）は `Item::Macro` として素通しされ、
/// 意図どおりリストに入らない（C の前方宣言は `Layer::after_includes` で注入する）。
fn collect_items(items: &[syn::Item], out: &mut Scanned) {
    use syn::Item;

    for item in items {
        match item {
            Item::Struct(i) if is_public(&i.vis) => push(&mut out.items, &i.ident),
            Item::Union(i) if is_public(&i.vis) => push(&mut out.items, &i.ident),
            Item::Enum(i) if is_public(&i.vis) => push(&mut out.items, &i.ident),
            Item::Type(i) if is_public(&i.vis) => push(&mut out.items, &i.ident),
            Item::Const(i) if is_public(&i.vis) => match string_literal(i) {
                Some(value) => out.strings.push((i.ident.to_string(), value)),
                None => push(&mut out.items, &i.ident),
            },
            Item::Mod(m) => {
                if let Some((_, inner)) = &m.content {
                    collect_items(inner, out);
                }
            }
            _ => {}
        }
    }
}

fn is_public(vis: &syn::Visibility) -> bool {
    matches!(vis, syn::Visibility::Public(_))
}

fn push(set: &mut BTreeSet<String>, ident: &syn::Ident) {
    set.insert(ident.to_string());
}

/// `pub const X: &str = "..."` なら中身の文字列を返す。
fn string_literal(item: &syn::ItemConst) -> Option<String> {
    let is_str = matches!(&*item.ty, syn::Type::Reference(r)
        if matches!(&*r.elem, syn::Type::Path(p) if p.path.is_ident("str")));
    if !is_str {
        return None;
    }
    match &*item.expr {
        syn::Expr::Lit(syn::ExprLit { lit: syn::Lit::Str(s), .. }) => Some(s.value()),
        _ => None,
    }
}
