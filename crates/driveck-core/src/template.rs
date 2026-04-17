pub fn expand_template(template: &str, replacements: &[impl AsRef<str>]) -> String {
    let bytes = template.as_bytes();
    let mut output = String::with_capacity(template.len());
    let mut index = 0usize;

    while index < bytes.len() {
        if bytes[index] == b'~' && index + 1 < bytes.len() && bytes[index + 1].is_ascii_digit() {
            let mut cursor = index + 1;
            let mut parsed = 0usize;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                parsed = parsed
                    .saturating_mul(10)
                    .saturating_add((bytes[cursor] - b'0') as usize);
                cursor += 1;
            }
            if parsed > 0 {
                if let Some(replacement) = replacements.get(parsed - 1) {
                    output.push_str(replacement.as_ref());
                }
                index = cursor;
                continue;
            }
        }

        output.push(bytes[index] as char);
        index += 1;
    }

    output
}

#[cfg(test)]
mod tests {
    use super::expand_template;

    #[test]
    fn expands_multi_digit_placeholders() {
        let result = expand_template(
            "~1 / ~12 / ~99",
            &["a", "b", "c", "", "", "", "", "", "", "", "", "z"],
        );
        assert_eq!(result, "a / z / ");
    }
}
