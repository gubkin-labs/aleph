//! The runtime budget meter: the guarantee behind static analysis.
//!
//! Static bounds are a promise. The meter is the enforcement: every execution charges what it
//! actually uses, and the first charge over a limit stops it.

use crate::limits::{Cost, Limits, Resource};

/// Tracks what one execution has used.
#[derive(Debug, Clone)]
pub(crate) struct Meter {
    limits: Limits,
    used: Cost,
}

/// An execution went over its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("execution exceeded its {resource} budget of {limit}")]
pub(crate) struct OverBudget {
    pub(crate) resource: Resource,
    pub(crate) limit: u64,
}

impl Meter {
    pub(crate) fn new(limits: Limits) -> Self {
        Self {
            limits,
            used: Cost::default(),
        }
    }

    pub(crate) fn used(&self) -> Cost {
        self.used
    }

    pub(crate) fn charge(&mut self, cost: Cost) -> Result<(), OverBudget> {
        self.used += cost;
        for resource in Resource::ALL {
            let limit = resource.limit(&self.limits);
            if self.used.get(resource) > limit {
                return Err(OverBudget { resource, limit });
            }
        }
        Ok(())
    }

    pub(crate) fn op(&mut self) -> Result<(), OverBudget> {
        self.charge(Cost::ops(1))
    }

    pub(crate) fn read(&mut self, bytes: usize) -> Result<(), OverBudget> {
        self.charge(Cost {
            reads: 1,
            bytes_read: bytes as u64,
            ..Cost::default()
        })
    }

    pub(crate) fn write(&mut self, writes: u64) -> Result<(), OverBudget> {
        self.charge(Cost {
            writes,
            ..Cost::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stops_at_the_first_charge_over_a_limit() {
        let mut meter = Meter::new(Limits {
            writes: 2,
            ..Limits::ROUTE
        });
        assert!(meter.write(2).is_ok());
        assert_eq!(
            meter.write(1),
            Err(OverBudget {
                resource: Resource::Writes,
                limit: 2
            })
        );
    }
}
