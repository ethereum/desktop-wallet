use std::{num::NonZeroU64, ops::RangeInclusive};

use alloy_rpc_types_eth::{Filter, Log};

use super::endpoint::{NetworkEndpoint, NetworkEndpointError};

/// Reads `filter` over `blocks` in spans of at most `span`, in ascending block order.
///
/// Costs one request per span. A failing span aborts the read, so a returned `Vec` always
/// covers the whole range.
///
/// # Errors
/// Whatever the endpoint reports for the first span that fails.
pub async fn logs_in_range(
    endpoint: &dyn NetworkEndpoint,
    filter: &Filter,
    blocks: RangeInclusive<u64>,
    span: NonZeroU64,
) -> Result<Vec<Log>, NetworkEndpointError> {
    let mut logs = Vec::new();
    for chunk in spans(blocks, span) {
        logs.extend(endpoint.logs(&bounded(filter, &chunk)).await?);
    }
    Ok(logs)
}

/// Consecutive spans of at most `span` blocks. An empty or inverted range yields none.
fn spans(blocks: RangeInclusive<u64>, span: NonZeroU64) -> Vec<RangeInclusive<u64>> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let to = *blocks.end();
    let width = span.get();
    let mut spans = Vec::new();
    let mut start = *blocks.start();
    loop {
        let end = start.saturating_add(width - 1).min(to);
        spans.push(start..=end);
        if end >= to {
            return spans;
        }
        start = end.saturating_add(1);
    }
}

/// `filter` restricted to `span`, replacing whatever bounds it arrived with.
fn bounded(filter: &Filter, span: &RangeInclusive<u64>) -> Filter {
    filter
        .clone()
        .from_block(*span.start())
        .to_block(*span.end())
}

#[cfg(test)]
mod tests {
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

    const SPAN_500: NonZeroU64 = NonZeroU64::new(500).expect("non-zero");

    fn log_at(block: u64) -> Log {
        Log {
            block_number: Some(block),
            ..Log::default()
        }
    }

    #[test]
    fn a_range_shorter_than_one_span_is_read_in_one_request() {
        assert_eq!(spans(0..=99, SPAN_500), vec![0..=99]);
    }

    #[test]
    fn a_range_that_divides_exactly_has_no_trailing_span() {
        assert_eq!(spans(0..=999, SPAN_500), vec![0..=499, 500..=999]);
    }

    #[test]
    fn a_range_with_a_remainder_ends_with_a_short_span() {
        assert_eq!(
            spans(0..=1100, SPAN_500),
            vec![0..=499, 500..=999, 1000..=1100],
        );
    }

    #[test]
    fn an_inverted_range_yields_no_spans() {
        let (from, to) = (10, 5);
        assert!(spans(from..=to, SPAN_500).is_empty());
    }

    #[test]
    fn each_span_bounds_the_filter_to_its_own_blocks() {
        let filter = bounded(&Filter::new(), &(500..=999));

        assert_eq!(filter.get_from_block(), Some(500));
        assert_eq!(filter.get_to_block(), Some(999));
    }

    #[tokio::test]
    async fn spans_are_stitched_in_ascending_block_order() {
        let asserter = Asserter::new();
        asserter.push_success(&vec![log_at(10)]);
        asserter.push_success(&vec![log_at(600)]);
        let endpoint = mocked_provider(&asserter);

        let logs = logs_in_range(endpoint.as_ref(), &Filter::new(), 0..=999, SPAN_500)
            .await
            .expect("both spans answer");

        assert_eq!(
            logs.iter().map(|log| log.block_number).collect::<Vec<_>>(),
            vec![Some(10), Some(600)],
        );
    }

    #[tokio::test]
    async fn a_failing_span_aborts_the_whole_range() {
        let asserter = Asserter::new();
        asserter.push_success(&vec![log_at(10)]);
        let endpoint = mocked_provider(&asserter);

        let result = logs_in_range(endpoint.as_ref(), &Filter::new(), 0..=999, SPAN_500).await;

        assert!(
            result.is_err(),
            "the second span has no queued answer, so the range is incomplete",
        );
    }
}
