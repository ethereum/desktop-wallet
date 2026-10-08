use std::{num::NonZeroU64, ops::RangeInclusive};

use alloy_rpc_types_eth::Filter;

/// `filter`, bounded to each span of at most `span` blocks across `blocks`, in ascending order.
/// An empty range, which includes an inverted one, yields none.
pub(super) fn span_filters(
    filter: &Filter,
    blocks: RangeInclusive<u64>,
    span: NonZeroU64,
) -> Vec<Filter> {
    if blocks.is_empty() {
        return Vec::new();
    }

    let to = *blocks.end();
    let width = span.get();
    let mut filters = Vec::new();
    let mut start = *blocks.start();
    loop {
        let end = start.saturating_add(width - 1).min(to);
        filters.push(filter.clone().from_block(start).to_block(end));
        if end >= to {
            return filters;
        }
        start = end.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use alloy_rpc_types_eth::Log;
    use alloy_transport::mock::Asserter;

    use super::*;
    use crate::test_support::mocked_provider;

    const SPAN_500: NonZeroU64 = NonZeroU64::new(500).expect("non-zero");

    fn bounds(blocks: RangeInclusive<u64>) -> Vec<(Option<u64>, Option<u64>)> {
        span_filters(&Filter::new(), blocks, SPAN_500)
            .iter()
            .map(|filter| (filter.get_from_block(), filter.get_to_block()))
            .collect()
    }

    fn log_at(block: u64) -> Log {
        Log {
            block_number: Some(block),
            ..Log::default()
        }
    }

    #[test]
    fn a_range_that_divides_exactly_has_no_trailing_span() {
        assert_eq!(
            bounds(0..=999),
            vec![(Some(0), Some(499)), (Some(500), Some(999))]
        );
    }

    #[test]
    fn a_range_with_a_remainder_ends_with_a_short_span() {
        assert_eq!(
            bounds(0..=1100),
            vec![
                (Some(0), Some(499)),
                (Some(500), Some(999)),
                (Some(1000), Some(1100))
            ],
        );
    }

    #[test]
    fn an_inverted_range_yields_no_spans() {
        let (from, to) = (10, 5);
        assert!(bounds(from..=to).is_empty());
    }

    #[tokio::test]
    async fn spans_are_stitched_in_ascending_block_order() {
        let asserter = Asserter::new();
        asserter.push_success(&vec![log_at(10)]);
        asserter.push_success(&vec![log_at(600)]);

        let logs = mocked_provider(&asserter)
            .logs_in_range(&Filter::new(), 0..=999, SPAN_500)
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

        let result = mocked_provider(&asserter)
            .logs_in_range(&Filter::new(), 0..=999, SPAN_500)
            .await;

        assert!(
            result.is_err(),
            "the second span has no queued answer, so the range is incomplete",
        );
    }
}
