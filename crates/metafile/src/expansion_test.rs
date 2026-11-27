use std::cell::Cell;

thread_local! {
    static USED: Cell<u64> = const { Cell::new(0) };
}

pub(crate) fn initial_used() -> u64 {
    USED.with(Cell::get)
}

pub(crate) fn with_used<T>(used: u64, action: impl FnOnce() -> T) -> T {
    struct Reset<'a>(&'a Cell<u64>, u64);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            self.0.set(self.1);
        }
    }
    USED.with(|value| {
        let _reset = Reset(value, value.replace(used));
        action()
    })
}
