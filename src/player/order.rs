//! The daemon owns both the next-play plan and the path back through it.
//! Queue indices identify occurrences: equal URIs must not collapse into one song.
//! Backtracking is bounded in memory and belongs to the current loaded context.
use std::collections::VecDeque;

const BACKTRACK_LIMIT: usize = 512;

#[derive(Default)]
pub(super) struct PlayOrder {
    previous: VecDeque<usize>,
    forward: VecDeque<usize>,
    upcoming: VecDeque<usize>,
    key: Option<(usize, usize, bool, u8)>,
}

impl PlayOrder {
    pub(super) fn reset(&mut self) {
        *self = Self::default();
    }

    fn remember(&mut self, index: usize) {
        self.previous.push_back(index);
        if self.previous.len() > BACKTRACK_LIMIT {
            self.previous.pop_front();
        }
    }

    fn plan(&mut self, current: usize, len: usize, shuffle: bool, repeat: u8) {
        // The generated tail starts after the last visited forward occurrence.
        let current = self.forward.back().copied().unwrap_or(current);
        let key = (current, len, shuffle, repeat);
        if self.key != Some(key) {
            self.upcoming.clear();
            self.key = Some(key);
            if !shuffle && current < len {
                self.upcoming.extend(current + 1..len);
            }
        }
        if current >= len || !self.upcoming.is_empty() {
            return;
        }
        if shuffle && len > 1 {
            let mut rows: Vec<_> = (0..len).filter(|i| *i != current).collect();
            let mut seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64
                | 1;
            for i in (1..rows.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                rows.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            self.upcoming.extend(rows);
        } else if repeat == 1 {
            self.upcoming.extend(0..len);
        }
    }

    pub(super) fn preview(
        &mut self,
        current: usize,
        len: usize,
        shuffle: bool,
        repeat: u8,
    ) -> Vec<usize> {
        if repeat == 2 && current < len {
            return vec![current];
        }
        self.plan(current, len, shuffle, repeat);
        self.forward.iter().chain(&self.upcoming).copied().collect()
    }

    pub(super) fn next(
        &mut self,
        current: usize,
        len: usize,
        shuffle: bool,
        repeat: u8,
        opened: bool,
    ) -> usize {
        if repeat == 2 && current < len {
            return current;
        }
        self.plan(current, len, shuffle, repeat);
        let next = self
            .forward
            .pop_front()
            .or_else(|| self.upcoming.pop_front())
            .unwrap_or(len);
        if opened && current < len && next != current {
            self.remember(current);
        }
        if self.forward.is_empty() {
            self.key = Some((next, len, shuffle, repeat));
        }
        next
    }

    /// Consume a prefix of the existing plan without opening skipped sources
    /// or putting songs that never played into Previous history.
    pub(super) fn select_upcoming(
        &mut self,
        current: usize,
        len: usize,
        shuffle: bool,
        repeat: u8,
        row: usize,
        opened: bool,
    ) -> Option<usize> {
        if row >= self.preview(current, len, shuffle, repeat).len() {
            return None;
        }
        let mut selected = current;
        for step in 0..=row {
            selected = self.next(selected, len, shuffle, repeat, step == 0 && opened);
        }
        Some(selected)
    }

    pub(super) fn previous(
        &mut self,
        current: usize,
        len: usize,
        shuffle: bool,
        repeat: u8,
    ) -> usize {
        self.plan(current, len, shuffle, repeat);
        let previous = self.previous.pop_back().unwrap_or_else(|| {
            if shuffle {
                current
            } else {
                current.saturating_sub(1)
            }
        });
        if previous != current && current < len {
            self.forward.push_front(current);
        }
        previous
    }

    pub(super) fn jump(&mut self, current: usize, next: usize, len: usize) {
        if current != next {
            if current < len {
                self.remember(current);
            }
            self.forward.clear();
            self.upcoming.clear();
            self.key = None;
        }
    }

    pub(super) fn remap(&mut self, map: impl Fn(usize) -> usize) {
        for index in self.previous.iter_mut().chain(&mut self.forward) {
            *index = map(*index);
        }
        self.upcoming.clear();
        self.key = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selecting_a_later_row_keeps_the_tail_and_only_the_played_history() {
        for shuffle in [false, true] {
            for repeat in [0, 1] {
                let mut order = PlayOrder::default();
                let plan = order.preview(0, 9, shuffle, repeat);
                let selected = order
                    .select_upcoming(0, 9, shuffle, repeat, 4, true)
                    .unwrap();
                assert_eq!(selected, plan[4]);
                assert_eq!(order.preview(selected, 9, shuffle, repeat), plan[5..]);
                assert_eq!(order.previous(selected, 9, shuffle, repeat), 0);
                assert_eq!(order.next(0, 9, shuffle, repeat, true), selected);
            }
        }
    }

    #[test]
    fn direct_selection_does_not_remember_a_superseded_unopened_candidate() {
        let mut order = PlayOrder::default();
        assert_eq!(order.next(0, 6, false, 0, true), 1);
        assert_eq!(order.select_upcoming(1, 6, false, 0, 2, false), Some(4));
        assert_eq!(order.previous(4, 6, false, 0), 0);
    }

    #[test]
    fn selecting_from_forward_history_and_repeat_track_uses_the_shown_plan() {
        let mut order = PlayOrder::default();
        order.next(0, 4, false, 0, true);
        order.next(1, 4, false, 0, true);
        assert_eq!(order.previous(2, 4, false, 0), 1);
        assert_eq!(order.previous(1, 4, false, 0), 0);
        assert_eq!(order.select_upcoming(0, 4, false, 0, 1, true), Some(2));
        assert_eq!(order.preview(2, 4, false, 0), [3]);
        assert_eq!(order.previous(2, 4, false, 0), 0);
        assert_eq!(order.select_upcoming(0, 4, false, 2, 1, true), None);
        assert_eq!(order.select_upcoming(0, 4, false, 2, 0, true), Some(0));
    }

    #[test]
    fn shuffle_preview_is_the_plan_that_next_and_back_follow() {
        let mut order = PlayOrder::default();
        let first = order.preview(4, 5, true, 0);
        assert_eq!(first.len(), 4);
        let mut members = first.clone();
        members.sort_unstable();
        assert_eq!(members, [0, 1, 2, 3]);
        assert_eq!(order.preview(4, 5, true, 0), first);
        let next = order.next(4, 5, true, 0, true);
        assert_eq!(next, first[0]);
        assert_eq!(order.previous(next, 5, true, 0), 4);
        assert_eq!(order.preview(4, 5, true, 0)[0], next);
        assert_eq!(order.next(4, 5, true, 0, true), next);
    }

    #[test]
    fn wrapping_and_sequential_backtracking_preserve_occurrences() {
        let mut order = PlayOrder::default();
        assert_eq!(order.previous(2, 3, false, 0), 1);
        assert_eq!(order.next(1, 3, false, 0, true), 2);
        assert_eq!(order.next(2, 3, false, 1, true), 0);
        assert_eq!(order.previous(0, 3, false, 1), 2);
        assert_eq!(order.next(2, 3, false, 1, true), 0);
        assert_eq!(order.next(0, 3, false, 2, true), 0);
    }

    #[test]
    fn reorder_keeps_the_visited_forward_occurrence() {
        let mut order = PlayOrder::default();
        order.jump(2, 0, 3);
        assert_eq!(order.previous(0, 3, false, 0), 2);
        order.remap(|i| {
            if i == 2 {
                1
            } else if i == 1 {
                2
            } else {
                i
            }
        });
        assert_eq!(
            order.next(1, 3, false, 0, true),
            0,
            "reorder must remap, not discard, visited forward history"
        );
    }

    #[test]
    fn context_reset_and_reorder_do_not_retarget_old_occurrences() {
        let mut order = PlayOrder::default();
        let next = order.next(2, 3, true, 0, true);
        order.remap(|i| {
            if i == 2 {
                1
            } else if i == 1 {
                2
            } else {
                i
            }
        });
        let remapped_next = if next == 1 { 2 } else { next };
        assert_eq!(order.previous(remapped_next, 3, true, 0), 1);
        order.reset();
        assert_eq!(order.previous(0, 1, true, 0), 0);
        assert!(order.preview(0, 1, true, 0).is_empty());
        assert_eq!(order.next(0, 0, true, 0, false), 0);
    }
}
