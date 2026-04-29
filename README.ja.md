# epubicus

`epubicus` は、英語 EPUB を日本語 EPUB に翻訳する CLI ツールです。EPUB のパッケージ構造と XHTML の体裁をできるだけ保ったまま翻訳します。

翻訳 provider は Ollama、OpenAI API、Claude API に対応しています。

## クイックスタート

まず EPUB の構造を確認します。翻訳コマンドの `FROM` / `TO` は EPUB リーダー上のページ番号ではなく、1 始まりの OPF spine 番号です。

```powershell
cargo run -- inspect .\book.epub
cargo run -- toc .\book.epub
```

指定範囲を標準出力に翻訳するテスト:

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider ollama --model qwen3:14b
```

翻訳済み EPUB を作成:

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider ollama --model qwen3:14b
```

ローカルモデルの生成が長くてタイムアウトする場合は、1 リクエストあたりのタイムアウトとリトライ回数を増やします。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider ollama --model qwen3:14b --timeout-secs 1800 --retries 3
```

翻訳結果は OS 標準のキャッシュ root（Windows: `%LOCALAPPDATA%\epubicus\cache`、Unix: `~/.cache/epubicus`）配下に、入力 EPUB ごとに保存されます。サブディレクトリ名は入力 EPUB の SHA-256 ハッシュ先頭 16 バイト hex で、中に `manifest.json` と `translations.jsonl` が入ります。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --cache-root .\.epubicus-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --clear-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --no-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --keep-cache
```

中断後は同じ `translate` コマンドを再実行すると、キャッシュ済みブロックを飛ばして続きから翻訳します。キャッシュは入力 EPUB のハッシュで識別されるため、出力先パスを変えても再開可能です。プログレスバーは開始時に `resuming: 991/5805 cached` のように既存キャッシュ分を反映した位置から始まります。

成功完了時にキャッシュは **自動削除** されます。デバッグ用途や部分再利用のため残したい場合は `--keep-cache` を指定します。

途中まで翻訳したキャッシュだけを使い、未翻訳ブロックは原文のまま EPUB を作成（このモードは **キャッシュを読み取り専用で参照** し、manifest 更新も自動削除も行いません）:

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --partial-from-cache
```

モデル、文体、用語集が違うとキャッシュキーも変わるため、途中まで翻訳したときと同じ条件を指定します。

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --provider ollama --model qwen3:14b --style tech --glossary .\glossary.json --partial-from-cache
```

