//! decode-time field validation driven by protovalidate annotations

use std::collections::HashSet;
use std::fmt;

/// Checks the field rules of a decoded message and recurses into nested messages.
pub trait Validate {
    fn validate(&self) -> Result<(), ValidationError>;
}

impl<T: Validate> Validate for Box<T> {
    fn validate(&self) -> Result<(), ValidationError> {
        (**self).validate()
    }
}

impl Validate for () {
    fn validate(&self) -> Result<(), ValidationError> {
        Ok(())
    }
}

/// A violated field rule together with the path of the offending field.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[error("{} `{}` {violation}", if *.for_key { "map key" } else { "field" }, FieldPath(.path))]
pub struct ValidationError {
    path: Vec<PathSegment>,
    for_key: bool,
    violation: Violation,
}

impl ValidationError {
    /// A violation of a field value. The path is built by the callers from the inside out.
    pub fn new(violation: Violation) -> Self {
        Self {
            path: Vec::new(),
            for_key: false,
            violation,
        }
    }

    /// A violation of a map key.
    pub fn key(violation: Violation) -> Self {
        Self {
            path: Vec::new(),
            for_key: true,
            violation,
        }
    }

    /// Prefix the path with a field name.
    pub fn in_field(mut self, name: &'static str) -> Self {
        self.path.insert(0, PathSegment::Field(name));
        self
    }

    /// Prefix the path with a repeated field index.
    pub fn at_index(mut self, index: usize) -> Self {
        self.path.insert(0, PathSegment::Index(index));
        self
    }

    /// Prefix the path with a map key.
    pub fn at_key(mut self, key: &impl fmt::Debug) -> Self {
        self.path.insert(0, PathSegment::Key(format!("{key:?}")));
        self
    }

    pub fn path(&self) -> &[PathSegment] {
        &self.path
    }

    /// Whether the violation concerns the map key at the end of the path.
    pub fn for_key(&self) -> bool {
        self.for_key
    }

    pub fn violation(&self) -> &Violation {
        &self.violation
    }

    /// The dotted field path, e.g. `rule.bounds[2]` or `weights["cu"]`.
    pub fn field_path(&self) -> String {
        FieldPath(&self.path).to_string()
    }
}

/// One element of a [ValidationError] path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathSegment {
    Field(&'static str),
    Index(usize),
    /// The `Debug` representation of the key.
    Key(String),
}

struct FieldPath<'a>(&'a [PathSegment]);

impl fmt::Display for FieldPath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (position, segment) in self.0.iter().enumerate() {
            match segment {
                PathSegment::Field(name) if position == 0 => write!(f, "{name}")?,
                PathSegment::Field(name) => write!(f, ".{name}")?,
                PathSegment::Index(index) => write!(f, "[{index}]")?,
                PathSegment::Key(key) => write!(f, "[{key}]")?,
            }
        }
        Ok(())
    }
}

/// The violated rule.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum Violation {
    #[error("must be {0}")]
    Compare(Comparison),
    /// An inverted range (e.g. `gt: 10, lt: 5`) where the value must lie outside.
    #[error("must be {0} or {1}")]
    Outside(Comparison, Comparison),
    #[error("must be finite")]
    Finite,
    #[error("must be at least {0} characters")]
    MinLen(u64),
    #[error("must be at most {0} characters")]
    MaxLen(u64),
    #[error("must be a valid UUID")]
    Uuid,
    #[error("must contain at least {0} item(s)")]
    MinItems(u64),
    #[error("must contain at most {0} item(s)")]
    MaxItems(u64),
    #[error("must contain unique items")]
    Unique,
    #[error("must contain at least {0} pair(s)")]
    MinPairs(u64),
    #[error("must contain at most {0} pair(s)")]
    MaxPairs(u64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Gt,
    Gte,
    Lt,
    Lte,
}

impl fmt::Display for Op {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Op::Gt => ">",
            Op::Gte => ">=",
            Op::Lt => "<",
            Op::Lte => "<=",
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Comparison {
    pub op: Op,
    pub bound: Number,
}

impl fmt::Display for Comparison {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.op, self.bound)
    }
}

