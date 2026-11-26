use percent_encoding::{utf8_percent_encode, AsciiSet, CONTROLS};

/// URL 路径段的安全字符集：保留常见 ASCII 标点，中文等非 ASCII 字节由
/// `utf8_percent_encode` 自动编码；`%` 本身必须编码，避免把上一层的编码误当成原始字符。
const PATH_SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'`')
    .add(b'{')
    .add(b'}');

/// 把比赛 ID、题号等单个路径段编码成可直接拼进 URL / HTTP 头的 ASCII 字符串。
pub fn segment(value: &str) -> String {
    utf8_percent_encode(value, PATH_SEGMENT).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_keeps_safe_ascii_unchanged() {
        assert_eq!(segment("demo-1"), "demo-1");
        assert_eq!(segment("2026_final"), "2026_final");
    }

    #[test]
    fn segment_encodes_chinese_and_percent() {
        let encoded = segment("2026秋季赛");
        assert!(encoded.starts_with("2026%"));
        assert!(encoded.contains("%E7%A7%8B"));
        assert_eq!(segment("a%b"), "a%25b");
    }
}