別の場所にキャッシュを作っていた場合は `--cache-root` も合わせて指定します。

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --cache-root .\.epubicus-cache --partial-from-cache
```

キャッシュの一覧表示・削除は `cache` サブコマンドで行います:

```powershell
cargo run -- cache list
cargo run -- cache show <hash-or-input.epub>
cargo run -- cache prune --older-than 30
cargo run -- cache clear --hash <hash>
cargo run -- cache clear --all
```

指定範囲だけ翻訳し、それ以外は原文のまま EPUB を作成:

```powershell
cargo run -- translate .\book.epub -o .\book.part-ja.epub --from 3 --to 5 --provider ollama --model qwen3:14b
```

モデルを呼ばずに EPUB 処理だけ確認:

```powershell
cargo run -- translate .\book.epub --from 1 --to 1 --dry-run
```

## コマンド

```powershell
cargo run -- translate <INPUT.epub> [-o OUTPUT.epub] [OPTIONS]
cargo run -- test      <INPUT.epub> --from N --to M [OPTIONS]
cargo run -- inspect   <INPUT.epub>
cargo run -- toc       <INPUT.epub>
cargo run -- glossary  <INPUT.epub> [-o glossary.json]
cargo run -- cache     <SUBCOMMAND>
```

`translate` は EPUB を作成します。本番翻訳では、経過時間、予想残り時間、選択した spine ページ、翻訳対象 XHTML ブロック数に基づくプログレスバーを表示します。ETA は未キャッシュの実翻訳が 5 ブロック以上、かつ 30 秒以上進むまでは `ETA warming up` と表示します。

`test` は指定 spine 範囲の翻訳結果を標準出力に表示します。EPUB は作成しません。

`inspect` は OPF のパス、spine 順、`linear` 状態、参照先ファイルの有無、ファイルサイズ、翻訳対象 XHTML ブロック数の概算を表示します。

`toc` は EPUB3 `nav.xhtml` または EPUB2 NCX の目次を、階層インデントとリンク先付きで表示します。

`glossary` は固有名詞や専門用語の候補を JSON に出力します。

## オプション一覧

### `translate`

| オプション | デフォルト | 説明 |
|--|--|--|
| `-o, --output PATH` | `<input>.ja.epub` | 出力 EPUB |
| `--from N` | 先頭 | 翻訳する最初の OPF spine 番号 |
| `--to N` | 末尾 | 翻訳する最後の OPF spine 番号 |
| `--partial-from-cache` | false | キャッシュ済みブロックだけ訳文に差し替え、ミスは原文維持 |

### `test`

| オプション | デフォルト | 説明 |
|--|--|--|
| `--from N` | 必須 | 標準出力に出す最初の OPF spine 番号 |
| `--to N` | 必須 | 標準出力に出す最後の OPF spine 番号 |

### `translate` / `test` 共通

| オプション | デフォルト | 説明 |
|--|--|--|
| `--provider ollama\|openai\|claude` | `ollama` | 翻訳 provider |
| `-m, --model NAME` | provider ごと | モデル名 |
| `--ollama-host URL` | `http://localhost:11434` | Ollama エンドポイント |
| `--openai-base-url URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `--claude-base-url URL` | `https://api.anthropic.com/v1` | Claude / Anthropic API base URL |
| `--openai-api-key KEY` | なし | OpenAI API キー |
| `--anthropic-api-key KEY` | なし | Anthropic API キー |
| `--prompt-api-key` | false | 実行時に API キーを非表示入力 |
| `--temperature F` | `0.3` | サンプリング温度 |
| `--num-ctx N` | `8192` | Ollama に渡すコンテキスト長 |
| `--timeout-secs N` | `900` | 1 リクエストあたりの HTTP タイムアウト秒数 |
| `--retries N` | `2` | タイムアウト、接続失敗、rate limit、server error 時のリトライ回数 |
| `--style STYLE` | `essay` | 文体プリセット。`novel`, `novel-polite`, `tech`, `essay`, `academic`, `business` |
| `--dry-run` | false | provider を呼ばず、原文を使って EPUB 処理だけ確認 |
| `--glossary PATH` | なし | 用語統一に使う glossary JSON |
| `--cache-root PATH` | OS 標準（`%LOCALAPPDATA%\epubicus\cache` / `~/.cache/epubicus`） | キャッシュ root を上書き。入力 EPUB ごとに `<cache-root>/<input-hash>/` 以下に保存 |
| `--no-cache` | false | キャッシュを読み書きしない。既存キャッシュは削除しない |
| `--clear-cache` | false | この入力 EPUB のキャッシュを削除してから翻訳開始 |
| `--keep-cache` | false | 成功完了後もキャッシュを保持（デフォルトは自動削除） |

### `glossary`

| オプション | デフォルト | 説明 |
|--|--|--|
| `-o, --output PATH` | `glossary.json` | 用語集候補 JSON の出力先 |
| `--min-occurrences N` | `3` | 候補に含める最小出現回数 |
| `--max-entries N` | `200` | 出力する最大候補数 |
| `--review-prompt PATH` | なし | ChatGPT / Claude に渡す用語集レビュー用 Markdown を出力 |

### `inspect` / `toc`

`inspect` と `toc` は `INPUT.epub` だけを受け取り、追加オプションはありません。

### `cache`

| サブコマンド | 説明 |
|--|--|
| `cache list` | キャッシュ済みラン一覧（hash / セグメント数 / サイズ / 最終更新 / 入力ファイル） |
| `cache show <hash\|input.epub>` | 指定ランの manifest を表示（hash プレフィックスまたは入力 EPUB パスで指定） |
| `cache prune --older-than <DAYS> [--yes] [--dry-run]` | `last_updated_at` が N 日以上経過したランを削除 |
| `cache clear --hash <HASH> [--dry-run]` | 単一ランを削除 |
| `cache clear --all [--yes] [--dry-run]` | 全削除。`yes` 全文入力で確認（`--yes` でスキップ） |

`cache` には `--cache-root <PATH>` を渡してデフォルト以外のキャッシュ root を対象にできます。

## Provider

Ollama はデフォルト provider で、ローカルで動作します。

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider ollama --model qwen3:14b
```

Ollama が PATH に入っていない場合は、別途フルパスで実行します。

```powershell
& 'C:\Users\n_fuk\AppData\Local\Programs\Ollama\ollama.exe' pull qwen3:14b
& 'C:\Users\n_fuk\AppData\Local\Programs\Ollama\ollama.exe' list
```

OpenAI は Responses API を使います。`OPENAI_API_KEY`、`--openai-api-key`、または `--prompt-api-key` を使います。

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
cargo run -- test .\book.epub --from 1 --to 1 --provider openai --model gpt-5-mini
```

