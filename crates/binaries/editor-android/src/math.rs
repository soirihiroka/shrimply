pub fn percentile(values: &[u128], percentile: usize) -> u128 {
    if values.is_empty() {
        0
    } else {
        values[(values.len() - 1) * percentile / 100]
    }
}
