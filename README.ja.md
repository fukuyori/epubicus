# epubicus

`epubicus` は、英語 EPUB を日本語 EPUB に翻訳する CLI ツールです。EPUB のパッケージ構造と XHTML の体裁をできるだけ保ったまま翻訳します。

翻訳 provider は Ollama、OpenAI API、Claude API に対応しています。

## ドキュメント

- [docs/README.md](docs/README.md): 運用ガイド、復旧手順、設計メモの索引。
- [docs/operation-guide.ja.md](docs/operation-guide.ja.md): 日本語の運用ガイド。
- [docs/runtime-progress.ja.md](docs/runtime-progress.ja.md): リリースビルド実行、ETA 計測、インラインマーカー検証の運用メモ。
- [docs/batch-recovery.ja.md](docs/batch-recovery.ja.md): OpenAI Batch API 実行後の復旧手順。

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

OpenAI などのリモート provider では、未キャッシュのリクエストを並列実行すると全体の待ち時間を短縮できます。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --provider openai --model gpt-5-mini --concurrency 4
```

変換前に概算のAPIリクエスト数とトークン数だけを確認するには `--usage-only` を使います。provider は呼びません。

```powershell
cargo run -- translate .\book.epub -p openai -m gpt-5-mini -j 4 --usage-only
```

OpenAI API の実際の使用状況は <https://platform.openai.com/usage>、請求状況は <https://platform.openai.com/settings/organization/billing/overview> で確認できます。

よく使う設定は PowerShell セッションで一度だけ `EPUBICUS_*` 環境変数に入れておくと、毎回長いオプションを書かずに済みます。

```powershell
$env:OPENAI_API_KEY = Read-Host "OpenAI API key" -MaskInput
$env:EPUBICUS_PROVIDER = "openai"
$env:EPUBICUS_MODEL = "gpt-5-mini"
$env:EPUBICUS_FALLBACK_PROVIDER = "ollama"
$env:EPUBICUS_FALLBACK_MODEL = "qwen3:14b"
$env:EPUBICUS_CONCURRENCY = "4"

cargo run -- translate .\book.epub -o .\book.ja.epub
```

翻訳結果は OS 標準のキャッシュ root（Windows: `%LOCALAPPDATA%\epubicus\cache`、Unix: `~/.cache/epubicus`）配下に、入力 EPUB ごとに保存されます。サブディレクトリ名は入力 EPUB の SHA-256 ハッシュ先頭 16 バイト hex で、中に `manifest.json` と `translations.jsonl` が入ります。

provider から返った内容はキャッシュに書く前に検証します。空応答、英語原文そのまま、プロンプト用タグの混入、インラインプレースホルダの欠落・変更・追加、拒否・説明文らしい応答は `--retries` に従って再試行します。`⟦/S1⟧` や `⟦DAX⟧` のように provider が追加した括弧型マーカーも拒否し、タグ復元用の記号が EPUB 出力に漏れないようにします。拒否・説明文らしい応答で再試行が尽きた場合、`--fallback-provider` が指定されていれば同じ原文を fallback provider で翻訳し直します。fallback も失敗した場合は翻訳として保存せずエラーにします。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --cache-root .\.epubicus-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --clear-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --no-cache
cargo run -- translate .\book.epub -o .\book.ja.epub --keep-cache
```

中断後は同じ `translate` コマンドを再実行すると、キャッシュ済みブロックを飛ばして続きから翻訳します。キャッシュは入力 EPUB のハッシュで識別されるため、出力先パスを変えても再開可能です。並列実行中も、成功したブロックはページ全体の完了を待たずに即座にキャッシュへ書き込むため、中断時に失われるのは「その瞬間に処理中で、まだ結果が返っていないブロック」に限られます。プログレスバーは開始時に `resuming: 991/5805 cached` のように既存キャッシュ分を反映した位置から始まります。

成功完了時にキャッシュは **自動削除** されます。デバッグ用途や部分再利用のため残したい場合は `--keep-cache` を指定します。

