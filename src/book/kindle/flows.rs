use super::header::{MobiHeader, NULL_INDEX};
use super::number;
use super::source::RecordSource;
use crate::book::ResourceLimits;
use crate::{Error, Result};
use std::ops::Range;

fn invalid_flows() -> Error {
    Error::Compatibility(
        "The Kindle flow table exceeds its bounds or contains invalid intervals.".into(),
    )
}

pub(super) fn read_flows(
    records: &RecordSource,
    header: &MobiHeader,
    base: usize,
    limits: ResourceLimits,
) -> Result<Vec<Range<usize>>> {
    if header.mobi_version != 8 || header.fdst_index == NULL_INDEX {
        return Ok(Vec::new());
    }
    let index = base
        .checked_add(header.fdst_index as usize)
        .ok_or_else(invalid_flows)?;
    let bytes = records.read(index)?;
    if bytes.get(..4) != Some(b"FDST") {
        return Err(invalid_flows());
    }
    let start = number(&bytes, 4)? as usize;
    let count = number(&bytes, 8)? as usize;
    if start < 12 || count == 0 || count > limits.archive_entries_max {
        return Err(invalid_flows());
    }
    let end = count
        .checked_mul(8)
        .and_then(|length| start.checked_add(length))
        .filter(|end| *end <= bytes.len())
        .ok_or_else(invalid_flows)?;
    let mut previous_end = 0;
    let mut flows = Vec::with_capacity(count);
    for interval in bytes[start..end].chunks_exact(8) {
        let start = number(interval, 0)? as usize;
        let end = number(interval, 4)? as usize;
        // #NOTE: Some writers declare only the HTML length; later flow bounds are also checked during decoding.
        if start < previous_end
            || start > end
            || end as u64 > limits.archive_bytes_max
            || (flows.is_empty() && (start != 0 || end > header.text_length as usize))
        {
            return Err(invalid_flows());
        }
        flows.push(start..end);
        previous_end = end;
    }
    Ok(flows)
}
