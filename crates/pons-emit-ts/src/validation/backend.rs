use pons_ir::{DecimalWire, Doc, FieldFormat, Presence};
use serde_json::Value;

use crate::EmitError;

const INTEGER_DIGITS: usize = 39;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Capability(&'static str);

impl Capability {
    #[allow(dead_code)]
    pub(crate) const fn new(id: &'static str) -> Self {
        Capability(id)
    }

    pub(crate) const fn id(self) -> &'static str {
        self.0
    }
}

#[allow(dead_code)]
pub(crate) enum Rendered {
    Exact(String),
    Widened(String, Capability),
    Unsupported(Capability),
}

impl Rendered {
    pub(crate) fn exact(schema: impl Into<String>) -> Self {
        Rendered::Exact(schema.into())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntCarrier {
    Text,
    Bigint,
    Number,
}

pub(crate) struct IntegerShape<'a> {
    pub(crate) carrier: IntCarrier,
    pub(crate) bits: u16,
    pub(crate) signed: bool,
    pub(crate) minimum: Option<&'a Value>,
    pub(crate) maximum: Option<&'a Value>,
    pub(crate) multiple_of: Option<&'a Value>,
}

pub(crate) fn integral_constraint(value: &Value) -> Result<String, EmitError> {
    let Some(number) = value.as_number() else {
        return Err(EmitError(format!(
            "integer constraint {value} is not a number"
        )));
    };
    let text = number.to_string();
    let (negative, unsigned) = match text.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, text.as_str()),
    };
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (
            mantissa,
            exponent.parse::<i64>().map_err(|_| integral_error(value))?,
        ),
        None => (unsigned, 0),
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((whole, fraction)) => (whole, fraction),
        None => (mantissa, ""),
    };
    if (whole.is_empty() && fraction.is_empty())
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(integral_error(value));
    }
    let mut digits = String::with_capacity(whole.len() + fraction.len());
    digits.push_str(whole);
    digits.push_str(fraction);
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return Ok("0".to_owned());
    }
    let scale = i64::try_from(fraction.len())
        .map_err(|_| integral_error(value))?
        .checked_sub(exponent)
        .ok_or_else(|| integral_error(value))?;
    let integer = if scale <= 0 {
        let zeros = usize::try_from(scale.unsigned_abs()).map_err(|_| integral_error(value))?;
        if significant.len().saturating_add(zeros) > INTEGER_DIGITS {
            return Err(integral_error(value));
        }
        let mut integer = significant.to_owned();
        integer.extend(std::iter::repeat_n('0', zeros));
        integer
    } else {
        let scale = usize::try_from(scale).map_err(|_| integral_error(value))?;
        if scale >= significant.len()
            || significant[significant.len() - scale..]
                .bytes()
                .any(|byte| byte != b'0')
        {
            return Err(integral_error(value));
        }
        significant[..significant.len() - scale].to_owned()
    };
    Ok(if negative {
        format!("-{integer}")
    } else {
        integer
    })
}

fn integral_error(value: &Value) -> EmitError {
    EmitError(format!(
        "integer constraint {value} is not an integral value"
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StringCarrier {
    Text,
    Date,
}

pub(crate) struct StringShape<'a> {
    pub(crate) format: Option<FieldFormat>,
    pub(crate) carrier: StringCarrier,
    pub(crate) min_length: Option<u64>,
    pub(crate) max_length: Option<u64>,
    pub(crate) pattern: Option<&'a str>,
}

pub(crate) struct Member {
    pub(crate) key: String,
    pub(crate) schema: String,
    pub(crate) presence: Presence,
    pub(crate) accepts_undefined: bool,
}

pub(crate) enum ObjectShape {
    Reject,
    PreserveOpaque,
    PreserveTyped(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeclarationKind {
    Variant,
    Type,
}

pub(crate) struct Declaration<'a> {
    pub(crate) name: &'a str,
    pub(crate) schema: &'a str,
    pub(crate) doc: Option<&'a Doc>,
    pub(crate) dependencies: &'a [String],
    pub(crate) kind: DeclarationKind,
}

pub(crate) struct Binding {
    pub(crate) name: String,
    pub(crate) schema: String,
}

pub(crate) trait Backend {
    fn name(&self) -> &'static str;
    fn reference(&mut self, name: &str, variant: Option<&str>) -> Rendered;
    fn schema(&self, name: &str) -> String;
    fn object(&mut self, members: &[Member], openness: &ObjectShape) -> Rendered;
    fn union(&mut self, members: &[String]) -> Rendered;
    fn literal(&mut self, value: &str) -> Rendered;
    fn external(&mut self, wire: &str, payload: Option<&str>) -> Rendered;
    fn string(&mut self, shape: &StringShape) -> Rendered;
    fn integer(&mut self, shape: &IntegerShape) -> Result<Rendered, EmitError>;
    fn float(&mut self, bits: u8) -> Rendered;
    fn decimal(&mut self, wire: DecimalWire) -> Rendered;
    fn bytes(&mut self) -> Rendered;
    fn boolean(&mut self) -> Rendered;
    fn array(&mut self, item: &str, unique: bool) -> Rendered;
    fn tuple(&mut self, items: &[String]) -> Rendered;
    fn map(&mut self, key: &str, value: &str) -> Rendered;
    fn nullable(&mut self, inner: &str) -> Rendered;
    fn any(&mut self) -> Rendered;
    fn unknown(&mut self) -> Rendered;
    fn null(&mut self) -> Rendered;
    fn unit(&mut self) -> Rendered;
    fn validator(&mut self, schema: &str) -> String;
    fn module(&mut self, declarations: &[Declaration<'_>]) -> Result<String, EmitError>;
}
