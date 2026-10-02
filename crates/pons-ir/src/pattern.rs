#[derive(Debug, Clone)]
enum Node {
    Empty,
    Literal(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Start,
    End,
    Concat(Vec<Node>),
    Alternate(Vec<Node>),
    Repeat {
        node: Box<Node>,
        bounds: Bounds,
    },
}

#[derive(Debug, Clone)]
struct Bounds {
    min: u32,
    max: Option<u32>,
    greedy: bool,
}

#[derive(Debug, Clone)]
enum ClassItem {
    Single(char),
    Range(char, char),
    Digit,
    NotDigit,
    Word,
    NotWord,
    Space,
    NotSpace,
}

impl ClassItem {
    fn contains(&self, character: char) -> bool {
        match self {
            Self::Single(expected) => character == *expected,
            Self::Range(start, end) => (*start..=*end).contains(&character),
            Self::Digit => character.is_ascii_digit(),
            Self::NotDigit => !character.is_ascii_digit(),
            Self::Word => character.is_ascii_alphanumeric() || character == '_',
            Self::NotWord => !(character.is_ascii_alphanumeric() || character == '_'),
            Self::Space => character.is_whitespace(),
            Self::NotSpace => !character.is_whitespace(),
        }
    }

    fn node(self) -> Node {
        match self {
            Self::Single(character) => Node::Literal(character),
            other => Node::Class {
                negated: false,
                items: vec![other],
            },
        }
    }
}

struct Parser<'a> {
    chars: &'a [char],
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn peek_at(&self, offset: usize) -> Option<char> {
        self.chars.get(self.at + offset).copied()
    }

    fn parse(&mut self) -> Result<Node, String> {
        let node = self.alternate()?;
        match self.peek() {
            Some(character) => Err(format!("unexpected {character:?} at offset {}", self.at)),
            None => Ok(node),
        }
    }

    fn alternate(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.concatenate()?];
        while self.peek() == Some('|') {
            self.at += 1;
            branches.push(self.concatenate()?);
        }
        Ok(if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Alternate(branches)
        })
    }

    fn concatenate(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(character) = self.peek() {
            if character == '|' || character == ')' {
                break;
            }
            items.push(self.repeated()?);
        }
        Ok(match items.len() {
            0 => Node::Empty,
            1 => items.remove(0),
            _ => Node::Concat(items),
        })
    }

    fn repeated(&mut self) -> Result<Node, String> {
        let node = self.atom()?;
        let (min, max) = match self.peek() {
            Some('*') => {
                self.at += 1;
                (0, None)
            }
            Some('+') => {
                self.at += 1;
                (1, None)
            }
            Some('?') => {
                self.at += 1;
                (0, Some(1))
            }
            Some('{') => {
                let start = self.at;
                self.at += 1;
                match self.bounds()? {
                    Some(bounds) => bounds,
                    None => {
                        self.at = start;
                        return Ok(node);
                    }
                }
            }
            _ => return Ok(node),
        };
        let greedy = if self.peek() == Some('?') {
            self.at += 1;
            false
        } else {
            true
        };
        Ok(Node::Repeat {
            node: Box::new(node),
            bounds: Bounds { min, max, greedy },
        })
    }

    fn bounds(&mut self) -> Result<Option<(u32, Option<u32>)>, String> {
        let Some(min) = self.count() else {
            return Ok(None);
        };
        if self.peek() == Some(',') {
            self.at += 1;
            if self.peek() == Some('}') {
                self.at += 1;
                return Ok(Some((min, None)));
            }
            let (Some(max), Some('}')) = (self.count(), self.peek()) else {
                return Ok(None);
            };
            self.at += 1;
            if max < min {
                return Err(format!("repetition range {min}-{max} is inverted"));
            }
            return Ok(Some((min, Some(max))));
        }
        if self.peek() == Some('}') {
            self.at += 1;
            return Ok(Some((min, Some(min))));
        }
        Ok(None)
    }

    fn count(&mut self) -> Option<u32> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|character| character.is_ascii_digit())
        {
            self.at += 1;
        }
        (self.at != start)
            .then(|| {
                self.chars[start..self.at]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .ok()
            })
            .flatten()
    }

    fn atom(&mut self) -> Result<Node, String> {
        let Some(character) = self.peek() else {
            return Err("pattern ends early".into());
        };
        match character {
            '^' => {
                self.at += 1;
                Ok(Node::Start)
            }
            '$' => {
                self.at += 1;
                Ok(Node::End)
            }
            '.' => {
                self.at += 1;
                Ok(Node::Any)
            }
            '[' => self.class(),
            '(' => self.group(),
            '\\' => {
                self.at += 1;
                Ok(self.escape()?.node())
            }
            '*' | '+' | '?' => Err(format!("nothing to repeat at offset {}", self.at)),
            ')' => Err(format!("unmatched ) at offset {}", self.at)),
            literal => {
                self.at += 1;
                Ok(Node::Literal(literal))
            }
        }
    }

    fn group(&mut self) -> Result<Node, String> {
        self.at += 1;
        if self.peek() == Some('?') {
            match self.peek_at(1) {
                Some(':') => self.at += 2,
                Some('=') | Some('!') => return Err("lookahead is not supported".into()),
                _ => return Err("unsupported group".into()),
            }
        }
        let node = self.alternate()?;
        if self.peek() != Some(')') {
            return Err("unclosed group".into());
        }
        self.at += 1;
        Ok(node)
    }

    fn class(&mut self) -> Result<Node, String> {
        self.at += 1;
        let negated = self.peek() == Some('^');
        if negated {
            self.at += 1;
        }
        let mut items: Vec<ClassItem> = Vec::new();
        loop {
            match self.peek() {
                None => return Err("unterminated character class".into()),
                Some(']') if !items.is_empty() => {
                    self.at += 1;
                    return Ok(Node::Class { negated, items });
                }
                Some(']') => return Err("empty character class".into()),
                Some(_) => {}
            }
            let first = self.class_item()?;
            if self.peek() == Some('-') && self.peek_at(1).is_some_and(|next| next != ']') {
                self.at += 1;
                let (ClassItem::Single(start), ClassItem::Single(end)) =
                    (first, self.class_item()?)
                else {
                    return Err("character range needs literal bounds".into());
                };
                if start > end {
                    return Err(format!("character range {start:?}-{end:?} is inverted"));
                }
                items.push(ClassItem::Range(start, end));
            } else {
                items.push(first);
            }
        }
    }

    fn class_item(&mut self) -> Result<ClassItem, String> {
        let Some(character) = self.peek() else {
            return Err("character class ends early".into());
        };
        self.at += 1;
        if character == '\\' {
            return self.escape();
        }
        Ok(ClassItem::Single(character))
    }

    fn escape(&mut self) -> Result<ClassItem, String> {
        let Some(character) = self.peek() else {
            return Err("pattern ends with a backslash".into());
        };
        self.at += 1;
        Ok(match character {
            'd' => ClassItem::Digit,
            'D' => ClassItem::NotDigit,
            'w' => ClassItem::Word,
            'W' => ClassItem::NotWord,
            's' => ClassItem::Space,
            'S' => ClassItem::NotSpace,
            'n' => ClassItem::Single('\n'),
            'r' => ClassItem::Single('\r'),
            't' => ClassItem::Single('\t'),
            'f' => ClassItem::Single('\u{c}'),
            'v' => ClassItem::Single('\u{b}'),
            '0' => ClassItem::Single('\0'),
            'b' | 'B' => return Err("word boundaries are not supported".into()),
            'A' | 'z' | 'Z' | 'p' | 'P' | 'Q' | 'E' | 'G' | 'k' | 'R' | 'X' => {
                return Err(format!("escape \\{character} is not supported"));
            }
            '1'..='9' => return Err("backreferences are not supported".into()),
            'x' | 'u' => {
                let braced = character == 'u' && self.peek() == Some('{');
                let digits = if braced {
                    let start = self.at + 1;
                    let close = self.chars[start..]
                        .iter()
                        .position(|digit| *digit == '}')
                        .map(|offset| start + offset)
                        .ok_or("unterminated code point escape")?;
                    let digits = self.chars[start..close].iter().collect::<String>();
                    self.at = close + 1;
                    digits
                } else {
                    let width = if character == 'x' { 2 } else { 4 };
                    let Some(digits) = self.chars.get(self.at..self.at + width) else {
                        return Err(format!("\\{character} needs {width} hexadecimal digits"));
                    };
                    self.at += width;
                    digits.iter().collect::<String>()
                };
                if !digits.chars().all(|digit| digit.is_ascii_hexdigit()) {
                    return Err(format!("\\{character}{digits} is not a hexadecimal escape"));
                }
                let code =
                    u32::from_str_radix(&digits, 16).map_err(|_| "escape is out of range")?;
                ClassItem::Single(char::from_u32(code).ok_or("escape is not a character")?)
            }
            literal => ClassItem::Single(literal),
        })
    }
}

