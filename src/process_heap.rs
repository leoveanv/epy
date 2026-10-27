use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct ProcessAllocator {
    used: AtomicUsize,
    limit: AtomicUsize,
}

#[global_allocator]
static ALLOCATOR: ProcessAllocator = ProcessAllocator {
    used: AtomicUsize::new(0),
    limit: AtomicUsize::new(usize::MAX),
};

pub fn limit(bytes: usize) {
    ALLOCATOR.limit.store(bytes, Ordering::Relaxed);
}

impl ProcessAllocator {
    fn reserve(&self, bytes: usize) {
        let reserved = self
            .used
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= self.limit.load(Ordering::Relaxed))
            });
        if reserved.is_err() {
            // #NOTE: Only the isolated codec enables a heap ceiling; exit without an OOM crash report.
            self.limit.store(usize::MAX, Ordering::Relaxed);
            std::process::exit(86);
        }
    }
}

// #NOTE: Every Rust allocation is accounted from startup so pre-limit frees cannot underflow the counter.
unsafe impl GlobalAlloc for ProcessAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.reserve(layout.size());
        let pointer = unsafe { System.alloc(layout) };
        if pointer.is_null() {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        self.reserve(layout.size());
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if pointer.is_null() {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe {
            System.dealloc(pointer, layout);
        }
        self.used.fetch_sub(layout.size(), Ordering::Relaxed);
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        // #NOTE: A moving realloc can hold both buffers briefly; reserve the whole replacement.
        self.reserve(size);
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if pointer.is_null() {
            self.used.fetch_sub(size, Ordering::Relaxed);
        } else {
            self.used.fetch_sub(layout.size(), Ordering::Relaxed);
        }
        pointer
    }
}
