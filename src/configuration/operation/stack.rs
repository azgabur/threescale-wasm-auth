use std::borrow::Cow;

use serde::{Deserialize, Serialize};

use super::OperationError;
use crate::log::LogLevel;

#[derive(Debug, thiserror::Error)]
pub enum StackError {
    #[error("input has no values")]
    NoValuesError,
    #[error("output has no values")]
    OutputNoValuesError,
    #[error("index out of bounds")]
    IndexOutOfBounds,
    #[error("requirement not satisfied")]
    RequirementNotSatisfied,
    #[error("inner operation error")]
    InnerOperationError(#[from] Box<OperationError>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CloneMode {
    #[serde(rename = "prepend")]
    PrependResult,
    #[serde(rename = "append")]
    AppendResult,
}

impl Default for CloneMode {
    fn default() -> Self {
        Self::AppendResult
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stack {
    Length {
        min: usize,
        max: usize,
    },
    Join(String),
    Reverse,
    Take {
        #[serde(skip_serializing_if = "Option::is_none")]
        head: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tail: Option<usize>,
    },
    Drop {
        #[serde(skip_serializing_if = "Option::is_none")]
        head: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        tail: Option<usize>,
    },
    Swap {
        from: isize,
        to: isize,
    },
    Indexes(#[serde(default)] Vec<isize>),
    FlatMap(Vec<super::Operation>),
    Select(Vec<super::Operation>),
    Cloned {
        #[serde(default)]
        result: CloneMode,
        ops: Vec<super::Operation>,
    },
    Values {
        #[serde(default)]
        level: LogLevel,
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<String>,
    },
}

impl Stack {
    pub fn process<'a>(
        &self,
        mut input: Vec<Cow<'a, str>>,
    ) -> Result<Vec<Cow<'a, str>>, StackError> {
        if input.is_empty() {
            return Err(StackError::NoValuesError);
        }

        let res = match self {
            Self::Length { min, max } => {
                if input.len() < *min {
                    return Err(StackError::RequirementNotSatisfied);
                }
                if input.len() > *max {
                    return Err(StackError::RequirementNotSatisfied);
                }

                input
            }
            Self::Join(separator) => {
                let joined = input.join(separator.as_str());
                vec![joined.into()]
            }
            Self::Reverse => {
                input.reverse();
                input
            }
            Self::Take { head, tail } => {
                let (mut head_vec, mut tail_vec) = if let Some(head) = head {
                    let tail = input.split_off(core::cmp::min(*head, input.len()));
                    (input, tail)
                } else {
                    (vec![], input)
                };

                let tail = if let Some(tail) = tail {
                    tail_vec.split_off(tail_vec.len().saturating_sub(*tail))
                } else {
                    vec![]
                };

                head_vec.extend(tail.into_iter());
                head_vec
            }
            Self::Drop { head, tail } => {
                let mut tail_vec = if let Some(head) = head {
                    let idx = core::cmp::min(*head, input.len());
                    input.split_off(idx)
                } else {
                    input
                };

                if let Some(tail) = tail {
                    let _ = tail_vec.split_off(tail_vec.len().saturating_sub(*tail));
                    tail_vec
                } else {
                    tail_vec
                }
            }
            Self::Swap { from, to } => {
                use self::indexing::{CollectionLength, Index};
                use core::convert::TryFrom;

                let input_len = CollectionLength::try_from(input.len())?;
                let from = Index::from(*from);
                let to = Index::from(*to);

                if from != to {
                    input.swap(input_len.index_into(from)?, input_len.index_into(to)?);
                }

                input
            }
            Self::Indexes(indexes) => {
                if indexes.is_empty() {
                    // take all values
                    input
                } else {
                    use self::indexing::{CollectionLength, Index};
                    use core::convert::TryFrom;

                    let input_len = CollectionLength::try_from(input.len())?;

                    indexes.iter().try_fold(vec![], |mut acc, &idx| {
                        input_len.index_into(Index::from(idx)).map(|computed_idx| {
                            acc.push(Cow::from(input[computed_idx].to_string()));
                            acc
                        })
                    })?
                }
            }
            Self::FlatMap(ops) => {
                let r = match input.into_iter().try_fold(vec![], |mut acc, e| {
                    let ops = ops.iter().collect::<Vec<_>>();
                    super::process_operations(vec![e], ops.as_slice()).map(|v| {
                        acc.push(v);
                        acc
                    })
                }) {
                    Ok(r) => r,
                    Err(e) => return Err(StackError::InnerOperationError(Box::new(e))),
                };
                r.into_iter().flatten().collect()
            }
            Self::Select(ops) => input
                .into_iter()
                .filter_map(|e| {
                    let ops = ops.iter().collect::<Vec<_>>();
                    super::process_operations(vec![e], ops.as_slice()).ok()
                })
                .flatten()
                .collect::<Vec<_>>(),
            Self::Cloned { result, ops } => {
                let new_stack = input.clone();
                let ops = ops.iter().collect::<Vec<_>>();
                match super::process_operations(new_stack, ops.as_slice()) {
                    Ok(mut v) => match result {
                        CloneMode::AppendResult => {
                            input.extend(v.into_iter());
                            input
                        }
                        CloneMode::PrependResult => {
                            v.extend(input.into_iter());
                            v
                        }
                    },
                    Err(e) => return Err(StackError::InnerOperationError(Box::new(e))),
                }
            }
            Self::Values { level, id } => {
                crate::log!(
                    &"[3scale-auth/stack]",
                    *level,
                    "values at {}: {}",
                    id.as_ref().map(|id| id.as_str()).unwrap_or("()"),
                    input
                        .iter()
                        .map(|s| format!(r#""{}""#, s))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                input
            }
        };

        if res.is_empty() {
            return Err(StackError::OutputNoValuesError);
        }

        Ok(res)
    }
}

mod indexing {
    use super::StackError;

    #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct Index(isize);

    impl Index {
        pub fn into_inner(self) -> isize {
            self.0
        }
    }

    impl core::convert::TryFrom<usize> for Index {
        type Error = StackError;

        fn try_from(value: usize) -> Result<Self, Self::Error> {
            Ok(Self(
                isize::try_from(value).map_err(|_| StackError::IndexOutOfBounds)?,
            ))
        }
    }

    impl From<isize> for Index {
        fn from(value: isize) -> Self {
            Self(value)
        }
    }

    #[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
    pub struct CollectionLength(isize);

    impl CollectionLength {
        pub fn new(value: isize) -> Result<Self, StackError> {
            if value < 0 {
                Err(StackError::IndexOutOfBounds)
            } else {
                Ok(Self(value))
            }
        }

        // This fn will use Ruby-inspired indexing, ie. -1 meaning last element,
        // (-collection_len) - 1 meaning as well last element, etc.
        pub fn index_into(&self, idx: Index) -> Result<usize, StackError> {
            let idx = idx.into_inner();

            // Safety: `usize` casts are safe - idx as usize is done if idx >= 0,
            // and `(idx % self.0) as usize` is safe because -n % m is always positive,
            // since m is always > 0.
            let computed_idx = if idx >= 0 {
                idx as usize
            } else {
                //let Self(total) = self;
                (idx % self.0) as usize
            };

            // Safety: self.0 is checked to be an isize >= 0, so can be casted to usize.
            if computed_idx >= self.0 as usize {
                Err(StackError::IndexOutOfBounds)
            } else {
                Ok(computed_idx)
            }
        }
    }

    impl core::convert::TryFrom<usize> for CollectionLength {
        type Error = StackError;

        fn try_from(value: usize) -> Result<Self, Self::Error> {
            Self::new(isize::try_from(value).map_err(|_| StackError::IndexOutOfBounds)?)
        }
    }
}
