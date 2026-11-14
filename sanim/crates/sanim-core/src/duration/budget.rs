//! Publication-local accounting for immutable duration DAG allocations.
use super::{DurationExpr, DurationSymbol, Expression, TimelineEffect};
use std::{
    collections::HashMap,
    mem::size_of,
    sync::{Arc, Weak},
};

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Kind {
    Expression,
    Terms,
    Alternatives,
    Symbol,
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    kind: Kind,
    address: usize,
}
#[derive(Debug)]
enum Guard {
    Expression(Weak<Expression>),
    Terms(Weak<[DurationExpr]>),
    Alternatives(Weak<[TimelineEffect]>),
    Symbol(Weak<str>),
}
impl Guard {
    fn alive(&self) -> bool {
        match self {
            Self::Expression(value) => value.strong_count() > 0,
            Self::Terms(value) => value.strong_count() > 0,
            Self::Alternatives(value) => value.strong_count() > 0,
            Self::Symbol(value) => value.strong_count() > 0,
        }
    }
}
/// One context belongs to one publication/analysis budget. Reuse it across roots,
/// then discard it with that analysis; it does not keep formula values alive.
#[derive(Default)]
pub struct DurationBudget {
    seen: HashMap<Key, Guard>,
}
// Charge spare hash buckets and allocator/table bookkeeping conservatively, not
// merely the live key/value payload. This overhead is itself budget bounded.
const TRACKER_ENTRY_BYTES: usize = 4 * size_of::<(Key, Guard)>() + size_of::<HashMap<Key, Guard>>();
const ARC_HEADER_BYTES: usize = 2 * size_of::<usize>();
impl DurationBudget {
    pub fn charge(&mut self, effect: &TimelineEffect, remaining: &mut usize) -> Result<(), String> {
        self.effect(effect, remaining, super::MAX_DURATION_DEPTH)
    }
    fn allocation(
        &mut self,
        key: Key,
        guard: impl FnOnce() -> Guard,
        bytes: usize,
        remaining: &mut usize,
    ) -> Result<bool, String> {
        if self.seen.get(&key).is_some_and(Guard::alive) {
            return Ok(false);
        }
        *remaining=remaining.checked_sub(bytes.saturating_add(ARC_HEADER_BYTES).saturating_add(TRACKER_ENTRY_BYTES)).ok_or("symbolic duration metadata budget exceeded; simplify formulas, branches, or repeated signatures")?;
        self.seen.insert(key, guard());
        Ok(true)
    }
    fn effect(
        &mut self,
        effect: &TimelineEffect,
        remaining: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("symbolic duration metadata nesting limit exceeded".into());
        }
        match effect {
            TimelineEffect::Symbolic(expr) => {
                self.expression(expr, remaining, super::MAX_DURATION_DEPTH)?
            }
            TimelineEffect::Alternatives(values) => {
                let key = Key {
                    kind: Kind::Alternatives,
                    address: Arc::as_ptr(values) as *const () as usize,
                };
                if self.allocation(
                    key,
                    || Guard::Alternatives(Arc::downgrade(values)),
                    size_of_val(values.as_ref()),
                    remaining,
                )? {
                    for value in values.iter() {
                        self.effect(value, remaining, depth - 1)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn expression(
        &mut self,
        expr: &DurationExpr,
        remaining: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth == 0 {
            return Err("symbolic duration metadata nesting limit exceeded".into());
        }
        let key = Key {
            kind: Kind::Expression,
            address: Arc::as_ptr(&expr.0) as usize,
        };
        if !self.allocation(
            key,
            || Guard::Expression(Arc::downgrade(&expr.0)),
            size_of::<Expression>(),
            remaining,
        )? {
            return Ok(());
        }
        match expr.0.as_ref() {
            Expression::Constant(_) => {}
            Expression::Symbol(DurationSymbol::Parameter(name) | DurationSymbol::Length(name)) => {
                let key = Key {
                    kind: Kind::Symbol,
                    address: Arc::as_ptr(name) as *const () as usize,
                };
                self.allocation(
                    key,
                    || Guard::Symbol(Arc::downgrade(name)),
                    name.len(),
                    remaining,
                )?;
            }
            Expression::Sum(values) | Expression::Product(values) | Expression::Maximum(values) => {
                let key = Key {
                    kind: Kind::Terms,
                    address: Arc::as_ptr(values) as *const () as usize,
                };
                if self.allocation(
                    key,
                    || Guard::Terms(Arc::downgrade(values)),
                    size_of_val(values.as_ref()),
                    remaining,
                )? {
                    for value in values.iter() {
                        self.expression(value, remaining, depth - 1)?;
                    }
                }
            }
        }
        Ok(())
    }
}