途中まで翻訳したキャッシュだけを使い、未翻訳ブロックは原文のまま EPUB を作成（このモードは **キャッシュを読み取り専用で参照** し、manifest 更新も自動削除も行いません）:

```powershell
cargo run -- translate .\book.epub -o .\book.partial-ja.epub --partial-from-cache
```

実行中の翻訳を中断して「ここまでのキャッシュは訳文、残りは原文」で EPUB を完成させたい場合は、`Ctrl+C` で止めたあと、同じ入力 EPUB と同じ翻訳条件に `--partial-from-cache` を付けて組み立てます。

OpenAI Batch 用キャッシュから組み立てる例:

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.batch-openai-cache `
  --provider openai `
  --model gpt-5-mini `
  --glossary .\glossary.json `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

ローカル Ollama 用キャッシュから組み立てる例:

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.local-ollama-cache `
  --provider ollama `
  --model qwen3:14b `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

原文のまま出力したブロックがある場合は、キャッシュディレクトリ配下に `recovery\<出力EPUB名>\untranslated.txt` を出力します。通常の警告が大量に流れる実行でも、このファイルを見るとページ番号、XHTML href、理由、原文ブロックを後から確認できます。キャッシュと同じ場所に置くため、`cache clear` / `cache prune` で翻訳キャッシュや batch artifact とまとめて整理できます。

同時に `recovery\<出力EPUB名>\recovery.jsonl` も作成します。実際のパスは `translate` の最後に `Recovery log:` として表示されます。復旧ログは `recover` サブコマンドで読み込み、不足ブロックだけを再翻訳して通常キャッシュへ戻せます。

```powershell
$log = ".\.local-ollama-cache\0123456789abcdef0123456789abcdef\recovery\book_jp\recovery.jsonl"
cargo run -- recover $log
```

復旧と EPUB の再生成を続けて行う場合は `--rebuild` を付けます。全件復旧できた場合だけ、復旧ログに記録された出力先へ `--partial-from-cache` で組み立て直します。

```powershell
cargo run -- recover $log --rebuild
```

出力先を変えたい場合は `--output` を指定します。手動で組み立て直す場合は、キャッシュから EPUB を再生成します。

```powershell
cargo run -- translate .\book.epub `
  --cache-root .\.local-ollama-cache `
  --partial-from-cache `
  --keep-cache `
  --output .\book_jp.epub
```

復旧不能なブロックが残る場合は、復旧ログと同じディレクトリに `failed.jsonl` を作り、コンソールにページ番号、block、href、理由、最後のエラー、cache key を表示します。provider や model を変えて再試行する場合は `recover` に `--provider` / `--model` を指定します。

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
cargo run -- batch     <SUBCOMMAND>
cargo run -- cache     <SUBCOMMAND>
```

`translate` は EPUB を作成します。本番翻訳では、経過時間、予想残り時間、選択した spine ページ、翻訳対象 XHTML ブロック数、未キャッシュブロックの provider リクエスト進捗をプログレスバーに表示します。ETA は現在の実行、または再開した時点から測りますが、spine 1〜3ページ目はETAの計測時間と文字数から除外します。開始時に4ページ目以降の未キャッシュ原文文字数を数え、4ページ目以降のprovider作業の計測時間が5分に達するまでは `ETA pending` のままにし、その後は4ページ目以降で完了した未キャッシュ文字数と経過時間の累積平均から、残りの未キャッシュ文字数を単純に予測します。以前の実行でキャッシュ済みだった分は進捗位置には反映しますが、ETA の分母には入れません。OpenAI / Claude など provider が usage を返す場合は、終了時に API リクエスト数と input / output / total tokens を表示します。

`test` は指定 spine 範囲の翻訳結果を標準出力に表示します。EPUB は作成しません。

`inspect` は OPF のパス、spine 順、`linear` 状態、参照先ファイルの有無、ファイルサイズ、翻訳対象 XHTML ブロック数の概算を表示します。

`toc` は EPUB3 `nav.xhtml` または EPUB2 NCX の目次を、階層インデントとリンク先付きで表示します。

`glossary` は固有名詞や専門用語の候補を JSON に出力します。