/// A rule bound in the width of the field type.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Number {
    I64(i64),
    U64(u64),
    F32(f32),
    F64(f64),
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::I64(value) => value.fmt(f),
            Number::U64(value) => value.fmt(f),
            Number::F32(value) => value.fmt(f),
            Number::F64(value) => value.fmt(f),
        }
    }
}

impl From<i32> for Number {
    fn from(value: i32) -> Self {
        Number::I64(value.into())
    }
}

impl From<i64> for Number {
    fn from(value: i64) -> Self {
        Number::I64(value)
    }
}

impl From<u32> for Number {
    fn from(value: u32) -> Self {
        Number::U64(value.into())
    }
}

impl From<u64> for Number {
    fn from(value: u64) -> Self {
        Number::U64(value)
    }
}

impl From<f32> for Number {
    fn from(value: f32) -> Self {
        Number::F32(value)
    }
}

impl From<f64> for Number {
    fn from(value: f64) -> Self {
        Number::F64(value)
    }
}

/// Rule checks called by the derived [Validate] implementations.
pub mod rules {
    use super::*;

    /// `gt` or `gte`
    #[derive(Clone, Copy, Debug)]
    pub enum Lower<T> {
        Gt(T),
        Gte(T),
    }

    /// `lt` or `lte`
    #[derive(Clone, Copy, Debug)]
    pub enum Upper<T> {
        Lt(T),
        Lte(T),
    }

    impl<T: PartialOrd + Copy + Into<Number>> Lower<T> {
        fn bound(self) -> T {
            match self {
                Lower::Gt(bound) | Lower::Gte(bound) => bound,
            }
        }

        fn holds(self, value: T) -> bool {
            match self {
                Lower::Gt(bound) => value > bound,
                Lower::Gte(bound) => value >= bound,
            }
        }

        fn comparison(self) -> Comparison {
            match self {
                Lower::Gt(bound) => Comparison {
                    op: Op::Gt,
                    bound: bound.into(),
                },
                Lower::Gte(bound) => Comparison {
                    op: Op::Gte,
                    bound: bound.into(),
                },
            }
        }
    }

    impl<T: PartialOrd + Copy + Into<Number>> Upper<T> {
        fn bound(self) -> T {
            match self {
                Upper::Lt(bound) | Upper::Lte(bound) => bound,
            }
        }

        fn holds(self, value: T) -> bool {
            match self {
                Upper::Lt(bound) => value < bound,
                Upper::Lte(bound) => value <= bound,
            }
        }

        fn comparison(self) -> Comparison {
            match self {
                Upper::Lt(bound) => Comparison {
                    op: Op::Lt,
                    bound: bound.into(),
                },
                Upper::Lte(bound) => Comparison {
                    op: Op::Lte,
                    bound: bound.into(),
                },
            }
        }
    }

    /// protovalidate `gt`/`gte`/`lt`/`lte`. When the upper bound is below the lower bound the
    /// range is inverted and the value must lie outside of it. NaN never satisfies a bound.
    pub fn range<T: PartialOrd + Copy + Into<Number>>(
        value: &T,
        lower: Option<Lower<T>>,
        upper: Option<Upper<T>>,
    ) -> Result<(), Violation> {
        let value = *value;
        match (lower, upper) {
            (Some(lower), Some(upper)) if upper.bound() < lower.bound() => {
                if lower.holds(value) || upper.holds(value) {
                    Ok(())
                } else {
                    Err(Violation::Outside(lower.comparison(), upper.comparison()))
                }
            }
            (lower, upper) => {
                if let Some(lower) = lower.filter(|lower| !lower.holds(value)) {
                    return Err(Violation::Compare(lower.comparison()));
                }
                if let Some(upper) = upper.filter(|upper| !upper.holds(value)) {
                    return Err(Violation::Compare(upper.comparison()));
                }
                Ok(())
            }
        }
    }

    pub trait Finite {
        fn is_finite_value(&self) -> bool;
    }

