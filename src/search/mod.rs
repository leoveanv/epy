mod lowercase;
mod models;
mod native;
pub use models::{SearchDirection, SearchJob, SearchResult, SearchSelection};
mod projection;
mod syntax;

use crate::cancellation::Cancellation;
use crate::document::layout::TextLayout;
use crate::{Error, Result};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
pub struct SearchLimits {
    pub pattern_bytes_max: usize,
    pub compiled_bytes_max: usize,
    pub match_steps_max: u32,
    pub depth_max: u32,
    pub heap_kib_max: u32,
    pub matches_max: usize,
    pub duration_max: Duration,
}

impl Default for SearchLimits {
    fn default() -> Self {
        Self {
            pattern_bytes_max: 4096,
            compiled_bytes_max: 4 * 1024 * 1024,
            match_steps_max: 1_000_000,
            depth_max: 256,
            heap_kib_max: 16 * 1024,
            matches_max: 50_000,
            duration_max: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    pub row: usize,
    pub column_start: usize,
    pub column_end: usize,
    pub offset: usize,
    pub screen_row: usize,
}

#[derive(Clone, Debug)]
pub struct DocumentMatches {
    pub content_id: String,
    pub hits: Vec<SearchHit>,
}

pub struct SearchPattern {
    native: native::CompiledPattern,
    limits: SearchLimits,
}

impl SearchPattern {
    pub fn compile(pattern: &str, limits: SearchLimits) -> Result<Self> {
        if pattern.len() > limits.pattern_bytes_max
            || limits.duration_max.is_zero()
            || limits.duration_max > Duration::from_secs(30)
            || limits.matches_max == 0
        {
            return Err(Error::Compatibility(
                "The search pattern or limits exceed the supported bounds.".into(),
            ));
        }
        let prepared = syntax::prepare(pattern)?;
        Ok(Self {
            native: native::CompiledPattern::compile(&prepared, limits)?,
            limits,
        })
    }

    pub fn find(
        &self,
        layout: &TextLayout,
        cancellation: &Cancellation,
    ) -> Result<DocumentMatches> {
        cancellation.check()?;
        let mut matcher = self.native.matcher(
            self.limits,
            cancellation.clone(),
            Instant::now() + self.limits.duration_max,
        )?;
        let mut hits = Vec::new();
        for (row, line) in layout.text_lines.iter().enumerate() {
            cancellation.check()?;
            let ranges = matcher.find(line, self.limits.matches_max.saturating_sub(hits.len()))?;
            hits.extend(projection::project_hits(layout, row, line, &ranges));
        }
        Ok(DocumentMatches {
            content_id: layout.content_id.clone(),
            hits,
        })
    }
}