`batch` は OpenAI Batch API 用の非同期翻訳ワークフローを管理します。`batch run` は準備、送信、状態確認、取得、取り込み、検証をまとめて実行します。途中で待機をやめた場合やリモート側で失敗・未完了が残った場合は、まず `batch reroute-local` で対象を `local_pending` にマークし、次に `batch translate-local` でその `local_pending` を Ollama などの通常 provider で翻訳します。`reroute-local` は対象選択だけを行い、翻訳はしません。

未完了分をローカルに回す例:

```powershell
cargo run -- batch health .\book.epub
cargo run -- batch reroute-local .\book.epub --remaining --priority short-first
cargo run -- batch translate-local .\book.epub --provider ollama --model qwen3:14b --limit 100
cargo run -- batch verify .\book.epub
cargo run -- translate .\book.epub --partial-from-cache --keep-cache -o .\book_jp.epub
```

## オプション一覧

### `translate`

| オプション | デフォルト | 説明 |
|--|--|--|
| `-o, --output PATH` | `<input>.ja.epub` | 出力 EPUB |
| `--from N` | 先頭 | 翻訳する最初の OPF spine 番号 |
| `--to N` | 末尾 | 翻訳する最後の OPF spine 番号 |
| `--partial-from-cache` | false | キャッシュ済みブロックだけ訳文に差し替え、ミスは原文維持。未翻訳が残った場合、EPUB と未翻訳レポートを書いた後にエラー終了 |

未翻訳が残った状態で EPUB と復旧ログを書けた場合、`recover` で復旧不能 item が `failed.jsonl` に残った場合、`scan-recovery` が未翻訳候補を検出して復旧ログを書いた場合は、継続可能エラーとして終了コード `2` を返します。入力 EPUB が壊れている、出力先に書けないなど処理を継続できない失敗は通常のエラーとして終了コード `1` です。

### `test`

| オプション | デフォルト | 説明 |
|--|--|--|
| `--from N` | 必須 | 標準出力に出す最初の OPF spine 番号 |
| `--to N` | 必須 | 標準出力に出す最後の OPF spine 番号 |

### `translate` / `test` 共通

CLI 引数を指定した場合は、環境変数より CLI 引数が優先されます。

