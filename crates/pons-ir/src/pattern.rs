const LIMIT: usize = 4096;

#[derive(Debug, Clone)]
enum Node {
    Empty,
    Literal(char),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Boundary(bool),
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
    fn contains(&self, unit: u16) -> bool {
        match self {
            Self::Single(expected) => lead(*expected) == unit || trail(*expected) == unit,
            Self::Range(start, end) => (lead(*start)..=trail(*end)).contains(&unit),
            Self::Digit => (0x30..=0x39).contains(&unit),
            Self::NotDigit => !(0x30..=0x39).contains(&unit),
            Self::Word => word(unit),
            Self::NotWord => !word(unit),
            Self::Space => space(unit),
            Self::NotSpace => !space(unit),
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

fn lead(character: char) -> u16 {
    let code = character as u32;
    if code > 0xffff {
        (0xd800 | (code - 0x10000) >> 10) as u16
    } else {
        code as u16
    }
}

fn trail(character: char) -> u16 {
    let code = character as u32;
    if code > 0xffff {
        (0xdc00 | (code - 0x10000) & 0x3ff) as u16
    } else {
        code as u16
    }
}

fn word(unit: u16) -> bool {
    matches!(unit, 0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a | 0x5f)
}

fn space(unit: u16) -> bool {
    matches!(unit,
        0x0009..=0x000d | 0x0020 | 0x00a0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029
        | 0x202f | 0x205f | 0x3000 | 0xfeff)
}

fn line_terminator(unit: u16) -> bool {
    matches!(unit, 0x000a | 0x000d | 0x2028 | 0x2029)
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

        if self.peek() == Some('?') {
            self.at += 1;
        }
        Ok(Node::Repeat {
            node: Box::new(node),
            bounds: Bounds { min, max },
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
                match self.peek() {
                    Some('b') => {
                        self.at += 1;
                        Ok(Node::Boundary(false))
                    }
                    Some('B') => {
                        self.at += 1;
                        Ok(Node::Boundary(true))
                    }
                    _ => Ok(self.escape()?.node()),
                }
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
            return match self.peek() {
                Some('b') => {
                    self.at += 1;
                    Ok(ClassItem::Single('\u{8}'))
                }
                _ => self.escape(),
            };
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
            '0' if self.peek().is_some_and(|next| next.is_ascii_digit()) => {
                return Err("octal escapes are not supported".into());
            }
            '0' => ClassItem::Single('\0'),
            'c' | 'A' | 'z' | 'Z' | 'p' | 'P' | 'Q' | 'E' | 'G' | 'k' | 'R' | 'X' => {
                return Err(format!("escape \\{character} is not supported"));
            }
            '1'..='9' => return Err("backreferences are not supported".into()),
            'x' | 'u' => {
                let width = if character == 'x' { 2 } else { 4 };
                let digits = self
                    .chars
                    .get(self.at..self.at + width)
                    .map(|digits| digits.iter().collect::<String>())
                    .filter(|digits| digits.chars().all(|digit| digit.is_ascii_hexdigit()));
                let Some(digits) = digits else {
                    return Ok(ClassItem::Single(character));
                };
                self.at += width;
                let code =
                    u32::from_str_radix(&digits, 16).map_err(|_| "escape is out of range")?;
                ClassItem::Single(char::from_u32(code).ok_or("escape is not an character")?)
            }
            literal => ClassItem::Single(literal),
        })
    }
}

enum Step {
    Unit(u16),
    Any,
    Class {
        negated: bool,
        items: Vec<ClassItem>,
    },
    Start,
    End,
    Boundary(bool),
    Split(usize, usize),
    Jump(usize),
    Match,
}

struct Program {
    steps: Vec<Step>,
}

impl Program {
    fn here(&self) -> usize {
        self.steps.len()
    }

    fn emit(&mut self, step: Step) -> usize {
        self.steps.push(step);
        self.steps.len() - 1
    }

    fn node(&mut self, node: &Node) -> Result<(), String> {
        match node {
            Node::Empty => {}
            Node::Literal(character) => {
                let lead = lead(*character);
                self.emit(Step::Unit(lead));
                if trail(*character) != lead {
                    self.emit(Step::Unit(trail(*character)));
                }
            }
            Node::Any => {
                self.emit(Step::Any);
            }
            Node::Class { negated, items } => {
                self.emit(Step::Class {
                    negated: *negated,
                    items: items.clone(),
                });
            }
            Node::Boundary(negated) => {
                self.emit(Step::Boundary(*negated));
            }
            Node::Start => {
                self.emit(Step::Start);
            }
            Node::End => {
                self.emit(Step::End);
            }
            Node::Concat(items) => {
                for item in items {
                    self.node(item)?;
                }
            }
            Node::Alternate(branches) => self.alternate(branches)?,
            Node::Repeat { node, bounds } => self.repeat(node, bounds)?,
        }
        Ok(())
    }

    fn alternate(&mut self, branches: &[Node]) -> Result<(), String> {
        let mut splits = Vec::new();
        for _ in 1..branches.len() {
            splits.push(self.emit(Step::Split(0, 0)));
        }
        let mut starts = Vec::new();
        let mut jumps = Vec::new();
        for (index, branch) in branches.iter().enumerate() {
            starts.push(self.here());
            self.node(branch)?;
            if index + 1 < branches.len() {
                jumps.push(self.emit(Step::Jump(0)));
            }
        }
        let end = self.here();
        let last = branches.len() - 1;
        for (index, split) in splits.iter().copied().enumerate() {
            let rest = if index + 1 < splits.len() {
                split + 1
            } else {
                starts[last]
            };
            self.steps[split] = Step::Split(starts[index], rest);
        }
        for jump in jumps {
            self.steps[jump] = Step::Jump(end);
        }
        Ok(())
    }

    fn repeat(&mut self, node: &Node, bounds: &Bounds) -> Result<(), String> {
        let longest = bounds.max.unwrap_or(bounds.min + 1);
        if longest as usize > LIMIT {
            return Err(format!(
                "a repetition of {longest} is too many steps to run"
            ));
        }
        for _ in 0..bounds.min {
            self.node(node)?;
        }
        match bounds.max {
            None => self.star(node)?,
            Some(max) => {
                for _ in bounds.min..max {
                    self.optional(node)?;
                }
            }
        }
        Ok(())
    }

    fn optional(&mut self, node: &Node) -> Result<(), String> {
        let split = self.emit(Step::Split(0, 0));
        self.node(node)?;
        let end = self.here();
        self.steps[split] = Step::Split(split + 1, end);
        Ok(())
    }

    fn star(&mut self, node: &Node) -> Result<(), String> {
        let split = self.emit(Step::Split(0, 0));
        self.node(node)?;
        self.emit(Step::Jump(split));
        let end = self.here();
        self.steps[split] = Step::Split(split + 1, end);
        Ok(())
    }
}

fn compile(node: &Node) -> Result<Vec<Step>, String> {
    let mut program = Program { steps: Vec::new() };
    program.node(node)?;
    program.emit(Step::Match);
    Ok(program.steps)
}

fn enter(
    steps: &[Step],
    at: usize,
    mut pc: usize,
    input: &[u16],
    marks: &mut [u64],
    generation: u64,
    live: &mut Vec<usize>,
) {
    loop {
        if marks[pc] == generation {
            return;
        }
        marks[pc] = generation;
        match steps[pc] {
            Step::Split(first, second) => {
                enter(steps, at, first, input, marks, generation, live);
                pc = second;
            }
            Step::Jump(target) => pc = target,
            Step::Start if at == 0 => pc += 1,
            Step::End if at == input.len() => pc += 1,
            Step::Boundary(negated) if boundary(input, at) != negated => pc += 1,
            Step::Start | Step::End | Step::Boundary(_) => return,
            _ => {
                live.push(pc);
                return;
            }
        }
    }
}

fn boundary(input: &[u16], at: usize) -> bool {
    let before = at > 0 && word(input[at - 1]);
    let after = at < input.len() && word(input[at]);
    before != after
}

fn generation(at: usize) -> u64 {
    2 * at as u64 + 1
}

fn run(steps: &[Step], input: &[u16]) -> bool {
    let finished = steps.len() - 1;
    let mut marks = vec![0; steps.len()];
    let mut live = Vec::with_capacity(steps.len());
    let mut ahead = Vec::with_capacity(steps.len());
    for at in 0..=input.len() {
        let generation = generation(at);

        enter(steps, at, 0, input, &mut marks, generation, &mut live);
        if live.contains(&finished) {
            return true;
        }
        ahead.clear();
        let unit = input.get(at).copied();
        for pc in live.iter().copied() {
            let matched = match &steps[pc] {
                Step::Unit(expected) => unit == Some(*expected),
                Step::Any => unit.is_some_and(|unit| !line_terminator(unit)),
                Step::Class { negated, items } => unit
                    .is_some_and(|unit| items.iter().any(|item| item.contains(unit)) != *negated),
                _ => false,
            };
            if matched {
                enter(
                    steps,
                    at + 1,
                    pc + 1,
                    input,
                    &mut marks,
                    generation + 1,
                    &mut ahead,
                );
            }
        }
        std::mem::swap(&mut live, &mut ahead);
    }
    false
}

fn parse(pattern: &str) -> Result<Node, String> {
    let chars = pattern.chars().collect::<Vec<_>>();
    Parser {
        chars: &chars,
        at: 0,
    }
    .parse()
}

pub(crate) fn compiles(pattern: &str) -> Result<(), String> {
    compile(&parse(pattern)?).map(drop)
}

pub(crate) fn matches(pattern: &str, value: &str) -> Result<bool, String> {
    let steps = compile(&parse(pattern)?)?;

    let input = value.encode_utf16().collect::<Vec<_>>();
    Ok(run(&steps, &input))
}

#[cfg(test)]
mod tests {
    use super::generation;

    #[test]
    fn a_generation_is_never_reused_so_a_mark_never_looks_visited_early() {
        let mut positions = vec![0, 1, 2, u32::MAX as usize / 2, u32::MAX as usize];
        positions.push(isize::MAX as usize / 2);
        positions.push(usize::MAX / 2);
        positions.sort_unstable();
        assert_eq!(
            generation(0),
            1,
            "0 is never a generation, so nothing looks visited"
        );
        for pair in positions.windows(2) {
            let (before, after) = (pair[0], pair[1]);
            assert!(
                generation(before) < generation(after),
                "position {after} reuses the generation of {before}, so a mark written for one \
                 reads as written for the other"
            );
            assert!(
                generation(before) + 1 < generation(after),
                "the lookahead generation at {before} must differ from the one at {after}"
            );
        }
        assert_eq!(
            generation(usize::MAX / 2),
            2 * (usize::MAX / 2) as u64 + 1,
            "the widest input a host can hold must not overflow the counter"
        );
    }
}
