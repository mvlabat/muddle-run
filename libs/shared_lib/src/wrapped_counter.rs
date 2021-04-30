use num::traits::AsPrimitive;
use serde::{Deserialize, Serialize};

pub trait Integer = num::Integer
    + Clone
    + Copy
    + Default
    + AsPrimitive<usize>
    + num::Bounded
    + num::traits::ops::overflowing::OverflowingSub
    + num::traits::ops::overflowing::OverflowingAdd
    + std::fmt::Display;

#[derive(Debug, Copy, Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct WrappedCounter<T: num::Integer + Default>(T);

impl<T: Integer> WrappedCounter<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    pub fn value(&self) -> T {
        self.0
    }
}

impl<T: Integer> WrappedCounter<T>
where
    u8: AsPrimitive<T>,
{
    pub fn diff_abs(&self, rhs: Self) -> Self {
        *self.max(&rhs) - *self.min(&rhs)
    }
}

impl<T: Integer> std::fmt::Display for WrappedCounter<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<T: Integer> std::ops::Add for WrappedCounter<T> {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        WrappedCounter(self.0.overflowing_add(&rhs.0).0)
    }
}

impl<T: Integer> std::ops::Sub for WrappedCounter<T> {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        WrappedCounter(self.0.overflowing_sub(&rhs.0).0)
    }
}

impl<T: Integer> std::ops::AddAssign for WrappedCounter<T> {
    fn add_assign(&mut self, rhs: Self) {
        *self = WrappedCounter(self.0.overflowing_add(&rhs.0).0);
    }
}

impl<T: Integer> std::ops::SubAssign for WrappedCounter<T> {
    fn sub_assign(&mut self, rhs: Self) {
        *self = WrappedCounter(self.0.overflowing_sub(&rhs.0).0);
    }
}

impl<T: 'static + Integer> std::cmp::PartialOrd for WrappedCounter<T>
where
    u8: AsPrimitive<T>,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T: 'static + Integer> std::cmp::Ord for WrappedCounter<T>
where
    u8: AsPrimitive<T>,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        let (d1, o1) = self.0.overflowing_sub(&other.0);
        let (d2, _) = other.0.overflowing_sub(&self.0);
        if o1 {
            if d2 > T::max_value() / 2u8.as_() {
                std::cmp::Ordering::Greater
            } else {
                std::cmp::Ordering::Less
            }
        } else if d1 > T::max_value() / 2u8.as_() {
            std::cmp::Ordering::Less
        } else {
            self.0.cmp(&other.0)
        }
    }
}

unsafe impl<T: 'static + Integer> std::iter::Step for WrappedCounter<T>
where
    u8: AsPrimitive<T>,
    usize: AsPrimitive<T>,
{
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start > end {
            None
        } else {
            Some((*end - *start).0.as_())
        }
    }

    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        if count > T::max_value().as_() {
            None
        } else {
            Some(start + WrappedCounter::new(count.as_()))
        }
    }

    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        if count > T::max_value().as_() {
            None
        } else {
            Some(start - WrappedCounter(count.as_()))
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::wrapped_counter::WrappedCounter;

    #[test]
    fn test_add_overflow() {
        assert_eq!(
            WrappedCounter::new(0),
            WrappedCounter::new(u16::MAX) + WrappedCounter::new(1)
        );
    }

    #[test]
    fn test_sub_overflow() {
        assert_eq!(
            WrappedCounter::new(0) - WrappedCounter::new(1),
            WrappedCounter::new(u16::MAX)
        );
    }

    #[test]
    fn test_equal() {
        assert_eq!(WrappedCounter::new(0), WrappedCounter::new(0));
    }

    #[test]
    fn test_less() {
        assert!(WrappedCounter::new(0) < WrappedCounter::new(1));
    }

    #[test]
    fn test_greater() {
        assert!(WrappedCounter::new(1) > WrappedCounter::new(0));
    }

    #[test]
    fn test_less_overflow() {
        assert!(
            WrappedCounter::new(u16::MAX) < WrappedCounter::new(u16::MAX) + WrappedCounter::new(1)
        );
    }

    #[test]
    fn test_greater_overflow() {
        assert!(
            WrappedCounter::new(u16::MAX) + WrappedCounter::new(1) > WrappedCounter::new(u16::MAX)
        );
    }
}
