//! Compilation-wide file name registry.
//! Diagnostics carry a file *index*; the human/JSON form resolves it here.
//! Single compilation = single thread, so a thread_local registry is safe
//! (cargo test runs each test in its own thread).

use std::cell::RefCell;

thread_local! {
    static FILES: RefCell<Vec<String>> = RefCell::new(Vec::new());
}

/// register a file name, returning its index
pub fn register(name: impl Into<String>) -> u32 {
    FILES.with(|f| {
        let mut f = f.borrow_mut();
        f.push(name.into());
        (f.len() - 1) as u32
    })
}

/// resolve a file index to its display name
pub fn name(idx: u32) -> String {
    FILES.with(|f| {
        f.borrow()
            .get(idx as usize)
            .cloned()
            .unwrap_or_else(|| "?".into())
    })
}

/// reset the registry (used between compilations in tests)
pub fn clear() {
    FILES.with(|f| f.borrow_mut().clear());
}

