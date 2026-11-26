pub fn normalize_output(bytes: &[u8]) -> Vec<u8> {
    let mut lines = bytes
        .split(|byte| *byte == b'\n')
        .map(|line| {
            let mut end = line.len();
            while end > 0 && matches!(line[end - 1], b' ' | b'\t' | b'\r') {
                end -= 1;
            }
            &line[..end]
        })
        .collect::<Vec<_>>();

    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }

    let mut normalized = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            normalized.push(b'\n');
        }
        normalized.extend_from_slice(line);
    }
    normalized
}

pub fn outputs_equal(expected: &[u8], actual: &[u8]) -> bool {
    normalize_output(expected) == normalize_output(actual)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_and_trailing_spaces_are_normalized() {
        assert!(outputs_equal(b"a\nb\n", b"a\r\nb  \r\n"));
        assert!(outputs_equal(b"a\nb", b"a\nb\n\n"));
        assert!(outputs_equal(b"", b"\n\n"));
        assert!(outputs_equal(b"a  \nb\t\n", b"a\nb"));
    }

    #[test]
    fn leading_and_internal_spaces_are_significant() {
        assert!(!outputs_equal(b" a\n", b"a\n"));
        assert!(!outputs_equal(b"a\n\nb\n", b"a\nb\n"));
        assert!(!outputs_equal(b"a \nb\n", b"a\n b\n"));
    }

    #[test]
    fn comparison_is_case_sensitive_and_byte_based() {
        assert!(!outputs_equal(b"ABC\n", b"abc\n"));
        assert!(outputs_equal(&[0xff, b'\n'], &[0xff, b'\n']));
    }
}
