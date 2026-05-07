use crate::glossary::GlossaryEntry;

pub(crate) fn user_prompt(source: &str, glossary_subset: &[GlossaryEntry]) -> String {
    let mut prompt = String::new();
    if !glossary_subset.is_empty() {
        prompt.push_str("<glossary>\n");
        for entry in glossary_subset {
            prompt.push_str("- ");
            prompt.push_str(entry.src.trim());
            prompt.push_str(" => ");
            prompt.push_str(entry.dst.trim());
            prompt.push('\n');
        }
        prompt.push_str("</glossary>\n\n");
    }
    prompt.push_str("<source>\n");
    prompt.push_str(source);
    prompt.push_str("\n</source>");
    prompt
}

pub(crate) fn retry_user_prompt(
    source: &str,
    glossary_subset: &[GlossaryEntry],
    invalid_translation: &str,
    validation_error: &str,
    validation_reason: Option<&str>,
) -> String {
    let mut prompt = user_prompt(source, glossary_subset);
    prompt.push_str("\n\n<retry_instruction>\n");
    prompt.push_str("Your previous response failed validation as a translation.\n");
    prompt.push_str("Fix the issue below and output only the Japanese translation.\n");
    prompt.push_str("Issue: ");
    prompt.push_str(validation_error);
    if let Some(reason) = validation_reason {
        append_reason_specific_retry_instruction(&mut prompt, source, reason);
    }
    prompt.push_str("\nPrevious response:\n");
    prompt.push_str(invalid_translation.trim());
    prompt.push_str("\n</retry_instruction>");
    prompt
}

pub(crate) fn system_prompt(style: &str, source_language: Option<&str>) -> String {
    let source_language = source_language_instruction(source_language);
    format!(
        "あなたは多言語から日本語への翻訳の専門家です。{source_language}出版物として通用する自然で読みやすい日本語に翻訳してください。\n\n\
【絶対遵守ルール】\n\
1. 入力中の ⟦…⟧ で囲まれたマーカは、形を一切変えずに訳文に含めてください。\n\
2. マーカの順序は日本語として自然になるように入れ替えて構いませんが、原文に現れた全てのマーカを過不足なく残してください。\n\
3. マーカの中身を改変・追加・削除しないでください。\n\
   Do not create any new ⟦...⟧ markers. A self-closing marker such as ⟦S1⟧ stands alone; never output ⟦/S1⟧.\n\
4. マーカに挟まれた本文も翻訳対象です。タグを表すマーカだけを残し、原文を不要に残さないでください。\n\
5. 翻訳のみを出力し、説明・前置き・括弧書きの注釈を一切付けないでください。\n\
6. <glossary> が与えられた場合、そこにある訳語を必ず使用し、表記を統一してください。\n\n{}",
        style_prompt(style)
    )
}

fn source_language_instruction(source_language: Option<&str>) -> String {
    let Some(language) = source_language
        .map(str::trim)
        .filter(|lang| !lang.is_empty())
    else {
        return "入力文の言語を推定し、".to_string();
    };
    if language.eq_ignore_ascii_case("und") || language.eq_ignore_ascii_case("unknown") {
        return "入力文の言語を推定し、".to_string();
    }
    format!("この EPUB の原文言語メタデータは `{language}` です。この情報を参考にし、")
}

fn style_prompt(style: &str) -> &'static str {
    match style {
        "novel" => {
            "【文体】\n- 地の文: である調。一文を長くしすぎず、句読点でリズムを整えてください。\n- 会話文: 話者の人物像にふさわしい口語。\n- 章タイトル: 簡潔。体言止めを基本にしてください。"
        }
        "novel-polite" => {
            "【文体】\n- 地の文: です・ます調。児童にも読みやすい自然な日本語にしてください。\n- 漢字を控えめにし、会話文は話者に合わせてください。"
        }
        "tech" => {
            "【文体】\n- 地の文: である調。事実を淡々と述べる調子。\n- 専門用語は一般に流通している訳語を優先してください。\n- コードや識別子は翻訳しないでください。"
        }
        "academic" => {
            "【文体】\n- 地の文: 硬めのである調。訳語の厳密さを優先してください。\n- 章タイトルは名詞句を基本にしてください。"
        }
        "business" => {
            "【文体】\n- 地の文: です・ます調。過度にくだけず、実務書として自然な表現にしてください。"
        }
        _ => {
            "【文体】\n- 地の文: である調。原文の論旨と語り口を尊重しつつ、日本語として自然にしてください。\n- 章タイトル: 体言止めを基本にしてください。"
        }
    }
}

fn inline_markers(source: &str) -> Vec<String> {
    let mut markers = Vec::new();
    let mut rest = source;
    while let Some(start) = rest.find('⟦') {
        let after_start = &rest[start..];
        let Some(end) = after_start.find('⟧') else {
            break;
        };
        let marker_end = end + '⟧'.len_utf8();
        let marker = &after_start[..marker_end];
        markers.push(marker.to_string());
        rest = &after_start[marker_end..];
    }
    markers
}

fn append_reason_specific_retry_instruction(prompt: &mut String, source: &str, reason: &str) {
    match reason {
        "missing_placeholder" => {
            let markers = inline_markers(source);
            if !markers.is_empty() {
                prompt.push_str("\nRequired markers:\n");
                for marker in markers {
                    prompt.push_str("- ");
                    prompt.push_str(&marker);
                    prompt.push('\n');
                }
            }
            prompt.push_str("Include every marker above exactly as written. You may reorder markers only when needed for natural Japanese.\n");
        }
        "unchanged_source" | "untranslated_text" | "untranslated_segment" => {
            prompt.push_str("\nProper nouns, URLs, numbers, symbols, file paths, and code-like identifiers may remain unchanged when appropriate. Translate the surrounding prose/body text into Japanese, regardless of the source language.\n");
            prompt.push_str("If the source is a title, heading, caption, or short phrase, still translate it into natural Japanese. Do not return the source text unchanged unless it is only a URL, DOI, ISBN, number, symbol, code, or identifier.\n");
        }
        "truncated" => {
            prompt.push_str("\nDo not omit, summarize, or stop early. Translate through the end of the source text.\n");
        }
        "prompt_leak" | "refusal_or_explanation" => {
            prompt.push_str(
                "\nDo not output explanations, prefaces, notes, tags, or refusal/judgment text. Return only the translated text.\n",
            );
        }
        "empty" => {
            prompt.push_str("\nReturn the translated text. Do not return an empty response.\n");
        }
        _ => {}
    }
}