| オプション | 環境変数 | デフォルト | 説明 |
|--|--|--|--|
| `-p, --provider ollama\|openai\|claude` | `EPUBICUS_PROVIDER` | `ollama` | 翻訳 provider |
| `-m, --model NAME` | `EPUBICUS_MODEL` | provider ごと | モデル名 |
| `--fallback-provider ollama\|openai\|claude` | `EPUBICUS_FALLBACK_PROVIDER` | なし | 主 provider が拒否・説明文らしい応答を返し、リトライが尽きた場合だけ使う fallback provider |
| `--fallback-model NAME` | `EPUBICUS_FALLBACK_MODEL` | fallback provider ごと | fallback provider のモデル名 |
| `--ollama-host URL` | `EPUBICUS_OLLAMA_HOST` | `http://localhost:11434` | Ollama エンドポイント |
| `--openai-base-url URL` | `EPUBICUS_OPENAI_BASE_URL` | `https://api.openai.com/v1` | OpenAI API base URL |
| `--claude-base-url URL` | `EPUBICUS_CLAUDE_BASE_URL` | `https://api.anthropic.com/v1` | Claude / Anthropic API base URL |
| `--openai-api-key KEY` | `OPENAI_API_KEY` | なし | OpenAI API キー。`--openai-api-key` が優先 |
| `--anthropic-api-key KEY` | `ANTHROPIC_API_KEY` | なし | Anthropic API キー。`--anthropic-api-key` が優先 |
| `--prompt-api-key` | なし | false | 実行時に API キーを非表示入力 |
| `-T, --temperature F` | `EPUBICUS_TEMPERATURE` | `0.3` | サンプリング温度 |
| `-n, --num-ctx N` | `EPUBICUS_NUM_CTX` | `8192` | Ollama に渡すコンテキスト長 |
| `-t, --timeout-secs N` | `EPUBICUS_TIMEOUT_SECS` | `900` | 1 リクエストあたりの HTTP タイムアウト秒数 |
| `-r, --retries N` | `EPUBICUS_RETRIES` | `3` | 初回試行後のリトライ回数。タイムアウト、接続失敗、rate limit、server error、検証失敗時に使う |
| `-x, --max-chars-per-request N` | `EPUBICUS_MAX_CHARS_PER_REQUEST` | `3500` | これより長い XHTML テキストブロックを文境界で複数リクエストに分割。`0` で分割を無効化 |
| `-j, --concurrency N` | `EPUBICUS_CONCURRENCY` | `1` | XHTML ファイル単位で、未キャッシュの provider リクエストを最大 N 件並列実行。rate limit、timeout、server error などの再試行対象エラーが出た場合は実効並列数を自動的に下げ、成功リクエストが続いたら指定上限まで少しずつ戻す |
| `-s, --style STYLE` | `EPUBICUS_STYLE` | `essay` | 文体プリセット。`novel`, `novel-polite`, `tech`, `essay`, `academic`, `business` |
| `-d, --dry-run` | なし | false | provider を呼ばず、原文を使って EPUB 処理だけ確認 |
| `-g, --glossary PATH` | なし | なし | 用語統一に使う glossary JSON |
| `--cache-root PATH` | なし | OS 標準（`%LOCALAPPDATA%\epubicus\cache` / `~/.cache/epubicus`） | キャッシュ root を上書き。入力 EPUB ごとに `<cache-root>/<input-hash>/` 以下に保存 |
| `--no-cache` | なし | false | キャッシュを読み書きしない。既存キャッシュは削除しない |
| `--clear-cache` | なし | false | この入力 EPUB のキャッシュを削除してから翻訳開始 |
| `-k, --keep-cache` | なし | false | 成功完了後もキャッシュを保持（デフォルトは自動削除） |
| `-u, --usage-only` | なし | false | provider を呼ばず、対象ページのAPIリクエスト数と概算トークン数だけを表示 |
| `--passthrough-on-validation-failure` | `EPUBICUS_PASSTHROUGH_ON_VALIDATION_FAILURE` | false | 検証リトライ後も失敗するブロックを原文のまま出力して処理を継続。キャッシュには保存しないため後で再試行可能。リンクやインライン構造を壊したくない目次・索引項目の救済用 |
| `--verbose` | `EPUBICUS_VERBOSE` | false | 処理中の詳細 warning（リトライ、並列数調整、fallback、長文分割など）を表示 |

### `recover`

| オプション | デフォルト | 説明 |
|--|--|--|
| `LOG` | `--cache` 未指定時は必須 | `translate` が `Recovery log:` として表示した `recovery.jsonl` |
| `--cache TARGET` | なし | 入力 EPUB パスまたは cache hash prefix から最新の `recovery.jsonl` を自動選択 |
| `--input PATH` | recovery log の `input_epub` | 入力 EPUB を明示 |
| `--limit N` | 全件 | 復旧する最大件数 |
| `--list` | false | 条件に一致する復旧ログ item を表示するだけで、翻訳は行わない |
| `--page N` | 全ページ | 指定 spine page の item だけを対象にする |
| `--block N` | 全 block | 指定 block index の item だけを対象にする |
| `--reason REASON` | 全理由 | 指定理由の item だけを対象にする。複数回指定可 |
| `--failed-log PATH` | `<LOG のディレクトリ>\failed.jsonl` | 復旧不能 item の出力先 |
| `--rebuild` | false | 選択 item がすべて復旧できた場合、キャッシュから EPUB を再生成 |
| `--output PATH` | recovery log の `output_epub` | `--rebuild` で再生成する EPUB の出力先 |

例:

```powershell
cargo run -- recover $log --list
cargo run -- recover $log --page 12 --block 3
cargo run -- recover $log --reason cache_miss --limit 20
cargo run -- recover $log --rebuild
cargo run -- recover --cache .\book.epub --rebuild
```

### `scan-recovery`

