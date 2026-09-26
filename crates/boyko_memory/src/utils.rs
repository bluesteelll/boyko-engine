

pub fn align_up(capacity: usize, cache_line_size: usize) -> usize {
    (capacity + cache_line_size - 1) & !(cache_line_size - 1)
}