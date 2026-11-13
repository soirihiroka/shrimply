//! Ordered edit identity, with exact source coordinates for fold-only changes.
use imbl::Vector;
use sanim_core::code::view::Token;
use std::collections::BTreeMap;

// Both the number of frontier entries retained for reconstruction and token
// comparisons consume this budget. A wholesale rewrite cannot trigger quadratic
// work: ambiguous unmatched tokens simply leave/enter instead of stealing identity.
const MAX_MATCH_WORK: usize = 1_000_000;

pub(super) fn matches(from: &Vector<Token>, to: &Vector<Token>) -> Vec<Option<usize>> {
    let mut output = vec![None; to.len()];
    if from
        .front()
        .zip(to.front())
        .is_some_and(|(a, b)| a.revision == b.revision)
    {
        let old: BTreeMap<_, _> = from
            .iter()
            .enumerate()
            .map(|(index, token)| ((token.line, token.column, token.key.as_ref()), index))
            .collect();
        for (index, token) in to.iter().enumerate() {
            output[index] = old
                .get(&(token.line, token.column, token.key.as_ref()))
                .copied();
        }
        return output;
    }
    let mut start = 0;
    while start < from.len().min(to.len()) && from[start].key == to[start].key {
        output[start] = Some(start);
        start += 1;
    }
    let (mut old_end, mut new_end) = (from.len(), to.len());
    while old_end > start && new_end > start && from[old_end - 1].key == to[new_end - 1].key {
        old_end -= 1;
        new_end -= 1;
        output[new_end] = Some(old_end);
    }
    ordered(
        old_end - start,
        new_end - start,
        start,
        &mut output,
        |old, new| from[old].key == to[new].key,
    );

    // A unique moved identifier can still morph across the ordered edit path.
    // Repeated words never use this fallback: their identity requires context.
    type Occurrences = (Option<usize>, usize, Option<usize>, usize);
    let mut unique: BTreeMap<&str, Occurrences> = BTreeMap::new();
    for (index, token) in from.iter().enumerate() {
        let entry = unique.entry(&token.key).or_default();
        entry.0 = Some(index);
        entry.1 += 1;
    }
    for (index, token) in to.iter().enumerate() {
        let entry = unique.entry(&token.key).or_default();
        entry.2 = Some(index);
        entry.3 += 1;
    }
    for (old, old_count, new, new_count) in unique.into_values() {
        if let (Some(old), 1, Some(new), 1) = (old, old_count, new, new_count) {
            output[new] = Some(old);
        }
    }
    output
}

fn ordered(
    n: usize,
    m: usize,
    start: usize,
    output: &mut [Option<usize>],
    equal: impl Fn(usize, usize) -> bool,
) {
    let (n, m) = (n as isize, m as isize);
    if n == 0 || m == 0 {
        return;
    }
    let mut work = MAX_MATCH_WORK;
    let mut trace: Vec<Vec<isize>> = Vec::new();
    for distance in 0..=(n + m) as usize {
        let mut frontier = Vec::with_capacity(distance + 1);
        for diagonal in 0..=distance {
            if work == 0 {
                return;
            }
            work -= 1;
            let k = 2 * diagonal as isize - distance as isize;
            let mut x = if distance == 0 {
                0
            } else {
                let previous = &trace[distance - 1];
                let down = (diagonal < distance).then(|| previous[diagonal]);
                let right = diagonal
                    .checked_sub(1)
                    .map(|index| previous[index])
                    .filter(|&x| x >= 0)
                    .map(|x| x + 1);
                down.into_iter()
                    .chain(right)
                    .filter(|&x| x >= 0 && x <= n && x - k >= 0 && x - k <= m)
                    .max()
                    .unwrap_or(-1)
            };
            if x < 0 {
                frontier.push(-1);
                continue;
            }
            let mut y = x - k;
            while x < n && y < m {
                if work == 0 {
                    return;
                }
                work -= 1;
                if !equal(start + x as usize, start + y as usize) {
                    break;
                }
                x += 1;
                y += 1;
            }
            frontier.push(x);
            if x == n && y == m {
                trace.push(frontier);
                restore(&trace, start, n, m, output);
                return;
            }
        }
        trace.push(frontier);
    }
}

fn restore(
    trace: &[Vec<isize>],
    start: usize,
    mut x: isize,
    mut y: isize,
    output: &mut [Option<usize>],
) {
    for distance in (1..trace.len()).rev() {
        let diagonal = ((x - y + distance as isize) / 2) as usize;
        let previous = &trace[distance - 1];
        let down = (diagonal < distance).then(|| (previous[diagonal], diagonal));
        let right = diagonal
            .checked_sub(1)
            .map(|index| (previous[index], index));
        let (old_x, previous_diagonal) = down
            .into_iter()
            .chain(right)
            .filter(|&(old_x, index)| {
                let old_y = old_x - (2 * index as isize - (distance - 1) as isize);
                old_x >= 0 && old_y >= 0 && old_x <= x && old_y <= y
            })
            .max_by_key(|&(old_x, index)| old_x + isize::from(index < diagonal))
            .expect("reachable Myers predecessor");
        let old_y = old_x - (2 * previous_diagonal as isize - (distance - 1) as isize);
        let (edge_x, edge_y) = if previous_diagonal < diagonal {
            (old_x + 1, old_y)
        } else {
            (old_x, old_y + 1)
        };
        while x > edge_x && y > edge_y {
            x -= 1;
            y -= 1;
            output[start + y as usize] = Some(start + x as usize);
        }
        x = old_x;
        y = old_y;
    }
    while x > 0 && y > 0 {
        x -= 1;
        y -= 1;
        output[start + y as usize] = Some(start + x as usize);
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_MATCH_WORK, ordered};
    #[test]
    fn ordered_paths_bound_work_for_wholesale_unrelated_rewrites() {
        let comparisons = std::cell::Cell::new(0usize);
        let mut output = vec![None; 10_000];
        ordered(10_000, output.len(), 0, &mut output, |_, _| {
            comparisons.set(comparisons.get() + 1);
            false
        });
        assert!(comparisons.get() <= MAX_MATCH_WORK);
        assert!(output.iter().all(Option::is_none));
    }
    #[test]
    fn ordered_paths_match_an_independent_lcs_for_all_short_repeated_sequences() {
        let sequences: Vec<Vec<bool>> = (0..=5)
            .flat_map(|len| {
                (0..1usize << len)
                    .map(move |bits| (0..len).map(|index| bits & (1 << index) != 0).collect())
            })
            .collect();
        for from in &sequences {
            for to in &sequences {
                let mut expected = vec![vec![0; to.len() + 1]; from.len() + 1];
                for (i, old) in from.iter().enumerate() {
                    for (j, new) in to.iter().enumerate() {
                        expected[i + 1][j + 1] = if old == new {
                            expected[i][j] + 1
                        } else {
                            expected[i][j + 1].max(expected[i + 1][j])
                        };
                    }
                }
                let mut actual = vec![None; to.len()];
                ordered(from.len(), to.len(), 0, &mut actual, |i, j| {
                    from[i] == to[j]
                });
                let mut previous = None;
                for (j, i) in actual.iter().enumerate() {
                    if let Some(i) = i {
                        assert_eq!(from[*i], to[j]);
                        assert!(previous.is_none_or(|previous| previous < *i));
                        previous = Some(*i);
                    }
                }
                assert_eq!(
                    actual.iter().flatten().count(),
                    expected[from.len()][to.len()],
                    "{from:?} -> {to:?}"
                );
            }
        }
    }
}