完成済み、または部分出力済みの EPUB を元 EPUB と突き合わせ、未翻訳らしい block から `recovery.jsonl` を作ります。出力先は通常の復旧ログと同じく、入力 EPUB のキャッシュ配下の `recovery\<出力EPUB名>\` です。

| オプション | デフォルト | 説明 |
|--|--|--|
| `INPUT` | 必須 | 元の入力 EPUB |
| `OUTPUT` | 必須 | 検査する翻訳済み、または部分翻訳済み EPUB |
| `--limit N` | 全件 | 記録する suspicious block の最大件数 |
| `--recover` | false | 復旧ログ作成後、検出 block を続けて再翻訳 |
| `--rebuild` | false | `--recover` 成功後、検査した EPUB を再生成 |
| `--failed-log PATH` | `<recovery log のディレクトリ>\failed.jsonl` | `--recover` で復旧不能 item を書く先 |

例:

```powershell
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b
cargo run -- recover --cache .\book.epub --rebuild
cargo run -- scan-recovery .\book.epub .\book_jp.epub --provider ollama --model qwen3:14b --recover --rebuild
```

provider ごとの `--model` デフォルト:

| provider | デフォルトモデル |
|--|--|
| `ollama` | `qwen3:14b` |
| `openai` | `gpt-5-mini` |
| `claude` | `claude-sonnet-4-5` |

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
| `cache list` | キャッシュ済みラン一覧（hash / セグメント数 / recovery log 件数 / サイズ / 最終更新 / 入力ファイル） |
| `cache show <hash\|input.epub>` | 指定ランの manifest と recovery log の保存場所・件数を表示。`recover` に渡す `recovery.jsonl` のパスも確認できる（hash プレフィックスまたは入力 EPUB パスで指定） |
| `cache prune --older-than <DAYS> [--yes] [--dry-run]` | `last_updated_at` が N 日以上経過したランを削除 |
| `cache clear --hash <HASH> [--dry-run]` | 単一ランを削除 |
| `cache clear --all [--yes] [--dry-run]` | 全削除。`yes` 全文入力で確認（`--yes` でスキップ） |

`cache` には `--cache-root <PATH>` を渡してデフォルト以外のキャッシュ root を対象にできます。

## Provider

Ollama はデフォルト provider で、ローカルで動作します。

OpenAI Batch API を使った将来の非同期翻訳モードについては
[docs/batch-api-design.md](docs/batch-api-design.md) に設計を、
[docs/batch-api-implementation-plan.md](docs/batch-api-implementation-plan.md) に実装計画をまとめています。

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

この場合は `glossary-review.md` の内容を ChatGPT / Claude に渡し、返ってきた JSON を `glossary.json` として保存して翻訳に使います。AI には、誤検出の削除、重複統合、`dst` の訳語案作成を依頼する前提です。

`glossary-review.md` には作業説明のコメント、各フィールドの意味、修正方針、候補 JSON がまとまって入るため、そのまま ChatGPT / Claude に貼り付けられます。`glossary.candidates.json` 側はコメントなしの正規 JSON として出力します。

`dst` に訳語を入れます。

```json
{
  "source_lang": "en",
  "target_lang": "ja",
  "entries": [
    {
      "src": "Horizon",
      "dst": "ホライゾン"
    }
  ]
}
```

翻訳時に指定します。

```powershell
cargo run -- translate .\book.epub -o .\book.ja.epub --glossary .\glossary.json
```

毎回すべての用語を送るのではなく、現在のブロックに登場する `src` だけを provider に渡します。技術書の専門用語、小説の人物名・地名・組織名の表記統一に使えます。
翻訳時に provider へ渡すのは `src => dst` だけです。既存の用語集に `kind` と `note` があっても読み込めますが、翻訳プロンプトには含めません。`src` / `dst` の前後空白は無視され、`dst` が空の entry は翻訳時に使われません。

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
- `--partial-from-cache` はモデルを呼ばず、キャッシュヒットしたブロックだけ訳文に差し替え、キャッシュミスしたブロックを原文のまま残します。未翻訳が残った場合は変換失敗としてエラー終了します。`--no-cache` とは併用できません。
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