    impl Finite for f32 {
        fn is_finite_value(&self) -> bool {
            self.is_finite()
        }
    }

    impl Finite for f64 {
        fn is_finite_value(&self) -> bool {
            self.is_finite()
        }
    }

    pub fn finite<T: Finite>(value: &T) -> Result<(), Violation> {
        if value.is_finite_value() {
            Ok(())
        } else {
            Err(Violation::Finite)
        }
    }

    fn count(len: usize) -> u64 {
        u64::try_from(len).unwrap_or(u64::MAX)
    }

    /// Length in Unicode code points.
    pub fn min_len(value: &str, min: u64) -> Result<(), Violation> {
        if count(value.chars().count()) < min {
            Err(Violation::MinLen(min))
        } else {
            Ok(())
        }
    }

    /// Length in Unicode code points.
    pub fn max_len(value: &str, max: u64) -> Result<(), Violation> {
        if count(value.chars().count()) > max {
            Err(Violation::MaxLen(max))
        } else {
            Ok(())
        }
    }

    /// RFC 4122 textual form, any case; the empty string is rejected.
    pub fn uuid(value: &str) -> Result<(), Violation> {
        let bytes = value.as_bytes();
        let valid = bytes.len() == 36
            && bytes.iter().enumerate().all(|(index, byte)| match index {
                8 | 13 | 18 | 23 => *byte == b'-',
                _ => byte.is_ascii_hexdigit(),
            });
        if valid { Ok(()) } else { Err(Violation::Uuid) }
    }

    pub fn min_items(len: usize, min: u64) -> Result<(), Violation> {
        if count(len) < min {
            Err(Violation::MinItems(min))
        } else {
            Ok(())
        }
    }

    pub fn max_items(len: usize, max: u64) -> Result<(), Violation> {
        if count(len) > max {
            Err(Violation::MaxItems(max))
        } else {
            Ok(())
        }
    }

    pub fn min_pairs(len: usize, min: u64) -> Result<(), Violation> {
        if count(len) < min {
            Err(Violation::MinPairs(min))
        } else {
            Ok(())
        }
    }

    pub fn max_pairs(len: usize, max: u64) -> Result<(), Violation> {
        if count(len) > max {
            Err(Violation::MaxPairs(max))
        } else {
            Ok(())
        }
    }

    /// Item types supported by the `unique` rule.
    pub trait UniqueItems: Sized {
        fn all_unique(items: &[Self]) -> bool;
    }

    macro_rules! hashed_unique {
        ($($ty:ty),*) => {$(
            impl UniqueItems for $ty {
                fn all_unique(items: &[Self]) -> bool {
                    let mut seen = HashSet::with_capacity(items.len());
                    items.iter().all(|item| seen.insert(item))
                }
            }
        )*};
    }

    hashed_unique!(i32, i64, u32, u64, bool, String);

    macro_rules! float_unique {
        ($($ty:ty),*) => {$(
            // Equality as in protovalidate: NaN differs from everything and -0.0 equals 0.0.
            // Values equal under `==` are adjacent in the total order.
            impl UniqueItems for $ty {
                fn all_unique(items: &[Self]) -> bool {
                    let mut sorted = items.to_vec();
                    sorted.sort_by(|left, right| left.total_cmp(right));
                    sorted.windows(2).all(|pair| pair[0] != pair[1])
                }
            }
        )*};
    }

    float_unique!(f32, f64);

