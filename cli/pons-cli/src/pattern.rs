pub(crate) fn sample(pattern: &str) -> Option<String> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let mut sampler = Sampler {
        chars: &chars,
        at: 0,
    };
    sampler.alternate()
}

struct Sampler<'a> {
    chars: &'a [char],
    at: usize,
}

impl Sampler<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn alternate(&mut self) -> Option<String> {
        let first = self.concatenate()?;
        while self.peek() == Some('|') {
            self.at += 1;
            self.concatenate()?;
        }
        Some(first)
    }

    fn concatenate(&mut self) -> Option<String> {
        let mut sample = String::new();
        while let Some(character) = self.peek() {
            if character == '|' || character == ')' {
                break;
            }
            sample.push_str(&self.repeated()?);
        }
        Some(sample)
    }

    fn repeated(&mut self) -> Option<String> {
        let atom = self.atom()?;
        let count = match self.peek() {
            Some('*') | Some('+') | Some('?') => {
                self.at += 1;
                if self.peek() == Some('?') {
                    self.at += 1;
                }
                usize::from(!atom.is_empty())
            }
            Some('{') => {
                let start = self.at;
                self.at += 1;
                match self.bounds() {
                    Some((minimum, maximum)) => {
                        if self.peek() == Some('?') {
                            self.at += 1;
                        }
                        if atom.is_empty() || maximum == Some(0) {
                            0
                        } else {
                            usize::try_from(minimum.max(1)).unwrap_or(1).min(64)
                        }
                    }
                    None => {
                        self.at = start;
                        1
                    }
                }
            }
            _ => 1,
        };
        Some(atom.repeat(count))
    }

    fn bounds(&mut self) -> Option<(u32, Option<u32>)> {
        let minimum = self.count()?;
        match self.peek() {
            Some('}') => {
                self.at += 1;
                Some((minimum, Some(minimum)))
            }
            Some(',') => {
                self.at += 1;
                if self.peek() == Some('}') {
                    self.at += 1;
                    return Some((minimum, None));
                }
                let maximum = self.count()?;
                if self.peek() != Some('}') {
                    return None;
                }
                self.at += 1;
                Some((minimum, Some(maximum)))
            }
            _ => None,
        }
    }

    fn count(&mut self) -> Option<u32> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_digit())
        {
            self.at += 1;
        }
        (self.at != start).then(|| {
            self.chars[start..self.at]
                .iter()
                .collect::<String>()
                .parse()
                .ok()
        })?
    }

    fn atom(&mut self) -> Option<String> {
        let character = self.peek()?;
        match character {
            '^' | '$' => {
                self.at += 1;
                Some(String::new())
            }
            '.' => {
                self.at += 1;
                Some("a".into())
            }
            '[' => self.class(),
            '(' => self.group(),
            '\\' => {
                self.at += 1;
                self.escape()
            }
            '*' | '+' | '?' | ')' | '|' => None,
            _ => {
                self.at += 1;
                Some(character.to_string())
            }
        }
    }

    fn group(&mut self) -> Option<String> {
        self.at += 1;
        if self.peek() == Some('?') {
            match self.peek_at(1) {
                Some(':') => self.at += 2,
                _ => return None,
            }
        }
        let sample = self.alternate()?;
        if self.peek() != Some(')') {
            return None;
        }
        self.at += 1;
        Some(sample)
    }

    fn class(&mut self) -> Option<String> {
        self.at += 1;
        let negated = self.peek() == Some('^');
        if negated {
            self.at += 1;
        }
        let mut members = Vec::new();
        loop {
            match self.peek() {
                None => return None,
                Some(']') if !members.is_empty() => {
                    self.at += 1;
                    break;
                }
                Some(']') => return None,
                Some(_) => {}
            }
            let first = self.class_member()?;
            match self.peek() {
                Some('-') if self.peek_at(1).is_some_and(|next| next != ']') => {
                    self.at += 1;
                    let last = self.class_member()?;
                    members.push(Member::Range(first.sample()?, last.sample()?));
                }
                _ => members.push(Member::One(first)),
            }
        }
        if negated {
            ['a', 'b', '0', '-', '_', ' ']
                .into_iter()
                .find(|candidate| !members.iter().any(|member| member.matches(*candidate)))
                .map(|candidate| candidate.to_string())
        } else {
            members
                .first()?
                .sample()
                .map(|character| character.to_string())
        }
    }

    fn class_member(&mut self) -> Option<ClassMember> {
        let character = self.peek()?;
        self.at += 1;
        if character != '\\' {
            return Some(ClassMember::Literal(character));
        }
        let escape = self.peek()?;
        self.at += 1;
        Some(match escape {
            'd' => ClassMember::Digit,
            'w' => ClassMember::Word,
            's' => ClassMember::Space,
            'n' => ClassMember::Literal('\n'),
            'r' => ClassMember::Literal('\r'),
            't' => ClassMember::Literal('\t'),
            'b' => ClassMember::Literal('\u{8}'),
            'x' | 'u' => {
                let width = if escape == 'x' { 2 } else { 4 };
                let digits = self
                    .chars
                    .get(self.at..self.at + width)
                    .map(|digits| digits.iter().collect::<String>())
                    .filter(|digits| digits.chars().all(|digit| digit.is_ascii_hexdigit()))?;
                self.at += width;
                ClassMember::Literal(char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?)
            }
            other => ClassMember::Literal(other),
        })
    }

    fn escape(&mut self) -> Option<String> {
        let character = self.peek()?;
        self.at += 1;
        Some(match character {
            'd' => "0".into(),
            'w' => "a".into(),
            's' => " ".into(),
            'D' | 'W' | 'S' => "a".into(),
            'n' => "\n".into(),
            'r' => "\r".into(),
            't' => "\t".into(),
            'b' | 'B' => String::new(),
            '0'..='9' => return None,
            'c' | 'A' | 'z' | 'Z' | 'p' | 'P' | 'Q' | 'E' | 'G' | 'k' | 'R' | 'X' => return None,
            'x' | 'u' => {
                let width = if character == 'x' { 2 } else { 4 };
                let digits = self
                    .chars
                    .get(self.at..self.at + width)
                    .map(|digits| digits.iter().collect::<String>())
                    .filter(|digits| digits.chars().all(|digit| digit.is_ascii_hexdigit()))?;
                self.at += width;
                char::from_u32(u32::from_str_radix(&digits, 16).ok()?)?.to_string()
            }
            literal => literal.to_string(),
        })
    }
}

