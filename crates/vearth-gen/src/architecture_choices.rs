//! Stable, allocation-free draws from authored relative construction weights.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Weighted<T> {
    pub value: T,
    pub weight: u16,
}

impl<T> Weighted<T> {
    pub const fn new(value: T, weight: u16) -> Self {
        Self { value, weight }
    }
}

/// Independent salts keep a roof draw stable when wall alternatives change.
/// Weights express relative prevalence; they need not sum to a percentage.
pub fn choose<T: Copy>(choices: &[Weighted<T>], seed: u64, salt: u64) -> Option<T> {
    choose_where(choices, seed, salt, |_| true)
}

/// Condition the distribution on compatible alternatives, without allocating
/// or transferring a rejected choice's weight to its next array neighbor.
pub fn choose_where<T: Copy>(
    choices: &[Weighted<T>],
    seed: u64,
    salt: u64,
    compatible: impl Fn(T) -> bool,
) -> Option<T> {
    let total: u64 = choices
        .iter()
        .filter(|c| compatible(c.value))
        .map(|c| u64::from(c.weight))
        .sum();
    if total == 0 {
        return None;
    }
    let mut state = seed ^ salt;
    state = (state ^ (state >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    state = (state ^ (state >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    state ^= state >> 31;
    let mut target = ((u128::from(state) * u128::from(total)) >> 64) as u64;
    for choice in choices.iter().filter(|c| compatible(c.value)) {
        if target < u64::from(choice.weight) {
            return Some(choice.value);
        }
        target -= u64::from(choice.weight);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_length_weights_control_frequency_without_duplicate_slots() {
        for count in [1, 3, 7, 19] {
            let choices: Vec<_> = (0..count)
                .map(|i| Weighted::new(i, (i + 1) as u16))
                .collect();
            let mut counts = vec![0_u32; count];
            for seed in 0..100_000 {
                counts[choose(&choices, seed, 0x464f524d).unwrap()] += 1;
            }
            let total = (count * (count + 1) / 2) as f64;
            for (i, actual) in counts.iter().enumerate() {
                let expected = 100_000.0 * (i + 1) as f64 / total;
                assert!((f64::from(*actual) - expected).abs() < expected * 0.09 + 15.0);
            }
        }
        assert_eq!(choose::<u8>(&[], 0, 1), None);
        assert_eq!(choose(&[Weighted::new(9, 0)], 0, 1), None);
    }
}
