# 詳細な実行例

README は最短手順だけに絞っています。長い手動コマンド、provider 別テンプレート、キャッシュ操作、Send to Kindle 向け固定レイアウトはこの文書にまとめます。

## 長い生成に備える

ローカルモデルの生成が長くてタイムアウトする場合は、1 リクエストあたりのタイムアウトとリトライ回数を増やします。

```powershell
cargo run --release -- translate .\book.epub -o .\book.ja.epub --provider ollama --model qwen3:14b --timeout-secs 1800 --retries 3
```

OpenAI などのリモート provider では、未キャッシュのリクエストを並列実行すると全体の待ち時間を短縮できます。

```powershell
cargo run --release -- translate .\book.epub -o .\book.ja.epub --provider openai --model gpt-5-mini --concurrency 4
```

変換前に概算の API リクエスト数とトークン数だけを確認するには `--usage-only` を使います。provider は呼びません。

```powershell
cargo run --release -- translate .\book.epub -p openai -m gpt-5-mini -j 4 --usage-only
```

OpenAI API の実際の使用状況は <https://platform.openai.com/usage>、請求状況は <https://platform.openai.com/settings/organization/billing/overview> で確認できます。

## 環境変数で既定値を固定する

よく使う設定は PowerShell セッションで一度だけ `EPUBICUS_*` 環境変数に入れておくと、毎回長いオプションを書かずに済みます。

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = "gpt-5-mini"
$env:EPUBICUS_FALLBACK_PROVIDER = "ollama"
$env:EPUBICUS_FALLBACK_MODEL = "qwen3:14b"
$env:EPUBICUS_CONCURRENCY = "4"

cargo run --release -- translate .\book.epub -o .\book.ja.epub
```

## キャッシュ操作

翻訳結果は OS 標準の cache root 配下に、入力 EPUB ごとに保存されます。サブディレクトリ名は入力 EPUB の SHA-256 ハッシュ先頭 16 バイト hex で、中に `manifest.json` と `translations.jsonl` が入ります。

```powershell
cargo run --release -- translate .\book.epub -o .\book.ja.epub --cache-root .\.epubicus-cache
cargo run --release -- translate .\book.epub -o .\book.ja.epub --clear-cache
cargo run --release -- translate .\book.epub -o .\book.ja.epub --no-cache
cargo run --release -- translate .\book.epub -o .\book.ja.epub --keep-cache
```

途中まで翻訳したキャッシュだけを使い、未翻訳ブロックは原文のまま EPUB を作成するには `--partial-from-cache` を使います。このモードはキャッシュを読み取り専用で参照します。

```powershell
cargo run --release -- translate .\book.epub -o .\book.partial-ja.epub --partial-from-cache
```

## Send to Kindle 用の固定レイアウト

画像と文字を絶対配置した EPUB は、Send to Kindle 側の変換でリフロー扱いになると、画像と文章の位置がオリジナルとずれることがあります。epubicus は、各 XHTML の `viewport` と固定配置らしいページ構造を検出した場合、自動で Kindle 向け固定レイアウトメタデータを追加します。

通常は自動判定のまま再生成します。

```powershell
.\scripts\rebuild-deepseek.ps1 .\book.epub
```

固定レイアウトメタデータを強制追加する例:

```powershell
.\scripts\rebuild-deepseek.ps1 .\book.epub fixed
```

自動判定を止める例:

```powershell
.\scripts\rebuild-deepseek.ps1 .\book.epub reflow
```

通常のリフロー EPUB に固定レイアウトメタデータを付けると、Kindle 側で文字サイズ変更などの読みやすさが落ちる場合があります。自動判定は強めの条件にしていますが、必要に応じてスクリプトでは `reflow`、CLI 直接実行では `--no-kindle-fixed-layout` で無効化してください。