Claude は Anthropic Messages API を使います。`ANTHROPIC_API_KEY`、`--anthropic-api-key`、または `--prompt-api-key` を使います。

```powershell
$env:ANTHROPIC_API_KEY = Read-Host "Anthropic API key" -MaskInput
cargo run -- test .\book.epub --from 1 --to 1 --provider claude --model claude-sonnet-4-5
```

実行時に API キーを非表示入力する例:

```powershell
cargo run -- test .\book.epub --from 1 --to 1 --provider openai --prompt-api-key
cargo run -- test .\book.epub --from 1 --to 1 --provider claude --prompt-api-key
```

## 用語集

候補を作成します。

```powershell
cargo run -- glossary .\book.epub -o .\glossary.json
```

ChatGPT や Claude で候補を整理するためのプロンプトも同時に作れます。

```powershell
cargo run -- glossary .\book.epub -o .\glossary.candidates.json --review-prompt .\glossary-review.md
```

この場合は `glossary-review.md` の内容を ChatGPT / Claude に渡し、返ってきた JSON を `glossary.json` として保存して翻訳に使います。AI には、誤検出の削除、重複統合、`person` / `place` / `organization` / `term` などの分類、`dst` の訳語案作成を依頼する前提です。

`glossary-review.md` には作業説明のコメント、各フィールドの意味、修正方針、候補 JSON がまとまって入るため、そのまま ChatGPT / Claude に貼り付けられます。`glossary.candidates.json` 側はコメントなしの正規 JSON として出力します。

`dst` に訳語を入れます。

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Horizon",
      "dst": "ホライゾン",
      "kind": "term",
      "note": "occurrences: 793"
    }
  ]
}
```

翻訳時に指定します。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --glossary .\glossary.json
```

毎回すべての用語を送るのではなく、現在のブロックに登場する `src` だけを provider に渡します。技術書の専門用語、小説の人物名・地名・組織名の表記統一に使えます。

## 現在の実装範囲

- EPUB の展開と再パック
- OPF container / manifest / spine の解析
- OPF spine 状態の表示
- EPUB3 nav / EPUB2 NCX 目次の表示
- 用語集候補の抽出と用語集を使った翻訳
- 入力 EPUB ごとの翻訳キャッシュ（SHA-256 ハッシュで識別）と成功完了時の自動削除、`cache` サブコマンド（list / show / prune / clear）
- キャッシュ済みブロックだけを反映する部分翻訳 EPUB 作成（キャッシュ読み取り専用）
- XHTML 本文ブロックの走査
- 対象ブロック: `p`、見出し、リスト項目、表セル、キャプション、脚注 `aside` など
- インラインタグのプレースホルダ保持
- 脚注リンク、本文リンクなどのインラインリンクタグ保持
- プレースホルダ形式: `⟦E1⟧`、`⟦/E1⟧`、`⟦S1⟧`
- Ollama `/api/chat`、OpenAI `/responses`、Claude `/messages` による翻訳
- 文体プリセット指定
- 翻訳済み EPUB を作成する本番モード
- 本番翻訳時のプログレスバー表示
- 指定 spine 範囲を標準出力に出すテストモード

## 制限

- EPUB リーダー上のページ番号ではなく、OPF spine 番号で範囲指定します。
- `--partial-from-cache` はモデルを呼ばず、キャッシュヒットしたブロックだけ訳文に差し替え、キャッシュミスしたブロックを原文のまま残します。`--no-cache` とは併用できません。
- `nav.xhtml` / NCX の表示はできますが、目次自体の翻訳は未実装です。
- リトライ制御とフォールバック詳細レポートは未実装です。
- `<code>` や `<pre>` などのコード・整形済みテキストは翻訳対象外です。
- provider ごとの料金見積もりは未実装です。

## よくあるエラー

`failed to open .\book.epub` と出る場合は、指定した EPUB ファイルが存在しません。`book.epub` は例なので、実際のファイル名に置き換えてください。

```powershell
Get-ChildItem -Filter *.epub
cargo run -- inspect .\実際のファイル名.epub
```

`ollama` が見つからない場合は、Ollama が PATH に入っていません。フルパスで実行するか、PATH に追加してください。
