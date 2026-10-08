use std::ops::Range;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct FreeRanges(pub(crate) Vec<Range<u32>>);

impl FreeRanges {
    pub fn take<T: Clone>(
        &mut self,
        store: &mut Vec<T>,
        needed: usize,
        empty: T,
        what: &str,
    ) -> Result<u32, String> {
        let too_many = || format!("too many {what}");
        let count = u32::try_from(needed).map_err(|_| too_many())?;
        if let Some(index) = self
            .0
            .iter()
            .position(|range| range.end - range.start >= count)
        {
            let range = &mut self.0[index];
            let base = range.start;
            range.start += count;
            if range.start == range.end {
                self.0.remove(index);
            }
            return Ok(base);
        }
        let base = u32::try_from(store.len()).map_err(|_| too_many())?;
        base.checked_add(count).ok_or_else(too_many)?;
        store.resize(store.len() + needed, empty);
        Ok(base)
    }

    pub fn give(&mut self, range: Range<u32>) {
        self.0.push(range);
        self.0.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<u32>> = Vec::with_capacity(self.0.len());
        for range in self.0.drain(..) {
            match merged.last_mut() {
                Some(last) if last.end == range.start => last.end = range.end,
                _ => merged.push(range),
            }
        }
        self.0 = merged;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freed_ranges_are_reused_first_fit_and_merged() {
        let mut free = FreeRanges::default();
        let mut store = Vec::new();
        assert_eq!(free.take(&mut store, 4, 0u8, "items"), Ok(0));
        assert_eq!(free.take(&mut store, 4, 0u8, "items"), Ok(4));
        assert_eq!(store.len(), 8);
        free.give(0..4);
        assert_eq!(free.take(&mut store, 2, 0u8, "items"), Ok(0));
        assert_eq!(free.0.len(), 1);
        assert_eq!(free.0[0], 2..4);
        free.give(4..8);
        free.give(0..2);
        assert_eq!(free.0.len(), 1);
        assert_eq!(free.0[0], 0..8);
        assert_eq!(store.len(), 8);
    }
}