fn walk(node: &Node, input: &[char], at: usize, next: &mut dyn FnMut(usize) -> bool) -> bool {
    match node {
        Node::Empty => next(at),
        Node::Literal(expected) => input.get(at) == Some(expected) && next(at + 1),
        Node::Any => matches!(input.get(at), Some(character) if *character != '\n') && next(at + 1),
        Node::Class { negated, items } => {
            let found = input.get(at).is_some_and(|character| {
                items.iter().any(|item| item.contains(*character)) != *negated
            });
            found && next(at + 1)
        }
        Node::Start => at == 0 && next(at),
        Node::End => at == input.len() && next(at),
        Node::Concat(items) => walk_items(items, input, at, next),
        Node::Alternate(branches) => branches.iter().any(|branch| walk(branch, input, at, next)),
        Node::Repeat { node, bounds } => walk_repeat(node, bounds, input, at, 0, next),
    }
}

fn walk_items(
    items: &[Node],
    input: &[char],
    at: usize,
    next: &mut dyn FnMut(usize) -> bool,
) -> bool {
    match items.split_first() {
        None => next(at),
        Some((head, tail)) => walk(head, input, at, &mut |position| {
            walk_items(tail, input, position, next)
        }),
    }
}

fn walk_repeat(
    node: &Node,
    bounds: &Bounds,
    input: &[char],
    at: usize,
    count: u32,
    next: &mut dyn FnMut(usize) -> bool,
) -> bool {
    let may_stop = count >= bounds.min;
    let may_continue = bounds.max.is_none_or(|max| count < max);
    if may_stop && !bounds.greedy && next(at) {
        return true;
    }
    if may_continue
        && walk(node, input, at, &mut |position| {
            position > at && walk_repeat(node, bounds, input, position, count + 1, next)
        })
    {
        return true;
    }
    may_stop && bounds.greedy && next(at)
}

pub(crate) fn matches(pattern: &str, value: &str) -> Result<bool, String> {
    let chars = pattern.chars().collect::<Vec<_>>();
    let node = Parser {
        chars: &chars,
        at: 0,
    }
    .parse()?;
    let input = value.chars().collect::<Vec<_>>();
    Ok((0..=input.len()).any(|at| walk(&node, &input, at, &mut |_| true)))
}
