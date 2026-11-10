//! Deterministic placement policy for WASM sheet images.
//! Runtime upload integration is deliberately separate: this module only decides
//! which requested images may be kept in the available flash budget.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Speed {
    Fast,
    Normal,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Request<'a> {
    pub name: &'a str,
    pub bytes: usize,
    pub speed: Speed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision<'a> {
    pub name: &'a str,
    pub speed: Speed,
    pub bytes: usize,
    pub enabled: bool,
}

/// Keep fast sheets first, then background, then normal sheets. Within a class
/// preserve input order so the result is stable and explainable to the UI.
pub fn plan<'a>(requests: &[Request<'a>], capacity: usize) -> alloc::vec::Vec<Decision<'a>> {
    let mut order: alloc::vec::Vec<usize> = (0..requests.len()).collect();
    order.sort_by_key(|&i| match requests[i].speed {
        Speed::Fast => 0,
        Speed::Background => 1,
        Speed::Normal => 2,
    });
    let mut used: usize = 0;
    order
        .into_iter()
        .map(|i| {
            let request = requests[i];
            let enabled = used.saturating_add(request.bytes) <= capacity;
            if enabled {
                used += request.bytes;
            }
            Decision {
                name: request.name,
                speed: request.speed,
                bytes: request.bytes,
                enabled,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_is_fast_background_normal_and_stable() {
        let result = plan(
            &[
                Request { name: "normal", bytes: 4, speed: Speed::Normal },
                Request { name: "fast", bytes: 4, speed: Speed::Fast },
                Request { name: "bg", bytes: 4, speed: Speed::Background },
            ],
            8,
        );
        assert_eq!(result.iter().map(|d| d.name).collect::<alloc::vec::Vec<_>>(), ["fast", "bg", "normal"]);
        assert!(result[0].enabled && result[1].enabled && !result[2].enabled);
    }

    #[test]
    fn fast_does_not_fit_when_budget_is_too_small() {
        let result = plan(&[Request { name: "fast", bytes: 5, speed: Speed::Fast }], 4);
        assert!(!result[0].enabled);
    }
}
