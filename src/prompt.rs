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
            if let Some(kind) = entry.kind.as_deref().filter(|kind| !kind.trim().is_empty()) {
                prompt.push_str(" (");
                prompt.push_str(kind.trim());
                prompt.push(')');
            }
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
) -> String {
    let mut prompt = user_prompt(source, glossary_subset);
    prompt.push_str("\n\n<retry_instruction>\n");
    prompt.push_str("前回の応答は翻訳として検証に失敗しました。以下の問題を修正し、翻訳のみを出力してください。\n");
    prompt.push_str("問題: ");
    prompt.push_str(validation_error);
    prompt.push_str("\n前回の応答:\n");
    prompt.push_str(invalid_translation.trim());
    prompt.push_str("\n</retry_instruction>");
    prompt
}

pub(crate) fn system_prompt(style: &str) -> String {
    format!(
        "あなたは英日翻訳の専門家です。出版物として通用する自然で読みやすい日本語に翻訳してください。\n\n\
【絶対遵守ルール】\n\
1. 入力中の ⟦…⟧ で囲まれたマーカは、形を一切変えずに訳文に含めてください。\n\
2. マーカの順序は日本語として自然になるように入れ替えて構いませんが、原文に現れた全てのマーカを過不足なく残してください。\n\
3. マーカの中身を改変・追加・削除しないでください。\n\
4. マーカに挟まれた英語本文も翻訳対象です。タグを表すマーカだけを残し、英語原文を不要に残さないでください。\n\
5. 翻訳のみを出力し、説明・前置き・括弧書きの注釈を一切付けないでください。\n\
6. <glossary> が与えられた場合、そこにある訳語を必ず使用し、表記を統一してください。\n\n{}",
        style_prompt(style)
    )
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
