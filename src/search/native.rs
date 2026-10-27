use super::SearchLimits;
use crate::cancellation::Cancellation;
use crate::{Error, Result};
use pcre2_sys as ffi;
use std::ffi::{c_int, c_void};
use std::ptr::{self, NonNull};
use std::time::Instant;

// #NOTE: PCRE2 callouts allow cancellation during backtracking, which a backtrack count alone cannot provide.
unsafe extern "C" {
    fn pcre2_set_callout_8(
        context: *mut ffi::pcre2_match_context_8,
        callback: Option<unsafe extern "C" fn(*mut c_void, *mut c_void) -> c_int>,
        data: *mut c_void,
    ) -> c_int;
}

macro_rules! owned_pointer {
    ($name:ident, $target:ty, $free:path) => {
        struct $name(NonNull<$target>);
        impl Drop for $name {
            fn drop(&mut self) {
                // #NOTE: This wrapper owns the allocation and frees it exactly once.
                unsafe {
                    $free(self.0.as_ptr());
                }
            }
        }
    };
}
owned_pointer!(Code, ffi::pcre2_code_8, ffi::pcre2_code_free_8);
owned_pointer!(
    CompileContext,
    ffi::pcre2_compile_context_8,
    ffi::pcre2_compile_context_free_8
);
owned_pointer!(
    MatchContext,
    ffi::pcre2_match_context_8,
    ffi::pcre2_match_context_free_8
);
owned_pointer!(
    MatchData,
    ffi::pcre2_match_data_8,
    ffi::pcre2_match_data_free_8
);

fn allocated<T>(pointer: *mut T) -> Result<NonNull<T>> {
    NonNull::new(pointer)
        .ok_or_else(|| Error::Compatibility("Cannot allocate the native search engine.".into()))
}

fn engine_error(code: c_int) -> String {
    let mut bytes = [0u8; 256];
    // #NOTE: The error buffer is writable for its full length and contains no borrowed engine data.
    let length = unsafe { ffi::pcre2_get_error_message_8(code, bytes.as_mut_ptr(), bytes.len()) };
    if length < 0 {
        format!("native regex error {code}")
    } else {
        String::from_utf8_lossy(&bytes[..length as usize]).into_owned()
    }
}

struct MatchBudget {
    cancellation: Cancellation,
    deadline: Instant,
}

unsafe extern "C" fn check_budget(_block: *mut c_void, data: *mut c_void) -> c_int {
    // #NOTE: The boxed budget outlives the match context and every synchronous match call.
    let budget = unsafe { &*data.cast::<MatchBudget>() };
    if budget.cancellation.is_canceled() || Instant::now() >= budget.deadline {
        ffi::PCRE2_ERROR_CALLOUT
    } else {
        0
    }
}

pub(super) struct CompiledPattern {
    code: Code,
}

impl CompiledPattern {
    pub fn compile(pattern: &str, limits: SearchLimits) -> Result<Self> {
        // #NOTE: All contexts use the engine's default allocator and remain owned by their Rust wrappers.
        let context = CompileContext(allocated(unsafe {
            ffi::pcre2_compile_context_create_8(ptr::null_mut())
        })?);
        let mut error = 0;
        let mut offset = 0;
        // #NOTE: Explicit input lengths preserve embedded NULs; no input is treated as a C string.
        let code = unsafe {
            ffi::pcre2_set_max_pattern_length_8(context.0.as_ptr(), pattern.len());
            ffi::pcre2_set_compile_extra_options_8(
                context.0.as_ptr(),
                ffi::PCRE2_EXTRA_PYTHON_OCTAL,
            );
            ffi::pcre2_set_max_pattern_compiled_length_8(
                context.0.as_ptr(),
                limits.compiled_bytes_max,
            );
            ffi::pcre2_set_parens_nest_limit_8(context.0.as_ptr(), limits.depth_max);
            ffi::pcre2_compile_8(
                pattern.as_ptr(),
                pattern.len(),
                ffi::PCRE2_UTF
                    | ffi::PCRE2_UCP
                    | ffi::PCRE2_CASELESS
                    | ffi::PCRE2_AUTO_CALLOUT
                    | ffi::PCRE2_NEVER_BACKSLASH_C,
                &mut error,
                &mut offset,
                context.0.as_ptr(),
            )
        };
        let code = NonNull::new(code).ok_or_else(|| {
            Error::Compatibility(format!(
                "Regex error near the expanded pattern byte {offset}: {}",
                engine_error(error)
            ))
        })?;
        Ok(Self { code: Code(code) })
    }

