pub(crate) fn decode_pairs(text: &str) -> Vec<(String, String)> {
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    if !bytes.contains(&b'%') && !bytes.contains(&b'+') {
        return text.to_owned();
    }
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                match (nibble(bytes[index + 1]), nibble(bytes[index + 2])) {
                    (Some(high), Some(low)) => {
                        decoded.push(high << 4 | low);
                        index += 3;
                    }
                    _ => {
                        decoded.push(b'%');
                        index += 1;
                    }
                }
            }
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urlencoded_pairs_are_decoded_like_a_form_submission() {
        assert_eq!(
            decode_pairs("name=Ada+Lovelace&tag=a%2Fb&flag&empty="),
            [
                ("name".to_owned(), "Ada Lovelace".to_owned()),
                ("tag".to_owned(), "a/b".to_owned()),
                ("flag".to_owned(), String::new()),
                ("empty".to_owned(), String::new()),
            ]
        );
        assert_eq!(decode_pairs(""), Vec::<(String, String)>::new());
        assert_eq!(
            decode_pairs("bad=%zz"),
            [("bad".to_owned(), "%zz".to_owned())]
        );
    }
}