#[derive(Clone, Copy)]
enum ClassMember {
    Literal(char),
    Digit,
    Word,
    Space,
}

impl ClassMember {
    fn sample(&self) -> Option<char> {
        match self {
            Self::Literal(character) => Some(*character),
            Self::Digit => Some('0'),
            Self::Word => Some('a'),
            Self::Space => Some(' '),
        }
    }
}

#[derive(Clone, Copy)]
enum Member {
    One(ClassMember),
    Range(char, char),
}

impl Member {
    fn sample(&self) -> Option<char> {
        match self {
            Self::One(ClassMember::Literal(character)) => Some(*character),
            Self::One(ClassMember::Digit) => Some('0'),
            Self::One(ClassMember::Word) => Some('a'),
            Self::One(ClassMember::Space) => Some(' '),
            Self::Range(start, _) => Some(*start),
        }
    }

    fn matches(&self, candidate: char) -> bool {
        match self {
            Self::One(ClassMember::Literal(character)) => *character == candidate,
            Self::One(ClassMember::Digit) => candidate.is_ascii_digit(),
            Self::One(ClassMember::Word) => candidate.is_ascii_alphanumeric() || candidate == '_',
            Self::One(ClassMember::Space) => candidate.is_whitespace(),
            Self::Range(start, end) => (*start..=*end).contains(&candidate),
        }
    }
}
