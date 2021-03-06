//! This crate provides little utilities to declare callbacks inside a scope,
//! that get executed on success, failure, or exit on that scope.
//!
//! This is different than the ScopeGuard crate does,
//! because here it's dependent on the scope's outcome which callbacks should run.
use std::cell::RefCell;

trait Defer {
    fn call(self: Box<Self>);
}

impl<F: FnMut(T), T> Defer for DeferCallback<T, F> {
    fn call(mut self: Box<Self>) {
        (self.call_fn)(self.item);
    }
}

#[derive(Debug)]
struct DeferCallback<T, F> {
    item: T,
    call_fn: F,
}

impl<T, F> DeferCallback<T, F> {
    fn new(item: T, call_fn: F) -> Self {
        Self { item, call_fn }
    }
}

#[derive(Default)]
pub struct Deferring<'a> {
    inner: RefCell<Vec<Box<dyn Defer + 'a>>>,
}

unsafe fn extend_lifetime_mut<'a, 'b, T: ?Sized>(x: &'a mut T) -> &'b mut T {
    std::mem::transmute(x)
}

impl<'a> Deferring<'a> {
    fn new() -> Self {
        Self {
            inner: RefCell::new(Vec::new()),
        }
    }

    fn push<T: 'a>(&self, item: T, closure: impl FnMut(T) + 'a) -> &'a mut T {
        let mut deferred = Box::new(DeferCallback::new(item, closure));

        // This operation is safe,
        // `deferred` is stored on the heap, moving the box does not invalidate pointers to the internals,
        // and we never touch the box internals again without &mut self.
        // Rust can't prove this, so in order to return a mutable reference to T,
        // we need to `unsafely` `extend` the lifetime of the borrow.
        let ret = unsafe { extend_lifetime_mut(&mut deferred.item) };
        self.inner.borrow_mut().push(deferred);
        ret
    }

    fn execute(mut self) {
        let v = std::mem::replace(self.inner.get_mut(), vec![]);
        for d in v.into_iter().rev() {
            d.call();
        }
    }
}

/// A guard is a handle to schedule callbacks on, from an outer scope.
#[derive(Default)]
pub struct Guard<'a> {
    /// Callbacks to be run on a scope's success.
    on_scope_success: Deferring<'a>,

    /// Callbacks to be run on a scope's failure.
    on_scope_failure: Deferring<'a>,

    /// Callbacks to be run on a scope's exit.
    on_scope_exit: Deferring<'a>,
}

impl<'a> Guard<'a> {
    /// Schedules defered closure `dc` to run on a scope's success.
    #[allow(clippy::mut_from_ref)]
    pub fn on_scope_success<T: 'a>(&self, item: T, dc: impl FnMut(T) + 'a) -> &mut T {
        self.on_scope_success.push(item, dc)
    }

    /// Schedules defered closure `dc` to run on a scope's exit.
    #[allow(clippy::mut_from_ref)]
    pub fn on_scope_exit<T: 'a>(&self, item: T, dc: impl FnMut(T) + 'a) -> &mut T {
        self.on_scope_exit.push(item, dc)
    }

    /// Schedules defered closure `dc` to run on a scope's failure.
    #[allow(clippy::mut_from_ref)]
    pub fn on_scope_failure<T: 'a>(&self, item: T, dc: impl FnMut(T) + 'a) -> &mut T {
        self.on_scope_failure.push(item, dc)
    }
}

/// A trait to annotate whether a type is `success` or `failure`.
pub trait Failure {
    /// Returns true if the type is in a failure state, false otherwise.
    fn is_error(&self) -> bool;
}

impl<T, E> Failure for Result<T, E> {
    /// `Ok(T)` is success, `Err(E)` is failure.
    fn is_error(&self) -> bool {
        self.is_err()
    }
}

impl<T> Failure for Option<T> {
    /// `Some(T)` is success, `None` is failure.
    fn is_error(&self) -> bool {
        self.is_none()
    }
}

/// Executes the scope `scope`.
/// A scope is a closure, in which access to a guard is granted.
/// A guard is used to schedule callbacks to run on a scope's success, failure, or exit, using
/// [`Guard::on_scope_success`], [`Guard::on_scope_failure`], [`Guard::on_scope_exit`].
///
/// Its important to note that callbacks scheduled with [`Guard::on_scope_exit`] will *always* run, and will always run last.
///
/// # Examples
/// ```
/// use scoped::{Guard, scoped};
///
/// fn main() {
///     use std::cell::Cell;
///
///     let mut number = Cell::new(0);
///
///     scoped(|guard| -> Result<(), ()> {     
///         
///         guard.on_scope_exit(&number, move |n| {
///             assert_eq!(n.get(), 2);
///             n.set(3);
///         });
///
///         guard.on_scope_success(&number, move |n| {
///             assert_eq!(n.get(), 1);
///             n.set(2);
///         });
///
///         number.set(1);
///         Ok(())
///     });
///     assert_eq!(number.get(), 3);
/// }
/// ```
pub fn scoped<'a, R: Failure>(scope: impl FnOnce(&mut Guard<'a>) -> R) -> R {
    let mut guard = Guard::default();

    let ret = scope(&mut guard);

    if !ret.is_error() {
        guard.on_scope_success.execute();
    } else {
        guard.on_scope_failure.execute();
    }

    guard.on_scope_exit.execute();

    ret
}

pub type ScopeResult<E> = Result<(), E>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list() {
        let mut v = vec![1, 2, 3, 4, 5];
        let scope = scoped(|guard| {
            let v = guard.on_scope_success(&mut v, |v| {
                println!("SUCCES!");

                assert_eq!(*v, vec![1, 2, 3, 4, 5, 6, 7]);

                v.push(10);
            });

            let boxed = guard.on_scope_exit(Box::new(1), move |boxed| {
                assert_eq!(*boxed, 12);
            });

            v.push(6);
            v.push(7);

            **boxed = 12;

            Some(10)
        });
    }

    #[test]
    fn main_test() {
        use std::cell::Cell;

        let number = Cell::new(0);

        let n = scoped(|guard| {
            let n = Some(1);

            guard.on_scope_success(&number, move |b| {
                assert!(2 == b.get());
                b.set(0);
            });

            guard.on_scope_failure(&number, move |b| {
                b.set(-1);
            });

            guard.on_scope_exit(&number, move |b| {
                b.set(0);
            });

            number.set(2);

            n
        });

        assert!(number.get() == 0);
        assert_eq!(n, Some(1));
    }
}