    pub fn matcher(
        &self,
        limits: SearchLimits,
        cancellation: Cancellation,
        deadline: Instant,
    ) -> Result<Matcher<'_>> {
        // #NOTE: Match data is sized from this compiled pattern, so its result vector always holds group zero.
        let context = MatchContext(allocated(unsafe {
            ffi::pcre2_match_context_create_8(ptr::null_mut())
        })?);
        let data = MatchData(allocated(unsafe {
            ffi::pcre2_match_data_create_from_pattern_8(self.code.0.as_ptr(), ptr::null_mut())
        })?);
        let mut budget = Box::new(MatchBudget {
            cancellation,
            deadline,
        });
        // #NOTE: The boxed budget has a stable address even when the owning Matcher moves.
        unsafe {
            ffi::pcre2_set_match_limit_8(context.0.as_ptr(), limits.match_steps_max);
            ffi::pcre2_set_depth_limit_8(context.0.as_ptr(), limits.depth_max);
            ffi::pcre2_set_heap_limit_8(context.0.as_ptr(), limits.heap_kib_max);
            pcre2_set_callout_8(
                context.0.as_ptr(),
                Some(check_budget),
                (&mut *budget as *mut MatchBudget).cast(),
            );
        }
        Ok(Matcher {
            code: &self.code,
            context,
            data,
            budget,
        })
    }
}

#[derive(Clone, Copy)]
pub(super) struct ByteRange {
    pub start: usize,
    pub end: usize,
}

pub(super) struct Matcher<'a> {
    code: &'a Code,
    context: MatchContext,
    data: MatchData,
    budget: Box<MatchBudget>,
}

impl Matcher<'_> {
    fn next(&mut self, text: &str, offset: usize, options: u32) -> Result<Option<ByteRange>> {
        self.budget.cancellation.check()?;
        if Instant::now() >= self.budget.deadline {
            return Err(Error::Compatibility(
                "Search exceeded its time limit. Refine the pattern.".into(),
            ));
        }
        // #NOTE: Pattern, subject, context, and output allocations remain alive for this synchronous call.
        let result = unsafe {
            ffi::pcre2_match_8(
                self.code.0.as_ptr(),
                text.as_ptr(),
                text.len(),
                offset,
                options,
                self.data.0.as_ptr(),
                self.context.0.as_ptr(),
            )
        };
        self.budget.cancellation.check()?;
        if Instant::now() >= self.budget.deadline {
            return Err(Error::Compatibility(
                "Search exceeded its time limit. Refine the pattern.".into(),
            ));
        }
        if result == ffi::PCRE2_ERROR_NOMATCH {
            return Ok(None);
        }
        if result < 0 {
            self.budget.cancellation.check()?;
            if Instant::now() >= self.budget.deadline {
                return Err(Error::Compatibility(
                    "Search exceeded its time limit. Refine the pattern.".into(),
                ));
            }
            return Err(Error::Compatibility(format!(
                "Search stopped: {}. Refine the pattern.",
                engine_error(result)
            )));
        }
        // #NOTE: MatchData was allocated from the same pattern and always contains group zero's two offsets.
        let range = unsafe {
            let vector = ffi::pcre2_get_ovector_pointer_8(self.data.0.as_ptr());
            ByteRange {
                start: *vector,
                end: *vector.add(1),
            }
        };
        if range.start > range.end
            || range.end > text.len()
            || !text.is_char_boundary(range.start)
            || !text.is_char_boundary(range.end)
        {
            return Err(Error::Compatibility(
                "The regex produced an invalid Unicode range.".into(),
            ));
        }
        Ok(Some(range))
    }

    pub fn find(&mut self, text: &str, limit: usize) -> Result<Vec<ByteRange>> {
        let mut matches = Vec::new();
        let mut offset = 0;
        let mut after_empty = false;
        while offset <= text.len() {
            let options = if after_empty {
                ffi::PCRE2_NOTEMPTY_ATSTART | ffi::PCRE2_ANCHORED
            } else {
                0
            };
            match self.next(text, offset, options)? {
                Some(found) => {
                    if matches.len() >= limit {
                        return Err(Error::Compatibility(
                            "Too many search matches. Refine the pattern.".into(),
                        ));
                    }
                    after_empty = found.start == found.end;
                    offset = found.end;
                    matches.push(found);
                }
                None if after_empty && offset < text.len() => {
                    offset += text[offset..].chars().next().map_or(1, char::len_utf8);
                    after_empty = false;
                }
                None => break,
            }
        }
        Ok(matches)
    }
}