    pub fn unique<T: UniqueItems>(items: &[T]) -> Result<(), Violation> {
        if T::all_unique(items) {
            Ok(())
        } else {
            Err(Violation::Unique)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::rules::*;
    use super::*;

    #[test]
    fn ranges_follow_protovalidate_semantics() {
        assert!(range(&5, Some(Lower::Gt(0)), Some(Upper::Lte(5))).is_ok());
        assert_eq!(
            range(&0, Some(Lower::Gt(0)), Some(Upper::Lte(5))),
            Err(Violation::Compare(Comparison {
                op: Op::Gt,
                bound: Number::I64(0)
            }))
        );
        assert_eq!(
            range(&6u64, None, Some(Upper::Lt(6)))
                .unwrap_err()
                .to_string(),
            "must be < 6"
        );
        assert!(range(&-1i64, None::<Lower<i64>>, None).is_ok());
        // Inverted: outside of [5, 10].
        assert!(range(&11, Some(Lower::Gt(10)), Some(Upper::Lt(5))).is_ok());
        assert!(range(&4, Some(Lower::Gt(10)), Some(Upper::Lt(5))).is_ok());
        assert_eq!(
            range(&7, Some(Lower::Gt(10)), Some(Upper::Lt(5)))
                .unwrap_err()
                .to_string(),
            "must be > 10 or < 5"
        );
        assert!(range(&f64::NAN, Some(Lower::Gte(0.0)), None).is_err());
        assert!(range(&f64::NAN, None, Some(Upper::Lt(1.0))).is_err());
        assert!(range(&f32::NAN, Some(Lower::Gt(1.0)), Some(Upper::Lt(0.0))).is_err());
        assert_eq!(
            range(&2.5f32, None, Some(Upper::Lte(0.1f32)))
                .unwrap_err()
                .to_string(),
            "must be <= 0.1"
        );
    }

    #[test]
    fn strings_count_code_points_and_check_uuid() {
        assert!(min_len("äö", 2).is_ok());
        assert!(max_len("äö", 2).is_ok());
        assert_eq!(min_len("ä", 2), Err(Violation::MinLen(2)));
        assert_eq!(max_len("äöü", 2), Err(Violation::MaxLen(2)));
        assert!(uuid("123e4567-E89B-12d3-a456-426614174000").is_ok());
        for invalid in [
            "",
            "123e4567e89b12d3a456426614174000",
            "123e4567-e89b-12d3-a456-42661417400g",
            "123e4567-e89b-12d3-a456_426614174000",
        ] {
            assert_eq!(uuid(invalid), Err(Violation::Uuid), "{invalid}");
        }
    }

    #[test]
    fn collections_check_sizes_and_uniqueness() {
        assert!(min_items(1, 1).is_ok());
        assert_eq!(min_items(0, 1), Err(Violation::MinItems(1)));
        assert_eq!(max_items(3, 2), Err(Violation::MaxItems(2)));
        assert_eq!(min_pairs(0, 1), Err(Violation::MinPairs(1)));
        assert_eq!(max_pairs(2, 1), Err(Violation::MaxPairs(1)));
        assert!(unique(&[1, 2, 3]).is_ok());
        assert_eq!(
            unique(&["a".to_owned(), "a".to_owned()]),
            Err(Violation::Unique)
        );
        assert!(unique(&[f64::NAN, f64::NAN, 1.0]).is_ok());
        assert_eq!(unique(&[-0.0f64, 1.0, 0.0]), Err(Violation::Unique));
        assert_eq!(unique(&[true, false, true]), Err(Violation::Unique));
    }

    #[test]
    fn errors_render_the_field_path() {
        let error = ValidationError::new(Violation::Compare(Comparison {
            op: Op::Lte,
            bound: Number::I64(10000),
        }))
        .in_field("period")
        .in_field("ta")
        .in_field("rule");
        assert_eq!(error.to_string(), "field `rule.ta.period` must be <= 10000");
        let error = ValidationError::new(Violation::MinLen(1))
            .at_index(2)
            .in_field("ids");
        assert_eq!(
            error.to_string(),
            "field `ids[2]` must be at least 1 characters"
        );
        assert_eq!(
            error.path(),
            &[PathSegment::Field("ids"), PathSegment::Index(2)]
        );
        let error = ValidationError::key(Violation::MinLen(1))
            .at_key(&String::new())
            .in_field("lots");
        assert!(error.for_key());
        assert_eq!(
            error.to_string(),
            "map key `lots[\"\"]` must be at least 1 characters"
        );
        let error = ValidationError::new(Violation::Finite)
            .at_key(&"cu".to_owned())
            .in_field("lots");
        assert_eq!(error.field_path(), "lots[\"cu\"]");
    }
}
